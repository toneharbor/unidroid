use super::*;

struct ReflectionHandler {
    class: &'static str,
    seen: Vec<String>,
}
impl ReflectionHandler {
    fn record(&mut self, call: &JniCall<'_>) {
        assert_eq!(call.class, self.class);
        self.seen.push(call.name.into());
    }
}
impl JniHandler for ReflectionHandler {
    fn member(&mut self, emu: &mut Emulator, call: &JniCall<'_>) -> Option<JniValue> {
        match (call.op, call.is_static, call.type_code) {
            (JniOp::Call, false, b'L') => {
                self.record(call);
                Some(JniValue::Object(emu.new_string("handled")))
            }
            (JniOp::Call, false, b'I') => {
                self.record(call);
                Some(JniValue::Int(7))
            }
            (JniOp::Call, false, b'V') => {
                self.record(call);
                assert_eq!(
                    emu.reflected_member(call.receiver).unwrap().class,
                    "example/Owner"
                );
                Some(JniValue::Void)
            }
            _ => None,
        }
    }
}

#[test]
fn reflection_answers_names_hierarchy_and_dispatches_other_calls_to_handler() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let class = emu.class("example/Owner");
        for (lookup, name, signature, to, java_class, java_name) in [
            (33, "run", "()V", 9, "java/lang/reflect/Method", "run"),
            (113, "run", "()V", 9, "java/lang/reflect/Method", "run"),
            (
                33,
                "<init>",
                "()V",
                9,
                "java/lang/reflect/Constructor",
                "example.Owner",
            ),
            (94, "count", "I", 12, "java/lang/reflect/Field", "count"),
            (144, "count", "I", 12, "java/lang/reflect/Field", "count"),
        ] {
            let id = get_id(&mut emu, lookup, class, name, signature)?;
            let reflected = Handle(jni_call(
                &mut emu,
                to,
                &[class.0, id, u64::from(lookup >= 113)],
            )?);
            let mut handler = ReflectionHandler {
                class: java_class,
                seen: Vec::new(),
            };
            let actual_class = member(
                &mut emu,
                &mut handler,
                reflected,
                "java/lang/Object",
                "getClass()Ljava/lang/Class;",
                &[],
                0,
            )?
            .object();
            let class_name =
                call(&mut emu, actual_class, "getName()Ljava/lang/String;", &[])?.object();
            assert_eq!(emu.string(class_name)?, java_class.replace('/', "."));
            let mut names = Vec::new();
            let mut declaring_classes = Vec::new();
            for _ in 0..2 {
                let name = member(
                    &mut emu,
                    &mut handler,
                    reflected,
                    "java/lang/reflect/AccessibleObject",
                    "getName()Ljava/lang/String;",
                    &[],
                    0,
                )?
                .object();
                assert_eq!(emu.string(name)?, java_name);
                names.push(name);
                let declaring = member(
                    &mut emu,
                    &mut handler,
                    reflected,
                    java_class,
                    "getDeclaringClass()Ljava/lang/Class;",
                    &[],
                    0,
                )?
                .object();
                assert_eq!(jni_call(&mut emu, 24, &[declaring.0, class.0])?, 1);
                declaring_classes.push(declaring);
            }
            assert_ne!(names[0], names[1]);
            assert_ne!(declaring_classes[0], declaring_classes[1]);
            assert!(handler.seen.is_empty());
            for (target, expected) in [
                (java_class, 1),
                ("java/lang/reflect/Executable", u64::from(to == 9)),
                ("java/lang/reflect/AccessibleObject", 1),
                ("java/lang/Object", 1),
                ("java/lang/String", 0),
            ] {
                let target = emu.class(target);
                assert_eq!(jni_call(&mut emu, 32, &[reflected.0, target.0])?, expected);
            }
            handler.class = "java/lang/Object";
            assert_eq!(
                member(
                    &mut emu,
                    &mut handler,
                    reflected,
                    "java/lang/Object",
                    "getModifiers()I",
                    &[],
                    0,
                )?,
                JniValue::Int(7)
            );
            member(
                &mut emu,
                &mut handler,
                reflected,
                "java/lang/Object",
                "hashCode()I",
                &[],
                0,
            )?;
            let value = member(
                &mut emu,
                &mut handler,
                reflected,
                "java/lang/Object",
                "toString()Ljava/lang/String;",
                &[],
                0,
            )?
            .object();
            assert_eq!(emu.string(value)?, "handled");
            assert_eq!(
                member(
                    &mut emu,
                    &mut handler,
                    reflected,
                    "java/lang/Object",
                    "equals(Ljava/lang/Object;)Z",
                    &[reflected.into()],
                    0,
                )?,
                JniValue::Boolean(true)
            );
            handler.class = "java/lang/reflect/AccessibleObject";
            assert_eq!(
                member(
                    &mut emu,
                    &mut handler,
                    reflected,
                    "java/lang/reflect/AccessibleObject",
                    "setAccessible(Z)V",
                    &[JniValue::Boolean(true)],
                    0,
                )?,
                JniValue::Void
            );
            handler.class = java_class;
            let value = member(
                &mut emu,
                &mut handler,
                reflected,
                java_class,
                "getName(I)Ljava/lang/String;",
                &[JniValue::Int(1)],
                0,
            )?
            .object();
            assert_eq!(emu.string(value)?, "handled");
            assert_eq!(
                handler.seen,
                ["getModifiers", "toString", "setAccessible", "getName"]
            );
        }
    }
    Ok(())
}
