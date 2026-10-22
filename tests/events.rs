use std::{cell::RefCell, rc::Rc};
use unidroid::{Emulator, Event, Result, Trace};

#[test]
fn jni_events_keep_the_existing_text_and_can_be_switched_off() -> Result<()> {
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .trace(Trace {
                jni: true,
                ..Trace::default()
            })
            .on_event(move |event| {
                sink.borrow_mut()
                    .push((event.to_string(), serde_json::to_value(event).unwrap()))
            })
            .build()?;
        let m = emu.load_library(format!("tests/fixtures/{abi}/libjnidispatch.so"))?;
        let error = emu.call_jni_onload(m, &mut ()).unwrap_err();
        let crash = error.downcast_ref::<unidroid::Crash>().unwrap();
        let captured = events.borrow();
        for function in ["FindClass", "NewGlobalRef", "GetStaticFieldID"] {
            let (text, _) = captured
                .iter()
                .rfind(|(_, value)| value["function"] == function)
                .unwrap();
            assert!(
                crash.jni.contains(text),
                "missing {function} details: {text}"
            );
        }
        assert!(captured.iter().any(|(text, value)| value["event"] == "jni"
            && text.starts_with("JNIEnv->FindClass(java/lang/Void) from libjnidispatch.so+")));
        assert!(captured.iter().any(|(text, value)| value["event"] == "jni_call" && text.starts_with("JNIEnv->GetStaticObjectField(java/lang/Void->TYPELjava/lang/Class;, []) from libjnidispatch.so+")));
        drop(captured);
        events.borrow_mut().clear();
        emu.set_trace(Trace::default())?;
        assert!(!emu.trace().jni);
        let _ = emu.call_jni_onload(m, &mut ());
        assert!(events.borrow().is_empty());
    }
    assert_eq!(
        Event::Warning {
            message: "missing".into()
        }
        .to_string(),
        "warning: missing"
    );
    Ok(())
}
