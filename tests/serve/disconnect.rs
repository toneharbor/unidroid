use super::*;
use std::io::{self, Cursor, Read};
use unidroid::{Emulator, Result};

struct FailedRead;
impl Read for FailedRead {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("client read failed"))
    }
}
struct Output<'a> {
    bytes: &'a mut Vec<u8>,
    failure: &'a str,
}
impl Write for Output<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.failure == "write" {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "client write failed",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.failure == "flush" {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "client flush failed",
            ))
        } else {
            Ok(())
        }
    }
}

fn disconnect(emu: &mut Emulator, command: Value, failure: &str, question: &str) -> Result<()> {
    let command = Cursor::new(format!("{command}\n").into_bytes());
    let input: Box<dyn BufRead> = if failure == "read" {
        Box::new(BufReader::new(command.chain(FailedRead)))
    } else {
        Box::new(command)
    };
    let mut bytes = Vec::new();
    let error = unidroid::serve::serve(
        emu,
        None,
        input,
        Output {
            bytes: &mut bytes,
            failure,
        },
    )
    .unwrap_err()
    .to_string();
    let expected = match failure {
        "eof" => match question {
            "import" => "stdin closed while an import question was waiting for an answer",
            "jni" => "stdin closed while a JNI question was waiting for an answer",
            _ => "stdin closed while a breakpoint stop was waiting for a resume command",
        },
        "read" => "client read failed",
        "write" => "client write failed",
        _ => "client flush failed",
    };
    assert_eq!(error, expected);
    if failure != "write" {
        let response: Value = serde_json::from_slice(&bytes)?;
        assert!(response.get(question).is_some(), "{response}");
    }
    Ok(())
}

#[test]
fn disconnected_import_stops_before_original_and_caller_resume() -> Result<()> {
    for arm64 in [true, false] {
        for failure in ["eof", "read", "write", "flush"] {
            let library = elf::TestLibrary::new(arm64, true);
            let mut emu = Emulator::builder().build()?;
            let module = emu.load_library_without_constructors(&library.0)?;
            let base = emu.modules()[module.0].base;
            let data = emu.find_symbol(module, "data")?;
            let original = emu.find_symbol(module, "answer")?;
            // Original and code after the hook both call the constructor, which writes data.
            let branch = if arm64 { 0x14000008u32 } else { 0xea000006 };
            emu.write_bytes(original, &branch.to_le_bytes())?;
            let code = if arm64 {
                vec![
                    0xa9bf7bfeu32,
                    0xd63f0000,
                    0x97ffffe6,
                    0xa8c17bfe,
                    0xd65f03c0,
                ]
            } else {
                vec![0xe92d4010, 0xe12fff30, 0xebffffe4, 0xe8bd8010]
            };
            emu.write_bytes(
                base + 0x280,
                &code
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
            )?;
            emu.hook_import("answer")?;
            let hook = emu.find_symbol(module, "answer")?;
            disconnect(
                &mut emu,
                json!({"cmd":"call_function", "address":(base + 0x280).to_string(), "args":[hook.to_string()]}),
                failure,
                "import",
            )?;
            assert_eq!(
                emu.read_bytes(data, 8)?,
                [0; 8],
                "{failure}: guest continued"
            );
            // The aborted call leaves no pending failure in the emulator.
            assert_eq!(emu.call_function(original, &[])?, 42);
            assert_eq!(emu.read_bytes(data, 8)?, 42u64.to_le_bytes());
        }
    }
    Ok(())
}

#[test]
fn disconnected_jni_and_stop_questions_return_the_session_error() -> Result<()> {
    for (arm64, abi) in [(true, "arm64-v8a"), (false, "armeabi-v7a")] {
        for failure in ["eof", "read", "write", "flush"] {
            let mut emu = Emulator::builder().build()?;
            let module = emu.load_library(format!("tests/fixtures/{abi}/libjnidispatch.so"))?;
            disconnect(
                &mut emu,
                json!({"cmd":"onload", "module":module.0}),
                failure,
                "jni",
            )?;
            let library = elf::TestLibrary::new(arm64, false);
            let mut emu = Emulator::builder().build()?;
            let module = emu.load_library_without_constructors(&library.0)?;
            let base = emu.modules()[module.0].base;
            emu.add_breakpoint(base + 0x220)?;
            disconnect(
                &mut emu,
                json!({"cmd":"run_constructors", "module":module.0}),
                failure,
                "break",
            )?;
            assert_eq!(emu.read_bytes(base + 0x1600, 8)?, [0; 8]);
        }
    }
    Ok(())
}
