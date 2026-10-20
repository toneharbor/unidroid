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
