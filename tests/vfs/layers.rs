use super::{call, errno};
use unidroid::{Emulator, Result};

#[test]
fn overlay_precedes_generated_files_and_whiteouts_hide_lower_subtrees() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        emu.add_file("/proc/self/cmdline", b"override")?;
        let path = emu.new_cstring("/proc/self/cmdline")?;
        let fd = call(&mut emu, libc, "open", &[path, 0])?;
        let buffer = emu.malloc(128)?;
        assert_eq!(call(&mut emu, libc, "read", &[fd, buffer, 128])?, 8);
        assert_eq!(emu.read_bytes(buffer, 8)?, b"override");
        emu.add_file("/proc/self/fd", b"regular")?;
        let blocked_link = emu.new_cstring(&format!("/proc/self/fd/{fd}"))?;
        let result = call(&mut emu, libc, "readlink", &[blocked_link, buffer, 128])?;
        errno(&mut emu, libc, result, 20)?;
        emu.remove_file("/proc")?;
        assert!(emu.file("/proc/self/cmdline").is_err());
        emu.add_file("/proc/self/added", b"new")?;
        assert_eq!(emu.file("/proc/self/added")?, b"new");
        assert!(emu.file("/proc/self/cmdline").is_err());
        let link = emu.new_cstring("/proc/self/exe")?;
        let result = call(&mut emu, libc, "readlink", &[link, buffer, 128])?;
        errno(&mut emu, libc, result, 2)?;
        emu.add_file("/proc/self/fd", b"regular")?;
        let link = emu.new_cstring(&format!("/proc/self/fd/{fd}"))?;
        let result = call(&mut emu, libc, "readlink", &[link, buffer, 128])?;
        // The whiteout still hides lower generated links.
        errno(&mut emu, libc, result, 2)?;
        let path = emu.new_cstring("/dev")?;
        let dev = call(&mut emu, libc, "opendir", &[path])?;
        let mut names = Vec::new();
        loop {
            let entry = call(&mut emu, libc, "readdir", &[dev])?;
            if entry == 0 {
                break;
            }
            let name = emu
                .read_bytes(entry + 19, 32)?
                .into_iter()
                .take_while(|b| *b != 0)
                .collect::<Vec<_>>();
            names.push(String::from_utf8(name)?);
        }
        assert_eq!(names, [".", "..", "random", "urandom"]);
        assert_eq!(call(&mut emu, libc, "closedir", &[dev])?, 0);
    }
    Ok(())
}

#[test]
fn rename_replaces_targets_but_open_files_keep_their_contents() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.add_file("/source", b"source")?;
        emu.add_file("/target", b"target")?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let source = emu.new_cstring("/source")?;
        let target = emu.new_cstring("/target")?;
        let old_target = call(&mut emu, libc, "open", &[target, 2])?;
        let old_source = call(&mut emu, libc, "open", &[source, 2])?;
        let buffer = emu.malloc(16)?;
        assert_eq!(call(&mut emu, libc, "rename", &[source, target])?, 0);
        assert_eq!(emu.file("/target")?, b"source");
        assert_eq!(call(&mut emu, libc, "read", &[old_target, buffer, 16])?, 6);
        assert_eq!(emu.read_bytes(buffer, 6)?, b"target");
        emu.write_bytes(buffer, b"S")?;
        assert_eq!(call(&mut emu, libc, "write", &[old_source, buffer, 1])?, 1);
        assert_eq!(emu.file("/target")?, b"Source");
        assert_eq!(call(&mut emu, libc, "unlink", &[target])?, 0);
        assert_eq!(call(&mut emu, libc, "lseek", &[old_source, 0, 0])?, 0);
        assert_eq!(call(&mut emu, libc, "read", &[old_source, buffer, 16])?, 6);
        assert_eq!(emu.read_bytes(buffer, 6)?, b"Source");
        assert!(emu.file("/target").is_err());
        let empty = emu.new_cstring("/empty")?;
        let from = emu.new_cstring("/tree")?;
        emu.add_file("/tree/value", b"child")?;
        assert_eq!(call(&mut emu, libc, "mkdir", &[empty, 0o700])?, 0);
        assert_eq!(call(&mut emu, libc, "rename", &[from, empty])?, 0);
        assert_eq!(emu.file("/empty/value")?, b"child");
    }
    Ok(())
}
