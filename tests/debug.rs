use unidroid::{Emulator, Result};

#[test]
fn symbols_cover_only_function_bodies() -> Result<()> {
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let mut emu = Emulator::builder().build()?;
        let module = emu.load_library(format!("tests/fixtures/{abi}/libjnidispatch.so"))?;
        let address = emu.find_symbol(module, "JNI_OnLoad")? & !1;
        assert_eq!(
            emu.symbolize(address),
            format!("{} (JNI_OnLoad)", emu.location(address))
        );
        assert_eq!(
            emu.symbolize(address + 4),
            format!("{} (JNI_OnLoad+0x4)", emu.location(address + 4))
        );
        assert_eq!(emu.symbolize(0x42), "0x42");
    }
    Ok(())
}
