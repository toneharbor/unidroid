use super::expr::Condition;
use crate::{Emulator, Result};
use unicorn_engine::UcHookId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    Breakpoint,
    Watchpoint,
    Step,
    Finish,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stop {
    pub reason: StopReason,
    pub address: u64,
    pub thread: u32,
    pub watch: Option<crate::WatchHit>,
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
            emu.remove_address_hook(target.address, hook)?;
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
        self.skip = (reason != StopReason::Watchpoint).then_some((thread, address));
        let watch = emu.cpu_mut()?.get_data_mut().watch.take();
        let resume = if let Some((hit, crate::WatchAction::Log)) = watch {
            emu.log_watch(hit)?;
            Resume::Continue
        } else {
            handler.stopped(
                emu,
                &Stop {
                    reason,
                    address,
                    thread,
                    watch: watch.map(|(hit, _)| hit),
                },
            )
        };
        match resume {
            Resume::Continue => {}
            Resume::Step => self.step = Some(thread),
            Resume::Finish => {
                let target = FinishTarget {
                    address: emu.lr()? & !1,
                    thread,
                    sp: emu.sp()?,
                };
                let sp = emu.architecture().sp();
                let hook = emu.install_address_hook(target.address, move |cpu, pc, _| {
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
        // Run tracepoints after stop hooks and after the preceding instruction's
        // trace is emitted. Stops then cannot log a hit twice on Continue; Thumb
        // IT instructions whose stop is deferred still reach their tracepoint.
        self.refresh_tracepoints()?;
        let cpu = self.cpu_mut()?;
        let result = cpu.emu_start(pc, crate::RETURN, 0, 0);
        let source_error = cpu.get_data_mut().source_error.take();
        super::hooks::remove_hooks(cpu, hook)?;
        self.settle_code(result.is_err() || source_error.is_some())?;
        if let Some((pc, error)) = source_error {
            return Err(format!("{error} from {}", self.location(pc)).into());
        }
        result.map_err(|error| self.emulation_error(error))
    }

    pub fn add_breakpoint(&mut self, address: u64) -> Result<()> {
        self.breakpoint(address, None)
    }

    pub fn add_conditional_breakpoint(&mut self, address: u64, condition: &str) -> Result<()> {
        self.cpu()?;
        self.breakpoint(address, Some(Condition::parse(self, condition)?))
    }

    fn breakpoint(&mut self, address: u64, condition: Option<Condition>) -> Result<()> {
        let address = address & !1;
        if self.breakpoints.contains_key(&address) {
            self.remove_breakpoint(address)?;
        }
        let hook = self.install_address_hook(address, move |cpu, pc, _| {
            let data = cpu.get_data();
            if pc == address {
                if data.skip_breakpoint == Some((data.thread, address)) {
                    cpu.get_data_mut().skip_breakpoint = None;
                } else if condition
                    .as_ref()
                    .is_none_or(|condition| condition.holds(cpu))
                {
                    cpu.get_data_mut().stop = Some(StopReason::Breakpoint);
                }
            }
            if cpu.get_data().stop.is_some() {
                let _ = cpu.emu_stop();
            }
        })?;
        self.breakpoints.insert(address, hook);
        Ok(())
    }

    pub fn remove_breakpoint(&mut self, address: u64) -> Result<()> {
        let address = address & !1;
        let hook = self
            .breakpoints
            .remove(&address)
            .ok_or_else(|| format!("no breakpoint at {}", self.symbolize(address)))?;
        self.remove_address_hook(address, hook)
    }
}
