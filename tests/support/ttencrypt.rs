use minidbg::{Emulator, Result};
use std::path::Path;

pub fn run(emu: &mut Emulator, directory: &Path) -> Result<String> {
    emu.load_library(directory.join("libttEncrypt.so"))?;
    let input = emu.new_byte_array(&[0; 16]);
    let output = emu
        .call_static_jni_method(
            &mut (),
            "com/bytedance/frameworks/core/encrypt/TTEncryptUtils",
            "ttEncrypt([BI)[B",
            &[input.into(), 16.into()],
        )?
        .object();
    let hex = emu
        .byte_array(output)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    emu.release(input);
    emu.release(output);
    Ok(hex)
}
