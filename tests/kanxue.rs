#[path = "support/kanxue.rs"]
mod sample;
use minidbg::{Emulator, Result};

#[test]
fn kanxue_matches_unidbg() -> Result<()> {
    let mut emu = Emulator::builder()
        .verbose(std::env::var_os("MINIDBG_TRACE").is_some())
        .build()?;
    let output = sample::run(&mut emu, std::path::Path::new("tests/fixtures/armeabi-v7a"))?;
    assert_eq!(output.found, "XuE");
    assert_eq!(output.tries, 64589);
    Ok(())
}
