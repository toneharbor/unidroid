use super::{call, errno};
use std::time::{SystemTime, UNIX_EPOCH};
use unidroid::{Emulator, Result};

#[test]
fn empty_path_stats_the_descriptor_or_cwd() -> Result<()> {
    let cwd = (-100i64) as u64;
    for (dir, size) in [("lib64", 128), ("lib", 104)] {
        let mut emu = Emulator::builder().fixed_clock(123).build()?;
        emu.add_file("/value", b"abc")?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let empty = emu.new_cstring("")?;
        let buffer = emu.malloc(256)?;
        for name in ["/", "/value"] {
            let path = emu.new_cstring(name)?;
            let fd = call(&mut emu, libc, "open", &[path, 0])?;
            assert_eq!(call(&mut emu, libc, "fstat", &[fd, buffer])?, 0);
            let expected = emu.read_bytes(buffer, size)?;
            for flags in [0x1000, 0x1100] {
                assert_eq!(
                    call(&mut emu, libc, "fstatat", &[fd, empty, buffer, flags])?,
                    0
                );
                assert_eq!(emu.read_bytes(buffer, size)?, expected);
            }
            if name == "/" {
                assert_eq!(
                    call(&mut emu, libc, "fstatat", &[cwd, empty, buffer, 0x1000])?,
                    0
                );
                assert_eq!(emu.read_bytes(buffer, size)?, expected);
            }
            let result = call(&mut emu, libc, "fstatat", &[fd, empty, buffer, 0])?;
            errno(&mut emu, libc, result, 2)?;
        }
        let result = call(&mut emu, libc, "fstatat", &[9999, empty, buffer, 0x1000])?;
        errno(&mut emu, libc, result, 9)?;
        let result = call(&mut emu, libc, "fstatat", &[cwd, 0, buffer, 0x1000])?;
        errno(&mut emu, libc, result, 14)?;
    }
    Ok(())
}

#[test]
fn overlay_stat_modes_inodes_blocks_and_change_times_match_both_layouts() -> Result<()> {
    for (dir, width, size, inode_offset) in [("lib64", 8, 128, 8), ("lib", 4, 104, 96)] {
        for fixed in [None, Some(-123)] {
            let mut builder = Emulator::builder();
            if let Some(seconds) = fixed {
                builder = builder.fixed_clock(seconds);
            }
            let mut emu = builder.build()?;
            let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
            emu.remove_file("/lib/libc.so")?;
            let name = emu.new_cstring("/lib/./libc.so")?;
            let fd = call(&mut emu, libc, "open", &[name, 0x42, 0o6754])?;
            let data = emu.malloc(513)?;
            emu.write_bytes(data, &[0x42; 513])?;
            assert_eq!(call(&mut emu, libc, "write", &[fd, data, 513])?, 513);
            let stat = emu.malloc(144)?;
            let before_stat = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let mut after_stat = 0;
            let mut saved_times = Vec::new();
            for function in ["stat", "fstat", "stat"] {
                emu.write_bytes(stat, &[0xa5; 144])?;
                assert_eq!(
                    call(
                        &mut emu,
                        libc,
                        function,
                        &[if function == "fstat" { fd } else { name }, stat]
                    )?,
                    0
                );
                if saved_times.is_empty() {
                    after_stat = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
                }
                for (offset, expected) in [
                    (0, 0u64),
                    (inode_offset, 0x834e0abd3c3c269e),
                    (32, 0),
                    (48, 513),
                    (64, 2),
                ] {
                    assert_eq!(emu.read_bytes(stat + offset, 8)?, expected.to_le_bytes());
                }
                for (offset, expected) in [(16, 0o106754u32), (20, 1), (24, 0), (28, 0), (56, 4096)]
                {
                    assert_eq!(emu.read_bytes(stat + offset, 4)?, expected.to_le_bytes());
                }
                if dir == "lib" {
                    assert_eq!(emu.read_bytes(stat + 12, 4)?, 0x3c3c269eu32.to_le_bytes());
                }
                let times = emu.read_bytes(stat + 72, 6 * width)?;
                if saved_times.is_empty() {
                    saved_times = times.clone();
                }
                assert_eq!(times, saved_times);
                for pair in times.chunks_exact(2 * width) {
                    if let Some(seconds) = fixed {
                        assert_eq!(&pair[..width], &seconds.to_le_bytes()[..width]);
                        assert_eq!(&pair[width..], vec![0; width]);
                    } else {
                        let mut seconds = [0; 8];
                        let mut nanos = [0; 8];
                        seconds[..width].copy_from_slice(&pair[..width]);
                        nanos[..width].copy_from_slice(&pair[width..]);
                        let time = u64::from_le_bytes(seconds) as u128 * 1_000_000_000
                            + u64::from_le_bytes(nanos) as u128;
                        assert!((before_stat..=after_stat).contains(&time));
                    }
                }
                assert_eq!(
                    emu.read_bytes(stat + size, 144 - size as usize)?,
                    vec![0xa5; 144 - size as usize]
                );
            }
            let directory = emu.new_cstring("/created-dir")?;
            assert_eq!(call(&mut emu, libc, "mkdir", &[directory, 0o2750])?, 0);
            assert_eq!(call(&mut emu, libc, "stat", &[directory, stat])?, 0);
            assert_eq!(emu.read_bytes(stat + 16, 4)?, 0o042750u32.to_le_bytes());
        }
    }
    Ok(())
}
