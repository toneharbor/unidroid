use super::*;
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc};
use unidroid::{Event, WatchAction};

type Events = Rc<RefCell<Vec<(Value, String)>>>;

fn logged(arm64: bool, words: &[u32]) -> Result<(Emulator, Events)> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    let mut emu = Emulator::builder()
        .on_event(move |event| {
            if let Event::Watch { .. } = event {
                sink.borrow_mut()
                    .push((serde_json::to_value(event).unwrap(), event.to_string()));
            }
        })
        .build()?;
    emu.load_library(if arm64 {
        "rootfs/lib64/libc.so"
    } else {
        "rootfs/lib/libc.so"
    })?;
    emu.write_bytes(
        CODE,
        &words
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect::<Vec<_>>(),
    )?;
    Ok((emu, events))
}

#[test]
fn log_reads_include_symbolized_callers_from_the_frame_chain() -> Result<()> {
    for arm64 in [true, false] {
        // Save lr, install fp and lr from arguments, load, return the value via saved lr.
        let words = if arm64 {
            [
                0xaa1e03e4, 0xaa0203fd, 0xaa0303fe, 0xb9400001, 0xaa0103e0, 0xd65f0080,
            ]
        } else {
            [
                0xe1a0400e, 0xe1a0b002, 0xe1a0e003, 0xe5901000, 0xe1a00001, 0xe12fff14,
            ]
        };
        let (mut emu, events) = logged(arm64, &words)?;
        let target = emu.malloc(4)?;
        emu.write_bytes(target, &42u32.to_le_bytes())?;
        let width = if arm64 { 8 } else { 4 };
        let fp = emu.malloc(4 * width)?;
        let callers = [
            emu.resolve("strlen")?,
            emu.resolve("memcpy")?,
            emu.resolve("malloc")?,
        ];
        for (i, value) in [fp + 2 * width as u64, callers[1], 0, callers[2]]
            .into_iter()
            .enumerate()
        {
            emu.write_bytes(fp + (i * width) as u64, &value.to_le_bytes()[..width])?;
        }
        let callers = callers.map(|pc| emu.symbolize(pc & !1));
        let lr = emu.resolve("strlen")?;
        emu.add_watchpoint(target, 4, Watch::Read, WatchAction::Log)?;
        assert_eq!(emu.call_function(CODE, &[target, 0, fp, lr])?, 42);
        let events = events.borrow();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].0,
            json!({
                "event":"watch", "at":"0x2010c", "pc":"0x2010c",
                "address":format!("0x{target:x}"), "size":4, "write":false,
                "value":"0x2a", "thread":1234, "callers":callers,
            })
        );
        assert_eq!(
            events[0].1,
            format!(
                "watch read 0x{target:x}:4=0x2a at 0x2010c thread 1234 from {}",
                callers.join(" <- ")
            )
        );
    }
    Ok(())
}

#[test]
fn log_watch_keeps_the_next_breakpoint_and_replacement_restores_stops() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            [0xb9000001, 0xd28000e0, 0xd65f03c0]
        } else {
            [0xe5801000, 0xe3a00007, 0xe12fff1e]
        };
        let (mut emu, events) = logged(arm64, &words)?;
        let target = emu.malloc(4)?;
        assert_eq!(emu.call_function(CODE, &[target, 1])?, 7);
        emu.add_breakpoint(CODE + 4)?;
        emu.add_watchpoint(target, 4, Watch::Write, WatchAction::Log)?;
        let mut stops = Vec::new();
        let mut handler = Handler(|_: &mut Emulator, stop: &Stop| {
            stops.push(*stop);
            Resume::Continue
        });
        assert_eq!(
            emu.call_function_with(&mut handler, CODE, &[target, 42])?,
            7
        );
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0].reason, StopReason::Breakpoint);
        assert_eq!(stops[0].address, CODE + 4);
        assert_eq!(stops[0].watch, None);
        assert_eq!(events.borrow().len(), 1);
        emu.remove_breakpoint(CODE + 4)?;
        emu.add_watchpoint(target, 4, Watch::Write, WatchAction::Stop)?;
        let mut hits = 0;
        let mut handler = Handler(|_: &mut Emulator, stop: &Stop| {
            hits += 1;
            assert_eq!(stop.reason, StopReason::Watchpoint);
            assert_eq!(stop.watch.unwrap().value, 43);
            Resume::Continue
        });
        assert_eq!(
            emu.call_function_with(&mut handler, CODE, &[target, 43])?,
            7
        );
        assert_eq!(hits, 1);
        assert_eq!(events.borrow().len(), 1);
        emu.add_watchpoint(target, 4, Watch::Write, WatchAction::Log)?;
        emu.remove_watchpoint(target)?;
        assert_eq!(emu.call_function(CODE, &[target, 44])?, 7);
        assert_eq!(events.borrow().len(), 1);
    }
    Ok(())
}

#[test]
fn log_watch_obeys_thumb_it_and_first_overlapping_access_rules() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            // cmp x2,#0; b.ne return; str w1,[x0]; mov x2,#9; ret
            vec![0xf100005f, 0x54000061, 0xb9000001, 0xd2800122, 0xd65f03c0]
        } else {
            // cmp r2,#0; itt eq; streq r1,[r0]; moveq r2,#9; bx lr
            vec![0xbf042a00, 0x22096001, 0x00004770]
        };
        let (mut emu, events) = logged(arm64, &words)?;
        let target = emu.malloc(16)?;
        emu.add_watchpoint(target + 3, 1, Watch::Write, WatchAction::Log)?;
        for condition in [0, 1] {
            assert_eq!(
                emu.call_function(CODE | u64::from(!arm64), &[target, 42, condition])?,
                target
            );
            assert_eq!(events.borrow().len(), 1);
            assert_eq!(
                events.borrow()[0].0["pc"],
                if arm64 { "0x20108" } else { "0x20104" }
            );
            assert_eq!(emu.read_bytes(target, 4)?, 42u32.to_le_bytes());
        }
        let words = if arm64 {
            [0xa9000801, 0xd65f03c0] // stp x1,x2,[x0]; ret
        } else {
            [0xe8800006, 0xe12fff1e] // stm r0,{r1,r2}; bx lr
        };
        emu.write_bytes(
            CODE,
            &words
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        emu.add_watchpoint(target, 16, Watch::Access, WatchAction::Log)?;
        assert_eq!(emu.call_function(CODE, &[target, 43, 44])?, target);
        let width = if arm64 { 8 } else { 4 };
        assert_eq!(
            emu.read_bytes(target + width as u64, width)?,
            44u64.to_le_bytes()[..width]
        );
        let events = events.borrow();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].0["pc"], "0x20100");
        assert_eq!(events[1].0["address"], format!("0x{target:x}"));
        assert_eq!(events[1].0["size"], width);
        assert_eq!(events[1].0["value"], "0x2b");
    }
    Ok(())
}

#[test]
fn log_watch_reports_the_guest_thread_and_ignores_faulting_stores() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            [0xd2800541, 0xb9000001, 0xd28000e0, 0xd65f03c0]
        } else {
            [0xe3a0102a, 0xe5801000, 0xe3a00007, 0xe12fff1e]
        };
        let (mut emu, events) = logged(arm64, &words)?;
        let create = emu.resolve("pthread_create")?;
        let join = emu.resolve("pthread_join")?;
        let out = emu.malloc(16)?;
        let target = emu.malloc(4)?;
        emu.add_watchpoint(target, 4, Watch::Write, WatchAction::Log)?;
        let mut handler = Handler(|_: &mut Emulator, _: &Stop| panic!("log hit must keep running"));
        assert_eq!(
            emu.call_function_with(&mut handler, create, &[out, 0, CODE, target])?,
            0
        );
        let thread = u64::from_le_bytes(emu.read_bytes(out, 8)?.try_into().unwrap())
            & if arm64 { u64::MAX } else { 0xffffffff };
        assert_eq!(
            emu.call_function_with(&mut handler, join, &[thread, out + 8])?,
            0
        );
        assert_eq!(emu.read_bytes(out + 8, 8)?, 7u64.to_le_bytes());
        assert_eq!(events.borrow().len(), 1);
        assert_eq!(events.borrow()[0].0["thread"], 1235);
        assert_eq!(events.borrow()[0].0["pc"], "0x20104");
        emu.remove_watchpoint(target)?;
        for target in [0x30000000, 0x20800] {
            emu.add_watchpoint(target, 4, Watch::Write, WatchAction::Log)?;
            assert!(
                emu.call_function_with(&mut handler, CODE, &[target])
                    .is_err()
            );
            assert_eq!(events.borrow().len(), 1);
            emu.remove_watchpoint(target)?;
        }
    }
    Ok(())
}

#[test]
fn log_watch_records_three_stores_without_stopping() -> Result<()> {
    for arm64 in [true, false] {
        // Three stores, then return 7.
        let words = if arm64 {
            [0xb9000001, 0xb9000402, 0xb9000803, 0xd28000e0, 0xd65f03c0]
        } else {
            [0xe5801000, 0xe5802004, 0xe5803008, 0xe3a00007, 0xe12fff1e]
        };
        let (mut emu, events) = logged(arm64, &words)?;
        let target = emu.malloc(12)?;
        emu.add_watchpoint(target, 12, Watch::Write, WatchAction::Log)?;
        let mut handler = Handler(|_: &mut Emulator, _: &Stop| panic!("log hit must keep running"));
        assert_eq!(
            emu.call_function_with(&mut handler, CODE, &[target, 42, 43, 44])?,
            7
        );
        let events = events.borrow();
        assert_eq!(events.len(), 3);
        for (i, (event, text)) in events.iter().enumerate() {
            let pc = CODE + i as u64 * 4;
            let address = target + i as u64 * 4;
            let value = 42 + i;
            assert_eq!(
                *event,
                json!({
                    "event":"watch", "at":format!("0x{pc:x}"), "pc":format!("0x{pc:x}"),
                    "address":format!("0x{address:x}"), "size":4, "write":true,
                    "value":format!("0x{value:x}"), "thread":1234, "callers":[],
                })
            );
            assert_eq!(
                *text,
                format!("watch write 0x{address:x}:4=0x{value:x} at 0x{pc:x} thread 1234")
            );
        }
    }
    Ok(())
}
