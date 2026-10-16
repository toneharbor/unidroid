use crate::{Emulator, MMAP_END, MMAP_START, Prot, Region, Result, page_up};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::Component,
};
impl Emulator {
    pub(crate) fn syscall(&mut self, nr: u64, a: [u64; 6], pc: u64) -> Result<u64> {
        let errno = |n: i64| (-n) as u64;
        Ok(match nr {
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
                let mut buffer = vec![0; a[2] as usize];
                let result = file.read(&mut buffer);
                if let Some(position) = position {
                    file.seek(SeekFrom::Start(position))?;
                }
                match result {
                    Ok(n) => {
                        self.write_bytes(a[1], &buffer[..n])?;
                        n as u64
                    }
                    Err(_) => errno(5),
                }
            }
            79 | 80 => {
                let metadata = if nr == 80 {
                    self.files.get(&a[0]).and_then(|f| f.metadata().ok())
                } else {
                    let path = self.cstring(a[1])?;
                    self.open_guest(&path).ok().and_then(|f| f.metadata().ok())
                };
                let Some(m) = metadata else {
                    return Ok(errno(if nr == 80 { 9 } else { 2 }));
                };
                let mut bytes = [0; 128];
                for (offset, value) in [
                    (0, m.dev()),
                    (8, m.ino()),
                    (32, m.rdev()),
                    (48, m.size()),
                    (64, m.blocks()),
                    (72, m.atime() as u64),
                    (80, m.atime_nsec() as u64),
                    (88, m.mtime() as u64),
                    (96, m.mtime_nsec() as u64),
                    (104, m.ctime() as u64),
                    (112, m.ctime_nsec() as u64),
                ] {
                    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
                }
                for (offset, value) in [
                    (16, m.mode()),
                    (20, m.nlink() as u32),
                    (24, m.uid()),
                    (28, m.gid()),
                    (56, 4096),
                ] {
                    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
                }
                self.write_bytes(if nr == 80 { a[1] } else { a[2] }, &bytes)?;
                0
            }
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
            98 => match a[1] & 127 {
                1 | 10 => 0,
                0 | 9 => errno(11),
                _ => errno(38),
            },
            99 | 124 | 233 => 0,
            113 | 169 => {
                let mut time = libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                };
                let clock = if nr == 169 || a[0] == 0 {
                    libc::CLOCK_REALTIME
                } else {
                    libc::CLOCK_MONOTONIC
                };
                unsafe {
                    libc::clock_gettime(clock, &mut time);
                }
                let out = if nr == 169 { a[0] } else { a[1] };
                if out != 0 {
                    self.write_u64(out, time.tv_sec as u64)?;
                    self.write_u64(
                        out + 8,
                        (if nr == 169 {
                            time.tv_nsec / 1000
                        } else {
                            time.tv_nsec
                        }) as u64,
                    )?;
                }
                if nr == 169 && a[1] != 0 {
                    self.write_u64(a[1], 0)?;
                }
                0
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
                            .uc
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
                if self.uc.mem_map(address, length, Prot(a[2] as u32)).is_err() {
                    return Ok(errno(12));
                }
                if a[3] & 32 == 0 {
                    let Some(file) = self.files.get_mut(&a[4]) else {
                        let _ = self.uc.mem_unmap(address, length);
                        return Ok(errno(9));
                    };
                    let old = file.stream_position()?;
                    file.seek(SeekFrom::Start(a[5]))?;
                    let mut bytes = vec![0; length as usize];
                    let n = file.read(&mut bytes)?;
                    file.seek(SeekFrom::Start(old))?;
                    if self.write_bytes(address, &bytes[..n]).is_err() {
                        let _ = self.uc.mem_unmap(address, length);
                        return Ok(errno(14));
                    }
                }
                self.regions.push(Region {
                    start: address,
                    end: address + length,
                    shared: false,
                });
                address
            }
            226 => {
                self.protect(a[0], a[1], Prot(a[2] as u32))?;
                0
            }
            _ => {
                eprintln!(
                    "warning: unsupported syscall {nr} at {}, returning -ENOSYS",
                    self.location(pc)
                );
                errno(38)
            }
        })
    }
    fn lowest_mmap_gap(&self, length: u64) -> Option<u64> {
        let mut cursor = crate::MMAP_START;
        let mut regions = self.uc.mem_regions().ok()?;
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

    fn open_guest(&self, path: &str) -> std::io::Result<File> {
        if path == "/dev/urandom" || path == "/dev/random" {
            return File::open("/dev/urandom");
        }
        let relative = std::path::Path::new(path.trim_start_matches('/'));
        if relative
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        {
            return Err(std::io::ErrorKind::NotFound.into());
        }
        let root = self.rootfs.canonicalize()?;
        let path = root.join(relative).canonicalize()?;
        if !path.starts_with(root) {
            return Err(std::io::ErrorKind::NotFound.into());
        }
        File::open(path)
    }
    fn unmap(&mut self, start: u64, size: u64) -> Result<()> {
        let end = start + size;
        for region in self.uc.mem_regions()? {
            let lo = start.max(region.begin);
            let hi = end.min(region.end + 1);
            if lo < hi {
                self.uc.mem_unmap(lo, hi - lo)?;
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
mod tests {
    use super::*;
    #[test]
    fn brk_can_shrink_and_anonymous_mappings_start_zeroed() -> Result<()> {
        let mut emu = Emulator::builder().build()?;
        let initial = emu.syscall(214, [0; 6], 0)?;
        assert_eq!(
            emu.syscall(214, [initial + 8192, 0, 0, 0, 0, 0], 0)?,
            initial + 8192
        );
        emu.write_bytes(initial + 4096, &[42])?;
        assert_eq!(emu.syscall(214, [initial, 0, 0, 0, 0, 0], 0)?, initial);
        assert!(emu.read_bytes(initial + 4096, 1).is_err());
        let ptr = emu.syscall(222, [0, 8192, 3, 0x22, u64::MAX, 0], 0)?;
        assert_eq!(emu.read_bytes(ptr, 8192)?, vec![0; 8192]);
        emu.syscall(215, [ptr, 4096, 0, 0, 0, 0], 0)?;
        assert!(emu.read_bytes(ptr, 1).is_err());
        assert_eq!(emu.read_bytes(ptr + 4096, 1)?, [0]);
        Ok(())
    }

    #[test]
    fn rootfs_files_are_read_only_and_fstat_uses_the_android_layout() -> Result<()> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("target/syscall-rootfs-{}", std::process::id()));
        std::fs::create_dir_all(&root)?;
        std::fs::write(root.join("file"), b"abcdef")?;
        let mut emu = Emulator::builder().rootfs(&root).build()?;
        let path = emu.new_cstring("/file")?;
        let fd = emu.syscall(56, [(-100i64) as u64, path, 0, 0, 0, 0], 0)?;
        let buffer = emu.malloc(128)?;
        assert_eq!(emu.syscall(63, [fd, buffer, 3, 0, 0, 0], 0)?, 3);
        assert_eq!(emu.read_bytes(buffer, 3)?, b"abc");
        assert_eq!(emu.syscall(80, [fd, buffer, 0, 0, 0, 0], 0)?, 0);
        assert_eq!(emu.read_u64(buffer + 48)?, 6);
        assert_eq!(
            emu.syscall(56, [(-100i64) as u64, path, 2, 0, 0, 0], 0)? as i64,
            -13
        );
        assert_eq!(emu.syscall(57, [fd, 0, 0, 0, 0, 0], 0)?, 0);
        let missing = emu.new_cstring("/missing")?;
        assert_eq!(
            emu.syscall(56, [(-100i64) as u64, missing, 0, 0, 0, 0], 0)? as i64,
            -2
        );
        let random = emu.new_cstring("/dev/urandom")?;
        let fd = emu.syscall(56, [(-100i64) as u64, random, 0, 0, 0, 0], 0)?;
        assert_eq!(emu.syscall(63, [fd, buffer, 16, 0, 0, 0], 0)?, 16);
        drop(emu);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn mmap_reuses_lowest_gap_and_prctl_manages_name() -> Result<()> {
        let mut emu = Emulator::builder().process_name("service").build()?;
        let first = emu.syscall(222, [0, 0x3000, 3, 0x22, u64::MAX, 0], 0)?;
        let second = emu.syscall(222, [0, 0x1000, 3, 0x22, u64::MAX, 0], 0)?;
        emu.syscall(215, [first, 0x3000, 0, 0, 0, 0], 0)?;
        assert_eq!(
            emu.syscall(222, [0, 0x1000, 3, 0x22, u64::MAX, 0], 0)?,
            first
        );
        assert_ne!(second, first);
        let name = emu.new_cstring("worker-name")?;
        assert_eq!(emu.syscall(167, [15, name, 0, 0, 0, 0], 0)?, 0);
        let out = emu.malloc(16)?;
        assert_eq!(emu.syscall(167, [16, out, 0, 0, 0, 0], 0)?, 0);
        assert_eq!(emu.read_bytes(out, 12)?, b"worker-name\0");
        assert_eq!(emu.syscall(167, [0x53564d41, 0, 0, 0, 0, 0], 0)?, 0);
        assert_eq!(emu.syscall(167, [99, 0, 0, 0, 0, 0], 0)? as i64, -22);
        Ok(())
    }

    #[test]
    fn unknown_syscall_returns_linux_enosys() {
        let mut emu = Emulator::builder().build().unwrap();
        assert_eq!(emu.syscall(9999, [0; 6], 0x8000).unwrap() as i64, -38);
    }
}
