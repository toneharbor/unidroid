use super::directories::{next, open_dir};
use super::*;

#[test]
fn checkpoint_restores_open_assets_buffers_and_directory_positions_into_fresh_emulators()
-> Result<()> {
    for dir in ["lib", "lib64"] {
        let (_file, mut emu, module) = emulator(dir)?;
        let stored = open(&mut emu, module, "stored.bin", 0)?;
        let deflated = open(&mut emu, module, "deflated.bin", 0)?;
        let buffer = emu.malloc(16)?;
        call(&mut emu, module, "AAsset_read", &[stored, buffer, 2])?;
        call(&mut emu, module, "AAsset_read", &[deflated, buffer, 3])?;
        let directory = open_dir(&mut emu, module, "dir")?;
        next(&mut emu, module, directory, Some("a.txt"))?;
        let manager = call(&mut emu, module, "AAssetManager_fromJava", &[0, 0])?;
        let contents = call(&mut emu, module, "AAsset_getBuffer", &[deflated])?;
        let saved = emu.checkpoint()?;
        call(&mut emu, module, "AAsset_close", &[stored])?;
        call(&mut emu, module, "AAsset_read", &[deflated, buffer, 5])?;
        emu.write_bytes(contents, b"xxxxxxxxxx")?;
        next(&mut emu, module, directory, Some("z.txt"))?;
        call(&mut emu, module, "AAssetDir_close", &[directory])?;
        let mut fresh = Emulator::builder().build()?;
        for restored in [&mut emu, &mut fresh] {
            restored.rewind(&saved)?;
            assert_eq!(
                call(restored, module, "AAssetManager_fromJava", &[0, 0])?,
                manager
            );
            assert_eq!(
                call(restored, module, "AAsset_read", &[stored, buffer, 16])?,
                4
            );
            assert_eq!(restored.read_bytes(buffer, 4)?, b"cdef");
            assert_eq!(
                call(restored, module, "AAsset_read", &[deflated, buffer, 16])?,
                7
            );
            assert_eq!(restored.read_bytes(buffer, 7)?, b"3456789");
            assert_eq!(restored.read_bytes(contents, 10)?, b"0123456789");
            assert_eq!(restored.read_bytes(directory, 6)?, b"a.txt\0");
            next(restored, module, directory, Some("z.txt"))?;
            next(restored, module, directory, None)?;
            call(restored, module, "AAsset_close", &[stored])?;
            call(restored, module, "AAsset_close", &[deflated])?;
            call(restored, module, "AAssetDir_close", &[directory])?;
        }
        // The saved positions remain reusable after both copies have advanced and closed.
        fresh.rewind(&saved)?;
        next(&mut fresh, module, directory, Some("z.txt"))?;
        assert_eq!(
            call(&mut fresh, module, "AAsset_getRemainingLength", &[stored])?,
            4
        );
    }
    Ok(())
}
