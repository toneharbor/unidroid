use super::*;
use crate::Trace;
use std::{cell::RefCell, rc::Rc};

#[test]
fn syscall_events_cover_yield_wait_wake_and_child_exit() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .on_event(move |event| {
                sink.borrow_mut().push(serde_json::to_value(event).unwrap());
            })
            .build()?;
        let libc = emu.load_library(format!("rootfs/{}/libc.so", arch.library_dir()))?;
        let worker = code(
            &mut emu,
            0x100,
            if arch == Architecture::Arm32 {
                // futex(arg, WAIT, 0, NULL); exit(0).
                &[
                    0xe3a01000, 0xe3a02000, 0xe3a03000, 0xe3a070f0, 0xef000000, 0xe3a00000,
                    0xe3a07001, 0xef000000,
                ]
            } else {
                &[
                    0xd2800001, 0xd2800002, 0xd2800003, 0xd2800c48, 0xd4000001, 0xd2800000,
                    0xd2800ba8, 0xd4000001,
                ]
            },
        )?;
        let wake = syscall_code(&mut emu, 0x200, 240, 98)?;
        let address = emu.malloc(4)?;
        emu.set_trace(Trace {
            syscalls: true,
            ..Trace::default()
        })?;
        let thread = spawn(&mut emu, libc, worker, address)?;
        assert_eq!(call(&mut emu, libc, "sched_yield", &[])?, 0);
        assert_eq!(emu.call_function(wake, &[address, 1, 1, 0, 0, 0])?, 1);
        assert_eq!(call(&mut emu, libc, "pthread_join", &[thread, 0])?, 0);
        let captured = events.borrow();
        for name in ["sched_yield", "futex", "exit"] {
            assert!(
                captured.iter().any(|event| event["name"] == name
                    && event.get("result") == Some(&serde_json::Value::Null)
                    && event.get("errno").is_none()),
                "missing nonreturning {name}: {captured:?}"
            );
        }
        assert!(
            captured
                .iter()
                .any(|event| event["name"] == "futex" && event["result"] == 1)
        );
    }
    Ok(())
}
