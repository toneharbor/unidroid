use super::*;

fn wide(emu: &mut Emulator, module: Module, name: &str, args: &[u64], arm32: bool) -> Result<i64> {
    let low = call(emu, module, name, args)?;
    let high = if arm32 {
        emu.registers()?
            .into_iter()
            .find(|(name, _)| *name == "r1")
            .unwrap()
            .1
            << 32
    } else {
        0
    };
    Ok((low | high) as i64)
}

pub(super) fn seek64(
    emu: &mut Emulator,
    module: Module,
    asset: u64,
    offset: i64,
    whence: u64,
    arm32: bool,
) -> Result<i64> {
    let bits = offset as u64;
    let args = if arm32 {
        vec![asset, 0xdeadbeef, bits as u32 as u64, bits >> 32, whence]
    } else {
        vec![asset, bits, whence]
    };
    wide(emu, module, "AAsset_seek64", &args, arm32)
}

#[test]
fn native_asset_seeks_obey_signed_offsets_whence_and_both_abis() -> Result<()> {
    for dir in ["lib", "lib64"] {
        let arm32 = dir == "lib";
        let (_file, mut emu, module) = emulator(dir)?;
        let buffer = emu.malloc(8)?;
        for (name, length, middle) in [("stored.bin", 6, b"bc"), ("deflated.bin", 10, b"12")] {
            let asset = open(&mut emu, module, name, 0)?;
            for is_wide in [false, true] {
                for (offset, whence, expected, position) in [
                    (2, 0, 2, 2),
                    (1, 1, 3, 3),
                    (-1, 2, length - 1, length - 1),
                    (20, 0, -1, length - 1),
                    (2, 1, -1, length - 1),
                    (1, 2, -1, length - 1),
                    (-21, 1, -1, length - 1),
                    (-1, 0, -1, length - 1),
                    (0, 7, -1, length - 1),
                    (0, 1, length - 1, length - 1),
                    (0, 2, length, length),
                    (1, 1, -1, length),
                    (0, 0, 0, 0),
                    (length, 0, length, length),
                ] {
                    let result = if is_wide {
                        seek64(&mut emu, module, asset, offset, whence, arm32)?
                    } else {
                        let bits = call(
                            &mut emu,
                            module,
                            "AAsset_seek",
                            &[asset, offset as u64, whence],
                        )?;
                        if arm32 {
                            bits as u32 as i32 as i64
                        } else {
                            bits as i64
                        }
                    };
                    assert_eq!(
                        result, expected,
                        "{dir}, {name}, wide={is_wide}, offset={offset}, whence={whence}"
                    );
                    assert_eq!(seek64(&mut emu, module, asset, 0, 1, arm32)?, position);
                    assert_eq!(
                        call(&mut emu, module, "AAsset_getRemainingLength", &[asset])?,
                        (length - position) as u64
                    );
                    assert_eq!(
                        wide(
                            &mut emu,
                            module,
                            "AAsset_getRemainingLength64",
                            &[asset],
                            arm32
                        )?,
                        length - position
                    );
                    if position == length {
                        assert_eq!(
                            call(&mut emu, module, "AAsset_read", &[asset, buffer, 8])?,
                            0
                        );
                    }
                }
                if is_wide || !arm32 {
                    let result = if is_wide {
                        seek64(&mut emu, module, asset, i64::MAX, 1, arm32)?
                    } else {
                        call(
                            &mut emu,
                            module,
                            "AAsset_seek",
                            &[asset, i64::MAX as u64, 1],
                        )? as i64
                    };
                    assert_eq!(result, -1); // length + i64::MAX overflows.
                    assert_eq!(seek64(&mut emu, module, asset, 0, 1, arm32)?, length);
                }
            }
            assert_eq!(seek64(&mut emu, module, asset, 1, 0, arm32)?, 1);
            // r1 is padding, offset occupies r2:r3, and whence comes from [sp].
            // Both offsets have valid positive low words but invalid full values.
            for offset in [0x1_0000_0002, -0xffff_ffff] {
                assert_eq!(seek64(&mut emu, module, asset, offset, 0, arm32)?, -1);
                assert_eq!(seek64(&mut emu, module, asset, 0, 1, arm32)?, 1);
            }
            assert_eq!(
                call(&mut emu, module, "AAsset_read", &[asset, buffer, 2])?,
                2
            );
            assert_eq!(emu.read_bytes(buffer, 2)?, middle);
            assert_eq!(seek64(&mut emu, module, asset, -3, 1, arm32)?, 0);
            assert_eq!(
                wide(&mut emu, module, "AAsset_getLength64", &[asset], arm32)?,
                length
            );
            call(&mut emu, module, "AAsset_close", &[asset])?;
        }
        let empty = open(&mut emu, module, "empty.bin", 0)?;
        for offset in [0, 1, -1] {
            let expected = if offset == 0 { 0 } else { -1 };
            let bits = call(&mut emu, module, "AAsset_seek", &[empty, offset as u64, 2])?;
            assert_eq!(
                if arm32 {
                    bits as u32 as i32 as i64
                } else {
                    bits as i64
                },
                expected
            );
            assert_eq!(seek64(&mut emu, module, empty, offset, 2, arm32)?, expected);
            assert_eq!(
                wide(
                    &mut emu,
                    module,
                    "AAsset_getRemainingLength64",
                    &[empty],
                    arm32
                )?,
                0
            );
            assert_eq!(
                call(&mut emu, module, "AAsset_getRemainingLength", &[empty])?,
                0
            );
        }
        call(&mut emu, module, "AAsset_close", &[empty])?;
    }
    Ok(())
}
