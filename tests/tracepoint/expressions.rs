use super::*;

#[test]
fn expressions_share_named_registers_and_read_nested_little_endian_memory() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            [0x91000400, 0xd65f03c0]
        } else {
            [0xe2800001, 0xe12fff1e]
        };
        let (mut emu, events) = emulator(arm64, &words)?;
        let reg = if arm64 { "x0" } else { "r0" };
        let base = emu.malloc(64)?;
        let target = base + 32;
        emu.write_bytes(base + 24, &target.to_le_bytes())?;
        emu.write_bytes(target, &0xfedcba9876543280u64.to_le_bytes())?;
        emu.set_register("d0", 0x8877665544332211)?;
        let names = [
            reg.to_string(),
            "0xfedcba9876543210".into(),
            "d0".into(),
            "sp".into(),
            "pc".into(),
            "lr".into(),
            format!("[{reg}+0x18]"),
            format!("[[{reg} + 0x18]]:1"),
            format!("[[{reg}+0x18]]:2"),
            format!("[[{reg}+0x18]]:4"),
            format!("[[{reg}+0x18]]:8"),
            format!("[[{reg}+0x18] - 0x8]"),
            format!("[0x{target:x}]:1"),
            "[[0x0]]:8".into(),
        ];
        emu.add_tracepoint(
            CODE,
            &Tracepoint {
                values: names.to_vec(),
                log: true,
            },
        )?;
        emu.add_breakpoint(CODE)?;
        let mut sp = 0;
        let mut handler = Handler(|emu: &mut Emulator, _: &Stop| {
            sp = emu.register("sp").unwrap();
            Resume::Continue
        });
        assert_eq!(
            emu.call_function_with(&mut handler, CODE, &[base])?,
            base + 1
        );
        let values = [
            Some(base),
            Some(0xfedcba9876543210),
            Some(0x8877665544332211),
            Some(sp),
            Some(CODE),
            Some(0x20000),
            Some(target),
            Some(0x80),
            Some(0x3280),
            Some(0x76543280),
            Some(0xfedcba9876543280),
            Some(target),
            Some(0x80),
            None,
        ];
        assert_eq!(events.borrow().len(), 1);
        assert_eq!(
            events.borrow()[0].0["values"],
            json!(
                names
                    .iter()
                    .zip(values)
                    .map(|(name, value)| (name, value.map(|v| format!("0x{v:x}"))))
                    .collect::<Vec<_>>()
            )
        );
    }
    Ok(())
}

#[test]
fn malformed_expressions_and_conditions_name_the_bad_text_and_keep_existing_points() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            [0x91000400, 0xd65f03c0]
        } else {
            [0xe2800001, 0xe12fff1e]
        };
        let (mut emu, _) = emulator(arm64, &words)?;
        emu.add_tracepoint(
            CODE,
            &Tracepoint {
                values: vec!["0x7".into()],
                log: false,
            },
        )?;
        for text in [
            "",
            "[]",
            "0x",
            "0x10000000000000000",
            "[0x0",
            "[0x0]:3",
            "[0x0]:16",
            "0x0+0x1",
            "[0x0+1]",
            "[0x0+0x1+0x2]",
            "0x1 junk",
            "notareg",
            "[unknown]:1",
            "[0x0]:",
            "[0x0]:8:1",
        ] {
            let error = emu
                .add_tracepoint(
                    CODE,
                    &Tracepoint {
                        values: vec![text.into()],
                        log: true,
                    },
                )
                .unwrap_err()
                .to_string();
            assert!(error.contains(&format!("{text:?}")), "{error}");
        }
        let error = emu
            .add_tracepoint(
                CODE,
                &Tracepoint {
                    values: vec![if arm64 { "r4" } else { "x20" }.into()],
                    log: true,
                },
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown register"));
        assert_eq!(
            emu.add_tracepoint(
                CODE,
                &Tracepoint {
                    values: vec![],
                    log: false
                }
            )
            .unwrap_err()
            .to_string(),
            "tracepoint needs at least one value"
        );
        emu.add_conditional_breakpoint(CODE, "0x0==0x0")?;
        for text in [
            "",
            "0x1",
            "0x1=0x1",
            "0x1<>0x1",
            "0x1===0x1",
            "0x1==",
            "==0x1",
            "0x0!=[bad]",
            "0x1==0x1!=0x0",
            "[0x1]:3==0x0",
        ] {
            let error = emu
                .add_conditional_breakpoint(CODE, text)
                .unwrap_err()
                .to_string();
            assert!(error.contains(&format!("{text:?}")), "{error}");
        }
        let mut stops = 0;
        let mut handler = Handler(|_: &mut Emulator, _: &Stop| {
            stops += 1;
            Resume::Continue
        });
        assert_eq!(emu.call_function_with(&mut handler, CODE, &[7])?, 8);
        assert_eq!(stops, 1);
        assert_eq!(emu.tracepoint_hits(CODE)?, [(7, 1)]);
    }
    Ok(())
}

#[test]
fn replacement_clears_counts_none_is_not_counted_and_reset_clears_all_points() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            [0x91000400, 0xd65f03c0]
        } else {
            [0xe2800001, 0xe12fff1e]
        };
        let (mut emu, events) = emulator(arm64, &words)?;
        let reg = if arm64 { "x0" } else { "r0" };
        for at in [CODE, CODE + 4] {
            emu.add_tracepoint(
                at,
                &Tracepoint {
                    values: vec![reg.into()],
                    log: false,
                },
            )?;
        }
        for input in [9, 2, 9] {
            assert_eq!(emu.call_function(CODE, &[input])?, input + 1);
        }
        assert!(events.borrow().is_empty());
        assert_eq!(emu.tracepoint_hits(CODE | 1)?, [(2, 1), (9, 2)]);
        assert_eq!(emu.tracepoint_hits(CODE + 4)?, [(3, 1), (10, 2)]);
        emu.reset_tracepoint_hits();
        assert!(emu.tracepoint_hits(CODE)?.is_empty());
        assert!(emu.tracepoint_hits(CODE + 4)?.is_empty());
        emu.add_tracepoint(
            CODE | 1,
            &Tracepoint {
                values: vec!["[0x0]:8".into()],
                log: true,
            },
        )?;
        assert_eq!(emu.call_function(CODE, &[9])?, 10);
        assert!(emu.tracepoint_hits(CODE)?.is_empty());
        assert_eq!(events.borrow()[0].0["values"], json!([["[0x0]:8", null]]));
        emu.remove_tracepoint(CODE | 1)?;
        emu.remove_tracepoint(CODE + 4)?;
        for error in [
            emu.remove_tracepoint(CODE).unwrap_err(),
            emu.tracepoint_hits(CODE).unwrap_err(),
        ] {
            assert_eq!(error.to_string(), "no tracepoint at 0x20100");
        }
        assert_eq!(emu.call_function(CODE, &[9])?, 10);
        assert_eq!(events.borrow().len(), 1);
    }
    Ok(())
}
