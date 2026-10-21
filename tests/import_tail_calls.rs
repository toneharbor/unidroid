use std::{cell::RefCell, rc::Rc};
use unidroid::{Emulator, Event, Result, Trace};

#[test]
fn imports_exclude_internal_tail_calls_but_include_app_plt_stubs() -> Result<()> {
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let sink = calls.clone();
        let mut emu = Emulator::builder()
            .on_event(move |event| {
                if let Event::Call { function, from, .. } = event {
                    sink.borrow_mut().push((function.clone(), from.clone()));
                }
            })
            .build()?;
        let m = emu.load_library(format!("tests/fixtures/{abi}/libjnidispatch.so"))?;
        let app = emu.find_symbol(m, "JNI_OnLoad")? & !1;
        let first = emu.find_symbol(m, "strlen")? & !1;
        let second = emu.find_symbol(m, "strcmp")? & !1;
        let protect = emu.find_symbol(m, "mprotect")?;
        for address in [app, first, second] {
            assert_eq!(emu.call_function(protect, &[address & !4095, 4096, 7])?, 0);
        }
        let (tail, leaf, plt): (u32, [u32; 2], u32) = if abi == "arm64-v8a" {
            (0xd61f0060, [0x528000e0, 0xd65f03c0], 0xd61f0040) // br x3; return 7; br x2
        } else {
            (0xe12fff13, [0xe3a00007, 0xe12fff1e], 0xe12fff12) // bx r3; return 7; bx r2
        };
        // Patch guest memory only: strlen tail-calls strcmp without changing lr.
        emu.write_bytes(first, &tail.to_le_bytes())?;
        emu.write_bytes(
            second,
            &leaf
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        emu.write_bytes(app + 0x40, &plt.to_le_bytes())?;
        emu.set_trace(Trace {
            imports: true,
            ..Trace::default()
        })?;
        for via_plt in [false, true] {
            let words = if abi == "arm64-v8a" {
                vec![
                    0xa9bf7bfd,
                    if via_plt { 0x9400000f } else { 0xd63f0040 },
                    0xa8c17bfd,
                    0xd65f03c0,
                ]
            } else {
                vec![
                    0xe92d4010,
                    if via_plt { 0xeb00000d } else { 0xe12fff32 },
                    0xe8bd8010,
                ]
            };
            emu.write_bytes(
                app,
                &words
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
            )?;
            calls.borrow_mut().clear();
            assert_eq!(emu.call_function(app, &[0, 0, first, second])?, 7);
            assert_eq!(
                *calls.borrow(),
                [("strlen".to_owned(), emu.symbolize(app + 8))],
                "{abi}, via_plt={via_plt}"
            );
        }
    }
    Ok(())
}
