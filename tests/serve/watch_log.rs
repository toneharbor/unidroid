use super::*;

#[test]
fn log_watch_uses_stderr_and_never_asks_a_break_question() {
    for (dir, words) in [
        ("lib64", [0xb9000001u32, 0xd28000e0, 0xd65f03c0]),
        ("lib", [0xe5801000, 0xe3a00007, 0xe12fff1e]),
    ] {
        let mut client = Client::start_with(&["--log-format", "json"]);
        client.request(json!({"cmd":"load", "path":format!("rootfs/{dir}/libc.so")}));
        client.request(
            json!({"cmd":"write", "address":"131328", "base64":BASE64_STANDARD.encode(
                words.into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>()
            )}),
        );
        let target = client.request(json!({"cmd":"malloc", "size":4}));
        let address = target.as_str().unwrap().parse::<u64>().unwrap();
        let at = format!("0x{address:x}");
        assert_eq!(
            client.request(json!({
                "cmd":"watch", "at":at, "size":4, "kind":"write", "log":true
            })),
            target
        );
        assert_eq!(
            client.request(json!({
                "cmd":"call_function", "address":"131328", "args":[target,"42"]
            })),
            "7"
        );
        client.request(json!({"cmd":"unwatch", "at":at}));
        assert_eq!(
            client.request(json!({
                "cmd":"call_function", "address":"131328", "args":[target,"43"]
            })),
            "7"
        );
        drop(client.input);
        let output = client.child.wait_with_output().unwrap();
        assert!(output.status.success());
        let events: Vec<Value> = String::from_utf8(output.stderr)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            events,
            [json!({
                "event":"watch", "at":"0x20100", "pc":"0x20100",
                "address":format!("0x{address:x}"), "size":4, "write":true,
                "value":"0x2a", "thread":1234, "callers":[],
            })]
        );
    }
}
