use crate::{Emulator, PAGE, Result, SharedMapping};
use std::sync::Arc;
use unicorn_engine::unicorn_const::Prot;

struct Layout {
    start: u64,
    end: u64,
    perms: Prot,
    shared: Option<SharedMapping>,
}
impl Layout {
    fn same_mapping(&self, other: &Self) -> bool {
        self.start == other.start
            && self.end == other.end
            && match (&self.shared, &other.shared) {
                (None, None) => true,
                (Some(a), Some(b)) => {
                    Arc::ptr_eq(&a.image, &b.image) && a.offset(self.start) == b.offset(other.start)
                }
                _ => false,
            }
    }

    fn current(emu: &Emulator) -> Result<Vec<Self>> {
        Ok(emu
            .cpu()?
            .mem_regions()?
            .into_iter()
            .map(|region| {
                let shared = emu
                    .regions
                    .iter()
                    .find(|r| r.start <= region.begin && r.end > region.begin)
                    .and_then(|r| r.shared.clone());
                Self {
                    start: region.begin,
                    end: region.end + 1,
                    perms: Prot(region.perms),
                    shared,
                }
            })
            .collect())
    }
}

pub(super) struct Memory {
    layout: Layout,
    bytes: Vec<u8>,
}
impl Memory {
    pub fn capture(emu: &Emulator) -> Result<Vec<Self>> {
        Layout::current(emu)?
            .into_iter()
            .map(|layout| {
                let bytes = if layout.shared.is_some() {
                    Vec::new()
                } else {
                    emu.read_bytes(layout.start, (layout.end - layout.start) as usize)?
                };
                Ok(Self { layout, bytes })
            })
            .collect()
    }

    pub fn restore(emu: &mut Emulator, saved: &[Self]) -> Result<()> {
        let current = Layout::current(emu)?;
        for old in &current {
            if !saved.iter().any(|saved| old.same_mapping(&saved.layout)) {
                emu.cpu_mut()?.ctl_remove_cache(old.start, old.end)?;
                emu.unmap(old.start, old.end - old.start)?;
            }
        }
        let mut page = [0; PAGE as usize];
        for saved in saved {
            let layout = &saved.layout;
            let old = current.iter().find(|old| old.same_mapping(layout));
            if let Some(old) = old {
                if old.perms != layout.perms {
                    emu.cpu_mut()?.ctl_remove_cache(layout.start, layout.end)?;
                    emu.cpu_mut()?.mem_protect(
                        layout.start,
                        layout.end - layout.start,
                        layout.perms,
                    )?;
                }
            } else {
                emu.cpu_mut()?.ctl_remove_cache(layout.start, layout.end)?;
                if let Some(shared) = &layout.shared {
                    // Both the checkpoint and restored regions retain this image.
                    unsafe {
                        emu.cpu_mut()?.mem_map_ptr(
                            layout.start,
                            layout.end - layout.start,
                            layout.perms,
                            shared
                                .image
                                .bytes
                                .as_ptr()
                                .add(shared.offset(layout.start))
                                .cast_mut()
                                .cast(),
                        )?;
                    }
                    emu.regions.push(crate::Region {
                        start: layout.start,
                        end: layout.end,
                        shared: Some(shared.clone()),
                    });
                } else {
                    emu.cpu_mut()?.mem_map(
                        layout.start,
                        layout.end - layout.start,
                        layout.perms,
                    )?;
                }
            }
            for (i, bytes) in saved.bytes.chunks(PAGE as usize).enumerate() {
                let address = layout.start + i as u64 * PAGE;
                emu.cpu()?.mem_read(address, &mut page[..bytes.len()])?;
                if page[..bytes.len()] != *bytes {
                    emu.write_bytes(address, bytes)?;
                }
            }
        }
        Ok(())
    }
}
