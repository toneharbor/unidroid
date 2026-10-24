use super::{call, errno};
use unidroid::{Emulator, Result};

#[test]
fn open_descriptors_follow_renamed_paths() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.add_file("/a/sub/value", b"abc")?;
        emu.add_file("/ab/value", b"stay")?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let mut fds = Vec::new();
        for name in ["/a", "/a/sub", "/a/sub/value", "/ab/value"] {
            let path = emu.new_cstring(name)?;
            fds.push(call(&mut emu, libc, "open", &[path, 0])?);
        }
        fds.push(call(&mut emu, libc, "dup", &[fds[0]])?);
        let from = emu.new_cstring("/a")?;
        let to = emu.new_cstring("/b")?;
        assert_eq!(call(&mut emu, libc, "rename", &[from, to])?, 0);
        assert_eq!(call(&mut emu, libc, "mkdir", &[from, 0o700])?, 0);
        let child = emu.new_cstring("x")?;
        let fd = call(&mut emu, libc, "openat", &[fds[4], child, 0x42, 0o600])?;
        assert_ne!(fd as u32, u32::MAX);
        assert_eq!(emu.file("/b/x")?, b"");
        assert!(emu.file("/a/x").is_err());
        let buffer = emu.malloc(128)?;
        for (fd, expected) in
            fds.into_iter()
                .zip(["/b", "/b/sub", "/b/sub/value", "/ab/value", "/b"])
        {
            let link = emu.new_cstring(&format!("/proc/self/fd/{fd}"))?;
            assert_eq!(
                call(&mut emu, libc, "readlink", &[link, buffer, 128])?,
                expected.len() as u64
            );
            assert_eq!(emu.read_bytes(buffer, expected.len())?, expected.as_bytes());
        }
    }
    Ok(())
}

#[test]
fn at_operations_resolve_directory_fds_and_absolute_paths_ignore_them() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.add_file("/cache/value", b"abc")?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let path = emu.new_cstring("/cache")?;
        let directory = call(&mut emu, libc, "open", &[path, 0])?;
        let relative = emu.new_cstring("./sub/../value")?;
        let fd = call(&mut emu, libc, "openat", &[directory, relative, 0])?;
        let buffer = emu.malloc(128)?;
        assert_eq!(call(&mut emu, libc, "read", &[fd, buffer, 3])?, 3);
        assert_eq!(emu.read_bytes(buffer, 3)?, b"abc");
        assert_eq!(
            call(&mut emu, libc, "fstatat", &[directory, relative, buffer, 0])?,
            0
        );
        assert_eq!(emu.read_bytes(buffer + 48, 8)?, 3u64.to_le_bytes());
        assert_eq!(
            call(&mut emu, libc, "faccessat", &[directory, relative, 0, 0])?,
            0
        );
        let sub = emu.new_cstring("sub")?;
        assert_eq!(
            call(&mut emu, libc, "mkdirat", &[directory, sub, 0o700])?,
            0
        );
        let target = emu.new_cstring("sub/renamed")?;
        assert_eq!(
            call(
                &mut emu,
                libc,
                "renameat",
                &[directory, relative, directory, target]
            )?,
            0
        );
        assert_eq!(emu.file("/cache/sub/renamed")?, b"abc");
        assert_eq!(
            call(&mut emu, libc, "unlinkat", &[directory, target, 0])?,
            0
        );
        assert_eq!(
            call(&mut emu, libc, "unlinkat", &[directory, sub, 0x200])?,
            0
        );
        let cwd = emu.new_cstring("../../cache")?;
        assert_eq!(call(&mut emu, libc, "access", &[cwd, 0])?, 0);
        let absolute = emu.new_cstring("/cache")?;
        assert!(call(&mut emu, libc, "openat", &[9999, absolute, 0])? < u32::MAX as u64);
        let proc = emu.new_cstring("/proc/self")?;
        let procfd = call(&mut emu, libc, "open", &[proc, 0])?;
        let link = emu.new_cstring(&format!("fd/{directory}"))?;
        assert_eq!(
            call(&mut emu, libc, "readlinkat", &[procfd, link, buffer, 128])?,
            6
        );
        assert_eq!(emu.read_bytes(buffer, 6)?, b"/cache");
    }
    Ok(())
}

#[test]
fn getdents_has_aligned_records_snapshot_position_and_restart() -> Result<()> {
    for (dir, nr) in [("lib64", 61), ("lib", 217)] {
        let mut emu = Emulator::builder().build()?;
        emu.add_file("/listing/b", b"b")?;
        emu.add_file("/listing/a", b"a")?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let path = emu.new_cstring("/listing")?;
        let fd = call(&mut emu, libc, "open", &[path, 0])?;
        emu.add_file("/listing/later", b"late")?;
        let buffer = emu.malloc(128)?;
        let result = call(&mut emu, libc, "syscall", &[nr, fd, buffer, 23])?;
        errno(&mut emu, libc, result, 22)?;
        for (index, name) in [".", "..", "a", "b"].into_iter().enumerate() {
            emu.write_bytes(buffer, &[0xa5; 128])?;
            assert_eq!(call(&mut emu, libc, "syscall", &[nr, fd, buffer, 47])?, 24);
            assert_eq!(
                emu.read_bytes(buffer + 8, 8)?,
                (index as i64 + 1).to_le_bytes()
            );
            assert_eq!(emu.read_bytes(buffer + 16, 2)?, 24u16.to_le_bytes());
            assert_eq!(
                emu.read_bytes(buffer + 18, 1)?,
                [if index < 2 { 4 } else { 8 }]
            );
            assert_eq!(emu.read_bytes(buffer + 19, name.len())?, name.as_bytes());
            assert_eq!(emu.read_bytes(buffer + 19 + name.len() as u64, 1)?, [0]);
            assert_eq!(emu.read_bytes(buffer + 24, 1)?, [0xa5]);
        }
        assert_eq!(call(&mut emu, libc, "syscall", &[nr, fd, buffer, 128])?, 0);
        assert_eq!(call(&mut emu, libc, "lseek", &[fd, 0, 0])?, 0);
        assert_eq!(call(&mut emu, libc, "syscall", &[nr, fd, buffer, 128])?, 96);
        let file = emu.new_cstring("/listing/a")?;
        let file = call(&mut emu, libc, "open", &[file, 0])?;
        let result = call(&mut emu, libc, "syscall", &[nr, file, buffer, 128])?;
        errno(&mut emu, libc, result, 20)?;
        let result = call(&mut emu, libc, "syscall", &[nr, 9999, buffer, 128])?;
        errno(&mut emu, libc, result, 9)?;
    }
    Ok(())
}
