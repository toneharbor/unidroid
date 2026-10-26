use super::{call, errno};
use unidroid::{Emulator, Module, Result};

fn listing(emu: &mut Emulator, libc: Module, path: &str) -> Result<Vec<String>> {
    let path = emu.new_cstring(path)?;
    let stream = call(emu, libc, "opendir", &[path])?;
    assert_ne!(stream, 0);
    let mut names = Vec::new();
    loop {
        let entry = call(emu, libc, "readdir", &[stream])?;
        if entry == 0 {
            break;
        }
        let bytes = emu.read_bytes(entry + 19, 256)?;
        names.push(String::from_utf8(
            bytes.into_iter().take_while(|b| *b != 0).collect(),
        )?);
    }
    assert_eq!(call(emu, libc, "closedir", &[stream])?, 0);
    Ok(names)
}

#[test]
fn proc_getdents64_lists_fd_as_a_directory() -> Result<()> {
    for (dir, nr) in [("lib64", 61), ("lib", 217)] {
        let mut emu = Emulator::builder().build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let buffer = emu.malloc(4096)?;
        for path in ["/proc/self", "/proc/1234"] {
            let path = emu.new_cstring(path)?;
            let fd = call(&mut emu, libc, "open", &[path, 0])?;
            let length = call(&mut emu, libc, "syscall", &[nr, fd, buffer, 4096])?;
            let bytes = emu.read_bytes(buffer, length as usize)?;
            let mut entries = Vec::new();
            let mut offset = 0;
            while offset < bytes.len() {
                let length =
                    u16::from_le_bytes(bytes[offset + 16..offset + 18].try_into()?) as usize;
                let name = bytes[offset + 19..offset + length]
                    .split(|byte| *byte == 0)
                    .next()
                    .unwrap();
                entries.push((name.to_vec(), bytes[offset + 18]));
                offset += length;
            }
            assert!(entries.contains(&(b"fd".to_vec(), 4)), "{entries:?}");
            assert_eq!(call(&mut emu, libc, "close", &[fd])?, 0);
        }
    }
    Ok(())
}

#[test]
fn directories_merge_sorted_entries_and_hide_removed_rootfs_paths() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        emu.add_file(&format!("/{dir}/z-added"), b"x")?;
        let removed = emu.new_cstring(&format!("/{dir}/libdl.so"))?;
        assert_eq!(call(&mut emu, libc, "unlink", &[removed])?, 0);
        let result = call(&mut emu, libc, "access", &[removed, 0])?;
        errno(&mut emu, libc, result, 2)?;
        let names = listing(&mut emu, libc, &format!("/{dir}"))?;
        assert_eq!(
            names,
            [
                ".",
                "..",
                "libc++.so",
                "libc.so",
                "libcrypto.so",
                "liblog.so",
                "libm.so",
                "libssl.so",
                "libstdc++.so",
                "libz.so",
                "z-added"
            ]
        );
        let path = emu.new_cstring("/cache")?;
        assert_eq!(call(&mut emu, libc, "mkdir", &[path, 0o750])?, 0);
        assert_eq!(listing(&mut emu, libc, "/cache")?, [".", ".."]);
        emu.add_file("/cache/value", b"value")?;
        let result = call(&mut emu, libc, "rmdir", &[path])?;
        errno(&mut emu, libc, result, 39)?;
        let file = emu.new_cstring("/cache/value")?;
        assert_eq!(call(&mut emu, libc, "unlink", &[file])?, 0);
        assert_eq!(call(&mut emu, libc, "rmdir", &[path])?, 0);
        let result = call(&mut emu, libc, "access", &[path, 0])?;
        errno(&mut emu, libc, result, 2)?;
    }
    Ok(())
}

#[test]
fn rename_copies_rootfs_files_and_directory_trees_without_host_changes() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let original = std::fs::read(format!("rootfs/{dir}/libdl.so"))?;
        let from = emu.new_cstring(&format!("/{dir}/libdl.so"))?;
        let to = emu.new_cstring("/moved")?;
        assert_eq!(call(&mut emu, libc, "rename", &[from, to])?, 0);
        assert_eq!(emu.file("/moved")?, original);
        assert!(emu.file(&format!("/{dir}/libdl.so")).is_err());
        assert_eq!(std::fs::read(format!("rootfs/{dir}/libdl.so"))?, original);
        emu.add_file("/tree/nested/value", b"saved")?;
        let from = emu.new_cstring("/tree")?;
        let to = emu.new_cstring("/new-tree")?;
        assert_eq!(call(&mut emu, libc, "rename", &[from, to])?, 0);
        assert_eq!(emu.file("/new-tree/nested/value")?, b"saved");
        assert!(emu.file("/tree/nested/value").is_err());
        // A rootfs directory is moved in full, including merged overlay entries.
        emu.add_file(&format!("/{dir}/extra"), b"extra")?;
        let from = emu.new_cstring(&format!("/{dir}"))?;
        let to = emu.new_cstring("/libraries")?;
        assert_eq!(call(&mut emu, libc, "rename", &[from, to])?, 0);
        assert_eq!(emu.file("/libraries/extra")?, b"extra");
        assert_eq!(
            emu.file("/libraries/libc.so")?,
            std::fs::read(format!("rootfs/{dir}/libc.so"))?
        );
        assert!(emu.file(&format!("/{dir}/extra")).is_err());
        assert!(emu.file("/libraries/libdl.so").is_err());
    }
    Ok(())
}
