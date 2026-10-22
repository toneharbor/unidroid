use super::*;

#[test]
fn policies_restore_setup_positions_and_job_overrides_start_at_zero() {
    let temp = TempDir::new();
    let policy = temp.file("policy.json", &json!({"rules":[
        {"op":"get", "class":"java/lang/Void", "name":"TYPE", "values":[null, {"object":"java/lang/Class"}]}
    ]}).to_string());
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let setup = temp.file(
            "setup.jsonl",
            &lines(&[
                json!({"cmd":"load", "path":format!("tests/fixtures/{abi}/libjnidispatch.so")}),
                json!({"cmd":"onload", "module":{"$ref":0}}),
            ]),
        );
        let commands = json!([{"cmd":"onload", "module":{"$setup":0}}]);
        let jobs = lines(&[
            json!({"commands":commands}),
            json!({"commands":commands, "policy":policy}),
            json!({"commands":commands, "policy":policy}),
            json!({"commands":commands}),
        ]);
        let mut opts = options();
        opts.policy = Some(policy.clone());
        let report = serve::batch(
            &opts,
            &BatchOptions {
                setup: Some(setup),
                workers: 1,
                ..Default::default()
            },
            Emulator::builder,
            jobs.as_bytes(),
        )
        .unwrap();
        assert_eq!(report["summary"]["ok"], 2, "{report}");
        for index in [0, 3] {
            assert!(
                report["jobs"][index]["error"]
                    .as_str()
                    .unwrap()
                    .contains("java/lang/Boolean->TYPE"),
                "{report}"
            );
            assert!(report["jobs"][index]["crash"]["registers"]["pc"].is_string());
        }
        for index in [1, 2] {
            assert_eq!(report["jobs"][index]["results"], json!([0]));
        }
    }
}

#[test]
fn policy_paths_are_loaded_once_per_worker_even_when_the_file_disappears() {
    let temp = TempDir::new();
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        for override_policy in [false, true] {
            let policy = temp.file(
                "once.json",
                r#"{"rules":[{"op":"get","class":"java/lang/Void","values":[null]}]}"#,
            );
            let setup = temp.file(
                "setup.jsonl",
                &lines(&[
                    json!({"cmd":"load", "path":format!("tests/fixtures/{abi}/libjnidispatch.so")}),
                ]),
            );
            let mut job =
                json!({"commands":[{"cmd":"trace", "jni":true}, {"cmd":"onload", "module":0}]});
            let mut opts = options();
            if override_policy {
                job["policy"] = json!(policy);
            } else {
                opts.policy = Some(policy.clone());
            }
            let jobs = lines(&[job.clone(), job]);
            let report = serve::batch(
                &opts,
                &BatchOptions {
                    setup: Some(setup),
                    workers: 1,
                    ..Default::default()
                },
                || {
                    let policy = policy.clone();
                    Emulator::builder().on_event(move |event| {
                        if matches!(event, unidroid::Event::Jni { .. }) && policy.exists() {
                            std::fs::remove_file(&policy).unwrap();
                        }
                    })
                },
                jobs.as_bytes(),
            )
            .unwrap();
            assert!(!policy.exists());
            assert_eq!(report["summary"]["ok"], 2, "{}", report["jobs"]);
        }
    }
}

#[test]
fn unanswered_jni_and_policy_errors_preserve_serve_crashes() {
    let temp = TempDir::new();
    let policy = temp.file("wrong.json", r#"{"rules":[{"value":42}]}"#);
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let setup = temp.file(
            "setup.jsonl",
            &lines(&[
                json!({"cmd":"load", "path":format!("tests/fixtures/{abi}/libjnidispatch.so")}),
            ]),
        );
        let jobs = lines(&[
            json!({"commands":[{"cmd":"onload", "module":0}, {"cmd":"new_string", "value":"never"}]}),
            json!({"policy":policy, "commands":[{"cmd":"onload", "module":0}]}),
            json!({"commands":[{"cmd":"new_string", "value":"alive"}, {"cmd":"string", "handle":{"$ref":0}}]}),
        ]);
        let report = serve::batch(
            &options(),
            &BatchOptions {
                setup: Some(setup),
                workers: 1,
                ..Default::default()
            },
            Emulator::builder,
            jobs.as_bytes(),
        )
        .unwrap();
        assert_eq!(report["summary"]["ok"], 1);
        assert!(
            report["jobs"][0]["error"]
                .as_str()
                .unwrap()
                .starts_with("unhandled JNI call:")
        );
        assert!(
            report["jobs"][1]["error"]
                .as_str()
                .unwrap()
                .starts_with("policy rule 0")
        );
        for index in [0, 1] {
            let job = &report["jobs"][index];
            assert!(
                job["error"]
                    .as_str()
                    .unwrap()
                    .contains("from libjnidispatch.so+")
            );
            assert!(
                job["crash"]["message"]
                    .as_str()
                    .unwrap()
                    .starts_with("unhandled JNI call:")
            );
            assert!(job["crash"]["jni"].as_array().unwrap().len() > 1);
            assert_eq!(job["results"], json!([]));
        }
        assert_eq!(report["jobs"][2]["results"][1], "alive");
    }
}
