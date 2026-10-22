use super::*;

fn conditional_code(arm64: bool, wide: bool) -> Vec<u32> {
    if arm64 {
        // cmp x0,#0; four csel x0,x1,x0,eq; add x0,#1; ret
        vec![
            0xf100001f, 0x9a800020, 0x9a800020, 0x9a800020, 0x9a800020, 0x91000400, 0xd65f03c0,
        ]
    } else if wide {
        // cmp r0,#0; itttt eq; four mov.w r0,#42; adds r0,#1; bx lr
        vec![
            0xbf012800, 0x002af04f, 0x002af04f, 0x002af04f, 0x002af04f, 0x47703001,
        ]
    } else {
        // cmp r0,#0; itttt eq; four mov r0,#42; adds r0,#1; bx lr
        vec![0xbf012800, 0x202a202a, 0x202a202a, 0x47703001]
    }
}

#[test]
fn conditional_breakpoints_stop_after_thumb_it_blocks() -> Result<()> {
    for arm64 in [true, false] {
        for wide in [false, true] {
            let mut emu = emulator(arm64, &conditional_code(arm64, wide))?;
            let entry = CODE | u64::from(!arm64);
            for input in [0, 1] {
                // Warm the translation before installing each breakpoint.
                let result = if input == 0 { 43 } else { 2 };
                assert_eq!(emu.call_function(entry, &[input, 42])?, result);
                for i in 0..4 {
                    let address = CODE + 4 + i * if arm64 || wide { 4 } else { 2 };
                    emu.add_breakpoint(address)?;
                    let mut stops = Vec::new();
                    let mut handler = Handler(|emu: &mut Emulator, stop: &Stop| {
                        stops.push((stop.reason, stop.address, emu.registers().unwrap()[0].1));
                        assert_eq!(emu.register("pc").unwrap(), stop.address);
                        Resume::Continue
                    });
                    assert_eq!(
                        emu.call_function_with(&mut handler, entry, &[input, 42])?,
                        result
                    );
                    emu.remove_breakpoint(address)?;
                    assert_eq!(
                        emu.call_function_with(&mut handler, entry, &[input, 42])?,
                        result
                    );
                    let expected_address = if arm64 {
                        address
                    } else {
                        CODE + if wide { 20 } else { 12 }
                    };
                    let expected_value = if input == 0 && (!arm64 || i > 0) {
                        42
                    } else {
                        input
                    };
                    let expected = if !arm64 && input != 0 {
                        Vec::new()
                    } else {
                        vec![(StopReason::Breakpoint, expected_address, expected_value)]
                    };
                    assert_eq!(
                        stops, expected,
                        "arm64={arm64} wide={wide} input={input} index={i}"
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
fn deferred_breakpoint_preserves_the_host_call_at_its_next_stop() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            vec![
                0xf100005f, 0x54000061, 0xd2800022, 0xd61f0060, 0xd2800120, 0xd65f03c0,
            ]
        } else {
            // cmp r2,#0; itt eq; moveq r2,#1; bxeq r3; mov r0,#9; bx lr
            vec![0xbf042a00, 0x47182201, 0x47702009]
        };
        let mut emu = emulator(arm64, &words)?;
        let property = emu.resolve("__system_property_get")?;
        let key = emu.new_cstring("ro.build.version.sdk")?;
        let out = emu.malloc(64)?;
        let address = CODE + if arm64 { 8 } else { 4 };
        emu.add_breakpoint(address)?;
        for condition in [0, 1] {
            let mut stops = Vec::new();
            let mut handler = Handler(|emu: &mut Emulator, stop: &Stop| {
                stops.push(*stop);
                assert_eq!(emu.register("pc").unwrap(), stop.address);
                Resume::Continue
            });
            assert_eq!(
                emu.call_function_with(
                    &mut handler,
                    CODE | u64::from(!arm64),
                    &[key, out, condition, property]
                )?,
                if condition == 0 { 2 } else { 9 }
            );
            if condition == 0 {
                assert_eq!(emu.read_bytes(out, 3)?, b"23\0");
                assert_eq!(
                    stops,
                    [Stop {
                        reason: StopReason::Breakpoint,
                        address: if arm64 { address } else { property + 4 },
                        thread: 1234,
                        watch: None,
                    }]
                );
            } else {
                assert!(stops.is_empty());
            }
        }
    }
    Ok(())
}

#[test]
fn a_deferred_hit_is_reported_even_when_the_branch_target_faults() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            vec![0xd2800540, 0xd61f0020]
        } else {
            // cmp r0,#0; itt eq; moveq r0,#42; bxeq r1
            vec![0xbf042800, 0x4708202a]
        };
        let mut emu = emulator(arm64, &words)?;
        emu.add_breakpoint(CODE + if arm64 { 0 } else { 4 })?;
        let mut stops = Vec::new();
        let mut handler = Handler(|_: &mut Emulator, stop: &Stop| {
            stops.push(*stop);
            Resume::Continue
        });
        let error = emu
            .call_function_with(&mut handler, CODE | u64::from(!arm64), &[0, 0x30000001])
            .unwrap_err();
        assert!(error.to_string().contains("Invalid memory fetch"));
        assert_eq!(
            stops,
            [Stop {
                reason: StopReason::Breakpoint,
                address: if arm64 { CODE } else { 0x30000000 },
                thread: 1234,
                watch: None,
            }]
        );
    }
    Ok(())
}

#[test]
fn the_single_instruction_thumb_it_example_stops_before_its_return() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            vec![0xf100001f, 0x9a800020, 0xd65f03c0]
        } else {
            // cmp r0,#0; it eq; moveq r0,#42; bx lr
            vec![0xbf082800, 0x4770202a]
        };
        let mut emu = emulator(arm64, &words)?;
        for step in [false, true] {
            let address = CODE + if !arm64 && step { 2 } else { 4 };
            emu.add_breakpoint(address)?;
            for input in [0, 1] {
                let mut stops = Vec::new();
                let mut handler = Handler(|emu: &mut Emulator, stop: &Stop| {
                    stops.push((stop.reason, stop.address, emu.registers().unwrap()[0].1));
                    if step && stops.len() == 1 {
                        Resume::Step
                    } else {
                        Resume::Continue
                    }
                });
                let result = if input == 0 { 42 } else { 1 };
                assert_eq!(
                    emu.call_function_with(&mut handler, CODE | u64::from(!arm64), &[input, 42])?,
                    result
                );
                let expected = if step {
                    vec![
                        (StopReason::Breakpoint, address, input),
                        (StopReason::Step, CODE + if arm64 { 8 } else { 6 }, result),
                    ]
                } else if arm64 {
                    vec![(StopReason::Breakpoint, address, input)]
                } else if input == 0 {
                    vec![(StopReason::Breakpoint, CODE + 6, result)]
                } else {
                    Vec::new()
                };
                assert_eq!(stops, expected);
            }
            emu.remove_breakpoint(address)?;
        }
    }
    Ok(())
}

#[test]
fn a_finish_target_inside_thumb_it_keeps_its_reason_when_deferred() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            vec![0xaa0303fe, 0xf100001f, 0x9a800020, 0x91000400, 0xd65f03c0]
        } else {
            // mov lr,r3; cmp r0,#0; it eq; moveq r0,#42; adds r0,#1; bx lr
            vec![0x2800469e, 0x202abf08, 0x47703001]
        };
        let mut emu = emulator(arm64, &words)?;
        emu.add_breakpoint(CODE)?;
        let mut stops = Vec::new();
        let mut handler = Handler(|emu: &mut Emulator, stop: &Stop| {
            stops.push((stop.reason, stop.address, emu.registers().unwrap()[0].1));
            if stops.len() == 1 {
                emu.set_register("lr", CODE + if arm64 { 8 } else { 7 })
                    .unwrap();
                Resume::Finish
            } else {
                Resume::Continue
            }
        });
        assert_eq!(
            emu.call_function_with(&mut handler, CODE | u64::from(!arm64), &[0, 42, 0, 0x20000])?,
            43
        );
        assert_eq!(
            stops,
            [
                (StopReason::Breakpoint, CODE, 0),
                (StopReason::Finish, CODE + 8, if arm64 { 0 } else { 42 }),
            ]
        );
    }
    Ok(())
}

#[test]
fn step_treats_thumb_it_and_its_conditional_instructions_as_one_instruction() -> Result<()> {
    for arm64 in [true, false] {
        for wide in [false, true] {
            let mut emu = emulator(arm64, &conditional_code(arm64, wide))?;
            let entry = CODE | u64::from(!arm64);
            let address = CODE + if arm64 { 4 } else { 2 };
            emu.add_breakpoint(address)?;
            for input in [0, 1] {
                let mut stops = Vec::new();
                let mut handler = Handler(|emu: &mut Emulator, stop: &Stop| {
                    stops.push((stop.reason, stop.address, emu.registers().unwrap()[0].1));
                    if stops.len() == 1 {
                        Resume::Step
                    } else {
                        Resume::Continue
                    }
                });
                assert_eq!(
                    emu.call_function_with(&mut handler, entry, &[input, 42])?,
                    if input == 0 { 43 } else { 2 }
                );
                assert_eq!(
                    stops,
                    [
                        (StopReason::Breakpoint, address, input),
                        (
                            StopReason::Step,
                            CODE + if arm64 {
                                8
                            } else if wide {
                                20
                            } else {
                                12
                            },
                            if input == 0 { 42 } else { 1 }
                        ),
                    ]
                );
            }
        }
    }
    Ok(())
}
