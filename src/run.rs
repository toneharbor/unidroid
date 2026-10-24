use crate::hook::HostResult;
use crate::{CpuData, Emulator, HostFunction, JniHandler, RETURN, Result, TRAMPOLINE, threads};
use unicorn_engine::unicorn_const::{MemType, uc_error};

impl Emulator {
    pub fn call_function(&mut self, function: u64, args: &[u64]) -> Result<u64> {
        self.invoke(
            function,
            args.iter().map(|&bits| (b'L', bits)),
            b'L',
            &mut (),
        )
    }
    pub fn call_function_with(
        &mut self,
        handler: &mut dyn JniHandler,
        function: u64,
        args: &[u64],
    ) -> Result<u64> {
        self.invoke(
            function,
            args.iter().map(|&bits| (b'L', bits)),
            b'L',
            handler,
        )
    }
    pub(crate) fn invoke(
        &mut self,
        function: u64,
        args: impl IntoIterator<Item = (u8, u64)>,
        kind: u8,
        handler: &mut dyn JniHandler,
    ) -> Result<u64> {
        let context = if self.threads.depth > 0 {
            Some(self.cpu()?.context_init()?)
        } else {
            None
        };
        let result = (|| {
            let sp = self.place_arguments(args)?;
            let result = self
                .run(function, handler)
                .and_then(|_| self.result_bits(kind));
            self.set_sp(sp)?;
            result
        })();
        if let Some(context) = context {
            self.cpu_mut()?.context_restore(&context)?;
        }
        result
    }
    fn run(&mut self, pc: u64, handler: &mut dyn JniHandler) -> Result<u64> {
        let started_thread = self.threads.tid();
        self.threads.depth += 1;
        let mut control = crate::debug::control::RunControl::default();
        let result = self
            .run_loop(pc, handler, &mut control)
            .map_err(|error| self.crash(error));
        self.finish_code(result.is_err())?;
        self.finish_calls();
        let cleanup = control.clear_finish(self);
        let result = result.and_then(|value| cleanup.map(|_| value));
        let cleanup = self.finish_heap(result.is_err(), started_thread);
        let result = result.and_then(|value| cleanup.map(|_| value));
        self.threads.depth -= 1;
        if self.threads.depth == 0 && result.is_err() {
            self.restore_main_thread()?;
        }
        result
    }
    fn run_loop(
        &mut self,
        mut pc: u64,
        handler: &mut dyn JniHandler,
        control: &mut crate::debug::control::RunControl,
    ) -> Result<u64> {
        let outermost = self.threads.depth == 1;
        loop {
            let mut stepping = control.step == Some(self.threads.tid());
            let slice_remaining = if !stepping && outermost && self.threads.has_runnable() {
                threads::SLICE_CODE_BYTES
            } else {
                0
            };
            let thread = self.threads.tid();
            let depth = self.threads.depth;
            *self.cpu_mut()?.get_data_mut() = CpuData {
                slice_remaining,
                thread,
                depth,
                skip_breakpoint: control.skip,
                finish: control.finish_target(),
                ..CpuData::default()
            };
            if pc & !1 == RETURN {
                if !outermost || thread == threads::MAIN_TID {
                    return self.reg(0);
                }
                return Err(format!(
                    "guest thread {thread} returned to RETURN at {}",
                    self.location(pc)
                )
                .into());
            }
            let result = self.run_cpu(pc, stepping);
            let (restart, location) = self.restart_pc()?;
            pc = restart;
            // MEM_WRITE fires before Unicorn checks mapping and permissions.
            // A fault in that instruction is a crash, not a completed watch hit.
            if result.is_err()
                && self
                    .cpu()?
                    .get_data()
                    .watch
                    .is_some_and(|(hit, _)| hit.pc == pc & !1)
            {
                self.cpu_mut()?.get_data_mut().watch = None;
            }
            control.skip = self.cpu()?.get_data().skip_breakpoint;
            let interrupt = self.cpu()?.get_data().interrupt;
            let finished = control.finished(self, pc)?;
            let pending = self.cpu()?.get_data().stop;
            let watched = self.cpu()?.get_data().watch.is_some();
            if pending.is_some() || finished || watched {
                if self.trace.imports || !self.trace.code.is_empty() || !self.trace.calls.is_empty()
                {
                    self.suspend_traces(pc & !1)?;
                }
                let reason = if watched {
                    crate::StopReason::Watchpoint
                } else if finished || pending == Some(crate::StopReason::Finish) {
                    crate::StopReason::Finish
                } else if stepping && self.cpu()?.get_data().step_started {
                    crate::StopReason::Step
                } else {
                    crate::StopReason::Breakpoint
                };
                self.cpu_mut()?.get_data_mut().stop = None;
                pc = control.stopped(self, handler, reason)?;
                // An IT block can branch out of the hook range and only stop at
                // an interrupt. Dispatch that instruction after the debugger resumes.
                if result.is_err() || interrupt.is_none() || pc != restart {
                    continue;
                }
                stepping = control.step == Some(thread);
            }
            result?;
            let Some(interrupt) = interrupt else {
                if stepping {
                    if self.trace.imports
                        || !self.trace.code.is_empty()
                        || !self.trace.calls.is_empty()
                    {
                        self.suspend_traces(pc & !1)?;
                    }
                    if !(TRAMPOLINE..RETURN).contains(&(pc & !1)) {
                        pc = control.stopped(self, handler, crate::StopReason::Step)?;
                    }
                    continue;
                }
                if pc & !1 == RETURN {
                    if !outermost || self.threads.tid() == threads::MAIN_TID {
                        return self.reg(0);
                    }
                    return Err(format!(
                        "guest thread {} returned to RETURN at {}",
                        self.threads.tid(),
                        self.location(pc)
                    )
                    .into());
                }
                if self.trace.imports || !self.trace.code.is_empty() || !self.trace.calls.is_empty()
                {
                    self.suspend_traces(pc & !1)?;
                }
                pc = self.switch_thread(pc)?;
                continue;
            };
            if interrupt != 2 {
                return Err(self.emulation_error(uc_error::EXCEPTION));
            }
            if (TRAMPOLINE..RETURN).contains(&location) {
                let function = self.host[((location - TRAMPOLINE) / 8) as usize].clone();
                let result = self.host_function(function, handler);
                // A nested callback can exit the current child. Unwind its host
                // calls before switching, even when the nested call returned an error.
                if outermost && matches!(self.threads.action, threads::Action::Exit) {
                    pc = self.switch_thread(pc)?;
                    continue;
                }
                match result? {
                    HostResult::Return(value) => {
                        self.write_host_result(value)?;
                        if stepping {
                            pc = self.lr()?;
                            let reg = self.architecture().pc();
                            self.cpu_mut()?.reg_write(reg, pc)?;
                        }
                    }
                    HostResult::Jump(address) => {
                        pc = address;
                        let reg = self.architecture().pc();
                        self.cpu_mut()?.reg_write(reg, pc)?;
                    }
                }
            } else {
                let args = [
                    self.reg(0)?,
                    self.reg(1)?,
                    self.reg(2)?,
                    self.reg(3)?,
                    self.reg(4)?,
                    self.reg(5)?,
                ];
                let result = self.syscall(
                    self.reg(self.architecture().syscall_register())?,
                    args,
                    location,
                )?;
                self.write_reg(0, result)?;
            }
            self.finish_code(false)?;
            if stepping {
                pc = control.stopped(self, handler, crate::StopReason::Step)?;
            }
            if outermost && !matches!(self.threads.action, threads::Action::Continue) {
                pc = self.switch_thread(pc)?;
            }
        }
    }
    pub(crate) fn emulation_error(&self, error: uc_error) -> Box<dyn std::error::Error> {
        let pc = self.pc().unwrap_or(0);
        let lr = self.lr().unwrap_or(0);
        let mut message = format!(
            "emulation stopped: {error} at {} (pc=0x{pc:x}, lr=0x{lr:x})",
            self.location(pc)
        );
        if matches!(
            error,
            uc_error::READ_UNMAPPED
                | uc_error::WRITE_UNMAPPED
                | uc_error::FETCH_UNMAPPED
                | uc_error::READ_PROT
                | uc_error::WRITE_PROT
                | uc_error::FETCH_PROT
        ) && let Some(fault) = self
            .uc
            .as_ref()
            .and_then(|uc| uc.get_data().memory_access.as_ref())
        {
            let access = match fault.kind {
                MemType::READ_UNMAPPED | MemType::READ_PROT => "read",
                MemType::WRITE_UNMAPPED | MemType::WRITE_PROT => "write",
                _ => "instruction fetch",
            };
            message.push_str(&format!(
                ": {}-byte {access} at 0x{:x}",
                fault.size, fault.address
            ));
            if fault.address < TRAMPOLINE {
                message.push_str(", likely a null pointer");
            }
        }
        message.into()
    }
    fn host_function(
        &mut self,
        function: HostFunction,
        handler: &mut dyn JniHandler,
    ) -> Result<HostResult> {
        let result = match function {
            HostFunction::Import { name, original } => {
                return self.import_call(&name, original, handler);
            }
            HostFunction::Jni(index) => self.jni_function(index, handler),
            HostFunction::Vm(index) => self.vm_function(index),
            HostFunction::Property => {
                let key = self.cstring(self.reg(0)?)?;
                let value = self.properties.get(&key).cloned().unwrap_or_default();
                let out = self.reg(1)?;
                self.write_bytes(out, value.as_bytes())?;
                self.write_bytes(out + value.len() as u64, &[0])?;
                Ok(value.len() as u64)
            }
            HostFunction::Unresolved { symbol, module } => Err(format!(
                "call to unresolved symbol {symbol} imported by {module} from {}",
                self.location(self.lr()?)
            )
            .into()),
            HostFunction::Dl(name) => self.dl_function(name, handler),
            HostFunction::Asset(name) => self.asset_function(name),
        };
        result.map(HostResult::Return)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Architecture, PAGE};
    use unicorn_engine::unicorn_const::Prot;

    #[test]
    fn memory_faults_name_the_address_and_only_hint_at_low_null_pointers() -> Result<()> {
        for arch in [Architecture::Arm32, Architecture::Arm64] {
            let mut emu = Emulator::builder().build()?;
            emu.setup_cpu(arch)?;
            let code = RETURN + 0x100;
            let (words, size) = match arch {
                Architecture::Arm32 => ([0xe5900000u32, 0xe12fff1e], 4), // ldr r0,[r0]; bx lr
                Architecture::Arm64 => ([0xf9400000, 0xd65f03c0], 8),    // ldr x0,[x0]; ret
            };
            emu.write_bytes(
                code,
                &words
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
            )?;
            for (address, hint) in [(0x18, ", likely a null pointer"), (0x30000000, "")] {
                assert_eq!(
                    emu.call_function(code, &[address]).unwrap_err().to_string(),
                    format!(
                        "emulation stopped: Invalid memory read (UC_ERR_READ_UNMAPPED) at 0x20100 (pc=0x20100, lr=0x20000): {size}-byte read at 0x{address:x}{hint}"
                    )
                );
            }
            let value = emu.malloc(8)?;
            emu.write_u64(value, 42)?;
            assert_eq!(emu.call_function(code, &[value])?, 42);
        }
        Ok(())
    }

    #[test]
    fn memory_fault_hooks_cover_writes_fetches_and_protection_errors() -> Result<()> {
        for arch in [Architecture::Arm32, Architecture::Arm64] {
            let mut emu = Emulator::builder().build()?;
            emu.setup_cpu(arch)?;
            let code = RETURN + 0x100;
            let address = 0x30000000;
            let (read, write, trap, size) = match arch {
                Architecture::Arm32 => (0xe5900000u32, 0xe5800000u32, 0xe1200070u32, 4),
                Architecture::Arm64 => (0xf9400000, 0xf9000000, 0xd4200000, 8),
            };
            emu.write_bytes(code, &write.to_le_bytes())?;
            assert_eq!(
                emu.call_function(code, &[address]).unwrap_err().to_string(),
                format!(
                    "emulation stopped: Invalid memory write (UC_ERR_WRITE_UNMAPPED) at 0x20100 (pc=0x20100, lr=0x20000): {size}-byte write at 0x30000000"
                )
            );
            assert_eq!(
                emu.call_function(address, &[]).unwrap_err().to_string(),
                "emulation stopped: Invalid memory fetch (UC_ERR_FETCH_UNMAPPED) at 0x30000000 (pc=0x30000000, lr=0x20000): 4-byte instruction fetch at 0x30000000"
            );
            emu.cpu_mut()?.mem_map(address, PAGE, Prot::READ)?;
            assert_eq!(
                emu.call_function(code, &[address]).unwrap_err().to_string(),
                format!(
                    "emulation stopped: Write to write-protected memory (UC_ERR_WRITE_PROT) at 0x20100 (pc=0x20100, lr=0x20000): {size}-byte write at 0x30000000"
                )
            );
            assert_eq!(
                emu.call_function(address, &[]).unwrap_err().to_string(),
                "emulation stopped: Fetch from non-executable memory (UC_ERR_FETCH_PROT) at 0x30000000 (pc=0x30000000, lr=0x20000): 4-byte instruction fetch at 0x30000000"
            );
            emu.cpu_mut()?.mem_protect(address, PAGE, Prot::WRITE)?;
            emu.write_bytes(code, &read.to_le_bytes())?;
            assert_eq!(
                emu.call_function(code, &[address]).unwrap_err().to_string(),
                format!(
                    "emulation stopped: Read from non-readable memory (UC_ERR_READ_PROT) at 0x20100 (pc=0x20100, lr=0x20000): {size}-byte read at 0x30000000"
                )
            );
            emu.write_bytes(code, &trap.to_le_bytes())?;
            assert_eq!(
                emu.call_function(code, &[]).unwrap_err().to_string(),
                "emulation stopped: Unhandled CPU exception (UC_ERR_EXCEPTION) at 0x20100 (pc=0x20100, lr=0x20000)"
            );
        }
        Ok(())
    }
}
