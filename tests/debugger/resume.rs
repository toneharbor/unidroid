use super::*;

#[test]
fn finish_ignores_recursive_returns_with_a_deeper_stack() -> Result<()> {
    for arm64 in [true, false] {
        // Recurse at the same call site, then add one while unwinding.
        let words = if arm64 {
            vec![
                0xa9bf7bfd, 0xb4000060, 0xd1000400, 0x97fffffd, 0x91000400, 0xa8c17bfd, 0xd65f03c0,
            ]
        } else {
            vec![
                0xe92d4800, 0xe3500000, 0x0a000001, 0xe2400001, 0xebfffffa, 0xe2800001, 0xe8bd4800,
                0xe12fff1e,
            ]
        };
        let mut emu = emulator(arm64, &words)?;
        emu.add_breakpoint(CODE)?;
        let mut entries = 0;
        let mut finished = 0;
        let mut saved_sp = 0;
        let mut handler = Handler(|emu: &mut Emulator, stop: &Stop| {
            if stop.reason == StopReason::Breakpoint {
                entries += 1;
                if entries == 2 {
                    emu.remove_breakpoint(CODE).unwrap();
                    saved_sp = emu.register("sp").unwrap();
                    Resume::Finish
                } else {
                    Resume::Continue
                }
            } else {
                finished += 1;
                assert_eq!(stop.reason, StopReason::Finish);
                assert_eq!(stop.address, CODE + if arm64 { 16 } else { 20 });
                assert_eq!(emu.register("sp").unwrap(), saved_sp);
                assert_eq!(emu.registers().unwrap()[0].1, 3);
                Resume::Continue
            }
        });
        assert_eq!(emu.call_function_with(&mut handler, CODE, &[3])?, 4);
        assert_eq!((entries, finished), (2, 1));
    }
    Ok(())
}

#[test]
fn breakpoint_hits_again_on_a_loop_and_step_skips_only_the_current_hit() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            vec![0xf1000400, 0x54ffffe1, 0xd65f03c0] // subs; b.ne; ret
        } else {
            vec![0xe2500001, 0x1afffffd, 0xe12fff1e]
        };
        let mut emu = emulator(arm64, &words)?;
        emu.add_breakpoint(CODE)?;
        let mut stops = Vec::new();
        let mut handler = Handler(|emu: &mut Emulator, stop: &Stop| {
            stops.push((stop.reason, stop.address, emu.registers().unwrap()[0].1));
            if stops.len() <= 2 {
                Resume::Step
            } else {
                Resume::Continue
            }
        });
        assert_eq!(emu.call_function_with(&mut handler, CODE, &[3])?, 0);
        assert_eq!(
            stops,
            [
                (StopReason::Breakpoint, CODE, 3),
                (StopReason::Step, CODE + 4, 2),
                (StopReason::Step, CODE, 2),
                (StopReason::Breakpoint, CODE, 1),
            ]
        );
    }
    Ok(())
}

#[test]
fn changing_pc_while_stopped_does_not_skip_a_future_breakpoint() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            // branch to add; add; ret
            vec![0x14000001, 0x91000400, 0xd65f03c0]
        } else {
            vec![0xeaffffff, 0xe2800001, 0xe12fff1e]
        };
        let mut emu = emulator(arm64, &words)?;
        emu.add_breakpoint(CODE + 4)?;
        let mut hits = 0;
        let mut handler = Handler(|emu: &mut Emulator, _: &Stop| {
            hits += 1;
            if hits == 1 {
                emu.set_register("pc", CODE).unwrap();
            }
            Resume::Continue
        });
        assert_eq!(emu.call_function_with(&mut handler, CODE, &[7])?, 8);
        assert_eq!(hits, 2);
    }
    Ok(())
}
