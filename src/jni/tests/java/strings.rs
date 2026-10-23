use super::*;

#[test]
fn strings_use_utf16_and_builtin_dispatch_before_the_handler() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let bytes = emu.new_byte_array("a😀é".as_bytes());
        let string = construct(&mut emu, "java/lang/String", "([B)V", &[bytes.into()])?;
        assert_eq!(call(&mut emu, string, "length()I", &[])?, JniValue::Int(4));
        assert_eq!(
            call(&mut emu, string, "charAt(I)C", &[1.into()])?,
            JniValue::Char(0xd83d)
        );
        let tail = call(
            &mut emu,
            string,
            "substring(I)Ljava/lang/String;",
            &[3.into()],
        )?
        .object();
        assert_eq!(emu.string(tail)?, "é");
        assert_eq!(text(&mut emu, string)?, "a😀é");
        let encoded = call(&mut emu, string, "getBytes()[B", &[])?.object();
        assert_eq!(emu.byte_array(encoded)?, "a😀é".as_bytes());
        assert_eq!(
            call(&mut emu, string, "hashCode()I", &[])?,
            JniValue::Int(57849829)
        );
        let other = emu.new_string("a😀é");
        assert_eq!(
            call(
                &mut emu,
                string,
                "equals(Ljava/lang/Object;)Z",
                &[other.into()]
            )?,
            JniValue::Boolean(true)
        );
    }
    Ok(())
}

#[test]
fn boxes_builders_charsets_and_class_objects_work_through_jni() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let boxed = static_call(
            &mut emu,
            "java/lang/Integer",
            "valueOf(I)Ljava/lang/Integer;",
            &[42.into()],
        )?
        .object();
        assert_eq!(
            call(&mut emu, boxed, "longValue()J", &[])?,
            JniValue::Long(42)
        );
        assert_eq!(text(&mut emu, boxed)?, "42");
        let builder = construct(&mut emu, "java/lang/StringBuilder", "()V", &[])?;
        let returned = call(
            &mut emu,
            builder,
            "append(Ljava/lang/Object;)Ljava/lang/StringBuilder;",
            &[boxed.into()],
        )?
        .object();
        assert_eq!(jni_call(&mut emu, 24, &[builder.0, returned.0])?, 1);
        call(
            &mut emu,
            builder,
            "append(C)Ljava/lang/StringBuilder;",
            &[JniValue::Char(0xd83d)],
        )?;
        call(
            &mut emu,
            builder,
            "append(C)Ljava/lang/StringBuilder;",
            &[JniValue::Char(0xde00)],
        )?;
        assert_eq!(text(&mut emu, builder)?, "42😀");
        let name = emu.new_string("utf_16le");
        let charset = static_call(
            &mut emu,
            "java/nio/charset/Charset",
            "forName(Ljava/lang/String;)Ljava/nio/charset/Charset;",
            &[name.into()],
        )?
        .object();
        let name = call(&mut emu, charset, "name()Ljava/lang/String;", &[])?.object();
        assert_eq!(emu.string(name)?, "UTF-16LE");
        let value = emu.new_string("é");
        let bytes = call(
            &mut emu,
            value,
            "getBytes(Ljava/nio/charset/Charset;)[B",
            &[charset.into()],
        )?
        .object();
        assert_eq!(emu.byte_array(bytes)?, [0xe9, 0]);
        let class = call(&mut emu, boxed, "getClass()Ljava/lang/Class;", &[])?.object();
        let name = call(&mut emu, class, "getName()Ljava/lang/String;", &[])?.object();
        assert_eq!(emu.string(name)?, "java.lang.Integer");
    }
    Ok(())
}
