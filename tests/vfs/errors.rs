use super::{call, errno};
use unidroid::{Emulator, Result};

#[test]
fn file_operations_return_linux_errors() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.add_file("/cache/value", b"abc")?;
        emu.add_file("/other/child", b"x")?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let value = emu.new_cstring("/cache/value")?;
        let cache = emu.new_cstring("/cache")?;
        let other = emu.new_cstring("/other")?;
        let missing = emu.new_cstring("/missing/child")?;
        let maps = emu.new_cstring("/proc/self/maps")?;
        let random = emu.new_cstring("/dev/random")?;
        let cmdline = emu.new_cstring("/proc/self/cmdline")?;
        let buffer = emu.malloc(128)?;
        let ro = call(&mut emu, libc, "open", &[value, 0])?;
        let rw = call(&mut emu, libc, "open", &[value, 2])?;
        let wo = call(&mut emu, libc, "open", &[value, 1])?;
        let directory = call(&mut emu, libc, "open", &[cache, 0])?;
        assert_eq!(call(&mut emu, libc, "opendir", &[value])?, 0);
        let error = call(&mut emu, libc, "__errno", &[])?;
        assert_eq!(emu.read_bytes(error, 4)?, 20u32.to_le_bytes());
        let direct = call(&mut emu, libc, "open", &[value, 0x10000])?;
        assert_ne!(direct as u32, u32::MAX);
        assert_eq!(call(&mut emu, libc, "close", &[direct])?, 0);
        for (name, args, expected) in [
            ("open", vec![missing, 0x42, 0o600], 2),
            ("open", vec![value, 0xc2, 0o600], 17),
            ("open", vec![cache, 2], 21),
            ("open", vec![maps, 2], 13),
            ("open", vec![random, 1], 13),
            ("open", vec![cmdline, 1], 13),
            ("read", vec![directory, buffer, 1], 21),
            ("read", vec![wo, buffer, 1], 9),
            ("write", vec![ro, buffer, 1], 9),
            ("writev", vec![ro, buffer, 0], 9),
            ("pwrite", vec![ro, buffer, 1, 0], 9),
            ("ftruncate", vec![ro, 1], 22),
            ("ftruncate", vec![9999, 1], 9),
            ("ftruncate", vec![rw, u64::MAX], 22),
            ("fsync", vec![9999], 9),
            ("fdatasync", vec![9999], 9),
            ("mkdir", vec![cache, 0o700], 17),
            ("mkdir", vec![missing, 0o700], 2),
            ("unlink", vec![cache], 21),
            ("unlink", vec![missing], 2),
            ("rmdir", vec![value], 20),
            ("rmdir", vec![cache], 39),
            ("rename", vec![cache, other], 39),
            ("rename", vec![value, other], 21),
            ("rename", vec![cache, value], 22),
            ("rename", vec![other, value], 20),
            ("rename", vec![value, missing], 2),
            ("fcntl", vec![rw, 999], 22),
            ("fcntl", vec![rw, 0, u64::MAX], 22),
            ("fcntl", vec![9999, 1], 9),
            ("dup", vec![9999], 9),
            ("dup2", vec![9999, ro], 9),
            ("dup3", vec![rw, 50, 1], 22),
            ("readlink", vec![value, buffer, 128], 22),
            ("readlink", vec![missing, buffer, 128], 2),
            ("readlink", vec![maps, buffer, 0], 22),
        ] {
            let result = call(&mut emu, libc, name, &args)?;
            assert_eq!(result as u32, u32::MAX, "{dir}: {name} {args:?}");
            errno(&mut emu, libc, result, expected)?;
        }
        assert_eq!(call(&mut emu, libc, "fsync", &[ro])?, 0);
        assert_eq!(call(&mut emu, libc, "fsync", &[directory])?, 0);
        assert_eq!(emu.file("/cache/value")?, b"abc");
    }
    Ok(())
}

#[test]
fn unreadable_paths_return_efault() -> Result<()> {
    let cwd = (-100i64) as u64;
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.add_file("/value", b"abc")?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let path = emu.new_cstring("/value")?;
        let buffer = emu.malloc(128)?;
        for bad in [0, 0xdead0000] {
            let mut operations = vec![
                ("openat", vec![cwd, bad, 0]),
                ("fstatat", vec![cwd, bad, buffer, 0]),
                ("faccessat", vec![cwd, bad, 0, 0]),
                ("mkdirat", vec![cwd, bad, 0o700]),
                ("unlinkat", vec![cwd, bad, 0]),
                ("renameat", vec![cwd, bad, cwd, path]),
                ("renameat", vec![cwd, path, cwd, bad]),
                ("readlinkat", vec![cwd, bad, buffer, 128]),
            ];
            if dir == "lib64" {
                operations.extend([
                    ("syscall", vec![276, cwd, bad, cwd, path, 0]),
                    ("syscall", vec![276, cwd, path, cwd, bad, 0]),
                ]);
            }
            for (name, args) in operations {
                let result = call(&mut emu, libc, name, &args)?;
                errno(&mut emu, libc, result, 14)?;
            }
        }
    }
    Ok(())
}

#[test]
fn relative_paths_check_dirfds_for_every_at_operation() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.add_file("/cache/value", b"abc")?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let path = emu.new_cstring("/cache/value")?;
        let ro = call(&mut emu, libc, "open", &[path, 0])?;
        let relative = emu.new_cstring("value")?;
        let absolute = emu.new_cstring("/cache/renamed")?;
        let buffer = emu.malloc(128)?;
        for (fd, expected) in [(9999, 9), (ro, 20)] {
            for (name, args) in [
                ("openat", vec![fd, relative, 0]),
                ("fstatat", vec![fd, relative, buffer, 0]),
                ("faccessat", vec![fd, relative, 0, 0]),
                ("mkdirat", vec![fd, relative, 0o700]),
                ("unlinkat", vec![fd, relative, 0]),
                ("renameat", vec![fd, relative, (-100i64) as u64, absolute]),
                ("renameat", vec![(-100i64) as u64, path, fd, relative]),
                ("readlinkat", vec![fd, relative, buffer, 128]),
            ] {
                let result = call(&mut emu, libc, name, &args)?;
                assert_eq!(result as u32, u32::MAX, "{dir}: {name}");
                errno(&mut emu, libc, result, expected)?;
            }
        }
    }
    Ok(())
}
