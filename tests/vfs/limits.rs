use super::{TestRoot, call, errno};
use unidroid::{Emulator, Result};

#[test]
fn host_copies_reject_large_files_but_truncation_skips_the_copy() -> Result<()> {
    let root = TestRoot::new("large-host")?;
    std::fs::create_dir(root.0.join("tree"))?;
    let host = root.0.join("tree/large");
    std::fs::File::create(&host)?.set_len((1 << 30) + 1)?;
    for dir in ["lib64", "lib"] {
        for name in ["/tree/large", "/linked"] {
            let mut emu = Emulator::builder().rootfs(&root.0).build()?;
            emu.add_host_file("/linked", &host)?;
            let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
            let path = emu.new_cstring(name)?;
            let destination = emu.new_cstring("/destination")?;
            emu.add_file("/destination", b"keep")?;
            let result = call(&mut emu, libc, "open", &[path, 2])?;
            errno(&mut emu, libc, result, 27)?;
            let result = call(&mut emu, libc, "rename", &[path, destination])?;
            errno(&mut emu, libc, result, 27)?;
            assert_eq!(emu.file("/destination")?, b"keep");
            let tree = emu.new_cstring("/tree")?;
            let moved = emu.new_cstring("/moved")?;
            let result = call(&mut emu, libc, "rename", &[tree, moved])?;
            errno(&mut emu, libc, result, 27)?;
            assert_eq!(call(&mut emu, libc, "access", &[path, 0])?, 0);
            let fd = call(&mut emu, libc, "open", &[path, 0x202])?;
            assert_ne!(fd as u32, u32::MAX);
            assert_eq!(emu.file(name)?, b"");
            assert_eq!(std::fs::metadata(&host)?.len(), (1 << 30) + 1);
        }
    }
    Ok(())
}

#[test]
fn seeks_past_signed_range_preserve_the_offset() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.add_file("/value", b"x")?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let path = emu.new_cstring("/value")?;
        let fd = call(&mut emu, libc, "open", &[path, 0])?;
        assert_eq!(call(&mut emu, libc, "lseek", &[fd, 1, 0])?, 1);
        let result = call(&mut emu, libc, "lseek", &[fd, u64::MAX, 0])?;
        errno(&mut emu, libc, result, 22)?;
        assert_eq!(call(&mut emu, libc, "lseek", &[fd, 0, 1])?, 1);
        // ARM32 currently exposes only 32-bit lseek, not _llseek.
        if dir == "lib" {
            continue;
        }
        for whence in [1, 2] {
            let result = call(&mut emu, libc, "lseek", &[fd, i64::MAX as u64, whence])?;
            errno(&mut emu, libc, result, 22)?;
            assert_eq!(call(&mut emu, libc, "lseek", &[fd, 0, 1])?, 1);
        }
        assert_eq!(
            call(&mut emu, libc, "lseek", &[fd, i64::MAX as u64, 0])?,
            i64::MAX as u64
        );
        let result = call(&mut emu, libc, "lseek", &[fd, 1, 1])?;
        errno(&mut emu, libc, result, 22)?;
        // The failed seek left INT64_MAX intact.
        assert_eq!(
            call(&mut emu, libc, "lseek", &[fd, (-i64::MAX) as u64, 1])?,
            0
        );
    }
    Ok(())
}

#[test]
fn truncation_rejects_lengths_past_file_limit() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.add_file("/value", b"abc")?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let path = emu.new_cstring("/value")?;
        let fd = call(&mut emu, libc, "open", &[path, 2])?;
        assert_eq!(call(&mut emu, libc, "lseek", &[fd, 2, 0])?, 2);
        let result = call(&mut emu, libc, "ftruncate", &[fd, (1 << 30) + 1])?;
        errno(&mut emu, libc, result, 27)?;
        let args = if dir == "lib64" {
            vec![fd, 1 << 40]
        } else {
            vec![fd, 0, 0, 1 << 8]
        };
        let result = call(&mut emu, libc, "ftruncate64", &args)?;
        errno(&mut emu, libc, result, 27)?;
        assert_eq!(call(&mut emu, libc, "lseek", &[fd, 0, 1])?, 2);
        assert_eq!(emu.file("/value")?, b"abc");
    }
    Ok(())
}

#[test]
fn large_write_offsets_fail_without_changing_contents() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.add_file("/value", b"abc")?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let path = emu.new_cstring("/value")?;
        let fd = call(&mut emu, libc, "open", &[path, 2])?;
        let buffer = emu.new_cstring("x")?;
        // off_t is 32-bit on arm32; pwrite64 below covers a terabyte on both.
        let offset = if dir == "lib64" {
            1 << 40
        } else {
            i32::MAX as u64
        };
        assert_eq!(call(&mut emu, libc, "lseek", &[fd, offset, 0])?, offset);
        let result = call(&mut emu, libc, "write", &[fd, buffer, 1])?;
        errno(&mut emu, libc, result, 27)?;
        let args = if dir == "lib64" {
            vec![fd, buffer, 1, 1 << 40]
        } else {
            vec![fd, buffer, 1, 0, 0, 1 << 8]
        };
        let result = call(&mut emu, libc, "pwrite64", &args)?;
        errno(&mut emu, libc, result, 27)?;
        assert_eq!(emu.file("/value")?, b"abc");
    }
    Ok(())
}

#[test]
fn writes_stop_at_file_limit_and_reject_offsets_at_or_past_it() -> Result<()> {
    let limit = 1 << 30;
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.add_file("/value", b"")?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let path = emu.new_cstring("/value")?;
        let fd = call(&mut emu, libc, "open", &[path, 2])?;
        let buffer = emu.new_cstring("abcd")?;
        let output = emu.malloc(4)?;
        let iovec = emu.malloc(32)?;
        let width = if dir == "lib64" { 8 } else { 4 };
        for (index, value) in [buffer, 2, buffer + 2, 2].into_iter().enumerate() {
            emu.write_bytes(
                iovec + (index * width) as u64,
                &value.to_le_bytes()[..width],
            )?;
        }
        for (function, append) in [
            ("write", false),
            ("writev", false),
            ("pwrite64", false),
            ("write", true),
            ("writev", true),
            ("pwrite64", true),
        ] {
            assert_eq!(call(&mut emu, libc, "ftruncate", &[fd, 0])?, 0);
            assert_eq!(
                call(
                    &mut emu,
                    libc,
                    "fcntl",
                    &[fd, 4, if append { 0x400 } else { 0 }]
                )?,
                0
            );
            if append {
                assert_eq!(call(&mut emu, libc, "ftruncate", &[fd, limit - 3])?, 0);
            }
            for offset in [limit - 3, limit, limit + 1] {
                let position = if function == "pwrite64" || append {
                    7
                } else {
                    offset
                };
                assert_eq!(call(&mut emu, libc, "lseek", &[fd, position, 0])?, position);
                let args = match function {
                    "write" => vec![fd, buffer, 4],
                    "writev" => vec![fd, iovec, 2],
                    _ if dir == "lib64" => vec![fd, buffer, 4, offset],
                    _ => vec![fd, buffer, 4, 0, offset, 0],
                };
                let result = call(&mut emu, libc, function, &args)?;
                let expected_position = if offset == limit - 3 {
                    assert_eq!(result, 3, "{dir}: {function}, append={append}");
                    if function == "pwrite64" {
                        position
                    } else {
                        limit
                    }
                } else {
                    errno(&mut emu, libc, result, 27)?;
                    position
                };
                assert_eq!(
                    call(&mut emu, libc, "lseek", &[fd, 0, 1])?,
                    expected_position
                );
                assert_eq!(call(&mut emu, libc, "lseek", &[fd, 0, 2])?, limit);
                assert_eq!(
                    call(&mut emu, libc, "pread", &[fd, output, 4, limit - 4])?,
                    4
                );
                assert_eq!(emu.read_bytes(output, 4)?, b"\0abc");
            }
        }
        assert_eq!(call(&mut emu, libc, "ftruncate", &[fd, limit])?, 0);
    }
    Ok(())
}
