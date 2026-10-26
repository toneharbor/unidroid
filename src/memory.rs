use crate::{CpuData, Emulator, Region, Result, arch::Architecture, page_up};
use unicorn_engine::{
    RegisterARM, UcHookId, Unicorn,
    unicorn_const::{HookType, MemType, Prot, uc_error},
};

impl Region {
    pub(super) fn split_around(self, start: u64, end: u64) -> (Option<Self>, Option<Self>) {
        let left = (self.start < start).then(|| Self {
            end: start,
            ..self.clone()
        });
        let right = (end < self.end).then_some(Self { start: end, ..self });
        (left, right)
    }
}

impl Emulator {
    pub(crate) fn memory_hook(
        &mut self,
        kind: HookType,
        begin: u64,
        end: u64,
        mut callback: impl FnMut(&mut Unicorn<'_, CpuData>, MemType, u64, usize, i64) -> bool + 'static,
    ) -> Result<UcHookId> {
        let arm32 = self.architecture() == Architecture::Arm32;
        Ok(self.cpu_mut()?.add_mem_hook(
            kind,
            begin,
            end,
            move |cpu, kind, address, size, value| {
                // Store hooks receive the source register, including bits beyond
                // a narrow store. Expose only the bytes actually transferred.
                let value = ((value as u64) & (u64::MAX >> (64 - size * 8))) as i64;
                let result = callback(cpu, kind, address, size, value);
                if arm32 {
                    // Unicorn 2.1.5 restores PC and IT bits before memory callbacks.
                    // Translated Thumb code tracks IT locally and only writes back
                    // nonzero state at a block exit. Clear the callback's restored
                    // bits so a completed IT block cannot predicate the next block.
                    // A partial IT block or a fault restores its own state on exit.
                    let cpsr = cpu.reg_read(RegisterARM::CPSR).expect("status register");
                    if cpsr & 0x0600_fc00 != 0 {
                        cpu.reg_write(RegisterARM::CPSR, cpsr & !0x0600_fc00)
                            .expect("status register");
                    }
                }
                result
            },
        )?)
    }

    pub fn malloc(&mut self, size: usize) -> Result<u64> {
        self.cpu()?;
        let size = (size.max(1) as u64 + 15) & !15;
        let address = if let Some(index) = self
            .free_blocks
            .iter()
            .position(|(_, block)| *block >= size)
        {
            let (address, block) = self.free_blocks.remove(index);
            if block > size {
                self.free_blocks.push((address + size, block - size));
            }
            address
        } else {
            let address = self.heap;
            self.heap = self.heap.checked_add(size).ok_or(uc_error::NOMEM)?;
            if self.heap > self.heap_end {
                let end = page_up(self.heap);
                let start = self.heap_end;
                self.cpu_mut()?
                    .mem_map(start, end - start, Prot::READ | Prot::WRITE)?;
                self.heap_end = end;
            }
            self.heap_high_water = self.heap_high_water.max(self.heap);
            address
        };
        self.allocations.insert(address, size);
        Ok(address)
    }
    pub fn free(&mut self, ptr: u64) {
        let Some(size) = self.allocations.remove(&ptr) else {
            return;
        };
        self.free_blocks.push((ptr, size));
        self.free_blocks
            .sort_unstable_by_key(|(address, _)| *address);
        let mut merged = Vec::with_capacity(self.free_blocks.len());
        for (address, size) in self.free_blocks.drain(..) {
            if let Some((last, length)) = merged.last_mut()
                && *last + *length == address
            {
                *length += size;
                continue;
            }
            merged.push((address, size));
        }
        self.free_blocks = merged;
    }
    pub fn heap_high_water_mark(&self) -> u64 {
        self.heap_high_water
    }
    pub fn new_cstring(&mut self, value: &str) -> Result<u64> {
        let ptr = self.malloc(value.len() + 1)?;
        self.write_bytes(ptr, value.as_bytes())?;
        self.write_bytes(ptr + value.len() as u64, &[0])?;
        Ok(ptr)
    }
    pub fn read_bytes(&self, address: u64, size: usize) -> Result<Vec<u8>> {
        Ok(self.cpu()?.mem_read_as_vec(address, size)?)
    }
    pub fn write_bytes(&mut self, address: u64, bytes: &[u8]) -> Result<()> {
        // Unicorn's host write API bypasses guest permissions. Never let it
        // write through a read-only host mmap; guest mprotect privatizes first.
        if self.regions.iter().any(|r| {
            r.shared.is_some() && address < r.end && address + bytes.len() as u64 > r.start
        }) {
            return Err(uc_error::WRITE_PROT.into());
        }
        self.cpu_mut()?.mem_write(address, bytes)?;
        if !bytes.is_empty() {
            self.cpu_mut()?
                .ctl_remove_cache(address, address + bytes.len() as u64)?;
        }
        Ok(())
    }
    pub(super) fn read_u64(&self, address: u64) -> Result<u64> {
        Ok(u64::from_le_bytes(
            self.read_bytes(address, 8)?.try_into().unwrap(),
        ))
    }
    pub(super) fn write_u64(&mut self, address: u64, value: u64) -> Result<()> {
        self.write_bytes(address, &value.to_le_bytes())
    }
    pub(super) fn cstring(&self, address: u64) -> Result<String> {
        let mut value = Vec::new();
        let mut p = address;
        loop {
            let byte = self.read_bytes(p, 1)?[0];
            if byte == 0 {
                break;
            }
            value.push(byte);
            p += 1;
        }
        Ok(String::from_utf8_lossy(&value).into_owned())
    }
    pub(super) fn protect(&mut self, start: u64, size: u64, permissions: Prot) -> Result<()> {
        let end = start + page_up(size);
        let mut regions = Vec::new();
        for region in std::mem::take(&mut self.regions) {
            let lo = start.max(region.start);
            let hi = end.min(region.end);
            if lo < hi && region.shared.is_some() && permissions.0 & Prot::WRITE.0 != 0 {
                let bytes = self.read_bytes(lo, (hi - lo) as usize)?;
                self.cpu_mut()?.mem_unmap(lo, hi - lo)?;
                self.cpu_mut()?.mem_map(lo, hi - lo, permissions)?;
                self.write_bytes(lo, &bytes)?;
                let (left, right) = region.split_around(lo, hi);
                regions.extend(left);
                regions.push(Region {
                    start: lo,
                    end: hi,
                    shared: None,
                });
                regions.extend(right);
            } else {
                regions.push(region);
            }
        }
        self.regions = regions;
        self.cpu_mut()?
            .mem_protect(start, end - start, permissions)?;
        Ok(())
    }
}
