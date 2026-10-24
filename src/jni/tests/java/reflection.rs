use super::*;

mod answers;
mod equality;
mod errors;
mod host;
mod lifecycle;

fn get_id(
    emu: &mut Emulator,
    index: u64,
    class: Handle,
    name: &str,
    signature: &str,
) -> Result<u64> {
    let name = emu.new_cstring(name)?;
    let signature = emu.new_cstring(signature)?;
    let result = jni_call(emu, index, &[class.0, name, signature]);
    emu.free(name);
    emu.free(signature);
    result
}

#[test]
fn reflected_members_round_trip_ids_and_have_java_classes() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let class = emu.class("example/Owner");
        for (lookup, name, signature, to, from, java_class) in [
            (33, "run", "()V", 9, 7, "java/lang/reflect/Method"),
            (113, "run", "()V", 9, 7, "java/lang/reflect/Method"),
            (33, "<init>", "()V", 9, 7, "java/lang/reflect/Constructor"),
            (94, "count", "I", 12, 8, "java/lang/reflect/Field"),
            (144, "count", "I", 12, 8, "java/lang/reflect/Field"),
        ] {
            let id = get_id(&mut emu, lookup, class, name, signature)?;
            let args = [class.0, id, u64::from(lookup >= 113)];
            let reflected = jni_call(&mut emu, to, &args)?;
            assert_eq!(jni_call(&mut emu, from, &[reflected])?, id);
            let actual_class = jni_call(&mut emu, 31, &[reflected])?;
            let expected_class = emu.class(java_class);
            assert_eq!(
                jni_call(&mut emu, 24, &[actual_class, expected_class.0])?,
                1
            );
            let another = jni_call(&mut emu, to, &args)?;
            assert_ne!(reflected, another);
            jni_call(&mut emu, 23, &[reflected])?;
            assert_eq!(jni_call(&mut emu, from, &[another])?, id);
        }
    }
    Ok(())
}
