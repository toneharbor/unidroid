use super::*;
use crate::Architecture;
use std::{fs::File, io::SeekFrom, os::unix::fs::MetadataExt};

#[test]
fn huge_random_counts_are_short_reads_with_a_linux_limit() -> Result<()> {
    for arch in [Architecture::Arm64, Architecture::Arm32] {
        let mut emu = Emulator::builder().random_seed(0).build()?;
        emu.setup_cpu(arch)?;
        let arm64 = arch == Architecture::Arm64;
        let limit = 33_554_431;
        let out = emu.malloc(limit + 1)?;
        emu.write_bytes(out + limit as u64, &[0xa5])?;
        assert_eq!(
            emu.syscall(
                if arm64 { 278 } else { 384 },
                [out, u64::MAX, 0, 0, 0, 0],
                0
            )?,
            limit as u64
        );
        assert_eq!(
            emu.read_bytes(out, 8)?,
            [0xaf, 0xcd, 0x1d, 0x7b, 0x39, 0xa8, 0x20, 0xe2]
        );
        for path in ["/dev/urandom", "/dev/random"] {
            let path = emu.new_cstring(path)?;
            let fd = emu.syscall(if arm64 { 56 } else { 322 }, [0, path, 0, 0, 0, 0], 0)?;
            for nr in if arm64 { [63, 67] } else { [3, 180] } {
                assert_eq!(
                    emu.syscall(nr, [fd, out, u64::MAX, 0, 0, 0], 0)?,
                    limit as u64
                );
            }
        }
        assert_eq!(emu.read_bytes(out + limit as u64, 1)?, [0xa5]);
    }
    Ok(())
}

#[test]
fn huge_file_reads_allocate_only_the_remaining_bytes_and_pread_keeps_position() -> Result<()> {
    let bytes = std::fs::read("rootfs/lib/libdl.so")?;
    for arch in [Architecture::Arm64, Architecture::Arm32] {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(arch)?;
        let arm64 = arch == Architecture::Arm64;
        let path = emu.new_cstring("/lib/libdl.so")?;
        let fd = emu.syscall(if arm64 { 56 } else { 322 }, [0, path, 0, 0, 0, 0], 0)?;
        let out = emu.malloc(bytes.len() + 8)?;
        for count in [u64::MAX, bytes.len() as u64 + 100] {
            emu.syscall(if arm64 { 62 } else { 19 }, [fd, 3, 0, 0, 0, 0], 0)?;
            emu.write_bytes(out, &vec![0xa5; bytes.len() + 8])?;
            let args = if arm64 {
                [fd, out, count, 7, 0, 0]
            } else {
                [fd, out, count, 0, 7, 0]
            };
            assert_eq!(
                emu.syscall(if arm64 { 67 } else { 180 }, args, 0)?,
                bytes.len() as u64 - 7
            );
            assert_eq!(emu.read_bytes(out, bytes.len() - 7)?, bytes[7..]);
            assert_eq!(emu.read_bytes(out + bytes.len() as u64 - 7, 8)?, [0xa5; 8]);
            assert_eq!(
                emu.syscall(if arm64 { 63 } else { 3 }, [fd, out, count, 0, 0, 0], 0)?,
                bytes.len() as u64 - 3
            );
            assert_eq!(emu.read_bytes(out, bytes.len() - 3)?, bytes[3..]);
            assert_eq!(
                emu.syscall(if arm64 { 63 } else { 3 }, [fd, out, count, 0, 0, 0], 0)?,
                0
            );
        }
    }
    Ok(())
}

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
    use std::io::{Seek, Write};
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
fn clock_and_timeval_use_two_pointer_sized_words() -> Result<()> {
    for (arch, width, clock, timeval) in [
        (Architecture::Arm64, 8, 113, 169),
        (Architecture::Arm32, 4, 263, 78),
    ] {
        let mut emu = Emulator::builder().fixed_clock(1_700_000_000).build()?;
        emu.setup_cpu(arch)?;
        let out = emu.malloc(32)?;
        for (nr, args, fraction) in [
            (clock, [0, out, 0, 0, 0, 0], 0u64),
            (timeval, [out, out + 24, 0, 0, 0, 0], 1),
        ] {
            emu.write_bytes(out, &[0xa5; 32])?;
            assert_eq!(emu.syscall(nr, args, 0)?, 0);
            assert_eq!(
                emu.read_bytes(out, width)?,
                1_700_000_000u64.to_le_bytes()[..width]
            );
            assert_eq!(
                emu.read_bytes(out + width as u64, width)?,
                fraction.to_le_bytes()[..width]
            );
            assert_eq!(emu.read_bytes(out + 2 * width as u64, 8)?, [0xa5; 8]);
            if nr == timeval {
                assert_eq!(emu.read_bytes(out + 24, 8)?, [0; 8]);
            }
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
fn rootfs_writes_are_private_and_fstat_uses_the_android_layout() -> Result<()> {
    fn call(emu: &mut Emulator, libc: crate::Module, name: &str, args: &[u64]) -> Result<u64> {
        let function = emu.find_symbol(libc, name)?;
        emu.call_function(function, args)
    }
    for dir in ["lib64", "lib"] {
        let original = std::fs::read(format!("rootfs/{dir}/libdl.so"))?;
        let mut emu = Emulator::builder().build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let path = emu.new_cstring(&format!("/{dir}/libdl.so"))?;
        let fd = call(&mut emu, libc, "open", &[path, 0])?;
        let buffer = emu.malloc(128)?;
        assert_eq!(call(&mut emu, libc, "read", &[fd, buffer, 3])?, 3);
        assert_eq!(emu.read_bytes(buffer, 3)?, original[..3]);
        assert_eq!(call(&mut emu, libc, "fstat", &[fd, buffer])?, 0);
        assert_eq!(emu.read_u64(buffer + 48)?, original.len() as u64);
        let writer = call(&mut emu, libc, "open", &[path, 2])?;
        assert!(writer < u32::MAX as u64);
        emu.write_bytes(buffer, b"new")?;
        assert_eq!(call(&mut emu, libc, "write", &[writer, buffer, 3])?, 3);
        assert_eq!(&emu.file(&format!("/{dir}/libdl.so"))?[..3], b"new");
        assert_eq!(std::fs::read(format!("rootfs/{dir}/libdl.so"))?, original);
        assert_eq!(call(&mut emu, libc, "close", &[fd])?, 0);
        let missing = emu.new_cstring("/missing")?;
        assert_eq!(
            call(&mut emu, libc, "open", &[missing, 0])? as u32,
            u32::MAX
        );
        let errno = call(&mut emu, libc, "__errno", &[])?;
        assert_eq!(emu.read_bytes(errno, 4)?, 2u32.to_le_bytes());
        let random = emu.new_cstring("/dev/urandom")?;
        let fd = call(&mut emu, libc, "open", &[random, 0])?;
        assert_eq!(call(&mut emu, libc, "read", &[fd, buffer, 16])?, 16);
    }
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

#[test]
fn nanosleep_returns_eintr() {
    for arch in [Architecture::Arm64, Architecture::Arm32] {
        let mut emu = Emulator::builder().build().unwrap();
        emu.setup_cpu(arch).unwrap();
        let nr = if arch == Architecture::Arm64 {
            101
        } else {
            162
        };
        assert_eq!(emu.syscall(nr, [0; 6], 0x8000).unwrap() as i64, -4);
    }
}

#[test]
fn clock_nanosleep_returns_eintr() {
    for arch in [Architecture::Arm64, Architecture::Arm32] {
        let mut emu = Emulator::builder().build().unwrap();
        emu.setup_cpu(arch).unwrap();
        let nr = if arch == Architecture::Arm64 {
            115
        } else {
            265
        };
        assert_eq!(emu.syscall(nr, [0; 6], 0x8000).unwrap() as i64, -4);
    }
}

#[test]
fn tgkill_returns_zero() {
    for arch in [Architecture::Arm64, Architecture::Arm32] {
        let mut emu = Emulator::builder().build().unwrap();
        emu.setup_cpu(arch).unwrap();
        let nr = if arch == Architecture::Arm64 {
            131
        } else {
            268
        };
        assert_eq!(emu.syscall(nr, [0; 6], 0x8000).unwrap(), 0);
    }
}

#[test]
fn rt_sigaction_returns_zero() {
    for arch in [Architecture::Arm64, Architecture::Arm32] {
        let mut emu = Emulator::builder().build().unwrap();
        emu.setup_cpu(arch).unwrap();
        let nr = if arch == Architecture::Arm64 {
            134
        } else {
            174
        };
        assert_eq!(emu.syscall(nr, [0; 6], 0x8000).unwrap(), 0);
    }
}

#[test]
fn accept4_returns_eagain() {
    for arch in [Architecture::Arm64, Architecture::Arm32] {
        let mut emu = Emulator::builder().build().unwrap();
        emu.setup_cpu(arch).unwrap();
        let nr = if arch == Architecture::Arm64 {
            242
        } else {
            366
        };
        assert_eq!(emu.syscall(nr, [0; 6], 0x8000).unwrap() as i64, -11);
    }
}
