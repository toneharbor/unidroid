use super::*;

#[test]
fn invalid_fields_name_the_field_and_fail_before_building_workers() {
    for (command, field, expected) in [
        (
            json!({"cmd":"malloc", "size":8}),
            json!({"command":0, "offset":0, "length":1}),
            "payload",
        ),
        (
            json!({"cmd":"new_string", "value":"abc"}),
            json!({"command":1, "offset":0, "length":1}),
            "command",
        ),
        (
            json!({"cmd":"new_string", "value":"abc"}),
            json!({"command":0, "offset":2, "length":2}),
            "range",
        ),
        (
            json!({"cmd":"new_string", "value":"abc"}),
            json!({"command":0, "offset":usize::MAX, "length":1}),
            "range",
        ),
        (
            json!({"cmd":"new_array", "type":"B", "base64":"!!!!"}),
            json!({"command":0, "offset":0, "length":1}),
            "symbol",
        ),
        (
            json!({"cmd":"new_array", "type":"B", "base64":"YQ=="}),
            json!({"command":0, "offset":1, "length":1}),
            "range",
        ),
        (
            json!({"cmd":"new_string", "value":"abc"}),
            json!({"command":0, "offset":0, "length":1, "values":["YWI="]}),
            "value 0: expected 1 bytes, got 2",
        ),
        (
            json!({"cmd":"new_string", "value":"abc"}),
            json!({"command":0, "offset":0, "length":1, "values":["!"]}),
            "value 0:",
        ),
    ] {
        let mut field = field;
        field["name"] = json!("bad_field");
        let error = serve::fields(
            &options(),
            &FieldsOptions { workers: 2 },
            || panic!("a bad field must be rejected before setup"),
            json!({"commands":[command], "fields":[field]}),
        )
        .unwrap_err()
        .to_string();
        assert!(error.starts_with("field bad_field: "), "{error}");
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn missing_output_and_bad_output_index_fail_before_running() {
    for spec in [
        json!({"commands":[], "fields":[]}),
        json!({
            "commands":[{"cmd":"new_string", "value":"abc"}], "output":1, "fields":[]
        }),
    ] {
        let error = serve::fields(
            &options(),
            &FieldsOptions { workers: 1 },
            || panic!("invalid output must be rejected before setup"),
            spec,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(error, "output command out of range");
    }
}

#[test]
fn setup_and_jobs_keep_batch_reference_and_refusal_errors() {
    for (spec, expected) in [
        (
            json!({"setup":[{"cmd":"checkpoint"}], "commands":[{"cmd":"new_string", "value":"a"}], "fields":[]}),
            "setup command 1: checkpoint is not available in batch",
        ),
        (
            json!({"commands":[{"cmd":"answer"}], "fields":[]}),
            "base run: answer is not available in batch",
        ),
        (
            json!({"commands":[{"cmd":"string", "handle":{"$ref":0}}], "fields":[]}),
            "base run: bad reference $ref: 0",
        ),
        (
            json!({"commands":[{"cmd":"string", "handle":{"$setup":0}}], "fields":[]}),
            "base run: bad reference $setup: 0",
        ),
    ] {
        let error = serve::fields(
            &options(),
            &FieldsOptions { workers: 3 },
            Emulator::builder,
            spec,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(error, expected);
    }
}

#[test]
fn nonrepeatable_base_stops_before_any_variant_runs() {
    let temp = TempDir::new();
    for arm64 in [true, false] {
        let base = temp.0.join("base000.snap");
        let changed = temp.0.join("changed.snap");
        let mut spec = cli::random_spec(arm64);
        spec["commands"][0]["value"] = json!(base);
        spec["output"] = json!(5);
        spec["commands"].as_array_mut().unwrap().extend([
            json!({"cmd":"string", "handle":{"$ref":0}}),
            json!({"cmd":"snapshot", "path":{"$ref":6}}),
        ]);
        spec["fields"] = json!([{"name":"path", "command":0,
            "offset":base.to_str().unwrap().len()-12, "length":12,
            "values":[BASE64_STANDARD.encode(b"changed.snap")]}]);
        let mut options = options();
        options.fixed_clock = None;
        options.random_seed = None;
        let error = serve::fields(
            &options,
            &FieldsOptions { workers: 3 },
            Emulator::builder,
            spec,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            error,
            "the unmodified run is not repeatable; use --fixed-clock and --random-seed"
        );
        assert!(base.exists());
        assert!(!changed.exists());
        std::fs::remove_file(base).unwrap();
    }
}
