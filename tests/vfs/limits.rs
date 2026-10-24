use super::{TestRoot, call, errno};
use unidroid::{Emulator, Module, Result};

fn seek64(
    emu: &mut Emulator,
    libc: Module,
    fd: u64,
    offset: i64,
    whence: u64,
    arm32: bool,
) -> Result<i64> {
    let bits = offset as u64;
    if arm32 {
        // AAPCS aligns the 64-bit offset to r2:r3; whence is on the stack.
        let low = call(
            emu,
            libc,
            "lseek64",
            &[fd, 0, bits as u32 as u64, bits >> 32, whence],
        )?;
        Ok((low | (emu.register("r1")? << 32)) as i64)
    } else {
        Ok(call(emu, libc, "lseek64", &[fd, bits, whence])? as i64)
    }
}

#[test]
fn lseek64_seeks_past_four_gib_with_set_cur_and_end() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let arm32 = dir == "lib";
        let mut emu = Emulator::builder().build()?;
        emu.add_file("/value", b"abc")?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let path = emu.new_cstring("/value")?;
        let fd = call(&mut emu, libc, "open", &[path, 0])?;
        let buffer = emu.malloc(1)?;
        for (offset, whence, expected) in [
            (0x1_8000_0001, 0, 0x1_8000_0001),
            (0x1_0000_0002, 1, 0x2_8000_0003),
            (-0x1_0000_0001, 1, 0x1_8000_0002),
            (0x3_0000_0004, 2, 0x3_0000_0007),
        ] {
            assert_eq!(seek64(&mut emu, libc, fd, offset, whence, arm32)?, expected);
            assert_eq!(seek64(&mut emu, libc, fd, 0, 1, arm32)?, expected);
            assert_eq!(call(&mut emu, libc, "read", &[fd, buffer, 1])?, 0);
            assert_eq!(seek64(&mut emu, libc, fd, 0, 1, arm32)?, expected);
        }
        assert_eq!(emu.file("/value")?, b"abc");
    }
    Ok(())
}

#[test]
fn lseek64_preserves_host_generated_and_directory_seek_rules() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let arm32 = dir == "lib";
        let mut emu = Emulator::builder().process_name("test").build()?;
        emu.add_file("/directory/value", b"x")?;
        let host = format!("rootfs/{dir}/libdl.so");
        emu.add_host_file("/linked", &host)?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        for (name, length) in [
            ("/linked", std::fs::metadata(&host)?.len() as i64),
            ("/proc/self/cmdline", 5),
        ] {
            let path = emu.new_cstring(name)?;
            let fd = call(&mut emu, libc, "open", &[path, 0])?;
            assert_eq!(seek64(&mut emu, libc, fd, -1, 2, arm32)?, length - 1);
            assert_eq!(
                seek64(&mut emu, libc, fd, 0x1_0000_0000, 2, arm32)?,
                length + 0x1_0000_0000
            );
            assert_eq!(call(&mut emu, libc, "close", &[fd])?, 0);
        }
        let path = emu.new_cstring("/directory")?;
        let fd = call(&mut emu, libc, "open", &[path, 0])?;
        let buffer = emu.malloc(24)?;
        let getdents64 = if arm32 { 217 } else { 61 };
        assert_eq!(
            call(&mut emu, libc, "syscall", &[getdents64, fd, buffer, 24])?,
            24
        );
        let first = emu.read_bytes(buffer, 24)?;
        assert_eq!(seek64(&mut emu, libc, fd, 0, 0, arm32)?, 0);
        assert_eq!(
            call(&mut emu, libc, "syscall", &[getdents64, fd, buffer, 24])?,
            24
        );
        assert_eq!(emu.read_bytes(buffer, 24)?, first);
    }
    Ok(())
}

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
        // Keep the narrow lseek sign-extension check on both architectures.
        assert_eq!(call(&mut emu, libc, "lseek", &[fd, 1, 0])?, 1);
        let result = call(&mut emu, libc, "lseek", &[fd, u64::MAX, 0])?;
        errno(&mut emu, libc, result, 22)?;
        assert_eq!(call(&mut emu, libc, "lseek", &[fd, 0, 1])?, 1);
        let arm32 = dir == "lib";
        for (offset, whence) in [
            (0, 3),
            (-1, 0),
            (-2, 1),
            (-2, 2),
            (i64::MIN, 1),
            (i64::MAX, 1),
            (i64::MAX, 2),
        ] {
            let result = seek64(&mut emu, libc, fd, offset, whence, arm32)?;
            assert_eq!(result, -1);
            errno(&mut emu, libc, result as u64, 22)?;
            assert_eq!(seek64(&mut emu, libc, fd, 0, 1, arm32)?, 1);
        }
        assert_eq!(seek64(&mut emu, libc, fd, i64::MAX, 0, arm32)?, i64::MAX);
        let result = seek64(&mut emu, libc, fd, 1, 1, arm32)?;
        assert_eq!(result, -1);
        errno(&mut emu, libc, result as u64, 22)?;
        // The failed seek left INT64_MAX intact.
        assert_eq!(seek64(&mut emu, libc, fd, 0, 1, arm32)?, i64::MAX);
        assert_eq!(seek64(&mut emu, libc, fd, -i64::MAX, 1, arm32)?, 0);
    }
    Ok(())
}

#[test]
fn arm32_llseek_bad_result_pointer_moves_the_offset() -> Result<()> {
    let mut emu = Emulator::builder().build()?;
    emu.add_file("/value", b"abc")?;
    let libc = emu.load_library("rootfs/lib/libc.so")?;
    let path = emu.new_cstring("/value")?;
    let fd = call(&mut emu, libc, "open", &[path, 0])?;
    // mov r7, #140; ldr r4, [sp]; svc #0; bx lr
    emu.write_bytes(
        0x20100,
        &[0xe3a0708cu32, 0xe59d4000, 0xef000000, 0xe12fff1e]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>(),
    )?;
    assert!(emu.read_bytes(1, 8).is_err());
    assert_eq!(emu.call_function(0x20100, &[fd, 1, 7, 1, 0])? as i32, -14);
    assert_eq!(seek64(&mut emu, libc, fd, 0, 1, true)?, 0x1_0000_0007);
    Ok(())
}

#[test]
fn arm32_lseek_overflow_moves_the_offset() -> Result<()> {
    let mut emu = Emulator::builder().build()?;
    emu.add_file("/value", b"abc")?;
    let libc = emu.load_library("rootfs/lib/libc.so")?;
    let path = emu.new_cstring("/value")?;
    let fd = call(&mut emu, libc, "open", &[path, 0])?;
    assert_eq!(
        call(&mut emu, libc, "lseek", &[fd, 0x7fff_ffff, 0])?,
        0x7fff_ffff
    );
    let result = call(&mut emu, libc, "lseek", &[fd, 1, 1])?;
    errno(&mut emu, libc, result, 75)?;
    assert_eq!(seek64(&mut emu, libc, fd, 0, 1, true)?, 0x8000_0000);
    let result = call(&mut emu, libc, "lseek", &[fd, 0x7fff_ffff, 2])?;
    errno(&mut emu, libc, result, 75)?;
    assert_eq!(seek64(&mut emu, libc, fd, 0, 1, true)?, 0x8000_0002);
    // The narrow argument remains signed, even while the position is wide.
    assert_eq!(
        call(&mut emu, libc, "lseek", &[fd, (-3i32) as u32 as u64, 1])?,
        0x7fff_ffff
    );
    assert_eq!(seek64(&mut emu, libc, fd, 0, 1, true)?, 0x7fff_ffff);
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
