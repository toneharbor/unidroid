use std::{cell::RefCell, rc::Rc};
use unidroid::{Emulator, Handle, JniCall, JniHandler, Result, Trace};

struct Answers;
impl JniHandler for Answers {
    fn get_static_object_field(
        &mut self,
        emu: &mut Emulator,
        call: &JniCall<'_>,
    ) -> Option<Handle> {
        Some(emu.class(call.class))
    }
    fn call_static_object_method(&mut self, emu: &mut Emulator, _: &JniCall<'_>) -> Option<Handle> {
        Some(emu.new_string("UTF-8"))
    }
}

#[test]
fn import_strings_are_capped_and_unreadable_arguments_stay_hex() -> Result<()> {
    for (abi, words) in [
        ("arm64-v8a", [0xaa0303feu32, 0xd61f0040]),
        ("armeabi-v7a", [0xe1a0e003, 0xe12fff12]),
    ] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .trace(Trace {
                imports: true,
                ..Trace::default()
            })
            .on_event(move |event| sink.borrow_mut().push(serde_json::to_value(event).unwrap()))
            .build()?;
        let path = format!("tests/fixtures/{abi}/libjnidispatch.so");
        let m = emu.load_library(&path)?;
        let bytes = std::fs::read(path)?;
        let elf = goblin::elf::Elf::parse(&bytes)?;
        let onload = elf
            .dynsyms
            .iter()
            .find(|s| elf.dynstrtab.get_at(s.st_name) == Some("JNI_OnLoad"))
            .unwrap();
        let code = emu.find_symbol(m, "JNI_OnLoad")? & !1;
        let base = code - (onload.st_value & !1);
        let data = elf
            .program_headers
            .iter()
            .find(|s| s.p_type == 1 && s.p_flags & 1 == 0)
            .unwrap()
            .p_vaddr
            + base;
        // Enter strlen from app code with lr in app data, so returning faults.
        let protect = emu.find_symbol(m, "mprotect")?;
        assert_eq!(emu.call_function(protect, &[code & !4095, 4096, 7])?, 0);
        emu.write_bytes(
            code,
            &words
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        let strlen = emu.find_symbol(m, "strlen")?;
        let long = emu.new_cstring(&"a".repeat(300))?;
        for (pointer, expected) in [
            (long, format!("\"{}\"", "a".repeat(256))),
            (1, "0x1".into()),
        ] {
            events.borrow_mut().clear();
            assert!(
                emu.call_function(code, &[pointer, 0, strlen, data])
                    .is_err()
            );
            let captured = events.borrow();
            let call = captured.iter().find(|v| v["function"] == "strlen").unwrap();
            assert_eq!(call["args"][0], expected);
            assert_eq!(call["from"], emu.symbolize(data));
        }
    }
    Ok(())
}

#[test]
fn imports_trace_app_calls_and_can_be_removed_from_translated_code() -> Result<()> {
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .on_event(move |event| sink.borrow_mut().push(serde_json::to_value(event).unwrap()))
            .build()?;
        let m = emu.load_library(format!("tests/fixtures/{abi}/libjnidispatch.so"))?;
        for enabled in [true, false, true] {
            events.borrow_mut().clear();
            emu.set_trace(Trace {
                imports: enabled,
                ..Trace::default()
            })?;
            let _ = emu.call_jni_onload(m, &mut Answers);
            let captured = events.borrow();
            if enabled {
                let call = captured
                    .iter()
                    .find(|v| v["event"] == "call" && v["module"] == "libc.so")
                    .expect("app import into libc");
                assert!(
                    call["from"]
                        .as_str()
                        .unwrap()
                        .starts_with("libjnidispatch.so+")
                );
                assert_eq!(call["args"].as_array().unwrap().len(), 4);
                assert!(
                    captured
                        .iter()
                        .all(|v| !v["function"].as_str().unwrap().starts_with("JNIEnv"))
                );
            } else {
                assert!(captured.is_empty());
            }
        }
    }
    Ok(())
}
