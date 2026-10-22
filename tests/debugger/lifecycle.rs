use super::*;

#[test]
fn adding_to_warm_code_removing_and_duplicate_breakpoints_work() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            [0x91000400, 0xd65f03c0]
        } else {
            [0xe2800001, 0xe12fff1e]
        };
        let mut emu = emulator(arm64, &words)?;
        assert_eq!(emu.call_function(CODE, &[7])?, 8);
        emu.add_breakpoint(CODE)?;
        emu.add_breakpoint(CODE | 1)?;
        let mut hits = 0;
        let mut handler = Handler(|_: &mut Emulator, _: &Stop| {
            hits += 1;
            Resume::Continue
        });
        assert_eq!(emu.call_function_with(&mut handler, CODE, &[7])?, 8);
        emu.remove_breakpoint(CODE | 1)?;
        assert_eq!(emu.call_function_with(&mut handler, CODE, &[7])?, 8);
        assert_eq!(hits, 1);
        assert!(
            emu.remove_breakpoint(CODE)
                .unwrap_err()
                .to_string()
                .contains("no breakpoint")
        );
        // Both built-in handlers keep the default Continue.
        emu.add_breakpoint(CODE)?;
        assert_eq!(emu.call_function(CODE, &[7])?, 8);
        assert_eq!(
            emu.call_function_with(&mut unidroid::Probe::default(), CODE, &[7])?,
            8
        );
    }
    Ok(())
}

#[test]
fn a_nested_call_hits_the_same_breakpoint_and_restores_outer_registers() -> Result<()> {
    struct Nested {
        inner: bool,
        hits: Vec<u64>,
    }
    impl JniHandler for Nested {
        fn stopped(&mut self, emu: &mut Emulator, stop: &Stop) -> Resume {
            assert_eq!(stop.address, CODE);
            self.hits.push(emu.registers().unwrap()[0].1);
            if !self.inner {
                self.inner = true;
                assert_eq!(emu.call_function_with(self, CODE, &[41]).unwrap(), 42);
                assert_eq!(emu.registers().unwrap()[0].1, 7);
                self.inner = false;
            }
            Resume::Continue
        }
    }
    for arm64 in [true, false] {
        let words = if arm64 {
            [0x91000400, 0xd65f03c0]
        } else {
            [0xe2800001, 0xe12fff1e]
        };
        let mut emu = emulator(arm64, &words)?;
        emu.add_breakpoint(CODE)?;
        let mut handler = Nested {
            inner: false,
            hits: Vec::new(),
        };
        assert_eq!(emu.call_function_with(&mut handler, CODE, &[7])?, 8);
        assert_eq!(handler.hits, [7, 41]);
    }
    Ok(())
}

#[test]
fn a_guest_thread_reports_its_own_id_and_can_finish() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            [0x91000400, 0xd65f03c0]
        } else {
            [0xe2800001, 0xe12fff1e]
        };
        let mut emu = emulator(arm64, &words)?;
        let create = emu.resolve("pthread_create")?;
        let join = emu.resolve("pthread_join")?;
        let out = emu.malloc(16)?;
        emu.add_breakpoint(CODE)?;
        let mut stops = Vec::new();
        let mut handler = Handler(|emu: &mut Emulator, stop: &Stop| {
            stops.push(*stop);
            assert_ne!(stop.thread, 1234);
            if stop.reason == StopReason::Breakpoint {
                assert_eq!(stop.address, CODE);
                Resume::Finish
            } else {
                assert_eq!(stop.reason, StopReason::Finish);
                assert_eq!(emu.registers().unwrap()[0].1, 42);
                Resume::Continue
            }
        });
        assert_eq!(
            emu.call_function_with(&mut handler, create, &[out, 0, CODE, 41])?,
            0
        );
        let thread = u64::from_le_bytes(emu.read_bytes(out, 8)?.try_into().unwrap())
            & if arm64 { u64::MAX } else { 0xffffffff };
        assert_eq!(
            emu.call_function_with(&mut handler, join, &[thread, out + 8])?,
            0
        );
        assert_eq!(
            u64::from_le_bytes(emu.read_bytes(out + 8, 8)?.try_into().unwrap()),
            42
        );
        assert_eq!(stops.len(), 2);
        assert_eq!(stops[0].thread, stops[1].thread);
    }
    Ok(())
}

#[test]
fn thumb_breakpoints_step_by_instruction_width_and_return_in_thumb_mode() -> Result<()> {
    // The arm64 case also checks switching from an indirect call back to its caller.
    for arm64 in [true, false] {
        let words = if arm64 {
            [
                0xaa1e03f3, 0xd63f0020, 0xd65f0260, 0xd503201f, 0x91000400, 0xd65f03c0,
            ]
        } else {
            // Thumb: mov r4,lr; blx r1; bx r4. ARM callee: add r0,#1; bx lr.
            [0x47884674, 0x00004720, 0, 0, 0xe2800001, 0xe12fff1e]
        };
        let mut emu = emulator(arm64, &words)?;
        let entry = CODE | u64::from(!arm64);
        emu.add_breakpoint(entry)?;
        let mut stops = Vec::new();
        let mut handler = Handler(|_: &mut Emulator, stop: &Stop| {
            stops.push(stop.address);
            if stops.len() < 5 {
                Resume::Step
            } else {
                Resume::Continue
            }
        });
        assert_eq!(
            emu.call_function_with(&mut handler, entry, &[7, CODE + 16])?,
            8
        );
        assert_eq!(
            stops,
            [
                CODE,
                CODE + if arm64 { 4 } else { 2 },
                CODE + 16,
                CODE + 20,
                CODE + if arm64 { 8 } else { 4 }
            ]
        );
        emu.remove_breakpoint(entry)?;
        emu.add_breakpoint(CODE + 16)?;
        let mut handler = Handler(|_: &mut Emulator, _: &Stop| Resume::Return(42));
        assert_eq!(
            emu.call_function_with(&mut handler, entry, &[7, CODE + 16])?,
            42
        );
    }
    Ok(())
}
