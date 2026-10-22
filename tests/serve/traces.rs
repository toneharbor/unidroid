use super::*;

#[test]
fn call_trace_cli_and_commands_emit_json_on_stderr_and_can_turn_off() {
    for arm64 in [true, false] {
        let spec = "libc.so+0x1000-0x1010";
        let mut client = Client::start_with(&[
            "--log-format",
            "json",
            "--trace-calls",
            spec,
            "--trace-calls",
            spec,
        ]);
        client.request(json!({"cmd":"load", "path":if arm64 {"rootfs/lib64/libc.so"} else {"rootfs/lib/libc.so"}}));
        let modules = client.request(json!({"cmd":"modules"}));
        let code = modules[0]["base"].as_str().unwrap().parse::<u64>().unwrap() + 0x1000;
        let protect = client.request(json!({"cmd":"resolve", "at":"mprotect"}));
        client.request(json!({"cmd":"call_function", "address":protect, "args":[code.to_string(), "4096", "7"]}));
        let words = if arm64 {
            [
                0xaa1e03f3u32,
                0x94000003,
                0xd65f0260,
                0xd503201f,
                0x91000400,
                0xd65f03c0,
            ]
        } else {
            [
                0xe1a0400e, 0xeb000001, 0xe12fff14, 0xe1a00000, 0xe2800001, 0xe12fff1e,
            ]
        };
        client.request(json!({"cmd":"write", "address":code.to_string(), "base64":BASE64_STANDARD.encode(words.into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>())}));
        let call =
            json!({"cmd":"call_function", "address":code.to_string(), "args":["7","8","9","10"]});
        assert_eq!(client.request(call.clone()), "8");
        client.request(json!({"cmd":"trace", "calls":[spec]}));
        client.send(json!({"cmd":"trace", "calls":["libc.so+1-2"]}));
        assert_eq!(
            client.receive(),
            json!({"error":"invalid code trace \"libc.so+1-2\": expected libfoo.so or libfoo.so+0x1000-0x2000"})
        );
        assert_eq!(client.request(call.clone()), "8");
        client.request(json!({"cmd":"trace"}));
        assert_eq!(client.request(call), "8");
        drop(client.input);
        let output = client.child.wait_with_output().unwrap();
        assert!(output.status.success());
        let events: Vec<Value> = String::from_utf8(output.stderr)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let expected = json!({
            "event":"function_call", "to":"libc.so+0x1010", "address":format!("0x{:x}", code+16),
            "args":["0x7", "0x8", "0x9", "0xa"], "from":"libc.so+0x1004"
        });
        assert_eq!(events, [expected.clone(), expected]);
    }
}

#[test]
fn coverage_commands_keep_counts_and_reset_after_returning_them() {
    for arm64 in [true, false] {
        let mut client = Client::start();
        assert_eq!(client.request(json!({"cmd":"coverage"})), json!([]));
        assert_eq!(
            client.request(json!({"cmd":"trace", "coverage":["libc.so+0x1000-0x1008"]})),
            Value::Null
        );
        client.request(json!({"cmd":"load", "path":if arm64 {"rootfs/lib64/libc.so"} else {"rootfs/lib/libc.so"}}));
        let modules = client.request(json!({"cmd":"modules"}));
        let code = modules[0]["base"].as_str().unwrap().parse::<u64>().unwrap() + 0x1000;
        let protect = client.request(json!({"cmd":"resolve", "at":"mprotect"}));
        client.request(json!({"cmd":"call_function", "address":protect, "args":[code.to_string(), "4096", "7"]}));
        // Return seven in one block.
        let words = if arm64 {
            [0x528000e0u32, 0xd65f03c0]
        } else {
            [0xe3a00007, 0xe12fff1e]
        };
        client.request(json!({"cmd":"write", "address":code.to_string(), "base64":BASE64_STANDARD.encode(words.into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>())}));
        client.request(json!({"cmd":"coverage", "reset":true}));
        let call = json!({"cmd":"call_function", "address":code.to_string(), "args":[]});
        for _ in 0..2 {
            assert_eq!(client.request(call.clone()), "7");
        }
        let expected =
            json!([{"address":code.to_string(), "location":"libc.so+0x1000", "count":2}]);
        assert_eq!(client.request(json!({"cmd":"coverage"})), expected);
        assert_eq!(
            client.request(json!({"cmd":"coverage", "reset":true})),
            expected
        );
        assert_eq!(client.request(json!({"cmd":"coverage"})), json!([]));
        assert_eq!(client.request(call.clone()), "7");
        client.request(json!({"cmd":"trace"}));
        assert_eq!(client.request(call), "7");
        assert_eq!(
            client.request(json!({"cmd":"coverage", "reset":true})),
            json!([{"address":code.to_string(), "location":"libc.so+0x1000", "count":1}])
        );
        assert_eq!(client.request(json!({"cmd":"coverage"})), json!([]));
        client.finish();
    }
}

#[test]
fn inspect_accepts_repeatable_call_specs_and_invalid_specs_use_the_shared_error() {
    for dir in ["lib64", "lib"] {
        let path = format!("rootfs/{dir}/libc.so");
        let output = Command::new(env!("CARGO_BIN_EXE_unidroid"))
            .args([
                "inspect",
                "--trace-calls",
                "libc.so+0x1000-0x1010",
                "--trace-calls",
                "libc.so+0x1004-0x1008",
                &path,
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for command in ["serve", "inspect"] {
        let mut process = Command::new(env!("CARGO_BIN_EXE_unidroid"));
        process.args([command, "--trace-calls", "libc.so+1-2"]);
        if command == "inspect" {
            process.arg("rootfs/lib/libc.so");
        }
        let output = process.output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            "error: invalid code trace \"libc.so+1-2\": expected libfoo.so or libfoo.so+0x1000-0x2000\n"
        );
    }
}
