use std::{cell::Cell, rc::Rc};
use unidroid::{Emulator, Event, Result, Trace};

#[test]
fn tracing_imports_preserves_thread_progress() -> Result<()> {
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        for (imports, before_spawn, code) in [
            (false, false, false),
            (true, false, false),
            (true, true, false),
            (false, false, true),
            (false, true, true),
            (true, false, true),
            (true, true, true),
        ] {
            let instructions = Rc::new(Cell::new(0));
            let instruction_sink = instructions.clone();
            let calls = Rc::new(Cell::new(0));
            let sink = calls.clone();
            let mut emu = Emulator::builder()
                .on_event(move |event| {
                    if matches!(event, Event::Instruction { text, .. } if text.starts_with("blr ") || text.starts_with("blx ")) {
                        instruction_sink.set(instruction_sink.get() + 1);
                    }
                    if matches!(event, Event::Call { function, .. } if function == "strlen") {
                        sink.set(sink.get() + 1);
                    }
                })
                .build()?;
            let m = emu.load_library(format!("tests/fixtures/{abi}/libjnidispatch.so"))?;
            let loop_address = emu.find_symbol(m, "JNI_OnLoad")? & !1;
            let protect = emu.find_symbol(m, "mprotect")?;
            assert_eq!(
                emu.call_function(protect, &[loop_address & !4095, 4096, 7])?,
                0
            );
            let (worker, main): (&[u32], &[u32]) = if abi == "arm64-v8a" {
                (
                    &[0x52800021, 0xb9000001, 0xd65f03c0], // *arg = 1; return.
                    &[
                        0xa9bf7bfd, 0x910003fd, 0xa9bf53f3, 0xa9bf5bf5, 0xaa0003f3, 0xaa0103f4,
                        0xaa0203f5, 0x5289c416, 0xaa1403e0, 0xd63f0260, 0x710006d6, 0x54ffffa1,
                        0xb94002a0, 0xa8c15bf5, 0xa8c153f3, 0xa8c17bfd, 0xd65f03c0,
                    ],
                )
            } else {
                (
                    &[0xe3a01001, 0xe5801000, 0xe12fff1e],
                    &[
                        0xe92d41f0, 0xe1a04000, 0xe1a05001, 0xe1a06002, 0xe3047e20, 0xe1a00005,
                        0xe12fff34, 0xe2577001, 0x1afffffb, 0xe5960000, 0xe8bd81f0,
                    ],
                )
            };
            // The app calls strlen 20,000 times, then reads the worker's flag.
            emu.write_bytes(
                loop_address,
                &main
                    .iter()
                    .flat_map(|w| w.to_le_bytes())
                    .collect::<Vec<_>>(),
            )?;
            emu.write_bytes(
                0x20100,
                &worker
                    .iter()
                    .flat_map(|w| w.to_le_bytes())
                    .collect::<Vec<_>>(),
            )?;
            let flag = emu.malloc(4)?;
            let thread = emu.malloc(8)?;
            let trace = Trace {
                imports,
                code: if code {
                    let start = loop_address - emu.modules()[m.0].base;
                    vec![format!(
                        "libjnidispatch.so+0x{start:x}-0x{:x}",
                        start + main.len() as u64 * 4
                    )]
                } else {
                    Vec::new()
                },
                ..Trace::default()
            };
            if before_spawn {
                emu.set_trace(trace.clone())?;
            }
            let create = emu.find_symbol(m, "pthread_create")?;
            assert_eq!(emu.call_function(create, &[thread, 0, 0x20100, flag])?, 0);
            assert_eq!(emu.read_bytes(flag, 4)?, [0; 4]);
            if !before_spawn {
                emu.set_trace(trace)?;
            }
            let strlen = emu.find_symbol(m, "strlen")?;
            let text = emu.new_cstring("abc")?;
            assert_eq!(
                emu.call_function(loop_address, &[strlen, text, flag])?,
                1,
                "{abi}, imports={imports}, before_spawn={before_spawn}"
            );
            assert_eq!(calls.get(), if imports { 20_000 } else { 0 });
            assert_eq!(instructions.get(), if code { 20_000 } else { 0 });
        }
    }
    Ok(())
}
