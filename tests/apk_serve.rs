#[path = "support/apk.rs"]
mod support;
use base64::{Engine, prelude::BASE64_STANDARD};
use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};
use support::{
    TestApk,
    manifest::{self, package},
};
use unidroid::{Emulator, Result};
use zip::CompressionMethod::Stored;

fn run(args: &[&str], commands: &[Value]) -> Result<Vec<Value>> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_unidroid"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().unwrap();
    for command in commands {
        writeln!(input, "{command}")?;
    }
    drop(input);
    let output = child.wait_with_output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)?
        .lines()
        .map(|line| Ok(serde_json::from_str(line)?))
        .collect()
}

#[test]
fn serve_apk_commands_read_update_load_and_report_missing_apk() -> Result<()> {
    let commands = [
        json!({"cmd":"apk"}),
        json!({"cmd":"set_apk", "package":"override"}),
        json!({"cmd":"load_apk_library", "name":"libdl.so"}),
    ];
    let replies = run(&["serve"], &commands)?;
    assert_eq!(replies[0], json!({"ok":null}));
    for reply in &replies[1..] {
        assert!(
            reply["error"].as_str().unwrap().contains("no APK"),
            "{reply}"
        );
    }
    for (abi, dir) in [("arm64-v8a", "lib64"), ("armeabi-v7a", "lib")] {
        let xml = manifest::xml(true, &[package()]);
        let library = std::fs::read(format!("rootfs/{dir}/libdl.so"))?;
        let file = TestApk::new(&[
            ("AndroidManifest.xml", &xml, Stored, 1),
            (&format!("lib/{abi}/libdl.so"), &library, Stored, 1),
        ])?;
        let replies = run(
            &["serve", "--apk", file.path.to_str().unwrap()],
            &[
                json!({"cmd":"apk"}),
                json!({"cmd":"load_apk_library", "name":"libdl.so", "constructors":false}),
                json!({"cmd":"modules"}),
                json!({"cmd":"set_apk", "package":"override", "version_name":"custom", "version_code":8589934593i64, "signatures":["MAEq"]}),
                json!({"cmd":"apk"}),
                json!({"cmd":"set_apk", "version_name":null}),
                json!({"cmd":"apk"}),
                json!({"cmd":"set_apk", "signatures":["!"]}),
                json!({"cmd":"apk"}),
                json!({"cmd":"load_apk_library", "name":"absent.so"}),
                json!({"cmd":"load_apk_library", "name":"libdl.so"}),
            ],
        )?;
        assert_eq!(
            replies[0],
            json!({"ok":{"path":file.path.canonicalize()?, "package":"com.example.apk", "version_name":null, "version_code":0, "signatures":[], "signers":[], "signing_history":[]}})
        );
        assert_eq!(replies[1], json!({"ok":0}));
        assert_eq!(
            replies[2]["ok"][0]["path"],
            format!(
                "{}!/lib/{abi}/libdl.so",
                file.path.canonicalize()?.display()
            )
        );
        assert_eq!(replies[3], json!({"ok":null}));
        assert_eq!(replies[4]["ok"]["package"], "override");
        assert_eq!(replies[4]["ok"]["version_name"], "custom");
        assert_eq!(replies[4]["ok"]["version_code"], 8589934593i64);
        assert_eq!(replies[4]["ok"]["signatures"], json!(["MAEq"]));
        assert_eq!(replies[5], json!({"ok":null}));
        assert_eq!(replies[6]["ok"]["version_name"], Value::Null);
        assert_eq!(replies[6]["ok"]["package"], "override");
        assert_eq!(replies[6]["ok"]["signatures"], json!(["MAEq"]));
        assert!(replies[7]["error"].as_str().unwrap().contains("signatures"));
        assert_eq!(replies[8], replies[6]);
        assert!(replies[9]["error"].as_str().unwrap().contains("absent.so"));
        assert_eq!(replies[10], json!({"ok":0}));
    }
    Ok(())
}

#[test]
fn serve_updates_signers_and_history_without_partial_changes() -> Result<()> {
    let file = TestApk::manifest(&manifest::xml(true, &[package()]))?;
    let replies = run(
        &["serve", "--apk", file.path.to_str().unwrap()],
        &[
            json!({"cmd":"set_apk", "signers":["MAEq"], "signing_history":["MAEr","MAEq"]}),
            json!({"cmd":"apk"}),
            json!({"cmd":"set_apk", "package":"must-not-change", "signers":["!"]}),
            json!({"cmd":"set_apk", "signers":[], "signing_history":["!"]}),
            json!({"cmd":"apk"}),
            json!({"cmd":"set_apk", "signing_history":[]}),
            json!({"cmd":"apk"}),
        ],
    )?;
    assert_eq!(replies[0], json!({"ok":null}));
    assert_eq!(replies[1]["ok"]["signers"], json!(["MAEq"]));
    assert_eq!(replies[1]["ok"]["signing_history"], json!(["MAEr", "MAEq"]));
    assert_eq!(replies[1]["ok"]["signatures"], json!([]));
    assert!(replies[2]["error"].as_str().unwrap().contains("signers"));
    assert!(
        replies[3]["error"]
            .as_str()
            .unwrap()
            .contains("signing_history")
    );
    assert_eq!(replies[4], replies[1]);
    assert_eq!(replies[6]["ok"]["signing_history"], json!([]));
    assert_eq!(replies[6]["ok"]["signers"], json!(["MAEq"]));
    Ok(())
}

#[test]
fn recording_keeps_builder_apk_and_process_name_when_options_omit_them() -> Result<()> {
    let file = TestApk::manifest(&manifest::xml(true, &[package()]))?;
    let relative = file.path.strip_prefix(env!("CARGO_MANIFEST_DIR"))?;
    let options = serde_json::from_value(json!({
        "rootfs":"rootfs", "apk":null, "process_name":null,
        "prop":[], "alias":[], "file":[], "superclass":[], "translation_cache_mb":8
    }))?;
    let record = file.path.with_file_name("builder.jsonl");
    unidroid::serve::record(
        &options,
        Emulator::builder()
            .apk(relative)
            .process_name("builder.name"),
        std::fs::File::create(&record)?,
        b"{\"cmd\":\"apk\"}\n".as_slice(),
        std::io::sink(),
    )?;
    let recording = std::fs::read_to_string(&record)?;
    let header: Value = serde_json::from_str(recording.lines().next().unwrap())?;
    assert_eq!(header["options"]["apk"], json!(relative));
    assert_eq!(header["options"]["process_name"], "builder.name");
    assert_eq!(
        unidroid::serve::replay(recording.as_bytes(), Emulator::builder())?,
        1
    );
    Ok(())
}

#[test]
fn batch_and_fields_accept_apk_options_and_restore_apk_setup() -> Result<()> {
    for (abi, dir) in [("arm64-v8a", "lib64"), ("armeabi-v7a", "lib")] {
        let xml = manifest::xml(true, &[package()]);
        let library = std::fs::read(format!("rootfs/{dir}/libdl.so"))?;
        let file = TestApk::new(&[
            ("AndroidManifest.xml", &xml, Stored, 1),
            (&format!("lib/{abi}/libdl.so"), &library, Stored, 1),
        ])?;
        let setup = file.path.with_file_name("setup.jsonl");
        std::fs::write(
            &setup,
            "{\"cmd\":\"load_apk_library\",\"name\":\"libdl.so\"}\n",
        )?;
        let jobs = file.path.with_file_name("jobs.jsonl");
        let job = json!({"commands":[{"cmd":"apk"}, {"cmd":"set_apk","package":"per-job"}, {"cmd":"apk"}]});
        std::fs::write(&jobs, format!("{job}\n{job}\n{job}\n"))?;
        let output = Command::new(env!("CARGO_BIN_EXE_unidroid"))
            .args(["batch", "--apk"])
            .arg(&file.path)
            .args(["--workers", "2", "--setup"])
            .arg(&setup)
            .arg(&jobs)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(report["summary"]["ok"], 3);
        for job in report["jobs"].as_array().unwrap() {
            assert_eq!(job["results"][0]["package"], "com.example.apk");
            assert_eq!(job["results"][2]["package"], "per-job");
        }
        let spec = file.path.with_file_name("fields.json");
        std::fs::write(
            &spec,
            json!({
                "setup":[{"cmd":"load_apk_library","name":"libdl.so"}],
                "commands":[{"cmd":"apk"}], "fields":[]
            })
            .to_string(),
        )?;
        let output = Command::new(env!("CARGO_BIN_EXE_unidroid"))
            .args(["fields", "--apk"])
            .arg(&file.path)
            .args(["--workers", "2"])
            .arg(&spec)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout)?;
        let bytes = BASE64_STANDARD.decode(report["output"]["base64"].as_str().unwrap())?;
        let apk: Value = serde_json::from_slice(&bytes)?;
        assert_eq!(apk["package"], "com.example.apk");
        assert_eq!(apk["path"], json!(file.path.canonicalize()?));
    }
    Ok(())
}

#[test]
fn inspect_reads_apk_libraries_on_both_architectures() -> Result<()> {
    for (abi, dir, architecture) in [
        ("arm64-v8a", "lib64", "64-bit ARM (arm64-v8a)"),
        ("armeabi-v7a", "lib", "32-bit ARM (armeabi-v7a)"),
    ] {
        let xml = manifest::xml(true, &[package()]);
        let library = std::fs::read(format!("rootfs/{dir}/libdl.so"))?;
        let file = TestApk::new(&[
            ("AndroidManifest.xml", &xml, Stored, 1),
            (&format!("lib/{abi}/libdl.so"), &library, Stored, 1),
        ])?;
        let output = Command::new(env!("CARGO_BIN_EXE_unidroid"))
            .args(["inspect", "--apk"])
            .arg(&file.path)
            .arg("libdl.so")
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout)?;
        assert!(
            text.starts_with(&format!("libdl.so: {architecture}\n")),
            "{text}"
        );
        assert!(text.contains("JNI_OnLoad: no"));
    }
    Ok(())
}

#[test]
fn recording_keeps_apk_argument_and_effective_process_name_for_replay() -> Result<()> {
    for (abi, dir) in [("arm64-v8a", "lib64"), ("armeabi-v7a", "lib")] {
        for explicit in [None, Some("explicit.name")] {
            let xml = manifest::xml(true, &[package()]);
            let library = std::fs::read(format!("rootfs/{dir}/libdl.so"))?;
            let file = TestApk::new(&[
                ("AndroidManifest.xml", &xml, Stored, 1),
                (&format!("lib/{abi}/libdl.so"), &library, Stored, 1),
            ])?;
            let relative = file
                .path
                .strip_prefix(env!("CARGO_MANIFEST_DIR"))?
                .to_str()
                .unwrap();
            let record = file.path.with_file_name("record.jsonl");
            let mut args = vec![
                "serve",
                "--apk",
                relative,
                "--record",
                record.to_str().unwrap(),
            ];
            if let Some(name) = explicit {
                args.extend(["--process-name", name]);
            }
            let commands = [
                json!({"cmd":"apk"}),
                json!({"cmd":"load_apk_library", "name":"libdl.so"}),
                json!({"cmd":"file", "path":"/proc/self/cmdline"}),
                json!({"cmd":"set_apk", "version_name":"recorded", "signers":["MAEq"], "signing_history":["MAEr","MAEq"]}),
                json!({"cmd":"apk"}),
            ];
            let replies = run(&args, &commands)?;
            let name = explicit.unwrap_or("com.example.apk");
            assert_eq!(
                replies[2]["ok"],
                BASE64_STANDARD.encode(format!("{name}\0"))
            );
            let recording = std::fs::read_to_string(&record)?;
            let header: Value = serde_json::from_str(recording.lines().next().unwrap())?;
            assert_eq!(header["options"]["apk"], relative);
            assert_eq!(header["options"]["process_name"], name);
            assert_eq!(
                unidroid::serve::replay(recording.as_bytes(), Emulator::builder())?,
                commands.len()
            );
            let output = Command::new(env!("CARGO_BIN_EXE_unidroid"))
                .arg("replay")
                .arg(&record)
                .output()?;
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8(output.stdout)?,
                "replayed 5 commands, all responses match\n"
            );
        }
    }
    Ok(())
}
