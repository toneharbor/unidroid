use super::*;

#[test]
fn explicit_values_union_only_successful_runs_and_keep_other_fields() {
    let report = analyze(
        json!({
            "commands":[
                {"cmd":"new_string", "value":"abcd"},
                {"cmd":"string", "handle":{"$ref":0}}
            ],
            "fields":[
                {"name":"ends", "command":0, "offset":0, "length":4,
                 "values":["WmJjZA==", "YWJjWg==", "YWJjZA==", "/2JjZA=="]},
                {"name":"middle", "command":0, "offset":1, "length":2},
                {"name":"none", "command":0, "offset":0, "length":1, "values":[]}
            ]
        }),
        3,
    );
    assert_eq!(report["fields"][0]["runs"], 4);
    assert_eq!(report["fields"][0]["changed"], json!([[0, 1], [3, 4]]));
    let errors = report["fields"][0]["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].as_str().unwrap().starts_with("value 3: "));
    assert!(errors[0].as_str().unwrap().contains("utf-8"));
    assert_eq!(
        report["fields"][1],
        json!({"name":"middle", "runs":1, "changed":[[1,3]]})
    );
    assert_eq!(
        report["fields"][2],
        json!({"name":"none", "runs":0, "changed":[]})
    );
    assert_eq!(report["unaffected"], json!([]));
}

#[test]
fn a_default_change_that_breaks_utf8_is_a_field_error() {
    // U+1000's E1 lead byte becomes E0 followed by 80, which is below A0.
    let report = analyze(
        json!({
            "commands":[{"cmd":"new_string", "value":"\u{1000}a"}, {"cmd":"string", "handle":{"$ref":0}}],
            "fields":[
                {"name":"invalid", "command":0, "offset":0, "length":1},
                {"name":"valid", "command":0, "offset":3, "length":1}
            ]
        }),
        2,
    );
    assert_eq!(report["fields"][0]["changed"], json!([]));
    assert_eq!(report["fields"][0]["runs"], 1);
    assert!(
        report["fields"][0]["errors"][0]
            .as_str()
            .unwrap()
            .contains("value 0: ")
    );
    assert_eq!(report["fields"][1]["changed"], json!([[3, 4]]));
    assert_eq!(report["unaffected"], json!([[0, 3]]));
}

#[test]
fn failed_jobs_do_not_contribute_offsets() {
    for arm64 in [true, false] {
        let report = analyze(
            json!({
                "setup":[{"cmd":"load", "path":libc(arm64)}],
                "commands":[
                    {"cmd":"new_string", "value":"strlen"},
                    {"cmd":"string", "handle":{"$ref":0}},
                    {"cmd":"resolve", "at":{"$ref":1}}
                ],
                "output":1,
                "fields":[
                    {"name":"symbol", "command":0, "offset":0, "length":6,
                     "values":[BASE64_STANDARD.encode(b"notfun"), BASE64_STANDARD.encode(b"strlen"), BASE64_STANDARD.encode(b"malloc")]}
                ]
            }),
            2,
        );
        assert_eq!(report["fields"][0]["runs"], 3);
        // The failed "notfun" run must not mark the unchanged 'l' at byte 3.
        assert_eq!(report["fields"][0]["changed"], json!([[0, 3], [4, 6]]));
        assert!(
            report["fields"][0]["errors"][0]
                .as_str()
                .unwrap()
                .starts_with("value 0: ")
        );
        assert!(
            report["fields"][0]["errors"][0]
                .as_str()
                .unwrap()
                .contains("notfun")
        );
        assert_eq!(report["unaffected"], json!([[3, 4]]));
    }
}

#[test]
fn length_changes_include_added_and_removed_bytes_but_index_only_the_base() {
    for arm64 in [true, false] {
        for (input, changed, length, base64, ranges, by_end) in [
            (
                "123456789\0ab",
                "123456789abc",
                3,
                "Ijki",
                json!([[1, 4]]),
                3,
            ),
            (
                "123456789abc",
                "123456789\0ab",
                4,
                "IjEyIg==",
                json!([[1, 4]]),
                4,
            ),
        ] {
            let report = analyze(
                json!({
                    "setup":[
                        {"cmd":"load", "path":libc(arm64)},
                        {"cmd":"find_symbol", "module":{"$ref":0}, "name":"strlen"}
                    ],
                    "commands":[
                        {"cmd":"new_cstring", "value":input},
                        {"cmd":"call_function", "address":{"$setup":1}, "args":[{"$ref":0}]}
                    ],
                    "fields":[{"name":"text", "command":0, "offset":0, "length":12,
                        "values":[BASE64_STANDARD.encode(changed)]}]
                }),
                2,
            );
            assert_eq!(
                report,
                json!({
                    "output":{"length":length, "base64":base64},
                    "fields":[{"name":"text", "runs":1, "changed":ranges, "length_changed":true}],
                    "by_output":[{"start":1, "end":by_end, "fields":["text"]}],
                    "unaffected":[[0,1]]
                })
            );
        }
    }
}
