use super::Client;
use base64::{Engine, prelude::BASE64_STANDARD};
use serde_json::json;

#[test]
fn crypto_command_returns_named_decimal_addresses() {
    for dir in ["lib64", "lib"] {
        let mut client = Client::start();
        client.request(json!({
            "cmd":"load",
            "path":format!("rootfs/{dir}/libc.so")
        }));
        let memory = client.request(json!({"cmd":"malloc", "size":32}));
        client.request(json!({
            "cmd":"write",
            "address":memory,
            "base64":BASE64_STANDARD.encode(b"expand 32-byte k")
        }));
        let constants = client.request(json!({"cmd":"crypto"}));
        let address = memory.as_str().unwrap().parse::<u64>().unwrap();
        let hit = constants
            .as_array()
            .unwrap()
            .iter()
            .find(|constant| constant["address"] == memory)
            .unwrap();
        assert_eq!(hit["name"], "ChaCha/Salsa constant");
        assert_eq!(hit["location"], format!("0x{address:x} [host malloc]"));
    }
}
