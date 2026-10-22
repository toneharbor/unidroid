use std::{cell::RefCell, rc::Rc};
use unidroid::{Emulator, Event, Result, Trace};

#[path = "instruction_trace/control.rs"]
mod control;

#[test]
fn instruction_json_keeps_full_width_values_as_hex_strings_in_order() -> Result<()> {
    let event = Event::Instruction {
        at: "libtest.so+0x1000".into(),
        address: 0x10001000,
        bytes: "e10300aa".into(),
        text: "mov x1, x0".into(),
        read: vec![
            ("x0".into(), 0xfedcba9876543210),
            ("v1".into(), 0xfedcba98765432100123456789abcdef),
        ],
        written: vec![("x1".into(), 0xfedcba9876543210)],
    };
    assert_eq!(
        serde_json::to_value(&event)?,
        serde_json::json!({
            "event":"instruction", "at":"libtest.so+0x1000", "address":"0x10001000", "bytes":"e10300aa", "text":"mov x1, x0",
            "read":[["x0","0xfedcba9876543210"],["v1","0xfedcba98765432100123456789abcdef"]], "written":[["x1","0xfedcba9876543210"]]
        })
    );
    Ok(())
}

#[test]
fn instruction_values_survive_range_exit_and_trace_can_turn_off() -> Result<()> {
    for (directory, words, register, text) in [
        (
            "lib64",
            [0xd2800020u32, 0xd2800040, 0xd65f03c0],
            "x0",
            "mov x0, #1",
        ),
        (
            "lib",
            [0xe3a00001, 0xe3a00002, 0xe12fff1e],
            "r0",
            "mov r0, #1",
        ),
    ] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .on_event(move |event| {
                if matches!(event, Event::Instruction { .. }) {
                    sink.borrow_mut().push(serde_json::to_value(event).unwrap());
                }
            })
            .build()?;
        emu.load_library(format!("rootfs/{directory}/libc.so"))?;
        let code = emu.modules()[0].base + 0x1000;
        let protect = emu.resolve("mprotect")?;
        assert_eq!(emu.call_function(protect, &[code, 4096, 7])?, 0);
        emu.write_bytes(
            code,
            &words
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        emu.set_trace(Trace {
            code: vec!["libc.so+0x1000-0x1004".into()],
            ..Trace::default()
        })?;
        assert_eq!(emu.call_function(code, &[0])?, 2);
        let captured = events.borrow();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0]["text"], text);
        assert_eq!(captured[0]["read"], serde_json::json!([]));
        assert_eq!(
            captured[0]["written"],
            serde_json::json!([[register, "0x1"]])
        );
        drop(captured);
        emu.set_trace(Trace::default())?;
        assert_eq!(emu.call_function(code, &[0])?, 2);
        assert_eq!(events.borrow().len(), 1);
    }
    Ok(())
}

#[test]
fn exact_register_values_include_thumb_and_the_faulting_instruction() -> Result<()> {
    for (directory, thumb, words, expected) in [
        (
            "lib64",
            false,
            vec![0xaa0003e1u32, 0x91000c20, 0xf9400040],
            vec!["mov x1, x0", "add x0, x1, #3", "ldr x0, [x2]"],
        ),
        (
            "lib",
            false,
            vec![0xe1a01000, 0xe2810003, 0xe5920000],
            vec!["mov r1, r0", "add r0, r1, #3", "ldr r0, [r2]"],
        ),
        (
            "lib",
            true,
            vec![0x4601, 0x1cc8, 0x6810],
            vec!["mov r1, r0", "adds r0, r1, #3", "ldr r0, [r2]"],
        ),
    ] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .on_event(move |event| {
                if matches!(event, Event::Instruction { .. }) {
                    sink.borrow_mut()
                        .push((event.to_string(), serde_json::to_value(event).unwrap()));
                }
            })
            .build()?;
        emu.load_library(format!("rootfs/{directory}/libc.so"))?;
        let code = emu.modules()[0].base + 0x1000;
        let protect = emu.resolve("mprotect")?;
        emu.call_function(protect, &[code, 4096, 7])?;
        let width = if thumb { 2 } else { 4 };
        let bytes: Vec<u8> = words
            .iter()
            .flat_map(|w| w.to_le_bytes()[..width].to_vec())
            .collect();
        emu.write_bytes(code, &bytes)?;
        emu.set_trace(Trace {
            code: vec![format!("libc.so+0x1000-0x{:x}", 0x1000 + bytes.len())],
            ..Trace::default()
        })?;
        assert!(
            emu.call_function(code | u64::from(thumb), &[7, 0, 0])
                .is_err()
        );
        let events = events.borrow();
        assert_eq!(events.len(), 3, "{directory} thumb={thumb}: {events:?}");
        for (event, text) in events.iter().zip(expected) {
            assert_eq!(event.1["text"], text);
            println!("{}\n{}", event.0, event.1);
        }
        let prefix = if directory == "lib64" { "x" } else { "r" };
        assert_eq!(
            events[0].1["read"],
            serde_json::json!([[format!("{prefix}0"), "0x7"]])
        );
        assert_eq!(
            events[0].1["written"],
            serde_json::json!([[format!("{prefix}1"), "0x7"]])
        );
        assert_eq!(
            events[1].1["read"],
            serde_json::json!([[format!("{prefix}1"), "0x7"]])
        );
        if thumb {
            assert_eq!(
                events[1].1["written"],
                serde_json::json!([["r0", "0xa"], ["cpsr", "0x1f3"]])
            );
        } else {
            assert_eq!(
                events[1].1["written"],
                serde_json::json!([[format!("{prefix}0"), "0xa"]])
            );
        }
        assert_eq!(
            events[2].1["read"],
            serde_json::json!([[format!("{prefix}2"), "0x0"]])
        );
        assert_eq!(events[2].1["written"], serde_json::json!([]));
    }
    Ok(())
}

#[test]
fn bad_specs_fail_before_loading_or_changing_settings() -> Result<()> {
    for spec in [
        "",
        "libc.so+",
        "libc.so+1-2",
        "libc.so+0x2-0x1",
        "libc.so+0x1-0x1",
        "libc.so+0xz-0x3",
        "a/libc.so",
    ] {
        let trace = Trace {
            code: vec![spec.into()],
            ..Trace::default()
        };
        assert!(
            Emulator::builder().trace(trace.clone()).build().is_err(),
            "{spec}"
        );
        let mut emu = Emulator::builder().build()?;
        assert!(emu.set_trace(trace).is_err(), "{spec}");
        assert!(emu.trace().code.is_empty());
    }
    Ok(())
}
