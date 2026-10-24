use super::{call, errno};
use unidroid::{Emulator, Result};

#[test]
fn write_flags_pwrite_truncate_and_stdio_use_overlay_contents() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().fixed_clock(123).build()?;
        emu.add_file("/cache/value", b"abc")?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let path = emu.new_cstring("/cache/value")?;
        let buffer = emu.new_cstring("XY")?;
        let fd = call(&mut emu, libc, "open", &[path, 0x402])?;
        assert_eq!(call(&mut emu, libc, "lseek", &[fd, 0, 0])?, 0);
        assert_eq!(call(&mut emu, libc, "write", &[fd, buffer, 2])?, 2);
        assert_eq!(emu.file("/cache/value")?, b"abcXY");
        assert_eq!(call(&mut emu, libc, "close", &[fd])?, 0);
        let fd = call(&mut emu, libc, "open", &[path, 2])?;
        assert_eq!(call(&mut emu, libc, "pwrite", &[fd, buffer, 2, 1])?, 2);
        assert_eq!(call(&mut emu, libc, "lseek", &[fd, 0, 1])?, 0);
        assert_eq!(emu.file("/cache/value")?, b"aXYXY");
        assert_eq!(call(&mut emu, libc, "ftruncate", &[fd, 2])?, 0);
        assert_eq!(emu.file("/cache/value")?, b"aX");
        assert_eq!(call(&mut emu, libc, "ftruncate", &[fd, 4])?, 0);
        assert_eq!(emu.file("/cache/value")?, b"aX\0\0");
        assert_eq!(call(&mut emu, libc, "fsync", &[fd])?, 0);
        assert_eq!(call(&mut emu, libc, "fdatasync", &[fd])?, 0);
        let stat = emu.malloc(128)?;
        assert_eq!(call(&mut emu, libc, "fstat", &[fd, stat])?, 0);
        assert_eq!(emu.read_bytes(stat + 48, 8)?, 4u64.to_le_bytes());
        assert_eq!(emu.read_bytes(stat + 64, 8)?, 1u64.to_le_bytes());
        assert_eq!(emu.read_bytes(stat + 16, 4)?, 0o100644u32.to_le_bytes());
        assert_eq!(emu.read_bytes(stat + 72, 4)?, 123u32.to_le_bytes());
        let result = call(&mut emu, libc, "open", &[path, 0xc2, 0o600])?;
        errno(&mut emu, libc, result, 17)?;
        let truncated = call(&mut emu, libc, "open", &[path, 0x201])?;
        assert_eq!(emu.file("/cache/value")?, b"");
        assert_eq!(call(&mut emu, libc, "close", &[truncated])?, 0);
        let mode = emu.new_cstring("w+")?;
        let stream = call(&mut emu, libc, "fopen", &[path, mode])?;
        assert_ne!(stream, 0);
        assert_eq!(call(&mut emu, libc, "fwrite", &[buffer, 1, 2, stream])?, 2);
        assert_eq!(call(&mut emu, libc, "fseek", &[stream, 0, 0])?, 0);
        assert_eq!(call(&mut emu, libc, "fread", &[buffer, 1, 2, stream])?, 2);
        assert_eq!(emu.read_bytes(buffer, 2)?, b"XY");
        assert_eq!(call(&mut emu, libc, "fclose", &[stream])?, 0);
        assert_eq!(emu.file("/cache/value")?, b"XY");
    }
    Ok(())
}

#[test]
fn rootfs_and_host_linked_writes_leave_host_bytes_unchanged() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let host = format!("rootfs/{dir}/libdl.so");
        let original = std::fs::read(&host)?;
        let mut emu = Emulator::builder().build()?;
        emu.add_host_file("/packages/library", &host)?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let buffer = emu.new_cstring("patch")?;
        for guest in [format!("/{dir}/libdl.so"), "/packages/library".into()] {
            let path = emu.new_cstring(&guest)?;
            let reader = call(&mut emu, libc, "open", &[path, 0])?;
            let writer = call(&mut emu, libc, "open", &[path, 2])?;
            assert_eq!(call(&mut emu, libc, "write", &[writer, buffer, 5])?, 5);
            let bytes = emu.file(&guest)?;
            assert_eq!(&bytes[..5], b"patch");
            assert_eq!(&bytes[5..], &original[5..]);
            assert_eq!(call(&mut emu, libc, "read", &[reader, buffer, 5])?, 5);
            assert_eq!(emu.read_bytes(buffer, 5)?, b"patch");
            assert_eq!(std::fs::read(&host)?, original);
        }
    }
    Ok(())
}
