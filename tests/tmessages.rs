#[path = "support/tmessages.rs"]
mod sample;
use unidroid::{Emulator, Result};
#[test]
fn telegram_crypto_matches_unidbg() -> Result<()> {
    let mut emu = Emulator::builder()
        .process_name("org.telegram.messenger")
        .trace(unidroid::Trace {
            jni: std::env::var_os("UNIDROID_TRACE").is_some(),
            ..Default::default()
        })
        .build()?;
    let out = sample::run(&mut emu, std::path::Path::new("tests/fixtures/arm64-v8a"))?;
    assert_eq!(out.cbc, "67671ce1fa91ddeb0f8fbbb366b531b4");
    assert_eq!(out.ctr, "dc95c078a2408989ad48a21492842087");
    assert_eq!(
        out.pbkdf2,
        "7e53e0b788ad3b5cb8990c35d6d65097f4c6580e337ac617a0903568ecb56e2d9b187e14a6a0e71a15840d42ac206d045badb04fbecba7a2852e1937552c3da2"
    );
    let _ = out.pbkdf2_ms;
    Ok(())
}
