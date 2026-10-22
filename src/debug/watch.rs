use crate::{Emulator, Result};
use serde::Deserialize;
use std::collections::HashMap;
use unicorn_engine::{
    UcHookId,
    unicorn_const::{HookType, MemType},
};

// Unicorn's ARM load/store helpers report at most 8 bytes per callback;
// paired and 128-bit vector accesses are split into separate callbacks.
const MAX_ACCESS_SIZE: u64 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Watch {
    Read,
    Write,
    Access,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WatchHit {
    pub pc: u64,
    pub address: u64,
    pub size: usize,
    pub write: bool,
    pub value: u64,
}

#[derive(Default)]
pub(crate) struct Watchpoints {
    ranges: HashMap<u64, UcHookId>,
    boundary: Option<UcHookId>,
}

impl Emulator {
    pub fn add_watchpoint(&mut self, address: u64, size: u64, watch: Watch) -> Result<()> {
        if size == 0 {
            return Err("empty watchpoint".into());
        }
        self.cpu()?;
        let pc = self.architecture().pc();
        let last = address.saturating_add(size - 1);
        let kind = match watch {
            Watch::Read => HookType::MEM_READ_AFTER,
            Watch::Write => HookType::MEM_WRITE,
            Watch::Access => HookType::MEM_READ_AFTER | HookType::MEM_WRITE,
        };
        let hook = self.cpu_mut()?.add_mem_hook(
            kind,
            address.saturating_sub(MAX_ACCESS_SIZE - 1),
            last,
            move |cpu, kind, at, size, value| {
                if cpu.get_data().watch.is_none()
                    && at <= last
                    && u128::from(at) + size as u128 > u128::from(address)
                {
                    let pc = cpu.reg_read(pc).expect("program counter") & !1;
                    let data = cpu.get_data_mut();
                    data.watch = Some(WatchHit {
                        pc,
                        address: at,
                        size,
                        write: kind == MemType::WRITE,
                        value: value as u64,
                    });
                }
                true
            },
        )?;
        if let Some(previous) = self.watchpoints.ranges.insert(address, hook) {
            self.cpu_mut()?.remove_hook(previous)?;
        }
        if self.watchpoints.boundary.is_none() {
            // Memory callbacks run inside an instruction. Let all of its accesses
            // finish, then stop at the next code hook (after an IT block on Thumb).
            self.watchpoints.boundary =
                Some(self.cpu_mut()?.add_code_hook(1, 0, |cpu, _, _| {
                    if cpu.get_data().watch.is_some() {
                        let _ = cpu.emu_stop();
                    }
                })?);
        }
        // The affected instructions can be anywhere, not just in the data range.
        self.cpu_mut()?.ctl_flush_tb()?;
        Ok(())
    }

    pub fn remove_watchpoint(&mut self, address: u64) -> Result<()> {
        let hook = self
            .watchpoints
            .ranges
            .remove(&address)
            .ok_or_else(|| format!("no watchpoint at {}", self.symbolize(address)))?;
        self.cpu_mut()?.remove_hook(hook)?;
        if self.watchpoints.ranges.is_empty()
            && let Some(hook) = self.watchpoints.boundary.take()
        {
            self.cpu_mut()?.remove_hook(hook)?;
        }
        self.cpu_mut()?.ctl_flush_tb()?;
        Ok(())
    }
}
