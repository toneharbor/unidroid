use super::*;

pub(super) fn open_dir(emu: &mut Emulator, module: Module, name: &str) -> Result<u64> {
    let manager = call(emu, module, "AAssetManager_fromJava", &[0, 0])?;
    let name = emu.new_cstring(name)?;
    let result = call(emu, module, "AAssetManager_openDir", &[manager, name]);
    emu.free(name);
    result
}

pub(super) fn next(
    emu: &mut Emulator,
    module: Module,
    dir: u64,
    expected: Option<&str>,
) -> Result<()> {
    let name = call(emu, module, "AAssetDir_getNextFileName", &[dir])?;
    if let Some(expected) = expected {
        assert_ne!(name, 0);
        assert_eq!(
            emu.read_bytes(name, expected.len() + 1)?,
            format!("{expected}\0").as_bytes()
        );
    } else {
        assert_eq!(name, 0);
    }
    Ok(())
}

#[test]
fn native_asset_directories_sort_only_direct_files_and_rewind() -> Result<()> {
    for dir in ["lib", "lib64"] {
        let (_file, mut emu, module) = emulator(dir)?;
        for (path, names) in [
            ("", vec!["deflated.bin", "empty.bin", "stored.bin"]),
            ("dir", vec!["a.txt", "z.txt"]),
            ("dir/sub", vec!["deep.txt"]),
            ("missing", vec![]),
            ("stored.bin", vec![]),
        ] {
            let directory = open_dir(&mut emu, module, path)?;
            assert_ne!(directory, 0);
            for _ in 0..2 {
                for &name in &names {
                    next(&mut emu, module, directory, Some(name))?;
                }
                next(&mut emu, module, directory, None)?;
                next(&mut emu, module, directory, None)?;
                assert_eq!(call(&mut emu, module, "AAssetDir_rewind", &[directory])?, 0);
            }
            call(&mut emu, module, "AAssetDir_close", &[directory])?;
            let error = call(&mut emu, module, "AAssetDir_getNextFileName", &[directory])
                .unwrap_err()
                .to_string();
            assert!(error.contains("from 0x20000"), "{error}");
        }
    }
    Ok(())
}
