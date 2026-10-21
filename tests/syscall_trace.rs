use std::{cell::RefCell, rc::Rc};
use unidroid::{Crash, Emulator, Result, Trace};

#[test]
fn nonreturning_syscalls_emit_before_the_crash() -> Result<()> {
    for (dir, exit_nr, open_nr, svc, ret) in [
        ("lib64", 93, 56, 0xd4000001u32, 0xd65f03c0),
        ("lib", 1, 322, 0xef000000, 0xe12fff1e),
    ] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .on_event(move |event| {
                sink.borrow_mut()
                    .push((serde_json::to_value(event).unwrap(), event.to_string()));
            })
            .build()?;
        emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        emu.set_trace(Trace {
            syscalls: true,
            ..Trace::default()
        })?;
        for (number, name, args) in [
            (exit_nr, "exit", vec![7]),
            (open_nr, "openat", vec![0, 1, 0, 0]),
        ] {
            let mov = if dir == "lib64" {
                0xd2800008 | (number << 5)
            } else {
                0xe3007000 | number
            };
            emu.write_bytes(
                0x20100,
                &[mov, svc, ret]
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
            )?;
            events.borrow_mut().clear();
            let error = emu.call_function(0x20100, &args).unwrap_err();
            assert!(error.downcast_ref::<Crash>().is_some());
            let captured = events.borrow();
            assert_eq!(
                captured.len(),
                1,
                "event must already exist when the error reaches the caller"
            );
            assert_eq!(captured[0].0["name"], name);
            assert_eq!(captured[0].0["number"], number);
            assert_eq!(captured[0].0.get("result"), Some(&serde_json::Value::Null));
            assert!(captured[0].0.get("errno").is_none());
            assert!(captured[0].1.ends_with("= (no return)"));
        }
    }
    Ok(())
}

#[test]
fn syscalls_report_guest_numbers_paths_results_and_errno() -> Result<()> {
    for (dir, number, words) in [
        ("lib64", 56, [0xd2800708u32, 0xd4000001, 0xd65f03c0]),
        ("lib", 322, [0xe3007142, 0xef000000, 0xe12fff1e]),
    ] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .on_event(move |event| sink.borrow_mut().push(serde_json::to_value(event).unwrap()))
            .build()?;
        emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let path = emu.new_cstring("/missing-test-file")?;
        emu.write_bytes(
            0x20100,
            &words
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        emu.set_trace(Trace {
            syscalls: true,
            ..Trace::default()
        })?;
        let _ = emu.call_function(0x20100, &[0, path, 0x80000, 0])?;
        let captured = events.borrow();
        assert_eq!(captured.len(), 1);
        assert_eq!(
            captured[0],
            serde_json::json!({"event":"syscall", "name":"openat", "number":number,
            "args":["0x0", "\"/missing-test-file\"", "0x80000", "0x0"], "result":-2, "errno":"ENOENT", "at":"0x20104", "from":"0x20000"})
        );
        drop(captured);
        events.borrow_mut().clear();
        emu.set_trace(Trace::default())?;
        let _ = emu.call_function(0x20100, &[0, path, 0, 0])?;
        assert!(events.borrow().is_empty());
    }
    Ok(())
}
