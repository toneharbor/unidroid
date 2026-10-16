#[path = "../tests/support/jna.rs"]
mod jna;
#[path = "../tests/support/tmessages.rs"]
mod tmessages;
use minidbg::{Emulator, Result};
use std::path::Path;

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let sample = &args[1];
    let directory = Path::new(&args[2]);
    let instances: usize = args.get(3).map_or(Ok(1), |n| n.parse())?;
    let mut emulators = Vec::new();
    for i in 0..instances {
        let mut builder = Emulator::builder().process_name("baseline");
        if let Some(size) = std::env::var_os("MINIDBG_CACHE_SIZE") {
            builder = builder.translation_cache_size(size.to_string_lossy().parse()?);
        }
        let mut emu = builder.build()?;
        if sample == "jna" {
            let out = jna::run(&mut emu, directory)?;
            if i + 1 == instances {
                println!("sdk={}", out.sdk);
                println!("malloc_nonzero={}", out.malloc_nonzero);
                println!("getNativeVersion={}", out.version);
                println!("getAPIChecksum={}", out.checksum);
                println!("sizeof(0)={}", out.size);
            }
        } else {
            let out = tmessages::run(&mut emu, directory)?;
            if i + 1 == instances {
                println!("aesCbc={}", out.cbc);
                println!("aesCtr={}", out.ctr);
                println!("pbkdf2={}", out.pbkdf2);
                println!("pbkdf2_ms={}", out.pbkdf2_ms);
            }
        }
        emulators.push(emu);
    }
    println!("instances={}", emulators.len());
    Ok(())
}
