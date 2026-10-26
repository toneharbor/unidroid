use super::*;

#[test]
fn string_builder_append_object_uses_the_handlers_to_string() -> Result<()> {
    struct Text;
    impl JniHandler for Text {
        fn member(&mut self, emu: &mut Emulator, call: &JniCall<'_>) -> Option<JniValue> {
            assert_eq!(
                (call.op, call.is_static, call.type_code),
                (JniOp::Call, false, b'L')
            );
            assert_eq!(
                (call.class, call.name, call.signature),
                ("app/Text", "toString", "()Ljava/lang/String;")
            );
            Some(emu.new_string("handler text").into())
        }
    }
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let object = emu.new_object("app/Text", ());
        let builder = construct(&mut emu, "java/lang/StringBuilder", "()V", &[])?;
        let result = member(
            &mut emu,
            &mut Text,
            builder,
            "java/lang/StringBuilder",
            "append(Ljava/lang/Object;)Ljava/lang/StringBuilder;",
            &[object.into()],
            0,
        )?
        .object();
        assert_eq!(jni_call(&mut emu, 24, &[builder.0, result.0])?, 1);
        assert_eq!(text(&mut emu, builder)?, "handler text");
    }
    Ok(())
}

#[test]
fn to_string_exceptions_leave_builders_unchanged_and_value_of_null() -> Result<()> {
    struct Raises {
        exception: Handle,
        answer: Handle,
    }
    impl JniHandler for Raises {
        fn member(&mut self, emu: &mut Emulator, call: &JniCall<'_>) -> Option<JniValue> {
            assert_eq!(
                (
                    call.op,
                    call.is_static,
                    call.class,
                    call.name,
                    call.signature
                ),
                (
                    JniOp::Call,
                    false,
                    "app/Text",
                    "toString",
                    "()Ljava/lang/String;"
                )
            );
            assert_eq!(jni_call(emu, 13, &[self.exception.0]).unwrap(), 0); // Throw
            Some(self.answer.into())
        }
    }
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let builder = construct(&mut emu, "java/lang/StringBuilder", "()V", &[])?;
        let object = emu.new_object("app/Text", ());
        let exception = emu.new_object("java/lang/IllegalStateException", ());
        let string = emu.new_string("ignored answer");
        let slots = emu.malloc(8)?;
        emu.write_u64(slots, object.0)?;
        let table = emu.read_pointer(emu.jni.env)?;
        for (class, name, signature, is_static) in [
            (
                "java/lang/StringBuilder",
                "append",
                "(Ljava/lang/Object;)Ljava/lang/StringBuilder;",
                false,
            ),
            (
                "java/lang/String",
                "valueOf",
                "(Ljava/lang/Object;)Ljava/lang/String;",
                true,
            ),
        ] {
            let class = emu.class(class);
            let name = emu.new_cstring(name)?;
            let signature = emu.new_cstring(signature)?;
            let id = jni_call(
                &mut emu,
                if is_static { 113 } else { 33 },
                &[class.0, name, signature],
            )?;
            let function =
                emu.read_pointer(table + (if is_static { 116 } else { 36 }) * emu.pointer_size())?;
            let receiver = if is_static { class } else { builder };
            for answer in [Handle(0), string] {
                // A raw guest call leaves the exception pending for inspection and clearing.
                let result = emu.call_function_with(
                    &mut Raises { exception, answer },
                    function,
                    &[emu.jni.env, receiver.0, id, slots],
                )?;
                assert_eq!(jni_call(&mut emu, 228, &[])?, 1); // ExceptionCheck
                let pending = jni_call(&mut emu, 15, &[])?; // ExceptionOccurred
                assert_eq!(jni_call(&mut emu, 24, &[pending, exception.0])?, 1);
                jni_call(&mut emu, 17, &[])?; // ExceptionClear
                assert_eq!(jni_call(&mut emu, 228, &[])?, 0);
                assert_eq!(text(&mut emu, builder)?, "", "{arch:?}");
                assert_eq!(result, 0, "{arch:?}: static={is_static}, answer={answer:?}");
            }
        }
    }
    Ok(())
}

#[test]
fn string_value_of_object_returns_its_to_string_result() -> Result<()> {
    struct Text(Handle);
    impl JniHandler for Text {
        fn member(&mut self, _: &mut Emulator, call: &JniCall<'_>) -> Option<JniValue> {
            match (call.op, call.is_static, call.type_code) {
                (JniOp::Call, false, b'L') => {
                    assert_eq!(
                        (call.class, call.name, call.signature),
                        ("test/Text", "toString", "()Ljava/lang/String;")
                    );
                    Some(JniValue::Object(self.0))
                }
                _ => None,
            }
        }
    }
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let string = emu.new_string("same string");
        let result = static_call(
            &mut emu,
            "java/lang/String",
            "valueOf(Ljava/lang/Object;)Ljava/lang/String;",
            &[string.into()],
        )?
        .object();
        assert_eq!(jni_call(&mut emu, 24, &[string.0, result.0])?, 1);
        emu.release(result);
        assert_eq!(emu.string(string)?, "same string");
        let null = static_call(
            &mut emu,
            "java/lang/String",
            "valueOf(Ljava/lang/Object;)Ljava/lang/String;",
            &[Handle(0).into()],
        )?
        .object();
        assert_eq!(emu.string(null)?, "null");
        let builder = construct(
            &mut emu,
            "java/lang/StringBuilder",
            "(Ljava/lang/String;)V",
            &[string.into()],
        )?;
        let result = static_call(
            &mut emu,
            "java/lang/String",
            "valueOf(Ljava/lang/Object;)Ljava/lang/String;",
            &[builder.into()],
        )?
        .object();
        assert_eq!(emu.string(result)?, text(&mut emu, builder)?);
        let object = emu.new_object("test/Text", ());
        let result = member(
            &mut emu,
            &mut Text(string),
            Handle(0),
            "java/lang/String",
            "valueOf(Ljava/lang/Object;)Ljava/lang/String;",
            &[object.into()],
            1,
        )?
        .object();
        assert_eq!(jni_call(&mut emu, 24, &[string.0, result.0])?, 1);
    }
    Ok(())
}

#[test]
fn strings_use_utf16_and_builtin_dispatch_before_the_handler() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let bytes = emu.new_byte_array("a😀é".as_bytes());
        let string = construct(&mut emu, "java/lang/String", "([B)V", &[bytes.into()])?;
        assert_eq!(call(&mut emu, string, "length()I", &[])?, JniValue::Int(4));
        assert_eq!(
            call(&mut emu, string, "charAt(I)C", &[1.into()])?,
            JniValue::Char(0xd83d)
        );
        let tail = call(
            &mut emu,
            string,
            "substring(I)Ljava/lang/String;",
            &[3.into()],
        )?
        .object();
        assert_eq!(emu.string(tail)?, "é");
        assert_eq!(text(&mut emu, string)?, "a😀é");
        let encoded = call(&mut emu, string, "getBytes()[B", &[])?.object();
        assert_eq!(emu.byte_array(encoded)?, "a😀é".as_bytes());
        assert_eq!(
            call(&mut emu, string, "hashCode()I", &[])?,
            JniValue::Int(57849829)
        );
        let other = emu.new_string("a😀é");
        assert_eq!(
            call(
                &mut emu,
                string,
                "equals(Ljava/lang/Object;)Z",
                &[other.into()]
            )?,
            JniValue::Boolean(true)
        );
    }
    Ok(())
}

#[test]
fn boxes_builders_charsets_and_class_objects_work_through_jni() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let boxed = static_call(
            &mut emu,
            "java/lang/Integer",
            "valueOf(I)Ljava/lang/Integer;",
            &[42.into()],
        )?
        .object();
        assert_eq!(
            call(&mut emu, boxed, "longValue()J", &[])?,
            JniValue::Long(42)
        );
        assert_eq!(text(&mut emu, boxed)?, "42");
        let builder = construct(&mut emu, "java/lang/StringBuilder", "()V", &[])?;
        let returned = call(
            &mut emu,
            builder,
            "append(Ljava/lang/Object;)Ljava/lang/StringBuilder;",
            &[boxed.into()],
        )?
        .object();
        assert_eq!(jni_call(&mut emu, 24, &[builder.0, returned.0])?, 1);
        call(
            &mut emu,
            builder,
            "append(C)Ljava/lang/StringBuilder;",
            &[JniValue::Char(0xd83d)],
        )?;
        call(
            &mut emu,
            builder,
            "append(C)Ljava/lang/StringBuilder;",
            &[JniValue::Char(0xde00)],
        )?;
        assert_eq!(text(&mut emu, builder)?, "42😀");
        let name = emu.new_string("utf_16le");
        let charset = static_call(
            &mut emu,
            "java/nio/charset/Charset",
            "forName(Ljava/lang/String;)Ljava/nio/charset/Charset;",
            &[name.into()],
        )?
        .object();
        let name = call(&mut emu, charset, "name()Ljava/lang/String;", &[])?.object();
        assert_eq!(emu.string(name)?, "UTF-16LE");
        let value = emu.new_string("é");
        let bytes = call(
            &mut emu,
            value,
            "getBytes(Ljava/nio/charset/Charset;)[B",
            &[charset.into()],
        )?
        .object();
        assert_eq!(emu.byte_array(bytes)?, [0xe9, 0]);
        let class = call(&mut emu, boxed, "getClass()Ljava/lang/Class;", &[])?.object();
        let name = call(&mut emu, class, "getName()Ljava/lang/String;", &[])?.object();
        assert_eq!(emu.string(name)?, "java.lang.Integer");
    }
    Ok(())
}
