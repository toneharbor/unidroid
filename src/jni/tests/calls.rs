use super::*;
use crate::jni::value::{argument_types, decode};

#[test]
fn arm32_mixed_variadic_v_and_a_calls_include_nonvirtual_stack_arguments() -> Result<()> {
    struct Handler;
    impl JniHandler for Handler {
        fn call_long_method(&mut self, _: &mut Emulator, call: &JniCall<'_>) -> Option<i64> {
            assert_eq!(
                call.args,
                [
                    JniValue::Long(0x1122_3344_5566_7788),
                    JniValue::Int(-7),
                    JniValue::Long(-2),
                    JniValue::Float(3.5),
                    JniValue::Double(9.5)
                ]
            );
            Some(0x8877_6655_4433_2211u64 as i64)
        }
        fn call_static_long_method(
            &mut self,
            emu: &mut Emulator,
            call: &JniCall<'_>,
        ) -> Option<i64> {
            self.call_long_method(emu, call)
        }
    }
    let mut emu = Emulator::builder().build()?;
    emu.setup_cpu(crate::Architecture::Arm32)?;
    let class = emu.class("example/Mixed");
    let name = emu.new_cstring("mixed")?;
    let sig = emu.new_cstring("(JIJFD)J")?;
    let table = emu.read_pointer(emu.jni.env)?;
    let memory = emu.malloc(64)?;
    let va = memory + 4; // The first jlong must align this address up to 8.
    for (offset, value) in [
        (8, 0x1122_3344_5566_7788),
        (24, (-2i64) as u64),
        (32, 3.5f64.to_bits()),
        (40, 9.5f64.to_bits()),
    ] {
        emu.write_u64(memory + offset, value)?;
    }
    emu.write_pointer(memory + 16, (-7i32) as u64)?;
    let slots = emu.malloc(40)?;
    for (i, value) in [
        0x1122_3344_5566_7788,
        (-7i32) as u64,
        (-2i64) as u64,
        3.5f32.to_bits() as u64,
        9.5f64.to_bits(),
    ]
    .into_iter()
    .enumerate()
    {
        emu.write_u64(slots + i as u64 * 8, value)?;
    }
    for base in [52, 82, 132] {
        let nonvirtual = base == 82;
        let get_id = emu.read_pointer(table + if base == 132 { 113 * 4 } else { 33 * 4 })?;
        let id = emu.call_function(get_id, &[emu.jni.env, class.0, name, sig])?;
        for variant in 0..3 {
            let mut words = vec![emu.jni.env, class.0];
            if nonvirtual {
                words.push(class.0);
            }
            words.push(id);
            if variant == 0 {
                if !nonvirtual {
                    words.push(0xdead_beef);
                } // r3 is skipped for jlong.
                words.extend([
                    0x5566_7788,
                    0x1122_3344,
                    0xffff_fff9,
                    0xdead_beef,
                    0xffff_fffe,
                    0xffff_ffff,
                    0,
                    0x400c_0000,
                    0,
                    0x4023_0000,
                ]);
            } else {
                words.push(if variant == 1 { va } else { slots });
            }
            let args: Vec<_> = words
                .into_iter()
                .map(|word| JniValue::Int(word as i32))
                .collect();
            let function = emu.read_pointer(table + (base + variant) * 4)?;
            assert_eq!(
                emu.invoke_jni(function, &args, b'J', &mut Handler)?,
                JniValue::Long(0x8877_6655_4433_2211u64 as i64)
            );
            assert_eq!(emu.reg(0)?, 0x4433_2211);
            assert_eq!(emu.reg(1)?, 0x8877_6655);
        }
    }
    Ok(())
}

#[test]
fn arm32_native_arguments_use_even_register_pairs_and_aligned_stack_slots() -> Result<()> {
    let mut emu = Emulator::builder().build()?;
    emu.setup_cpu(crate::Architecture::Arm32)?;
    let code = crate::RETURN + 0x100;
    // mov r0,r2; mov r1,r3; bx lr
    emu.write_bytes(
        code,
        &[0xe1a00002u32, 0xe1a01003, 0xe12fff1e]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>(),
    )?;
    assert_eq!(
        emu.invoke_jni(
            code,
            &[JniValue::Int(9), JniValue::Long(0x1122_3344_5566_7788)],
            b'J',
            &mut ()
        )?,
        JniValue::Long(0x1122_3344_5566_7788)
    );
    let args = [
        JniValue::Int(1),
        JniValue::Int(2),
        JniValue::Int(3),
        JniValue::Long(0x1122_3344_5566_7788),
        JniValue::Float(3.5),
        JniValue::Double(9.5),
        JniValue::Int(7),
    ];
    // ldr r0,[sp,#offset]; ldr r1,[sp,#offset+4]; bx lr
    for (offset, kind, expected) in [
        (0, b'J', args[3]),
        (8, b'F', args[4]),
        (16, b'D', args[5]),
        (24, b'I', args[6]),
    ] {
        emu.write_bytes(
            code,
            &[
                0xe59d0000u32 | offset,
                0xe59d1000 | (offset + 4),
                0xe12fff1e,
            ]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>(),
        )?;
        assert_eq!(emu.invoke_jni(code, &args, kind, &mut ())?, expected);
    }
    // ldr r0,[sp]; bx lr: the fifth raw argument is one truncated word.
    emu.write_bytes(
        code,
        &[0xe59d0000u32, 0xe12fff1e]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>(),
    )?;
    assert_eq!(
        emu.call_function(code, &[1, 2, 3, 4, 0x1234_5678_ffff_ffff])?,
        0xffff_ffff
    );
    Ok(())
}

#[test]
fn arm32_field_values_and_double_results_follow_softfp() -> Result<()> {
    struct Handler(JniValue);
    impl Handler {
        fn check(&self, call: &JniCall<'_>) -> Option<()> {
            assert_eq!(call.args, [self.0]);
            Some(())
        }
    }
    impl JniHandler for Handler {
        fn set_long_field(&mut self, _: &mut Emulator, c: &JniCall<'_>) -> Option<()> {
            self.check(c)
        }
        fn set_static_long_field(&mut self, _: &mut Emulator, c: &JniCall<'_>) -> Option<()> {
            self.check(c)
        }
        fn set_float_field(&mut self, _: &mut Emulator, c: &JniCall<'_>) -> Option<()> {
            self.check(c)
        }
        fn set_static_float_field(&mut self, _: &mut Emulator, c: &JniCall<'_>) -> Option<()> {
            self.check(c)
        }
        fn set_double_field(&mut self, _: &mut Emulator, c: &JniCall<'_>) -> Option<()> {
            self.check(c)
        }
        fn set_static_double_field(&mut self, _: &mut Emulator, c: &JniCall<'_>) -> Option<()> {
            self.check(c)
        }
        fn get_double_field(&mut self, _: &mut Emulator, _: &JniCall<'_>) -> Option<f64> {
            Some(9.5)
        }
        fn get_static_double_field(&mut self, _: &mut Emulator, _: &JniCall<'_>) -> Option<f64> {
            Some(9.5)
        }
    }
    let mut emu = Emulator::builder().build()?;
    emu.setup_cpu(crate::Architecture::Arm32)?;
    let class = emu.class("example/Fields");
    let table = emu.read_pointer(emu.jni.env)?;
    let name = emu.new_cstring("value")?;
    for static_field in [false, true] {
        for (signature, index, value) in [
            ("J", 110, JniValue::Long(0x1122_3344_5566_7788)),
            ("F", 111, JniValue::Float(3.5)),
            ("D", 112, JniValue::Double(9.5)),
        ] {
            let signature = emu.new_cstring(signature)?;
            let get_id = emu.read_pointer(table + if static_field { 144 * 4 } else { 94 * 4 })?;
            let id = emu.call_function(get_id, &[emu.jni.env, class.0, name, signature])?;
            let mut words = vec![emu.jni.env, class.0, id];
            if index != 111 {
                words.push(0xdead_beef);
            }
            words.push(value.bits() & 0xffff_ffff);
            if index != 111 {
                words.push(value.bits() >> 32);
            }
            let args: Vec<_> = words
                .into_iter()
                .map(|word| JniValue::Int(word as i32))
                .collect();
            let setter =
                emu.read_pointer(table + (index + if static_field { 50 } else { 0 }) * 4)?;
            emu.invoke_jni(setter, &args, b'V', &mut Handler(value))?;
            if index == 112 {
                let getter =
                    emu.read_pointer(table + if static_field { 153 * 4 } else { 103 * 4 })?;
                assert_eq!(
                    emu.invoke_jni(
                        getter,
                        &[Handle(emu.jni.env).into(), class.into(), Handle(id).into()],
                        b'D',
                        &mut Handler(value)
                    )?,
                    value
                );
                assert_eq!(emu.reg(0)?, 0);
                assert_eq!(emu.reg(1)?, 0x4023_0000);
            }
        }
    }
    Ok(())
}

#[test]
fn jni_a_and_v_and_variadic_calls_decode_the_same_arguments() -> Result<()> {
    struct Handler {
        array: Handle,
    }
    impl JniHandler for Handler {
        fn call_static_double_method(
            &mut self,
            _: &mut Emulator,
            call: &JniCall<'_>,
        ) -> Option<f64> {
            assert_eq!(call.class, "example/Math");
            assert_eq!(
                call.args,
                [
                    JniValue::Int(-7),
                    self.array.into(),
                    JniValue::Float(3.5),
                    JniValue::Long(-2)
                ]
            );
            Some(9.5)
        }
    }
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(arch)?;
        let p = emu.pointer_size();
        let class = emu.class("example/Math");
        let array = emu.new_byte_array(&[1, 2]);
        let name = emu.new_cstring("calculate")?;
        let sig = emu.new_cstring("(I[BFJ)D")?;
        let table = emu.read_pointer(emu.jni.env)?;
        let id_function = emu.read_pointer(table + 113 * p)?;
        let id = emu.call_function(id_function, &[emu.jni.env, class.0, name, sig])?;
        let slots = emu.malloc(32)?;
        for (i, bits) in [
            (-7i32) as u64,
            array.0,
            3.5f32.to_bits() as u64,
            (-2i64) as u64,
        ]
        .iter()
        .enumerate()
        {
            emu.write_u64(slots + i as u64 * 8, *bits)?;
        }
        let va = if arch == crate::Architecture::Arm32 {
            let va = emu.malloc(24)?;
            emu.write_pointer(va, (-7i32) as u64)?;
            emu.write_pointer(va + 4, array.0)?;
            emu.write_u64(va + 8, 3.5f64.to_bits())?;
            emu.write_u64(va + 16, (-2i64) as u64)?;
            va
        } else {
            let saved = emu.malloc(40)?;
            emu.write_u64(saved, (-7i32) as u64)?;
            emu.write_u64(saved + 8, array.0)?;
            emu.write_u64(saved + 16, (-2i64) as u64)?;
            emu.write_u64(saved + 24, 3.5f64.to_bits())?;
            let va = emu.malloc(32)?;
            emu.write_u64(va, 0)?;
            emu.write_u64(va + 8, saved + 24)?;
            emu.write_u64(va + 16, saved + 40)?;
            emu.write_bytes(va + 24, &(-24i32).to_le_bytes())?;
            emu.write_bytes(va + 28, &(-16i32).to_le_bytes())?;
            va
        };
        for variant in 0..3 {
            let function = emu.read_pointer(table + (138 + variant) * p)?;
            let mut args = vec![
                JniValue::Object(Handle(emu.jni.env)),
                class.into(),
                JniValue::Object(Handle(id)),
            ];
            if variant == 0 {
                args.extend([
                    (-7).into(),
                    array.into(),
                    JniValue::Double(3.5),
                    JniValue::Long(-2),
                ]);
            } else {
                args.push(JniValue::Object(Handle(if variant == 1 {
                    va
                } else {
                    slots
                })));
            }
            assert_eq!(
                emu.invoke_jni(function, &args, b'D', &mut Handler { array })?,
                JniValue::Double(9.5)
            );
        }
    }
    Ok(())
}

#[test]
fn signature_counts_arrays_and_classes_as_one_argument() {
    assert_eq!(argument_types("([B[[ILjava/lang/String;JFD)V"), b"[[LJFD");
    assert_eq!(decode(b'B', 255), JniValue::Byte(-1));
    assert_eq!(decode(b'I', 0xffff_ffff), JniValue::Int(-1));
}
#[test]
fn rust_jni_handler_can_call_guest_code_without_clobbering_the_outer_call() -> Result<()> {
    struct Handler {
        strlen: u64,
        text: u64,
        fields: usize,
    }
    impl JniHandler for Handler {
        fn get_static_object_field(
            &mut self,
            emu: &mut Emulator,
            call: &JniCall<'_>,
        ) -> Option<Handle> {
            if call.name != "TYPE" || call.signature != "Ljava/lang/Class;" {
                return None;
            }
            let name = match call.class {
                "java/lang/Void" => "void",
                "java/lang/Boolean" => "boolean",
                "java/lang/Byte" => "byte",
                "java/lang/Character" => "char",
                "java/lang/Short" => "short",
                "java/lang/Integer" => "int",
                "java/lang/Long" => "long",
                "java/lang/Float" => "float",
                "java/lang/Double" => "double",
                _ => return None,
            };
            assert_eq!(emu.call_function(self.strlen, &[self.text]).unwrap(), 6);
            self.fields += 1;
            Some(emu.class(name))
        }
        fn call_static_object_method(
            &mut self,
            emu: &mut Emulator,
            call: &JniCall<'_>,
        ) -> Option<Handle> {
            if (call.class, call.name, call.signature)
                == (
                    "java/lang/System",
                    "getProperty",
                    "(Ljava/lang/String;)Ljava/lang/String;",
                )
                && emu.string(call.args[0].object()).ok()? == "file.encoding"
            {
                Some(emu.new_string("UTF-8"))
            } else {
                None
            }
        }
    }
    for arch in ["arm64-v8a", "armeabi-v7a"] {
        let mut emu = Emulator::builder().build()?;
        let module = emu.load_library(format!("tests/fixtures/{arch}/libjnidispatch.so"))?;
        let mut handler = Handler {
            strlen: emu.find_symbol(module, "strlen")?,
            text: emu.new_cstring("nested")?,
            fields: 0,
        };
        assert_eq!(
            emu.call_jni_onload(module, &mut handler)?,
            0x10004,
            "{arch}"
        );
        assert_eq!(handler.fields, 9, "{arch}");
    }
    Ok(())
}
