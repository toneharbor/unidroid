use std::path::Path;
use unidroid::{Emulator, Result};

pub fn run(emu: &mut Emulator, directory: &Path) -> Result<String> {
    emu.load_library(directory.join("libthread-lib.so"))?;
    let result = emu.call_static_jni_method(
        &mut (),
        "com/mpt/jnithread/MainActivity",
        "stringFromJNI()Ljava/lang/String;",
        &[],
    )?;
    let handle = result.object();
    let output = emu.string(handle)?;
    emu.release(handle);
    Ok(output)
}
