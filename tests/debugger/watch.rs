use super::*;
use unidroid::{Watch, WatchHit};

#[path = "watch/lifecycle.rs"]
mod lifecycle;

#[test]
fn faulting_watched_stores_report_a_crash_without_a_completed_access_stop() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            [0xb9000001, 0xd65f03c0]
        } else {
            [0xe5801000, 0xe12fff1e]
        };
        let mut emu = emulator(arm64, &words)?;
        for (target, error) in [
            (0x30000000, "UC_ERR_WRITE_UNMAPPED"),
            (0x20800, "UC_ERR_WRITE_PROT"),
        ] {
            emu.add_watchpoint(target, 4, Watch::Write)?;
            let mut handler = Handler(|_: &mut Emulator, _: &Stop| {
                panic!("faulting instruction has not completed")
            });
            let error_message = emu
                .call_function_with(&mut handler, CODE, &[target, 42])
                .unwrap_err();
            let crash = error_message.downcast_ref::<unidroid::Crash>().unwrap();
            assert!(crash.message.contains(error), "{}", crash.message);
            assert!(crash.message.contains("at 0x20100"));
            emu.remove_watchpoint(target)?;
        }
    }
    Ok(())
}

#[test]
fn vector_accesses_use_the_full_eight_byte_overlap_extension() -> Result<()> {
    for arm64 in [true, false] {
        for load in [true, false] {
            // ldr/str q0,[x0] / vldr/vstr d0,[r0], followed by return.
            let words = if arm64 {
                [if load { 0x3dc00000 } else { 0x3d800000 }, 0xd65f03c0]
            } else {
                [if load { 0xed900b00 } else { 0xed800b00 }, 0xe12fff1e]
            };
            let mut emu = emulator(arm64, &words)?;
            let target = emu.malloc(16)?;
            let value = 0xfedcba9876543210u64;
            emu.set_register("d0", if load { 0 } else { value })?;
            emu.write_bytes(target, &value.to_le_bytes())?;
            emu.add_watchpoint(target + 7, 1, Watch::Access)?;
            let mut hits = 0;
            let mut handler = Handler(|emu: &mut Emulator, stop: &Stop| {
                hits += 1;
                assert_eq!(stop.address, CODE + 4);
                assert_eq!(
                    stop.watch,
                    Some(WatchHit {
                        pc: CODE,
                        address: target,
                        size: 8,
                        write: !load,
                        value
                    })
                );
                assert_eq!(emu.register("d0").unwrap(), value);
                assert_eq!(emu.read_bytes(target, 8).unwrap(), value.to_le_bytes());
                Resume::Continue
            });
            emu.call_function_with(&mut handler, CODE, &[target])?;
            assert_eq!(hits, 1);
        }
    }
    Ok(())
}

#[test]
fn watch_kinds_filter_loads_and_stores_and_capture_values_at_access() -> Result<()> {
    for arm64 in [true, false] {
        for load in [true, false] {
            let words = if arm64 {
                [
                    if load { 0xb9400001 } else { 0xb9000001 },
                    0xaa0103e0,
                    0xd65f03c0,
                ]
            } else {
                [
                    if load { 0xe5901000 } else { 0xe5801000 },
                    0xe1a00001,
                    0xe12fff1e,
                ]
            };
            let mut emu = emulator(arm64, &words)?;
            let target = emu.malloc(8)?;
            for watch in [Watch::Read, Watch::Write, Watch::Access] {
                emu.add_watchpoint(target, 4, watch)?;
                let value = 0xfedcba98u64;
                emu.write_bytes(target, &(value as u32).to_le_bytes())?;
                let mut stops = Vec::new();
                let mut handler = Handler(|emu: &mut Emulator, stop: &Stop| {
                    stops.push(*stop);
                    assert_eq!(stop.reason, StopReason::Watchpoint);
                    assert_eq!(stop.address, CODE + 4);
                    assert_eq!(emu.registers().unwrap()[1].1, value);
                    assert_eq!(
                        stop.watch,
                        Some(WatchHit {
                            pc: CODE,
                            address: target,
                            size: 4,
                            write: !load,
                            value
                        })
                    );
                    // The hit keeps the access-time value when host code changes memory.
                    emu.write_bytes(target, &[0; 4]).unwrap();
                    Resume::Continue
                });
                assert_eq!(
                    emu.call_function_with(
                        &mut handler,
                        CODE,
                        &[target, if load { 0 } else { value }]
                    )?,
                    value
                );
                assert_eq!(
                    stops.len(),
                    usize::from(
                        watch == Watch::Access
                            || (load && watch == Watch::Read)
                            || (!load && watch == Watch::Write)
                    )
                );
            }
        }
    }
    Ok(())
}

#[test]
fn overlapping_accesses_stop_but_neighbors_and_removed_ranges_do_not() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            [0xb9000001, 0xd65f03c0]
        } else {
            [0xe5801000, 0xe12fff1e]
        };
        let mut emu = emulator(arm64, &words)?;
        let allocation = emu.malloc(64)?;
        let start = allocation + 16;
        assert_eq!(
            emu.add_watchpoint(start, 0, Watch::Write)
                .unwrap_err()
                .to_string(),
            "empty watchpoint"
        );
        emu.add_watchpoint(start, 3, Watch::Write)?;
        for (at, hits) in [(start - 4, 0), (start - 1, 1), (start + 3, 0)] {
            let mut count = 0;
            let mut handler = Handler(|_: &mut Emulator, stop: &Stop| {
                count += 1;
                assert_eq!(stop.watch.unwrap().address, at);
                Resume::Continue
            });
            assert_eq!(emu.call_function_with(&mut handler, CODE, &[at, 42])?, at);
            assert_eq!(count, hits);
        }
        // Replacing the same start changes both kind and extent, and leaves one range.
        emu.add_watchpoint(start, 8, Watch::Read)?;
        let mut handler =
            Handler(|_: &mut Emulator, _: &Stop| panic!("read watch must ignore stores"));
        emu.call_function_with(&mut handler, CODE, &[start + 4, 42])?;
        emu.add_watchpoint(start, 8, Watch::Write)?;
        let mut hits = 0;
        let mut handler = Handler(|_: &mut Emulator, stop: &Stop| {
            hits += 1;
            assert_eq!(stop.watch.unwrap().address, start + 4);
            Resume::Continue
        });
        emu.call_function_with(&mut handler, CODE, &[start + 4, 42])?;
        emu.remove_watchpoint(start)?;
        emu.call_function_with(&mut handler, CODE, &[start + 4, 42])?;
        assert_eq!(hits, 1);
        assert_eq!(
            emu.remove_watchpoint(start).unwrap_err().to_string(),
            format!("no watchpoint at {}", emu.symbolize(start))
        );
    }
    Ok(())
}

#[test]
fn thumb_it_watch_keeps_the_store_pc_and_stops_after_the_block() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            // cmp x2,#0; b.ne return; str w1,[x0]; mov x2,#9; ret
            vec![0xf100005f, 0x54000061, 0xb9000001, 0xd2800122, 0xd65f03c0]
        } else {
            // cmp r2,#0; itt eq; streq r1,[r0]; moveq r2,#9; bx lr
            vec![0xbf042a00, 0x22096001, 0x00004770]
        };
        let mut emu = emulator(arm64, &words)?;
        let target = emu.malloc(4)?;
        emu.add_watchpoint(target, 4, Watch::Write)?;
        for condition in [0, 1] {
            let mut stops = Vec::new();
            let mut handler = Handler(|emu: &mut Emulator, stop: &Stop| {
                stops.push(*stop);
                assert_eq!(stop.reason, StopReason::Watchpoint);
                assert_eq!(stop.address, CODE + if arm64 { 12 } else { 8 });
                assert_eq!(stop.watch.unwrap().pc, CODE + if arm64 { 8 } else { 4 });
                assert_eq!(emu.registers().unwrap()[2].1, if arm64 { 0 } else { 9 });
                assert_eq!(emu.read_bytes(target, 4).unwrap(), 42u32.to_le_bytes());
                Resume::Continue
            });
            assert_eq!(
                emu.call_function_with(
                    &mut handler,
                    CODE | u64::from(!arm64),
                    &[target, 42, condition]
                )?,
                target
            );
            assert_eq!(stops.len(), usize::from(condition == 0));
        }
    }
    Ok(())
}

#[test]
fn paired_stores_report_only_the_first_access_after_both_writes_land() -> Result<()> {
    for arm64 in [true, false] {
        // stp x1,x2,[x0]; ret / stm r0,{r1,r2}; bx lr
        let words = if arm64 {
            [0xa9000801, 0xd65f03c0]
        } else {
            [0xe8800006, 0xe12fff1e]
        };
        let mut emu = emulator(arm64, &words)?;
        let target = emu.malloc(16)?;
        emu.add_watchpoint(target, 16, Watch::Access)?;
        emu.add_watchpoint(target + 1, 15, Watch::Write)?;
        let width = if arm64 { 8 } else { 4 };
        let mut hits = 0;
        let mut handler = Handler(|emu: &mut Emulator, stop: &Stop| {
            hits += 1;
            assert_eq!(stop.address, CODE + 4);
            assert_eq!(
                stop.watch,
                Some(WatchHit {
                    pc: CODE,
                    address: target,
                    size: width,
                    write: true,
                    value: 42
                })
            );
            assert_eq!(
                emu.read_bytes(target, width).unwrap(),
                42u64.to_le_bytes()[..width]
            );
            assert_eq!(
                emu.read_bytes(target + width as u64, width).unwrap(),
                43u64.to_le_bytes()[..width]
            );
            Resume::Continue
        });
        emu.call_function_with(&mut handler, CODE, &[target, 42, 43])?;
        assert_eq!(hits, 1);
    }
    Ok(())
}

#[test]
fn watched_store_finishes_before_the_stop_and_continue_keeps_the_next_breakpoint() -> Result<()> {
    for arm64 in [true, false] {
        // str w1,[x0]; mov x0,#7; ret / str r1,[r0]; mov r0,#7; bx lr
        let words = if arm64 {
            [0xb9000001, 0xd28000e0, 0xd65f03c0]
        } else {
            [0xe5801000, 0xe3a00007, 0xe12fff1e]
        };
        let mut emu = emulator(arm64, &words)?;
        let target = emu.malloc(8)?;
        assert_eq!(emu.call_function(CODE, &[target, 1])?, 7);
        // Install the breakpoint first so hook order cannot hide the watch hit.
        emu.add_breakpoint(CODE + 4)?;
        emu.add_watchpoint(target, 4, Watch::Write)?;
        emu.write_bytes(target, &[0; 8])?;
        let mut stops = Vec::new();
        let mut handler = Handler(|emu: &mut Emulator, stop: &Stop| {
            stops.push(*stop);
            assert_eq!(stop.address, CODE + 4);
            assert_eq!(emu.register("pc").unwrap(), CODE + 4);
            assert_eq!(emu.registers().unwrap()[0].1, target);
            assert_eq!(emu.read_bytes(target, 4).unwrap(), 42u32.to_le_bytes());
            assert!(stops.len() <= 2);
            Resume::Continue
        });
        assert_eq!(
            emu.call_function_with(&mut handler, CODE, &[target, 42])?,
            7
        );
        assert_eq!(
            stops,
            [
                Stop {
                    reason: StopReason::Watchpoint,
                    address: CODE + 4,
                    thread: 1234,
                    watch: Some(WatchHit {
                        pc: CODE,
                        address: target,
                        size: 4,
                        write: true,
                        value: 42
                    })
                },
                Stop {
                    reason: StopReason::Breakpoint,
                    address: CODE + 4,
                    thread: 1234,
                    watch: None
                },
            ]
        );
    }
    Ok(())
}
