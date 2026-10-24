use super::{call, errno};
use unidroid::{Emulator, Result};

#[test]
fn rewind_and_copy_restore_contents_whiteouts_offsets_flags_and_directory_position() -> Result<()> {
    for (dir, getdents) in [("lib64", 61), ("lib", 217)] {
        let mut emu = Emulator::builder().build()?;
        emu.add_file("/cache/value", b"abcdef")?;
        emu.add_host_file("/linked", format!("rootfs/{dir}/libdl.so"))?;
        let original = std::fs::read(format!("rootfs/{dir}/libdl.so"))?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let path = emu.new_cstring("/cache/value")?;
        let fd = call(&mut emu, libc, "open", &[path, 0x80002])?;
        let duplicate = call(&mut emu, libc, "dup", &[fd])?;
        let buffer = emu.malloc(128)?;
        assert_eq!(call(&mut emu, libc, "read", &[fd, buffer, 2])?, 2);
        let linked = emu.new_cstring("/linked")?;
        let host = call(&mut emu, libc, "open", &[linked, 0])?;
        assert_eq!(call(&mut emu, libc, "lseek", &[host, 7, 0])?, 7);
        let root = emu.new_cstring("/cache")?;
        let directory = call(&mut emu, libc, "open", &[root, 0])?;
        assert_eq!(
            call(
                &mut emu,
                libc,
                "syscall",
                &[getdents, directory, buffer, 24]
            )?,
            24
        );
        emu.remove_file(&format!("/{dir}/libdl.so"))?;
        assert_eq!(call(&mut emu, libc, "close", &[0])?, 0);
        let saved = emu.checkpoint()?;
        emu.write_bytes(buffer, b"changed")?;
        assert_eq!(call(&mut emu, libc, "write", &[fd, buffer, 7])?, 7);
        emu.remove_file("/cache/value")?;
        emu.add_file(&format!("/{dir}/libdl.so"), b"replacement")?;
        emu.add_file("/cache/later", b"later")?;
        assert_eq!(call(&mut emu, libc, "close", &[host])?, 0);
        assert_eq!(call(&mut emu, libc, "fcntl", &[fd, 4, 0xc00])?, 0);
        assert_eq!(call(&mut emu, libc, "fcntl", &[fd, 2, 0])?, 0);
        emu.rewind(&saved)?;
        let mut copy = Emulator::builder().build()?;
        copy.rewind(&saved)?;
        for current in [&mut emu, &mut copy] {
            assert_eq!(current.file("/cache/value")?, b"abcdef");
            assert!(current.file(&format!("/{dir}/libdl.so")).is_err());
            assert!(current.file("/cache/later").is_err());
            assert_eq!(call(current, libc, "fcntl", &[fd, 1])?, 1);
            assert_eq!(call(current, libc, "fcntl", &[fd, 3])?, 2);
            let result = call(current, libc, "fcntl", &[0, 1])?;
            errno(current, libc, result, 9)?;
            assert_eq!(call(current, libc, "read", &[duplicate, buffer, 2])?, 2);
            assert_eq!(current.read_bytes(buffer, 2)?, b"cd");
            assert_eq!(call(current, libc, "lseek", &[fd, 0, 1])?, 4);
            assert_eq!(call(current, libc, "read", &[host, buffer, 8])?, 8);
            assert_eq!(current.read_bytes(buffer, 8)?, original[7..15]);
            assert_eq!(
                call(current, libc, "syscall", &[getdents, directory, buffer, 24])?,
                24
            );
            assert_eq!(current.read_bytes(buffer + 19, 3)?, b"..\0");
            assert_eq!(call(current, libc, "open", &[path, 0])?, 0);
            current.write_bytes(buffer, b"!")?;
            assert_eq!(call(current, libc, "write", &[fd, buffer, 1])?, 1);
            assert_eq!(current.file("/cache/value")?, b"abcd!f");
        }
        emu.rewind(&saved)?;
        assert_eq!(emu.file("/cache/value")?, b"abcdef");
        assert_eq!(copy.file("/cache/value")?, b"abcd!f");
    }
    Ok(())
}
