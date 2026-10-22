use crate::{Emulator, Result};
use unicorn_engine::UcHookId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    Breakpoint,
    Step,
    Finish,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stop {
    pub reason: StopReason,
    pub address: u64,
    pub thread: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resume {
    Continue,
    Step,
    Finish,
    Return(u64),
    Abort,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct FinishTarget {
    address: u64,
    thread: u32,
    sp: u64,
}

#[derive(Default)]
pub(crate) struct RunControl {
    pub skip: Option<(u32, u64)>,
    pub step: Option<u32>,
    finish: Option<(FinishTarget, UcHookId)>,
}

impl RunControl {
    pub fn finish_target(&self) -> Option<FinishTarget> {
        self.finish.as_ref().map(|(target, _)| *target)
    }

    pub fn finished(&self, emu: &Emulator, pc: u64) -> Result<bool> {
        let Some(target) = self.finish_target() else {
            return Ok(false);
        };
        Ok(target.address == pc & !1
            && target.thread == emu.threads.tid()
            && emu.sp()? >= target.sp)
    }

    pub fn clear_finish(&mut self, emu: &mut Emulator) -> Result<()> {
        if let Some((target, hook)) = self.finish.take() {
            let end = emu.debug_hook_end(target.address);
            let cpu = emu.cpu_mut()?;
            cpu.remove_hook(hook)?;
            cpu.ctl_remove_cache(target.address, end + 1)?;
        }
        Ok(())
    }

    pub fn stopped(
        &mut self,
        emu: &mut Emulator,
        handler: &mut dyn crate::JniHandler,
        reason: StopReason,
    ) -> Result<u64> {
        let address = emu.pc()? & !1;
        let thread = emu.threads.tid();
        self.step = None;
        self.clear_finish(emu)?;
        self.skip = Some((thread, address));
        match handler.stopped(
            emu,
            &Stop {
                reason,
                address,
                thread,
            },
        ) {
            Resume::Continue => {}
            Resume::Step => self.step = Some(thread),
            Resume::Finish => {
                let target = FinishTarget {
                    address: emu.lr()? & !1,
                    thread,
                    sp: emu.sp()?,
                };
                let sp = emu.architecture().sp();
                let end = emu.debug_hook_end(target.address);
                let cpu = emu.cpu_mut()?;
                let hook = cpu.add_code_hook(target.address, end, move |cpu, pc, _| {
                    let data = cpu.get_data();
                    if pc == target.address
                        && data.finish == Some(target)
                        && data.thread == target.thread
                        && cpu.reg_read(sp).expect("stack pointer") >= target.sp
                    {
                        cpu.get_data_mut().stop = Some(StopReason::Finish);
                    }
                    if cpu.get_data().stop.is_some() {
                        let _ = cpu.emu_stop();
                    }
                })?;
                cpu.ctl_remove_cache(target.address, end + 1)?;
                self.finish = Some((target, hook));
            }
            Resume::Return(bits) => {
                emu.write_host_result(bits)?;
                let pc = emu.lr()?;
                let reg = emu.architecture().pc();
                emu.cpu_mut()?.reg_write(reg, pc)?;
                self.skip = None;
            }
            Resume::Abort => {
                emu.discard_suspended_import(address);
                return Err(
                    format!("stopped by the debugger at {}", emu.symbolize(address)).into(),
                );
            }
        }
        let pc = emu.restart_pc()?.0;
        if pc & !1 != address {
            emu.discard_suspended_import(address);
            self.skip = None;
        }
        Ok(pc)
    }
}

impl Emulator {
    fn debug_hook_end(&self, address: u64) -> u64 {
        // Thumb IT blocks can defer emu_stop through four 32-bit instructions.
        address
            + if self.arch == Some(crate::arch::Architecture::Arm32) {
                16
            } else {
                0
            }
    }

    pub(crate) fn run_cpu(&mut self, pc: u64, stepping: bool) -> Result<()> {
        let cpu = self.cpu_mut()?;
        let hook = if stepping {
            let hook = cpu.add_code_hook(1, 0, |cpu, _, _| {
                let data = cpu.get_data_mut();
                if data.step_started {
                    let _ = cpu.emu_stop();
                } else {
                    data.step_started = true;
                }
            })?;
            cpu.ctl_flush_tb()?;
            Some(hook)
        } else {
            None
        };
        let result = cpu.emu_start(pc, crate::RETURN, 0, 0);
        if let Some(hook) = hook {
            cpu.remove_hook(hook)?;
            cpu.ctl_flush_tb()?;
        }
        self.settle_code(result.is_err())?;
        result.map_err(|error| self.emulation_error(error))
    }

    pub fn add_breakpoint(&mut self, address: u64) -> Result<()> {
        let address = address & !1;
        if self.breakpoints.contains_key(&address) {
            return Ok(());
        }
        let end = self.debug_hook_end(address);
        let cpu = self.cpu_mut()?;
        let hook = cpu.add_code_hook(address, end, move |cpu, pc, _| {
            let data = cpu.get_data_mut();
            if pc == address {
                if data.skip_breakpoint == Some((data.thread, address)) {
                    data.skip_breakpoint = None;
                } else {
                    data.stop = Some(StopReason::Breakpoint);
                }
            }
            if data.stop.is_some() {
                let _ = cpu.emu_stop();
            }
        })?;
        cpu.ctl_remove_cache(address, end + 1)?;
        self.breakpoints.insert(address, hook);
        Ok(())
    }

    pub fn remove_breakpoint(&mut self, address: u64) -> Result<()> {
        let address = address & !1;
        let hook = self
            .breakpoints
            .remove(&address)
            .ok_or_else(|| format!("no breakpoint at {}", self.symbolize(address)))?;
        let end = self.debug_hook_end(address);
        let cpu = self.cpu_mut()?;
        cpu.remove_hook(hook)?;
        cpu.ctl_remove_cache(address, end + 1)?;
        Ok(())
    }
}
