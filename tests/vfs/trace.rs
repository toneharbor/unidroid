use super::{call, errno};
use std::{cell::RefCell, rc::Rc};
use unidroid::{Emulator, Result, Trace};

#[test]
fn new_file_syscalls_report_names_numbers_and_argument_counts() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .on_event(move |event| sink.borrow_mut().push(serde_json::to_value(event).unwrap()))
            .build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let path = emu.new_cstring("missing")?;
        let buffer = emu.malloc(64)?;
        emu.set_trace(Trace {
            syscalls: true,
            ..Trace::default()
        })?;
        for (function, name, numbers, args, count) in [
            ("dup", "dup", (23, 41), vec![9999], 1),
            ("dup3", "dup3", (24, 358), vec![9999, 100, 0], 3),
            ("fcntl", "fcntl", (25, 221), vec![9999, 1], 3),
            ("mkdirat", "mkdirat", (34, 323), vec![9999, path, 0o755], 3),
            ("unlinkat", "unlinkat", (35, 328), vec![9999, path, 0], 3),
            (
                "renameat",
                "renameat",
                (38, 329),
                vec![9999, path, 9999, path],
                4,
            ),
            ("ftruncate", "ftruncate", (46, 194), vec![9999, 10], 2),
            (
                "syscall",
                "getdents64",
                (61, 217),
                vec![if dir == "lib64" { 61 } else { 217 }, 9999, buffer, 64],
                3,
            ),
            ("pwrite", "pwrite64", (68, 181), vec![9999, buffer, 1, 5], 4),
            (
                "readlinkat",
                "readlinkat",
                (78, 332),
                vec![9999, path, buffer, 64],
                4,
            ),
            ("fsync", "fsync", (82, 118), vec![9999], 1),
            ("fdatasync", "fdatasync", (83, 148), vec![9999], 1),
        ] {
            events.borrow_mut().clear();
            let _ = call(&mut emu, libc, function, &args)?;
            let events = events.borrow();
            let event = events
                .iter()
                .find(|event| event["name"] == name)
                .unwrap_or_else(|| panic!("{dir} {name}: {events:?}"));
            assert_eq!(
                event["number"],
                if dir == "lib64" { numbers.0 } else { numbers.1 }
            );
            assert_eq!(event["args"].as_array().unwrap().len(), count);
            assert_eq!(event["errno"], "EBADF");
        }
        if dir == "lib64" {
            emu.add_file("/source", b"rename")?;
            let from = emu.new_cstring("/source")?;
            let to = emu.new_cstring("/target")?;
            events.borrow_mut().clear();
            assert_eq!(
                call(
                    &mut emu,
                    libc,
                    "syscall",
                    &[276, u64::MAX, from, u64::MAX, to, 0]
                )?,
                0
            );
            assert_eq!(emu.file("/target")?, b"rename");
            let event = events.borrow()[0].clone();
            assert_eq!(event["name"], "renameat2");
            assert_eq!(event["args"].as_array().unwrap().len(), 5);
            let result = call(
                &mut emu,
                libc,
                "syscall",
                &[276, u64::MAX, to, u64::MAX, from, 1],
            )?;
            errno(&mut emu, libc, result, 22)?;
        }
    }
    Ok(())
}

#[test]
fn wide_offsets_use_aligned_arm32_register_pairs() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .on_event(move |event| sink.borrow_mut().push(serde_json::to_value(event).unwrap()))
            .build()?;
        emu.add_file("/value", b"abc")?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let path = emu.new_cstring("/value")?;
        let fd = call(&mut emu, libc, "open", &[path, 2])?;
        let buffer = emu.new_cstring("Z")?;
        emu.set_trace(Trace {
            syscalls: true,
            ..Trace::default()
        })?;
        // Negative 64-bit values must be rejected before an allocation.
        let offset = 0xffff_ffff_0000_0002;
        for (function, args, name, index) in [
            (
                "pwrite64",
                if dir == "lib64" {
                    vec![fd, buffer, 1, offset]
                } else {
                    vec![fd, buffer, 1, 0x1234, 2, u32::MAX as u64]
                },
                "pwrite64",
                3,
            ),
            (
                "ftruncate64",
                if dir == "lib64" {
                    vec![fd, offset]
                } else {
                    vec![fd, 0x1234, 2, u32::MAX as u64]
                },
                "ftruncate",
                1,
            ),
        ] {
            events.borrow_mut().clear();
            let result = call(&mut emu, libc, function, &args)?;
            errno(&mut emu, libc, result, 22)?;
            let captured = events.borrow();
            let event = captured.iter().find(|event| event["name"] == name).unwrap();
            assert_eq!(event["args"][index], "0xffffffff00000002");
        }
        assert_eq!(emu.file("/value")?, b"abc");
    }
    Ok(())
}
