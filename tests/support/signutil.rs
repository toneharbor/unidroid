use std::path::Path;
use unidroid::{Emulator, Result};

pub fn run(emu: &mut Emulator, directory: &Path) -> Result<String> {
    let module = emu.load_library(directory.join("libsignutil.so"))?;
    let entries = [
        (emu.new_string("a"), emu.new_byte_array(b"b")),
        (emu.new_string("b"), emu.new_byte_array(b"b")),
    ];
    emu.call_jni_onload(module, &mut ())?;
    let aa = emu.new_string("aa");
    let bb = emu.new_string("bb");
    let map = emu.new_java_map("java/util/HashMap", &entries)?;
    let cc = emu.new_string("cc");
    let output = emu.call_static_jni_method(
        &mut (),
        "com/anjuke/mobile/sign/SignUtil",
        "getSign0(Ljava/lang/String;Ljava/lang/String;Ljava/util/Map;Ljava/lang/String;I)Ljava/lang/String;",
        &[aa.into(), bb.into(), map.into(), cc.into(), 10.into()],
    )?.object();
    let sign = emu.string(output)?;
    for handle in [aa, bb, map, cc, output] {
        emu.release(handle);
    }
    for (key, value) in entries {
        emu.release(key);
        emu.release(value);
    }
    Ok(sign)
}
