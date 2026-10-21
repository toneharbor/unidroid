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
        self.threads.depth += 1;
        let result = self
            .run_loop(pc, handler)
            .map_err(|error| self.crash(error));
        self.threads.depth -= 1;
        if self.threads.depth == 0 && result.is_err() {
            self.restore_main_thread()?;
        }
        result
    }
    fn run_loop(&mut self, mut pc: u64, handler: &mut dyn JniHandler) -> Result<u64> {
        let outermost = self.threads.depth == 1;
        loop {
            let slice_remaining = if outermost && self.threads.has_runnable() {
                threads::SLICE_CODE_BYTES
            } else {
                0
            };
            let thread = self.threads.tid();
            *self.cpu_mut()?.get_data_mut() = CpuData {
                slice_remaining,
                thread,
                ..CpuData::default()
            };
            if let Err(error) = self.cpu_mut()?.emu_start(pc, RETURN, 0, 0) {
                return Err(self.emulation_error(error));
            }
            let (restart, location) = self.restart_pc()?;
            pc = restart;
            let Some(interrupt) = self.cpu()?.get_data().interrupt else {
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
                if self.trace.imports {
                    self.suspend_import(pc & !1)?;
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
                self.write_host_result(result?)?;
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
            if outermost && !matches!(self.threads.action, threads::Action::Continue) {
                pc = self.switch_thread(pc)?;
            }
        }
    }
    fn emulation_error(&self, error: uc_error) -> Box<dyn std::error::Error> {
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
    ) -> Result<u64> {
        match function {
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
            HostFunction::Dl(name) => self.dl_function(name),
        }
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
