use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use unidroid::Event;

#[test]
fn retry_recovers_when_a_host_snapshot_directory_becomes_available() {
    let temp = TempDir::new();
    for (index, lib) in ["rootfs/lib64/libc.so", "rootfs/lib/libc.so"]
        .into_iter()
        .enumerate()
    {
        for checkpointed in [false, true] {
            let directory = temp.0.join(format!("later-{index}-{checkpointed}"));
            let preparation = [
                json!({"cmd":"load", "path":lib}),
                json!({"cmd":"find_symbol", "module":{"$ref":0}, "name":"getuid"}),
            ];
            let setup = checkpointed.then(|| temp.file("setup.jsonl", &lines(&preparation)));
            let mut commands = if checkpointed {
                Vec::new()
            } else {
                preparation.to_vec()
            };
            commands.extend([
            json!({"cmd":"trace", "syscalls":true}),
            json!({"cmd":"call_function", "address":if checkpointed {json!({"$setup":1})} else {json!({"$ref":1})}, "args":[]}),
            json!({"cmd":"snapshot", "path":directory.join("after.snap")}),
        ]);
            let jobs = lines(&[json!({"commands":commands})]);
            let calls = Arc::new(AtomicUsize::new(0));
            let report = serve::batch(
                &options(),
                &BatchOptions {
                    setup,
                    workers: 1,
                    retries: 2,
                    ..Default::default()
                },
                || {
                    let calls = calls.clone();
                    let directory = directory.clone();
                    Emulator::builder().on_event(move |event| {
                        if matches!(event, Event::Syscall { .. })
                            && calls.fetch_add(1, Ordering::Relaxed) == 1
                        {
                            // A controlled host-side change: attempt one cannot save;
                            // attempt two can. Policy positions never advance across retries.
                            std::fs::create_dir(&directory).unwrap();
                        }
                    })
                },
                jobs.as_bytes(),
            )
            .unwrap();
            let job = &report["jobs"][0];
            assert_eq!(job["ok"], true, "{report}");
            assert_eq!(job["attempts"], 2);
            assert_eq!(job["results"].as_array().unwrap().len(), commands.len());
            assert!(job.get("error").is_none());
            assert!(job.get("crash").is_none());
        }
    }
}

#[test]
fn retry_restores_policy_sequences_and_only_filters_the_report() {
    let temp = TempDir::new();
    let policy = temp.file(
        "policy.json",
        &json!({"rules":[
            {"op":"get", "class":"java/lang/Void", "values":[42, null]}
        ]})
        .to_string(),
    );
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let setup = temp.file(
            "setup.jsonl",
            &lines(&[
                json!({"cmd":"load", "path":format!("tests/fixtures/{abi}/libjnidispatch.so")}),
            ]),
        );
        let jobs = lines(&[
            json!({"id":"bad", "policy":policy, "commands":[{"cmd":"onload", "module":0}]}),
            json!({"id":"good", "commands":[]}),
        ]);
        for only in [serve::BatchOnly::Failed, serve::BatchOnly::Ok] {
            let report = serve::batch(
                &options(),
                &BatchOptions {
                    setup: Some(setup.clone()),
                    workers: 1,
                    retries: 2,
                    only: Some(only),
                },
                Emulator::builder,
                jobs.as_bytes(),
            )
            .unwrap();
            assert_eq!(report["summary"]["jobs"], 2);
            assert_eq!(report["summary"]["failed"], 1);
            assert_eq!(report["summary"]["ok"], 1);
            assert_eq!(report["jobs"].as_array().unwrap().len(), 1);
            let failed = matches!(only, serve::BatchOnly::Failed);
            assert_eq!(report["jobs"][0]["id"], if failed { "bad" } else { "good" });
            assert_eq!(report["jobs"][0]["attempts"], if failed { 3 } else { 1 });
        }
    }
}
