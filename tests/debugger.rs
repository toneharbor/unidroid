use unidroid::{Emulator, JniHandler, Result, Resume, Stop, StopReason};

const CODE: u64 = 0x20100;

#[path = "debugger/conditional.rs"]
mod conditional;
#[path = "debugger/imports.rs"]
mod imports;
#[path = "debugger/lifecycle.rs"]
mod lifecycle;
#[path = "debugger/resume.rs"]
mod resume;
#[path = "debugger/watch.rs"]
mod watch;

struct Handler<F>(F);
impl<F: FnMut(&mut Emulator, &Stop) -> Resume> JniHandler for Handler<F> {
    fn stopped(&mut self, emu: &mut Emulator, stop: &Stop) -> Resume {
        (self.0)(emu, stop)
    }
}

fn emulator(arm64: bool, words: &[u32]) -> Result<Emulator> {
    let mut emu = Emulator::builder().build()?;
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
    Ok(emu)
}

#[test]
fn return_hooks_a_callee_and_abort_keeps_the_stop_site() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            [
                0xaa1e03f3, 0x94000003, 0xd65f0260, 0xd503201f, 0x91000400, 0xd65f03c0,
            ]
        } else {
            [
                0xe1a0400e, 0xeb000001, 0xe12fff14, 0xe1a00000, 0xe2800001, 0xe12fff1e,
            ]
        };
        let mut emu = emulator(arm64, &words)?;
        emu.add_breakpoint(CODE + 16)?;
        emu.add_breakpoint(CODE + 8)?;
        for bits in [42, 0x123456780000002a] {
            let mut handler = Handler(|emu: &mut Emulator, stop: &Stop| {
                if stop.address == CODE + 16 {
                    Resume::Return(bits)
                } else {
                    let regs = emu.registers().unwrap();
                    assert_eq!(regs[0].1, if arm64 { bits } else { bits & 0xffffffff });
                    if !arm64 {
                        assert_eq!(regs[1].1, bits >> 32);
                    }
                    Resume::Continue
                }
            });
            assert_eq!(
                emu.call_function_with(&mut handler, CODE, &[7])?,
                if arm64 { bits } else { 42 }
            );
        }
        let mut handler = Handler(|_: &mut Emulator, _: &Stop| Resume::Abort);
        let error = emu.call_function_with(&mut handler, CODE, &[]).unwrap_err();
        let crash = error.downcast_ref::<unidroid::Crash>().unwrap();
        assert_eq!(
            crash.message,
            format!("stopped by the debugger at {}", emu.symbolize(CODE + 16))
        );
        assert_eq!(
            crash.registers.iter().find(|r| r.0 == "pc").unwrap().1,
            CODE + 16
        );
    }
    Ok(())
}

#[test]
fn finish_stops_at_saved_lr_with_result_and_restored_stack() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            [
                0xaa1e03f3, 0x94000003, 0xd65f0260, 0xd503201f, 0xa9bf7bfd, 0x91000400, 0xa8c17bfd,
                0xd65f03c0,
            ]
        } else {
            [
                0xe1a0400e, 0xeb000001, 0xe12fff14, 0xe1a00000, 0xe92d4800, 0xe2800001, 0xe8bd4800,
                0xe12fff1e,
            ]
        };
        let mut emu = emulator(arm64, &words)?;
        for (entry, target) in [(CODE + 16, CODE + 8), (CODE, 0x20000)] {
            emu.add_breakpoint(entry)?;
            let mut stops = Vec::new();
            let mut entry_sp = 0;
            let mut handler = Handler(|emu: &mut Emulator, stop: &Stop| {
                stops.push((stop.reason, stop.address));
                let regs = emu.registers().unwrap();
                let sp = regs.iter().find(|r| r.0 == "sp").unwrap().1;
                if stops.len() == 1 {
                    entry_sp = sp;
                    Resume::Finish
                } else {
                    assert_eq!(regs[0].1, 8);
                    assert_eq!(sp, entry_sp);
                    Resume::Continue
                }
            });
            assert_eq!(emu.call_function_with(&mut handler, CODE, &[7])?, 8);
            assert_eq!(
                stops,
                [
                    (StopReason::Breakpoint, entry),
                    (StopReason::Finish, target)
                ]
            );
            emu.remove_breakpoint(entry)?;
        }
    }
    Ok(())
}

#[test]
fn step_executes_one_instruction_and_enters_a_guest_callee() -> Result<()> {
    for arm64 in [true, false] {
        // save lr; bl callee; return; padding; add result; return to caller
        let words = if arm64 {
            [
                0xaa1e03f3, 0x94000003, 0xd65f0260, 0xd503201f, 0x91000400, 0xd65f03c0,
            ]
        } else {
            [
                0xe1a0400e, 0xeb000001, 0xe12fff14, 0xe1a00000, 0xe2800001, 0xe12fff1e,
            ]
        };
        let mut emu = emulator(arm64, &words)?;
        emu.add_breakpoint(CODE)?;
        let mut stops = Vec::new();
        let mut handler = Handler(|_: &mut Emulator, stop: &Stop| {
            stops.push((stop.reason, stop.address));
            if stops.len() <= 3 {
                Resume::Step
            } else {
                Resume::Continue
            }
        });
        assert_eq!(emu.call_function_with(&mut handler, CODE, &[7])?, 8);
        assert_eq!(
            stops,
            [
                (StopReason::Breakpoint, CODE),
                (StopReason::Step, CODE + 4),
                (StopReason::Step, CODE + 16),
                (StopReason::Step, CODE + 20),
            ]
        );
    }
    Ok(())
}

#[test]
fn breakpoint_stops_before_instruction_and_continue_executes_it_once() -> Result<()> {
    for arm64 in [true, false] {
        // add x0,x0,#1; ret / add r0,r0,#1; bx lr
        let words = if arm64 {
            [0x91000400, 0xd65f03c0]
        } else {
            [0xe2800001, 0xe12fff1e]
        };
        let mut emu = emulator(arm64, &words)?;
        emu.add_breakpoint(CODE)?;
        let mut hits = 0;
        let mut handler = Handler(|emu: &mut Emulator, stop: &Stop| {
            hits += 1;
            assert_eq!(hits, 1, "resuming must skip this hit");
            assert_eq!(stop.reason, StopReason::Breakpoint);
            assert_eq!(stop.address, CODE);
            assert_eq!(stop.thread, 1234);
            let regs = emu.registers().unwrap();
            assert_eq!(regs.iter().find(|r| r.0 == "pc").unwrap().1, CODE);
            assert_eq!(regs[0].1, 7);
            Resume::Continue
        });
        assert_eq!(emu.call_function_with(&mut handler, CODE, &[7])?, 8);
        assert_eq!(hits, 1);
    }
    Ok(())
}
