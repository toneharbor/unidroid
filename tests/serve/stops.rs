use super::*;
use unidroid::{Emulator, Result};

#[test]
fn watch_before_load_returns_the_cpu_setup_error() -> Result<()> {
    let mut emu = Emulator::builder().build()?;
    let mut output = Vec::new();
    unidroid::serve::serve(
        &mut emu,
        None,
        b"{\"cmd\":\"watch\",\"at\":\"0x3000\",\"size\":4,\"kind\":\"write\"}\n{\"cmd\":\"modules\"}\n".as_slice(),
        &mut output,
    )?;
    let replies: Vec<Value> = String::from_utf8(output)?
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(
        replies,
        [
            json!({"error":"no library loaded yet: the first load_library picks arm64 or arm32"}),
            json!({"ok":[]}),
        ]
    );
    Ok(())
}

#[test]
fn watch_questions_keep_full_width_values_and_unwatch_removes_the_stop() -> Result<()> {
    for arm64 in [true, false] {
        for kind in ["read", "write", "access"] {
            let words = if arm64 {
                [
                    if kind == "read" {
                        0xf9400001u32
                    } else {
                        0xf9000001
                    },
                    0xd28000e0,
                    0xd65f03c0,
                ]
            } else {
                [
                    if kind == "read" {
                        0xe5901000
                    } else {
                        0xe5801000
                    },
                    0xe3a00007,
                    0xe12fff1e,
                ]
            };
            let mut emu = Emulator::builder().build()?;
            emu.load_library(if arm64 {
                "rootfs/lib64/libc.so"
            } else {
                "rootfs/lib/libc.so"
            })?;
            emu.write_bytes(
                0x20100,
                &words
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
            )?;
            let target = emu.malloc(8)?;
            let value = if arm64 {
                0xfedcba9876543210u64
            } else {
                0xfedcba98
            };
            let size = if arm64 { 8 } else { 4 };
            let at = format!("0x{target:x}");
            let input = [
                json!({"cmd":"watch", "at":at, "size":size, "kind":kind}),
                json!({"cmd":"write", "address":target.to_string(), "base64":BASE64_STANDARD.encode(value.to_le_bytes())}),
                json!({"cmd":"call_function", "address":"131328", "args":[target.to_string(), value.to_string()]}),
                json!({"cmd":"read", "address":target.to_string(), "size":size}),
                json!({"cmd":"unwatch", "at":at}),
                json!({"cmd":"continue"}),
                json!({"cmd":"call_function", "address":"131328", "args":[target.to_string(), value.to_string()]}),
                json!({"cmd":"unwatch", "at":at}),
            ].map(|v| v.to_string()).join("\n");
            let mut output = Vec::new();
            unidroid::serve::serve(&mut emu, None, input.as_bytes(), &mut output)?;
            let replies: Vec<Value> = String::from_utf8(output)?
                .lines()
                .map(|s| serde_json::from_str(s).unwrap())
                .collect();
            assert_eq!(replies.len(), 8);
            assert_eq!(replies[0], json!({"ok":target.to_string()}));
            assert_eq!(replies[1], json!({"ok":null}));
            let question = &replies[2]["break"];
            assert_eq!(question["reason"], "watchpoint");
            assert_eq!(question["address"], "131332");
            assert_eq!(
                question["registers"][if arm64 { "x1" } else { "r1" }],
                value.to_string()
            );
            assert_eq!(
                question["watch"],
                json!({
                    "pc":"131328", "location":"0x20100", "address":target.to_string(),
                    "size":size, "write":kind != "read", "value":value.to_string(),
                })
            );
            assert_eq!(
                replies[3],
                json!({"ok":BASE64_STANDARD.encode(&value.to_le_bytes()[..size])})
            );
            assert_eq!(replies[4], json!({"ok":null}));
            assert_eq!(replies[5], json!({"ok":"7"}));
            assert_eq!(replies[6], json!({"ok":"7"}));
            assert_eq!(
                replies[7],
                json!({"error":format!("no watchpoint at {}", emu.symbolize(target))})
            );
        }
    }
    Ok(())
}

#[test]
fn floating_point_registers_can_be_written_without_appearing_in_dumps() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let bits = 0xfff800000000002au64;
        let input = [
            json!({"cmd":"set_register", "name":"d0", "value":bits.to_string()}),
            json!({"cmd":"set_register", "name":"d31", "value":bits.to_string()}),
            json!({"cmd":"registers"}),
        ]
        .map(|v| v.to_string())
        .join("\n");
        let mut output = Vec::new();
        unidroid::serve::serve(&mut emu, None, input.as_bytes(), &mut output)?;
        let replies: Vec<Value> = String::from_utf8(output)?
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        assert_eq!(replies[0], json!({"ok":null}));
        assert_eq!(replies[1], json!({"ok":null}));
        assert_eq!(emu.register("d0")?, bits);
        assert_eq!(emu.register("d31")?, bits);
        let registers = replies[2]["ok"].as_object().unwrap();
        assert!(registers.keys().all(|name| !name.starts_with('d')));
        assert_eq!(registers.len(), if dir == "lib64" { 34 } else { 17 });
    }
    Ok(())
}

#[test]
fn full_break_finish_exchange() -> Result<()> {
    for (dir, words) in [
        ("lib64", [0xd2800540u32, 0xd65f03c0]),
        ("lib", [0xe3a0002a, 0xe12fff1e]),
    ] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        emu.write_bytes(
            0x20100,
            &words
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        for (name, _) in emu.registers()? {
            if name.starts_with(['r', 'x']) || matches!(name, "fp" | "lr") {
                emu.set_register(name, 0)?;
            }
        }
        let commands = [
            json!({"cmd":"break","at":"0x20100"}),
            json!({"cmd":"call_function","address":"131328","args":[]}),
            json!({"cmd":"registers"}),
            json!({"cmd":"finish"}),
            json!({"cmd":"continue"}),
        ];
        let input = commands
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let mut output = Vec::new();
        unidroid::serve::serve(&mut emu, None, input.as_bytes(), &mut output)?;
        let output = String::from_utf8(output)?;
        let replies: Vec<Value> = output
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(replies.len(), 5);
        assert_eq!(replies[4], json!({"ok":"42"}));
        assert_eq!(replies[3]["break"]["reason"], "finish");
        if dir == "lib" {
            for (command, reply) in commands.iter().zip(output.lines()) {
                println!("→ {command}\n← {reply}");
            }
        }
    }
    Ok(())
}

#[test]
fn debugger_commands_inspect_step_finish_and_resume_a_c_call() -> Result<()> {
    for (dir, gp, words) in [
        ("lib64", "x0", [0x91000400u32, 0xd65f03c0]),
        ("lib", "r0", [0xe2800001, 0xe12fff1e]),
    ] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        emu.write_bytes(
            0x20100,
            &words
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        let input = [
            json!({"cmd":"break", "at":"0x20100"}),
            json!({"cmd":"resolve", "at":"0x20100"}),
            json!({"cmd":"modules"}),
            json!({"cmd":"call_function", "address":"131328", "args":["7"]}),
            json!({"cmd":"registers"}),
            json!({"cmd":"set_register", "name":gp, "value":"8"}),
            json!({"cmd":"backtrace"}),
            json!({"cmd":"step"}),
            json!({"cmd":"finish"}),
            json!({"cmd":"continue"}),
            json!({"cmd":"unbreak", "at":"0x20100"}),
        ]
        .map(|v| v.to_string())
        .join("\n");
        let mut output = Vec::new();
        unidroid::serve::serve(&mut emu, None, input.as_bytes(), &mut output)?;
        let output = String::from_utf8(output)?;
        let replies: Vec<Value> = output
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        assert_eq!(replies[0], json!({"ok":"131328"}));
        assert_eq!(replies[1], replies[0]);
        assert_eq!(replies[2]["ok"][0]["name"], "libc.so");
        assert_eq!(replies[2]["ok"][0]["path"], format!("rootfs/{dir}/libc.so"));
        assert!(replies[2]["ok"][0]["base"].is_string());
        assert!(replies[2]["ok"][0]["size"].is_string());
        assert_eq!(replies[3]["break"]["reason"], "breakpoint");
        assert!(replies[3]["break"].get("watch").is_none());
        assert_eq!(replies[3]["break"]["address"], "131328");
        assert_eq!(replies[3]["break"]["location"], "0x20100");
        assert_eq!(replies[3]["break"]["thread"], 1234);
        assert_eq!(replies[3]["break"]["registers"][gp], "7");
        assert_eq!(replies[4]["ok"], replies[3]["break"]["registers"]);
        assert_eq!(replies[5], json!({"ok":null}));
        assert_eq!(
            replies[6],
            json!({"ok":[{"address":"131328", "location":"0x20100", "symbol":null}]})
        );
        assert_eq!(replies[7]["break"]["reason"], "step");
        assert_eq!(replies[7]["break"]["address"], "131332");
        assert_eq!(replies[7]["break"]["registers"][gp], "9");
        assert_eq!(replies[8]["break"]["reason"], "finish");
        assert_eq!(replies[8]["break"]["address"], "131072");
        assert_eq!(replies[9], json!({"ok":"9"}));
        assert_eq!(replies[10], json!({"ok":null}));
        let raw = output.lines().nth(4).unwrap();
        let prefix = if dir == "lib64" { "x" } else { "r" };
        assert!(
            raw.find(&format!("\"{prefix}2\"")).unwrap()
                < raw.find(&format!("\"{prefix}10\"")).unwrap()
        );
        assert!(!raw.contains("\"d0\""));
        assert_eq!(
            replies[4]["ok"].as_object().unwrap().len(),
            if dir == "lib64" { 34 } else { 17 }
        );
    }
    Ok(())
}
