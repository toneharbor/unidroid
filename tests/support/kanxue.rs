use minidbg::{Emulator, JniValue, Result};
use std::path::Path;

pub struct Output {
    pub found: String,
    pub tries: usize,
}

pub fn run(emu: &mut Emulator, directory: &Path) -> Result<Output> {
    emu.load_library(directory.join("libnative-lib.so"))?;
    let class = "com/kanxue/test2/MainActivity";
    let activity = emu.new_object(class, ());
    let letters = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let mut tries = 0;
    let mut found = String::new();
    'search: for &a in letters {
        for &b in letters {
            for &c in letters {
                let candidate = String::from_utf8(vec![a, b, c])?;
                let input = emu.new_string(&candidate);
                tries += 1;
                let result = emu.call_jni_method(
                    &mut (),
                    class,
                    activity,
                    "jnitest(Ljava/lang/String;)Z",
                    &[input.into()],
                );
                emu.release(input);
                if result? == JniValue::Boolean(true) {
                    found = candidate;
                    break 'search;
                }
            }
        }
    }
    emu.release(activity);
    Ok(Output { found, tries })
}
