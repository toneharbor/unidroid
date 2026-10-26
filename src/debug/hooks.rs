use crate::{CpuData, Emulator, Result, arch::Architecture};
use unicorn_engine::{UcHookId, Unicorn};

pub(super) fn remove_hooks(
    cpu: &mut Unicorn<'_, CpuData>,
    hooks: impl IntoIterator<Item = UcHookId>,
) -> Result<()> {
    let mut removed = false;
    for hook in hooks {
        cpu.remove_hook(hook)?;
        removed = true;
    }
    if removed {
        cpu.ctl_flush_tb()?;
    }
    Ok(())
}

impl Emulator {
    fn address_hook_end(&self, address: u64) -> u64 {
        // Thumb IT blocks can defer emu_stop through four 32-bit instructions.
        // Leave room for the exclusive cache endpoint, even at the address limit.
        address.saturating_add(if self.arch == Some(Architecture::Arm32) {
            16
        } else {
            0
        }) & !1
    }

    pub(super) fn install_address_hook(
        &mut self,
        address: u64,
        callback: impl FnMut(&mut Unicorn<'_, CpuData>, u64, u32) + 'static,
    ) -> Result<UcHookId> {
        let end = self.address_hook_end(address);
        let cpu = self.cpu_mut()?;
        cpu.ctl_remove_cache(address, end + 1)?;
        Ok(cpu.add_code_hook(address, end, callback)?)
    }

    pub(super) fn remove_address_hook(&mut self, address: u64, hook: UcHookId) -> Result<()> {
        let end = self.address_hook_end(address);
        let cpu = self.cpu_mut()?;
        cpu.ctl_remove_cache(address, end + 1)?;
        Ok(cpu.remove_hook(hook)?)
    }
}
