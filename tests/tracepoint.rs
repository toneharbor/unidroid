use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc};
use unidroid::{Emulator, JniHandler, Result, Resume, Stop, StopReason, Tracepoint};

#[path = "tracepoint/conditional.rs"]
mod conditional;
#[path = "tracepoint/expressions.rs"]
mod expressions;
#[path = "tracepoint/integration.rs"]
mod integration;
#[path = "tracepoint/lifecycle.rs"]
mod lifecycle;
#[path = "tracepoint/serve.rs"]
mod serve;
#[path = "tracepoint/vm.rs"]
mod vm;

const CODE: u64 = 0x20100;
type Events = Rc<RefCell<Vec<(Value, String)>>>;

struct Handler<F>(F);
impl<F: FnMut(&mut Emulator, &Stop) -> Resume> JniHandler for Handler<F> {
    fn stopped(&mut self, emu: &mut Emulator, stop: &Stop) -> Resume {
        (self.0)(emu, stop)
    }
}

fn emulator(arm64: bool, words: &[u32]) -> Result<(Emulator, Events)> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    let mut emu = Emulator::builder()
        .on_event(move |event| {
            sink.borrow_mut()
                .push((serde_json::to_value(event).unwrap(), event.to_string()));
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
    events.borrow_mut().clear();
    Ok((emu, events))
}

#[test]
fn tracepoint_text_and_json_keep_symbol_names_expression_text_and_missing_values() -> Result<()> {
    let event = unidroid::Event::Tracepoint {
        at: "libfoo.so+0x1234 (f+0x10)".into(),
        address: 0x10001234,
        thread: 1234,
        values: vec![
            ("[x20+0x18]".into(), Some(0x40001000)),
            ("[[x20+0x18]]:1".into(), Some(0x2a)),
            ("[0x0]".into(), None),
        ],
    };
    assert_eq!(
        event.to_string(),
        "tracepoint libfoo.so+0x1234 (f+0x10) thread 1234 [x20+0x18]=0x40001000 [[x20+0x18]]:1=0x2a [0x0]=?"
    );
    assert_eq!(
        serde_json::to_value(event)?,
        json!({
            "event":"tracepoint", "at":"libfoo.so+0x1234 (f+0x10)", "address":"0x10001234", "thread":1234,
            "values":[["[x20+0x18]","0x40001000"],["[[x20+0x18]]:1","0x2a"],["[0x0]",null]],
        })
    );
    Ok(())
}

#[test]
fn tracepoint_captures_values_before_execution_without_stopping() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            [0x91000400, 0xd65f03c0]
        } else {
            [0xe2800001, 0xe12fff1e]
        };
        let (mut emu, events) = emulator(arm64, &words)?;
        let reg = if arm64 { "x0" } else { "r0" };
        emu.add_tracepoint(
            CODE | 1,
            &Tracepoint {
                values: vec![reg.into(), "0x2a".into(), "[0x0]:1".into()],
                log: true,
            },
        )?;
        let mut handler = Handler(|_: &mut Emulator, _: &Stop| panic!("tracepoints never stop"));
        assert_eq!(emu.call_function_with(&mut handler, CODE, &[7])?, 8);
        assert_eq!(emu.tracepoint_hits(CODE)?, [(7, 1)]);
        assert_eq!(
            events.borrow().as_slice(),
            &[(
                json!({
                    "event":"tracepoint", "at":"0x20100", "address":"0x20100", "thread":1234,
                    "values":[[reg,"0x7"],["0x2a","0x2a"],["[0x0]:1",null]],
                }),
                format!("tracepoint 0x20100 thread 1234 {reg}=0x7 0x2a=0x2a [0x0]:1=?")
            )]
        );
    }
    Ok(())
}
