use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};

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
