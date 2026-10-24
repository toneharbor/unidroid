use crate::{Emulator, MMAP_END, MMAP_START, Prot, Region, Result, page_up};
mod files;
mod output;
mod procfs;
mod system;
impl Emulator {
    pub(crate) fn syscall(&mut self, guest_nr: u64, mut a: [u64; 6], pc: u64) -> Result<u64> {
        let nr = self
            .architecture()
            .syscall(guest_nr, &mut a)
            .unwrap_or(u64::MAX);
        let event = if self.trace.syscalls {
            Some(self.syscall_event(guest_nr, nr, a, pc, self.lr()?))
        } else {
            None
        };
        let result = self.dispatch_syscall(guest_nr, nr, a, pc);
        if let Some(event) = event {
            let returned = result
                .as_ref()
                .ok()
                .copied()
                .filter(|_| matches!(self.threads.action, crate::threads::Action::Continue));
            self.emit_syscall(event, returned);
        }
        result
    }
    fn dispatch_syscall(&mut self, guest_nr: u64, nr: u64, a: [u64; 6], pc: u64) -> Result<u64> {
        let errno = |n: i64| (-n) as u64;
        Ok(match nr {
            23 | 24 | 25 | 34 | 35 | 38 | 46 | 48 | 56 | 57 | 61 | 62 | 63 | 67 | 68 | 78 | 79
            | 80 | 82 | 83 | 276 => self.file_syscall(nr, a)?,
            140 if self.architecture() == crate::arch::Architecture::Arm32 => {
                self.file_syscall(nr, a)?
            }
            64 => self.write_output(a[0], a[1], a[2]),
            66 => self.writev_output(a[0], a[1], a[2]),
            160 => self.uname(a[0]),
            167 => match a[0] {
                0x53564d41 => 0,
                15 => {
                    let mut bytes = Vec::new();
                    for i in 0..16 {
                        let byte = self.read_bytes(a[1] + i, 1).ok().map_or(0, |v| v[0]);
                        if byte == 0 {
                            break;
                        }
                        bytes.push(byte);
                    }
                    self.process_name = String::from_utf8_lossy(&bytes).into_owned();
                    0
                }
                16 => {
                    let bytes = self.process_name.as_bytes().to_vec();
                    let length = bytes.len().min(15);
                    if self.write_bytes(a[1], &bytes[..length]).is_err()
                        || self.write_bytes(a[1] + length as u64, &[0]).is_err()
                    {
                        errno(14)
                    } else {
                        0
                    }
                }
                _ => errno(22),
            },
            93 | 94 => self.exit_thread(a[0], nr == 94, pc)?,
            96 => self.threads.set_tid_address(a[0]),
            98 => self.futex(a, pc)?,
            101 | 115 | 124 => {
                if self.threads.depth == 1 && self.threads.has_runnable() {
                    self.threads.action = crate::threads::Action::Yield;
                }
                0
            }
            99 | 132 | 135 | 233 => 0,
            172 => crate::threads::MAIN_TID as u64,
            178 => self.threads.tid() as u64,
            220 => self.clone_thread(a)?,
            113 | 169 => {
                let clock = if nr == 169 || a[0] == 0 {
                    crate::source::Clock::Realtime
                } else {
                    crate::source::Clock::Monotonic
                };
                let time = self.source.time(clock)?;
                let out = if nr == 169 { a[0] } else { a[1] };
                if out != 0 {
                    self.write_pointer(out, time.sec as u64)?;
                    self.write_pointer(
                        out + self.pointer_size(),
                        (if nr == 169 {
                            time.nsec / 1000
                        } else {
                            time.nsec
                        }) as u64,
                    )?;
                }
                if nr == 169 && a[1] != 0 {
                    self.write_u64(a[1], 0)?;
                }
                0
            }
            278 => {
                let mut buffer = vec![0; a[1].min(crate::vfs::IO_LIMIT) as usize];
                let mut count = 0;
                loop {
                    count += self.source.random(&mut buffer[count..])?;
                    if count == buffer.len() {
                        break;
                    }
                }
                self.write_bytes(a[0], &buffer)?;
                count as u64
            }
            174..=177 => 10000,
            214 => {
                if a[0] < crate::BRK_START {
                    self.brk
                } else {
                    let old_end = page_up(self.brk);
                    let new_end = page_up(a[0]);
                    if new_end > old_end {
                        if self
                            .cpu_mut()?
                            .mem_map(old_end, new_end - old_end, Prot::READ | Prot::WRITE)
                            .is_err()
                        {
                            return Ok(errno(12));
                        } else {
                            self.brk = a[0];
                            self.brk
                        }
                    } else if new_end < old_end {
                        if self.unmap(new_end, old_end - new_end).is_err() {
                            self.brk
                        } else {
                            self.brk = a[0];
                            self.brk
                        }
                    } else {
                        self.brk = a[0];
                        self.brk
                    }
                }
            }
            215 => {
                if self.unmap(a[0], page_up(a[1])).is_err() {
                    errno(22)
                } else {
                    0
                }
            }
            222 => {
                let length = page_up(a[1]);
                if length == 0 {
                    return Ok(errno(22));
                }
                let fixed = a[3] & 16 != 0;
                let address = if fixed {
                    if self.unmap(a[0], length).is_err() {
                        return Ok(errno(22));
                    }
                    a[0]
                } else {
                    match self.lowest_mmap_gap(length) {
                        Some(address) => address,
                        None => return Ok(errno(12)),
                    }
                };
                if self
                    .cpu_mut()?
                    .mem_map(address, length, Prot(a[2] as u32))
                    .is_err()
                {
                    return Ok(errno(12));
                }
                if a[3] & 32 == 0 {
                    let bytes = match self.read_file_bytes(a[4], length, Some(a[5])) {
                        Ok(bytes) => bytes,
                        Err(error) => {
                            let _ = self.cpu_mut()?.mem_unmap(address, length);
                            return match error.downcast::<crate::vfs::Error>() {
                                Ok(error) => Ok(error.result()),
                                Err(error) => Err(error),
                            };
                        }
                    };
                    let n = bytes.len();
                    if self.write_bytes(address, &bytes[..n]).is_err() {
                        let _ = self.cpu_mut()?.mem_unmap(address, length);
                        return Ok(errno(14));
                    }
                }
                self.regions.push(Region {
                    start: address,
                    end: address + length,
                    shared: None,
                });
                address
            }
            226 => {
                self.protect(a[0], a[1], Prot(a[2] as u32))?;
                0
            }
            _ => {
                self.events.borrow_mut().emit(crate::Event::Warning {
                    message: format!(
                        "unsupported syscall {guest_nr} at {}, returning -ENOSYS",
                        self.location(pc)
                    ),
                });
                errno(38)
            }
        })
    }
    fn lowest_mmap_gap(&self, length: u64) -> Option<u64> {
        let mut cursor = crate::MMAP_START;
        let mut regions = self.cpu().ok()?.mem_regions().ok()?;
        regions.sort_unstable_by_key(|region| region.begin);
        for region in regions {
            if region.end < MMAP_START || region.begin >= MMAP_END {
                continue;
            }
            let begin = region.begin.max(MMAP_START);
            let end = region.end.saturating_add(1).min(MMAP_END);
            if begin > cursor && begin - cursor >= length {
                return Some(cursor);
            }
            cursor = cursor.max(end);
            if cursor + length > MMAP_END {
                return None;
            }
        }
        (cursor + length <= MMAP_END).then_some(cursor)
    }

    pub(super) fn unmap(&mut self, start: u64, size: u64) -> Result<()> {
        let end = start + size;
        for region in self.cpu()?.mem_regions()? {
            let lo = start.max(region.begin);
            let hi = end.min(region.end + 1);
            if lo < hi {
                self.cpu_mut()?.mem_unmap(lo, hi - lo)?;
            }
        }
        let mut keep = Vec::new();
        for region in std::mem::take(&mut self.regions) {
            if region.end <= start || region.start >= end {
                keep.push(region);
            } else {
                if region.start < start {
                    keep.push(Region {
                        end: start,
                        ..region.clone()
                    });
                }
                if region.end > end {
                    keep.push(Region {
                        start: end,
                        ..region
                    });
                }
            }
        }
        self.regions = keep;
        Ok(())
    }
}
#[cfg(test)]
mod tests;
