use super::*;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Recording(PathBuf);
impl Recording {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Self(std::env::temp_dir().join(format!(
            "unidroid-record-{}-{}.jsonl",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
    fn client(&self) -> Client {
        Client::start_with(&["--record", self.0.to_str().unwrap()])
    }
    fn replay(&self, options: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_unidroid"))
            .arg("replay")
            .arg(&self.0)
            .args(options)
            .output()
            .unwrap()
    }
    fn entries(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.0)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    fn replace(&self, entries: &[Value]) {
        std::fs::write(
            &self.0,
            entries
                .iter()
                .map(|line| format!("{line}\n"))
                .collect::<String>(),
        )
        .unwrap();
    }
}
impl Drop for Recording {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).unwrap();
    }
}

#[test]
fn signutil_session_records_and_replays_including_nested_jni_questions() {
    let recording = Recording::new();
    signutil_session(recording.client());
    let entries = recording.entries();
    for line in entries.iter().take(10) {
        println!("{line}");
    }
    let count = entries.iter().filter(|v| v.get("in").is_some()).count();
    let output = recording.replay(&[]);
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("replayed {count} commands, all responses match\n")
    );
    assert!(entries.iter().any(|v| v["out"]["ok"] == SIGN));
    let traced = recording.replay(&[
        "--trace-code",
        "libsignutil.so",
        "--trace-calls",
        "libsignutil.so",
        "--trace-calls",
        "libsignutil.so+0x1000-0x2000",
        "--log-format",
        "json",
    ]);
    assert!(
        traced.status.success(),
        "{}{}",
        String::from_utf8_lossy(&traced.stdout),
        String::from_utf8_lossy(&traced.stderr)
    );
    assert_eq!(
        String::from_utf8(traced.stdout).unwrap(),
        format!("replayed {count} commands, all responses match\n")
    );
    let events: Vec<Value> = String::from_utf8(traced.stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(events.iter().any(|event| event["event"] == "instruction"));
    assert!(events.iter().any(|event| event["event"] == "function_call"));
    let invalid = recording.replay(&["--trace-calls", "libsignutil.so+1-2"]);
    assert_eq!(invalid.status.code(), Some(1));
    assert!(
        String::from_utf8(invalid.stdout)
            .unwrap()
            .contains("invalid code trace \"libsignutil.so+1-2\"")
    );
}

fn syscall(client: &mut Client, arm64: bool, number: u32, args: &[Value]) -> Value {
    let code = 0x20100u64;
    let words = if arm64 {
        [0xd2800008 | (number << 5), 0xd4000001, 0xd65f03c0]
    } else {
        [
            0xe3007000 | ((number & 0xf000) << 4) | (number & 0xfff),
            0xef000000,
            0xe12fff1e,
        ]
    };
    let bytes: Vec<_> = words.into_iter().flat_map(u32::to_le_bytes).collect();
    client.request(
        json!({"cmd":"write", "address":code.to_string(), "base64":BASE64_STANDARD.encode(bytes)}),
    );
    client.request(json!({"cmd":"call_function", "address":code.to_string(), "args":args}))
}

#[test]
fn guest_time_and_both_random_devices_replay_with_syscall_tracing() {
    for arm64 in [true, false] {
        let recording = Recording::new();
        let mut client = recording.client();
        client.request(json!({"cmd":"load", "path":if arm64 {"rootfs/lib64/libc.so"} else {"rootfs/lib/libc.so"}}));
        let buffer = client.request(json!({"cmd":"malloc", "size":32}));
        for clock in ["0", "1"] {
            assert_eq!(
                syscall(
                    &mut client,
                    arm64,
                    if arm64 { 113 } else { 263 },
                    &[json!(clock), buffer.clone()]
                ),
                "0"
            );
            client.request(json!({"cmd":"read", "address":buffer, "size":if arm64 {16} else {8}}));
        }
        assert_eq!(
            syscall(
                &mut client,
                arm64,
                if arm64 { 169 } else { 78 },
                &[buffer.clone(), json!("0")]
            ),
            "0"
        );
        client.request(json!({"cmd":"read", "address":buffer, "size":if arm64 {16} else {8}}));
        for device in ["/dev/urandom", "/dev/random"] {
            let path = client.request(json!({"cmd":"new_cstring", "value":device}));
            let fd = syscall(
                &mut client,
                arm64,
                if arm64 { 56 } else { 322 },
                &[json!("0"), path, json!("0")],
            );
            assert_eq!(
                syscall(
                    &mut client,
                    arm64,
                    if arm64 { 63 } else { 3 },
                    &[fd.clone(), buffer.clone(), json!("32")]
                ),
                "32"
            );
            client.request(json!({"cmd":"read", "address":buffer, "size":32}));
            assert_eq!(
                syscall(&mut client, arm64, if arm64 { 57 } else { 6 }, &[fd]),
                "0"
            );
        }
        client.finish();
        let entries = recording.entries();
        assert!(entries.iter().any(|e| e["time"]["clock"] == "realtime"));
        assert!(entries.iter().any(|e| e["time"]["clock"] == "monotonic"));
        assert_eq!(
            entries.iter().filter(|e| e.get("random").is_some()).count(),
            2
        );
        let output = recording.replay(&["--trace-syscalls", "--log-format", "json"]);
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let events: Vec<Value> = String::from_utf8(output.stderr)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        for name in ["clock_gettime", "gettimeofday", "read", "openat"] {
            assert!(
                events
                    .iter()
                    .any(|event| event["event"] == "syscall" && event["name"] == name),
                "{name}: {events:?}"
            );
        }
        let time = entries
            .iter()
            .position(|e| e.get("time").is_some())
            .unwrap();
        let random = entries
            .iter()
            .position(|e| e.get("random").is_some())
            .unwrap();
        let mut changed = entries.clone();
        changed[time] = json!({"random":"AA=="});
        recording.replace(&changed);
        let output = recording.replay(&[]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!(
                "line {}: guest asked for time, next recorded value is random\n",
                time + 1
            )
        );
        let mut changed = entries.clone();
        changed[random] = entries[time].clone();
        recording.replace(&changed);
        let output = recording.replay(&[]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!(
                "line {}: guest asked for random, next recorded value is time\n",
                random + 1
            )
        );
        recording.replace(&entries[..time]);
        let output = recording.replay(&[]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!(
                "line {}: recording ran out of values: expected time\n",
                time + 1
            )
        );
        let mut changed = entries.clone();
        changed.push(json!({"random":"AA=="}));
        recording.replace(&changed);
        let output = recording.replay(&[]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!(
                "line {}: unused random value at end of replay\n",
                entries.len() + 1
            )
        );
    }
}
