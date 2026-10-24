use base64::{Engine, prelude::BASE64_STANDARD};
use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};
use unidroid::{Emulator, Result};

#[test]
fn serve_file_commands_work_before_load_and_report_errors() -> Result<()> {
    let mut emu = Emulator::builder().build()?;
    let input = [
        json!({"cmd":"add_file", "path":"/config/value", "base64":"aGVsbG8="}),
        json!({"cmd":"file", "path":"/config/value"}),
        json!({"cmd":"add_host_file", "path":"/app", "host_path":"rootfs/lib/libdl.so"}),
        json!({"cmd":"file", "path":"/app"}),
        json!({"cmd":"remove_file", "path":"/lib/libdl.so"}),
        json!({"cmd":"file", "path":"/lib/libdl.so"}),
        json!({"cmd":"add_file", "path":"/bad", "base64":"!"}),
        json!({"cmd":"add_host_file", "path":"/missing", "host_path":"tests/fixtures/no-file"}),
        json!({"cmd":"add_file", "path":"relative", "base64":""}),
        json!({"cmd":"remove_file", "path":"relative"}),
        json!({"cmd":"file", "path":"/config"}),
    ]
    .iter()
    .map(|value| format!("{value}\n"))
    .collect::<String>();
    let mut output = Vec::new();
    unidroid::serve::serve(&mut emu, None, input.as_bytes(), &mut output)?;
    let responses = String::from_utf8(output)?
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<Vec<Value>, _>>()?;
    assert_eq!(responses[0], json!({"ok":null}));
    assert_eq!(responses[1], json!({"ok":"aGVsbG8="}));
    assert_eq!(responses[2], json!({"ok":null}));
    assert_eq!(
        responses[3],
        json!({"ok":BASE64_STANDARD.encode(std::fs::read("rootfs/lib/libdl.so")?)})
    );
    assert_eq!(responses[4], json!({"ok":null}));
    assert_eq!(
        responses[5..],
        [
            json!({"error":"/lib/libdl.so: ENOENT"}),
            json!({"error":"Invalid symbol 33, offset 0."}),
            json!({"error":"add_host_file /missing: tests/fixtures/no-file: No such file or directory (os error 2)"}),
            json!({"error":"relative: EINVAL (path must be absolute)"}),
            json!({"error":"relative: EINVAL (path must be absolute)"}),
            json!({"error":"/config: EISDIR"}),
        ]
    );
    Ok(())
}

#[test]
fn repeated_file_options_are_recorded_and_replayed() -> Result<()> {
    let record =
        std::env::temp_dir().join(format!("unidroid-vfs-record-{}.jsonl", std::process::id()));
    let mut child = Command::new(env!("CARGO_BIN_EXE_unidroid"))
        .args([
            "serve",
            "--file",
            "/one=rootfs/lib/libdl.so",
            "--file",
            "/two=rootfs/lib64/libdl.so",
            "--record",
        ])
        .arg(&record)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child.stdin.take().unwrap().write_all(
        b"{\"cmd\":\"file\",\"path\":\"/one\"}\n{\"cmd\":\"file\",\"path\":\"/two\"}\n",
    )?;
    let output = child.wait_with_output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let replies = String::from_utf8(output.stdout)?
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<Vec<Value>, _>>()?;
    for (index, dir) in ["lib", "lib64"].iter().enumerate() {
        assert_eq!(
            replies[index],
            json!({"ok":BASE64_STANDARD.encode(std::fs::read(format!("rootfs/{dir}/libdl.so"))?)})
        );
    }
    let text = std::fs::read_to_string(&record)?;
    let header: Value = serde_json::from_str(text.lines().next().unwrap())?;
    assert_eq!(
        header["options"]["file"],
        json!([
            ["/one", "rootfs/lib/libdl.so"],
            ["/two", "rootfs/lib64/libdl.so"]
        ])
    );
    assert_eq!(
        unidroid::serve::replay(text.as_bytes(), Emulator::builder())?,
        2
    );
    std::fs::remove_file(record)?;
    Ok(())
}
