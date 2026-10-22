use super::*;

#[test]
fn a_thumb_it_branch_reports_the_watch_hit_at_the_next_stop() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            vec![0xb9000001, 0xd61f0040] // str w1,[x0]; br x2
        } else {
            // cmp r3,#0; itt eq; streq r1,[r0]; bxeq r2
            vec![0xbf042b00, 0x47106001]
        };
        let mut emu = emulator(arm64, &words)?;
        let target = emu.malloc(4)?;
        emu.add_watchpoint(target, 4, Watch::Write)?;
        for destination in [0x20000, 0x30000000] {
            let mut stops = Vec::new();
            let mut handler = Handler(|emu: &mut Emulator, stop: &Stop| {
                stops.push(*stop);
                assert_eq!(stop.reason, StopReason::Watchpoint);
                assert_eq!(stop.watch.unwrap().pc, CODE + if arm64 { 0 } else { 4 });
                assert_eq!(stop.address, if arm64 { CODE + 4 } else { destination });
                assert_eq!(emu.read_bytes(target, 4).unwrap(), 42u32.to_le_bytes());
                Resume::Continue
            });
            let result = emu.call_function_with(
                &mut handler,
                CODE | u64::from(!arm64),
                &[target, 42, destination | u64::from(!arm64), 0],
            );
            if destination == 0x20000 {
                assert_eq!(result?, target);
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("UC_ERR_FETCH_UNMAPPED")
                );
            }
            assert_eq!(stops.len(), 1);
        }
    }
    Ok(())
}

#[test]
fn nested_watch_stops_keep_their_own_hits_and_restore_outer_registers() -> Result<()> {
    struct Nested {
        target: u64,
        inner: bool,
        values: Vec<u64>,
    }
    impl JniHandler for Nested {
        fn stopped(&mut self, emu: &mut Emulator, stop: &Stop) -> Resume {
            assert_eq!(stop.reason, StopReason::Watchpoint);
            assert_eq!(stop.address, CODE + 4);
            self.values.push(stop.watch.unwrap().value);
            if !self.inner {
                self.inner = true;
                let target = self.target;
                assert_eq!(
                    emu.call_function_with(self, CODE, &[target, 43]).unwrap(),
                    target
                );
                assert_eq!(emu.registers().unwrap()[1].1, 42);
                assert_eq!(stop.watch.unwrap().value, 42);
                self.inner = false;
            }
            Resume::Continue
        }
    }
    for arm64 in [true, false] {
        let words = if arm64 {
            [0xb9000001, 0xd65f03c0]
        } else {
            [0xe5801000, 0xe12fff1e]
        };
        let mut emu = emulator(arm64, &words)?;
        let target = emu.malloc(4)?;
        emu.add_watchpoint(target, 4, Watch::Write)?;
        let mut handler = Nested {
            target,
            inner: false,
            values: Vec::new(),
        };
        assert_eq!(
            emu.call_function_with(&mut handler, CODE, &[target, 42])?,
            target
        );
        assert_eq!(handler.values, [42, 43]);
    }
    Ok(())
}

#[test]
fn watchpoints_in_guest_threads_can_finish_in_the_same_thread() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            [0xd2800541, 0xb9000001, 0xd28000e0, 0xd65f03c0]
        } else {
            [0xe3a0102a, 0xe5801000, 0xe3a00007, 0xe12fff1e]
        };
        let mut emu = emulator(arm64, &words)?;
        let create = emu.resolve("pthread_create")?;
        let join = emu.resolve("pthread_join")?;
        let out = emu.malloc(16)?;
        let target = emu.malloc(4)?;
        emu.add_watchpoint(target, 4, Watch::Write)?;
        let mut stops = Vec::new();
        let mut handler = Handler(|emu: &mut Emulator, stop: &Stop| {
            stops.push(*stop);
            assert_ne!(stop.thread, 1234);
            if stop.reason == StopReason::Watchpoint {
                assert_eq!(stop.address, CODE + 8);
                assert_eq!(stop.watch.unwrap().pc, CODE + 4);
                assert_eq!(emu.read_bytes(target, 4).unwrap(), 42u32.to_le_bytes());
                Resume::Finish
            } else {
                assert_eq!(stop.reason, StopReason::Finish);
                assert_eq!(stop.watch, None);
                assert_eq!(emu.registers().unwrap()[0].1, 7);
                Resume::Continue
            }
        });
        assert_eq!(
            emu.call_function_with(&mut handler, create, &[out, 0, CODE, target])?,
            0
        );
        let thread = u64::from_le_bytes(emu.read_bytes(out, 8)?.try_into().unwrap())
            & if arm64 { u64::MAX } else { 0xffffffff };
        assert_eq!(
            emu.call_function_with(&mut handler, join, &[thread, out + 8])?,
            0
        );
        assert_eq!(emu.read_bytes(out + 8, 8)?, 7u64.to_le_bytes());
        assert_eq!(stops.len(), 2);
        assert_eq!(stops[0].thread, stops[1].thread);
    }
    Ok(())
}

#[test]
fn step_finish_return_and_abort_work_from_a_watch_stop() -> Result<()> {
    for arm64 in [true, false] {
        for resume in [
            Resume::Step,
            Resume::Finish,
            Resume::Return(99),
            Resume::Abort,
        ] {
            let words = if arm64 {
                [0xb9000001, 0xd28000e0, 0xd65f03c0]
            } else {
                [0xe5801000, 0xe3a00007, 0xe12fff1e]
            };
            let mut emu = emulator(arm64, &words)?;
            let target = emu.malloc(4)?;
            emu.add_watchpoint(target, 4, Watch::Write)?;
            let mut stops = Vec::new();
            let mut handler = Handler(|_: &mut Emulator, stop: &Stop| {
                stops.push(*stop);
                if stops.len() == 1 {
                    assert_eq!(stop.reason, StopReason::Watchpoint);
                    assert_eq!(stop.address, CODE + 4);
                    resume
                } else {
                    assert_eq!(stop.watch, None);
                    Resume::Continue
                }
            });
            let result = emu.call_function_with(&mut handler, CODE, &[target, 42]);
            if resume == Resume::Abort {
                let error = result.unwrap_err();
                let crash = error.downcast_ref::<unidroid::Crash>().unwrap();
                assert_eq!(
                    crash.message,
                    format!("stopped by the debugger at {}", emu.symbolize(CODE + 4))
                );
                assert_eq!(stops.len(), 1);
            } else {
                assert_eq!(result?, if resume == Resume::Return(99) { 99 } else { 7 });
                match resume {
                    Resume::Step => assert_eq!(
                        (stops[1].reason, stops[1].address),
                        (StopReason::Step, CODE + 8)
                    ),
                    Resume::Finish => assert_eq!(
                        (stops[1].reason, stops[1].address),
                        (StopReason::Finish, 0x20000)
                    ),
                    _ => assert_eq!(stops.len(), 1),
                }
            }
            assert_eq!(emu.read_bytes(target, 4)?, 42u32.to_le_bytes());
        }
    }
    Ok(())
}

#[test]
fn host_function_writes_do_not_trigger_watchpoints() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64, &[])?;
        let property = emu.resolve("__system_property_get")?;
        let key = emu.new_cstring("ro.build.version.sdk")?;
        let out = emu.malloc(64)?;
        emu.add_watchpoint(out, 64, Watch::Access)?;
        let mut handler =
            Handler(|_: &mut Emulator, _: &Stop| panic!("host memory access stopped"));
        emu.write_bytes(out, &[0; 64])?;
        assert_eq!(
            emu.call_function_with(&mut handler, property, &[key, out])?,
            2
        );
        assert_eq!(emu.read_bytes(out, 3)?, b"23\0");
    }
    Ok(())
}
