#[path = "support/jna.rs"]
mod sample;
use unidroid::{Emulator, Result};
#[test]
fn jna32_matches_unidbg() -> Result<()> {
    let mut emu = Emulator::builder()
        .trace(unidroid::Trace {
            jni: std::env::var_os("UNIDROID_TRACE").is_some(),
            ..Default::default()
        })
        .build()?;
    let out = sample::run(&mut emu, std::path::Path::new("tests/fixtures/armeabi-v7a"))?;
    assert_eq!(out.sdk, "23");
    assert!(out.malloc_nonzero);
    assert_eq!(out.version, "4.0.1");
    assert_eq!(out.checksum, "1a6047467b59e8748f975e03016ce3d9");
    assert_eq!(out.size, 4);
    Ok(())
}
