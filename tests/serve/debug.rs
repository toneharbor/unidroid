use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc};
use unidroid::{Emulator, Result};

#[test]
fn search_replies_use_the_same_locations_as_snapshot_changes() -> Result<()> {
    use base64::{Engine, prelude::BASE64_STANDARD};
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let allocation = emu.malloc(32)?;
        let library = emu
            .maps()?
            .into_iter()
            .find(|m| m.name == "libc.so")
            .unwrap()
            .start;
        let library_pattern = emu.read_bytes(library, 32)?;
        let before = emu.snapshot()?;
        let pattern = b"known-search-input";
        emu.write_bytes(allocation, pattern)?;
        let location = before
            .diff(&emu.snapshot()?)
            .changes
            .into_iter()
            .find(|c| c.address == allocation)
            .unwrap()
            .location;
        let input = [
            json!({"cmd":"search", "base64":BASE64_STANDARD.encode(pattern)}),
            json!({"cmd":"search", "base64":BASE64_STANDARD.encode(library_pattern)}),
            json!({"cmd":"search", "base64":""}),
        ]
        .map(|v| v.to_string())
        .join("\n");
        let mut output = Vec::new();
        unidroid::serve::serve(&mut emu, None, input.as_bytes(), &mut output)?;
        let replies: Vec<Value> = String::from_utf8(output)?
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        assert_eq!(location, format!("0x{allocation:x} [host malloc]"));
        assert_eq!(
            replies[0],
            json!({"ok":[{"address":allocation.to_string(), "location":location}]})
        );
        assert!(
            replies[1]["ok"]
                .as_array()
                .unwrap()
                .contains(&json!({"address":library.to_string(), "location":"libc.so+0x0"}))
        );
        assert_eq!(replies[2], json!({"error":"empty search pattern"}));
    }
    Ok(())
}

#[test]
fn maps_and_snapshots_work_between_calls_and_at_breakpoints() -> Result<()> {
    for (dir, ret, register) in [("lib64", 0xd65f03c0u32, "x0"), ("lib", 0xe12fff1e, "r0")] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        emu.write_bytes(0x20100, &ret.to_le_bytes())?;
        emu.add_breakpoint(0x20100)?;
        let before = emu.snapshot()?;
        let idle = format!("target/serve-idle-{dir}-{}.snap", std::process::id());
        let stopped = format!("target/serve-stopped-{dir}-{}.snap", std::process::id());
        let input = [
            json!({"cmd":"maps"}),
            json!({"cmd":"snapshot", "path":idle}),
            json!({"cmd":"call_function", "address":"131328", "args":["7"]}),
            json!({"cmd":"snapshot", "path":stopped}),
            json!({"cmd":"continue"}),
        ]
        .map(|v| v.to_string())
        .join("\n");
        let mut output = Vec::new();
        unidroid::serve::serve(&mut emu, None, input.as_bytes(), &mut output)?;
        let replies: Vec<Value> = String::from_utf8(output)?
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        let maps = replies[0]["ok"].as_array().unwrap();
        assert_eq!(maps.len(), before.memory.len());
        for (value, (mapping, _)) in maps.iter().zip(&before.memory) {
            assert_eq!(
                *value,
                json!({"start":mapping.start.to_string(), "end":mapping.end.to_string(), "perms":mapping.perms, "name":mapping.name})
            );
        }
        assert_eq!(replies[1], json!({"ok":null}));
        assert_eq!(replies[2]["break"]["address"], "131328");
        assert_eq!(replies[3], json!({"ok":null}));
        assert_eq!(replies[4], json!({"ok":"7"}));
        let loaded = unidroid::Snapshot::load(&idle)?;
        assert_eq!(loaded.registers, before.registers);
        assert_eq!(loaded.memory, before.memory);
        let loaded = unidroid::Snapshot::load(&stopped)?;
        assert_eq!(loaded.thread, 1234);
        assert_eq!(
            loaded.registers.iter().find(|r| r.0 == "pc").unwrap().1,
            0x20100
        );
        assert_eq!(
            loaded.registers.iter().find(|r| r.0 == register).unwrap().1,
            7
        );
        std::fs::remove_file(idle)?;
        std::fs::remove_file(stopped)?;
    }
    Ok(())
}

#[test]
fn trace_command_replaces_the_whole_state_while_serving() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .on_event(move |event| sink.borrow_mut().push(serde_json::to_value(event).unwrap()))
            .build()?;
        let m = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let getpid = emu.find_symbol(m, "getpid")?.to_string();
        let mut output = Vec::new();
        let input = [
            json!({"cmd":"trace", "jni":true, "syscalls":true, "imports":true, "code":["libc.so"]}),
            json!({"cmd":"call_function", "address":getpid, "args":[]}),
            json!({"cmd":"trace"}),
            json!({"cmd":"call_function", "address":getpid, "args":[]}),
        ]
        .map(|v| v.to_string())
        .join("\n");
        unidroid::serve::serve(&mut emu, None, input.as_bytes(), &mut output)?;
        let replies: Vec<Value> = String::from_utf8(output)?
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        assert_eq!(
            replies,
            [
                json!({"ok":null}),
                json!({"ok":"1234"}),
                json!({"ok":null}),
                json!({"ok":"1234"})
            ]
        );
        assert_eq!(
            events
                .borrow()
                .iter()
                .filter(|e| e["event"] == "syscall")
                .count(),
            1
        );
        assert!(!emu.trace().jni && !emu.trace().syscalls && !emu.trace().imports);
        assert!(emu.trace().code.is_empty());
        let captured = events.borrow();
        let svc = captured
            .iter()
            .filter(|e| e["event"] == "instruction" && e["text"] == "svc #0")
            .collect::<Vec<_>>();
        assert_eq!(svc.len(), 1);
        assert!(
            svc[0]["written"]
                .as_array()
                .unwrap()
                .contains(&json!([if dir == "lib64" { "x0" } else { "r0" }, "0x4d2"]))
        );
    }
    Ok(())
}
