use minidbg::{Emulator, JniHandler, Result};
use std::{path::Path, time::Instant};
pub struct TelegramHandler;
impl JniHandler for TelegramHandler {}

pub struct Output {
    pub cbc: String,
    pub ctr: String,
    pub pbkdf2: String,
    pub pbkdf2_ms: u128,
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn run(emu: &mut Emulator, directory: &Path) -> Result<Output> {
    let module = emu.load_library(directory.join("libtmessages.29.so"))?;
    let mut handler = TelegramHandler;
    emu.call_jni_onload(module, &mut handler)?;
    let class = "org/telegram/messenger/Utilities";
    let data = emu.new_byte_array(&[0; 16]);
    let key = emu.new_byte_array(&[0; 32]);
    let iv = emu.new_byte_array(&[0; 16]);
    emu.call_static_jni_method(
        &mut handler,
        class,
        "aesCbcEncryptionByteArray([B[B[BIIII)V",
        &[
            data.into(),
            key.into(),
            iv.into(),
            0.into(),
            16.into(),
            0.into(),
            0.into(),
        ],
    )?;
    let cbc = hex(&emu.byte_array(data)?);
    let data = emu.new_byte_array(&[0; 16]);
    let iv = emu.new_byte_array(&[0; 16]);
    emu.call_static_jni_method(
        &mut handler,
        class,
        "aesCtrDecryptionByteArray([B[B[BIII)V",
        &[
            data.into(),
            key.into(),
            iv.into(),
            0.into(),
            16.into(),
            0.into(),
        ],
    )?;
    let ctr = hex(&emu.byte_array(data)?);
    let password = emu.new_byte_array(b"123456");
    let salt = emu.new_byte_array(&[0; 8]);
    let dst = emu.new_byte_array(&[0; 64]);
    let start = Instant::now();
    emu.call_static_jni_method(
        &mut handler,
        class,
        "pbkdf2([B[B[BI)V",
        &[password.into(), salt.into(), dst.into(), 100000.into()],
    )?;
    let pbkdf2_ms = start.elapsed().as_millis();
    Ok(Output {
        cbc,
        ctr,
        pbkdf2: hex(&emu.byte_array(dst)?),
        pbkdf2_ms,
    })
}
