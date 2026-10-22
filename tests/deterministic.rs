use unidroid::{Emulator, EmulatorBuilder, Result, Trace};

#[path = "support/thread.rs"]
mod thread;
#[path = "support/tmessages.rs"]
mod tmessages;

const CODE: u64 = 0x20100;

fn emulator(builder: EmulatorBuilder, arm64: bool) -> Result<Emulator> {
    let mut emu = builder.build()?;
    emu.load_library(format!(
        "rootfs/{}/libc.so",
        if arm64 { "lib64" } else { "lib" }
    ))?;
    emu.write_bytes(CODE, &syscall_bytes(arm64))?;
    Ok(emu)
}

fn syscall_bytes(arm64: bool) -> Vec<u8> {
    // svc #0; ret / bx lr. Arguments use the normal function-call registers.
    let words: [u32; 2] = if arm64 {
        [0xd4000001, 0xd65f03c0]
    } else {
        [0xef000000, 0xe12fff1e]
    };
    words.into_iter().flat_map(u32::to_le_bytes).collect()
}

fn syscall(emu: &mut Emulator, arm64: bool, numbers: (u64, u64), args: &[u64]) -> Result<u64> {
    emu.set_register(
        if arm64 { "x8" } else { "r7" },
        if arm64 { numbers.0 } else { numbers.1 },
    )?;
    emu.call_function(CODE, args)
}

fn word(emu: &Emulator, address: u64, width: usize) -> Result<u64> {
    let mut bytes = [0; 8];
    bytes[..width].copy_from_slice(&emu.read_bytes(address, width)?);
    Ok(u64::from_le_bytes(bytes))
}

#[test]
fn random_syscalls_and_devices_share_a_seeded_byte_stream() -> Result<()> {
    fn stream(arm64: bool, seed: u64, split: bool) -> Result<Vec<u8>> {
        let mut emu = emulator(Emulator::builder().random_seed(seed), arm64)?;
        let out = emu.malloc(24)?;
        if split {
            assert_eq!(
                syscall(&mut emu, arm64, (278, 384), &[out, 3, u64::MAX])?,
                3
            );
            let mut offset = 3;
            for (path, count) in [("/dev/urandom", 5), ("/dev/random", 7)] {
                let path = emu.new_cstring(path)?;
                let fd = syscall(&mut emu, arm64, (56, 322), &[0, path, 0])?;
                assert_eq!(
                    syscall(&mut emu, arm64, (63, 3), &[fd, out + offset, count])?,
                    count
                );
                offset += count;
            }
            assert_eq!(
                syscall(&mut emu, arm64, (278, 384), &[out + offset, 0, 0])?,
                0
            );
            assert_eq!(
                syscall(&mut emu, arm64, (278, 384), &[out + offset, 9, 0])?,
                9
            );
        } else {
            assert_eq!(syscall(&mut emu, arm64, (278, 384), &[out, 24, 0])?, 24);
        }
        emu.read_bytes(out, 24)
    }
    // SplitMix64's first three outputs for seed zero, in little-endian order.
    let expected = [
        0xaf, 0xcd, 0x1d, 0x7b, 0x39, 0xa8, 0x20, 0xe2, 0xf4, 0x65, 0xb9, 0xa1, 0x6a, 0x9e, 0x78,
        0x6e, 0x4f, 0x45, 0x09, 0x80, 0x18, 0x5d, 0xc4, 0x06,
    ];
    for arm64 in [true, false] {
        assert_eq!(stream(arm64, 0, false)?, expected);
        assert_eq!(stream(arm64, 0, true)?, expected);
        assert_ne!(stream(arm64, 1, true)?, expected);
    }
    Ok(())
}

#[test]
fn getrandom_without_a_seed_returns_the_requested_count() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(Emulator::builder(), arm64)?;
        let out = emu.malloc(32)?;
        for count in [0, 17] {
            emu.write_bytes(out, &[0xa5; 32])?;
            assert_eq!(
                syscall(&mut emu, arm64, (278, 384), &[out, count, u64::MAX])?,
                count
            );
            assert_eq!(
                emu.read_bytes(out + count, 32 - count as usize)?,
                vec![0xa5; 32 - count as usize]
            );
        }
    }
    Ok(())
}

#[test]
fn stat_and_fstat_fix_metadata_and_hash_the_original_guest_path() -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    for arm64 in [true, false] {
        let mut emu = emulator(Emulator::builder().fixed_clock(-123), arm64)?;
        let metadata = std::fs::metadata("rootfs/lib/libc.so")?;
        let out = emu.malloc(144)?;
        for (path, inode) in [
            ("/lib/libc.so", 0x834e0abd3c3c269e),
            ("lib/./libc.so", 0xf8082b640c10ed26),
        ] {
            let path = emu.new_cstring(path)?;
            let fd = syscall(&mut emu, arm64, (56, 322), &[0, path, 0])?;
            for stat in [true, false] {
                emu.write_bytes(out, &[0xa5; 144])?;
                let (numbers, args) = if stat {
                    ((79, 327), vec![0, path, out, 0])
                } else {
                    // fstat must retain the open path, not the guest's mutable buffer.
                    emu.write_bytes(path, b"/missing\0")?;
                    ((80, 197), vec![fd, out])
                };
                assert_eq!(syscall(&mut emu, arm64, numbers, &args)?, 0);
                for (offset, expected) in [
                    (0, 0),
                    (if arm64 { 8 } else { 96 }, inode),
                    (32, metadata.rdev()),
                    (48, metadata.size()),
                    (64, metadata.size().div_ceil(512)),
                ] {
                    assert_eq!(word(&emu, out + offset, 8)?, expected);
                }
                for (offset, expected) in [
                    (16, metadata.mode()),
                    (20, metadata.nlink() as u32),
                    (24, 0),
                    (28, 0),
                    (56, 4096),
                ] {
                    assert_eq!(word(&emu, out + offset, 4)?, expected as u64);
                }
                if !arm64 {
                    assert_eq!(word(&emu, out + 12, 4)?, inode as u32 as u64);
                }
                let width = if arm64 { 8 } else { 4 };
                let seconds = if arm64 {
                    -123i64 as u64
                } else {
                    -123i32 as u32 as u64
                };
                for offset in [72, 72 + 2 * width, 72 + 4 * width] {
                    assert_eq!(word(&emu, out + offset as u64, width)?, seconds);
                    assert_eq!(word(&emu, out + (offset + width) as u64, width)?, 0);
                }
                let size = if arm64 { 128 } else { 104 };
                assert_eq!(emu.read_bytes(out + size, 16)?, [0xa5; 16]);
            }
        }
        // Stat calls do not consume clock ticks.
        assert_eq!(syscall(&mut emu, arm64, (113, 263), &[1, out])?, 0);
        let width = if arm64 { 8 } else { 4 };
        assert_eq!(word(&emu, out, width)?, 1);
        assert_eq!(word(&emu, out + width as u64, width)?, 0);
    }
    Ok(())
}

#[test]
fn repeated_runs_have_identical_snapshots_and_coverage() -> Result<()> {
    for (abi, library) in [
        ("armeabi-v7a", "libthread-lib.so"),
        ("arm64-v8a", "libtmessages.29.so"),
        ("armeabi-v7a", "libtmessages.29.so"),
    ] {
        let run = || -> Result<_> {
            let mut emu = Emulator::builder()
                .fixed_clock(1_700_000_000)
                .random_seed(42)
                .trace(Trace {
                    coverage: vec![library.into(), "libc.so".into()],
                    ..Trace::default()
                })
                .on_event(|_| {})
                .build()?;
            let directory = std::path::Path::new("tests/fixtures").join(abi);
            let output = if library == "libthread-lib.so" {
                vec![thread::run(&mut emu, &directory)?]
            } else {
                let output = tmessages::run(&mut emu, &directory)?;
                // This helper measures host execution time; it never reaches the guest.
                let _ = output.pbkdf2_ms;
                vec![output.cbc, output.ctr, output.pbkdf2]
            };
            let coverage: Vec<_> = emu
                .coverage()
                .iter()
                .map(|b| (b.address, b.count))
                .collect();
            Ok((emu.snapshot()?, coverage, output))
        };
        let (first, coverage, output) = run()?;
        let (second, next_coverage, next_output) = run()?;
        assert_eq!(output, next_output);
        let diff = first.diff(&second);
        assert!(
            diff.registers.is_empty()
                && diff.mapped.is_empty()
                && diff.unmapped.is_empty()
                && diff.changes.is_empty(),
            "{abi}/{library}: {}",
            diff.report()
        );
        assert!(!coverage.is_empty());
        assert_eq!(coverage, next_coverage, "{abi}/{library}");
    }
    Ok(())
}

#[test]
fn cli_options_record_fixed_values_and_replay_getrandom_in_both_modes() -> Result<()> {
    use base64::{Engine, prelude::BASE64_STANDARD};
    use serde_json::{Value, json};
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    for arm64 in [true, false] {
        for fixed in [true, false] {
            let path = std::env::temp_dir().join(format!(
                "unidroid-deterministic-{}-{arm64}-{fixed}.jsonl",
                std::process::id()
            ));
            let out = 0x70001000u64;
            let name = out + 256;
            let width = if arm64 { 8 } else { 4 };
            let mut commands = vec![
                json!({"cmd":"load", "path":if arm64 {"rootfs/lib64/libc.so"} else {"rootfs/lib/libc.so"}}),
                json!({"cmd":"write", "address":CODE.to_string(), "base64":BASE64_STANDARD.encode(syscall_bytes(arm64))}),
                json!({"cmd":"write", "address":name.to_string(), "base64":BASE64_STANDARD.encode(b"/lib/libc.so\0")}),
            ];
            let mut calls = vec![
                ((113, 263), vec![0, out], 2 * width),
                ((113, 263), vec![1, out], 2 * width),
                ((169, 78), vec![out, 0], 2 * width),
                ((278, 384), vec![out, 17, 0], 17),
            ];
            if fixed {
                calls.push((
                    (79, 327),
                    vec![0, name, out, 0],
                    if arm64 { 128 } else { 104 },
                ));
            }
            for (numbers, args, size) in calls {
                commands.extend([
                    json!({"cmd":"set_register", "name":if arm64 {"x8"} else {"r7"}, "value":(if arm64 {numbers.0} else {numbers.1}).to_string()}),
                    json!({"cmd":"call_function", "address":CODE.to_string(), "args":args.iter().map(u64::to_string).collect::<Vec<_>>()}),
                    json!({"cmd":"read", "address":out.to_string(), "size":size}),
                ]);
            }
            let input: String = commands.iter().map(|c| format!("{c}\n")).collect();
            let mut command = Command::new(env!("CARGO_BIN_EXE_unidroid"));
            command.args(["serve", "--record"]).arg(&path);
            if fixed {
                command.args(["--fixed-clock", "-123", "--random-seed", "0"]);
            }
            let mut child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            child.stdin.take().unwrap().write_all(input.as_bytes())?;
            let output = child.wait_with_output()?;
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            for line in String::from_utf8(output.stdout)?.lines() {
                assert!(
                    serde_json::from_str::<Value>(line)?.get("ok").is_some(),
                    "{line}"
                );
            }
            let text = std::fs::read_to_string(&path)?;
            let mut entries: Vec<Value> = text
                .lines()
                .map(serde_json::from_str)
                .collect::<std::result::Result<_, _>>()?;
            let times: Vec<_> = entries.iter().filter_map(|e| e.get("time")).collect();
            assert_eq!(times.len(), 3);
            let random: Vec<_> = entries.iter().filter_map(|e| e.get("random")).collect();
            assert_eq!(random.len(), 1);
            assert_eq!(
                BASE64_STANDARD.decode(random[0].as_str().unwrap())?.len(),
                17
            );
            if fixed {
                assert_eq!(entries[0]["options"]["fixed_clock"], -123);
                assert_eq!(entries[0]["options"]["random_seed"], 0);
                assert_eq!(times[0], &json!({"clock":"realtime", "sec":-123, "nsec":0}));
                assert_eq!(
                    times[1],
                    &json!({"clock":"monotonic", "sec":1, "nsec":1000})
                );
                assert_eq!(
                    times[2],
                    &json!({"clock":"realtime", "sec":-123, "nsec":2000})
                );
                assert_eq!(
                    random[0],
                    &json!(BASE64_STANDARD.encode([
                        0xaf, 0xcd, 0x1d, 0x7b, 0x39, 0xa8, 0x20, 0xe2, 0xf4, 0x65, 0xb9, 0xa1,
                        0x6a, 0x9e, 0x78, 0x6e, 0x4f,
                    ]))
                );
            } else {
                assert!(entries[0]["options"]["fixed_clock"].is_null());
                assert!(entries[0]["options"]["random_seed"].is_null());
            }
            assert_eq!(
                unidroid::serve::replay(text.as_bytes(), Emulator::builder())?,
                commands.len()
            );
            // Recorded random values win even over a different seed in the header.
            entries[0]["options"]["random_seed"] = json!(99);
            let changed: String = entries.iter().map(|e| format!("{e}\n")).collect();
            assert_eq!(
                unidroid::serve::replay(changed.as_bytes(), Emulator::builder())?,
                commands.len()
            );
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

#[test]
fn guest_clocks_share_one_microsecond_ticks() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(Emulator::builder().fixed_clock(1_700_000_000), arm64)?;
        let out = emu.malloc(32)?;
        let width = if arm64 { 8 } else { 4 };
        for (numbers, args, sec, fraction) in [
            ((113, 263), vec![0, out], 1_700_000_000, 0),
            ((113, 263), vec![1, out], 1, 1_000),
            ((169, 78), vec![out, out + 24], 1_700_000_000, 2),
            ((113, 263), vec![0, out], 1_700_000_000, 3_000),
        ] {
            emu.write_bytes(out, &[0xa5; 32])?;
            assert_eq!(syscall(&mut emu, arm64, numbers, &args)?, 0);
            assert_eq!(word(&emu, out, width)?, sec);
            assert_eq!(word(&emu, out + width as u64, width)?, fraction);
            assert_eq!(emu.read_bytes(out + 2 * width as u64, 8)?, [0xa5; 8]);
            if numbers.0 == 169 {
                assert_eq!(emu.read_bytes(out + 24, 8)?, [0; 8]);
            }
        }
    }
    Ok(())
}
