use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc};
use unidroid::{Emulator, Result, Trace};

type Events = Rc<RefCell<Vec<(String, Value)>>>;

#[path = "call_trace/lifecycle.rs"]
mod lifecycle;

fn emulator(arm64: bool, thumb: bool, before_load: bool) -> Result<(Emulator, u64, u64, Events)> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    let mut emu = Emulator::builder()
        .trace(Trace {
            calls: if before_load {
                vec!["libc.so+0x1000-0x1010".into()]
            } else {
                Vec::new()
            },
            ..Trace::default()
        })
        .on_event(move |event| {
            sink.borrow_mut()
                .push((event.to_string(), serde_json::to_value(event).unwrap()));
        })
        .build()?;
    emu.load_library(format!(
        "rootfs/{}/libc.so",
        if arm64 { "lib64" } else { "lib" }
    ))?;
    let code = emu.modules()[0].base + 0x1000;
    let protect = emu.resolve("mprotect")?;
    emu.call_function(protect, &[code, 4096, 7])?;
    let (caller, callee) = if thumb {
        // mov r4,lr; bl callee; blx r2; bx r4. callee: adds r0,#1; bx lr.
        (
            halfwords(&[0x4674, 0xf000, 0xf83d, 0x4790, 0x4720]),
            halfwords(&[0x3001, 0x4770]),
        )
    } else if arm64 {
        (
            words(&[0xaa1e03f3, 0x9400001f, 0xd63f0040, 0xd65f0260]),
            words(&[0x91000400, 0xd65f03c0]),
        )
    } else {
        (
            words(&[0xe1a0400e, 0xeb00001d, 0xe12fff32, 0xe12fff14]),
            words(&[0xe2800001, 0xe12fff1e]),
        )
    };
    emu.write_bytes(code, &caller)?;
    emu.write_bytes(code + 0x80, &callee)?;
    events.borrow_mut().clear();
    Ok((
        emu,
        code | u64::from(thumb),
        (code + 0x80) | u64::from(thumb),
        events,
    ))
}

fn words(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn halfwords(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

#[test]
fn immediate_calls_cross_arm_thumb_modes_and_keep_thumb_bits_out_of_addresses() -> Result<()> {
    for (arm64, thumb) in [(true, false), (false, false), (false, true)] {
        let (mut emu, entry, target, events) = emulator(arm64, thumb, false)?;
        let target = if arm64 {
            target
        } else if thumb {
            emu.write_bytes((entry & !1) + 2, &halfwords(&[0xf000, 0xe83e]))?;
            emu.write_bytes(target & !1, &words(&[0xe2800001, 0xe12fff1e]))?;
            target & !1
        } else {
            emu.write_bytes(entry + 4, &words(&[0xfa00001d]))?;
            emu.write_bytes(target, &halfwords(&[0x3001, 0x4770]))?;
            target | 1
        };
        emu.set_trace(Trace {
            calls: vec!["libc.so+0x1000-0x1010".into()],
            ..Trace::default()
        })?;
        assert_eq!(emu.call_function(entry, &[7, 8, target, 10])?, 9);
        assert_eq!(events.borrow().len(), 2);
        for event in events.borrow().iter() {
            assert_eq!(event.1["address"], format!("0x{:x}", target & !1));
        }
    }
    Ok(())
}

#[test]
fn constructor_calls_are_traced_when_the_module_arrives() -> Result<()> {
    for directory in ["lib64", "lib"] {
        let captured = Rc::new(RefCell::new(Vec::new()));
        let sink = captured.clone();
        let mut emu = Emulator::builder()
            .trace(Trace {
                calls: vec!["libc.so".into()],
                ..Trace::default()
            })
            .on_event(move |event| sink.borrow_mut().push(serde_json::to_value(event).unwrap()))
            .build()?;
        emu.load_library(format!("rootfs/{directory}/libc.so"))?;
        let captured = captured.borrow();
        assert!(!captured.is_empty());
        assert!(
            captured
                .iter()
                .all(|event| event["event"] == "function_call")
        );
        for event in captured.iter() {
            let target = u64::from_str_radix(
                event["address"].as_str().unwrap().trim_start_matches("0x"),
                16,
            )?;
            assert_eq!(event["to"], emu.symbolize(target));
            assert!(event["from"].as_str().unwrap().starts_with("libc.so+"));
            assert_eq!(event["args"].as_array().unwrap().len(), 4);
        }
    }
    Ok(())
}

#[test]
fn invalid_call_specs_preserve_existing_trace_settings() -> Result<()> {
    for spec in [
        "",
        "libc.so+",
        "libc.so+1-2",
        "libc.so+0x2-0x1",
        "libc.so+0x1-0x1",
        "a/libc.so",
    ] {
        let trace = Trace {
            calls: vec![spec.into()],
            ..Trace::default()
        };
        let expected =
            format!("invalid code trace {spec:?}: expected libfoo.so or libfoo.so+0x1000-0x2000");
        assert_eq!(
            Emulator::builder()
                .trace(trace.clone())
                .build()
                .err()
                .unwrap()
                .to_string(),
            expected
        );
        let mut emu = Emulator::builder()
            .trace(Trace {
                code: vec!["libc.so".into()],
                ..Trace::default()
            })
            .build()?;
        assert_eq!(emu.set_trace(trace).unwrap_err().to_string(), expected);
        assert_eq!(emu.trace().code, ["libc.so"]);
        assert!(emu.trace().calls.is_empty());
    }
    Ok(())
}

#[test]
fn immediate_and_register_calls_report_targets_sites_and_entry_arguments() -> Result<()> {
    for (arm64, thumb) in [(true, false), (false, false), (false, true)] {
        let (mut emu, entry, target, events) = emulator(arm64, thumb, false)?;
        emu.set_trace(Trace {
            calls: vec!["libc.so+0x1000-0x1010".into()],
            ..Trace::default()
        })?;
        assert!(emu.trace().code.is_empty());
        assert_eq!(emu.call_function(entry, &[7, 8, target, 10])?, 9);
        let captured = events.borrow();
        assert_eq!(captured.len(), 2, "{captured:?}");
        for (i, site) in if thumb { [2, 6] } else { [4, 8] }.into_iter().enumerate() {
            let from = emu.symbolize((entry & !1) + site);
            let to = emu.symbolize(target & !1);
            let args = [
                format!("0x{:x}", 7 + i),
                "0x8".into(),
                format!("0x{target:x}"),
                "0xa".into(),
            ];
            assert_eq!(
                captured[i].1,
                json!({
                    "event":"function_call", "to":to, "address":format!("0x{:x}", target & !1), "args":args, "from":from
                })
            );
            assert_eq!(
                captured[i].0,
                format!("function {to}({}) from {from}", args.join(", "))
            );
        }
    }
    Ok(())
}
