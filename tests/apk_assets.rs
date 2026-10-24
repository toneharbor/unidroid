#[path = "support/apk.rs"]
mod support;
use support::TestApk;
use unidroid::{Emulator, Module, Result};
use zip::CompressionMethod::{Deflated, Stored};

#[path = "apk_assets/directories.rs"]
mod directories;
#[path = "apk_assets/imports.rs"]
mod imports;
#[path = "apk_assets/seeks.rs"]
mod seeks;
#[path = "apk_assets/state.rs"]
mod state;

fn call(emu: &mut Emulator, module: Module, name: &str, args: &[u64]) -> Result<u64> {
    let function = emu.find_symbol(module, name)?;
    emu.call_function(function, args)
}

fn emulator(dir: &str) -> Result<(TestApk, Emulator, Module)> {
    let file = TestApk::assets(&[
        ("stored.bin", b"abcdef", Stored),
        ("deflated.bin", b"0123456789", Deflated),
        ("empty.bin", b"", Stored),
        ("dir/z.txt", b"z", Stored),
        ("dir/a.txt", b"a", Deflated),
        ("dir/sub/deep.txt", b"deep", Stored),
    ])?;
    let mut emu = Emulator::builder().apk(&file.path).build()?;
    let module = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
    Ok((file, emu, module))
}

fn open(emu: &mut Emulator, module: Module, name: &str, mode: u64) -> Result<u64> {
    let manager = call(emu, module, "AAssetManager_fromJava", &[0, 0])?;
    assert_ne!(manager, 0);
    assert_eq!(
        call(emu, module, "AAssetManager_fromJava", &[0, 0])?,
        manager
    );
    let name = emu.new_cstring(name)?;
    let result = call(emu, module, "AAssetManager_open", &[manager, name, mode]);
    emu.free(name);
    result
}

#[test]
fn native_assets_open_stored_and_deflated_entries_read_buffers_and_close() -> Result<()> {
    for dir in ["lib", "lib64"] {
        let (_file, mut emu, module) = emulator(dir)?;
        let buffer = emu.malloc(32)?;
        for mode in [0, 1, 2, 3, 99] {
            for (name, bytes) in [
                ("stored.bin", b"abcdef".as_slice()),
                ("deflated.bin", b"0123456789"),
                ("empty.bin", b""),
            ] {
                let asset = open(&mut emu, module, name, mode)?;
                assert_ne!(asset, 0);
                for function in [
                    "AAsset_getLength",
                    "AAsset_getLength64",
                    "AAsset_getRemainingLength",
                    "AAsset_getRemainingLength64",
                ] {
                    assert_eq!(
                        call(&mut emu, module, function, &[asset])?,
                        bytes.len() as u64
                    );
                }
                let contents = call(&mut emu, module, "AAsset_getBuffer", &[asset])?;
                assert_ne!(contents, 0);
                assert_eq!(emu.read_bytes(contents, bytes.len())?, bytes);
                assert_eq!(
                    call(&mut emu, module, "AAsset_read", &[asset, buffer, 0])?,
                    0
                );
                let count = call(&mut emu, module, "AAsset_read", &[asset, buffer, 2])?;
                assert_eq!(count, bytes.len().min(2) as u64);
                assert_eq!(
                    emu.read_bytes(buffer, count as usize)?,
                    bytes[..count as usize]
                );
                for function in ["AAsset_getRemainingLength", "AAsset_getRemainingLength64"] {
                    assert_eq!(
                        call(&mut emu, module, function, &[asset])?,
                        bytes.len() as u64 - count
                    );
                }
                let rest = call(&mut emu, module, "AAsset_read", &[asset, buffer, 32])?;
                assert_eq!(
                    emu.read_bytes(buffer, rest as usize)?,
                    bytes[count as usize..]
                );
                assert_eq!(
                    call(&mut emu, module, "AAsset_read", &[asset, buffer, 1])?,
                    0
                );
                assert_eq!(call(&mut emu, module, "AAsset_close", &[asset])?, 0);
                let error = call(&mut emu, module, "AAsset_getLength", &[asset])
                    .unwrap_err()
                    .to_string();
                assert!(error.contains("from 0x20000"), "{error}");
            }
        }
        assert_eq!(open(&mut emu, module, "missing.bin", 0)?, 0);
        assert_eq!(open(&mut emu, module, "dir", 0)?, 0);
    }
    Ok(())
}
