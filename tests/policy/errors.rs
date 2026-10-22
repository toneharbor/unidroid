use super::*;
use serde_json::json;

#[test]
fn rule_errors_name_the_zero_based_rule_index() {
    for rule in [
        r#"{"op":"invoke"}"#,
        r#"{"clas":"test/A"}"#,
        r#"{"value":null,"values":[]}"#,
        r#"{"static":"true"}"#,
        r#"{"value":}"#,
    ] {
        let error = Policy::parse(&format!(r#"{{"rules":[{{}},{rule}]}}"#))
            .unwrap_err()
            .to_string();
        assert!(error.contains("rule 1"), "{error}");
    }
}

#[test]
fn errors_outside_a_rule_have_no_rule_index() {
    for text in [
        "",
        "{",
        "{}",
        r#"{"rules": null}"#,
        r#"{"rules": ["#,
        r#"{"rules": []} x"#,
        r#"{"policy rule ":null}"#,
    ] {
        let error = Policy::parse(text).unwrap_err().to_string();
        assert!(error.starts_with("policy: "), "{error}");
        assert!(!error.contains("rule 0"), "{error}");
    }
}

#[test]
fn wrong_primitive_and_object_values_report_the_rule_index() -> Result<()> {
    macro_rules! bad {
        ($emu:ident, $value:expr, $method:ident, $signature:literal) => {{
            let mut policy = Policy::parse(&json!({"rules":[{"op":"set"},{"value":$value}]}).to_string())?;
            let mut probe = Probe::default();
            {
                let mut handler = policy.handler(&mut probe);
                assert_eq!(handler.$method(&mut $emu, &call($signature)), None);
                let error = handler.finish(Ok(())).unwrap_err().to_string();
                assert!(error.contains(concat!("rule 1 for call test/Answers->value", $signature)), "{error}");
            }
            assert!(probe.calls().is_empty());
        }};
    }
    for lib in ["rootfs/lib64/libc.so", "rootfs/lib/libc.so"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(lib)?;
        bad!(emu, 1, call_boolean_method, "()Z");
        bad!(emu, 128, call_byte_method, "()B");
        bad!(emu, -1, call_char_method, "()C");
        bad!(emu, 32768, call_short_method, "()S");
        bad!(emu, 2147483648i64, call_int_method, "()I");
        bad!(emu, 1.5, call_int_method, "()I");
        bad!(emu, 9007199254740993i64, call_long_method, "()J");
        bad!(emu, "9223372036854775808", call_long_method, "()J");
        bad!(emu, 1e40, call_float_method, "()F");
        bad!(emu, "1.5", call_double_method, "()D");
        for value in [
            json!(5),
            json!(true),
            json!([]),
            json!({"object":42}),
            json!({"object":"test/A","extra":0}),
            json!({"array":"B"}),
            json!({"array":"B","base64":"!"}),
        ] {
            bad!(emu, value, call_object_method, "()Ljava/lang/Object;");
        }
    }
    Ok(())
}

#[test]
fn bad_answers_stop_without_falling_back_and_name_the_call_and_rule() -> Result<()> {
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        for rule in [
            json!({}),
            json!({"value":42}),
            json!({"value":{"array":"X","base64":""}}),
        ] {
            let mut emu = Emulator::builder().build()?;
            let module = emu.load_library(format!("tests/fixtures/{abi}/libjnidispatch.so"))?;
            let mut policy = Policy::parse(&json!({"rules":[{"op":"call"},rule]}).to_string())?;
            let mut probe = Probe::default();
            {
                let mut handler = policy.handler(&mut probe);
                let result = emu.call_jni_onload(module, &mut handler);
                let error = handler.finish(result).unwrap_err().to_string();
                for text in [
                    "rule 1",
                    "static get java/lang/Void->TYPELjava/lang/Class;",
                    "from libjnidispatch.so+",
                ] {
                    assert!(error.contains(text), "{error}");
                }
            }
            assert!(probe.calls().is_empty());
        }
    }
    Ok(())
}
