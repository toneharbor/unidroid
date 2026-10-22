use super::{vm::*, *};
use unidroid::{Trace, Watch, WatchAction};

#[test]
fn code_trace_order_survives_stops_trace_changes_and_last_tracepoint_removal() -> Result<()> {
    for (arm64, thumb) in MODES {
        for trace_first in [false, true] {
            let mut vm = Vm::new(arm64, thumb)?;
            let trace = Trace {
                code: vec!["libc.so+0x1000-0x1080".into()],
                coverage: vec!["libc.so+0x1000-0x1080".into()],
                ..Trace::default()
            };
            if trace_first {
                vm.emu.set_trace(trace.clone())?;
            }
            vm.tracepoint(true)?;
            if !trace_first {
                vm.emu.set_trace(trace)?;
            }
            vm.emu.add_breakpoint(vm.dispatch)?;
            assert_eq!(vm.run(&mut ())?, 42);
            assert!(!vm.emu.coverage().is_empty());
            let mut instructions = Vec::new();
            let captured = vm.events.borrow();
            let mut hits = 0;
            for (i, (event, _)) in captured.iter().enumerate() {
                if event["event"] == "tracepoint" {
                    hits += 1;
                    assert_eq!(captured[i + 1].0["event"], "instruction");
                    assert_eq!(captured[i + 1].0["address"], event["address"]);
                    assert_eq!(captured[i - 1].0["event"], "instruction");
                } else {
                    assert_eq!(event["event"], "instruction", "{event}");
                    instructions.push(event.clone());
                }
            }
            assert_eq!(hits, 7);
            assert_eq!(
                instructions
                    .iter()
                    .filter(|e| e["address"] == format!("0x{:x}", vm.dispatch))
                    .count(),
                7
            );
            drop(captured);
            vm.emu.remove_tracepoint(vm.dispatch | 1)?;
            vm.events.borrow_mut().clear();
            assert_eq!(vm.run(&mut ())?, 42);
            assert_eq!(
                vm.events
                    .borrow()
                    .iter()
                    .map(|e| e.0.clone())
                    .collect::<Vec<_>>(),
                instructions
            );
        }
    }
    Ok(())
}

#[test]
fn thumb_it_captures_only_executed_instructions_before_their_values_change() -> Result<()> {
    for arm64 in [true, false] {
        for wide in [false, true] {
            let words = if arm64 {
                vec![
                    0xf100001f, 0x9a800020, 0x9a800020, 0x9a800020, 0x9a800020, 0x91000400,
                    0xd65f03c0,
                ]
            } else if wide {
                vec![
                    0xbf012800, 0x002af04f, 0x002af04f, 0x002af04f, 0x002af04f, 0x47703001,
                ]
            } else {
                vec![0xbf012800, 0x202a202a, 0x202a202a, 0x47703001]
            };
            let (mut emu, events) = emulator(arm64, &words)?;
            let reg = if arm64 { "x0" } else { "r0" };
            let end = CODE + if arm64 || wide { 20 } else { 12 };
            for i in 0..4 {
                let at = CODE + 4 + i * if arm64 || wide { 4 } else { 2 };
                emu.add_tracepoint(
                    at | 1,
                    &Tracepoint {
                        values: vec![reg.into()],
                        log: true,
                    },
                )?;
            }
            // Only the first conditional instruction sees zero. Its stop on Thumb
            // is deferred until all four MOVs have run; the trace still sees zero.
            emu.add_conditional_breakpoint(CODE + 4, &format!("{reg}==0x0"))?;
            for input in [0, 1] {
                events.borrow_mut().clear();
                let mut stops = Vec::new();
                let mut handler = Handler(|_: &mut Emulator, stop: &Stop| {
                    stops.push(*stop);
                    Resume::Continue
                });
                assert_eq!(
                    emu.call_function_with(&mut handler, CODE | u64::from(!arm64), &[input, 42])?,
                    if input == 0 { 43 } else { 2 }
                );
                assert_eq!(stops.len(), usize::from(input == 0));
                if input == 0 {
                    assert_eq!(stops[0].address, if arm64 { CODE + 4 } else { end });
                }
                let expected: &[u64] = if input == 0 {
                    &[0, 42, 42, 42]
                } else if arm64 {
                    &[1, 1, 1, 1]
                } else {
                    &[]
                };
                assert_eq!(
                    events
                        .borrow()
                        .iter()
                        .map(|e| e.0["values"][0][1].clone())
                        .collect::<Vec<_>>(),
                    expected
                        .iter()
                        .map(|v| json!(format!("0x{v:x}")))
                        .collect::<Vec<_>>()
                );
            }
        }
    }
    Ok(())
}

#[test]
fn tracepoints_follow_watch_logs_and_step_boundaries_in_execution_order() -> Result<()> {
    for arm64 in [true, false] {
        for watch_first in [false, true] {
            let words = if arm64 {
                [0xb9000001, 0xd28000e0, 0xd65f03c0]
            } else {
                [0xe5801000, 0xe3a00007, 0xe12fff1e]
            };
            let (mut emu, events) = emulator(arm64, &words)?;
            let target = emu.malloc(8)?;
            if watch_first {
                emu.add_watchpoint(target, 4, Watch::Write, WatchAction::Log)?;
            }
            emu.add_tracepoint(
                CODE + 4,
                &Tracepoint {
                    values: vec![format!("[0x{target:x}]:4")],
                    log: true,
                },
            )?;
            if !watch_first {
                emu.add_watchpoint(target, 4, Watch::Write, WatchAction::Log)?;
            }
            emu.add_breakpoint(CODE)?;
            emu.add_breakpoint(CODE + 4)?;
            let mut stops = Vec::new();
            let mut handler = Handler(|_: &mut Emulator, stop: &Stop| {
                stops.push((stop.reason, stop.address));
                Resume::Step
            });
            assert_eq!(
                emu.call_function_with(&mut handler, CODE, &[target, 42])?,
                7
            );
            assert_eq!(
                stops,
                [
                    (StopReason::Breakpoint, CODE),
                    (StopReason::Breakpoint, CODE + 4),
                    (StopReason::Step, CODE + 8),
                    (StopReason::Step, 0x20000),
                ]
            );
            assert_eq!(
                events
                    .borrow()
                    .iter()
                    .map(|e| e.0["event"].clone())
                    .collect::<Vec<_>>(),
                [json!("watch"), json!("tracepoint")]
            );
            assert_eq!(emu.tracepoint_hits(CODE + 4)?, [(42, 1)]);
        }
    }
    Ok(())
}
