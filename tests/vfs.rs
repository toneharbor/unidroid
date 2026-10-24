use unidroid::{Emulator, Module, Result};

#[path = "vfs/checkpoint.rs"]
mod checkpoint;
#[path = "vfs/descriptors.rs"]
mod descriptors;
#[path = "vfs/directories.rs"]
mod directories;
#[path = "vfs/edges.rs"]
mod edges;
#[path = "vfs/errors.rs"]
mod errors;
#[path = "vfs/layers.rs"]
mod layers;
#[path = "vfs/limits.rs"]
mod limits;
#[path = "vfs/paths.rs"]
mod paths;
#[path = "vfs/serve.rs"]
mod serve;
#[path = "vfs/stat.rs"]
mod stat;
#[path = "vfs/trace.rs"]
mod trace;
#[path = "vfs/writes.rs"]
mod writes;

fn errno(emu: &mut Emulator, libc: Module, result: u64, expected: u32) -> Result<()> {
    assert_eq!(result as u32, u32::MAX);
    let address = call(emu, libc, "__errno", &[])?;
    assert_eq!(emu.read_bytes(address, 4)?, expected.to_le_bytes());
    Ok(())
}

fn call(emu: &mut Emulator, libc: Module, name: &str, args: &[u64]) -> Result<u64> {
    let function = emu.find_symbol(libc, name)?;
    emu.call_function(function, args)
}

#[test]
fn guest_creates_writes_reopens_and_reads_shared_contents() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.add_file("/cache/seed", b"")?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let path = emu.new_cstring("/cache/new")?;
        let buffer = emu.new_cstring("hello")?;
        let fd = call(&mut emu, libc, "open", &[path, 0x42, 0o640])?;
        assert_eq!(fd, 3);
        assert_eq!(call(&mut emu, libc, "write", &[fd, buffer, 5])?, 5);
        assert_eq!(emu.file("/cache/new")?, b"hello");
        let reader = call(&mut emu, libc, "open", &[path, 0])?;
        assert_eq!(call(&mut emu, libc, "read", &[reader, buffer, 2])?, 2);
        assert_eq!(emu.read_bytes(buffer, 2)?, b"he");
        assert_eq!(call(&mut emu, libc, "lseek", &[fd, 8, 0])?, 8);
        assert_eq!(call(&mut emu, libc, "write", &[fd, buffer, 2])?, 2);
        assert_eq!(call(&mut emu, libc, "read", &[reader, buffer, 8])?, 8);
        assert_eq!(emu.read_bytes(buffer, 8)?, b"llo\0\0\0he");
        assert_eq!(call(&mut emu, libc, "close", &[fd])?, 0);
        assert_eq!(call(&mut emu, libc, "close", &[reader])?, 0);
        let reopened = call(&mut emu, libc, "open", &[path, 0])?;
        assert_eq!(reopened, 3);
        let out = emu.malloc(16)?;
        assert_eq!(call(&mut emu, libc, "read", &[reopened, out, 16])?, 10);
        assert_eq!(emu.read_bytes(out, 10)?, b"hello\0\0\0he");
    }
    Ok(())
}

#[test]
fn host_files_work_before_loading_and_paths_are_normalized() -> Result<()> {
    let mut emu = Emulator::builder()
        .process_name("com.example.app")
        .build()?;
    emu.add_file("/config//nested/../value", b"one")?;
    assert_eq!(emu.file("/config/./value")?, b"one");
    emu.add_file("/config/value", b"two")?;
    assert_eq!(emu.file("/../../config/value")?, b"two");
    emu.add_host_file(
        "/packages/app.apk",
        "tests/fixtures/armeabi-v7a/libsignutil.so",
    )?;
    assert_eq!(
        emu.file("/packages/app.apk")?,
        std::fs::read("tests/fixtures/armeabi-v7a/libsignutil.so")?
    );
    assert_eq!(
        emu.file("/lib/libdl.so")?,
        std::fs::read("rootfs/lib/libdl.so")?
    );
    assert_eq!(emu.file("/proc/self/cmdline")?, b"com.example.app\0");
    assert_eq!(emu.file("/proc/1234/cmdline")?, b"com.example.app\0");
    assert_eq!(
        emu.file("/config").unwrap_err().to_string(),
        "/config: EISDIR"
    );
    emu.remove_file("/lib/libdl.so")?;
    assert_eq!(
        emu.file("/lib/libdl.so").unwrap_err().to_string(),
        "/lib/libdl.so: ENOENT"
    );
    emu.remove_file("/config")?;
    assert!(emu.file("/config/value").is_err());
    assert_eq!(
        emu.add_file("relative", b"").unwrap_err().to_string(),
        "relative: EINVAL (path must be absolute)"
    );
    assert_eq!(
        emu.file("relative").unwrap_err().to_string(),
        "relative: EINVAL (path must be absolute)"
    );
    assert_eq!(
        emu.remove_file("relative").unwrap_err().to_string(),
        "relative: EINVAL (path must be absolute)"
    );
    assert_eq!(
        emu.add_host_file("/missing", "tests/fixtures/no-file")
            .unwrap_err()
            .to_string(),
        "add_host_file /missing: tests/fixtures/no-file: No such file or directory (os error 2)"
    );
    emu.add_file("/parent", b"")?;
    assert_eq!(
        emu.add_file("/parent/child", b"").unwrap_err().to_string(),
        "/parent/child: ENOTDIR"
    );
    assert_eq!(
        emu.add_host_file("/parent/child", "rootfs/lib/libdl.so")
            .unwrap_err()
            .to_string(),
        "/parent/child: ENOTDIR"
    );
    Ok(())
}
