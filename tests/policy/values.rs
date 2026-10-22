use super::*;
use serde_json::json;

#[test]
fn primitive_answers_and_valueless_void_calls_cover_every_handler_method() -> Result<()> {
    macro_rules! check {
        ($emu:ident, $value:expr, $expected:expr, $kind:literal, $call:ident, $static_call:ident, $get:ident, $static_get:ident, $set:ident, $static_set:ident) => {{
            let mut policy = Policy::parse(&json!({"rules":[{"op":"set"},{"value":$value}]}).to_string())?;
            let mut fallback = ();
            let mut handler = policy.handler(&mut fallback);
            assert_eq!(handler.$call(&mut $emu, &call(concat!("()", $kind))), Some($expected));
            assert_eq!(handler.$static_call(&mut $emu, &call(concat!("()", $kind))), Some($expected));
            assert_eq!(handler.$get(&mut $emu, &call($kind)), Some($expected));
            assert_eq!(handler.$static_get(&mut $emu, &call($kind)), Some($expected));
            assert_eq!(handler.$set(&mut $emu, &call($kind)), Some(()));
            assert_eq!(handler.$static_set(&mut $emu, &call($kind)), Some(()));
            handler.finish(Ok(()))?;
        }};
    }
    for lib in ["rootfs/lib64/libc.so", "rootfs/lib/libc.so"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(lib)?;
        check!(
            emu,
            true,
            true,
            "Z",
            call_boolean_method,
            call_static_boolean_method,
            get_boolean_field,
            get_static_boolean_field,
            set_boolean_field,
            set_static_boolean_field
        );
        check!(
            emu,
            -128,
            -128,
            "B",
            call_byte_method,
            call_static_byte_method,
            get_byte_field,
            get_static_byte_field,
            set_byte_field,
            set_static_byte_field
        );
        check!(
            emu,
            55296,
            55296,
            "C",
            call_char_method,
            call_static_char_method,
            get_char_field,
            get_static_char_field,
            set_char_field,
            set_static_char_field
        );
        check!(
            emu,
            -32768,
            -32768,
            "S",
            call_short_method,
            call_static_short_method,
            get_short_field,
            get_static_short_field,
            set_short_field,
            set_static_short_field
        );
        check!(
            emu,
            -2147483648i64,
            -2147483648,
            "I",
            call_int_method,
            call_static_int_method,
            get_int_field,
            get_static_int_field,
            set_int_field,
            set_static_int_field
        );
        check!(
            emu,
            "-9007199254740993",
            -9007199254740993,
            "J",
            call_long_method,
            call_static_long_method,
            get_long_field,
            get_static_long_field,
            set_long_field,
            set_static_long_field
        );
        check!(
            emu,
            1.5,
            1.5,
            "F",
            call_float_method,
            call_static_float_method,
            get_float_field,
            get_static_float_field,
            set_float_field,
            set_static_float_field
        );
        check!(
            emu,
            -2.25,
            -2.25,
            "D",
            call_double_method,
            call_static_double_method,
            get_double_field,
            get_static_double_field,
            set_double_field,
            set_static_double_field
        );
        let mut policy = Policy::parse(r#"{"rules":[{}]}"#)?;
        let mut fallback = ();
        let mut handler = policy.handler(&mut fallback);
        assert_eq!(handler.call_void_method(&mut emu, &call("()V")), Some(()));
        assert_eq!(
            handler.call_static_void_method(&mut emu, &call("()V")),
            Some(())
        );
        assert_eq!(
            handler.set_object_field(&mut emu, &call("Ljava/lang/Object;")),
            Some(())
        );
        assert_eq!(
            handler.set_static_object_field(&mut emu, &call("Ljava/lang/Object;")),
            Some(())
        );
        handler.finish(Ok(()))?;
        assert_eq!(handler.call_int_method(&mut emu, &call("()I")), None);
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
        assert_eq!(handler.call_object_method(&mut emu, &c), Some(Handle(0)));
        let string = handler.call_object_method(&mut emu, &c).unwrap();
        assert_eq!(emu.string(string)?, "héllo");
        let object = handler.call_object_method(&mut emu, &c).unwrap();
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
            let array = handler.call_object_method(&mut emu, &c).unwrap();
            assert_eq!(emu.primitive_array(array)?, [1, 2, 3, 4, 5, 6, 7, 8]);
            handler.finish(Ok(()))?;
        }
    }
    Ok(())
}
