use super::*;

#[test]
fn get_class_returns_a_new_local_reference_to_the_cached_class() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        jni_call(&mut emu, 19, &[16])?;
        jni_call(&mut emu, 19, &[16])?;
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
        // Keep local references across helper calls, which enter their own native frame.
        jni_call(&mut emu, 19, &[16])?;
        jni_call(&mut emu, 19, &[16])?;
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
