use super::{vm::*, *};

#[test]
fn conditions_stop_on_each_matching_bytecode_and_unreadable_operands_are_false() -> Result<()> {
    for (arm64, thumb) in MODES {
        let mut vm = Vm::new(arm64, thumb)?;
        vm.tracepoint(true)?;
        for (condition, expected) in [
            (format!("{} == 0x{:x}", vm.reg, vm.program + 3), vec![3, 3]),
            (
                format!("{}!=0x{:x}", vm.reg, vm.program + 3),
                vec![0, 1, 0, 1, 6],
            ),
            (format!("[{}]:1==0x1", vm.reg), vec![1, 1]),
            ("[0x0]==0x0".into(), vec![]),
            ("0x0!=[0x0]".into(), vec![]),
            ("0x2a==0x2b".into(), vec![]),
        ] {
            vm.emu
                .add_conditional_breakpoint(vm.dispatch | 1, &condition)?;
            let mut stops = Vec::new();
            let reg = vm.reg;
            let program = vm.program;
            let mut handler = Handler(|emu: &mut Emulator, stop: &Stop| {
                assert_eq!(stop.reason, StopReason::Breakpoint);
                assert_eq!(stop.watch, None);
                stops.push(emu.register(reg).unwrap() - program);
                assert!(stops.len() <= 7);
                Resume::Continue
            });
            vm.events.borrow_mut().clear();
            vm.emu.reset_tracepoint_hits();
            assert_eq!(vm.run(&mut handler)?, 42);
            assert_eq!(stops, expected, "{condition}");
            assert_eq!(vm.events.borrow().len(), 7);
            assert_eq!(vm.emu.tracepoint_hits(vm.dispatch)?, vm.counts(1));
        }
    }
    Ok(())
}

#[test]
fn plain_and_conditional_breakpoints_replace_and_remove_each_other() -> Result<()> {
    for (arm64, thumb) in MODES {
        let mut vm = Vm::new(arm64, thumb)?;
        for plain in [true, false, true] {
            if plain {
                vm.emu.add_breakpoint(vm.dispatch)?;
            } else {
                vm.emu.add_conditional_breakpoint(vm.dispatch, "0x0==0x1")?;
            }
            let mut count = 0;
            let mut handler = Handler(|_: &mut Emulator, _: &Stop| {
                count += 1;
                Resume::Continue
            });
            assert_eq!(vm.run(&mut handler)?, 42);
            assert_eq!(count, if plain { 7 } else { 0 });
        }
        vm.emu.add_conditional_breakpoint(vm.dispatch, "0x0==0x0")?;
        vm.emu.remove_breakpoint(vm.dispatch | 1)?;
        assert_eq!(
            vm.run(&mut Handler(|_: &mut Emulator, _: &Stop| panic!(
                "removed breakpoint"
            )))?,
            42
        );
        assert_eq!(
            vm.emu
                .remove_breakpoint(vm.dispatch)
                .unwrap_err()
                .to_string(),
            format!("no breakpoint at {}", vm.emu.symbolize(vm.dispatch))
        );
    }
    Ok(())
}
