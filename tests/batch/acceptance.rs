use super::*;
use std::time::Instant;
use unidroid::JniValue;

#[test]
fn thousand_jna_inputs_match_direct_calls_on_both_architectures() {
    let temp = TempDir::new();
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let lib = format!("tests/fixtures/{abi}/libjnidispatch.so");
        let setup = temp.file("setup.jsonl", &lines(&[json!({"cmd":"load", "path":lib})]));
        let inputs: Vec<_> = (1..=1000).map(|i| i * 16).collect();
        let jobs = lines(&inputs.iter().map(|input| json!({"id":input, "commands":[
            {"cmd":"call_static", "class":"com/sun/jna/Native", "method":"malloc(J)J", "args":[input.to_string()]},
            {"cmd":"call_static", "class":"com/sun/jna/Native", "method":"malloc(J)J", "args":["16"]}
        ]})).collect::<Vec<_>>());
        let mut emu = options().build(Emulator::builder()).unwrap();
        emu.load_library(&lib).unwrap();
        let checkpoint = emu.checkpoint().unwrap();
        let expected: Vec<_> = inputs
            .iter()
            .map(|&input| {
                emu.rewind(&checkpoint).unwrap();
                [input, 16].map(|size| {
                    emu.call_static_jni_method(
                        &mut (),
                        "com/sun/jna/Native",
                        "malloc(J)J",
                        &[JniValue::Long(size)],
                    )
                    .unwrap()
                    .long()
                    .to_string()
                })
            })
            .collect();
        // The second allocation's address changes with the first input's size.
        assert_ne!(expected[0][1], expected[999][1]);
        let start = Instant::now();
        let report = serve::batch(
            &options(),
            &BatchOptions {
                setup: Some(setup),
                workers: 4,
                ..Default::default()
            },
            Emulator::builder,
            jobs.as_bytes(),
        )
        .unwrap();
        eprintln!(
            "1000-job acceptance {abi}: {:?}, 4 workers",
            start.elapsed()
        );
        assert_eq!(report["summary"]["jobs"], 1000);
        assert_eq!(report["summary"]["ok"], 1000, "{}", report["summary"]);
        assert_eq!(report["summary"]["failed"], 0);
        for (index, job) in report["jobs"].as_array().unwrap().iter().enumerate() {
            assert_eq!(job["id"], inputs[index]);
            assert_eq!(
                job["results"],
                json!(expected[index]),
                "{abi}, input {}",
                inputs[index]
            );
            assert_eq!(job["attempts"], 1);
            assert!(job.get("error").is_none());
            assert!(job.get("crash").is_none());
            assert!(job.get("coverage").is_none());
            assert!(job.get("diff").is_none());
        }
    }
}
