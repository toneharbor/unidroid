use super::{TestRoot, call, errno};
use unidroid::{Emulator, Result};

#[test]
fn truncation_copies_host_metadata_without_reading_contents_and_rejects_generated_files()
-> Result<()> {
    use std::os::unix::fs::MetadataExt;
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        let host = format!("rootfs/{dir}/libdl.so");
        emu.add_host_file("/linked", &host)?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let path = emu.new_cstring("/linked")?;
        let fd = call(&mut emu, libc, "open", &[path, 0x201])?;
        let stat = emu.malloc(128)?;
        assert_eq!(call(&mut emu, libc, "fstat", &[fd, stat])?, 0);
        assert_eq!(
            emu.read_bytes(stat + 16, 4)?,
            std::fs::metadata(&host)?.mode().to_le_bytes()
        );
        assert_eq!(emu.file("/linked")?, b"");
        for (path, expected) in [
            ("/proc/self/maps", 13),
            ("/proc/self/cmdline", 13),
            ("/dev/random", 13),
            ("/lib", 21),
        ] {
            let path = emu.new_cstring(path)?;
            let result = call(&mut emu, libc, "open", &[path, 0x200])?;
            errno(&mut emu, libc, result, expected)?;
        }
    }
    Ok(())
}

#[test]
fn writes_and_writev_share_the_io_limit_and_update_overlay_files() -> Result<()> {
    for (dir, width) in [("lib64", 8), ("lib", 4)] {
        let mut emu = Emulator::builder().build()?;
        emu.add_file("/value", b"")?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let path = emu.new_cstring("/value")?;
        let fd = call(&mut emu, libc, "open", &[path, 2])?;
        let limit = 33_554_431u64;
        let buffer = emu.malloc(limit as usize + 1)?;
        emu.write_bytes(buffer, b"A")?;
        emu.write_bytes(buffer + limit - 1, b"XY")?;
        let iov = emu.malloc(width * 4)?;
        let vectors = [buffer, limit + 1, 0, 1]
            .into_iter()
            .flat_map(|word| word.to_le_bytes()[..width].to_vec())
            .collect::<Vec<_>>();
        emu.write_bytes(iov, &vectors)?;
        for (name, args) in [
            ("write", vec![fd, buffer, limit + 1]),
            ("writev", vec![fd, iov, 2]),
            ("pwrite", vec![fd, buffer, limit + 1, 0]),
        ] {
            assert_eq!(call(&mut emu, libc, "ftruncate", &[fd, 0])?, 0);
            assert_eq!(call(&mut emu, libc, "lseek", &[fd, 0, 0])?, 0);
            assert_eq!(call(&mut emu, libc, name, &args)?, limit);
            let bytes = emu.file("/value")?;
            assert_eq!(bytes.len(), limit as usize);
            assert_eq!(bytes[0], b'A');
            assert_eq!(bytes[bytes.len() - 1], b'X');
        }
    }
    Ok(())
}

#[test]
fn rootfs_links_report_target_text_and_directory_link_types() -> Result<()> {
    use std::os::unix::fs::symlink;
    for (dir, nr) in [("lib64", 61), ("lib", 217)] {
        let sandbox = TestRoot::new("rootfs-links")?;
        let root = &sandbox.0;
        std::fs::copy(format!("rootfs/{dir}/libdl.so"), root.join("file"))?;
        symlink("file", root.join("link"))?;
        symlink("missing", root.join("dangling"))?;
        let mut emu = Emulator::builder().rootfs(root).build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let buffer = emu.malloc(256)?;
        for (name, target) in [
            ("/link", b"file".as_slice()),
            ("/dangling", b"missing".as_slice()),
        ] {
            let path = emu.new_cstring(name)?;
            emu.write_bytes(buffer, &[0xa5; 256])?;
            assert_eq!(
                call(&mut emu, libc, "readlink", &[path, buffer, 256])?,
                target.len() as u64
            );
            assert_eq!(emu.read_bytes(buffer, target.len())?, target);
            assert_eq!(emu.read_bytes(buffer + target.len() as u64, 1)?, [0xa5]);
        }
        let rootpath = emu.new_cstring("/")?;
        let fd = call(&mut emu, libc, "open", &[rootpath, 0])?;
        let length = call(&mut emu, libc, "syscall", &[nr, fd, buffer, 256])?;
        let bytes = emu.read_bytes(buffer, length as usize)?;
        let mut offset = 0;
        let mut links = Vec::new();
        while offset < bytes.len() {
            let length = u16::from_le_bytes(bytes[offset + 16..offset + 18].try_into()?) as usize;
            if bytes[offset + 18] == 10 {
                links.push(String::from_utf8(
                    bytes[offset + 19..offset + length]
                        .iter()
                        .copied()
                        .take_while(|byte| *byte != 0)
                        .collect(),
                )?);
            }
            offset += length;
        }
        assert_eq!(links, ["dangling", "link"]);
        emu.remove_file("/link")?;
        let path = emu.new_cstring("/link")?;
        let result = call(&mut emu, libc, "readlink", &[path, buffer, 256])?;
        errno(&mut emu, libc, result, 2)?;
        emu.add_file("/link", b"overlay")?;
        let result = call(&mut emu, libc, "readlink", &[path, buffer, 256])?;
        errno(&mut emu, libc, result, 22)?;
        drop(emu);
    }
    Ok(())
}
