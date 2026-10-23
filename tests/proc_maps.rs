use unidroid::{Emulator, Module, Result};

fn call(emu: &mut Emulator, libc: Module, name: &str, args: &[u64]) -> Result<u64> {
    let function = emu.find_symbol(libc, name)?;
    emu.call_function(function, args)
}

fn word(emu: &Emulator, address: u64, width: usize) -> Result<u64> {
    let mut bytes = [0; 8];
    bytes[..width].copy_from_slice(&emu.read_bytes(address, width)?);
    Ok(u64::from_le_bytes(bytes))
}

#[test]
fn maps_labels_modules_without_so_suffix_under_system_bin() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("target/maps-label-{}-{dir}", std::process::id()));
        std::fs::create_dir_all(&root)?;
        let executable = root.join("linker");
        // Use the committed ELF under another guest module name.
        std::fs::copy(format!("rootfs/{dir}/libdl.so"), &executable)?;
        let mut emu = Emulator::builder().build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        emu.load_library(&executable)?;
        let buffer = emu.malloc(65536)?;
        let path = emu.new_cstring("/proc/self/maps")?;
        let fd = call(&mut emu, libc, "open", &[path, 0])?;
        let count = call(&mut emu, libc, "read", &[fd, buffer, 65536])?;
        assert!(count > 0 && count < 65536);
        let text = String::from_utf8(emu.read_bytes(buffer, count as usize)?)?;
        let lines: Vec<_> = text
            .lines()
            .filter(|line| line.ends_with("/system/bin/linker"))
            .collect();
        assert!(!lines.is_empty());
        assert!(
            lines
                .iter()
                .all(|line| line.contains("b3:19 0          /system/bin/linker"))
        );
        drop(emu);
        std::fs::remove_dir_all(root)?;
    }
    Ok(())
}

#[test]
fn maps_stat_uses_byte_array_metadata_with_fixed_or_live_time() -> Result<()> {
    use std::time::{SystemTime, UNIX_EPOCH};
    for (dir, width, size, inode) in [("lib64", 8, 128, 8), ("lib", 4, 104, 96)] {
        for fixed in [None, Some(-123)] {
            let mut builder = Emulator::builder();
            if let Some(seconds) = fixed {
                builder = builder.fixed_clock(seconds);
            }
            let mut emu = builder.build()?;
            let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
            let stat = emu.malloc(144)?;
            let path = emu.new_cstring("/proc/self/maps")?;
            let fd = call(&mut emu, libc, "open", &[path, 0])?;
            let length = call(&mut emu, libc, "lseek", &[fd, 0, 2])?;
            assert!(length > 0 && length < 65536);
            for (name, args) in [
                ("fstat", vec![fd, stat]),
                ("fstatat", vec![(-100i64) as u64, path, stat, 0]),
            ] {
                emu.write_bytes(stat, &[0xff; 144])?;
                let before = SystemTime::now().duration_since(UNIX_EPOCH)?;
                assert_eq!(call(&mut emu, libc, name, &args)?, 0);
                let after = SystemTime::now().duration_since(UNIX_EPOCH)?;
                for (offset, expected) in [
                    (0, 1),
                    (inode, 1),
                    (32, 0),
                    (48, length),
                    (64, length.div_ceil(4096)),
                ] {
                    assert_eq!(word(&emu, stat + offset, 8)?, expected);
                }
                for (offset, expected) in [(16, 0o100000), (20, 0), (24, 0), (28, 0), (56, 4096)] {
                    assert_eq!(word(&emu, stat + offset, 4)?, expected);
                }
                if dir == "lib" {
                    assert_eq!(word(&emu, stat + 12, 4)?, 1);
                }
                let time = emu.read_bytes(stat + 72, 2 * width)?;
                for index in 0..3 {
                    let offset = stat + 72 + (index * 2 * width) as u64;
                    assert_eq!(emu.read_bytes(offset, 2 * width)?, time);
                    if let Some(seconds) = fixed {
                        assert_eq!(
                            emu.read_bytes(offset, width)?,
                            seconds.to_le_bytes()[..width]
                        );
                        assert_eq!(word(&emu, offset + width as u64, width)?, 0);
                    } else {
                        let seconds = word(&emu, offset, width)?;
                        let nanos = word(&emu, offset + width as u64, width)?;
                        assert!(nanos < 1_000_000_000);
                        let timestamp = seconds as u128 * 1_000_000_000 + nanos as u128;
                        assert!((before.as_nanos()..=after.as_nanos()).contains(&timestamp));
                    }
                }
                assert_eq!(
                    emu.read_bytes(stat + size, 144 - size as usize)?,
                    vec![0xff; 144 - size as usize]
                );
            }
        }
    }
    Ok(())
}

#[test]
fn maps_snapshot_and_open_offset_survive_rewind_and_copy() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let buffer = emu.malloc(65536)?;
        let path = emu.new_cstring("/proc/self/maps")?;
        let fd = call(&mut emu, libc, "open", &[path, 0])?;
        let length = call(&mut emu, libc, "read", &[fd, buffer, 65536])?;
        assert!(length > 17 && length < 65536);
        let original = emu.read_bytes(buffer, length as usize)?;
        assert_eq!(call(&mut emu, libc, "lseek", &[fd, 17, 0])?, 17);
        let mapped = call(&mut emu, libc, "mmap", &[0, 4096, 3, 0x22, u64::MAX, 0])?;
        assert_ne!(mapped as u32, u32::MAX);
        let newer = call(&mut emu, libc, "open", &[path, 0])?;
        let count = call(&mut emu, libc, "read", &[newer, buffer, 65536])?;
        assert!(count > 0 && count < 65536);
        assert_ne!(emu.read_bytes(buffer, count as usize)?, original);
        assert_eq!(call(&mut emu, libc, "close", &[newer])?, 0);
        let checkpoint = emu.checkpoint()?;
        assert_eq!(
            call(&mut emu, libc, "read", &[fd, buffer, 65536])?,
            length - 17
        );
        assert_eq!(
            emu.read_bytes(buffer, length as usize - 17)?,
            original[17..]
        );
        assert_eq!(call(&mut emu, libc, "close", &[fd])?, 0);
        emu.rewind(&checkpoint)?;
        let mut copy = Emulator::builder().build()?;
        copy.rewind(&checkpoint)?;
        for restored in [&mut emu, &mut copy] {
            assert_eq!(call(restored, libc, "lseek", &[fd, 0, 1])?, 17);
            assert_eq!(
                call(restored, libc, "read", &[fd, buffer, 65536])?,
                length - 17
            );
            assert_eq!(
                restored.read_bytes(buffer, length as usize - 17)?,
                original[17..]
            );
        }
    }
    Ok(())
}

#[test]
fn maps_paths_expose_sorted_mappings_and_support_file_operations() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().fixed_clock(1_700_000_000).build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let buffer = emu.malloc(65536)?;
        let stat = emu.malloc(144)?;
        let pid = call(&mut emu, libc, "getpid", &[])?;
        for path in [
            "/proc/self/maps".to_string(),
            format!("/proc/{pid}/maps"),
            format!("/proc/self/task/{pid}/maps"),
        ] {
            let path = emu.new_cstring(&path)?;
            assert_eq!(call(&mut emu, libc, "access", &[path, 7])?, 0);
            let mappings = emu.maps()?;
            let fd = call(&mut emu, libc, "open", &[path, 0])?;
            assert!(fd >= 3 && fd < u32::MAX as u64);
            let count = call(&mut emu, libc, "read", &[fd, buffer, 65536])?;
            assert!(count > 0 && count < 65536);
            let bytes = emu.read_bytes(buffer, count as usize)?;
            let text = std::str::from_utf8(&bytes)?;
            assert!(text.ends_with('\n'));
            assert!(text.contains(&format!("b3:19 0          /system/{dir}/libc.so\n")));
            assert!(text.contains("00:00 0          [stack]\n"));
            assert!(text.contains("00:00 0          anonymous\n"));
            assert_eq!(text.lines().count(), mappings.len());
            let mut previous_end = 0;
            for (line, mapping) in text.lines().zip(&mappings) {
                let (fields, label) = line.split_once(" 0          ").unwrap();
                assert!(!label.starts_with(' '));
                let fields: Vec<_> = fields.split(' ').collect();
                assert_eq!(fields.len(), 4);
                let (start, end) = fields[0].split_once('-').unwrap();
                for address in [start, end] {
                    assert!(address.len() >= 8);
                    assert!(
                        address
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    );
                }
                let start = u64::from_str_radix(start, 16)?;
                let end = u64::from_str_radix(end, 16)?;
                assert!(start >= previous_end && end > start);
                previous_end = end;
                assert_eq!((start, end), (mapping.start, mapping.end));
                assert_eq!(fields[1].len(), 4);
                assert_eq!(&fields[1][..3], mapping.perms);
                assert_eq!(&fields[1][3..], "p");
                assert_eq!(fields[2], "00000000");
                if mapping.name.ends_with(".so") {
                    assert_eq!(fields[3], "b3:19");
                    assert_eq!(label, format!("/system/{dir}/{}", mapping.name));
                } else {
                    assert_eq!(fields[3], "00:00");
                    assert_eq!(
                        label,
                        if mapping.name == "[stack]" {
                            "[stack]"
                        } else {
                            "anonymous"
                        }
                    );
                }
            }
            assert_eq!(call(&mut emu, libc, "read", &[fd, buffer, 1])?, 0);
            assert_eq!(call(&mut emu, libc, "fstat", &[fd, stat])?, 0);
            assert_eq!(word(&emu, stat + 48, 8)?, count);
            assert_eq!(call(&mut emu, libc, "lseek", &[fd, 0, 2])?, count);
            assert_eq!(call(&mut emu, libc, "lseek", &[fd, 0, 0])?, 0);
            assert_eq!(call(&mut emu, libc, "lseek", &[fd, 7, 1])?, 7);
            let mapped = call(&mut emu, libc, "mmap", &[0, count, 1, 2, fd, 0])?;
            assert_eq!(emu.read_bytes(mapped, count as usize)?, bytes);
            assert_eq!(call(&mut emu, libc, "lseek", &[fd, 0, 1])?, 7);
            assert_eq!(call(&mut emu, libc, "munmap", &[mapped, count])?, 0);
            let args = if dir == "lib64" {
                vec![fd, buffer, 11, 19]
            } else {
                vec![fd, buffer, 11, 0, 19, 0]
            };
            assert_eq!(call(&mut emu, libc, "pread64", &args)?, 11);
            assert_eq!(emu.read_bytes(buffer, 11)?, bytes[19..30]);
            assert_eq!(call(&mut emu, libc, "read", &[fd, buffer, 11])?, 11);
            assert_eq!(emu.read_bytes(buffer, 11)?, bytes[7..18]);
            assert_eq!(call(&mut emu, libc, "close", &[fd])?, 0);
            assert_eq!(
                call(&mut emu, libc, "read", &[fd, buffer, 1])? as u32,
                u32::MAX
            );
            let errno = call(&mut emu, libc, "__errno", &[])?;
            assert_eq!(word(&emu, errno, 4)?, 9);
        }
    }
    Ok(())
}
