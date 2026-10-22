use super::*;
use std::process::Command;

fn run(temp: &TempDir, spec: &Value, extra: &[&str]) -> std::process::Output {
    let path = temp.file("spec.json", &spec.to_string());
    Command::new(env!("CARGO_BIN_EXE_unidroid"))
        .args(["fields", "--workers", "2", "--log-format", "json"])
        .args(extra)
        .arg(path)
        .output()
        .unwrap()
}

#[test]
fn cli_emits_one_report_and_succeeds_even_when_a_field_fails() {
    let temp = TempDir::new();
    let output = run(
        &temp,
        &json!({
            "commands":[{"cmd":"new_string", "value":"a"}, {"cmd":"string", "handle":{"$ref":0}}],
            "fields":[{"name":"bad_utf8", "command":0, "offset":0, "length":1, "values":["/w=="]}]
        }),
        &[
            "--rootfs",
            "rootfs",
            "--process-name",
            "fields-test",
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
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["output"], json!({"length":1, "base64":"YQ=="}));
    assert!(
        report["fields"][0]["errors"][0]
            .as_str()
            .unwrap()
            .starts_with("value 0: ")
    );
}

#[test]
fn cli_base_failures_exit_one_with_the_original_error() {
    let temp = TempDir::new();
    let output = run(
        &temp,
        &json!({
            "commands":[{"cmd":"string", "handle":999}], "fields":[]
        }),
        &[],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.starts_with("error: base run: "), "{stderr}");
    assert!(stderr.contains("999"), "{stderr}");
}

#[test]
fn cli_policy_sequences_reset_and_json_logs_stay_on_stderr() {
    let temp = TempDir::new();
    let policy = temp.file(
        "policy.json",
        &json!({"rules":[
            {"op":"get", "class":"java/lang/Void", "name":"TYPE", "values":[null,null]}
        ]})
        .to_string(),
    );
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let output = run(
            &temp,
            &json!({
                "setup":[
                    {"cmd":"load", "path":format!("tests/fixtures/{abi}/libjnidispatch.so")},
                    {"cmd":"onload", "module":{"$ref":0}}
                ],
                "commands":[
                    {"cmd":"trace", "jni":true},
                    {"cmd":"onload", "module":{"$setup":0}},
                    {"cmd":"new_string", "value":"a"},
                    {"cmd":"string", "handle":{"$ref":2}}
                ],
                "fields":[{"name":"text", "command":2, "offset":0, "length":1}]
            }),
            &["--policy", policy.to_str().unwrap()],
        );
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            report["fields"],
            json!([{"name":"text", "runs":1, "changed":[[0,1]]}])
        );
        let stderr = String::from_utf8(output.stderr).unwrap();
        let events: Vec<Value> = stderr
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(events.iter().any(|event| event["event"] == "jni"));
    }
}

pub(super) fn random_spec(arm64: bool) -> Value {
    let words: [u32; 2] = if arm64 {
        [0xd4000001, 0xd65f03c0]
    } else {
        [0xef000000, 0xe12fff1e]
    };
    let bytes: Vec<_> = words.into_iter().flat_map(u32::to_le_bytes).collect();
    json!({
        "setup":[
            {"cmd":"load", "path":libc(arm64)},
            {"cmd":"write", "address":"131328", "base64":BASE64_STANDARD.encode(bytes)}
        ],
        "commands":[
            {"cmd":"new_string", "value":"a"},
            {"cmd":"set_register", "name":if arm64 {"x8"} else {"r7"}, "value":if arm64 {"113"} else {"263"}},
            {"cmd":"call_function", "address":"131328", "args":["0", "1879052288"]},
            {"cmd":"set_register", "name":if arm64 {"x8"} else {"r7"}, "value":if arm64 {"278"} else {"384"}},
            {"cmd":"call_function", "address":"131328", "args":["1879052304", "32", "0"]},
            {"cmd":"read", "address":"1879052288", "size":48}
        ],
        "fields":[{"name":"unused", "command":0, "offset":0, "length":1}]
    })
}

#[test]
fn cli_requires_a_repeatable_base_and_accepts_clock_and_seed_on_both_architectures() {
    let temp = TempDir::new();
    for arm64 in [true, false] {
        let spec = random_spec(arm64);
        let output = run(&temp, &spec, &[]);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            "error: the unmodified run is not repeatable; use --fixed-clock and --random-seed\n"
        );
        let output = run(
            &temp,
            &spec,
            &["--fixed-clock", "1700000000", "--random-seed", "42"],
        );
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            report["fields"],
            json!([{"name":"unused", "runs":1, "changed":[]}])
        );
        assert_eq!(report["unaffected"], json!([[0, 48]]));
    }
}
