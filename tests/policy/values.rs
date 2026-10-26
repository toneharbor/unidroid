use super::*;
use serde_json::json;

#[test]
fn primitive_answers_and_valueless_void_calls_cover_every_operation() -> Result<()> {
    for lib in ["rootfs/lib64/libc.so", "rootfs/lib/libc.so"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(lib)?;
        for (kind, value, expected) in [
            ("Z", json!(true), JniValue::Boolean(true)),
            ("B", json!(-128), JniValue::Byte(-128)),
            ("C", json!(55296), JniValue::Char(55296)),
            ("S", json!(-32768), JniValue::Short(-32768)),
            ("I", json!(-2147483648i64), JniValue::Int(-2147483648)),
            (
                "J",
                json!("-9007199254740993"),
                JniValue::Long(-9007199254740993),
            ),
            ("F", json!(1.5), JniValue::Float(1.5)),
            ("D", json!(-2.25), JniValue::Double(-2.25)),
        ] {
            let mut policy =
                Policy::parse(&json!({"rules":[{"op":"set"},{"value":value}]}).to_string())?;
            let mut fallback = ();
            let mut handler = policy.handler(&mut fallback);
            for op in [JniOp::Call, JniOp::Get, JniOp::Set] {
                let signature = if op == JniOp::Call {
                    format!("(){kind}")
                } else {
                    kind.into()
                };
                for is_static in [false, true] {
                    let expected = if op == JniOp::Set {
                        JniValue::Void
                    } else {
                        expected
                    };
                    let request = JniCall {
                        op,
                        is_static,
                        type_code: expected.type_code(),
                        ..call(&signature)
                    };
                    assert_eq!(handler.member(&mut emu, &request), Some(expected));
                }
            }
            handler.finish(Ok(()))?;
        }
        let mut policy = Policy::parse(r#"{"rules":[{}]}"#)?;
        let mut fallback = ();
        let mut handler = policy.handler(&mut fallback);
        for (op, signature) in [(JniOp::Call, "()V"), (JniOp::Set, "Ljava/lang/Object;")] {
            for is_static in [false, true] {
                assert_eq!(
                    handler.member(
                        &mut emu,
                        &JniCall {
                            op,
                            is_static,
                            type_code: b'V',
                            ..call(signature)
                        }
                    ),
                    Some(JniValue::Void)
                );
            }
        }
        handler.finish(Ok(()))?;
        assert_eq!(handler.member(&mut emu, &call("()I")), None);
        let error = handler.finish(Ok(())).unwrap_err().to_string();
        assert!(
            error.contains("rule 0 for call test/Answers->value()I"),
            "{error}"
        );
        assert!(error.contains("needs a value"), "{error}");
    }
    Ok(())
}

#[test]
fn object_answers_create_null_strings_objects_and_primitive_arrays() -> Result<()> {
    for lib in ["rootfs/lib64/libc.so", "rootfs/lib/libc.so"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(lib)?;
        let mut policy =
            Policy::parse(r#"{"rules":[{"values":[null,"héllo",{"object":"test/Thing"}]}]}"#)?;
        let mut fallback = ();
        let mut handler = policy.handler(&mut fallback);
        let c = call("()Ljava/lang/Object;");
        assert_eq!(handler.member(&mut emu, &c), Some(Handle(0).into()));
        let string = handler.member(&mut emu, &c).unwrap().object();
        assert_eq!(emu.string(string)?, "héllo");
        let object = handler.member(&mut emu, &c).unwrap().object();
        assert!(emu.user_value::<()>(object).is_some());
        assert!(
            emu.string(object)
                .unwrap_err()
                .to_string()
                .contains("test/Thing")
        );
        handler.finish(Ok(()))?;
        for kind in ["Z", "B", "C", "S", "I", "J", "F", "D"] {
            let mut policy = Policy::parse(
                &json!({"rules":[{"value":{"array":kind,"base64":"AQIDBAUGBwg="}}]}).to_string(),
            )?;
            let mut handler = policy.handler(&mut fallback);
            let array = handler.member(&mut emu, &c).unwrap().object();
            assert_eq!(emu.primitive_array(array)?, [1, 2, 3, 4, 5, 6, 7, 8]);
            handler.finish(Ok(()))?;
        }
    }
    Ok(())
}
