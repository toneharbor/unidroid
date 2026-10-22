#[path = "lifecycle.rs"]
mod lifecycle;
use super::*;
use unidroid::{JniHandler, Resume, Stop};

type Captured = Rc<RefCell<Vec<Value>>>;

pub(super) fn emulator(
    arm64: bool,
    words: &[u32],
    before_load: bool,
) -> Result<(Emulator, u64, Captured)> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    let trace = Trace {
        code: vec!["libc.so+0x1000-0x1040".into()],
        ..Trace::default()
    };
    let mut builder = Emulator::builder().on_event(move |event| {
        if matches!(event, Event::Instruction { .. }) {
            sink.borrow_mut().push(serde_json::to_value(event).unwrap());
        }
    });
    if before_load {
        builder = builder.trace(trace.clone());
    }
    let mut emu = builder.build()?;
    emu.load_library(if arm64 {
        "rootfs/lib64/libc.so"
    } else {
        "rootfs/lib/libc.so"
    })?;
    let code = emu.modules()[0].base + 0x1000;
    let protect = emu.resolve("mprotect")?;
    emu.call_function(protect, &[code, 4096, 7])?;
    emu.write_bytes(
        code,
        &words
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect::<Vec<_>>(),
    )?;
    if !before_load {
        emu.set_trace(trace)?;
    }
    events.borrow_mut().clear();
    Ok((emu, code, events))
}

struct Handler<F>(F);
impl<F: FnMut(&mut Emulator, &Stop) -> Resume> JniHandler for Handler<F> {
    fn stopped(&mut self, emu: &mut Emulator, stop: &Stop) -> Resume {
        self.0(emu, stop)
    }
}
use serde_json::Value;

#[test]
fn instruction_runs_once_across_stops_and_nested_calls() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            [0xaa0003e1, 0x91000c20, 0xd65f03c0]
        } else {
            [0xe1a01000, 0xe2810003, 0xe12fff1e]
        };
        for before_load in [true, false] {
            let (mut emu, code, events) = emulator(arm64, &words, before_load)?;
            emu.add_breakpoint(code)?;
            for resume in [
                Resume::Continue,
                Resume::Step,
                Resume::Finish,
                Resume::Return(42),
                Resume::Abort,
                Resume::Continue,
            ] {
                events.borrow_mut().clear();
                let mut hits = 0;
                let mut handler = Handler(|_: &mut Emulator, _: &Stop| {
                    hits += 1;
                    if hits == 1 { resume } else { Resume::Continue }
                });
                let result = emu.call_function_with(&mut handler, code, &[7]);
                match resume {
                    Resume::Abort => assert!(result.is_err()),
                    Resume::Return(_) => assert_eq!(result?, 42),
                    _ => assert_eq!(result?, 10),
                }
                let addresses: Vec<_> = events
                    .borrow()
                    .iter()
                    .map(|v| v["address"].as_str().unwrap().to_owned())
                    .collect();
                assert_eq!(
                    addresses,
                    if matches!(resume, Resume::Abort | Resume::Return(_)) {
                        vec![]
                    } else {
                        vec![
                            format!("0x{code:x}"),
                            format!("0x{:x}", code + 4),
                            format!("0x{:x}", code + 8),
                        ]
                    },
                    "{arm64} {resume:?}"
                );
            }
            events.borrow_mut().clear();
            let mut handler = Handler(|emu: &mut Emulator, _: &Stop| {
                assert_eq!(emu.call_function(code, &[2]).unwrap(), 5);
                Resume::Continue
            });
            assert_eq!(emu.call_function_with(&mut handler, code, &[7])?, 10);
            assert_eq!(events.borrow().len(), 6);
            assert_eq!(events.borrow()[0]["read"][0][1], "0x2");
            assert_eq!(events.borrow()[3]["read"][0][1], "0x7");
        }
    }
    Ok(())
}

#[test]
fn supervisor_values_are_captured_after_dispatch_and_branch_exits_finish() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            [0xd2801588, 0xd4000001, 0xd65f03c0]
        } else {
            [0xe3a07014, 0xef000000, 0xe12fff1e]
        };
        let (mut emu, code, events) = emulator(arm64, &words, false)?;
        assert_eq!(emu.call_function(code, &[0])?, 1234);
        let events = events.borrow();
        assert_eq!(events.len(), 3);
        assert_eq!(events[1]["text"], "svc #0");
        assert!(
            events[1]["written"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!([
                    if arm64 { "x0" } else { "r0" },
                    "0x4d2"
                ]))
        );
        drop(events);
        // bl outside the traced range; the callee overwrites lr, then returns through a saved register.
        let words = if arm64 {
            [0xaa1e03f3, 0x9400001f, 0xd65f0260]
        } else {
            [0xe1a0400e, 0xeb00001d, 0xe12fff14]
        };
        let (mut emu, code, events) = emulator(arm64, &words, false)?;
        let callee = if arm64 {
            [0xaa1e03f4, 0xd280001e, 0xd65f0280]
        } else {
            [0xe1a0500e, 0xe3a0e000, 0xe12fff15]
        };
        emu.write_bytes(
            code + 0x80,
            &callee
                .iter()
                .flat_map(|w: &u32| w.to_le_bytes())
                .collect::<Vec<_>>(),
        )?;
        assert_eq!(emu.call_function(code, &[7])?, 7);
        assert_eq!(
            events.borrow()[1]["written"][0],
            serde_json::json!(["lr", format!("0x{:x}", code + 8)])
        );
    }
    Ok(())
}
