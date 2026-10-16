#[path = "support/jna.rs"]
mod sample;
use minidbg::{Emulator, Result};
#[test]
fn jna_matches_unidbg() -> Result<()> {
    let mut emu = Emulator::builder()
        .verbose(std::env::var_os("MINIDBG_TRACE").is_some())
        .build()?;
    let out = sample::run(&mut emu, std::path::Path::new("tests/fixtures/arm64-v8a"))?;
    assert_eq!(out.sdk, "23");
    assert!(out.malloc_nonzero);
    assert_eq!(out.version, "4.0.1");
    assert_eq!(out.checksum, "1a6047467b59e8748f975e03016ce3d9");
    assert_eq!(out.size, 8);
    Ok(())
}
