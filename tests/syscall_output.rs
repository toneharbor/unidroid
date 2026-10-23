use std::{cell::RefCell, rc::Rc};
use unidroid::{Emulator, Event, Module, Result};

fn assert_errno(emu: &mut Emulator, libc: Module, result: u64, expected: u32) -> Result<()> {
    assert_eq!(result as u32, u32::MAX);
    let errno = emu.find_symbol(libc, "__errno")?;
    let address = emu.call_function(errno, &[])?;
    assert_eq!(emu.read_bytes(address, 4)?, expected.to_le_bytes());
    Ok(())
}

#[test]
fn write_caps_large_counts_before_reading_guest_memory() -> Result<()> {
    for (dir, huge) in [("lib64", 1u64 << 62), ("lib", u32::MAX as u64)] {
        let output = Rc::new(RefCell::new(Vec::new()));
        let sink = output.clone();
        let mut emu = Emulator::builder()
            .on_event(move |event| {
                if let Event::Output { fd, text } = event {
                    sink.borrow_mut().push((
                        *fd,
                        text.len(),
                        text.as_bytes().first().copied(),
                        text.as_bytes().last().copied(),
                    ));
                }
            })
            .build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let write = emu.find_symbol(libc, "write")?;
        let limit = 33_554_431u64;
        let buffer = emu.malloc(limit as usize + 1)?;
        emu.write_bytes(buffer, b"A")?;
        emu.write_bytes(buffer + limit - 1, b"XY")?;
        assert_eq!(emu.call_function(write, &[1, buffer, limit + 1])?, limit);
        assert_eq!(
            *output.borrow(),
            [(1, limit as usize, Some(b'A'), Some(b'X'))]
        );
        // This allocation is at the end of mapped host-malloc memory.
        let small = emu.malloc(1)?;
        let result = emu.call_function(write, &[2, small, huge])?;
        assert_errno(&mut emu, libc, result, 14)?;
        assert_eq!(output.borrow().len(), 1);
    }
    Ok(())
}

#[test]
fn writev_caps_each_length_and_the_combined_output() -> Result<()> {
    for (dir, width, huge) in [("lib64", 8, 1u64 << 62), ("lib", 4, u32::MAX as u64)] {
        let output = Rc::new(RefCell::new(Vec::new()));
        let sink = output.clone();
        let mut emu = Emulator::builder()
            .on_event(move |event| {
                if let Event::Output { fd, text } = event {
                    sink.borrow_mut().push((
                        *fd,
                        text.len(),
                        text.as_bytes().first().copied(),
                        text.as_bytes().last().copied(),
                    ));
                }
            })
            .build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let writev = emu.find_symbol(libc, "writev")?;
        let iov = emu.malloc(6 * width)?;
        let tail = emu.new_cstring("XY")?;
        let limit = 33_554_431u64;
        let buffer = emu.malloc(limit as usize + 1)?;
        emu.write_bytes(buffer, b"A")?;
        emu.write_bytes(buffer + limit - 1, b"XY")?;
        // Stop inside the second entry, inside a huge entry, or exactly at its end.
        // In every case the unreadable next buffer must never be read.
        for words in [
            [buffer, limit - 1, tail, 2, 0, 1],
            [buffer, huge, 0, 1, 0, 0],
            [buffer, limit, 0, 1, 0, 0],
        ] {
            let bytes: Vec<_> = words
                .into_iter()
                .flat_map(|word| word.to_le_bytes()[..width].to_vec())
                .collect();
            emu.write_bytes(iov, &bytes)?;
            assert_eq!(emu.call_function(writev, &[2, iov, 3])?, limit);
        }
        assert_eq!(
            *output.borrow(),
            vec![(2, limit as usize, Some(b'A'), Some(b'X')); 3]
        );
        let small = emu.malloc(1)?;
        let bytes: Vec<_> = [small, huge]
            .into_iter()
            .flat_map(|word| word.to_le_bytes()[..width].to_vec())
            .collect();
        emu.write_bytes(iov, &bytes)?;
        let result = emu.call_function(writev, &[1, iov, 1])?;
        assert_errno(&mut emu, libc, result, 14)?;
        assert_eq!(output.borrow().len(), 3);
    }
    Ok(())
}

#[test]
fn writev_concatenates_buffers_before_decoding_and_emits_once() -> Result<()> {
    for (dir, width) in [("lib64", 8), ("lib", 4)] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .on_event(move |event| sink.borrow_mut().push(serde_json::to_value(event).unwrap()))
            .build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let writev = emu.find_symbol(libc, "writev")?;
        let buffer = emu.malloc(8)?;
        emu.write_bytes(buffer, b"hi \xe4\xbd\xa0\n")?;
        let iov = emu.malloc(6 * width)?;
        let bytes: Vec<_> = [buffer, 4, 0, 0, buffer + 4, 3]
            .into_iter()
            .flat_map(|word| word.to_le_bytes()[..width].to_vec())
            .collect();
        emu.write_bytes(iov, &bytes)?;
        events.borrow_mut().clear();
        for fd in [1, 2] {
            assert_eq!(emu.call_function(writev, &[fd, iov, 3])?, 7);
        }
        assert_eq!(
            *events.borrow(),
            [
                serde_json::json!({"event":"output", "fd":1, "text":"hi 你\n"}),
                serde_json::json!({"event":"output", "fd":2, "text":"hi 你\n"}),
            ]
        );
    }
    Ok(())
}

#[test]
fn output_rejects_other_fds_bad_memory_and_too_many_vectors() -> Result<()> {
    for (dir, width) in [("lib64", 8), ("lib", 4)] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .on_event(move |event| sink.borrow_mut().push(serde_json::to_value(event).unwrap()))
            .build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let write = emu.find_symbol(libc, "write")?;
        let writev = emu.find_symbol(libc, "writev")?;
        let open = emu.find_symbol(libc, "open")?;
        let path = emu.new_cstring(&format!("/{dir}/libc.so"))?;
        let fd = emu.call_function(open, &[path, 0])?;
        assert!(fd >= 3 && fd < u32::MAX as u64);
        let buffer = emu.new_cstring("hello")?;
        let iov = emu.malloc(4 * width)?;
        let bytes: Vec<_> = [buffer, 5, 0, 1]
            .into_iter()
            .flat_map(|word| word.to_le_bytes()[..width].to_vec())
            .collect();
        emu.write_bytes(iov, &bytes)?;
        events.borrow_mut().clear();
        for bad_fd in [0, fd, 9999] {
            for (function, address, count) in [(write, buffer, 5), (writev, iov, 1)] {
                let result = emu.call_function(function, &[bad_fd, address, count])?;
                assert_errno(&mut emu, libc, result, 9)?;
            }
        }
        for (function, address, count, errno) in [
            (write, 0, 1, 14),
            (writev, 0, 1, 14),
            (writev, iov, 2, 14),
            (writev, iov, 1025, 22),
        ] {
            let result = emu.call_function(function, &[1, address, count])?;
            assert_errno(&mut emu, libc, result, errno)?;
        }
        assert!(events.borrow().is_empty());
        for function in [write, writev] {
            assert_eq!(emu.call_function(function, &[1, 0, 0])?, 0);
        }
        assert_eq!(
            *events.borrow(),
            vec![serde_json::json!({"event":"output", "fd":1, "text":""}); 2]
        );
    }
    Ok(())
}

#[test]
fn write_emits_stdout_and_stderr_without_tracing() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .on_event(move |event| sink.borrow_mut().push(serde_json::to_value(event).unwrap()))
            .build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let write = emu.find_symbol(libc, "write")?;
        let buffer = emu.malloc(8)?;
        emu.write_bytes(buffer, b"hello\xff\n")?;
        events.borrow_mut().clear();
        for fd in [1, 2] {
            assert_eq!(emu.call_function(write, &[fd, buffer, 7])?, 7);
        }
        assert_eq!(
            *events.borrow(),
            [
                serde_json::json!({"event":"output", "fd":1, "text":"hello\u{fffd}\n"}),
                serde_json::json!({"event":"output", "fd":2, "text":"hello\u{fffd}\n"}),
            ]
        );
    }
    Ok(())
}
