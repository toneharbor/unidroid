use crate::{Emulator, MMAP_END, MMAP_START, Prot, Region, Result, page_up};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Component,
};
impl Emulator {
    pub(crate) fn syscall(&mut self, guest_nr: u64, mut a: [u64; 6], pc: u64) -> Result<u64> {
        let nr = self
            .architecture()
            .syscall(guest_nr, &mut a)
            .unwrap_or(u64::MAX);
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
                let (bytes, size) = self.architecture().stat(&m);
                self.write_bytes(if nr == 80 { a[1] } else { a[2] }, &bytes[..size])?;
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
                    self.write_pointer(out, time.tv_sec as u64)?;
                    self.write_pointer(
                        out + self.pointer_size(),
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
                    let n = file.read(&mut bytes)?;
                    file.seek(SeekFrom::Start(old))?;
                    if self.write_bytes(address, &bytes[..n]).is_err() {
                        let _ = self.cpu_mut()?.mem_unmap(address, length);
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
                    "warning: unsupported syscall {guest_nr} at {}, returning -ENOSYS",
                    self.location(pc)
                );
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
mod tests {
    use super::*;
    use crate::Architecture;
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn arm32_syscalls_preserve_argument_registers_and_resume_in_arm_or_thumb() -> Result<()> {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(Architecture::Arm32)?;
        let code = crate::RETURN + 0x100;
        // mov r7,#199; svc #0; mov r0,r1; bx lr
        let arm: Vec<_> = [0xe3a070c7u32, 0xef000000, 0xe1a00001, 0xe12fff1e]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        let thumb = vec![0xc7, 0x27, 0, 0xdf, 8, 0x46, 0x70, 0x47];
        for (bytes, bit) in [(arm, 0), (thumb, 1)] {
            emu.write_bytes(code, &bytes)?;
            assert_eq!(
                emu.call_function(code | bit, &[0, 0x1234_5678])?,
                0x1234_5678
            );
        }
        Ok(())
    }

    #[test]
    fn arm32_mmap2_uses_pages_and_pread64_uses_r4_r5_without_seeking() -> Result<()> {
        use std::io::Write;
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("target/offset32-{}", std::process::id()));
        std::fs::create_dir_all(&root)?;
        let mut file = File::create(root.join("file"))?;
        file.write_all(&[0x41; 4096])?;
        file.write_all(b"page two")?;
        file.seek(SeekFrom::Start(0x1_0000_0002))?;
        file.write_all(b"high")?;
        drop(file);
        let mut emu = Emulator::builder().rootfs(&root).build()?;
        emu.setup_cpu(Architecture::Arm32)?;
        let path = emu.new_cstring("/file")?;
        let fd = emu.syscall(322, [0, path, 0, 0, 0, 0], 0)?;
        let mapped = emu.syscall(192, [0, 4096, 1, 2, fd, 1], 0)?;
        assert_eq!(emu.read_bytes(mapped, 8)?, b"page two");
        let out = emu.malloc(8)?;
        assert_eq!(emu.syscall(180, [fd, out, 4, 0xdeadbeef, 2, 1], 0)?, 4);
        assert_eq!(emu.read_bytes(out, 4)?, b"high");
        assert_eq!(emu.syscall(180, [fd, out, 8, 0xdeadbeef, 4096, 0], 0)?, 8);
        assert_eq!(emu.read_bytes(out, 8)?, b"page two");
        assert_eq!(emu.syscall(3, [fd, out, 8, 0, 0, 0], 0)?, 8);
        assert_eq!(emu.read_bytes(out, 8)?, [0x41; 8]);
        drop(emu);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn arm32_stat64_writes_both_inode_fields_and_32_bit_times() -> Result<()> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("target/stat32-{}", std::process::id()));
        std::fs::create_dir_all(&root)?;
        std::fs::write(root.join("file"), b"abcdef")?;
        let mut emu = Emulator::builder().rootfs(&root).build()?;
        emu.setup_cpu(Architecture::Arm32)?;
        let path = emu.new_cstring("/file")?;
        let fd = emu.syscall(322, [0, path, 0, 0, 0, 0], 0)?;
        let m = std::fs::metadata(root.join("file"))?;
        let out = emu.malloc(128)?;
        for (nr, args) in [(197, [fd, out, 0, 0, 0, 0]), (327, [0, path, out, 0, 0, 0])] {
            emu.write_bytes(out, &[0xa5; 128])?;
            assert_eq!(emu.syscall(nr, args, 0)?, 0);
            for (offset, expected) in [
                (0, m.dev()),
                (32, m.rdev()),
                (48, 6),
                (64, m.blocks()),
                (96, m.ino()),
            ] {
                assert_eq!(emu.read_u64(out + offset)?, expected);
            }
            for (offset, expected) in [
                (12, m.ino() as u32),
                (16, m.mode()),
                (20, m.nlink() as u32),
                (24, m.uid()),
                (28, m.gid()),
                (56, 4096),
                (72, m.atime() as u32),
                (76, m.atime_nsec() as u32),
                (80, m.mtime() as u32),
                (84, m.mtime_nsec() as u32),
                (88, m.ctime() as u32),
                (92, m.ctime_nsec() as u32),
            ] {
                assert_eq!(emu.read_bytes(out + offset, 4)?, expected.to_le_bytes());
            }
            assert_eq!(emu.read_bytes(out + 104, 24)?, [0xa5; 24]);
        }
        drop(emu);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn arm32_clock_and_timeval_are_two_32_bit_words() -> Result<()> {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(Architecture::Arm32)?;
        let out = emu.malloc(24)?;
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        for (nr, args, fraction_limit) in [
            (263, [0, out, 0, 0, 0, 0], 1_000_000_000),
            (78, [out, out + 16, 0, 0, 0, 0], 1_000_000),
        ] {
            emu.write_bytes(out, &[0xa5; 24])?;
            assert_eq!(emu.syscall(nr, args, 0)?, 0);
            let seconds = u32::from_le_bytes(emu.read_bytes(out, 4)?.try_into().unwrap()) as u64;
            let fraction = u32::from_le_bytes(emu.read_bytes(out + 4, 4)?.try_into().unwrap());
            let after = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs();
            assert!((before..=after).contains(&seconds));
            assert!(fraction < fraction_limit);
            assert_eq!(emu.read_bytes(out + 8, 8)?, [0xa5; 8]);
            if nr == 78 {
                assert_eq!(emu.read_bytes(out + 16, 8)?, [0; 8]);
            }
        }
        Ok(())
    }

    #[test]
    fn brk_can_shrink_and_anonymous_mappings_start_zeroed() -> Result<()> {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(Architecture::Arm64)?;
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
        emu.setup_cpu(Architecture::Arm64)?;
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
        emu.setup_cpu(Architecture::Arm64)?;
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
        emu.setup_cpu(Architecture::Arm64).unwrap();
        assert_eq!(emu.syscall(9999, [0; 6], 0x8000).unwrap() as i64, -38);
    }
}
