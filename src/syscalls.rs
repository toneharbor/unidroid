use crate::{Emulator, MMAP_END, MMAP_START, Prot, Region, Result, page_up};
use std::{fs::File, io::SeekFrom, path::Component};
mod files;
mod output;
mod procfs;
mod system;
pub(crate) use files::{FileState, GuestFile};
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
            48 => self.faccessat(a[1])?,
            56 => {
                if a[2] & 3 != 0 {
                    return Ok(errno(13));
                }
                let path = self.cstring(a[1])?;
                match self.open_guest(&path) {
                    Ok(file) => {
                        let fd = self.next_fd;
                        self.next_fd += 1;
                        self.files.insert(fd, file);
                        fd
                    }
                    Err(_) => errno(2),
                }
            }
            57 => {
                if self.files.remove(&a[0]).is_some() {
                    0
                } else {
                    errno(9)
                }
            }
            62 => {
                let Some(file) = self.files.get_mut(&a[0]) else {
                    return Ok(errno(9));
                };
                let from = match a[2] {
                    0 => SeekFrom::Start(a[1]),
                    1 => SeekFrom::Current(a[1] as i64),
                    2 => SeekFrom::End(a[1] as i64),
                    _ => return Ok(errno(22)),
                };
                file.seek(from).unwrap_or_else(|_| errno(22))
            }
            63 | 67 => {
                let Some(file) = self.files.get_mut(&a[0]) else {
                    return Ok(errno(9));
                };
                let position = if nr == 67 {
                    Some(file.stream_position()?)
                } else {
                    None
                };
                if nr == 67 {
                    file.seek(SeekFrom::Start(a[3]))?;
                }
                let mut buffer = vec![0; file.read_length(a[2])?];
                let random = matches!(file, GuestFile::Random { .. });
                let result = self.read_file(a[0], &mut buffer);
                if let Some(position) = position {
                    self.files
                        .get_mut(&a[0])
                        .unwrap()
                        .seek(SeekFrom::Start(position))?;
                }
                match result {
                    Ok(n) => {
                        self.write_bytes(a[1], &buffer[..n])?;
                        n as u64
                    }
                    Err(error) if random => return Err(error),
                    Err(_) => errno(5),
                }
            }
            64 => self.write_output(a[0], a[1], a[2]),
            66 => self.writev_output(a[0], a[1], a[2]),
            79 | 80 => {
                let stat = |file: &GuestFile| {
                    file.stat(
                        self.architecture(),
                        self.source.shared.borrow().state.fixed_clock,
                    )
                    .ok()
                };
                let metadata = if nr == 80 {
                    self.files.get(&a[0]).and_then(stat)
                } else {
                    let path = self.cstring(a[1])?;
                    self.open_guest(&path).ok().as_ref().and_then(stat)
                };
                let Some((bytes, size)) = metadata else {
                    return Ok(errno(if nr == 80 { 9 } else { 2 }));
                };
                self.write_bytes(if nr == 80 { a[1] } else { a[2] }, &bytes[..size])?;
                0
            }
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
                let mut buffer = vec![0; a[1].min(files::IO_LIMIT) as usize];
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
                    let Some(file) = self.files.get_mut(&a[4]) else {
                        let _ = self.cpu_mut()?.mem_unmap(address, length);
                        return Ok(errno(9));
                    };
                    let old = file.stream_position()?;
                    file.seek(SeekFrom::Start(a[5]))?;
                    let mut bytes = vec![0; length as usize];
                    let n = self.read_file(a[4], &mut bytes)?;
                    self.files
                        .get_mut(&a[4])
                        .unwrap()
                        .seek(SeekFrom::Start(old))?;
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

    pub(super) fn open_guest(&self, path: &str) -> std::io::Result<GuestFile> {
        let pid = crate::threads::MAIN_TID;
        if path == "/proc/self/maps"
            || path == format!("/proc/{pid}/maps")
            || path == format!("/proc/self/task/{pid}/maps")
        {
            let bytes = self
                .proc_maps()
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            return Ok(GuestFile::Memory(std::io::Cursor::new(bytes)));
        }
        if path == "/dev/urandom" || path == "/dev/random" {
            return Ok(GuestFile::Random { path: path.into() });
        }
        let relative = std::path::Path::new(path.trim_start_matches('/'));
        if relative
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        {
            return Err(std::io::ErrorKind::NotFound.into());
        }
        let root = self.rootfs.canonicalize()?;
        let host_path = root.join(relative).canonicalize()?;
        if !host_path.starts_with(root) {
            return Err(std::io::ErrorKind::NotFound.into());
        }
        File::open(host_path).map(|file| GuestFile::Host {
            file,
            path: path.into(),
        })
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
