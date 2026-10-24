use super::*;

fn pending(
    emu: &mut Emulator,
    receiver: Handle,
    class: &str,
    method: &str,
    args: &[JniValue],
    operation: u8,
    expected: &str,
) -> Result<()> {
    let (name, sig) = method.split_at(method.find('(').unwrap());
    let class = emu.class(class);
    let name = emu.new_cstring(name)?;
    let signature = emu.new_cstring(sig)?;
    let id = jni_call(
        emu,
        if operation == 1 { 113 } else { 33 },
        &[class.0, name, signature],
    )?;
    let slots = emu.malloc(args.len().max(1) * 8)?;
    for (i, arg) in args.iter().enumerate() {
        emu.write_u64(slots + i as u64 * 8, arg.bits())?;
    }
    let kind = sig.as_bytes()[sig.find(')').unwrap() + 1];
    let offset = b"LZBCSIJFDV"
        .iter()
        .position(|&b| b == if kind == b'[' { b'L' } else { kind })
        .unwrap() as u64
        * 3;
    assert_eq!(
        jni_call(
            emu,
            if operation == 6 {
                30
            } else {
                (if operation == 1 { 116 } else { 36 }) + offset
            },
            &[if operation == 0 { receiver.0 } else { class.0 }, id, slots]
        )?,
        0
    );
    assert_eq!(jni_call(emu, 228, &[])?, 1);
    let exception = jni_call(emu, 15, &[])?;
    let class = Handle(jni_call(emu, 31, &[exception])?);
    jni_call(emu, 17, &[])?;
    let name = call(emu, class, "getName()Ljava/lang/String;", &[])?.object();
    assert_eq!(emu.string(name)?, expected);
    Ok(())
}

#[test]
fn null_builtin_arrays_raise_java_exceptions() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let charset = emu.new_string("UTF-8");
        for (signature, args) in [
            ("([B)V", vec![Handle(0).into()]),
            ("([C)V", vec![Handle(0).into()]),
            ("([BII)V", vec![Handle(0).into(), 0.into(), 0.into()]),
            (
                "([BLjava/lang/String;)V",
                vec![Handle(0).into(), charset.into()],
            ),
            (
                "([BIILjava/lang/String;)V",
                vec![Handle(0).into(), 0.into(), 0.into(), charset.into()],
            ),
        ] {
            pending(
                &mut emu,
                Handle(0),
                "java/lang/String",
                &format!("<init>{signature}"),
                &args,
                6,
                "java.lang.NullPointerException",
            )?;
            let mut invalid = args;
            invalid[0] = Handle(0x123456).into();
            let error = construct(&mut emu, "java/lang/String", signature, &invalid)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("invalid") && error.contains(" from "),
                "{error}"
            );
            assert_eq!(jni_call(&mut emu, 228, &[])?, 0);
        }
        for method in [
            "encode([BI)[B",
            "encodeToString([BI)Ljava/lang/String;",
            "decode([BI)[B",
        ] {
            pending(
                &mut emu,
                Handle(0),
                "android/util/Base64",
                method,
                &[Handle(0).into(), 0.into()],
                1,
                "java.lang.NullPointerException",
            )?;
        }
        let algorithm = emu.new_string("SHA-256");
        let digest = static_call(
            &mut emu,
            "java/security/MessageDigest",
            "getInstance(Ljava/lang/String;)Ljava/security/MessageDigest;",
            &[algorithm.into()],
        )?
        .object();
        for (method, args, expected, message) in [
            (
                "update([B)V",
                vec![Handle(0).into()],
                "java.lang.NullPointerException",
                "",
            ),
            (
                "update([BII)V",
                vec![Handle(0).into(), 0.into(), 0.into()],
                "java.lang.IllegalArgumentException",
                ": No input buffer given",
            ),
            (
                "digest([B)[B",
                vec![Handle(0).into()],
                "java.lang.NullPointerException",
                "",
            ),
        ] {
            pending(
                &mut emu,
                digest,
                "java/security/MessageDigest",
                method,
                &args,
                0,
                expected,
            )?;
            let error = call(&mut emu, digest, method, &args)
                .unwrap_err()
                .to_string();
            assert_eq!(
                error,
                format!(
                    "java exception thrown by native code: {}{message} from 0x20000",
                    expected.replace('.', "/")
                )
            );
        }
    }
    Ok(())
}

#[test]
fn null_charset_names_raise_pending_exceptions_and_invalid_names_stay_reference_errors()
-> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let bytes = emu.new_byte_array(b"a");
        let receiver = emu.new_string("a");
        let released = emu.new_string("UTF-8");
        emu.release(released);
        for (method, args, operation, function) in [
            (
                "<init>([BLjava/lang/String;)V",
                vec![bytes.into(), Handle(0).into()],
                6,
                "NewObjectA",
            ),
            (
                "<init>([BIILjava/lang/String;)V",
                vec![bytes.into(), 0.into(), 1.into(), Handle(0).into()],
                6,
                "NewObjectA",
            ),
            (
                "getBytes(Ljava/lang/String;)[B",
                vec![Handle(0).into()],
                0,
                "CallObjectMethodA",
            ),
        ] {
            pending(
                &mut emu,
                receiver,
                "java/lang/String",
                method,
                &args,
                operation,
                "java.lang.NullPointerException",
            )?;
            for invalid in [Handle(0x123456), bytes, released] {
                let mut args = args.clone();
                *args.last_mut().unwrap() = invalid.into();
                let error = member(
                    &mut emu,
                    &mut (),
                    receiver,
                    "java/lang/String",
                    method,
                    &args,
                    operation,
                )
                .unwrap_err()
                .to_string();
                assert_eq!(
                    error,
                    format!(
                        "invalid JNI reference 0x{:x} in {function} from 0x20000",
                        invalid.0
                    )
                );
                assert_eq!(jni_call(&mut emu, 228, &[])?, 0);
            }
        }
    }
    Ok(())
}

#[test]
fn data_errors_return_zero_and_set_pending_java_exceptions() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let bad = emu.new_string("x");
        pending(
            &mut emu,
            Handle(0),
            "java/lang/Integer",
            "parseInt(Ljava/lang/String;)I",
            &[bad.into()],
            1,
            "java.lang.NumberFormatException",
        )?;
        let error = static_call(
            &mut emu,
            "java/lang/Integer",
            "parseInt(Ljava/lang/String;)I",
            &[bad.into()],
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("java/lang/NumberFormatException: For input string: \"x\""),
            "{error}"
        );
        let list = emu.new_java_list("java/util/ArrayList", &[])?;
        pending(
            &mut emu,
            list,
            "java/util/List",
            "get(I)Ljava/lang/Object;",
            &[0.into()],
            0,
            "java.lang.IndexOutOfBoundsException",
        )?;
        let iterator = call(&mut emu, list, "iterator()Ljava/util/Iterator;", &[])?.object();
        pending(
            &mut emu,
            iterator,
            "java/util/Iterator",
            "next()Ljava/lang/Object;",
            &[],
            0,
            "java.util.NoSuchElementException",
        )?;
        let bad = emu.new_string("a");
        pending(
            &mut emu,
            Handle(0),
            "android/util/Base64",
            "decode(Ljava/lang/String;I)[B",
            &[bad.into(), 0.into()],
            1,
            "java.lang.IllegalArgumentException",
        )?;
        let error = static_call(
            &mut emu,
            "android/util/Base64",
            "decode(Ljava/lang/String;I)[B",
            &[bad.into(), 0.into()],
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("java/lang/IllegalArgumentException: bad base-64"));
    }
    Ok(())
}
