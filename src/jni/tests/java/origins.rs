use super::*;

// Call a JNI function and restore lr before returning, so the final lr differs
// from the address where the exception was raised (code + 12 on both ABIs).
fn raising_call(
    emu: &mut Emulator,
    code: u64,
    index: u64,
    args: &[u64],
    handler: &mut dyn JniHandler,
) -> Result<u64> {
    let table = emu.read_pointer(emu.jni.env)?;
    let function = emu.read_pointer(table + index * emu.pointer_size())?;
    let words = match emu.architecture() {
        Architecture::Arm32 => vec![
            0xe92d4010,
            0xe59fc004,
            0xe12fff3c,
            0xe8bd8010,
            function as u32,
        ],
        Architecture::Arm64 => vec![
            0xa9bf7bfd,
            0x580000b0,
            0xd63f0200,
            0xa8c17bfd,
            0xd65f03c0,
            0xd503201f,
            function as u32,
            (function >> 32) as u32,
        ],
    };
    emu.write_bytes(
        code,
        &words
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>(),
    )?;
    let mut words = vec![emu.jni.env];
    words.extend_from_slice(args);
    emu.call_function_with(handler, code, &words)
}

#[test]
fn pending_exceptions_keep_the_raising_call_location_through_inspection_and_rewind() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let class = emu.class("test/Exception");
        let message = emu.new_cstring("raised here")?;
        let code = crate::RETURN + 0x100;
        assert_eq!(
            raising_call(&mut emu, code, 14, &[class.0, message], &mut ())?,
            0
        );
        let exception = Handle(jni_call(&mut emu, 15, &[])?);
        assert_eq!(jni_call(&mut emu, 228, &[])?, 1);
        let saved = emu.checkpoint()?;
        let mut fresh = Emulator::builder().build()?;
        for target in [&mut emu, &mut fresh] {
            target.rewind(&saved)?;
            let error = target
                .invoke_jni(crate::RETURN, &[], b'V', &mut ())
                .unwrap_err()
                .to_string();
            assert_eq!(
                error,
                format!(
                    "java exception thrown by native code: test/Exception: raised here from {}",
                    target.location(code + 12)
                )
            );
            assert_eq!(jni_call(target, 228, &[])?, 0);
        }
        // Rethrow the same object at another site; the new pending exception owns that origin.
        let code = code + 0x80;
        assert_eq!(
            raising_call(&mut emu, code, 13, &[exception.0], &mut ())?,
            0
        );
        jni_call(&mut emu, 15, &[])?;
        let error = emu
            .invoke_jni(crate::RETURN, &[], b'V', &mut ())
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            format!(
                "java exception thrown by native code: test/Exception: raised here from {}",
                emu.location(code + 12)
            )
        );
        raising_call(&mut emu, code, 14, &[class.0, message], &mut ())?;
        jni_call(&mut emu, 17, &[])?;
        emu.invoke_jni(crate::RETURN, &[], b'V', &mut ())?;
    }
    Ok(())
}

#[test]
fn builtin_and_handler_exceptions_remember_their_jni_origin() -> Result<()> {
    struct Raises;
    impl JniHandler for Raises {
        fn member(&mut self, emu: &mut Emulator, call: &JniCall<'_>) -> Option<JniValue> {
            match (call.op, call.is_static, call.type_code) {
                (JniOp::Call, true, b'I') => {
                    // Nested guest calls restore the outer JNI call before the handler raises.
                    assert_eq!(jni_call(emu, 4, &[]).unwrap(), 0x10006);
                    emu.pending_exception("test/HandlerException", None);
                    Some(JniValue::Int(0))
                }
                _ => None,
            }
        }
    }
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let slots = emu.malloc(8)?;
        let bad = emu.new_string("x");
        emu.write_u64(slots, bad.0)?;
        for (i, class, signature, expected) in [
            (
                0,
                "java/lang/Integer",
                "(Ljava/lang/String;)I",
                "java/lang/NumberFormatException: For input string: \"x\"",
            ),
            (1, "test/Handler", "()I", "test/HandlerException"),
            (2, "test/PolicyFallback", "()I", "test/HandlerException"),
        ] {
            let class = emu.class(class);
            let name = emu.new_cstring(if i == 0 { "parseInt" } else { "raise" })?;
            let signature = emu.new_cstring(signature)?;
            let id = jni_call(&mut emu, 113, &[class.0, name, signature])?;
            let code = crate::RETURN + 0x200 + i * 0x80;
            let mut fallback = Raises;
            let mut policy = crate::Policy::parse(r#"{"rules":[]}"#)?;
            let mut handler = policy.handler(&mut fallback);
            assert_eq!(
                raising_call(&mut emu, code, 131, &[class.0, id, slots], &mut handler)?,
                0
            );
            handler.finish(Ok(()))?;
            jni_call(&mut emu, 228, &[])?;
            let error = emu
                .invoke_jni(crate::RETURN, &[], b'V', &mut ())
                .unwrap_err()
                .to_string();
            assert_eq!(
                error,
                format!(
                    "java exception thrown by native code: {expected} from {}",
                    emu.location(code + 12)
                )
            );
        }
    }
    Ok(())
}
