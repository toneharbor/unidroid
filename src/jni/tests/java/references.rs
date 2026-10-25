use super::*;

#[test]
fn null_is_an_instance_of_any_valid_class_but_invalid_references_still_fail() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        for name in [
            "java/lang/Object",
            "java/lang/String",
            "java/lang/Class",
            "app/Unknown",
            "[B",
        ] {
            let class = emu.class(name);
            assert_eq!(jni_call(&mut emu, 32, &[0, class.0])?, 1, "{name}");
        }
        let wrong_kind = emu.new_string("java/lang/String");
        let released = emu.class("app/Released");
        emu.release(released);
        for invalid in [0, 0x123456, wrong_kind.0, released.0] {
            assert_eq!(
                jni_call(&mut emu, 32, &[0, invalid])
                    .unwrap_err()
                    .to_string(),
                format!("invalid JNI reference 0x{invalid:x} in IsInstanceOf from 0x20000")
            );
        }
        let class = emu.class("java/lang/Object");
        assert_eq!(
            jni_call(&mut emu, 32, &[0x123456, class.0])
                .unwrap_err()
                .to_string(),
            "invalid JNI reference 0x123456 in IsInstanceOf from 0x20000"
        );
    }
    Ok(())
}

#[test]
fn class_instances_use_runtime_type_but_assignability_uses_represented_type() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let string = emu.class("java/lang/String");
        let class = emu.class("java/lang/Class");
        let object = emu.class("java/lang/Object");
        assert_eq!(jni_call(&mut emu, 32, &[string.0, class.0])?, 1);
        assert_eq!(jni_call(&mut emu, 32, &[string.0, string.0])?, 0);
        assert_eq!(jni_call(&mut emu, 32, &[string.0, object.0])?, 1);
        assert_eq!(jni_call(&mut emu, 11, &[string.0, class.0])?, 0);
        assert_eq!(jni_call(&mut emu, 11, &[string.0, string.0])?, 1);
        assert_eq!(jni_call(&mut emu, 11, &[string.0, object.0])?, 1);
    }
    Ok(())
}

#[test]
fn get_class_returns_a_new_local_reference_to_the_cached_class() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        // Act as code inside a native method's frame.
        emu.jni.scope = 2;
        let name = emu.new_cstring("java/lang/Class")?;
        let class = Handle(jni_call(&mut emu, 6, &[name])?);
        emu.free(name);
        let result = call(&mut emu, class, "getClass()Ljava/lang/Class;", &[])?.object();
        assert_eq!(jni_call(&mut emu, 24, &[class.0, result.0])?, 1);
        jni_call(&mut emu, 23, &[result.0])?;
        assert_eq!(jni_call(&mut emu, 232, &[result.0])?, 0);
        assert_eq!(jni_call(&mut emu, 232, &[class.0])?, 1);
        let name = call(&mut emu, class, "getName()Ljava/lang/String;", &[])?.object();
        assert_eq!(emu.string(name)?, "java.lang.Class");
        jni_call(&mut emu, 20, &[0])?;
        jni_call(&mut emu, 20, &[0])?;
    }
    Ok(())
}

#[test]
fn returned_local_references_can_be_deleted_without_releasing_receivers() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        // Act as code inside a native method's frame.
        emu.jni.scope = 2;
        let string = emu.new_string("abc");
        let string = Handle(jni_call(&mut emu, 25, &[string.0])?);
        let builder = construct(&mut emu, "java/lang/StringBuilder", "()V", &[])?;
        for (receiver, method, args, length) in [
            (string, "toString()Ljava/lang/String;", vec![], 3),
            (
                builder,
                "append(I)Ljava/lang/StringBuilder;",
                vec![JniValue::Int(7)],
                1,
            ),
        ] {
            let result = call(&mut emu, receiver, method, &args)?.object();
            assert_eq!(jni_call(&mut emu, 232, &[result.0])?, 1);
            assert_eq!(jni_call(&mut emu, 24, &[receiver.0, result.0])?, 1);
            jni_call(&mut emu, 23, &[result.0])?;
            assert_eq!(jni_call(&mut emu, 232, &[result.0])?, 0);
            assert_eq!(jni_call(&mut emu, 232, &[receiver.0])?, 1);
            assert_eq!(
                call(&mut emu, receiver, "length()I", &[])?,
                JniValue::Int(length)
            );
        }
        jni_call(&mut emu, 20, &[0])?;
        jni_call(&mut emu, 20, &[0])?;
    }
    Ok(())
}
