#[path = "support/signutil.rs"]
mod sample;
use unidroid::{Emulator, Result};

#[test]
fn signutil_matches_unidbg() -> Result<()> {
    let mut emu = Emulator::builder()
        .trace(unidroid::Trace {
            jni: std::env::var_os("UNIDROID_TRACE").is_some(),
            ..Default::default()
        })
        .build()?;
    let output = sample::run(&mut emu, std::path::Path::new("tests/fixtures/armeabi-v7a"))?;
    assert_eq!(output, "1000ed2f7e5ecd00bb4a6454e1d487b7dc7500040002000acc");
    Ok(())
}
