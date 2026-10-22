use super::*;
use unidroid::{JniHandler, Resume, Stop};

#[test]
fn selection_refresh_removal_and_instruction_tracing_are_independent() -> Result<()> {
    for (arm64, thumb) in [(true, false), (false, false), (false, true)] {
        let (mut emu, entry, target, events) = emulator(arm64, thumb, true)?;
        // A trace configured before load already observes the new module.
        assert_eq!(emu.call_function(entry, &[7, 8, target, 10])?, 9);
        assert_eq!(events.borrow().len(), 2);
        for (calls, code, count) in [
            (vec!["libc.so+0x1000-0x1010"], false, 2),
            (vec!["libc.so+0x1080-0x1088"], false, 0),
            (vec!["libc.so", "libc.so+0x1000-0x1010"], true, 2),
            (vec![], true, 0),
            (vec![], false, 0),
        ] {
            events.borrow_mut().clear();
            emu.set_trace(Trace {
                calls: calls.into_iter().map(String::from).collect(),
                code: if code {
                    vec!["libc.so+0x1000-0x1010".into()]
                } else {
                    vec![]
                },
                ..Trace::default()
            })?;
            assert_eq!(emu.call_function(entry, &[7, 8, target, 10])?, 9);
            let captured = events.borrow();
            assert_eq!(
                captured
                    .iter()
                    .filter(|e| e.1["event"] == "function_call")
                    .count(),
                count
            );
            assert_eq!(
                captured
                    .iter()
                    .filter(|e| e.1["event"] == "instruction")
                    .count(),
                if code { 4 } else { 0 }
            );
        }
    }
    Ok(())
}

#[test]
fn plain_branches_do_not_report_calls() -> Result<()> {
    for (arm64, thumb) in [(true, false), (false, false), (false, true)] {
        let (mut emu, entry, target, events) = emulator(arm64, thumb, false)?;
        emu.set_trace(Trace {
            calls: vec!["libc.so".into()],
            ..Trace::default()
        })?;
        for indirect in [false, true] {
            let code = if thumb {
                halfwords(&[if indirect { 0x4710 } else { 0xe03e }])
            } else if arm64 {
                words(&[if indirect { 0xd61f0040 } else { 0x14000020 }])
            } else {
                words(&[if indirect { 0xe12fff12 } else { 0xea00001e }])
            };
            emu.write_bytes(entry & !1, &code)?;
            assert_eq!(emu.call_function(entry, &[7, 8, target, 10])?, 8);
            assert!(events.borrow().is_empty());
        }
    }
    Ok(())
}

#[test]
fn host_stub_targets_have_locations_and_entry_arguments() -> Result<()> {
    for arm64 in [true, false] {
        let (mut emu, entry, _, events) = emulator(arm64, false, false)?;
        let stub = emu.resolve("dlerror")?;
        emu.call_function(stub, &[])?;
        emu.set_trace(Trace {
            calls: vec!["libc.so+0x1000-0x1010".into()],
            ..Trace::default()
        })?;
        assert_eq!(emu.call_function(entry, &[7, 8, stub, 10])?, 0);
        let captured = events.borrow();
        assert_eq!(captured.len(), 2);
        assert_eq!(
            captured[1].1,
            json!({
                "event":"function_call", "to":emu.symbolize(stub), "address":format!("0x{stub:x}"),
                "args":["0x8", "0x8", format!("0x{stub:x}"), "0xa"], "from":emu.symbolize(entry+8)
            })
        );
    }
    Ok(())
}

#[test]
fn conditional_calls_emit_only_when_executed() -> Result<()> {
    for (arm64, thumb) in [(true, false), (false, false), (false, true)] {
        let (mut emu, entry, _, events) = emulator(arm64, thumb, false)?;
        let code = if thumb {
            halfwords(&[0x4674, 0x2800, 0xbf18, 0xf000, 0xf83b, 0x4720])
        } else if arm64 {
            // mov x19,lr; cbz x0,return; bl callee; return: ret x19.
            words(&[0xaa1e03f3, 0xb4000040, 0x9400001e, 0xd65f0260])
        } else {
            words(&[0xe1a0400e, 0xe3500000, 0x1b00001c, 0xe12fff14])
        };
        emu.write_bytes(entry & !1, &code)?;
        emu.set_trace(Trace {
            calls: vec!["libc.so+0x1000-0x1010".into()],
            ..Trace::default()
        })?;
        for input in [0, 7] {
            events.borrow_mut().clear();
            assert_eq!(
                emu.call_function(entry, &[input])?,
                if input == 0 { 0 } else { 8 }
            );
            assert_eq!(events.borrow().len(), usize::from(input != 0));
        }
    }
    Ok(())
}

struct Handler<F>(F);
impl<F: FnMut(&mut Emulator, &Stop) -> Resume> JniHandler for Handler<F> {
    fn stopped(&mut self, emu: &mut Emulator, stop: &Stop) -> Resume {
        self.0(emu, stop)
    }
}

#[test]
fn stops_and_nested_calls_do_not_duplicate_or_invent_calls() -> Result<()> {
    for (arm64, thumb) in [(true, false), (false, false), (false, true)] {
        let (mut emu, entry, target, events) = emulator(arm64, thumb, true)?;
        let site = (entry & !1) + if thumb { 2 } else { 4 };
        emu.add_breakpoint(site)?;
        for resume in [
            Resume::Continue,
            Resume::Step,
            Resume::Finish,
            Resume::Return(42),
            Resume::Abort,
        ] {
            events.borrow_mut().clear();
            let mut stops = 0;
            let mut handler = Handler(|_: &mut Emulator, _: &Stop| {
                stops += 1;
                if stops == 1 { resume } else { Resume::Continue }
            });
            let result = emu.call_function_with(&mut handler, entry, &[7, 8, target, 10]);
            match resume {
                Resume::Abort => assert!(result.is_err()),
                Resume::Return(_) => assert_eq!(result?, 42),
                _ => assert_eq!(result?, 9),
            }
            assert_eq!(
                events.borrow().len(),
                if matches!(resume, Resume::Return(_) | Resume::Abort) {
                    0
                } else {
                    2
                }
            );
        }
        events.borrow_mut().clear();
        let mut handler = Handler(|emu: &mut Emulator, _: &Stop| {
            assert_eq!(emu.call_function(entry, &[1, 2, target, 3]).unwrap(), 3);
            Resume::Continue
        });
        assert_eq!(
            emu.call_function_with(&mut handler, entry, &[7, 8, target, 10])?,
            9
        );
        assert_eq!(events.borrow().len(), 4);
        assert_eq!(events.borrow()[0].1["args"][0], "0x1");
        assert_eq!(events.borrow()[2].1["args"][0], "0x7");
        emu.remove_breakpoint(site)?;
        emu.add_breakpoint(target)?;
        events.borrow_mut().clear();
        assert_eq!(emu.call_function(entry, &[7, 8, target, 10])?, 9);
        assert_eq!(events.borrow().len(), 2);
    }
    Ok(())
}
