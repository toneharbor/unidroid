use super::*;
use std::process::Command;

#[test]
fn cli_reports_json_and_exit_status_with_and_without_filtering() {
    let temp = TempDir::new();
    let jobs = temp.file(
        "jobs.jsonl",
        &lines(&[json!({"id":"one", "commands":[
            {"cmd":"new_string", "value":"hello"}, {"cmd":"string", "handle":{"$ref":0}}
        ]})]),
    );
    let run = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_unidroid"))
            .args([
                "batch",
                "--workers",
                "2",
                "--rootfs",
                "rootfs",
                "--fixed-clock",
                "1700000000",
                "--random-seed",
                "42",
                "--prop",
                "ro.product.model=Pixel",
                "--alias",
                "unused=strlen",
                "--superclass",
                "test/A=test/B",
                "--translation-cache-mb",
                "8",
            ])
            .args(extra)
            .arg(&jobs)
            .output()
            .unwrap()
    };
    let success = run(&[]);
    assert_eq!(
        success.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&success.stderr)
    );
    assert!(success.stderr.is_empty());
    let report: Value = serde_json::from_slice(&success.stdout).unwrap();
    assert!(report["summary"]["ms"].is_u64());
    assert_eq!(report["summary"]["ok"], 1);
    assert_eq!(report["jobs"][0]["results"][1], "hello");
    assert_eq!(report["jobs"][0]["attempts"], 1);
    assert!(report["jobs"][0]["ms"].is_u64());
    assert_eq!(report["jobs"][0].as_object().unwrap().len(), 5);
    std::fs::write(
        &jobs,
        format!("{}bad json\n", std::fs::read_to_string(&jobs).unwrap()),
    )
    .unwrap();
    for filter in ["ok", "failed"] {
        let output = run(&["--only", filter, "--retries", "1"]);
        assert_eq!(output.status.code(), Some(1));
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["summary"]["jobs"], 2);
        assert_eq!(report["summary"]["ok"], 1);
        assert_eq!(report["summary"]["failed"], 1);
        assert_eq!(report["jobs"].as_array().unwrap().len(), 1);
        assert_eq!(report["jobs"][0]["ok"], filter == "ok");
        assert_eq!(
            report["jobs"][0]["attempts"],
            if filter == "ok" { 1 } else { 2 }
        );
    }
}

#[test]
fn cli_setup_failures_emit_only_the_numbered_error() {
    let temp = TempDir::new();
    let setup = temp.file(
        "setup.jsonl",
        "{\"cmd\":\"new_string\",\"value\":\"one\"}\n{\"cmd\":\"answer\"}\n",
    );
    let jobs = temp.file("jobs.jsonl", "{\"commands\":[]}\n");
    let output = Command::new(env!("CARGO_BIN_EXE_unidroid"))
        .args(["batch", "--workers", "4", "--setup"])
        .arg(setup)
        .arg(jobs)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "error: setup command 2: answer is not available in batch\n"
    );
}

#[test]
fn cli_diagnostics_stay_on_stderr_in_text_and_json() {
    let temp = TempDir::new();
    let policy = temp.file(
        "policy.json",
        r#"{"rules":[{"op":"get","class":"java/lang/Void","value":null}]}"#,
    );
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let setup = temp.file(
            "setup.jsonl",
            &lines(&[
                json!({"cmd":"load", "path":format!("tests/fixtures/{abi}/libjnidispatch.so")}),
            ]),
        );
        let jobs = temp.file(
            "jobs.jsonl",
            &lines(&[json!({"commands":[
                {"cmd":"trace", "jni":true}, {"cmd":"onload", "module":{"$setup":0}}
            ]})]),
        );
        for format in ["text", "json"] {
            let output = Command::new(env!("CARGO_BIN_EXE_unidroid"))
                .args(["batch", "--workers", "1", "--log-format", format, "--setup"])
                .arg(&setup)
                .arg("--policy")
                .arg(&policy)
                .arg(&jobs)
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(0),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let report: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(report["jobs"][0]["results"], json!([null, 0]));
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(!stderr.is_empty());
            if format == "json" {
                let events: Vec<Value> = stderr
                    .lines()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect();
                assert!(events.iter().any(|event| event["event"] == "jni"));
                assert!(events.iter().all(|event| event.get("job").is_none()));
            } else {
                assert!(stderr.contains("JNIEnv->FindClass"));
            }
        }
    }
}
