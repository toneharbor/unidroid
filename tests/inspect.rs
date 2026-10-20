use std::process::Command;

#[path = "support/elf.rs"]
mod elf;

#[test]
fn options_require_key_value_pairs() {
    for subcommand in ["inspect", "serve"] {
        for (option, label) in [
            ("--prop", "KEY=VALUE"),
            ("--alias", "NAME=TARGET"),
            ("--superclass", "CLASS=SUPERCLASS"),
        ] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_minidbg"));
            command.args([subcommand, option, "missing"]);
            if subcommand == "inspect" {
                command.arg("tests/fixtures/armeabi-v7a/libsignutil.so");
            }
            let output = command.output().unwrap();
            assert_eq!(output.status.code(), Some(2));
            assert!(output.stdout.is_empty());
            assert_eq!(
                String::from_utf8(output.stderr)
                    .unwrap()
                    .lines()
                    .next()
                    .unwrap(),
                format!(
                    "error: invalid value 'missing' for '{option} <{label}>': expected KEY=VALUE"
                )
            );
        }
    }
}

#[test]
fn both_subcommands_accept_repeatable_superclasses() {
    for subcommand in ["inspect", "serve"] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_minidbg"));
        command.args([
            subcommand,
            "--superclass",
            "a/B=a/C",
            "--superclass",
            "a/C=java/lang/Object",
        ]);
        if subcommand == "inspect" {
            command.arg("tests/fixtures/armeabi-v7a/libsignutil.so");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn inspect_names_a_non_arm_library_in_its_error() {
    let library = elf::TestLibrary::new(false);
    let mut bytes = std::fs::read(&library.0).unwrap();
    bytes[18..20].copy_from_slice(&62u16.to_le_bytes()); // EM_X86_64
    std::fs::write(&library.0, bytes).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_minidbg"))
        .arg("inspect")
        .arg(&library.0)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!(
            "error: {}: not an ARM library\n",
            library.0.file_name().unwrap().to_string_lossy()
        )
    );
}

#[test]
fn inspect_reports_arm32_exports_and_no_registered_natives() {
    let output = Command::new(env!("CARGO_BIN_EXE_minidbg"))
        .args([
            "inspect",
            "--rootfs",
            "rootfs",
            "tests/fixtures/armeabi-v7a/libsignutil.so",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        stdout.lines().next(),
        Some("libsignutil.so: 32-bit ARM (armeabi-v7a)")
    );
    assert!(stdout.lines().any(|line| line == "JNI_OnLoad: yes"));
    let exports = stdout
        .split_once("Java_ exports:\n")
        .unwrap()
        .1
        .split_once("JNI_OnLoad:")
        .unwrap()
        .0;
    assert!(
        exports
            .lines()
            .any(|line| line == "  Java_com_anjuke_mobile_sign_SignUtil_getSign0")
    );
    let natives = stdout.split_once("Registered natives:\n").unwrap().1;
    assert_eq!(natives, "  (none)\n");
}

#[test]
fn inspect_lists_registered_natives_in_order() {
    let output = Command::new(env!("CARGO_BIN_EXE_minidbg"))
        .args([
            "inspect",
            "--rootfs",
            "rootfs",
            "tests/fixtures/arm64-v8a/libtmessages.29.so",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let natives: Vec<_> = stdout
        .split_once("Registered natives:\n")
        .unwrap()
        .1
        .lines()
        .collect();
    assert!(natives.contains(
        &"  org/telegram/tgnet/NativeByteBuffer.native_limit(J)I at libtmessages.29.so+0x208eb8"
    ));
    assert!(natives.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn inspect_reports_arm64_and_stops_at_unanswered_java() {
    let output = Command::new(env!("CARGO_BIN_EXE_minidbg"))
        .args([
            "inspect",
            "--rootfs",
            "rootfs",
            "tests/fixtures/arm64-v8a/libjnidispatch.so",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout.lines().next(),
        Some("libjnidispatch.so: 64-bit ARM (arm64-v8a)")
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout.contains("JNI_OnLoad stopped: unhandled JNI call:"));
    assert!(stdout.contains("Registered natives:\n"));
}
