use super::*;
use base64::{Engine, prelude::BASE64_STANDARD};
use unidroid::Trace;

#[test]
fn coverage_is_counted_per_job_and_matches_direct_calls() {
    let temp = TempDir::new();
    for lib in ["rootfs/lib64/libc.so", "rootfs/lib/libc.so"] {
        let setup = temp.file(
            "setup.jsonl",
            &lines(&[
                json!({"cmd":"load", "path":lib}),
                json!({"cmd":"find_symbol", "module":{"$ref":0}, "name":"strlen"}),
            ]),
        );
        let inputs = ["a".to_string(), "b".repeat(512), "a".to_string()];
        let jobs = lines(
            &inputs
                .iter()
                .map(|input| {
                    json!({"coverage":["libc.so"], "commands":[
                        {"cmd":"new_cstring", "value":input},
                        {"cmd":"call_function", "address":{"$setup":1}, "args":[{"$ref":0}]},
                        {"cmd":"coverage"}
                    ]})
                })
                .collect::<Vec<_>>(),
        );
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
        let mut emu = options().build(Emulator::builder()).unwrap();
        let module = emu.load_library(lib).unwrap();
        let strlen = emu.find_symbol(module, "strlen").unwrap();
        let checkpoint = emu.checkpoint().unwrap();
        for (index, input) in inputs.iter().enumerate() {
            emu.rewind(&checkpoint).unwrap();
            emu.set_trace(Trace {
                coverage: vec!["libc.so".into()],
                ..Default::default()
            })
            .unwrap();
            emu.reset_coverage();
            let string = emu.new_cstring(input).unwrap();
            assert_eq!(
                emu.call_function(strlen, &[string]).unwrap() as usize,
                input.len()
            );
            let expected: Vec<_> = emu.coverage().iter().map(|b| json!({"address":b.address.to_string(), "location":b.location, "count":b.count})).collect();
            let job = &report["jobs"][index];
            assert_eq!(job["ok"], true);
            assert_eq!(job["coverage"], json!(expected));
            assert_eq!(job["results"][2], job["coverage"]);
        }
        assert_ne!(report["jobs"][0]["coverage"], report["jobs"][1]["coverage"]);
        assert_eq!(report["jobs"][0]["coverage"], report["jobs"][2]["coverage"]);
    }
}

#[test]
fn job_trace_settings_do_not_leak_to_the_next_job() {
    let temp = TempDir::new();
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let setup = temp.file(
            "setup.jsonl",
            &lines(&[
                json!({"cmd":"load", "path":format!("tests/fixtures/{abi}/libjnidispatch.so")}),
            ]),
        );
        let call = json!({"cmd":"call_static", "class":"com/sun/jna/Native", "method":"sizeof(I)I", "args":[0]});
        let jobs = lines(&[
            json!({"commands":[{"cmd":"trace", "coverage":["libjnidispatch.so"]}, call, {"cmd":"coverage"}]}),
            json!({"commands":[call, {"cmd":"coverage"}]}),
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
        assert!(
            !report["jobs"][0]["results"][2]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(report["jobs"][1]["results"][1], json!([]));
    }
}

#[test]
fn diffs_use_decimal_registers_and_base64_bytes_per_job_even_on_failure() {
    let temp = TempDir::new();
    for (lib, register) in [("rootfs/lib64/libc.so", "x0"), ("rootfs/lib/libc.so", "r0")] {
        let setup = temp.file(
            "setup.jsonl",
            &lines(&[
                json!({"cmd":"load", "path":lib}),
                json!({"cmd":"new_cstring", "value":"aaaaaaa"}),
                json!({"cmd":"set_register", "name":register, "value":"7"}),
            ]),
        );
        let job = json!({"diff":true, "commands":[
            {"cmd":"write", "address":{"$setup":1}, "base64":"AQIDBA=="},
            {"cmd":"set_register", "name":register, "value":"42"},
            {"cmd":"read", "address":{"$setup":1}, "size":4}
        ]});
        let mut failed = job.clone();
        failed["commands"]
            .as_array_mut()
            .unwrap()
            .push(json!({"cmd":"string", "handle":999999}));
        let jobs = lines(&[job.clone(), failed, job]);
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
        for job in report["jobs"].as_array().unwrap() {
            let diff = &job["diff"];
            assert_eq!(diff["registers"], json!({register:["7","42"]}));
            assert_eq!(diff["mapped"], json!([]));
            assert_eq!(diff["unmapped"], json!([]));
            let changes = diff["changes"].as_array().unwrap();
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[0]["size"], 4);
            assert_eq!(
                BASE64_STANDARD
                    .decode(changes[0]["after"].as_str().unwrap())
                    .unwrap(),
                [1, 2, 3, 4]
            );
            let address = changes[0]["address"]
                .as_str()
                .unwrap()
                .parse::<u64>()
                .unwrap();
            assert_eq!(
                changes[0]["location"],
                format!("0x{address:x} [host malloc]")
            );
        }
        assert_eq!(report["jobs"][0]["diff"], report["jobs"][2]["diff"]);
    }
}

pub(super) fn without_times(mut report: Value) -> Value {
    report["summary"].as_object_mut().unwrap().remove("ms");
    for job in report["jobs"].as_array_mut().unwrap() {
        assert!(job["ms"].is_u64());
        job.as_object_mut().unwrap().remove("ms");
    }
    report
}

#[test]
fn repeated_batches_are_identical_except_for_elapsed_time() {
    let temp = TempDir::new();
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let setup = temp.file(
            "setup.jsonl",
            &lines(&[
                json!({"cmd":"load", "path":format!("tests/fixtures/{abi}/libjnidispatch.so")}),
            ]),
        );
        let jni = json!({"commands":[{"cmd":"onload", "module":0}]});
        let crash = json!({"commands":[{"cmd":"call_function", "address":"0", "args":[]}]});
        let jobs = lines(&[crash.clone(), jni.clone(), crash.clone(), jni, crash]);
        let settings = BatchOptions {
            setup: Some(setup),
            workers: 1,
            ..Default::default()
        };
        let run = |settings: &BatchOptions| {
            without_times(
                serve::batch(&options(), settings, Emulator::builder, jobs.as_bytes()).unwrap(),
            )
        };
        let first = run(&settings);
        assert_eq!(first, run(&settings));
        let parallel = BatchOptions {
            workers: 4,
            ..settings
        };
        assert_eq!(first, run(&parallel));
        assert_eq!(first, run(&parallel));
        assert_eq!(first["jobs"][0]["crash"], first["jobs"][2]["crash"]);
        assert_eq!(first["jobs"][0]["crash"], first["jobs"][4]["crash"]);
    }
}
