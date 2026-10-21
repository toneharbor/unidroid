use unidroid::{Crash, Emulator, Result};

#[test]
fn zero_return_addresses_end_the_backtrace() -> Result<()> {
    for (dir, width, words) in [
        ("lib64", 8, [0xaa0103fdu32, 0xaa0203fe, 0xf9400000]),
        ("lib", 4, [0xe1a0b001, 0xe1a0e002, 0xe5900000]),
    ] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let code = 0x20100;
        emu.write_bytes(
            code,
            &words
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        let fp = emu.malloc(4 * width)?;
        // A readable, increasing chain with a zero return address before a valid one.
        for (i, value) in [fp + 2 * width as u64, 0, 0, 0x20300]
            .into_iter()
            .enumerate()
        {
            emu.write_bytes(fp + (i * width) as u64, &value.to_le_bytes()[..width])?;
        }
        for lr in [0x20200, 0] {
            let error = emu.call_function(code, &[0x18, fp, lr]).unwrap_err();
            let crash = error.downcast_ref::<Crash>().unwrap();
            let addresses = crash
                .backtrace
                .iter()
                .map(|frame| frame.address)
                .collect::<Vec<_>>();
            assert_eq!(
                addresses,
                if lr == 0 {
                    vec![code + 8]
                } else {
                    vec![code + 8, lr]
                }
            );
        }
        // A faulting pc of zero is still the crash site, unlike a zero return address.
        let error = emu.call_function(0, &[]).unwrap_err();
        assert_eq!(
            error.downcast_ref::<Crash>().unwrap().backtrace[0].address,
            0
        );
    }
    Ok(())
}

#[test]
fn crash_keeps_fault_registers_and_frame_chain() -> Result<()> {
    for (dir, words, count, thumb) in [
        (
            "lib64",
            vec![
                0xa9bf7bfdu32,
                0x910003fd,
                0x94000002,
                0xd65f03c0,
                0xa9bf7bfd,
                0x910003fd,
                0x94000002,
                0xd65f03c0,
                0xf9400000,
            ],
            34,
            false,
        ),
        (
            "lib",
            vec![
                0xe92d4800, 0xe1a0b00d, 0xeb000000, 0xe12fff1e, 0xe92d4800, 0xe1a0b00d, 0xeb000000,
                0xe12fff1e, 0xe5900000,
            ],
            17,
            false,
        ),
        (
            "lib",
            // Thumb: push {r7,lr}; mov r7,sp; blx r1 / r2; leaf ldr r0,[r0].
            vec![
                0x466fb580, 0x00004788, 0, 0, 0x466fb580, 0x00004790, 0, 0, 0x00006800,
            ],
            17,
            true,
        ),
    ] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let code = 0x20100;
        emu.write_bytes(
            code,
            &words
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        let args = if thumb {
            vec![0x18, code + 17, code + 33]
        } else {
            vec![0x18]
        };
        let error = emu
            .call_function(code | u64::from(thumb), &args)
            .unwrap_err();
        let crash = error.downcast_ref::<Crash>().expect("guest crash");
        assert_eq!(crash.to_string(), error.to_string());
        assert!(crash.message.contains("likely a null pointer"));
        assert_eq!(crash.thread, 1234);
        assert_eq!(crash.registers.len(), count);
        assert_eq!(
            crash.registers.iter().find(|r| r.0 == "pc").unwrap().1,
            code + 32
        );
        assert_eq!(
            crash
                .backtrace
                .iter()
                .map(|f| f.address)
                .collect::<Vec<_>>(),
            if thumb {
                [code + 32, code + 22, code + 6]
            } else {
                [code + 32, code + 28, code + 12]
            }
        );
        assert!(crash.report().contains("#0 0x20120"));
        println!("{}", crash.report());
    }
    Ok(())
}

#[test]
fn host_failure_starts_at_guest_caller() -> Result<()> {
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let mut emu = Emulator::builder().build()?;
        let m = emu.load_library(format!("tests/fixtures/{abi}/libjnidispatch.so"))?;
        let error = emu.call_jni_onload(m, &mut ()).unwrap_err();
        let crash = error.downcast_ref::<Crash>().unwrap();
        let lr = crash.registers.iter().find(|r| r.0 == "lr").unwrap().1;
        assert_eq!(crash.backtrace[0].address, lr & !1);
        assert!(crash.message.ends_with(&emu.location(lr)));
        assert!(
            crash.backtrace[0]
                .location
                .starts_with("libjnidispatch.so+")
        );
    }
    Ok(())
}
