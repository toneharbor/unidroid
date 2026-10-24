use super::*;

#[test]
fn deferred_load_and_constructor_commands_stop_once() {
    // This generated constructor uses AArch64 code.
    let library = elf::TestLibrary::new(false);
    let mut client = Client::start();
    let module = client.request(json!({"cmd":"load", "path":library.0, "constructors":false}));
    let data = client.request(json!({"cmd":"find_symbol", "module":module, "name":"data"}));
    assert_eq!(
        client.request(json!({"cmd":"read", "address":data, "size":8})),
        "AAAAAAAAAAA="
    );
    let modules = client.request(json!({"cmd":"modules"}));
    let base: u64 = modules[0]["base"].as_str().unwrap().parse().unwrap();
    client.request(json!({"cmd":"break", "at":format!("0x{:x}", base + 0x220)}));
    client.send(json!({"cmd":"run_constructors", "module":module}));
    assert_eq!(
        client.receive()["break"]["address"],
        (base + 0x220).to_string()
    );
    client.send(json!({"cmd":"continue"}));
    assert_eq!(client.receive(), json!({"ok":null}));
    assert_eq!(
        client.request(json!({"cmd":"read", "address":data, "size":8})),
        "KgAAAAAAAAA="
    );
    assert_eq!(
        client.request(json!({"cmd":"run_constructors", "module":module})),
        Value::Null
    );
    client.finish();
}

#[test]
fn load_runs_constructor_stops_and_import_questions() {
    let library = elf::TestLibrary::new(false);
    let mut bytes = std::fs::read(&library.0).unwrap();
    // DT_INIT runs the normal constructor; DT_INIT_ARRAY calls the answer export.
    bytes[0x10a0..0x10a8].copy_from_slice(&12u64.to_le_bytes());
    bytes[0x10a8..0x10b0].copy_from_slice(&0x220u64.to_le_bytes());
    bytes[0x1408..0x1410].copy_from_slice(&((1u64 << 32) | 257).to_le_bytes());
    bytes[0x1410..0x1418].fill(0);
    std::fs::write(&library.0, bytes).unwrap();
    let mut client = Client::start();
    let module = client.request(json!({"cmd":"load", "path":library.0, "constructors":false}));
    let modules = client.request(json!({"cmd":"modules"}));
    let base: u64 = modules[0]["base"].as_str().unwrap().parse().unwrap();
    client.request(json!({"cmd":"break", "at":format!("0x{:x}", base + 0x220)}));
    client.request(json!({"cmd":"hook_import", "name":"answer"}));
    client.send(json!({"cmd":"load", "path":library.0}));
    assert_eq!(
        client.receive()["break"]["address"],
        (base + 0x220).to_string()
    );
    client.send(json!({"cmd":"continue"}));
    let question = client.receive();
    assert_eq!(question["import"]["name"], "answer");
    assert_eq!(question["import"]["original"], (base + 0x200).to_string());
    client.send(json!({"cmd":"answer", "value":"73"}));
    assert_eq!(client.receive(), json!({"ok":module}));
    client.finish();
}

#[test]
fn recording_replays_import_questions_and_constructor_commands() {
    for directory in ["lib64", "lib"] {
        let path = std::env::temp_dir().join(format!(
            "unidroid-import-record-{}-{directory}.jsonl",
            std::process::id()
        ));
        let mut client = Client::start_with(&["--record", path.to_str().unwrap()]);
        let module = client.request(json!({"cmd":"load", "path":format!("rootfs/{directory}/libc.so"), "constructors":false}));
        client.request(json!({"cmd":"run_constructors", "module":module}));
        client.request(json!({"cmd":"hook_import", "name":"strlen"}));
        let text = client.request(json!({"cmd":"new_cstring", "value":"hello"}));
        let function =
            client.request(json!({"cmd":"find_symbol", "module":module, "name":"strlen"}));
        for (answer, value) in [
            (json!({"cmd":"answer", "value":"9"}), "9"),
            (json!({"cmd":"unhandled"}), "5"),
        ] {
            client.send(json!({"cmd":"call_function", "address":function, "args":[text]}));
            assert_eq!(client.receive()["import"]["name"], "strlen");
            client.send(answer);
            assert_eq!(client.receive(), json!({"ok":value}));
        }
        client.finish();
        let recording = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            unidroid::serve::replay(recording.as_bytes(), unidroid::Emulator::builder()).unwrap(),
            9
        );
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn imports_without_original_report_null_and_require_decimal_answers() {
    for directory in ["lib64", "lib"] {
        let mut client = Client::start();
        client.request(json!({"cmd":"hook_import", "name":"missing_import"}));
        let module =
            client.request(json!({"cmd":"load", "path":format!("rootfs/{directory}/libc.so")}));
        let function =
            client.request(json!({"cmd":"find_symbol", "module":module, "name":"missing_import"}));
        let call = json!({"cmd":"call_function", "address":function, "args":[]});
        for (answer, expected) in [
            (json!({"cmd":"answer", "value":"9"}), Some("9")),
            (json!({"cmd":"unhandled"}), None),
            (json!({"cmd":"answer", "value":9}), None),
        ] {
            client.send(call.clone());
            let question = client.receive();
            assert_eq!(question["import"]["name"], "missing_import");
            assert_eq!(question["import"]["original"], Value::Null);
            client.send(answer.clone());
            let result = client.receive();
            if let Some(value) = expected {
                assert_eq!(result, json!({"ok":value}));
            } else if answer["cmd"] == "unhandled" {
                assert_eq!(
                    result["error"],
                    "call to unresolved symbol missing_import from 0x20000"
                );
            } else {
                assert!(
                    result["error"]
                        .as_str()
                        .unwrap()
                        .contains("expected a string"),
                    "{result}"
                );
                client.send(json!({"cmd":"answer", "value":"9"}));
                assert_eq!(client.receive(), json!({"ok":"9"}));
            }
        }
        client.finish();
    }
}

#[test]
fn malformed_import_answers_keep_original_suspended() {
    for directory in ["lib64", "lib"] {
        let mut client = Client::start();
        let module =
            client.request(json!({"cmd":"load", "path":format!("rootfs/{directory}/libc.so")}));
        let original =
            client.request(json!({"cmd":"find_symbol", "module":module, "name":"memset"}));
        client.request(json!({"cmd":"hook_import", "name":"memset"}));
        let hook = client.request(json!({"cmd":"find_symbol", "module":module, "name":"memset"}));
        let buffer = client.request(json!({"cmd":"malloc", "size":4}));
        client.request(json!({"cmd":"write", "address":buffer, "base64":"AAAAAA=="}));
        let call = json!({"cmd":"call_function", "address":hook, "args":[buffer,"65","4"]});
        client.send(call.clone());
        let question = client.receive();
        assert_eq!(question["import"]["name"], "memset");
        assert_eq!(question["import"]["original"], original);
        for value in [
            json!(9),
            json!("not-a-number"),
            json!("0x9"),
            json!("-1"),
            json!("18446744073709551616"),
        ] {
            client.send(json!({"cmd":"answer", "value":value}));
            let error = client.receive();
            assert!(
                error["error"]
                    .as_str()
                    .unwrap()
                    .starts_with("import answer:"),
                "{error}"
            );
            assert!(error.get("crash").is_none(), "{error}");
            assert_eq!(
                client.request(json!({"cmd":"read", "address":buffer, "size":4})),
                "AAAAAA=="
            );
        }
        client.send(json!({"cmd":"answer", "value":"7"}));
        assert_eq!(client.receive(), json!({"ok":"7"}));
        assert_eq!(
            client.request(json!({"cmd":"read", "address":buffer, "size":4})),
            "AAAAAA=="
        );
        client.send(call);
        assert_eq!(client.receive()["import"]["name"], "memset");
        client.send(json!({"cmd":"unhandled"}));
        assert_eq!(client.receive(), json!({"ok":buffer}));
        assert_eq!(
            client.request(json!({"cmd":"read", "address":buffer, "size":4})),
            "QUFBQQ=="
        );
        client.finish();
    }
}

#[test]
fn hooks_without_a_driver_pass_through_in_batch() {
    let options = serde_json::from_value(json!({
        "rootfs":"rootfs", "apk":null, "process_name":null, "prop":[], "alias":[], "file":[], "superclass":[],
        "fixed_clock":1700000000, "random_seed":42, "translation_cache_mb":8
    }))
    .unwrap();
    for directory in ["lib64", "lib"] {
        let job = json!({"commands":[
            {"cmd":"hook_import", "name":"strlen"},
            {"cmd":"load", "path":format!("rootfs/{directory}/libc.so"), "constructors":false},
            {"cmd":"run_constructors", "module":{"$ref":1}},
            {"cmd":"find_symbol", "module":{"$ref":1}, "name":"strlen"},
            {"cmd":"new_cstring", "value":"hello"},
            {"cmd":"call_function", "address":{"$ref":3}, "args":[{"$ref":4}]}
        ]});
        let input = format!("{job}\n");
        let report = unidroid::serve::batch(
            &options,
            &unidroid::serve::BatchOptions {
                workers: 1,
                ..Default::default()
            },
            unidroid::Emulator::builder,
            input.as_bytes(),
        )
        .unwrap();
        assert_eq!(report["jobs"][0]["ok"], true, "{report}");
        assert_eq!(report["jobs"][0]["results"][5], "5");
    }
}

#[test]
fn imports_support_answers_passthrough_and_nested_original_calls() {
    for (directory, registers) in [("lib64", 8), ("lib", 4)] {
        let mut client = Client::start();
        let module =
            client.request(json!({"cmd":"load", "path":format!("rootfs/{directory}/libc.so")}));
        let original =
            client.request(json!({"cmd":"find_symbol", "module":module, "name":"strlen"}));
        assert_eq!(
            client.request(json!({"cmd":"hook_import", "name":"strlen"})),
            Value::Null
        );
        let function =
            client.request(json!({"cmd":"find_symbol", "module":module, "name":"strlen"}));
        let text = client.request(json!({"cmd":"new_cstring", "value":"hello"}));
        let mut args = vec![json!("0"); registers];
        args[0] = text.clone();
        let call = json!({"cmd":"call_function", "address":function, "args":args});
        let expected =
            json!({"import":{"name":"strlen", "args":args, "original":original, "from":"0x20000"}});
        client.send(call.clone());
        assert_eq!(client.receive(), expected);
        client.send(json!({"cmd":"continue"}));
        assert_eq!(
            client.receive(),
            json!({"error":"import question expects answer or unhandled"})
        );
        assert_eq!(
            client.request(json!({"cmd":"call_function", "address":original, "args":[text]})),
            "5"
        );
        client.send(json!({"cmd":"answer", "value":"73"}));
        assert_eq!(client.receive(), json!({"ok":"73"}));
        client.send(call);
        assert_eq!(client.receive(), expected);
        client.send(json!({"cmd":"unhandled"}));
        assert_eq!(client.receive(), json!({"ok":"5"}));
        client.finish();
    }
}
