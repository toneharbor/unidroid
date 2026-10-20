use minidbg::{Emulator, Handle, JniCall, JniHandler, Result};
use std::path::Path;

struct SignHandler {
    entries: [(Handle, Handle); 2],
    key_set: Handle,
    keys: Handle,
}
impl JniHandler for SignHandler {
    fn call_int_method(&mut self, _: &mut Emulator, call: &JniCall<'_>) -> Option<i32> {
        match (call.class, call.name, call.signature) {
            ("java/util/HashMap", "size", "()I") => Some(self.entries.len() as i32),
            _ => None,
        }
    }

    fn call_object_method(&mut self, emu: &mut Emulator, call: &JniCall<'_>) -> Option<Handle> {
        match (call.class, call.name, call.signature) {
            ("java/util/HashMap", "keySet", "()Ljava/util/Set;") => Some(self.key_set),
            ("java/util/Set", "toArray", "()[Ljava/lang/Object;") => Some(self.keys),
            ("java/util/HashMap", "get", "(Ljava/lang/Object;)Ljava/lang/Object;") => {
                let key = emu.string(call.args[0].object()).ok()?;
                Some(
                    self.entries
                        .iter()
                        .find(|(k, _)| emu.string(*k).ok().as_ref() == Some(&key))
                        .map_or(Handle(0), |(_, value)| *value),
                )
            }
            _ => None,
        }
    }
}

pub fn run(emu: &mut Emulator, directory: &Path) -> Result<String> {
    let module = emu.load_library(directory.join("libsignutil.so"))?;
    let entries = [
        (emu.new_string("a"), emu.new_byte_array(b"b")),
        (emu.new_string("b"), emu.new_byte_array(b"b")),
    ];
    let keys = emu.new_object_array("java/lang/Object", &[entries[0].0, entries[1].0])?;
    let key_set = emu.new_object("java/util/Set", ());
    let mut handler = SignHandler {
        entries,
        key_set,
        keys,
    };
    emu.call_jni_onload(module, &mut handler)?;
    let aa = emu.new_string("aa");
    let bb = emu.new_string("bb");
    let map = emu.new_object("java/util/HashMap", ());
    let cc = emu.new_string("cc");
    let output = emu.call_static_jni_method(
        &mut handler,
        "com/anjuke/mobile/sign/SignUtil",
        "getSign0(Ljava/lang/String;Ljava/lang/String;Ljava/util/Map;Ljava/lang/String;I)Ljava/lang/String;",
        &[aa.into(), bb.into(), map.into(), cc.into(), 10.into()],
    )?.object();
    let sign = emu.string(output)?;
    for handle in [aa, bb, map, cc, output, keys, key_set] {
        emu.release(handle);
    }
    for (key, value) in entries {
        emu.release(key);
        emu.release(value);
    }
    Ok(sign)
}
