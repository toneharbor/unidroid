use super::{code, emulator};
use unidroid::{Emulator, Result};

fn pointer(emu: &Emulator, address: u64, arm64: bool) -> Result<u64> {
    let mut bytes = [0; 8];
    let width = if arm64 { 8 } else { 4 };
    bytes[..width].copy_from_slice(&emu.read_bytes(address, width)?);
    Ok(u64::from_le_bytes(bytes))
}

#[test]
fn rewind_and_copy_resume_a_partly_executed_thread() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        // entered = 1; count down; finished = 1; return argument.
        let worker = code(
            &mut emu,
            0x20200,
            if arm64 {
                &[
                    0x52800021, 0xb9000001, 0x52986a02, 0x71000442, 0x54ffffe1, 0xb9000401,
                    0xd65f03c0,
                ]
            } else {
                &[
                    0xe3a01001, 0xe5801000, 0xe30c2350, 0xe2522001, 0x1afffffd, 0xe5801004,
                    0xe12fff1e,
                ]
            },
        )?;
        // Wait for a flag without syscalls: this needs the thread slice hook.
        let spin = code(
            &mut emu,
            0x20300,
            if arm64 {
                &[
                    0x52986a02, 0xb9400001, 0x35000061, 0x71000442, 0x54ffffa1, 0x2a0103e0,
                    0xd65f03c0,
                ]
            } else {
                &[
                    0xe30c2350, 0xe5901000, 0xe3510000, 0x1a000001, 0xe2522001, 0x1afffffa,
                    0xe1a00001, 0xe12fff1e,
                ]
            },
        )?;
        let flags = emu.malloc(8)?;
        let out = emu.malloc(8)?;
        let create = emu.resolve("pthread_create")?;
        assert_eq!(emu.call_function(create, &[out, 0, worker, flags])?, 0);
        let thread = pointer(&emu, out, arm64)?;
        assert_eq!(emu.call_function(spin, &[flags])?, 1);
        assert_eq!(emu.read_bytes(flags + 4, 4)?, [0; 4]);
        let checkpoint = emu.checkpoint()?;
        let finish = |emu: &mut Emulator| -> Result<()> {
            assert_eq!(emu.call_function(spin, &[flags + 4])?, 1);
            let join = emu.resolve("pthread_join")?;
            assert_eq!(emu.call_function(join, &[thread, out])?, 0);
            assert_eq!(pointer(emu, out, arm64)?, flags);
            Ok(())
        };
        finish(&mut emu)?;
        for _ in 0..3 {
            emu.rewind(&checkpoint)?;
            assert_eq!(emu.read_bytes(flags + 4, 4)?, [0; 4]);
            finish(&mut emu)?;
        }
        let mut copy = Emulator::builder().build()?;
        copy.rewind(&checkpoint)?;
        finish(&mut copy)?;
        // The captured CPU contexts outlive the original engine as well.
        drop(emu);
        copy.rewind(&checkpoint)?;
        finish(&mut copy)?;
    }
    Ok(())
}
