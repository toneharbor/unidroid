#[path = "support/ttencrypt.rs"]
mod sample;
use unidroid::{Emulator, Result};

#[test]
fn ttencrypt_matches_unidbg() -> Result<()> {
    let mut emu = Emulator::builder()
        .trace(unidroid::Trace {
            jni: std::env::var_os("UNIDROID_TRACE").is_some(),
            ..Default::default()
        })
        .build()?;
    let output = sample::run(&mut emu, std::path::Path::new("tests/fixtures/armeabi-v7a"))?;
    assert_eq!(output, "7463030000019fd0866aa0cbd0323933d2d2fc8c20ec");
    Ok(())
}
