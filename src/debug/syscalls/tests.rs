use crate::{Emulator, Event, Result, Trace};
use std::{cell::RefCell, rc::Rc};

#[test]
fn syscall_trace_strings_are_capped_at_256_bytes() -> Result<()> {
    for (dir, words) in [
        ("lib64", [0xd2800708u32, 0xd4000001, 0xd65f03c0]),
        ("lib", [0xe3007142, 0xef000000, 0xe12fff1e]),
    ] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .on_event(move |event| {
                if let Event::Syscall { args, .. } = event {
                    sink.borrow_mut().push(args.clone());
                }
            })
            .build()?;
        emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        emu.write_bytes(
            0x20100,
            &words
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        let path = emu.new_cstring(&"a".repeat(300))?;
        emu.set_trace(Trace {
            syscalls: true,
            ..Trace::default()
        })?;
        emu.call_function(0x20100, &[0, path, 0, 0])?;
        assert_eq!(events.borrow().len(), 1);
        assert_eq!(events.borrow()[0][1], format!("\"{}\"", "a".repeat(256)));
    }
    Ok(())
}
