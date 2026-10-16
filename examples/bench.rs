#[path = "../tests/support/jna.rs"]
mod jna;
#[path = "../tests/support/kanxue.rs"]
mod kanxue;
#[path = "../tests/support/qdreader.rs"]
mod qdreader;
#[path = "../tests/support/signutil.rs"]
mod signutil;
#[path = "../tests/support/tmessages.rs"]
mod tmessages;
#[path = "../tests/support/ttencrypt.rs"]
mod ttencrypt;
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
        if sample == "jna" || sample == "jna32" {
            let out = jna::run(&mut emu, directory)?;
            if i + 1 == instances {
                println!("sdk={}", out.sdk);
                println!("malloc_nonzero={}", out.malloc_nonzero);
                println!("getNativeVersion={}", out.version);
                println!("getAPIChecksum={}", out.checksum);
                println!("sizeof(0)={}", out.size);
            }
        } else if sample == "ttencrypt" {
            let out = ttencrypt::run(&mut emu, directory)?;
            if i + 1 == instances {
                println!("ttEncrypt={out}");
            }
        } else if sample == "signutil" {
            let out = signutil::run(&mut emu, directory)?;
            if i + 1 == instances {
                println!("sign={out}");
            }
        } else if sample == "qdreader" {
            let out = qdreader::run(&mut emu, directory)?;
            if i + 1 == instances {
                println!("c={out}");
            }
        } else if sample == "kanxue" {
            let out = kanxue::run(&mut emu, directory)?;
            if i + 1 == instances {
                println!("found={}", out.found);
                println!("tries={}", out.tries);
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
