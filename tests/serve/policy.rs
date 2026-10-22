use super::*;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct PolicyFile(PathBuf);
impl PolicyFile {
    fn new(value: &Value) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "unidroid-policy-{}-{}.json",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, value.to_string()).unwrap();
        Self(path)
    }
}
impl Drop for PolicyFile {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).unwrap();
    }
}

fn void_type() -> Value {
    json!({"rules":[{"op":"get","static":true,"class":"java/lang/Void","name":"TYPE","value":{"object":"java/lang/Class"}}]})
}

#[test]
fn policy_covers_calls_and_leaves_uncovered_calls_to_the_driver() {
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let policy = PolicyFile::new(&void_type());
        let mut client = Client::start_with(&["--policy", policy.0.to_str().unwrap()]);
        let module = client.request(
            json!({"cmd":"load","path":format!("tests/fixtures/{abi}/libjnidispatch.so")}),
        );
        client.send(json!({"cmd":"onload","module":module}));
        let question = client.receive();
        assert_eq!(question["jni"]["class"], "java/lang/Boolean", "{question}");
        client.send(json!({"cmd":"unhandled"}));
        assert!(
            client.receive()["error"]
                .as_str()
                .unwrap()
                .contains("java/lang/Boolean->TYPE")
        );
        client.finish();
    }
}

#[test]
fn policy_errors_end_the_command_and_the_session_can_continue() {
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let policy = PolicyFile::new(&json!({"rules":[{"op":"call"},{"value":42}]}));
        let mut client = Client::start_with(&["--policy", policy.0.to_str().unwrap()]);
        let module = client.request(
            json!({"cmd":"load","path":format!("tests/fixtures/{abi}/libjnidispatch.so")}),
        );
        client.send(json!({"cmd":"onload","module":module}));
        let response = client.receive();
        let error = response["error"].as_str().unwrap();
        assert!(
            error.contains("rule 1 for static get java/lang/Void->TYPE"),
            "{response}"
        );
        assert!(error.contains("from libjnidispatch.so+"), "{response}");
        assert!(response.get("crash").is_some(), "{response}");
        let handle = client.request(json!({"cmd":"new_string","value":"still running"}));
        assert_eq!(
            client.request(json!({"cmd":"string","handle":handle})),
            "still running"
        );
        client.finish();
        let output = Command::new(env!("CARGO_BIN_EXE_unidroid"))
            .args(["inspect", "--probe", "--policy"])
            .arg(&policy.0)
            .arg(format!("tests/fixtures/{abi}/libjnidispatch.so"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            stdout.contains("JNI_OnLoad stopped: policy rule 1"),
            "{stdout}"
        );
        assert!(stdout.contains("Probed calls:\n  (none)"), "{stdout}");
    }
}

#[test]
fn serve_assignability_uses_policy_sequences_then_asks_the_driver() {
    for (abi, wide) in [("arm64-v8a", true), ("armeabi-v7a", false)] {
        let policy = PolicyFile::new(
            &json!({"rules":[{"op":"is_assignable","from":"test/A","to":"test/B","values":[true,false]}]}),
        );
        let mut client = Client::start_with(&["--policy", policy.0.to_str().unwrap()]);
        let module = client.request(
            json!({"cmd":"load","path":format!("tests/fixtures/{abi}/libjnidispatch.so")}),
        );
        let entry = client.request(json!({"cmd":"find_symbol","module":module,"name":"Java_com_sun_jna_Native_getNativeVersion"})).as_str().unwrap().parse::<u64>().unwrap();
        // Inspect JNIEnv at a JNI entry point, then return without running its body.
        client.request(json!({"cmd":"break","at":format!("0x{entry:x}")}));
        client.send(json!({"cmd":"call_static","class":"com/sun/jna/Native","method":"getNativeVersion()I","args":[]}));
        let stop = client.receive();
        let env = stop["break"]["registers"][if wide { "x0" } else { "r0" }]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap();
        client.send(json!({"cmd":"return","value":"0"}));
        assert_eq!(client.receive(), json!({"ok":0}));
        let mut pointer = |address: u64| {
            let value = client.request(
                json!({"cmd":"read","address":address.to_string(),"size":if wide {8} else {4}}),
            );
            let bytes = BASE64_STANDARD.decode(value.as_str().unwrap()).unwrap();
            let mut word = [0; 8];
            word[..bytes.len()].copy_from_slice(&bytes);
            u64::from_le_bytes(word)
        };
        let table = pointer(env);
        let function = pointer(table + 11 * if wide { 8 } else { 4 });
        let from = client.request(json!({"cmd":"class","name":"test/A"}));
        let to = client.request(json!({"cmd":"class","name":"test/B"}));
        let command = json!({"cmd":"call_function","address":function.to_string(),"args":[env.to_string(),from.to_string(),to.to_string()]});
        assert_eq!(client.request(command.clone()), "1");
        assert_eq!(client.request(command.clone()), "0");
        client.send(command);
        assert_eq!(
            client.receive(),
            json!({"jni":{"op":"is_assignable","from":"test/A","to":"test/B"}})
        );
        client.send(json!({"cmd":"answer","value":true}));
        assert_eq!(client.receive(), json!({"ok":"1"}));
        client.finish();
    }
}

#[test]
fn recordings_embed_rules_and_replay_sequences_after_the_file_changes_or_disappears() {
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let rules = json!({"rules":[{"op":"get","class":"java/lang/Void","values":[{"object":"java/lang/Class"},null]}]});
        let policy = PolicyFile::new(&rules);
        let recording = policy.0.with_extension("jsonl");
        let mut client = Client::start_with(&[
            "--policy",
            policy.0.to_str().unwrap(),
            "--record",
            recording.to_str().unwrap(),
        ]);
        let module = client.request(
            json!({"cmd":"load","path":format!("tests/fixtures/{abi}/libjnidispatch.so")}),
        );
        client.send(json!({"cmd":"onload","module":module}));
        assert_eq!(client.receive()["jni"]["class"], "java/lang/Boolean");
        client.send(json!({"cmd":"answer","value":0}));
        assert_eq!(client.receive(), json!({"ok":0}));
        assert_eq!(client.request(json!({"cmd":"onload","module":module})), 0);
        client.send(json!({"cmd":"onload","module":module}));
        assert_eq!(client.receive()["jni"]["class"], "java/lang/Void");
        client.send(json!({"cmd":"unhandled"}));
        assert!(client.receive().get("error").is_some());
        client.finish();
        let text = std::fs::read_to_string(&recording).unwrap();
        let header: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(header["policy"], rules);
        assert!(header["options"].get("policy").is_none());
        let replay = || {
            let output = Command::new(env!("CARGO_BIN_EXE_unidroid"))
                .arg("replay")
                .arg(&recording)
                .output()
                .unwrap();
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(
                output.status.success(),
                "{stdout}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(stdout, "replayed 6 commands, all responses match\n");
        };
        std::fs::write(&policy.0, "broken JSON").unwrap();
        replay();
        drop(policy);
        replay();
        std::fs::remove_file(recording).unwrap();
    }
}

#[test]
fn inspect_uses_policy_before_stop_or_probe_defaults() {
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        for probe in [false, true] {
            let policy = PolicyFile::new(&void_type());
            let mut command = Command::new(env!("CARGO_BIN_EXE_unidroid"));
            command.args(["inspect", "--policy"]).arg(&policy.0);
            if probe {
                command.arg("--probe");
            }
            let output = command
                .arg(format!("tests/fixtures/{abi}/libjnidispatch.so"))
                .output()
                .unwrap();
            let stdout = String::from_utf8(output.stdout).unwrap();
            if probe {
                assert!(output.status.success(), "{stdout}");
                let calls = stdout.split_once("Probed calls:\n").unwrap().1;
                assert!(calls.contains("java/lang/Boolean->TYPE"), "{stdout}");
                assert!(!calls.contains("java/lang/Void->TYPE"), "{stdout}");
            } else {
                assert_eq!(output.status.code(), Some(1));
                assert!(
                    stdout.contains(
                        "unhandled JNI call: GetStaticObjectField java/lang/Boolean->TYPE"
                    ),
                    "{stdout}"
                );
            }
        }
    }
}
