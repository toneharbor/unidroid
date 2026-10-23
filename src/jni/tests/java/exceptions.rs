use super::*;

fn pending(
    emu: &mut Emulator,
    receiver: Handle,
    class: &str,
    method: &str,
    args: &[JniValue],
    static_method: bool,
    expected: &str,
) -> Result<()> {
    let (name, sig) = method.split_at(method.find('(').unwrap());
    let class = emu.class(class);
    let name = emu.new_cstring(name)?;
    let signature = emu.new_cstring(sig)?;
    let id = jni_call(
        emu,
        if static_method { 113 } else { 33 },
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
            if static_method { 116 } else { 36 } + offset,
            &[if static_method { class.0 } else { receiver.0 }, id, slots]
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
            true,
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
            false,
            "java.lang.IndexOutOfBoundsException",
        )?;
        let iterator = call(&mut emu, list, "iterator()Ljava/util/Iterator;", &[])?.object();
        pending(
            &mut emu,
            iterator,
            "java/util/Iterator",
            "next()Ljava/lang/Object;",
            &[],
            false,
            "java.util.NoSuchElementException",
        )?;
        let bad = emu.new_string("a");
        pending(
            &mut emu,
            Handle(0),
            "android/util/Base64",
            "decode(Ljava/lang/String;I)[B",
            &[bad.into(), 0.into()],
            true,
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
