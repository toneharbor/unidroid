use super::*;

const REFUSED: &[&str] = &[
    "break",
    "unbreak",
    "watch",
    "unwatch",
    "continue",
    "step",
    "finish",
    "return",
    "abort",
    "answer",
    "unhandled",
    "checkpoint",
    "rewind",
    "drop_checkpoint",
];

#[test]
fn driver_commands_are_refused_in_jobs_and_setup() {
    let temp = TempDir::new();
    let jobs = lines(
        &REFUSED
            .iter()
            .map(|cmd| json!({"commands":[{"cmd":cmd}]}))
            .collect::<Vec<_>>(),
    );
    let report = serve::batch(
        &options(),
        &BatchOptions {
            workers: 2,
            ..Default::default()
        },
        Emulator::builder,
        jobs.as_bytes(),
    )
    .unwrap();
    for (job, cmd) in report["jobs"].as_array().unwrap().iter().zip(REFUSED) {
        assert_eq!(job["error"], format!("{cmd} is not available in batch"));
        assert_eq!(job["results"], json!([]));
        let setup = temp.file("setup.jsonl", &lines(&[json!({"cmd":cmd})]));
        let error = serve::batch(
            &options(),
            &BatchOptions {
                setup: Some(setup),
                workers: 2,
                ..Default::default()
            },
            Emulator::builder,
            b"".as_slice(),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("setup command 1: {cmd} is not available in batch")
        );
    }
}

#[test]
fn malformed_jobs_and_bad_references_do_not_stop_other_jobs() {
    let jobs = format!(
        "not json\n{}",
        lines(&[
            json!({"id":null, "commands":[{"cmd":"new_string", "value":"kept"}, {"cmd":"not_a_command"}]}),
            json!({"commands":[{"cmd":"new_string", "value":{"$ref":0}}]}),
            json!({"commands":[{"cmd":"new_string", "value":{"$ref":1}}]}),
            json!({"commands":[{"cmd":"new_string", "value":{"$setup":0}}]}),
            json!({"commands":[{"cmd":"new_string", "value":{"$ref":-1}}]}),
            json!({"commands":[{"cmd":"new_string", "value":{"$ref":"0"}}]}),
            json!({"commands":[{"cmd":"new_string"}]}),
            json!({"id":"missing commands"}),
            json!({"commands":[{"cmd":"new_string", "value":"alive"}, {"cmd":"string", "handle":{"$ref":0}}]}),
        ])
    );
    let report = serve::batch(
        &options(),
        &BatchOptions {
            workers: 3,
            ..Default::default()
        },
        Emulator::builder,
        jobs.as_bytes(),
    )
    .unwrap();
    assert_eq!(report["summary"]["jobs"], 10);
    assert_eq!(report["summary"]["failed"], 9);
    assert_eq!(report["jobs"][0]["id"], 1);
    assert_eq!(report["jobs"][1]["id"], Value::Null);
    assert_eq!(report["jobs"][1]["results"].as_array().unwrap().len(), 1);
    for i in [0, 1, 7, 8] {
        assert!(
            report["jobs"][i]["error"]
                .as_str()
                .unwrap()
                .starts_with("bad job:")
        );
    }
    for i in 2..7 {
        assert!(
            report["jobs"][i]["error"]
                .as_str()
                .unwrap()
                .starts_with("bad reference ")
        );
    }
    assert_eq!(report["jobs"][9]["results"][1], "alive");
}

#[test]
fn nested_references_resolve_but_objects_with_extra_keys_are_literal() {
    let jobs = lines(&[
        json!({"commands":[
            {"cmd":"new_string", "value":"first"},
            {"cmd":"new_object_array", "class":"java/lang/Object", "items":[{"$ref":0}, {"$ref":0}]},
            {"cmd":"object_array", "handle":{"$ref":1}},
            {"cmd":"new_object_array", "class":"java/lang/Object", "items":{"$ref":2}}
        ]}),
        json!({"commands":[{"cmd":"new_string", "value":{"$ref":0, "literal":true}}]}),
    ]);
    let report = serve::batch(
        &options(),
        &BatchOptions {
            workers: 1,
            ..Default::default()
        },
        Emulator::builder,
        jobs.as_bytes(),
    )
    .unwrap();
    assert_eq!(report["jobs"][0]["ok"], true, "{report}");
    let results = &report["jobs"][0]["results"];
    assert_eq!(results[2].as_array().unwrap().len(), 2);
    assert!(results[3].is_u64());
    assert!(
        report["jobs"][1]["error"]
            .as_str()
            .unwrap()
            .starts_with("bad job:")
    );
}
