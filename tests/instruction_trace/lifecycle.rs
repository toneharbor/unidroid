use super::*;

#[test]
fn disjoint_range_exits_and_a_boundary_branch_emit_each_event_once() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            [0xd2800020, 0xd2800040, 0xd2800061, 0xd2800081, 0xd65f03c0]
        } else {
            [0xe3a00001, 0xe3a00002, 0xe3a01003, 0xe3a01004, 0xe12fff1e]
        };
        let (mut emu, code, events) = emulator(arm64, &words, false)?;
        emu.set_trace(Trace {
            code: vec![
                "libc.so+0x1000-0x1004".into(),
                "libc.so+0x1008-0x100c".into(),
            ],
            ..Trace::default()
        })?;
        assert_eq!(emu.call_function(code, &[])?, 2);
        let prefix = if arm64 { "x" } else { "r" };
        assert_eq!(events.borrow().len(), 2);
        assert_eq!(
            events.borrow()[0]["written"],
            serde_json::json!([[format!("{prefix}0"), "0x1"]])
        );
        assert_eq!(
            events.borrow()[1]["written"],
            serde_json::json!([[format!("{prefix}1"), "0x3"]])
        );
        // A branch to the boundary invokes both exit observers. Pending is consumed once.
        let branch: u32 = if arm64 { 0x14000001 } else { 0xeaffffff };
        emu.write_bytes(code, &branch.to_le_bytes())?;
        events.borrow_mut().clear();
        assert_eq!(emu.call_function(code, &[])?, 2);
        assert_eq!(events.borrow().len(), 2);
        assert_eq!(events.borrow()[0]["address"], format!("0x{code:x}"));
        assert_eq!(events.borrow()[1]["address"], format!("0x{:x}", code + 8));
    }
    Ok(())
}

#[test]
fn conditional_code_is_unchanged_and_stopped_instructions_are_not_duplicated() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            vec![
                0xf100001f, 0x9a800020, 0x9a800020, 0x9a800020, 0x9a800020, 0x91000400, 0xd65f03c0,
            ]
        } else {
            vec![0xbf012800, 0x202a202a, 0x202a202a, 0x47703001]
        };
        let (mut emu, code, events) = emulator(arm64, &words, false)?;
        for input in [0, 1] {
            events.borrow_mut().clear();
            let entry = code | u64::from(!arm64);
            assert_eq!(
                emu.call_function(entry, &[input, 42])?,
                if input == 0 { 43 } else { 2 }
            );
            let baseline = events.borrow().clone();
            if !arm64 && input == 0 {
                let conditional = baseline
                    .iter()
                    .find(|v| v["address"] == format!("0x{:x}", code + 4))
                    .unwrap();
                assert_eq!(conditional["text"], "moveq r0, #0x2a");
                assert_eq!(conditional["written"], serde_json::json!([["r0", "0x2a"]]));
            }
            emu.add_breakpoint(code + 4)?;
            events.borrow_mut().clear();
            assert_eq!(
                emu.call_function(entry, &[input, 42])?,
                if input == 0 { 43 } else { 2 }
            );
            let stopped = events.borrow().clone();
            assert_eq!(
                baseline.iter().map(|v| &v["address"]).collect::<Vec<_>>(),
                stopped.iter().map(|v| &v["address"]).collect::<Vec<_>>()
            );
            emu.remove_breakpoint(code + 4)?;
        }
    }
    Ok(())
}

#[test]
fn final_return_fault_and_overlapping_ranges_keep_each_instruction_once() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            [0xd2800540, 0xd65f03c0]
        } else {
            [0xe3a0002a, 0xe12fff1e]
        };
        let (mut emu, code, events) = emulator(arm64, &words, false)?;
        emu.set_trace(Trace {
            code: vec![
                "libc.so+0x1000-0x1008".into(),
                "libc.so+0x1004-0x1008".into(),
            ],
            ..Trace::default()
        })?;
        assert_eq!(emu.call_function(code, &[])?, 42);
        assert_eq!(events.borrow().len(), 2);
        assert_eq!(
            events.borrow()[1]["text"],
            if arm64 { "ret" } else { "bx lr" }
        );
        events.borrow_mut().clear();
        let invalid: u32 = if arm64 { 0xffffffff } else { 0xe7f000f0 };
        emu.write_bytes(code, &invalid.to_le_bytes())?;
        assert!(emu.call_function(code, &[]).is_err());
        assert_eq!(events.borrow().len(), 1);
        assert_eq!(events.borrow()[0]["written"], serde_json::json!([]));
    }
    Ok(())
}

#[test]
fn d_and_q_values_are_not_truncated() -> Result<()> {
    for arm64 in [true, false] {
        // Load v1/q1, then copy it to v0/q0. The upper 64 bits must survive in the event.
        let captured = Rc::new(RefCell::new(Vec::new()));
        let sink = captured.clone();
        let mut emu = Emulator::builder()
            .on_event(move |event| {
                if let Event::Instruction { read, written, .. } = event {
                    sink.borrow_mut().push((read.clone(), written.clone()));
                }
            })
            .build()?;
        emu.load_library(if arm64 {
            "rootfs/lib64/libc.so"
        } else {
            "rootfs/lib/libc.so"
        })?;
        let code = emu.modules()[0].base + 0x1000;
        let protect = emu.resolve("mprotect")?;
        emu.call_function(protect, &[code, 4096, 7])?;
        let words: [u32; 3] = if arm64 {
            [0x3dc00001, 0x4ea11c20, 0xd65f03c0]
        } else {
            [0xe1a00000, 0xf2220152, 0xe12fff1e]
        };
        emu.write_bytes(
            code,
            &words
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        let value = 0xfedcba98765432100123456789abcdefu128;
        let buffer = emu.malloc(16)?;
        emu.write_bytes(buffer, &value.to_le_bytes())?;
        if !arm64 {
            emu.set_register("d2", 0x0123456789abcdef)?;
            emu.set_register("d3", 0xfedcba9876543210)?;
        }
        emu.set_trace(Trace {
            code: vec!["libc.so+0x1004-0x1008".into()],
            ..Trace::default()
        })?;
        emu.call_function(code, &[buffer])?;
        let name = if arm64 { "v" } else { "q" };
        assert_eq!(
            *captured.borrow(),
            vec![(
                vec![(format!("{name}1"), value)],
                vec![(format!("{name}0"), value)]
            )]
        );
    }
    Ok(())
}
