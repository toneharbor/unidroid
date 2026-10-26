use clap::{Args, Parser, Subcommand, ValueEnum};
use std::{fs, path::PathBuf, process::ExitCode};
use unidroid::{Crash, Emulator, JniHandler, Probe, Result, Snapshot, Trace};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Compare two memory snapshots.
    Diff { before: PathBuf, after: PathBuf },
    /// Serve JSON Lines commands over stdin and stdout.
    Serve {
        #[command(flatten)]
        options: Options,
        /// Record the session as flushed JSON Lines.
        #[arg(long, value_name = "FILE")]
        record: Option<PathBuf>,
    },
    /// Run independent JSON Lines jobs and print a JSON report.
    Batch {
        #[command(flatten)]
        options: unidroid::serve::Options,
        #[command(flatten)]
        batch: unidroid::serve::BatchOptions,
        #[command(flatten)]
        logging: LogOptions,
        jobs: PathBuf,
    },
    /// Identify which input fields change which output bytes.
    Fields {
        #[command(flatten)]
        options: unidroid::serve::Options,
        #[command(flatten)]
        fields: unidroid::serve::FieldsOptions,
        #[command(flatten)]
        logging: LogOptions,
        spec: PathBuf,
    },
    /// Replay a recorded serve session and compare every response.
    Replay {
        file: PathBuf,
        #[command(flatten)]
        diagnostics: Diagnostics,
    },
    /// Inspect a library and run JNI_OnLoad.
    Inspect {
        #[command(flatten)]
        options: Options,
        /// Answer Java calls with defaults and list each distinct call.
        #[arg(long)]
        probe: bool,
        lib: PathBuf,
    },
}

#[derive(Clone, Copy, Default, ValueEnum)]
enum LogFormat {
    #[default]
    Text,
    Json,
}

#[derive(Args)]
#[group(id = "RunOptions")]
struct Options {
    #[command(flatten)]
    emulator: unidroid::serve::Options,
    #[command(flatten)]
    diagnostics: Diagnostics,
}

#[derive(Args)]
struct Diagnostics {
    /// Trace JNI calls to stderr.
    #[arg(long)]
    verbose: bool,
    /// Trace guest syscalls to stderr.
    #[arg(long)]
    trace_syscalls: bool,
    /// Trace app calls into other libraries to stderr.
    #[arg(long)]
    trace_imports: bool,
    /// Trace instructions in a module or module+0xSTART-0xEND.
    #[arg(long, value_name = "SPEC")]
    trace_code: Vec<String>,
    /// Trace function calls from a module or module+0xSTART-0xEND.
    #[arg(long, value_name = "SPEC")]
    trace_calls: Vec<String>,
    #[command(flatten)]
    logging: LogOptions,
}

#[derive(Args)]
struct LogOptions {
    /// Format diagnostic events on stderr.
    #[arg(long, value_enum, default_value = "text")]
    log_format: LogFormat,
}

impl Diagnostics {
    fn builder(self, inspect: bool) -> unidroid::EmulatorBuilder {
        self.logging.log_format.builder().trace(Trace {
            jni: inspect || self.verbose,
            syscalls: self.trace_syscalls,
            imports: self.trace_imports,
            heap: false,
            code: self.trace_code,
            calls: self.trace_calls,
            coverage: Vec::new(),
        })
    }
}

impl LogFormat {
    fn builder(self) -> unidroid::EmulatorBuilder {
        let mut builder = Emulator::builder();
        if matches!(self, LogFormat::Json) {
            builder = builder.on_event(|event| {
                use std::io::Write;
                let mut stderr = std::io::stderr().lock();
                let _ = serde_json::to_writer(&mut stderr, event);
                let _ = writeln!(stderr);
            });
        }
        builder
    }
}

fn inspect(options: Options, lib: PathBuf, probe: bool) -> Result<ExitCode> {
    let mut emu = options.emulator.build(options.diagnostics.builder(true))?;
    let module = if options.emulator.apk.is_some() {
        emu.load_apk_library_without_constructors(&lib.to_string_lossy())?
    } else {
        emu.load_library_without_constructors(&lib)?
    };
    let info = emu.library_info(module)?;
    let name = lib.file_name().unwrap().to_string_lossy();
    let architecture = if info.arm64 {
        "64-bit ARM (arm64-v8a)"
    } else {
        "32-bit ARM (armeabi-v7a)"
    };
    println!("{name}: {architecture}");
    println!(
        "Needs: {}",
        if info.needed.is_empty() {
            "(nothing)".into()
        } else {
            info.needed.join(", ")
        }
    );
    println!("Java_ exports:");
    let mut any = false;
    for name in info.exports.iter().filter(|name| name.starts_with("Java_")) {
        println!("  {name}");
        any = true;
    }
    if !any {
        println!("  (none)");
    }
    let onload = info.exports.iter().any(|name| name == "JNI_OnLoad");
    println!("JNI_OnLoad: {}", if onload { "yes" } else { "no" });
    let mut policy = options.emulator.load_policy()?;
    emu.run_constructors(module, &mut ())?;
    let mut status = ExitCode::SUCCESS;
    let mut answers = Probe::default();
    if onload {
        let handler: &mut dyn JniHandler = if probe { &mut answers } else { &mut () };
        let result = if let Some(policy) = &mut policy {
            let mut handler = policy.handler(handler);
            let result = emu.call_jni_onload(module, &mut handler);
            handler.finish(result)
        } else {
            emu.call_jni_onload(module, handler)
        };
        match result {
            Ok(value) => println!("JNI_OnLoad returned 0x{value:x}"),
            Err(error) => {
                println!("JNI_OnLoad stopped: {error}");
                if let Some(crash) = error.downcast_ref::<Crash>() {
                    println!("{}", crash.report());
                }
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
    println!("Crypto constants:");
    let constants = emu.crypto_constants()?;
    if constants.is_empty() {
        println!("  (none)");
    }
    for constant in constants {
        println!("  {} at {}", constant.name, constant.location);
    }
    if probe {
        println!("Probed calls:");
        if answers.calls().is_empty() {
            println!("  (none)");
        }
        for call in answers.calls() {
            println!(
                "  {}{} {}->{}{} from {} (count {})",
                if call.is_static { "static " } else { "" },
                call.op,
                call.class,
                call.name,
                call.signature,
                call.from,
                call.count
            );
        }
    }
    Ok(status)
}

fn run() -> Result<ExitCode> {
    match Cli::parse().command {
        Command::Diff { before, after } => {
            println!(
                "{}",
                Snapshot::load(before)?
                    .diff(&Snapshot::load(after)?)
                    .report()
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Serve { options, record } => {
            let builder = options.diagnostics.builder(false);
            if let Some(path) = record {
                unidroid::serve::record(
                    &options.emulator,
                    builder,
                    fs::File::create(path)?,
                    std::io::stdin().lock(),
                    std::io::stdout().lock(),
                )?;
            } else {
                let mut emu = options.emulator.build(builder)?;
                unidroid::serve::serve(
                    &mut emu,
                    options.emulator.load_policy()?,
                    std::io::stdin().lock(),
                    std::io::stdout().lock(),
                )?;
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Batch {
            options,
            batch,
            logging,
            jobs,
        } => {
            let report = unidroid::serve::batch(
                &options,
                &batch,
                || logging.log_format.builder(),
                std::io::BufReader::new(fs::File::open(jobs)?),
            )?;
            println!("{report}");
            Ok(if report["summary"]["failed"] == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Command::Fields {
            options,
            fields,
            logging,
            spec,
        } => {
            let report = unidroid::serve::fields(
                &options,
                &fields,
                || logging.log_format.builder(),
                serde_json::from_reader(fs::File::open(spec)?)?,
            )?;
            println!("{report}");
            Ok(ExitCode::SUCCESS)
        }
        Command::Replay { file, diagnostics } => {
            let result = fs::File::open(file)
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
                .and_then(|file| {
                    unidroid::serve::replay(
                        std::io::BufReader::new(file),
                        diagnostics.builder(false),
                    )
                });
            match result {
                Ok(count) => {
                    println!("replayed {count} commands, all responses match");
                    Ok(ExitCode::SUCCESS)
                }
                Err(error) => {
                    println!("{error}");
                    Ok(ExitCode::FAILURE)
                }
            }
        }
        Command::Inspect {
            options,
            lib,
            probe,
        } => inspect(options, lib, probe),
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
