use unidroid::{BlockCount, Emulator, Result, Trace};

const MODES: [(bool, bool); 3] = [(true, false), (false, false), (false, true)];

fn words(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

fn halfwords(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

fn emulator(arm64: bool, thumb: bool) -> Result<(Emulator, u64)> {
    let mut emu = Emulator::builder().on_event(|_| {}).build()?;
    emu.load_library(format!(
        "rootfs/{}/libc.so",
        if arm64 { "lib64" } else { "lib" }
    ))?;
    let code = emu.modules()[0].base + 0x1000;
    let protect = emu.resolve("mprotect")?;
    emu.call_function(protect, &[code, 4096, 7])?;
    // Branch to a loop that decrements the argument to zero, then return.
    let bytes = if thumb {
        halfwords(&[0xe7ff, 0x3801, 0xd1fd, 0x4770])
    } else if arm64 {
        words(&[0x14000001, 0x71000400, 0x54ffffe1, 0xd65f03c0])
    } else {
        words(&[0xeaffffff, 0xe2500001, 0x1afffffd, 0xe12fff1e])
    };
    emu.write_bytes(code, &bytes)?;
    Ok((emu, code | u64::from(thumb)))
}

#[test]
fn coverage_configured_before_load_counts_constructors() -> Result<()> {
    for directory in ["lib64", "lib"] {
        let mut emu = Emulator::builder()
            .trace(Trace {
                coverage: vec!["libc.so".into()],
                ..Trace::default()
            })
            .build()?;
        assert!(emu.coverage().is_empty());
        emu.reset_coverage();
        emu.load_library(format!("rootfs/{directory}/libc.so"))?;
        let blocks = emu.coverage();
        assert!(!blocks.is_empty());
        assert!(
            blocks
                .windows(2)
                .all(|pair| pair[0].address < pair[1].address)
        );
        for block in blocks {
            assert_eq!(block.location, emu.symbolize(block.address));
            assert!(block.location.starts_with("libc.so+"));
            assert!(block.count > 0);
        }
    }
    Ok(())
}

#[test]
fn reset_separates_input_dependent_paths_in_address_order() -> Result<()> {
    for (arm64, thumb) in MODES {
        let (mut emu, entry) = emulator(arm64, thumb)?;
        let start = entry & !1;
        // Zero skips the block that sets the result to seven.
        let (bytes, path, exit) = if thumb {
            (halfwords(&[0x2800, 0xd001, 0x2007, 0xe7ff, 0x4770]), 4, 8)
        } else if arm64 {
            (
                words(&[0xb4000060, 0x528000e0, 0x14000001, 0xd65f03c0]),
                4,
                12,
            )
        } else {
            (
                words(&[0xe3500000, 0x0a000001, 0xe3a00007, 0xeaffffff, 0xe12fff1e]),
                8,
                16,
            )
        };
        emu.write_bytes(start, &bytes)?;
        emu.set_trace(Trace {
            coverage: vec!["libc.so+0x1000-0x1040".into()],
            ..Trace::default()
        })?;
        assert_eq!(emu.call_function(entry, &[0])?, 0);
        let first = emu.coverage();
        assert_eq!(
            first.iter().map(|b| b.address).collect::<Vec<_>>(),
            [start, start + exit]
        );
        emu.reset_coverage();
        assert!(emu.coverage().is_empty());
        assert_eq!(emu.call_function(entry, &[1])?, 7);
        let second = emu.coverage();
        assert_eq!(
            second.iter().map(|b| b.address).collect::<Vec<_>>(),
            [start, start + path, start + exit]
        );
        assert!(first.iter().chain(&second).all(|b| b.count == 1));
    }
    Ok(())
}

#[test]
fn trace_changes_and_disabling_preserve_counts_until_reset() -> Result<()> {
    for (arm64, thumb) in MODES {
        let (mut emu, entry) = emulator(arm64, thumb)?;
        let offset = if thumb { 0x1002 } else { 0x1004 };
        let spec = format!("libc.so+0x{offset:x}-0x{:x}", offset + 1);
        emu.set_trace(Trace {
            coverage: vec![spec.clone()],
            ..Trace::default()
        })?;
        assert_eq!(emu.call_function(entry, &[5])?, 0);
        assert_eq!(emu.coverage()[0].count, 5);
        emu.set_trace(Trace {
            jni: true,
            syscalls: true,
            imports: true,
            heap: false,
            code: vec!["libc.so+0x1000-0x1040".into()],
            calls: vec!["libc.so+0x1000-0x1040".into()],
            coverage: vec![spec.clone()],
        })?;
        assert_eq!(emu.coverage()[0].count, 5);
        assert_eq!(emu.call_function(entry, &[5])?, 0);
        assert_eq!(emu.coverage()[0].count, 10);
        let exit = if thumb { 0x1006 } else { 0x100c };
        emu.set_trace(Trace {
            coverage: vec![format!("libc.so+0x{exit:x}-0x{:x}", exit + 1)],
            ..Trace::default()
        })?;
        assert_eq!(emu.coverage()[0].count, 10);
        assert_eq!(emu.call_function(entry, &[5])?, 0);
        let saved = serde_json::to_value(emu.coverage())?;
        assert_eq!(saved.as_array().unwrap().len(), 2);
        assert_eq!(saved[0]["count"], 10);
        assert_eq!(saved[1]["count"], 1);
        emu.set_trace(Trace::default())?;
        assert_eq!(emu.call_function(entry, &[5])?, 0);
        assert_eq!(serde_json::to_value(emu.coverage())?, saved);
        emu.reset_coverage();
        assert!(emu.coverage().is_empty());
        assert_eq!(emu.call_function(entry, &[5])?, 0);
        assert!(emu.coverage().is_empty());
        emu.set_trace(Trace {
            coverage: vec![spec],
            ..Trace::default()
        })?;
        assert_eq!(emu.call_function(entry, &[5])?, 0);
        assert_eq!(emu.coverage()[0].count, 5);
    }
    Ok(())
}

#[test]
fn later_libraries_refresh_selection_without_clearing_earlier_counts() -> Result<()> {
    for arm64 in [true, false] {
        let (mut emu, entry) = emulator(arm64, false)?;
        emu.set_trace(Trace {
            coverage: vec!["libc.so+0x1004-0x1005".into(), "libm.so".into()],
            ..Trace::default()
        })?;
        assert_eq!(emu.call_function(entry, &[5])?, 0);
        let module = emu.load_library(format!(
            "rootfs/{}/libm.so",
            if arm64 { "lib64" } else { "lib" }
        ))?;
        let function = emu.find_symbol(module, "fabs")?;
        emu.call_function(function, &[])?;
        let blocks = emu.coverage();
        assert_eq!(blocks[0].address, entry + 4);
        assert_eq!(blocks[0].count, 5);
        let block = blocks.iter().find(|b| b.address == function & !1).unwrap();
        assert_eq!(block.location, emu.symbolize(function & !1));
        assert!(block.location.contains("(fabs)"), "{block:?}");
        assert_eq!(block.count, 1);
    }
    Ok(())
}

#[test]
fn nested_calls_share_the_same_counts() -> Result<()> {
    struct Nested {
        entry: u64,
        stops: usize,
    }
    impl unidroid::JniHandler for Nested {
        fn stopped(&mut self, emu: &mut Emulator, _: &unidroid::Stop) -> unidroid::Resume {
            self.stops += 1;
            assert_eq!(emu.call_function(self.entry, &[5]).unwrap(), 0);
            unidroid::Resume::Continue
        }
    }
    for (arm64, thumb) in MODES {
        let (mut emu, entry) = emulator(arm64, thumb)?;
        let offset = if thumb { 2 } else { 4 };
        emu.set_trace(Trace {
            coverage: vec![format!(
                "libc.so+0x{:x}-0x{:x}",
                0x1000 + offset,
                0x1001 + offset
            )],
            ..Trace::default()
        })?;
        emu.add_breakpoint(entry)?;
        let mut handler = Nested {
            entry: entry + offset,
            stops: 0,
        };
        assert_eq!(emu.call_function_with(&mut handler, entry, &[3])?, 0);
        assert_eq!(handler.stops, 1);
        let blocks = emu.coverage();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].count, 8);
    }
    Ok(())
}

#[test]
fn guest_threads_and_the_main_thread_share_counts() -> Result<()> {
    for (arm64, thumb) in MODES {
        let (mut emu, entry) = emulator(arm64, thumb)?;
        let offset = if thumb { 0x1002 } else { 0x1004 };
        emu.set_trace(Trace {
            coverage: vec![format!("libc.so+0x{offset:x}-0x{:x}", offset + 1)],
            ..Trace::default()
        })?;
        let create = emu.resolve("pthread_create")?;
        let join = emu.resolve("pthread_join")?;
        let out = emu.malloc(8)?;
        let mut threads = Vec::new();
        for iterations in [3, 5] {
            emu.write_bytes(out, &[0; 8])?;
            assert_eq!(emu.call_function(create, &[out, 0, entry, iterations])?, 0);
            threads.push(u64::from_le_bytes(
                emu.read_bytes(out, 8)?.try_into().unwrap(),
            ));
        }
        assert_eq!(emu.call_function(entry, &[7])?, 0);
        for thread in threads {
            assert_eq!(emu.call_function(join, &[thread, 0])?, 0);
        }
        let blocks = emu.coverage();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].count, 15);
    }
    Ok(())
}

#[test]
fn ranges_exclude_the_end_and_blocks_that_only_overlap() -> Result<()> {
    for (arm64, thumb) in MODES {
        let (mut emu, entry) = emulator(arm64, thumb)?;
        let offset = if thumb { 0x1002 } else { 0x1004 };
        emu.set_trace(Trace {
            coverage: vec![format!("libc.so+0x1000-0x{offset:x}")],
            ..Trace::default()
        })?;
        assert_eq!(emu.call_function(entry, &[5])?, 0);
        let blocks = emu.coverage();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].address, entry & !1);
        assert_eq!(blocks[0].count, 1);
        emu.reset_coverage();
        emu.set_trace(Trace {
            coverage: vec![format!("libc.so+0x{:x}-0x{:x}", offset + 1, offset + 2)],
            ..Trace::default()
        })?;
        assert_eq!(emu.call_function(entry, &[5])?, 0);
        assert!(emu.coverage().is_empty());
    }
    Ok(())
}

#[test]
fn invalid_specs_use_the_shared_parser_and_preserve_active_coverage() -> Result<()> {
    for spec in ["", "libc.so+1-2", "libc.so+0x2-0x1", "a/libc.so"] {
        let trace = Trace {
            coverage: vec![spec.into()],
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
        for (arm64, thumb) in MODES {
            let (mut emu, entry) = emulator(arm64, thumb)?;
            let selection = "libc.so+0x1000-0x1040";
            emu.set_trace(Trace {
                coverage: vec![selection.into()],
                ..Trace::default()
            })?;
            assert_eq!(emu.call_function(entry, &[5])?, 0);
            let before = emu.coverage();
            assert_eq!(
                emu.set_trace(trace.clone()).unwrap_err().to_string(),
                expected
            );
            assert_eq!(emu.trace().coverage, [selection]);
            assert_eq!(
                serde_json::to_value(emu.coverage())?,
                serde_json::to_value(&before)?
            );
            assert_eq!(emu.call_function(entry, &[5])?, 0);
            for (old, new) in before.iter().zip(emu.coverage()) {
                assert_eq!(new.address, old.address);
                assert_eq!(new.count, old.count * 2);
            }
        }
    }
    Ok(())
}

#[test]
fn counts_loop_blocks_once_per_hit_only_when_the_start_is_selected() -> Result<()> {
    for (arm64, thumb) in MODES {
        let (mut emu, entry) = emulator(arm64, thumb)?;
        assert_eq!(emu.call_function(entry, &[5])?, 0);
        assert!(emu.coverage().is_empty());
        let offset = if thumb { 0x1002 } else { 0x1004 };
        // Overlapping ranges must count once. The block extends beyond their end.
        emu.set_trace(Trace {
            coverage: vec![
                format!("libc.so+0x{offset:x}-0x{:x}", offset + 1),
                format!("libc.so+0x{offset:x}-0x{:x}", offset + 2),
            ],
            ..Trace::default()
        })?;
        assert_eq!(emu.call_function(entry, &[5])?, 0);
        let blocks: Vec<BlockCount> = emu.coverage();
        assert_eq!(blocks.len(), 1, "{blocks:?}");
        let block = blocks[0].clone();
        assert_eq!(block.address, (entry & !1) + if thumb { 2 } else { 4 });
        assert_eq!(block.location, format!("libc.so+0x{offset:x}"));
        assert_eq!(block.count, 5);
        assert_eq!(serde_json::to_value(&block)?["count"], 5);
    }
    Ok(())
}
