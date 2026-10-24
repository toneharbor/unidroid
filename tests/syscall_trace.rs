use std::{cell::RefCell, rc::Rc};
use unidroid::{Crash, Emulator, Result, Trace};

#[test]
fn output_and_environment_syscalls_have_names_and_argument_counts() -> Result<()> {
    for (dir, numbers) in [("lib64", [64, 66, 160, 48]), ("lib", [4, 146, 122, 334])] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .on_event(move |event| sink.borrow_mut().push(serde_json::to_value(event).unwrap()))
            .build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let buffer = emu.malloc(390)?;
        let path = emu.new_cstring(&format!("/{dir}/libc.so"))?;
        emu.set_trace(Trace {
            syscalls: true,
            ..Trace::default()
        })?;
        for ((function, name, args, count), number) in [
            ("write", "write", vec![1, buffer, 0], 3),
            ("writev", "writev", vec![2, buffer, 0], 3),
            ("uname", "uname", vec![buffer], 1),
            ("access", "faccessat", vec![path, 0], 4),
        ]
        .into_iter()
        .zip(numbers)
        {
            let address = emu.find_symbol(libc, function)?;
            events.borrow_mut().clear();
            assert_eq!(emu.call_function(address, &args)?, 0);
            let captured = events.borrow();
            let syscalls: Vec<_> = captured
                .iter()
                .filter(|event| event["event"] == "syscall")
                .collect();
            assert_eq!(syscalls.len(), 1);
            let event = syscalls[0];
            assert_eq!(event["name"], name);
            assert_eq!(event["number"], number);
            assert_eq!(event["args"].as_array().unwrap().len(), count);
            assert_eq!(event["result"], 0);
            assert!(event.get("errno").is_none());
        }
        for (function, errno, result) in [
            ("write", "EBADF", -9),
            ("writev", "EINVAL", -22),
            ("uname", "EFAULT", -14),
            ("access", "ENOENT", -2),
        ] {
            let missing = emu.new_cstring("/system/bin/su")?;
            let args = match function {
                "write" => vec![0, buffer, 1],
                "writev" => vec![1, buffer, 1025],
                "uname" => vec![0],
                _ => vec![missing, 0],
            };
            let address = emu.find_symbol(libc, function)?;
            events.borrow_mut().clear();
            assert_eq!(emu.call_function(address, &args)? as u32, u32::MAX);
            let captured = events.borrow();
            assert_eq!(captured.len(), 1);
            assert_eq!(captured[0]["result"], result);
            assert_eq!(captured[0]["errno"], errno);
        }
    }
    Ok(())
}

#[test]
fn nonreturning_syscalls_emit_before_the_crash() -> Result<()> {
    for (dir, exit_nr, exit_group_nr, svc, ret) in [
        ("lib64", 93, 94, 0xd4000001u32, 0xd65f03c0),
        ("lib", 1, 248, 0xef000000, 0xe12fff1e),
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
            (exit_group_nr, "exit_group", vec![7]),
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
        assert_eq!(
            emu.call_function(0x20100, &[0, 1, 0, 0])? as u32,
            (-14i32) as u32
        );
        let captured = events.borrow();
        assert_eq!(captured.len(), 1);
        assert_eq!(
            captured[0],
            serde_json::json!({"event":"syscall", "name":"openat", "number":number,
            "args":["0x0", "0x1", "0x0", "0x0"], "result":-14, "errno":"EFAULT", "at":"0x20104", "from":"0x20000"})
        );
        drop(captured);
        events.borrow_mut().clear();
        emu.set_trace(Trace::default())?;
        let _ = emu.call_function(0x20100, &[0, path, 0, 0])?;
        assert!(events.borrow().is_empty());
    }
    Ok(())
}
