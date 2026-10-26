use super::*;

#[test]
fn malformed_guest_signatures_report_the_jni_function_and_location() -> Result<()> {
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(arch)?;
        let class = emu.class("app/BadSignature");
        let receiver = emu.new_object("app/BadSignature", ());
        let name = emu.new_cstring("value")?;
        for signature in ["([", "(I", "(Ljava/lang/String)V", "()["] {
            let sig = emu.new_cstring(signature)?;
            let id = jni_call(&mut emu, 33, &[class.0, name, sig])?;
            let table = emu.read_pointer(emu.jni.env)?;
            let function = emu.read_pointer(table + 51 * emu.pointer_size())?;
            let error = emu
                .call_function_with(
                    &mut crate::Probe::default(),
                    function,
                    &[emu.jni.env, receiver.0, id, 0],
                )
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("invalid JNI signature {signature} in CallIntMethodA from 0x20000")
            );
        }
    }
    Ok(())
}
