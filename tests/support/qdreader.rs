use minidbg::{Emulator, Result};
use std::path::Path;

pub fn run(emu: &mut Emulator, directory: &Path) -> Result<String> {
    emu.symbol_alias("DES_set_key_unchecked", "DES_set_key");
    emu.load_library(directory.join("libd-lib.so"))?;
    let input = emu.new_string("359250054370919||1551086094");
    let output = emu
        .call_static_jni_method(&mut (), "a/d", "c(Ljava/lang/String;)[B", &[input.into()])?
        .object();
    let hex = emu
        .byte_array(output)?
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    emu.release(input);
    emu.release(output);
    Ok(hex)
}
