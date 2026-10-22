use super::*;
use capstone::{
    RegId,
    arch::{arm::ArmReg, arm64::Arm64Reg},
};

#[test]
fn a_hook_read_failure_warns_once_and_later_instructions_still_trace() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .on_event(move |event| {
                sink.borrow_mut().push(serde_json::to_value(event).unwrap());
            })
            .build()?;
        emu.load_library(format!("rootfs/{}/libc.so", arch.library_dir()))?;
        let code = emu.modules()[0].base + 0x1000;
        let protect = emu.resolve("mprotect")?;
        emu.call_function(protect, &[code, crate::PAGE, 7])?;
        let words = match arch {
            Architecture::Arm32 => [0xe3a0002au32, 0xe12fff1e],
            Architecture::Arm64 => [0xd2800540, 0xd65f03c0],
        };
        let bytes = words
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>();
        emu.write_bytes(code, &bytes)?;
        // Translation has already fetched this block when the first code hook runs.
        let remove = emu.cpu_mut()?.add_code_hook(code, code, move |cpu, _, _| {
            cpu.mem_unmap(code, crate::PAGE).unwrap();
        })?;
        emu.set_trace(crate::Trace {
            code: vec!["libc.so+0x1000-0x1004".into()],
            ..crate::Trace::default()
        })?;
        let _ = emu.call_function(code, &[]);
        assert_eq!(events.borrow().len(), 1);
        assert_eq!(events.borrow()[0]["event"], "warning");
        assert!(
            events.borrow()[0]["message"]
                .as_str()
                .unwrap()
                .contains("libc.so+0x1000")
        );
        emu.cpu_mut()?.remove_hook(remove)?;
        emu.cpu_mut()?
            .mem_map(code, crate::PAGE, crate::Prot::ALL)?;
        emu.write_bytes(code, &bytes)?;
        assert_eq!(emu.call_function(code, &[])?, 42);
        assert_eq!(events.borrow().len(), 2);
        assert_eq!(events.borrow()[1]["event"], "instruction");
    }
    Ok(())
}

#[test]
fn an_unavailable_written_register_warns_once_with_the_instruction_address() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(arch)?;
        let name = if arch == Architecture::Arm32 {
            "fpinst"
        } else {
            "z0"
        };
        let mut state = State {
            pending: Some(Pending {
                event: Event::Instruction {
                    at: "libtest.so+0x1000".into(),
                    address: 0x10001000,
                    bytes: String::new(),
                    text: String::new(),
                    read: Vec::new(),
                    written: Vec::new(),
                },
                writes: vec![name.into()],
                address: 0x10001000,
                supervisor: false,
                itstate: None,
            }),
            ..State::default()
        };
        let event = state.finish(emu.cpu()?, arch, false).unwrap();
        assert!(
            matches!(event, Event::Warning { message } if message.contains("libtest.so+0x1000") && message.contains(name))
        );
        assert!(state.finish(emu.cpu()?, arch, false).is_none());
    }
    Ok(())
}

#[test]
fn every_capstone_register_name_has_a_trace_read() -> Result<()> {
    for (arch, thumb) in [
        (Architecture::Arm32, false),
        (Architecture::Arm32, true),
        (Architecture::Arm64, false),
    ] {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(arch)?;
        let (cs, end) = match arch {
            Architecture::Arm32 => (
                Capstone::new()
                    .arm()
                    .mode(if thumb {
                        capstone::arch::arm::ArchMode::Thumb
                    } else {
                        capstone::arch::arm::ArchMode::Arm
                    })
                    .build()?,
                ArmReg::ARM_REG_ENDING,
            ),
            Architecture::Arm64 => (
                Capstone::new()
                    .arm64()
                    .mode(capstone::arch::arm64::ArchMode::Arm)
                    .build()?,
                Arm64Reg::ARM64_REG_ENDING,
            ),
        };
        let mut failures = Vec::new();
        let mut count = 0;
        for id in 1..end {
            if let Some(name) = cs.reg_name(RegId(id as u16)) {
                count += 1;
                let unavailable = match arch {
                    Architecture::Arm32 => matches!(
                        id,
                        ArmReg::ARM_REG_FPINST
                            | ArmReg::ARM_REG_FPINST2
                            | ArmReg::ARM_REG_MVFR0
                            | ArmReg::ARM_REG_MVFR1
                            | ArmReg::ARM_REG_MVFR2
                    ),
                    Architecture::Arm64 => {
                        matches!(id, Arm64Reg::ARM64_REG_FFR | Arm64Reg::ARM64_REG_VG | Arm64Reg::ARM64_REG_ZA | Arm64Reg::ARM64_REG_P0..=Arm64Reg::ARM64_REG_P15 | Arm64Reg::ARM64_REG_Z0..=Arm64Reg::ARM64_REG_ZAS3)
                    }
                };
                match (registers::value(emu.cpu()?, arch, &name), unavailable) {
                    (Ok(_), false) => {}
                    (Err(error), true)
                        if error.to_string()
                            == format!("register {name} is not exposed by Unicorn") => {}
                    (result, _) => failures.push(format!("{name}: {result:?}")),
                }
            }
        }
        assert_eq!(count, end - 1);
        assert!(
            failures.is_empty(),
            "{arch:?} thumb={thumb}: {}",
            failures.join(", ")
        );
    }
    Ok(())
}
