use super::*;

#[test]
fn superclass_uses_builtin_classes_and_interfaces_after_explicit_declarations() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        for (name, parent) in [
            ("java/lang/String", Some("java/lang/Object")),
            ("java/util/HashMap", Some("java/util/AbstractMap")),
            ("java/lang/CharSequence", None),
            ("java/util/List", None),
            ("java/lang/Object", None),
        ] {
            let class = emu.class(name);
            let result = Handle(jni_call(&mut emu, 10, &[class.0])?);
            if let Some(parent) = parent {
                assert_eq!(emu.class_name(result)?, parent);
            } else {
                assert_eq!(result, Handle(0));
            }
        }
        emu.superclass("java/lang/String", "app/Declared");
        let class = emu.class("java/lang/String");
        let result = Handle(jni_call(&mut emu, 10, &[class.0])?);
        assert_eq!(emu.class_name(result)?, "app/Declared");
        let class = emu.class("app/Unknown");
        assert_eq!(
            jni_call(&mut emu, 10, &[class.0]).unwrap_err().to_string(),
            "GetSuperclass app/Unknown from 0x20000: superclass not declared; declare it with Emulator::superclass, --superclass, or the serve command superclass"
        );
    }
    Ok(())
}
