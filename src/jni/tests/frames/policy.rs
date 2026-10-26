use super::{append::append_loop, *};
use crate::Policy;
use serde_json::{Value, json};

fn object_answers() -> [Value; 5] {
    [
        json!("answer"),
        json!({"object":"test/Thing"}),
        json!({"array":"B","base64":"YQ=="}),
        json!({"map":"java/util/HashMap","entries":[["key","value"]]}),
        json!({"list":"java/util/ArrayList","items":["item"]}),
    ]
}

fn value_method(emu: &mut Emulator) -> Result<(Handle, u64)> {
    let class = emu.class("test/Answers");
    let name = emu.new_cstring("value")?;
    let signature = emu.new_cstring("(Ljava/lang/Object;)Ljava/lang/Object;")?;
    let method = jni_call(emu, 33, &[class.0, name, signature])?;
    Ok((emu.new_object("test/Answers", ()), method))
}

#[test]
fn policy_answers_are_deleted_during_guest_loop() -> Result<()> {
    for arch in ARCHES {
        for value in object_answers() {
            let mut emu = emulator(arch)?;
            let stop = append_loop(&mut emu)?;
            emu.add_breakpoint(stop)?;
            let (receiver, method) = value_method(&mut emu)?;
            let mut policy = Policy::parse(
                &json!({"rules":[{"class":"test/Answers","name":"value","value":value}]})
                    .to_string(),
            )?;
            let before = emu.jni.objects.len();
            let mut fallback = CountAtStop::default();
            let mut handler = policy.handler(&mut fallback);
            let result =
                emu.call_function_with(&mut handler, CODE, &[emu.jni.env, receiver.0, method, 0]);
            assert_eq!(handler.finish(result)?, 0);
            assert_eq!(fallback.0, Some(before), "{arch:?}: {value}");
            assert_eq!(emu.jni.objects.len(), before, "{arch:?}: {value}");
        }
    }
    Ok(())
}

#[test]
fn policy_answers_are_freed_on_return_without_delete() -> Result<()> {
    for arch in ARCHES {
        for value in object_answers() {
            let mut emu = emulator(arch)?;
            // CallObjectMethod once, then return zero without deleting its answer.
            let words: &[u32] = match arch {
                Architecture::Arm32 => &[
                    0xe92d4010, // push {r4,lr}
                    0xe5904000, // ldr r4,[r0]
                    0xe5944088, // ldr r4,[r4,#34*4]
                    0xe12fff34, // blx r4
                    0xe3a00000, // mov r0,#0
                    0xe8bd8010, // pop {r4,pc}
                ],
                Architecture::Arm64 => &[
                    0xa9bf7bfd, // stp x29,x30,[sp,#-16]!
                    0xf9400010, // ldr x16,[x0]
                    0xf9408a10, // ldr x16,[x16,#34*8]
                    0xd63f0200, // blr x16
                    0xd2800000, // mov x0,#0
                    0xa8c17bfd, // ldp x29,x30,[sp],#16
                    0xd65f03c0, // ret
                ],
            };
            write_code(&mut emu, words)?;
            emu.add_breakpoint(CODE + 4 * 4)?;
            let (receiver, method) = value_method(&mut emu)?;
            let mut policy = Policy::parse(
                &json!({"rules":[{"class":"test/Answers","name":"value","value":value}]})
                    .to_string(),
            )?;
            let before = emu.jni.objects.len();
            let mut fallback = CountAtStop::default();
            let mut handler = policy.handler(&mut fallback);
            let result =
                emu.call_function_with(&mut handler, CODE, &[emu.jni.env, receiver.0, method, 0]);
            assert_eq!(handler.finish(result)?, 0);
            assert_eq!(fallback.0, Some(before + 1), "{arch:?}: {value}");
            assert_eq!(emu.jni.objects.len(), before, "{arch:?}: {value}");
        }
    }
    Ok(())
}

#[test]
fn policy_answer_returned_to_host_needs_release() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        // Tail-call CallObjectMethod so its answer is the raw call's result.
        let words = match arch {
            Architecture::Arm32 => [
                0xe590c000, // ldr r12,[r0]
                0xe59cc088, // ldr r12,[r12,#34*4]
                0xe12fff1c, // bx r12
            ],
            Architecture::Arm64 => [
                0xf9400010, // ldr x16,[x0]
                0xf9408a10, // ldr x16,[x16,#34*8]
                0xd61f0200, // br x16
            ],
        };
        write_code(&mut emu, &words)?;
        let (receiver, method) = value_method(&mut emu)?;
        let mut policy =
            Policy::parse(r#"{"rules":[{"class":"test/Answers","name":"value","value":"kept"}]}"#)?;
        let before = emu.jni.objects.len();
        let mut fallback = ();
        let mut handler = policy.handler(&mut fallback);
        let result =
            emu.call_function_with(&mut handler, CODE, &[emu.jni.env, receiver.0, method, 0]);
        let answer = Handle(handler.finish(result)?);
        assert_eq!(emu.string(answer)?, "kept", "{arch:?}");
        assert_eq!(emu.jni.objects.len(), before + 1, "{arch:?}");
        jni_call(&mut emu, 23, &[answer.0])?;
        assert_eq!(emu.string(answer)?, "kept", "{arch:?}");
        emu.release(answer);
        assert_eq!(emu.jni.objects.len(), before, "{arch:?}");
        assert!(emu.string(answer).is_err());
    }
    Ok(())
}

#[test]
fn append_policy_to_string_does_not_grow_the_local_frame() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let stop = append_loop(&mut emu)?;
        emu.add_breakpoint(stop)?;
        let class = emu.class("java/lang/StringBuilder");
        let mut methods = Vec::new();
        for (name, signature) in [
            ("<init>", "()V"),
            ("append", "(Ljava/lang/Object;)Ljava/lang/StringBuilder;"),
            ("toString", "()Ljava/lang/String;"),
        ] {
            let name = emu.new_cstring(name)?;
            let signature = emu.new_cstring(signature)?;
            methods.push(jni_call(&mut emu, 33, &[class.0, name, signature])?);
        }
        let builder = jni_call(&mut emu, 28, &[class.0, methods[0]])?;
        let object = emu.new_object("test/Thing", ());
        let mut policy = Policy::parse(
            r#"{"rules":[{"class":"test/Thing","name":"toString","signature":"()Ljava/lang/String;","value":"x"}]}"#,
        )?;
        let before = emu.jni.objects.len();
        let mut fallback = CountAtStop::default();
        let mut handler = policy.handler(&mut fallback);
        let result = emu.call_function_with(
            &mut handler,
            CODE,
            &[emu.jni.env, builder, methods[1], object.0],
        );
        assert_eq!(handler.finish(result)?, 0);
        assert_eq!(fallback.0, Some(before), "{arch:?}");
        assert_eq!(emu.jni.objects.len(), before, "{arch:?}");
        let text = Handle(jni_call(&mut emu, 34, &[builder, methods[2]])?);
        assert_eq!(emu.string(text)?, "x".repeat(1000), "{arch:?}");
        emu.release(text);
    }
    Ok(())
}

#[test]
fn direct_host_policy_answer_survives_guest_calls_until_release() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let mut policy = Policy::parse(r#"{"rules":[{"value":"host answer"}]}"#)?;
        let mut fallback = ();
        let mut handler = policy.handler(&mut fallback);
        let before = emu.jni.objects.len();
        let answer = handler.member(
            &mut emu,
            &JniCall {
                op: JniOp::Call,
                is_static: false,
                type_code: b'L',
                class: "test/Answers",
                name: "value",
                signature: "()Ljava/lang/String;",
                receiver: Handle(0),
                args: &[],
            },
        );
        let answer = handler.finish(Ok(answer))?.unwrap().object();
        assert_eq!(emu.string(answer)?, "host answer", "{arch:?}");
        assert_eq!(jni_call(&mut emu, 4, &[])?, 0x10006); // GetVersion opens and pops a frame.
        assert_eq!(emu.string(answer)?, "host answer", "{arch:?}");
        assert_eq!(emu.jni.objects.len(), before + 1, "{arch:?}");
        emu.release(answer);
        assert_eq!(emu.jni.objects.len(), before, "{arch:?}");
        assert!(emu.string(answer).is_err());
    }
    Ok(())
}
