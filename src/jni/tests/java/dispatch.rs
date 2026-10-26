use super::*;

#[test]
fn invalid_receivers_fail_before_builtin_and_handler_dispatch() -> Result<()> {
    #[derive(Default)]
    struct Handler(Vec<Handle>);
    impl JniHandler for Handler {
        fn member(&mut self, _: &mut Emulator, call: &JniCall<'_>) -> Option<JniValue> {
            match (call.op, call.is_static, call.type_code) {
                (JniOp::Call, false, b'I') => {
                    self.0.push(call.receiver);
                    Some(JniValue::Int(7))
                }
                _ => None,
            }
        }
    }
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let released = emu.new_string("released");
        emu.release(released);
        let mut handler = Handler::default();
        let mut probe = crate::Probe::default();
        for receiver in [Handle(0), released, Handle(999_999)] {
            let expected = format!(
                "invalid JNI reference 0x{:x} in CallIntMethodA from 0x20000",
                receiver.0
            );
            let error = member(
                &mut emu,
                &mut handler,
                receiver,
                "java/lang/String",
                "length()I",
                &[],
                0,
            )
            .unwrap_err();
            assert_eq!(error.to_string(), expected);
            let error = member(
                &mut emu,
                &mut probe,
                receiver,
                "java/lang/String",
                "length()I",
                &[],
                0,
            )
            .unwrap_err();
            assert_eq!(error.to_string(), expected);
        }
        assert!(handler.0.is_empty());
        assert!(probe.calls().is_empty());
    }
    Ok(())
}

#[test]
fn builtins_precede_policy_and_probe_without_consuming_answers() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let string = emu.new_string("abc");
        let opaque = emu.new_object("java/lang/String", ());
        let mut policy = crate::Policy::parse(r#"{"rules":[{"name":"length","values":[99]}]}"#)?;
        let mut probe = crate::Probe::default();
        {
            let mut handler = policy.handler(&mut probe);
            assert_eq!(
                member(
                    &mut emu,
                    &mut handler,
                    string,
                    "java/lang/CharSequence",
                    "length()I",
                    &[],
                    0
                )?,
                JniValue::Int(3)
            );
            assert_eq!(
                member(
                    &mut emu,
                    &mut handler,
                    opaque,
                    "java/lang/String",
                    "length()I",
                    &[],
                    0
                )?,
                JniValue::Int(99)
            );
            handler.finish(Ok(()))?;
        }
        assert!(probe.calls().is_empty());
        assert_eq!(
            member(
                &mut emu,
                &mut probe,
                string,
                "java/lang/String",
                "length()I",
                &[],
                0
            )?,
            JniValue::Int(3)
        );
        assert!(probe.calls().is_empty());
        member(
            &mut emu,
            &mut probe,
            string,
            "java/lang/String",
            "uncovered()I",
            &[],
            0,
        )?;
        assert_eq!(probe.calls().len(), 1);
    }
    Ok(())
}

#[test]
fn builtin_assignability_uses_declared_chains_and_leaves_app_types_to_handlers() -> Result<()> {
    struct Handler;
    impl JniHandler for Handler {
        fn is_assignable(&mut self, _: &mut Emulator, from: &str, to: &str) -> Option<bool> {
            assert_eq!(from, "java/lang/String");
            assert_eq!(to, "app/Marker");
            Some(true)
        }
    }
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        emu.superclass("app/Map", "java/util/HashMap");
        for (from, to, expected) in [
            ("java/util/HashMap", "java/util/Map", 1),
            ("app/Map", "java/util/Map", 1),
            ("java/util/ArrayList", "java/lang/Iterable", 1),
            ("java/util/LinkedList", "java/util/Deque", 1),
            ("java/lang/String", "java/lang/CharSequence", 1),
            ("java/lang/Integer", "java/lang/Number", 1),
            ("java/util/HashMap", "java/util/List", 0),
            ("java/lang/String", "javax/example/Unknown", 0),
        ] {
            let from = emu.class(from);
            let to = emu.class(to);
            assert_eq!(jni_call(&mut emu, 11, &[from.0, to.0])?, expected);
        }
        let string = emu.new_string("s");
        let to = emu.class("java/lang/CharSequence");
        assert_eq!(jni_call(&mut emu, 32, &[string.0, to.0])?, 1);
        let from = emu.class("java/lang/String");
        let to = emu.class("app/Marker");
        let table = emu.read_pointer(emu.jni.env)?;
        let function = emu.read_pointer(table + 11 * emu.pointer_size())?;
        assert_eq!(
            emu.call_function_with(&mut Handler, function, &[emu.jni.env, from.0, to.0])?,
            1
        );
    }
    Ok(())
}

#[test]
fn opaque_objects_and_uncovered_builtins_reach_the_handler() -> Result<()> {
    #[derive(Default)]
    struct Handler(usize);
    impl JniHandler for Handler {
        fn member(&mut self, emu: &mut Emulator, call: &JniCall<'_>) -> Option<JniValue> {
            match (call.op, call.is_static, call.type_code) {
                (JniOp::Call, false, b'Z') => {
                    self.0 += 1;
                    Some(JniValue::Boolean(true))
                }
                (JniOp::Call, false, b'I') => {
                    self.0 += 1;
                    Some(JniValue::Int(99))
                }
                (JniOp::Call, false, b'L')
                | (JniOp::Call, true, b'L')
                | (JniOp::New, false, b'L') => {
                    self.0 += 1;
                    Some(JniValue::Object(emu.new_string("fallback")))
                }
                _ => None,
            }
        }
    }
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let mut handler = Handler::default();
        let opaque = emu.new_object("java/util/HashMap", ());
        assert_eq!(
            member(
                &mut emu,
                &mut handler,
                opaque,
                "java/util/Map",
                "size()I",
                &[],
                0
            )?,
            JniValue::Int(99)
        );
        let map = emu.new_java_map("java/util/HashMap", &[])?;
        assert_eq!(
            member(
                &mut emu,
                &mut handler,
                map,
                "java/util/Map",
                "size()I",
                &[],
                0
            )?,
            JniValue::Int(0)
        );
        assert_eq!(
            member(
                &mut emu,
                &mut handler,
                map,
                "java/util/Map",
                "uncovered()I",
                &[],
                0
            )?,
            JniValue::Int(99)
        );
        let string = emu.new_string("s");
        let charset = emu.new_string("UTF-32");
        let bytes = emu.new_byte_array(b"s");
        member(
            &mut emu,
            &mut handler,
            string,
            "java/lang/String",
            "getBytes(Ljava/lang/String;)[B",
            &[charset.into()],
            0,
        )?;
        member(
            &mut emu,
            &mut handler,
            Handle(0),
            "java/lang/String",
            "<init>([BLjava/lang/String;)V",
            &[bytes.into(), charset.into()],
            6,
        )?;
        let unknown = emu.new_string("SHA-3");
        member(
            &mut emu,
            &mut handler,
            Handle(0),
            "java/security/MessageDigest",
            "getInstance(Ljava/lang/String;)Ljava/security/MessageDigest;",
            &[unknown.into()],
            1,
        )?;
        let tree = emu.new_java_map("java/util/TreeMap", &[])?;
        member(
            &mut emu,
            &mut handler,
            tree,
            "java/util/Map",
            "put(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
            &[opaque.into(), string.into()],
            0,
        )?;
        assert!(emu.java_map(tree)?.is_empty());
        let opaque_set = emu.new_object("java/util/HashSet", ());
        assert_eq!(
            member(
                &mut emu,
                &mut handler,
                opaque_set,
                "java/util/Set",
                "add(Ljava/lang/Object;)Z",
                &[Handle(999_999).into()],
                0
            )?,
            JniValue::Boolean(true)
        );
        let view = call(&mut emu, map, "keySet()Ljava/util/Set;", &[])?.object();
        assert_eq!(
            member(
                &mut emu,
                &mut handler,
                view,
                "java/util/Set",
                "add(Ljava/lang/Object;)Z",
                &[Handle(999_999).into()],
                0
            )?,
            JniValue::Boolean(true)
        );
        assert_eq!(handler.0, 8);
    }
    Ok(())
}
