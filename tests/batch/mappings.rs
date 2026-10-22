use super::*;

#[test]
fn diff_reports_new_and_removed_mappings_as_decimal_ranges() {
    let temp = TempDir::new();
    for lib in ["rootfs/lib64/libc.so", "rootfs/lib/libc.so"] {
        let args = json!(["0", "4096", "3", "34", "4294967295", "0"]);
        let setup = temp.file(
            "setup.jsonl",
            &lines(&[
                json!({"cmd":"load", "path":lib}),
                json!({"cmd":"find_symbol", "module":{"$ref":0}, "name":"mmap"}),
                json!({"cmd":"find_symbol", "module":{"$ref":0}, "name":"munmap"}),
                json!({"cmd":"call_function", "address":{"$ref":1}, "args":args}),
            ]),
        );
        let jobs = lines(&[json!({"diff":true, "commands":[
            {"cmd":"call_function", "address":{"$setup":1}, "args":args},
            {"cmd":"call_function", "address":{"$setup":2}, "args":[{"$setup":3}, "4096"]}
        ]})]);
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
        let job = &report["jobs"][0];
        assert_eq!(job["ok"], true, "{job}");
        assert_eq!(job["results"][1], "0");
        let diff = &job["diff"];
        for name in ["mapped", "unmapped"] {
            let ranges = diff[name].as_array().unwrap();
            assert_eq!(ranges.len(), 1);
            let range = &ranges[0];
            assert_eq!(range.as_object().unwrap().len(), 2);
            let start = range["start"].as_str().unwrap().parse::<u64>().unwrap();
            let end = range["end"].as_str().unwrap().parse::<u64>().unwrap();
            assert_eq!(end - start, 4096);
        }
        assert_eq!(diff["mapped"][0]["start"], job["results"][0]);
    }
}

#[test]
fn fresh_jobs_can_diff_the_first_library_load() {
    for lib in ["rootfs/lib64/libc.so", "rootfs/lib/libc.so"] {
        let jobs = lines(&[
            json!({"diff":true, "commands":[]}),
            json!({"diff":true, "commands":[{"cmd":"load", "path":lib}]}),
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
        assert_eq!(report["summary"]["ok"], 2);
        assert_eq!(
            report["jobs"][0]["diff"],
            json!({"registers":{}, "changes":[], "mapped":[], "unmapped":[]})
        );
        let diff = &report["jobs"][1]["diff"];
        assert!(!diff["mapped"].as_array().unwrap().is_empty());
        assert_eq!(diff["registers"], json!({}));
        assert_eq!(diff["unmapped"], json!([]));
    }
}

#[test]
fn requested_coverage_adds_to_setup_trace_settings() {
    let temp = TempDir::new();
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let setup = temp.file(
            "setup.jsonl",
            &lines(&[
                json!({"cmd":"load", "path":format!("tests/fixtures/{abi}/libjnidispatch.so")}),
                json!({"cmd":"trace", "coverage":["libc.so"]}),
            ]),
        );
        let commands = json!([
            {"cmd":"call_static", "class":"com/sun/jna/Native", "method":"malloc(J)J", "args":["16"]},
            {"cmd":"coverage"}
        ]);
        let jobs = lines(&[
            json!({"coverage":["libjnidispatch.so"], "commands":commands}),
            json!({"commands":commands}),
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
        assert_eq!(report["summary"]["ok"], 2);
        let blocks = report["jobs"][0]["coverage"].as_array().unwrap();
        for name in ["libc.so+", "libjnidispatch.so+"] {
            assert!(
                blocks
                    .iter()
                    .any(|block| block["location"].as_str().unwrap().starts_with(name))
            );
        }
        let next = report["jobs"][1]["results"][1].as_array().unwrap();
        assert!(!next.is_empty());
        assert!(
            next.iter()
                .all(|block| block["location"].as_str().unwrap().starts_with("libc.so+"))
        );
    }
}
