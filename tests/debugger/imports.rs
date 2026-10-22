use super::*;
use std::{cell::RefCell, rc::Rc};
use unidroid::{Event, Trace};

#[test]
fn stepping_into_a_host_function_completes_it_before_the_next_stop() -> Result<()> {
    for arm64 in [true, false] {
        // Save lr; call the property stub in register 2; return.
        let words = if arm64 {
            [0xaa1e03f3, 0xd63f0040, 0xd65f0260]
        } else {
            [0xe1a0400e, 0xe12fff32, 0xe12fff14]
        };
        let mut emu = emulator(arm64, &words)?;
        let property = emu.resolve("__system_property_get")?;
        let key = emu.new_cstring("ro.build.version.sdk")?;
        let out = emu.malloc(64)?;
        for address in [CODE + 4, property] {
            emu.add_breakpoint(address)?;
            let mut stops = Vec::new();
            let mut handler = Handler(|emu: &mut Emulator, stop: &Stop| {
                stops.push((stop.reason, stop.address));
                if stops.len() == 1 {
                    Resume::Step
                } else {
                    assert_eq!(emu.registers().unwrap()[0].1, 2);
                    assert_eq!(emu.read_bytes(out, 3).unwrap(), b"23\0");
                    Resume::Continue
                }
            });
            assert_eq!(
                emu.call_function_with(&mut handler, CODE, &[key, out, property])?,
                2
            );
            assert_eq!(
                stops,
                [
                    (StopReason::Breakpoint, address),
                    (StopReason::Step, CODE + 8)
                ]
            );
            emu.remove_breakpoint(address)?;
        }
    }
    Ok(())
}

#[test]
fn traced_imports_emit_once_across_break_step_finish_and_return() -> Result<()> {
    for (abi, words) in [
        ("arm64-v8a", [0xaa1e03f3u32, 0xd63f0040, 0xd65f0260]),
        ("armeabi-v7a", [0xe1a0400e, 0xe12fff32, 0xe12fff14]),
    ] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .on_event(move |e| {
                if let Event::Call { function, .. } = e {
                    sink.borrow_mut().push(function.clone());
                }
            })
            .build()?;
        let m = emu.load_library(format!("tests/fixtures/{abi}/libjnidispatch.so"))?;
        let code = emu.find_symbol(m, "JNI_OnLoad")? & !1;
        let protect = emu.resolve("mprotect")?;
        assert_eq!(emu.call_function(protect, &[code & !4095, 4096, 7])?, 0);
        emu.write_bytes(
            code,
            &words
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        let string = emu.new_cstring("hello")?;
        let strlen = emu.resolve("strlen")?;
        emu.set_trace(Trace {
            imports: true,
            ..Trace::default()
        })?;
        emu.add_breakpoint(strlen)?;
        for resume in [
            Resume::Continue,
            Resume::Step,
            Resume::Finish,
            Resume::Return(42),
            Resume::Continue,
        ] {
            events.borrow_mut().clear();
            let mut hits = 0;
            let mut handler = Handler(|_: &mut Emulator, _: &Stop| {
                hits += 1;
                if hits == 1 { resume } else { Resume::Continue }
            });
            let result = emu.call_function_with(&mut handler, code, &[string, 0, strlen])?;
            assert_eq!(result, if resume == Resume::Return(42) { 42 } else { 5 });
            assert_eq!(*events.borrow(), ["strlen"], "{abi} {resume:?}");
        }
        emu.remove_breakpoint(strlen)?;
        // A step ending on the first instruction of an import emits it only once.
        emu.add_breakpoint(code + 4)?;
        events.borrow_mut().clear();
        let mut hits = 0;
        let mut handler = Handler(|_: &mut Emulator, stop: &Stop| {
            hits += 1;
            if hits == 1 {
                Resume::Step
            } else if hits == 2 {
                assert_eq!(stop.address, strlen & !1);
                Resume::Finish
            } else {
                Resume::Continue
            }
        });
        assert_eq!(
            emu.call_function_with(&mut handler, code, &[string, 0, strlen])?,
            5
        );
        assert_eq!(*events.borrow(), ["strlen"]);
        emu.remove_breakpoint(code + 4)?;
        emu.add_breakpoint(strlen)?;
        events.borrow_mut().clear();
        let mut handler = Handler(|emu: &mut Emulator, _: &Stop| {
            assert_eq!(*events.borrow(), ["strlen"]);
            let mut inner = Handler(|_: &mut Emulator, _: &Stop| {
                assert_eq!(*events.borrow(), ["strlen", "strlen"]);
                Resume::Continue
            });
            assert_eq!(
                emu.call_function_with(&mut inner, code, &[string, 0, strlen])
                    .unwrap(),
                5
            );
            Resume::Continue
        });
        assert_eq!(
            emu.call_function_with(&mut handler, code, &[string, 0, strlen])?,
            5
        );
        assert_eq!(*events.borrow(), ["strlen", "strlen"]);
    }
    Ok(())
}
