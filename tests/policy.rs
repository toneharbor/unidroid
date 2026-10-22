use unidroid::{Emulator, Handle, JniCall, JniHandler, Policy, Probe, Result};

fn call(signature: &str) -> JniCall<'_> {
    JniCall {
        class: "test/Answers",
        name: "value",
        signature,
        receiver: Handle(0),
        args: &[],
    }
}

#[path = "policy/errors.rs"]
mod errors;
#[path = "policy/matching.rs"]
mod matching;
#[path = "policy/values.rs"]
mod values;

#[test]
fn policy_answers_onload_before_the_fallback() -> Result<()> {
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let mut emu = Emulator::builder().build()?;
        let module = emu.load_library(format!("tests/fixtures/{abi}/libjnidispatch.so"))?;
        let error = emu
            .call_jni_onload(module, &mut ())
            .unwrap_err()
            .to_string();
        assert!(error.contains("java/lang/Void->TYPE"), "{error}");
        let mut policy = Policy::parse(
            r#"{"rules":[{"op":"get", "class":"java/lang/Void", "name":"TYPE", "value":{"object":"java/lang/Class"}}]}"#,
        )?;
        let mut probe = Probe::default();
        {
            let mut handler = policy.handler(&mut probe);
            let result = emu.call_jni_onload(module, &mut handler);
            assert_eq!(handler.finish(result)?, 0);
        }
        assert!(!probe.calls().is_empty());
        assert!(probe.calls().iter().all(|c| c.class != "java/lang/Void"));
        assert!(
            probe
                .calls()
                .iter()
                .all(|c| c.from.starts_with("libjnidispatch.so+"))
        );
    }
    Ok(())
}

#[test]
fn ordered_rules_match_wildcards_and_static_then_fall_through() -> Result<()> {
    for lib in ["rootfs/lib64/libc.so", "rootfs/lib/libc.so"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(lib)?;
        let mut policy = Policy::parse(
            r#"{"rules":[
            {"op":"get", "value":99},
            {"static":true, "value":88},
            {"op":"call", "class":"test/*", "name":"v*lue", "signature":"()I", "values":[12,34]},
            {"op":"call", "class":"*Answers", "name":"value", "signature":"()I", "values":[56]}
        ]}"#,
        )?;
        let mut fallback = Probe::default();
        {
            let mut handler = policy.handler(&mut fallback);
            assert_eq!(
                handler.call_static_int_method(&mut emu, &call("()I")),
                Some(88)
            );
            assert_eq!(handler.get_int_field(&mut emu, &call("I")), Some(99));
            for value in [12, 34, 56, 0] {
                assert_eq!(handler.call_int_method(&mut emu, &call("()I")), Some(value));
            }
            handler.finish(Ok(()))?;
        }
        assert_eq!(fallback.calls().len(), 1);
        assert_eq!(fallback.calls()[0].count, 1);
    }
    Ok(())
}
