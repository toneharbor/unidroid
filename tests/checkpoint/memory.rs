use super::{code, emulator, syscall};
use unidroid::{Emulator, Result, Trace};

#[test]
fn rewind_restores_guest_mapping_permissions_and_shared_code() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        let address = syscall(
            &mut emu,
            arm64,
            (222, 192),
            &[0, 0x3000, 7, 0x22, u64::MAX, 0],
        )?;
        let answer = if arm64 {
            [0x52800540, 0xd65f03c0]
        } else {
            [0xe3a0002a, 0xe12fff1e]
        };
        code(&mut emu, address + 0x2000, &answer)?;
        assert_eq!(emu.call_function(address + 0x2000, &[])?, 42);
        syscall(&mut emu, arm64, (226, 125), &[address + 0x1000, 0x1000, 0])?;
        let shared = emu
            .maps()?
            .into_iter()
            .find(|m| m.name == "libc.so" && m.perms == "r-x")
            .unwrap();
        // Keep split shared regions in the checkpoint to exercise their file offsets.
        syscall(
            &mut emu,
            arm64,
            (226, 125),
            &[shared.start + 0x1000, 0x1000, 1],
        )?;
        let checkpoint = emu.checkpoint()?;
        let before = emu.snapshot()?;
        let maps = emu.maps()?;
        for _ in 0..2 {
            syscall(&mut emu, arm64, (215, 91), &[address, 0x1000])?;
            syscall(&mut emu, arm64, (226, 125), &[address + 0x1000, 0x1000, 3])?;
            emu.write_bytes(address + 0x1000, &[5; 32])?;
            let changed = if arm64 {
                [0x528000e0, 0xd65f03c0]
            } else {
                [0xe3a00007, 0xe12fff1e]
            };
            code(&mut emu, address + 0x2000, &changed)?;
            assert_eq!(emu.call_function(address + 0x2000, &[])?, 7);
            let extra = syscall(
                &mut emu,
                arm64,
                (222, 192),
                &[0, 0x5000, 3, 0x22, u64::MAX, 0],
            )?;
            syscall(
                &mut emu,
                arm64,
                (226, 125),
                &[shared.start + 0x2000, 0x1000, 7],
            )?;
            emu.write_bytes(shared.start + 0x2000, &[0xa5; 32])?;
            // Remove part of a shared range too, so rewind must map its image again.
            syscall(&mut emu, arm64, (215, 91), &[shared.start + 0x4000, 0x1000])?;
            emu.rewind(&checkpoint)?;
            assert_eq!(emu.maps()?, maps);
            assert!(emu.read_bytes(extra, 1).is_err());
            let diff = before.diff(&emu.snapshot()?);
            assert!(
                diff.registers.is_empty()
                    && diff.changes.is_empty()
                    && diff.mapped.is_empty()
                    && diff.unmapped.is_empty(),
                "{}",
                diff.report()
            );
            assert_eq!(emu.call_function(address + 0x2000, &[])?, 42);
            assert!(emu.write_bytes(shared.start + 0x2000, &[1]).is_err());
        }
        let mut copy = Emulator::builder().build()?;
        copy.rewind(&checkpoint)?;
        assert_eq!(copy.maps()?, maps);
        let diff = before.diff(&copy.snapshot()?);
        assert!(diff.changes.is_empty(), "{}", diff.report());
        drop(emu);
        drop(checkpoint);
        assert_eq!(copy.call_function(address + 0x2000, &[])?, 42);
        assert_eq!(copy.read_bytes(shared.start, 4)?, b"\x7fELF");
    }
    Ok(())
}

#[test]
fn rewind_removes_later_modules_and_refreshes_trace_selections() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        let checkpoint = emu.checkpoint()?;
        let before = serde_json::to_value(emu.modules())?;
        emu.set_trace(Trace {
            imports: true,
            code: vec!["libm.so".into()],
            calls: vec!["libm.so".into()],
            coverage: vec!["libm.so".into()],
            ..Trace::default()
        })?;
        let path = format!("rootfs/{}/libm.so", if arm64 { "lib64" } else { "lib" });
        let module = emu.load_library(&path)?;
        let base = emu.modules()[module.0].base;
        let size = emu.modules()[module.0].size;
        emu.rewind(&checkpoint)?;
        assert_eq!(serde_json::to_value(emu.modules())?, before);
        assert!(
            emu.maps()?
                .iter()
                .all(|m| m.end <= base || m.start >= base + size)
        );
        assert_eq!(emu.trace().coverage, ["libm.so"]);
        // Reuse the removed module's address: stale coverage hooks must not count it.
        syscall(
            &mut emu,
            arm64,
            (222, 192),
            &[base, 0x1000, 7, 0x32, u64::MAX, 0],
        )?;
        code(
            &mut emu,
            base,
            if arm64 { &[0xd65f03c0] } else { &[0xe12fff1e] },
        )?;
        emu.reset_coverage();
        emu.call_function(base, &[3])?;
        assert!(emu.coverage().is_empty());
        emu.rewind(&checkpoint)?;
        let again = emu.load_library(&path)?;
        assert_eq!(again, module);
        assert_eq!(emu.modules()[again.0].base, base);
    }
    Ok(())
}
