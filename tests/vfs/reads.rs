use super::{call, errno};
use unidroid::{Emulator, Result};

#[test]
fn failed_reads_preserve_shared_offsets() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.add_file("/value", b"abc")?;
        emu.add_host_file("/host", format!("rootfs/{dir}/libdl.so"))?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let buffer = emu.malloc(8)?;
        for (name, expected) in [
            ("/value", b'a'),
            ("/host", 0x7f),
            ("/proc/self/cmdline", b'u'),
        ] {
            let path = emu.new_cstring(name)?;
            let fd = call(&mut emu, libc, "open", &[path, 0])?;
            let duplicate = call(&mut emu, libc, "dup", &[fd])?;
            let result = call(&mut emu, libc, "read", &[fd, 0, 1])?;
            errno(&mut emu, libc, result, 14)?;
            assert_eq!(call(&mut emu, libc, "lseek", &[duplicate, 0, 1])?, 0);
            assert_eq!(call(&mut emu, libc, "read", &[duplicate, buffer, 1])?, 1);
            assert_eq!(emu.read_bytes(buffer, 1)?, [expected]);
            let args = if dir == "lib64" {
                vec![fd, 0, 1, 0]
            } else {
                vec![fd, 0, 1, 0, 0, 0]
            };
            let result = call(&mut emu, libc, "pread64", &args)?;
            errno(&mut emu, libc, result, 14)?;
            assert_eq!(call(&mut emu, libc, "lseek", &[duplicate, 0, 1])?, 1);
            assert_eq!(call(&mut emu, libc, "pread", &[fd, buffer, 1, 0])?, 1);
            assert_eq!(emu.read_bytes(buffer, 1)?, [expected]);
            assert_eq!(call(&mut emu, libc, "lseek", &[duplicate, 0, 1])?, 1);
        }
    }
    Ok(())
}

#[test]
fn failed_getdents_preserves_directory_position() -> Result<()> {
    for (dir, nr) in [("lib64", 61), ("lib", 217)] {
        let mut emu = Emulator::builder().build()?;
        emu.add_file("/directory/value", b"abc")?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let path = emu.new_cstring("/directory")?;
        let fd = call(&mut emu, libc, "open", &[path, 0])?;
        let duplicate = call(&mut emu, libc, "dup", &[fd])?;
        let buffer = emu.malloc(128)?;
        let result = call(&mut emu, libc, "syscall", &[nr, fd, 0, 24])?;
        errno(&mut emu, libc, result, 14)?;
        assert_eq!(
            call(&mut emu, libc, "syscall", &[nr, duplicate, buffer, 24])?,
            24
        );
        assert_eq!(emu.read_bytes(buffer + 19, 2)?, b".\0");
        let result = call(&mut emu, libc, "syscall", &[nr, fd, 0, 128])?;
        errno(&mut emu, libc, result, 14)?;
        assert_eq!(
            call(&mut emu, libc, "syscall", &[nr, duplicate, buffer, 24])?,
            24
        );
        assert_eq!(emu.read_bytes(buffer + 19, 3)?, b"..\0");
    }
    Ok(())
}
