use minidbg::{Emulator, Result};
use std::path::Path;

pub fn run(emu: &mut Emulator, directory: &Path) -> Result<String> {
    emu.load_library(directory.join("libthread-lib.so"))?;
    let result = emu.call_static_jni_method(
        &mut (),
        "com/mpt/jnithread/MainActivity",
        "stringFromJNI()Ljava/lang/String;",
        &[],
    )?;
    let handle = result.object();
    let output = emu.string(handle).to_owned();
    emu.release(handle);
    Ok(output)
}
