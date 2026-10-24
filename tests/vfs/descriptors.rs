use super::{call, errno};
use unidroid::{Emulator, Result};

#[test]
fn dup_and_fcntl_share_offsets_and_status_but_not_close_on_exec() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.add_file("/value", b"abcdef")?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let path = emu.new_cstring("/value")?;
        let buffer = emu.malloc(8)?;
        let fd = call(&mut emu, libc, "open", &[path, 0x80002])?;
        assert_eq!(fd, 3);
        assert_eq!(call(&mut emu, libc, "fcntl", &[fd, 1])?, 1);
        let dup = call(&mut emu, libc, "dup", &[fd])?;
        assert_eq!(dup, 4);
        assert_eq!(call(&mut emu, libc, "fcntl", &[dup, 1])?, 0);
        assert_eq!(call(&mut emu, libc, "read", &[fd, buffer, 2])?, 2);
        assert_eq!(call(&mut emu, libc, "read", &[dup, buffer, 2])?, 2);
        assert_eq!(emu.read_bytes(buffer, 2)?, b"cd");
        assert_eq!(call(&mut emu, libc, "fcntl", &[dup, 4, 0xc00])?, 0);
        assert_eq!(call(&mut emu, libc, "fcntl", &[fd, 3])?, 0xc02);
        assert_eq!(call(&mut emu, libc, "fcntl", &[dup, 2, 1])?, 0);
        assert_eq!(call(&mut emu, libc, "fcntl", &[fd, 2, 0])?, 0);
        assert_eq!(call(&mut emu, libc, "fcntl", &[dup, 1])?, 1);
        assert_eq!(call(&mut emu, libc, "fcntl", &[fd, 0, 8])?, 8);
        assert_eq!(call(&mut emu, libc, "fcntl", &[fd, 1030, 8])?, 9);
        assert_eq!(call(&mut emu, libc, "fcntl", &[9, 1])?, 1);
        assert_eq!(call(&mut emu, libc, "dup2", &[fd, 8])?, 8);
        assert_eq!(call(&mut emu, libc, "fcntl", &[8, 1])?, 0);
        assert_eq!(call(&mut emu, libc, "lseek", &[8, 0, 1])?, 4);
        assert_eq!(call(&mut emu, libc, "dup2", &[fd, fd])?, fd);
        let result = call(&mut emu, libc, "dup3", &[fd, fd, 0])?;
        errno(&mut emu, libc, result, 22)?;
        assert_eq!(call(&mut emu, libc, "dup3", &[fd, 12, 0x80000])?, 12);
        assert_eq!(call(&mut emu, libc, "fcntl", &[12, 1])?, 1);
        assert_eq!(call(&mut emu, libc, "close", &[dup])?, 0);
        assert_eq!(call(&mut emu, libc, "open", &[path, 0])?, dup);
        for standard in 0..3 {
            assert_eq!(
                call(&mut emu, libc, "fcntl", &[standard, 3])?,
                u64::from(standard != 0)
            );
            let copy = call(&mut emu, libc, "dup", &[standard])?;
            assert_eq!(call(&mut emu, libc, "close", &[copy])?, 0);
            assert_eq!(call(&mut emu, libc, "close", &[standard])?, 0);
            let result = call(&mut emu, libc, "close", &[standard])?;
            errno(&mut emu, libc, result, 9)?;
            assert_eq!(call(&mut emu, libc, "open", &[path, 0])?, standard);
        }
    }
    Ok(())
}

#[test]
fn proc_links_return_normalized_paths_without_nul_and_cmdline_snapshots() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder()
            .process_name("com.example.app")
            .build()?;
        emu.add_file("/cache/value", b"saved")?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let name = emu.new_cstring("//cache/./sub/../value")?;
        let fd = call(&mut emu, libc, "open", &[name, 0])?;
        let buffer = emu.malloc(128)?;
        for path in [format!("/proc/self/fd/{fd}"), format!("/proc/1234/fd/{fd}")] {
            let path = emu.new_cstring(&path)?;
            for size in [5, 128] {
                emu.write_bytes(buffer, &[0xa5; 128])?;
                let length = call(&mut emu, libc, "readlink", &[path, buffer, size])?;
                assert_eq!(length, size.min(12));
                assert_eq!(
                    emu.read_bytes(buffer, length as usize)?,
                    b"/cache/value"[..length as usize]
                );
                assert_eq!(emu.read_bytes(buffer + length, 1)?, [0xa5]);
            }
        }
        let path = emu.new_cstring("/proc/self/exe")?;
        let expected = if dir == "lib64" {
            "/system/bin/app_process64"
        } else {
            "/system/bin/app_process32"
        };
        assert_eq!(
            call(&mut emu, libc, "readlink", &[path, buffer, 128])?,
            expected.len() as u64
        );
        assert_eq!(emu.read_bytes(buffer, expected.len())?, expected.as_bytes());
        for path in ["/proc/self/fd/999", "/missing"] {
            let path = emu.new_cstring(path)?;
            let result = call(&mut emu, libc, "readlink", &[path, buffer, 128])?;
            errno(&mut emu, libc, result, 2)?;
        }
        let result = call(&mut emu, libc, "readlink", &[name, buffer, 128])?;
        errno(&mut emu, libc, result, 22)?;
        for path in ["/proc/self/cmdline", "/proc/1234/cmdline"] {
            let path = emu.new_cstring(path)?;
            let fd = call(&mut emu, libc, "open", &[path, 0])?;
            assert_eq!(call(&mut emu, libc, "read", &[fd, buffer, 128])?, 16);
            assert_eq!(emu.read_bytes(buffer, 16)?, b"com.example.app\0");
        }
        assert_eq!(call(&mut emu, libc, "close", &[fd])?, 0);
        let path = emu.new_cstring(&format!("/proc/self/fd/{fd}"))?;
        let result = call(&mut emu, libc, "readlink", &[path, buffer, 128])?;
        errno(&mut emu, libc, result, 2)?;
    }
    Ok(())
}
