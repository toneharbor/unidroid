use super::*;

#[test]
fn string_methods_and_constructors_preserve_java_text_rules() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let bytes = emu.new_byte_array(b"xabcy");
        let charset = emu.new_string("uTf_8");
        for (signature, args, expected) in [
            ("()V", vec![], ""),
            ("([B)V", vec![bytes.into()], "xabcy"),
            (
                "([BLjava/lang/String;)V",
                vec![bytes.into(), charset.into()],
                "xabcy",
            ),
            ("([BII)V", vec![bytes.into(), 1.into(), 3.into()], "abc"),
            (
                "([BIILjava/lang/String;)V",
                vec![bytes.into(), 1.into(), 3.into(), charset.into()],
                "abc",
            ),
            ("(Ljava/lang/String;)V", vec![charset.into()], "uTf_8"),
        ] {
            let string = construct(&mut emu, "java/lang/String", signature, &args)?;
            assert_eq!(emu.string(string)?, expected);
        }
        let chars = emu.new_primitive_array(PrimitiveType::Char, &[0x41, 0, 0x3d, 0xd8, 0, 0xde]);
        let string = construct(&mut emu, "java/lang/String", "([C)V", &[chars.into()])?;
        assert_eq!(emu.string(string)?, "A😀");
        let chars = call(&mut emu, string, "toCharArray()[C", &[])?.object();
        assert_eq!(emu.primitive_array(chars)?, [0x41, 0, 0x3d, 0xd8, 0, 0xde]);
        let string = emu.new_string("abcabc😀");
        let a = emu.new_string("abc");
        let upper = emu.new_string("ABC");
        let emoji = emu.new_string("😀");
        for (method, args, expected) in [
            ("isEmpty()Z", vec![], false),
            ("startsWith(Ljava/lang/String;)Z", vec![a.into()], true),
            ("endsWith(Ljava/lang/String;)Z", vec![emoji.into()], true),
            (
                "contains(Ljava/lang/CharSequence;)Z",
                vec![upper.into()],
                false,
            ),
        ] {
            assert_eq!(
                call(&mut emu, string, method, &args)?,
                JniValue::Boolean(expected)
            );
        }
        assert_eq!(
            call(
                &mut emu,
                a,
                "equalsIgnoreCase(Ljava/lang/String;)Z",
                &[upper.into()]
            )?,
            JniValue::Boolean(true)
        );
        assert_eq!(
            call(
                &mut emu,
                a,
                "compareTo(Ljava/lang/String;)I",
                &[upper.into()]
            )?,
            JniValue::Int(32)
        );
        assert_eq!(
            call(
                &mut emu,
                string,
                "indexOf(Ljava/lang/String;)I",
                &[emoji.into()]
            )?,
            JniValue::Int(6)
        );
        assert_eq!(
            call(&mut emu, string, "indexOf(I)I", &[0x1f600.into()])?,
            JniValue::Int(6)
        );
        let replaced = call(
            &mut emu,
            string,
            "replace(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Ljava/lang/String;",
            &[a.into(), upper.into()],
        )?
        .object();
        assert_eq!(emu.string(replaced)?, "ABCABC😀");
        let part = call(
            &mut emu,
            string,
            "substring(II)Ljava/lang/String;",
            &[3.into(), 6.into()],
        )?
        .object();
        assert_eq!(emu.string(part)?, "abc");
        let joined = call(
            &mut emu,
            part,
            "concat(Ljava/lang/String;)Ljava/lang/String;",
            &[emoji.into()],
        )?
        .object();
        assert_eq!(emu.string(joined)?, "abc😀");
        for (input, method, expected) in [
            (" ß\t", "trim()Ljava/lang/String;", "ß"),
            ("straße", "toUpperCase()Ljava/lang/String;", "STRASSE"),
            ("ΟΣ", "toLowerCase()Ljava/lang/String;", "ος"),
        ] {
            let s = emu.new_string(input);
            let out = call(&mut emu, s, method, &[])?.object();
            assert_eq!(emu.string(out)?, expected);
        }
        let sharp = emu.new_string("ß");
        let s = emu.new_string("s");
        assert_eq!(
            call(
                &mut emu,
                sharp,
                "equalsIgnoreCase(Ljava/lang/String;)Z",
                &[s.into()]
            )?,
            JniValue::Boolean(false)
        );
        for (signature, value, expected) in [
            ("(I)Ljava/lang/String;", JniValue::Int(-42), "-42"),
            (
                "(J)Ljava/lang/String;",
                JniValue::Long(i64::MIN),
                "-9223372036854775808",
            ),
            ("(Z)Ljava/lang/String;", JniValue::Boolean(true), "true"),
            ("(C)Ljava/lang/String;", JniValue::Char(0xe9), "é"),
            (
                "(Ljava/lang/Object;)Ljava/lang/String;",
                Handle(0).into(),
                "null",
            ),
        ] {
            let out = static_call(
                &mut emu,
                "java/lang/String",
                &format!("valueOf{signature}"),
                &[value],
            )?
            .object();
            assert_eq!(emu.string(out)?, expected);
        }
    }
    Ok(())
}

#[test]
fn charsets_match_java_replacement_rules_and_standard_fields() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        for (field, expected, bytes) in [
            ("UTF_8", "UTF-8", vec![0xc3, 0xa9]),
            ("US_ASCII", "US-ASCII", vec![b'?']),
            ("ISO_8859_1", "ISO-8859-1", vec![0xe9]),
            ("UTF_16LE", "UTF-16LE", vec![0xe9, 0]),
            ("UTF_16BE", "UTF-16BE", vec![0, 0xe9]),
        ] {
            let class = emu.class("java/nio/charset/StandardCharsets");
            let name = emu.new_cstring(field)?;
            let sig = emu.new_cstring("Ljava/nio/charset/Charset;")?;
            let id = jni_call(&mut emu, 144, &[class.0, name, sig])?;
            let charset = Handle(jni_call(&mut emu, 145, &[class.0, id])?);
            let name = call(&mut emu, charset, "name()Ljava/lang/String;", &[])?.object();
            assert_eq!(emu.string(name)?, expected);
            let s = emu.new_string("é");
            let out = call(
                &mut emu,
                s,
                "getBytes(Ljava/nio/charset/Charset;)[B",
                &[charset.into()],
            )?
            .object();
            assert_eq!(emu.byte_array(out)?, bytes);
        }
        let bytes = emu.new_byte_array(&[0xd8, 0, 0, 65]);
        let charset = emu.new_string("UTF-16BE");
        let s = construct(
            &mut emu,
            "java/lang/String",
            "([BLjava/lang/String;)V",
            &[bytes.into(), charset.into()],
        )?;
        assert_eq!(emu.string(s)?, "�");
        for (bytes, expected) in [
            (vec![0xed, 0xa0, 0x80], "�"),
            (vec![0xe1, 0x80], "�"),
            (vec![0xf4, 0x90, 0x80, 0x80], "����"),
        ] {
            let bytes = emu.new_byte_array(&bytes);
            let s = construct(&mut emu, "java/lang/String", "([B)V", &[bytes.into()])?;
            assert_eq!(emu.string(s)?, expected);
        }
    }
    Ok(())
}
