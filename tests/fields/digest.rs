use super::*;

#[test]
fn a_digest_spreads_each_input_field_across_every_output_byte() {
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let report = analyze(
            json!({
                "setup":[
                    {"cmd":"load", "path":format!("tests/fixtures/{abi}/libtmessages.29.so")},
                    {"cmd":"onload", "module":{"$ref":0}}
                ],
                "commands":[
                    {"cmd":"new_array", "type":"B", "base64":"MTIzNDU2"},
                    {"cmd":"new_array", "type":"B", "base64":"AAAAAAAAAAA="},
                    {"cmd":"new_array", "type":"B", "base64":BASE64_STANDARD.encode([0;64])},
                    {"cmd":"call_static", "class":"org/telegram/messenger/Utilities",
                     "method":"pbkdf2([B[B[BI)V", "args":[{"$ref":0},{"$ref":1},{"$ref":2},1]},
                    {"cmd":"array", "handle":{"$ref":2}}
                ],
                "fields":[
                    {"name":"password", "command":0, "offset":0, "length":1},
                    {"name":"salt", "command":1, "offset":0, "length":8}
                ]
            }),
            2,
        );
        assert_eq!(
            report,
            json!({
                // PBKDF2-HMAC-SHA512, one iteration, independently checked with hashlib.
                "output":{"length":64, "base64":"p/3IVsFVzlLEdzLUhvW0lpJKdpPgMHyTAiD0ycpya1TFavMcdzp298zVmD5eRAyzUOJQb0wSQPW3f32Cobp6TQ=="},
                "fields":[
                    {"name":"password", "runs":1, "changed":[[0,64]]},
                    {"name":"salt", "runs":1, "changed":[[0,64]]}
                ],
                "by_output":[{"start":0, "end":64, "fields":["password", "salt"]}],
                "unaffected":[]
            })
        );
    }
}
