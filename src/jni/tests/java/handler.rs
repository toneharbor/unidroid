use super::*;

#[test]
fn text_fallback_rejects_wrong_handler_answer_types() -> Result<()> {
    struct Wrong;
    impl JniHandler for Wrong {
        fn member(&mut self, _: &mut Emulator, call: &JniCall<'_>) -> Option<JniValue> {
            assert_eq!(
                (call.op, call.is_static, call.type_code),
                (JniOp::Call, false, b'L')
            );
            assert_eq!(
                (call.name, call.signature),
                ("toString", "()Ljava/lang/String;")
            );
            Some(JniValue::Int(7))
        }
    }
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let object = emu.new_object("app/Text", ());
        let builder = construct(&mut emu, "java/lang/StringBuilder", "()V", &[])?;
        for (class, method, receiver, op, function) in [
            (
                "java/lang/String",
                "valueOf(Ljava/lang/Object;)Ljava/lang/String;",
                Handle(0),
                1,
                "CallStaticObjectMethodA",
            ),
            (
                "java/lang/StringBuilder",
                "append(Ljava/lang/Object;)Ljava/lang/StringBuilder;",
                builder,
                0,
                "CallObjectMethodA",
            ),
        ] {
            let error = member(
                &mut emu,
                &mut Wrong,
                receiver,
                class,
                method,
                &[object.into()],
                op,
            )
            .unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "wrong JNI answer type: {function} app/Text->toString()Ljava/lang/String; expected L, got I from 0x20000"
                )
            );
        }
    }
    Ok(())
}

#[test]
fn wrong_handler_answer_types_name_the_member_and_guest_location() -> Result<()> {
    struct Wrong;
    impl JniHandler for Wrong {
        fn member(&mut self, _: &mut Emulator, call: &JniCall<'_>) -> Option<JniValue> {
            assert_eq!(call.op, JniOp::Call);
            assert!(!call.is_static);
            assert_eq!(call.type_code, b'I');
            Some(JniValue::Long(7))
        }
    }
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let receiver = emu.new_object("app/Answer", ());
        let error = member(
            &mut emu,
            &mut Wrong,
            receiver,
            "app/Answer",
            "value()I",
            &[],
            0,
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "wrong JNI answer type: CallIntMethodA app/Answer->value()I expected I, got J from 0x20000"
        );
    }
    Ok(())
}
