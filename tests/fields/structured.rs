use super::*;

fn code(arm64: bool) -> Vec<u8> {
    // (input, output): copy [0,4), increment each byte of [4,8) into [8,12),
    // sum [8,11) into output[12]. Output [4,8) is left untouched.
    let mut words = if arm64 {
        vec![0xb9400002, 0xb9000022] // ldr w2,[x0]; str w2,[x1]
    } else {
        vec![0xe5902000, 0xe5812000] // ldr r2,[r0]; str r2,[r1]
    };
    for i in 4..8 {
        words.extend(if arm64 {
            [0x39400002 | i << 10, 0x11000442, 0x39000022 | (i + 4) << 10]
        } else {
            [0xe5d02000 | i, 0xe2822001, 0xe5c12000 | (i + 4)]
        });
    }
    words.extend(if arm64 {
        [
            0x39402002, 0x39402403, 0x0b030042, 0x39402803, 0x0b030042, 0x39003022, 0xd65f03c0,
        ]
    } else {
        [
            0xe5d02008, 0xe5d03009, 0xe0822003, 0xe5d0300a, 0xe0822003, 0xe5c1200c, 0xe12fff1e,
        ]
    });
    words.into_iter().flat_map(u32::to_le_bytes).collect()
}

#[test]
fn native_fields_map_copy_arithmetic_and_fold_on_both_architectures() {
    for arm64 in [true, false] {
        let mut spec = json!({
            "setup":[
                {"cmd":"load", "path":libc(arm64)},
                {"cmd":"write", "address":"131328", "base64":BASE64_STANDARD.encode(code(arm64))},
                {"cmd":"new_cstring", "value":"0000....00000"}
            ],
            "commands":[
                {"cmd":"new_cstring", "value":"abcdefghijk"},
                {"cmd":"call_function", "address":"131328", "args":[{"$ref":0}, {"$setup":2}]},
                {"cmd":"read", "address":{"$setup":2}, "size":13}
            ],
            "fields":[
                {"name":"copy", "command":0, "offset":0, "length":4},
                {"name":"increment", "command":0, "offset":4, "length":4},
                {"name":"fold", "command":0, "offset":8, "length":3}
            ]
        });
        let expected = json!({
            "output":{"length":13, "base64":BASE64_STANDARD.encode(b"abcd....fghi>")},
            "fields":[
                {"name":"copy", "runs":1, "changed":[[0,4]]},
                {"name":"increment", "runs":1, "changed":[[8,12]]},
                {"name":"fold", "runs":1, "changed":[[12,13]]}
            ],
            "by_output":[
                {"start":0, "end":4, "fields":["copy"]},
                {"start":8, "end":12, "fields":["increment"]},
                {"start":12, "end":13, "fields":["fold"]}
            ],
            "unaffected":[[4,8]]
        });
        for workers in [1, 3] {
            assert_eq!(analyze(spec.clone(), workers), expected);
        }
        for field in spec["fields"].as_array().unwrap().clone() {
            spec["fields"] = json!([field]);
            let report = analyze(spec.clone(), 2);
            let name = field["name"].as_str().unwrap();
            let expected_field = expected["fields"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["name"] == name)
                .unwrap();
            assert_eq!(report["fields"], json!([expected_field]));
            assert_eq!(report["by_output"].as_array().unwrap().len(), 1);
        }
    }
}

#[test]
fn binary_payloads_decode_before_mutation_and_comparison() {
    for arm64 in [true, false] {
        for array in [true, false] {
            let (input, output) = if array {
                (
                    json!({"cmd":"new_array", "type":"B", "base64":"AP9B"}),
                    json!({"cmd":"array", "handle":{"$ref":0}}),
                )
            } else {
                (
                    json!({"cmd":"write", "address":{"$setup":1}, "base64":"AP9B"}),
                    json!({"cmd":"read", "address":{"$setup":1}, "size":3}),
                )
            };
            let report = analyze(
                json!({
                    "setup":[{"cmd":"load", "path":libc(arm64)}, {"cmd":"malloc", "size":3}],
                    "commands":[input, output],
                    "fields":[{"name":"byte", "command":0, "offset":1, "length":1}]
                }),
                2,
            );
            assert_eq!(
                report,
                json!({
                    "output":{"length":3, "base64":"AP9B"},
                    "fields":[{"name":"byte", "runs":1, "changed":[[1,2]]}],
                    "by_output":[{"start":1, "end":2, "fields":["byte"]}],
                    "unaffected":[[0,1],[2,3]]
                })
            );
        }
    }
}
