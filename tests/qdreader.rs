#[path = "support/qdreader.rs"]
mod sample;
use unidroid::{Emulator, Result};

#[test]
fn qdreader_matches_unidbg() -> Result<()> {
    let mut emu = Emulator::builder()
        .trace(unidroid::Trace {
            jni: std::env::var_os("UNIDROID_TRACE").is_some(),
            ..Default::default()
        })
        .build()?;
    let output = sample::run(&mut emu, std::path::Path::new("tests/fixtures/armeabi-v7a"))?;
    assert_eq!(
        output,
        "ac34e9132d40e69ddf5359c01fbe305df5688254edce02c0c2336fb65460bd73"
    );
    Ok(())
}
