use super::*;

#[test]
fn every_box_supports_guest_construction_values_text_and_hashes() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        for (class, signature, value, method, expected, hash) in [
            (
                "Boolean",
                "Z",
                JniValue::Boolean(true),
                "booleanValue()Z",
                "true",
                1231,
            ),
            ("Byte", "B", JniValue::Byte(-1), "byteValue()B", "-1", -1),
            (
                "Character",
                "C",
                JniValue::Char(233),
                "charValue()C",
                "é",
                233,
            ),
            ("Short", "S", JniValue::Short(-2), "shortValue()S", "-2", -2),
            (
                "Integer",
                "I",
                JniValue::Int(-42),
                "intValue()I",
                "-42",
                -42,
            ),
            (
                "Long",
                "J",
                JniValue::Long(0x1122334455667788),
                "longValue()J",
                "1234605616436508552",
                0x444444cc,
            ),
            (
                "Float",
                "F",
                JniValue::Float(1.5),
                "floatValue()F",
                "1.5",
                0x3fc00000,
            ),
            (
                "Double",
                "D",
                JniValue::Double(-2.25),
                "doubleValue()D",
                "-2.25",
                0xc0020000u32 as i32,
            ),
        ] {
            let class = format!("java/lang/{class}");
            let boxed = construct(&mut emu, &class, &format!("({signature})V"), &[value])?;
            assert_eq!(call(&mut emu, boxed, method, &[])?, value);
            assert_eq!(emu.unboxed(boxed)?, value);
            assert_eq!(text(&mut emu, boxed)?, expected);
            assert_eq!(
                call(&mut emu, boxed, "hashCode()I", &[])?,
                JniValue::Int(hash)
            );
            let other = static_call(
                &mut emu,
                &class,
                &format!("valueOf({signature})L{class};"),
                &[value],
            )?
            .object();
            assert_eq!(
                call(
                    &mut emu,
                    boxed,
                    "equals(Ljava/lang/Object;)Z",
                    &[other.into()]
                )?,
                JniValue::Boolean(true)
            );
        }
        for (value, expected) in [
            (JniValue::Float(f32::from_bits(1)), "1.4E-45"),
            (JniValue::Double(f64::from_bits(1)), "4.9E-324"),
            (JniValue::Double(-0.0), "-0.0"),
            (JniValue::Double(1e7), "1.0E7"),
            (JniValue::Float(f32::INFINITY), "Infinity"),
        ] {
            let boxed = emu.new_boxed(value)?;
            assert_eq!(text(&mut emu, boxed)?, expected);
        }
        let nan = emu.new_boxed(JniValue::Float(f32::NAN))?;
        let other = emu.new_boxed(JniValue::Float(f32::from_bits(0x7f800001)))?;
        assert_eq!(
            call(
                &mut emu,
                nan,
                "equals(Ljava/lang/Object;)Z",
                &[other.into()]
            )?,
            JniValue::Boolean(true)
        );
        assert_eq!(
            call(&mut emu, nan, "hashCode()I", &[])?,
            JniValue::Int(0x7fc00000)
        );
        let map = emu.new_java_map("java/util/HashMap", &[])?;
        for value in [
            JniValue::Int(7),
            JniValue::Long(7),
            JniValue::Float(0.0),
            JniValue::Float(-0.0),
        ] {
            let key = emu.new_boxed(value)?;
            call(
                &mut emu,
                map,
                "put(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
                &[key.into(), key.into()],
            )?;
        }
        assert_eq!(call(&mut emu, map, "size()I", &[])?, JniValue::Int(4));
    }
    Ok(())
}

#[test]
fn numeric_parse_and_format_follow_java_ranges_and_radices() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        for (input, radix, expected) in [
            ("-80000000", 16, i32::MIN),
            ("+7fffffff", 16, i32::MAX),
            ("１２３", 10, 123),
            ("101", 2, 5),
        ] {
            let input = emu.new_string(input);
            assert_eq!(
                static_call(
                    &mut emu,
                    "java/lang/Integer",
                    "parseInt(Ljava/lang/String;I)I",
                    &[input.into(), radix.into()]
                )?,
                JniValue::Int(expected)
            );
        }
        let input = emu.new_string("-9223372036854775808");
        assert_eq!(
            static_call(
                &mut emu,
                "java/lang/Long",
                "parseLong(Ljava/lang/String;)J",
                &[input.into()]
            )?,
            JniValue::Long(i64::MIN)
        );
        for (class, method, arg, expected) in [
            (
                "java/lang/Integer",
                "toHexString(I)Ljava/lang/String;",
                JniValue::Int(-1),
                "ffffffff",
            ),
            (
                "java/lang/Integer",
                "toString(I)Ljava/lang/String;",
                JniValue::Int(-42),
                "-42",
            ),
            (
                "java/lang/Long",
                "toString(J)Ljava/lang/String;",
                JniValue::Long(i64::MAX),
                "9223372036854775807",
            ),
        ] {
            let out = static_call(&mut emu, class, method, &[arg])?.object();
            assert_eq!(emu.string(out)?, expected);
        }
        for input in ["2147483648", " 1", "1 ", "", "+", "--1"] {
            let input = emu.new_string(input);
            assert!(
                static_call(
                    &mut emu,
                    "java/lang/Integer",
                    "parseInt(Ljava/lang/String;)I",
                    &[input.into()]
                )
                .unwrap_err()
                .to_string()
                .contains("NumberFormatException")
            );
        }
    }
    Ok(())
}
