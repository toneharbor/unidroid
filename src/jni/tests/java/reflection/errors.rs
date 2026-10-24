use super::*;

#[test]
fn reflection_conversions_reject_wrong_ids_classes_and_objects() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let class = emu.class("example/Owner");
        let method = get_id(&mut emu, 33, class, "run", "()V")?;
        let constructor = get_id(&mut emu, 33, class, "<init>", "()V")?;
        let field = get_id(&mut emu, 94, class, "count", "I")?;
        let text = emu.new_string("wrong kind");
        for (index, function, wrong_id) in [
            (9, "ToReflectedMethod", field),
            (12, "ToReflectedField", method),
            (12, "ToReflectedField", constructor),
        ] {
            for invalid in [wrong_id, 0, text.0, 0xffff_fffe] {
                let error = jni_call(&mut emu, index, &[class.0, invalid, 0]).unwrap_err();
                assert_eq!(
                    error.to_string(),
                    format!("invalid JNI id 0x{invalid:x} in {function} from 0x20000")
                );
            }
            for invalid in [0, 0x123456, text.0] {
                let id = if index == 9 { method } else { field };
                let error = jni_call(&mut emu, index, &[invalid, id, 0]).unwrap_err();
                assert_eq!(
                    error.to_string(),
                    format!("invalid JNI reference 0x{invalid:x} in {function} from 0x20000")
                );
            }
        }
        let reflected_method = jni_call(&mut emu, 9, &[class.0, method, 0])?;
        let reflected_constructor = jni_call(&mut emu, 9, &[class.0, constructor, 0])?;
        let reflected_field = jni_call(&mut emu, 12, &[class.0, field, 0])?;
        let mut bad_objects = vec![
            (0, "null"),
            (0x123456, "invalid reference"),
            (text.0, "java/lang/String"),
            (class.0, "java/lang/Class"),
        ];
        for java_class in [
            "java/lang/reflect/Method",
            "java/lang/reflect/Constructor",
            "java/lang/reflect/Field",
            "example/Owner",
        ] {
            bad_objects.push((emu.new_object(java_class, ()).0, java_class));
        }
        for (index, function) in [(7, "FromReflectedMethod"), (8, "FromReflectedField")] {
            let wrong_kinds = if index == 7 {
                vec![(reflected_field, "java/lang/reflect/Field")]
            } else {
                vec![
                    (reflected_method, "java/lang/reflect/Method"),
                    (reflected_constructor, "java/lang/reflect/Constructor"),
                ]
            };
            for &(object, java_class) in bad_objects.iter().chain(&wrong_kinds) {
                let error = jni_call(&mut emu, index, &[object]).unwrap_err();
                assert_eq!(
                    error.to_string(),
                    format!(
                        "invalid reflected member 0x{object:x}: {java_class} in {function} from 0x20000"
                    )
                );
            }
        }
    }
    Ok(())
}
