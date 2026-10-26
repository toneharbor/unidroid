use crate::{Emulator, JniHandler, Result, Resume, Stop, Tracepoint};

struct Stops(usize);
impl JniHandler for Stops {
    fn stopped(&mut self, _: &mut Emulator, _: &Stop) -> Resume {
        self.0 += 1;
        Resume::Continue
    }
}

#[test]
fn named_registers_and_expressions_accept_scalar_aliases() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(if arm64 {
            "rootfs/lib64/libc.so"
        } else {
            "rootfs/lib/libc.so"
        })?;
        // nop / mov ip,r0; ret / bx lr
        let words = if arm64 {
            [0xd503201fu32, 0xd65f03c0]
        } else {
            [0xe1a0c000, 0xe12fff1e]
        };
        emu.write_bytes(
            0x20100,
            &words
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        let aliases: &[(&str, &str)] = if arm64 {
            &[("w0", "x0"), ("x29", "fp"), ("x30", "lr"), ("s0", "d0")]
        } else {
            &[("ip", "r12"), ("sb", "r9"), ("fp", "r11"), ("s0", "d0")]
        };
        for &(name, general) in aliases {
            emu.set_register(general, 0)?;
            emu.set_register(name, 42)?;
            assert_eq!(emu.register(name)?, 42);
            assert_eq!(emu.register(general)?, 42);
            emu.set_register(general, 7)?;
            assert_eq!(emu.register(name)?, 7);
        }
        let error = emu.register("oops").unwrap_err().to_string();
        assert!(error.starts_with("unknown register oops; valid names: "));
        for range in if arm64 {
            &["x0-x28", "w0-w30", "s0-s31", "d0-d31"][..]
        } else {
            &["r0-r12", "s0-s31", "d0-d31"][..]
        } {
            assert!(error.contains(range), "{error}");
        }
        if !arm64 {
            let flags = emu.register("apsr_nzcv")?;
            emu.add_conditional_breakpoint(0x20100, &format!("apsr_nzcv==0x{flags:x}"))?;
            let mut stops = Stops(0);
            emu.call_function_with(&mut stops, 0x20100, &[7])?;
            assert_eq!(stops.0, 1);
            emu.remove_breakpoint(0x20100)?;
        }
        let names: &[&str] = if arm64 {
            &["w0", "x29", "s0"]
        } else {
            &["ip", "sb", "s0"]
        };
        for name in names {
            emu.set_register(name, 7)?;
            emu.add_conditional_breakpoint(0x20104, &format!("{name}==0x7"))?;
            emu.add_tracepoint(
                0x20104,
                &Tracepoint {
                    values: vec![(*name).into()],
                    log: false,
                },
            )?;
            let mut stops = Stops(0);
            assert_eq!(emu.call_function_with(&mut stops, 0x20100, &[7])?, 7);
            assert_eq!(stops.0, 1, "{name}");
            assert_eq!(emu.tracepoint_hits(0x20104)?, [(7, 1)], "{name}");
            emu.remove_breakpoint(0x20104)?;
            emu.remove_tracepoint(0x20104)?;
        }
    }
    Ok(())
}
