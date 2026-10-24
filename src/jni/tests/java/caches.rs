use super::*;

fn standard_charset(emu: &mut Emulator, name: &str) -> Result<Handle> {
    let class = emu.class("java/nio/charset/StandardCharsets");
    let name = emu.new_cstring(name)?;
    let signature = emu.new_cstring("Ljava/nio/charset/Charset;")?;
    let id = jni_call(emu, 144, &[class.0, name, signature])?;
    emu.free(name);
    emu.free(signature);
    Ok(Handle(jni_call(emu, 145, &[class.0, id])?))
}

#[test]
fn standard_charsets_share_identity_and_compare_by_canonical_name() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let mut charsets: Vec<(&str, Handle)> = Vec::new();
        for (field, alias, name) in [
            ("UTF_8", "utf-8", "UTF-8"),
            ("US_ASCII", "us_ascii", "US-ASCII"),
            ("ISO_8859_1", "iso88591", "ISO-8859-1"),
            ("UTF_16LE", "utf16le", "UTF-16LE"),
            ("UTF_16BE", "utf16be", "UTF-16BE"),
        ] {
            let first = standard_charset(&mut emu, field)?;
            let again = standard_charset(&mut emu, field)?;
            assert_ne!(first, again);
            assert_eq!(jni_call(&mut emu, 24, &[first.0, again.0])?, 1);
            let alias = emu.new_string(alias);
            let parsed = static_call(
                &mut emu,
                "java/nio/charset/Charset",
                "forName(Ljava/lang/String;)Ljava/nio/charset/Charset;",
                &[alias.into()],
            )?
            .object();
            assert_eq!(
                call(
                    &mut emu,
                    parsed,
                    "equals(Ljava/lang/Object;)Z",
                    &[first.into()]
                )?,
                JniValue::Boolean(true)
            );
            let name = emu.new_string(name);
            let hash = call(&mut emu, name, "hashCode()I", &[])?;
            assert_eq!(call(&mut emu, first, "hashCode()I", &[])?, hash);
            assert_eq!(call(&mut emu, parsed, "hashCode()I", &[])?, hash);
            let map = emu.new_java_map("java/util/HashMap", &[(first, name)])?;
            let found = call(
                &mut emu,
                map,
                "get(Ljava/lang/Object;)Ljava/lang/Object;",
                &[parsed.into()],
            )?
            .object();
            assert_eq!(jni_call(&mut emu, 24, &[found.0, name.0])?, 1);
            assert_eq!(
                call(
                    &mut emu,
                    first,
                    "equals(Ljava/lang/Object;)Z",
                    &[name.into()]
                )?,
                JniValue::Boolean(false)
            );
            for &(_, earlier) in &charsets {
                assert_eq!(
                    call(
                        &mut emu,
                        first,
                        "equals(Ljava/lang/Object;)Z",
                        &[earlier.into()]
                    )?,
                    JniValue::Boolean(false)
                );
            }
            charsets.push((field, first));
        }
        let saved = emu.checkpoint()?;
        let mut fresh = Emulator::builder().build()?;
        for target in [&mut emu, &mut fresh] {
            target.rewind(&saved)?;
            for &(field, first) in &charsets {
                let again = standard_charset(target, field)?;
                assert_eq!(jni_call(target, 24, &[first.0, again.0])?, 1);
            }
        }
    }
    Ok(())
}

fn value_of(emu: &mut Emulator, class: &str, value: JniValue) -> Result<Handle> {
    Ok(static_call(
        emu,
        class,
        &format!("valueOf({})L{class};", value.type_code() as char),
        &[value],
    )?
    .object())
}

#[test]
fn value_of_caches_required_ranges_and_constructors_stay_fresh() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        for i in -128..=127 {
            for (class, value) in [
                ("java/lang/Byte", JniValue::Byte(i as i8)),
                ("java/lang/Short", JniValue::Short(i as i16)),
                ("java/lang/Integer", JniValue::Int(i)),
                ("java/lang/Long", JniValue::Long(i as i64)),
            ]
            .into_iter()
            .chain((i >= 0).then_some(("java/lang/Character", JniValue::Char(i as u16))))
            .chain(
                (0..=1)
                    .contains(&i)
                    .then_some(("java/lang/Boolean", JniValue::Boolean(i != 0))),
            ) {
                let first = value_of(&mut emu, class, value)?;
                let again = value_of(&mut emu, class, value)?;
                assert_ne!(first, again);
                assert_eq!(
                    jni_call(&mut emu, 24, &[first.0, again.0])?,
                    1,
                    "{class}: {value:?}"
                );
                assert_eq!(emu.unboxed(first)?, value);
                let new = construct(
                    &mut emu,
                    class,
                    &format!("({})V", value.type_code() as char),
                    &[value],
                )?;
                let other = construct(
                    &mut emu,
                    class,
                    &format!("({})V", value.type_code() as char),
                    &[value],
                )?;
                assert_eq!(jni_call(&mut emu, 24, &[new.0, other.0])?, 0);
                assert_eq!(jni_call(&mut emu, 24, &[new.0, first.0])?, 0);
                for handle in [first, again, new, other] {
                    emu.release(handle);
                }
            }
        }
        for (class, value) in [
            ("java/lang/Short", JniValue::Short(-129)),
            ("java/lang/Integer", JniValue::Int(128)),
            ("java/lang/Long", JniValue::Long(128)),
            ("java/lang/Character", JniValue::Char(128)),
            ("java/lang/Float", JniValue::Float(7.0)),
            ("java/lang/Double", JniValue::Double(7.0)),
        ] {
            let first = value_of(&mut emu, class, value)?;
            let again = value_of(&mut emu, class, value)?;
            assert_eq!(jni_call(&mut emu, 24, &[first.0, again.0])?, 0);
        }
    }
    Ok(())
}

#[test]
fn boxed_caches_survive_released_handles_and_checkpoint_copies() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let mut boxes = Vec::new();
        for (class, value) in [
            ("java/lang/Boolean", JniValue::Boolean(false)),
            ("java/lang/Boolean", JniValue::Boolean(true)),
            ("java/lang/Byte", JniValue::Byte(7)),
            ("java/lang/Short", JniValue::Short(7)),
            ("java/lang/Integer", JniValue::Int(7)),
            ("java/lang/Long", JniValue::Long(7)),
            ("java/lang/Character", JniValue::Char(7)),
        ] {
            let handle = value_of(&mut emu, class, value)?;
            let retained = emu.new_java_list("java/util/ArrayList", &[handle])?;
            emu.release(handle);
            boxes.push((class, value, retained));
        }
        let saved = emu.checkpoint()?;
        let mut fresh = Emulator::builder().build()?;
        for target in [&mut emu, &mut fresh] {
            target.rewind(&saved)?;
            for &(class, value, retained) in &boxes {
                let old = target.java_list(retained)?[0];
                let new = value_of(target, class, value)?;
                assert_eq!(jni_call(target, 24, &[old.0, new.0])?, 1, "{class}");
            }
        }
    }
    Ok(())
}
