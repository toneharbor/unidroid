use super::TestRoot;
use base64::{Engine, prelude::BASE64_STANDARD};
use serde_json::{Value, json};
use unidroid::{Emulator, Result};

fn syscall(commands: &mut Vec<Value>, arm64: bool, nr: u64, args: &[u64]) -> usize {
    commands.push(
        json!({"cmd":"set_register", "name":if arm64 {"x8"} else {"r7"}, "value":nr.to_string()}),
    );
    commands.push(json!({"cmd":"call_function", "address":"131328", "args":args.iter().map(u64::to_string).collect::<Vec<_>>()}));
    commands.len() - 1
}

#[test]
fn file_stat_times_record_and_replay_without_a_fixed_clock() -> Result<()> {
    let root = TestRoot::new("stat-recording")?;
    for (dir, arm64, size) in [("lib64", true, 128), ("lib", false, 104)] {
        for fixed in [None, Some(123)] {
            let options: unidroid::serve::Options = serde_json::from_value(json!({
                "rootfs":"rootfs", "apk":null, "process_name":null, "prop":[], "alias":[], "file":[], "superclass":[],
                "fixed_clock":fixed, "random_seed":null, "translation_cache_mb":8, "no_threads":false,
            }))?;
            let words: [u32; 2] = if arm64 {
                [0xd4000001, 0xd65f03c0]
            } else {
                [0xef000000, 0xe12fff1e]
            };
            let path = 0x70001000u64;
            let generated = path + 64;
            let buffer = path + 256;
            let mut commands = vec![
                json!({"cmd":"load", "path":format!("rootfs/{dir}/libc.so")}),
                json!({"cmd":"write", "address":"131328", "base64":BASE64_STANDARD.encode(words.into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>())}),
                json!({"cmd":"write", "address":path.to_string(), "base64":BASE64_STANDARD.encode(b"/overlay\0")}),
                json!({"cmd":"write", "address":generated.to_string(), "base64":BASE64_STANDARD.encode(b"/proc/self/cmdline\0")}),
            ];
            let mut expected = vec![(
                syscall(
                    &mut commands,
                    arm64,
                    if arm64 { 56 } else { 322 },
                    &[0, path, 0x42, 0o600],
                ),
                "3",
            )];
            expected.push((
                syscall(
                    &mut commands,
                    arm64,
                    if arm64 { 64 } else { 4 },
                    &[3, path, 1],
                ),
                "1",
            ));
            expected.push((
                syscall(
                    &mut commands,
                    arm64,
                    if arm64 { 48 } else { 334 },
                    &[0, path, 0],
                ),
                "0",
            ));
            let mut reads = Vec::new();
            for (index, name) in [path, path, generated, generated, path, path]
                .into_iter()
                .enumerate()
            {
                if index == 1 {
                    commands.push(json!({"cmd":"checkpoint"}));
                }
                if index == 4 {
                    expected.push((
                        syscall(
                            &mut commands,
                            arm64,
                            if arm64 { 64 } else { 4 },
                            &[3, path, 1],
                        ),
                        "1",
                    ));
                }
                if index == 5 {
                    commands.push(json!({"cmd":"rewind", "id":1}));
                }
                let (nr, args) = if index >= 4 {
                    (if arm64 { 80 } else { 197 }, vec![3, buffer])
                } else {
                    (if arm64 { 79 } else { 327 }, vec![0, name, buffer, 0])
                };
                expected.push((syscall(&mut commands, arm64, nr, &args), "0"));
                reads.push(commands.len());
                commands.push(json!({"cmd":"read", "address":buffer.to_string(), "size":size}));
            }
            let input: String = commands.iter().map(|c| format!("{c}\n")).collect();
            let recording = root.0.join(format!("{dir}-{}.jsonl", fixed.is_some()));
            let mut output = Vec::new();
            unidroid::serve::record(
                &options,
                Emulator::builder(),
                std::fs::File::create(&recording)?,
                input.as_bytes(),
                &mut output,
            )?;
            let responses: Vec<Value> = String::from_utf8(output)?
                .lines()
                .map(serde_json::from_str)
                .collect::<std::result::Result<_, _>>()?;
            for (index, result) in expected {
                assert_eq!(responses[index], json!({"ok":result}));
            }
            assert_eq!(responses[reads[0]], responses[reads[1]]);
            assert_eq!(responses[reads[0]], responses[reads[5]]);
            let text = std::fs::read_to_string(&recording)?;
            // Replaying also compares the stat buffers read from guest memory.
            assert_eq!(
                unidroid::serve::replay(text.as_bytes(), Emulator::builder())?,
                commands.len()
            );
            let entries: Vec<Value> = text
                .lines()
                .map(serde_json::from_str)
                .collect::<std::result::Result<_, _>>()?;
            let times: Vec<_> = entries
                .iter()
                .filter_map(|entry| entry.get("time"))
                .collect();
            assert_eq!(times.len(), if fixed.is_some() { 0 } else { 4 });
            assert!(times.iter().all(|time| time["clock"] == "realtime"));
        }
    }
    Ok(())
}
