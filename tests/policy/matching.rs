use super::*;
use unidroid::{Resume, Stop};

#[test]
fn patterns_are_anchored_and_only_star_is_special() -> Result<()> {
    for lib in ["rootfs/lib64/libc.so", "rootfs/lib/libc.so"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(lib)?;
        for (pattern, name, expected) in [
            ("v**l*", "value", Some(7)),
            ("*a*a", "aba", Some(7)),
            ("a*a", "a", None),
            ("value", "values", None),
            ("value", "avalue", None),
            ("v?lue", "value", None),
            ("v?lue", "v?lue", Some(7)),
            ("[ab]", "a", None),
            ("[ab]", "[ab]", Some(7)),
            ("é*文", "é中文", Some(7)),
            ("*", "", Some(7)),
        ] {
            let text =
                serde_json::json!({"rules":[{"op":"c*","name":pattern,"value":7}]}).to_string();
            let mut policy = Policy::parse(&text)?;
            let mut fallback = ();
            let mut handler = policy.handler(&mut fallback);
            assert_eq!(
                handler.member(
                    &mut emu,
                    &JniCall {
                        name,
                        ..call("()I")
                    }
                ),
                expected.map(JniValue::Int),
                "{pattern} / {name}"
            );
            handler.finish(Ok(()))?;
        }
    }
    Ok(())
}

#[test]
fn null_class_matches_any_class() -> Result<()> {
    for lib in ["rootfs/lib64/libc.so", "rootfs/lib/libc.so"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(lib)?;
        let mut policy = Policy::parse(r#"{"rules":[{"class":null,"value":7}]}"#)?;
        let mut fallback = ();
        let mut handler = policy.handler(&mut fallback);
        for class in ["test/Answers", "other/Thing"] {
            assert_eq!(
                handler.member(
                    &mut emu,
                    &JniCall {
                        class,
                        ..call("()I")
                    }
                ),
                Some(JniValue::Int(7)),
                "{class}"
            );
        }
        handler.finish(Ok(()))?;
    }
    Ok(())
}

#[test]
fn assignability_uses_from_and_to_and_missing_fields_do_not_match() -> Result<()> {
    for lib in ["rootfs/lib64/libc.so", "rootfs/lib/libc.so"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(lib)?;
        let mut policy = Policy::parse(
            r#"{"rules":[
            {"op":"is_assignable", "static":false, "value":false},
            {"op":"is_assignable", "class":"*", "value":false},
            {"op":"is_assignable", "from":"java/lang/*", "to":"java/lang/Object", "values":[true,false]}
        ]}"#,
        )?;
        let mut probe = Probe::default();
        {
            let mut handler = policy.handler(&mut probe);
            assert_eq!(
                handler.is_assignable(&mut emu, "java/lang/String", "java/lang/Object"),
                Some(true)
            );
            assert_eq!(
                handler.is_assignable(&mut emu, "other/A", "java/lang/Object"),
                Some(false)
            );
            handler.finish(Ok(()))?;
        }
        {
            let mut handler = policy.handler(&mut probe);
            assert_eq!(
                handler.is_assignable(&mut emu, "java/lang/String", "java/lang/Object"),
                Some(false)
            );
            assert_eq!(
                handler.is_assignable(&mut emu, "java/lang/String", "java/lang/Object"),
                Some(false)
            );
            handler.finish(Ok(()))?;
        }
        assert_eq!(probe.calls().len(), 2);
        assert!(probe.calls().iter().all(|c| c.count == 1));
    }
    Ok(())
}

#[test]
fn empty_sequences_fall_through_and_constructors_use_object_answers() -> Result<()> {
    for lib in ["rootfs/lib64/libc.so", "rootfs/lib/libc.so"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(lib)?;
        let mut policy = Policy::parse(
            r#"{"rules":[
            {"values":[]},
            {"op":"new", "signature":"()V", "value":{"object":"test/Created"}}
        ]}"#,
        )?;
        let mut fallback = ();
        let mut handler = policy.handler(&mut fallback);
        let object = handler
            .member(
                &mut emu,
                &JniCall {
                    op: JniOp::New,
                    type_code: b'L',
                    ..call("()V")
                },
            )
            .unwrap()
            .object();
        assert!(
            emu.string(object)
                .unwrap_err()
                .to_string()
                .contains("test/Created")
        );
        assert_eq!(
            handler.member(&mut emu, &call("()Ljava/lang/Object;")),
            None
        );
        handler.finish(Ok(()))?;
    }
    Ok(())
}

#[test]
fn debugger_stops_reach_the_fallback() -> Result<()> {
    struct Fallback(usize);
    impl JniHandler for Fallback {
        fn stopped(&mut self, _emu: &mut Emulator, _stop: &Stop) -> Resume {
            self.0 += 1;
            Resume::Return(123)
        }
    }
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let mut emu = Emulator::builder().build()?;
        let module = emu.load_library(format!("tests/fixtures/{abi}/libjnidispatch.so"))?;
        let entry = emu.find_symbol(module, "JNI_OnLoad")?;
        emu.add_breakpoint(entry)?;
        let mut policy = Policy::parse(r#"{"rules":[]}"#)?;
        let mut fallback = Fallback(0);
        {
            let mut handler = policy.handler(&mut fallback);
            let result = emu.call_jni_onload(module, &mut handler);
            assert_eq!(handler.finish(result)?, 123);
        }
        assert_eq!(fallback.0, 1);
    }
    Ok(())
}
