use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc};
use unidroid::{Emulator, Result};

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
            json!({"cmd":"trace", "jni":true, "syscalls":true, "imports":true}),
            json!({"cmd":"call_function", "address":getpid, "args":[]}),
            json!({"cmd":"trace"}),
            json!({"cmd":"call_function", "address":getpid, "args":[]}),
        ]
        .map(|v| v.to_string())
        .join("\n");
        unidroid::serve::serve(&mut emu, input.as_bytes(), &mut output)?;
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
    }
    Ok(())
}
