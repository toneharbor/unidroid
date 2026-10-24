use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn recording_header_uses_effective_builder_clock_and_seed() -> unidroid::Result<()> {
    use base64::{Engine, prelude::BASE64_STANDARD};
    use unidroid::Emulator;
    for arm64 in [true, false] {
        for seed in [None, Some(42)] {
            for override_options in [false, true] {
                let path = std::env::temp_dir().join(format!(
                    "unidroid-effective-options-{}.jsonl",
                    std::process::id()
                ));
                let options: unidroid::serve::Options = serde_json::from_value(json!({
                    "rootfs":"rootfs", "apk":null, "process_name":null, "prop":[], "alias":[], "file":[], "superclass":[],
                    "fixed_clock":override_options.then_some(456), "random_seed":override_options.then_some(7), "translation_cache_mb":8,
                }))?;
                let words: [u32; 2] = if arm64 {
                    [0xd4000001, 0xd65f03c0]
                } else {
                    [0xef000000, 0xe12fff1e]
                };
                let commands = [
                    json!({"cmd":"load", "path":if arm64 { "rootfs/lib64/libc.so" } else { "rootfs/lib/libc.so" }}),
                    json!({"cmd":"write", "address":"131328", "base64":BASE64_STANDARD.encode(words.into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>())}),
                    json!({"cmd":"write", "address":"1879052288", "base64":BASE64_STANDARD.encode(b"/lib/libc.so\0")}),
                    json!({"cmd":"set_register", "name":if arm64 { "x8" } else { "r7" }, "value":if arm64 { "79" } else { "327" }}),
                    json!({"cmd":"call_function", "address":"131328", "args":["0", "1879052288", "1879052544", "0"]}),
                    json!({"cmd":"read", "address":"1879052544", "size":if arm64 {128} else {104}}),
                ];
                let input: String = commands.iter().map(|c| format!("{c}\n")).collect();
                let mut builder = Emulator::builder().fixed_clock(123);
                if let Some(seed) = seed {
                    builder = builder.random_seed(seed);
                }
                let mut output = Vec::new();
                unidroid::serve::record(
                    &options,
                    builder,
                    std::fs::File::create(&path)?,
                    input.as_bytes(),
                    &mut output,
                )?;
                for line in String::from_utf8(output)?.lines() {
                    assert!(
                        serde_json::from_str::<Value>(line)?.get("ok").is_some(),
                        "{line}"
                    );
                }
                let recording = std::fs::read_to_string(&path)?;
                let header: Value = serde_json::from_str(recording.lines().next().unwrap())?;
                assert_eq!(
                    header["options"]["fixed_clock"],
                    if override_options { 456 } else { 123 }
                );
                assert_eq!(
                    header["options"]["random_seed"],
                    json!(if override_options { Some(7) } else { seed })
                );
                assert_eq!(
                    unidroid::serve::replay(recording.as_bytes(), Emulator::builder())?,
                    commands.len()
                );
                std::fs::remove_file(path)?;
            }
        }
    }
    Ok(())
}

#[test]
fn arm64_counter_reads_are_recorded_and_replayed_across_rewind() -> unidroid::Result<()> {
    use base64::{Engine, prelude::BASE64_STANDARD};
    use unidroid::Emulator;
    let path = std::env::temp_dir().join(format!(
        "unidroid-counter-record-{}.jsonl",
        std::process::id()
    ));
    let options: unidroid::serve::Options = serde_json::from_value(json!({
        "rootfs":"rootfs", "apk":null, "process_name":null, "prop":[], "alias":[], "file":[], "superclass":[],
        "fixed_clock":123, "random_seed":null, "translation_cache_mb":8,
    }))?;
    let code: Vec<_> = [0xd53be040u32, 0xd65f03c0, 0xd53be020, 0xd65f03c0]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    let commands = [
        json!({"cmd":"load", "path":"rootfs/lib64/libc.so"}),
        json!({"cmd":"write", "address":"131328", "base64":BASE64_STANDARD.encode(code)}),
        json!({"cmd":"call_function", "address":"131328", "args":[]}),
        json!({"cmd":"call_function", "address":"131336", "args":[]}),
        json!({"cmd":"checkpoint"}),
        json!({"cmd":"call_function", "address":"131328", "args":[]}),
        json!({"cmd":"rewind", "id":1}),
        json!({"cmd":"call_function", "address":"131336", "args":[]}),
    ];
    let input: String = commands.iter().map(|c| format!("{c}\n")).collect();
    unidroid::serve::record(
        &options,
        Emulator::builder(),
        std::fs::File::create(&path)?,
        input.as_bytes(),
        std::io::sink(),
    )?;
    let recording = std::fs::read_to_string(&path)?;
    let mut entries: Vec<Value> = recording
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    assert_eq!(
        entries
            .iter()
            .filter_map(|e| e.get("time"))
            .cloned()
            .collect::<Vec<_>>(),
        [
            json!({"clock":"monotonic", "sec":1, "nsec":0}),
            json!({"clock":"monotonic", "sec":1, "nsec":1000}),
            json!({"clock":"monotonic", "sec":1, "nsec":2000}),
            json!({"clock":"monotonic", "sec":1, "nsec":2000}),
        ]
    );
    assert_eq!(
        unidroid::serve::replay(recording.as_bytes(), Emulator::builder())?,
        commands.len()
    );
    let time = entries
        .iter_mut()
        .find(|e| e.get("time").is_some())
        .unwrap();
    time["time"]["clock"] = json!("realtime");
    let changed: String = entries.iter().map(|e| format!("{e}\n")).collect();
    let error = unidroid::serve::replay(changed.as_bytes(), Emulator::builder()).unwrap_err();
    assert!(error.to_string().contains("time clock differs"), "{error}");
    std::fs::remove_file(path)?;
    Ok(())
}

fn run_record(input: &str, path: &std::path::Path) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_unidroid"))
        .args(["serve", "--record"])
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn records_exact_inputs_and_replays_or_reports_the_edited_response_line() {
    let path = std::env::temp_dir().join(format!("unidroid-replay-{}.jsonl", std::process::id()));
    let input =
        "  {\"cmd\":\"new_string\",\"value\":\"hello\"}  \n{\"cmd\":\"string\",\"handle\":1}\n";
    let output = run_record(input, &path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = std::fs::read_to_string(&path).unwrap();
    let mut lines: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines[0]["record"], 1);
    assert!(lines[0].get("policy").is_none());
    assert_eq!(lines[0]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        lines[1],
        json!({"in":"  {\"cmd\":\"new_string\",\"value\":\"hello\"}  "})
    );
    let replay = || {
        Command::new(env!("CARGO_BIN_EXE_unidroid"))
            .arg("replay")
            .arg(&path)
            .output()
            .unwrap()
    };
    let output = replay();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "replayed 2 commands, all responses match\n"
    );
    lines[2] = json!({"out":{"ok":999}});
    std::fs::write(
        &path,
        lines
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let output = replay();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "line 3: response differs: expected {\"ok\":999}, got {\"ok\":1}\n"
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn missing_and_extra_responses_name_the_recording_line() {
    let path = std::env::temp_dir().join(format!(
        "unidroid-response-errors-{}.jsonl",
        std::process::id()
    ));
    let output = run_record("{\"cmd\":\"new_string\",\"value\":\"a\"}\n", &path);
    assert!(output.status.success());
    let original = std::fs::read_to_string(&path).unwrap();
    let entries: Vec<Value> = original
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for (changed, expected) in [
        (
            entries[..2].to_vec(),
            "line 3: extra response: got {\"ok\":1}\n",
        ),
        (
            vec![entries[0].clone(), entries[2].clone()],
            "line 2: missing response: expected {\"ok\":1}\n",
        ),
    ] {
        std::fs::write(
            &path,
            changed
                .iter()
                .map(|line| format!("{line}\n"))
                .collect::<String>(),
        )
        .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_unidroid"))
            .arg("replay")
            .arg(&path)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
        assert!(output.stderr.is_empty());
    }
    std::fs::remove_file(path).unwrap();
}
