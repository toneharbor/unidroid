use clap::{Args, Parser, Subcommand};
use goblin::elf::{Elf, sym};
use std::{fs, path::PathBuf, process::ExitCode};
use unidroid::{Emulator, Result};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serve JSON Lines commands over stdin and stdout.
    Serve {
        #[command(flatten)]
        options: Options,
    },
    /// Inspect a library and run JNI_OnLoad.
    Inspect {
        #[command(flatten)]
        options: Options,
        lib: PathBuf,
    },
}

#[derive(Args)]
struct Options {
    /// Android system library directory.
    #[arg(long, default_value = "rootfs")]
    rootfs: PathBuf,
    /// Guest process name.
    #[arg(long)]
    process_name: Option<String>,
    /// Translation cache size in MB (default 8). Large libraries run faster with a bigger cache.
    #[arg(
        long,
        value_name = "MB",
        value_parser = clap::value_parser!(u32).range(1..4096),
        verbatim_doc_comment
    )]
    translation_cache_mb: Option<u32>,
    /// Set a system property.
    #[arg(long = "prop", value_name = "KEY=VALUE", value_parser = pair)]
    properties: Vec<(String, String)>,
    /// Resolve a symbol under another name.
    #[arg(long, value_name = "NAME=TARGET", value_parser = pair)]
    alias: Vec<(String, String)>,
    /// Declare that CLASS extends SUPERCLASS.
    #[arg(long, value_name = "CLASS=SUPERCLASS", value_parser = pair)]
    superclass: Vec<(String, String)>,
    /// Trace JNI calls to stderr.
    #[arg(long)]
    verbose: bool,
}

fn pair(value: &str) -> std::result::Result<(String, String), String> {
    value
        .split_once('=')
        .map(|(key, value)| (key.into(), value.into()))
        .ok_or_else(|| "expected KEY=VALUE".into())
}

impl Options {
    fn build(self, inspect: bool) -> Result<Emulator> {
        let mut builder = Emulator::builder()
            .rootfs(self.rootfs)
            .verbose(inspect || self.verbose);
        if let Some(name) = self.process_name {
            builder = builder.process_name(name);
        }
        if let Some(mb) = self.translation_cache_mb {
            builder = builder.translation_cache_size(mb * 1024 * 1024);
        }
        for (key, value) in self.properties {
            builder = builder.system_property(key, value);
        }
        let mut emu = builder.build()?;
        for (name, target) in self.alias {
            emu.symbol_alias(name, target);
        }
        for (class, superclass) in self.superclass {
            emu.superclass(class, superclass);
        }
        Ok(emu)
    }
}

fn inspect(options: Options, lib: PathBuf) -> Result<ExitCode> {
    let bytes = fs::read(&lib)?;
    let elf = Elf::parse(&bytes)?;
    let name = lib.file_name().unwrap().to_string_lossy();
    let architecture = match (elf.header.e_machine, elf.is_64) {
        (183, true) => "64-bit ARM (arm64-v8a)",
        (40, false) => "32-bit ARM (armeabi-v7a)",
        _ => return Err(format!("{name}: not an ARM library").into()),
    };
    println!("{name}: {architecture}");
    println!(
        "Needs: {}",
        if elf.libraries.is_empty() {
            "(nothing)".into()
        } else {
            elf.libraries.join(", ")
        }
    );
    println!("Java_ exports:");
    let exports: Vec<_> = elf
        .dynsyms
        .iter()
        .filter(|symbol| {
            symbol.st_shndx != 0 && matches!(symbol.st_bind(), sym::STB_GLOBAL | sym::STB_WEAK)
        })
        .filter_map(|symbol| elf.dynstrtab.get_at(symbol.st_name))
        .collect();
    let mut any = false;
    for name in exports.iter().filter(|name| name.starts_with("Java_")) {
        println!("  {name}");
        any = true;
    }
    if !any {
        println!("  (none)");
    }
    let onload = exports.contains(&"JNI_OnLoad");
    println!("JNI_OnLoad: {}", if onload { "yes" } else { "no" });
    let mut emu = options.build(true)?;
    let module = emu.load_library(lib)?;
    let mut status = ExitCode::SUCCESS;
    if onload {
        match emu.call_jni_onload(module, &mut ()) {
            Ok(value) => println!("JNI_OnLoad returned 0x{value:x}"),
            Err(error) => {
                println!("JNI_OnLoad stopped: {error}");
                status = ExitCode::FAILURE;
            }
        }
    }
    println!("Registered natives:");
    let natives = emu.registered_natives();
    if natives.is_empty() {
        println!("  (none)");
    }
    for (class, method, address) in natives {
        println!("  {class}.{method} at {}", emu.location(address));
    }
    Ok(status)
}

fn run() -> Result<ExitCode> {
    match Cli::parse().command {
        Command::Serve { options } => {
            let mut emu = options.build(false)?;
            unidroid::serve::serve(&mut emu, std::io::stdin().lock(), std::io::stdout().lock())?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Inspect { options, lib } => inspect(options, lib),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(status) => status,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
