use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn setup_results_and_guest_memory_are_restored_for_each_job() {
    let temp = TempDir::new();
    for lib in ["rootfs/lib64/libc.so", "rootfs/lib/libc.so"] {
        let setup = temp.file(
            "setup.jsonl",
            &lines(&[
                json!({"cmd":"load", "path":lib}),
                json!({"cmd":"new_cstring", "value":"original"}),
                json!({"cmd":"find_symbol", "module":{"$ref":0}, "name":"strlen"}),
            ]),
        );
        let jobs = lines(&[
            json!({"commands":[{"cmd":"write", "address":{"$setup":1}, "base64":"eAA="}]}),
            json!({"commands":[{"cmd":"call_function", "address":{"$setup":2}, "args":[{"$setup":1}]}]}),
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
        assert_eq!(report["summary"]["ok"], 2, "{report}");
        assert_eq!(report["jobs"][1]["results"], json!(["8"]));
    }
}

#[test]
fn setups_without_libraries_rerun_on_every_attempt() {
    let temp = TempDir::new();
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        for setup in [
            None,
            Some(temp.file(
                "setup.jsonl",
                &lines(&[
                    json!({"cmd":"new_string", "value":"prepared"}),
                    json!({"cmd":"string", "handle":{"$ref":0}}),
                ]),
            )),
        ] {
            let mut commands = json!({"commands":[
                {"cmd":"load", "path":format!("tests/fixtures/{abi}/libjnidispatch.so")},
                {"cmd":"call_static", "class":"com/sun/jna/Native", "method":"sizeof(I)I", "args":[0]}
            ]});
            if setup.is_some() {
                commands["commands"].as_array_mut().unwrap().extend([
                    json!({"cmd":"string", "handle":{"$setup":0}}),
                    json!({"cmd":"new_string", "value":{"$setup":1}}),
                ]);
            }
            let jobs = lines(&[commands.clone(), commands]);
            let builds = Arc::new(AtomicUsize::new(0));
            let report = serve::batch(
                &options(),
                &BatchOptions {
                    setup,
                    workers: 1,
                    ..Default::default()
                },
                || {
                    builds.fetch_add(1, Ordering::Relaxed);
                    Emulator::builder()
                },
                jobs.as_bytes(),
            )
            .unwrap();
            assert_eq!(report["summary"]["ok"], 2, "{report}");
            for job in report["jobs"].as_array().unwrap() {
                if job["results"].as_array().unwrap().len() == 4 {
                    assert_eq!(job["results"][2], "prepared");
                }
                assert_eq!(job["results"][0], 0);
                assert_eq!(job["results"][1], if abi == "arm64-v8a" { 8 } else { 4 });
            }
            assert_eq!(builds.load(Ordering::Relaxed), 3);
        }
    }
}

#[test]
fn one_workers_failed_setup_prevents_every_job() {
    use std::sync::{Mutex, mpsc};
    use std::time::Duration;
    let (finished, completion) = mpsc::channel();
    std::thread::spawn(move || {
        for (lib, getpid) in [("rootfs/lib64/libc.so", 172), ("rootfs/lib/libc.so", 20)] {
            let temp = TempDir::new();
            let snapshot = temp.0.join("job.snapshot");
            let setup = temp.file(
                "setup.jsonl",
                &lines(&[
                    json!({"cmd":"load", "path":lib}),
                    json!({"cmd":"find_symbol", "module":{"$ref":0}, "name":"syscall"}),
                ]),
            );
            let jobs = lines(&[json!({"commands":[
                {"cmd":"snapshot", "path":snapshot},
                {"cmd":"call_function", "address":{"$setup":1}, "args":[getpid.to_string()]}
            ]})]);
            let builds = AtomicUsize::new(0);
            let (sent, received) = mpsc::channel();
            let received = Mutex::new(received);
            let error = serve::batch(
                &options(),
                &BatchOptions {
                    setup: Some(setup),
                    workers: 4,
                    ..Default::default()
                },
                || {
                    let second = builds.fetch_add(1, Ordering::SeqCst) == 1;
                    if second {
                        // Let healthy workers finish setup before this worker fails.
                        // An early job signals us after writing its snapshot.
                        let _ = received
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(1));
                    }
                    let sent = sent.clone();
                    Emulator::builder().trace(unidroid::Trace {
                    syscalls: true,
                    code: if second { vec!["libc.so+bad".into()] } else { vec![] },
                    ..Default::default()
                }).on_event(move |event| {
                    if matches!(event, unidroid::Event::Syscall { name, .. } if name == "getpid") {
                        let _ = sent.send(());
                    }
                })
                },
                jobs.as_bytes(),
            )
            .unwrap_err();
            assert!(error.to_string().contains("invalid code trace"), "{error}");
            assert_eq!(builds.load(Ordering::SeqCst), 4);
            assert!(
                !snapshot.exists(),
                "a job ran before every worker finished setup"
            );
        }
        finished.send(()).unwrap();
    });
    // Removing the setup wait can strand healthy workers at the preflight wait.
    completion
        .recv_timeout(Duration::from_secs(10))
        .expect("batch did not finish after a worker failed setup");
}

#[test]
fn malformed_setup_commands_report_their_position() {
    let temp = TempDir::new();
    for (command, expected) in [
        (
            json!({"cmd":"string", "handle":{"$ref":8}}).to_string(),
            "setup command 2: bad reference $ref: 8",
        ),
        (
            "{".into(),
            "setup command 2: bad command: EOF while parsing an object at line 1 column 1",
        ),
        (
            json!({"cmd":"new_string"}).to_string(),
            "setup command 2: bad command: missing field `value`",
        ),
    ] {
        let setup = temp.file(
            "setup.jsonl",
            &format!(
                "{}{}\n",
                lines(&[json!({"cmd":"new_string", "value":"one"})]),
                command
            ),
        );
        let error = serve::batch(
            &options(),
            &BatchOptions {
                setup: Some(setup),
                workers: 4,
                ..Default::default()
            },
            Emulator::builder,
            b"{\"commands\":[]}\n".as_slice(),
        )
        .unwrap_err();
        assert_eq!(error.to_string(), expected);
    }
}
