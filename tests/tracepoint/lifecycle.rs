use super::{vm::*, *};
use unidroid::Trace;

#[test]
fn tracepoints_and_conditions_work_in_guest_threads() -> Result<()> {
    for (arm64, thumb) in MODES {
        let words = if arm64 {
            vec![0x91000400, 0xd65f03c0]
        } else if thumb {
            vec![0x47703001]
        } else {
            vec![0xe2800001, 0xe12fff1e]
        };
        let (mut emu, events) = emulator(arm64, &words)?;
        let reg = if arm64 { "x0" } else { "r0" };
        let entry = CODE | u64::from(thumb);
        emu.add_tracepoint(
            entry,
            &Tracepoint {
                values: vec![reg.into()],
                log: true,
            },
        )?;
        emu.add_conditional_breakpoint(entry, &format!("{reg}==0x7"))?;
        let create = emu.resolve("pthread_create")?;
        let join = emu.resolve("pthread_join")?;
        let out = emu.malloc(16)?;
        let mut stops = Vec::new();
        let mut handler = Handler(|_: &mut Emulator, stop: &Stop| {
            stops.push(*stop);
            Resume::Continue
        });
        assert_eq!(
            emu.call_function_with(&mut handler, create, &[out, 0, entry, 7])?,
            0
        );
        let thread = u64::from_le_bytes(emu.read_bytes(out, 8)?.try_into().unwrap())
            & if arm64 { u64::MAX } else { 0xffffffff };
        assert_eq!(
            emu.call_function_with(&mut handler, join, &[thread, out + 8])?,
            0
        );
        assert_eq!(emu.read_bytes(out + 8, 8)?, 8u64.to_le_bytes());
        assert_eq!(
            stops,
            [Stop {
                reason: StopReason::Breakpoint,
                address: CODE,
                thread: 1235,
                watch: None
            }]
        );
        assert_eq!(events.borrow().len(), 1);
        assert_eq!(events.borrow()[0].0["thread"], 1235);
        assert_eq!(events.borrow()[0].0["values"], json!([[reg, "0x7"]]));
        assert_eq!(emu.tracepoint_hits(entry)?, [(7, 1)]);
    }
    Ok(())
}

#[test]
fn nested_calls_keep_their_trace_order_and_restore_the_outer_values() -> Result<()> {
    struct Nested {
        inner: bool,
        entry: u64,
        stops: usize,
    }
    impl JniHandler for Nested {
        fn stopped(&mut self, emu: &mut Emulator, _: &Stop) -> Resume {
            self.stops += 1;
            if !self.inner {
                let before = emu.registers().unwrap();
                self.inner = true;
                let entry = self.entry;
                assert_eq!(emu.call_function_with(self, entry, &[9]).unwrap(), 10);
                assert_eq!(emu.registers().unwrap(), before);
                self.inner = false;
            }
            Resume::Continue
        }
    }
    for (arm64, thumb) in MODES {
        let words = if arm64 {
            vec![0x91000400, 0xd65f03c0]
        } else if thumb {
            vec![0x47703001]
        } else {
            vec![0xe2800001, 0xe12fff1e]
        };
        let (mut emu, events) = emulator(arm64, &words)?;
        let reg = if arm64 { "x0" } else { "r0" };
        let next = CODE + if thumb { 2 } else { 4 };
        for at in [CODE, next] {
            emu.add_tracepoint(
                at,
                &Tracepoint {
                    values: vec![reg.into()],
                    log: true,
                },
            )?;
        }
        emu.add_conditional_breakpoint(next, "0x0!=0x1")?;
        let mut handler = Nested {
            inner: false,
            entry: CODE | u64::from(thumb),
            stops: 0,
        };
        let entry = handler.entry;
        assert_eq!(emu.call_function_with(&mut handler, entry, &[7])?, 8);
        assert_eq!(handler.stops, 2);
        assert_eq!(
            events
                .borrow()
                .iter()
                .map(|e| e.0["values"][0][1].clone())
                .collect::<Vec<_>>(),
            [json!("0x7"), json!("0x9"), json!("0xa"), json!("0x8")]
        );
        assert_eq!(emu.tracepoint_hits(CODE)?, [(7, 1), (9, 1)]);
        assert_eq!(emu.tracepoint_hits(next)?, [(8, 1), (10, 1)]);
    }
    Ok(())
}

#[test]
fn rewind_preserves_current_tracepoints_conditions_and_counts_not_checkpoint_debug_state()
-> Result<()> {
    for (arm64, thumb) in MODES {
        let mut vm = Vm::new(arm64, thumb)?;
        let checkpoint = vm.emu.checkpoint()?;
        vm.tracepoint(false)?;
        vm.emu.add_conditional_breakpoint(
            vm.dispatch,
            &format!("{}==0x{:x}", vm.reg, vm.program + 3),
        )?;
        for times in [1, 2] {
            let mut stops = 0;
            assert_eq!(
                vm.run(&mut Handler(|_: &mut Emulator, _: &Stop| {
                    stops += 1;
                    Resume::Continue
                }))?,
                42
            );
            assert_eq!(stops, 2);
            vm.emu.rewind(&checkpoint)?;
            assert_eq!(vm.emu.tracepoint_hits(vm.dispatch)?, vm.counts(times));
        }
        vm.emu.remove_tracepoint(vm.dispatch)?;
        vm.emu.remove_breakpoint(vm.dispatch)?;
        vm.emu.rewind(&checkpoint)?;
        assert_eq!(
            vm.run(&mut Handler(|_: &mut Emulator, _: &Stop| panic!(
                "rewind restored removed breakpoint"
            )))?,
            42
        );
        assert!(vm.emu.tracepoint_hits(vm.dispatch).is_err());
    }
    Ok(())
}

#[test]
fn function_call_events_precede_entry_tracepoints_and_instruction_events() -> Result<()> {
    for arm64 in [true, false] {
        let mut vm = Vm::new(arm64, false)?;
        let wrapper = vm.entry + 0x80;
        // Save lr in a register the toy interpreter leaves alone, call it, return.
        let words: [u32; 3] = if arm64 {
            [0xaa1e03f3, 0x97ffffdf, 0xd65f0260]
        } else {
            [0xe1a0500e, 0xebffffdd, 0xe12fff15]
        };
        vm.emu.write_bytes(
            wrapper,
            &words
                .iter()
                .flat_map(|w| w.to_le_bytes())
                .collect::<Vec<_>>(),
        )?;
        vm.emu.add_tracepoint(
            vm.entry,
            &Tracepoint {
                values: vec!["0x1234".into()],
                log: true,
            },
        )?;
        vm.emu.set_trace(Trace {
            calls: vec!["libc.so+0x1000-0x1090".into()],
            code: vec!["libc.so+0x1000-0x1090".into()],
            ..Trace::default()
        })?;
        assert_eq!(vm.emu.call_function(wrapper, &[vm.program, vm.table])?, 42);
        let events = vm.events.borrow();
        let calls: Vec<_> = events
            .iter()
            .enumerate()
            .filter(|(_, e)| e.0["event"] == "function_call")
            .collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1.0["address"], format!("0x{:x}", vm.entry));
        let point = events
            .iter()
            .position(|e| e.0["event"] == "tracepoint")
            .unwrap();
        assert!(calls[0].0 < point);
        assert_eq!(events[point + 1].0["event"], "instruction");
        assert_eq!(events[point + 1].0["address"], format!("0x{:x}", vm.entry));
        assert_eq!(vm.emu.tracepoint_hits(vm.entry)?, [(0x1234, 1)]);
    }
    Ok(())
}
