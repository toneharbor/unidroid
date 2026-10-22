use unidroid::{Emulator, Result};

#[path = "checkpoint/diagnostics.rs"]
mod diagnostics;
#[path = "checkpoint/memory.rs"]
mod memory;
#[path = "checkpoint/sample.rs"]
mod sample;
#[path = "checkpoint/serve.rs"]
mod serve;
#[path = "checkpoint/threads.rs"]
mod threads;

fn emulator(arm64: bool) -> Result<Emulator> {
    let mut emu = Emulator::builder()
        .fixed_clock(1_700_000_000)
        .random_seed(42)
        .build()?;
    emu.load_library(format!(
        "rootfs/{}/libc.so",
        if arm64 { "lib64" } else { "lib" }
    ))?;
    Ok(emu)
}

fn code(emu: &mut Emulator, address: u64, words: &[u32]) -> Result<u64> {
    emu.write_bytes(
        address,
        &words
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect::<Vec<_>>(),
    )?;
    Ok(address)
}

fn syscall(emu: &mut Emulator, arm64: bool, numbers: (u32, u32), args: &[u64]) -> Result<u64> {
    let words = if arm64 {
        vec![0xd2800008 | (numbers.0 << 5), 0xd4000001, 0xd65f03c0]
    } else {
        vec![
            0xe92d40b0,
            0xe59d4010,
            0xe59d5014,
            0xe3007000 | (numbers.1 & 0xfff) | ((numbers.1 & 0xf000) << 4),
            0xef000000,
            0xe8bd80b0,
        ]
    };
    code(emu, 0x20100, &words)?;
    emu.call_function(0x20100, args)
}

#[test]
fn rewind_reopens_files_and_repeats_clock_and_random() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        let path = emu.new_cstring("/lib/./libc.so")?;
        let random_path = emu.new_cstring("/dev/random")?;
        let fd = syscall(&mut emu, arm64, (56, 322), &[0, path, 0])?;
        let random = syscall(&mut emu, arm64, (56, 322), &[0, random_path, 0])?;
        let out = emu.malloc(64)?;
        assert_eq!(syscall(&mut emu, arm64, (63, 3), &[fd, out, 7])?, 7);
        assert_eq!(syscall(&mut emu, arm64, (278, 384), &[out, 3, 0])?, 3);
        syscall(&mut emu, arm64, (113, 263), &[0, out])?;
        let checkpoint = emu.checkpoint()?;
        let run = |emu: &mut Emulator| -> Result<_> {
            assert_eq!(syscall(emu, arm64, (63, 3), &[fd, out, 32])?, 32);
            let bytes = emu.read_bytes(out, 32)?;
            syscall(emu, arm64, (113, 263), &[0, out])?;
            let time = emu.read_bytes(out, if arm64 { 16 } else { 8 })?;
            syscall(emu, arm64, (63, 3), &[random, out, 5])?;
            syscall(emu, arm64, (278, 384), &[out + 5, 19, 0])?;
            Ok((bytes, time, emu.read_bytes(out, 24)?))
        };
        let first = run(&mut emu)?;
        for _ in 0..3 {
            syscall(&mut emu, arm64, (57, 6), &[fd])?;
            let next_fd = syscall(&mut emu, arm64, (56, 322), &[0, path, 0])?;
            emu.rewind(&checkpoint)?;
            assert_eq!(run(&mut emu)?, first);
            assert_eq!(syscall(&mut emu, arm64, (56, 322), &[0, path, 0])?, next_fd);
            emu.rewind(&checkpoint)?;
        }
        let mut copy = Emulator::builder().build()?;
        copy.rewind(&checkpoint)?;
        assert_eq!(run(&mut copy)?, first);
        emu.rewind(&checkpoint)?;
        assert_eq!(run(&mut emu)?, first);
    }
    Ok(())
}

#[test]
fn rewind_restores_registers_memory_and_host_allocations() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        let address = emu.malloc(32)?;
        emu.write_bytes(address, &[1; 32])?;
        let register = if arm64 { "x19" } else { "r9" };
        emu.set_register(register, 123)?;
        emu.set_register("d5", 0x1234_5678_9abc_def0)?;
        let checkpoint = emu.checkpoint()?;
        let before = emu.snapshot()?;
        let high_water = emu.heap_high_water_mark();
        for _ in 0..2 {
            emu.set_register(register, 999)?;
            emu.set_register("d5", 0)?;
            let memset = emu.resolve("memset")?;
            emu.call_function(memset, &[address, 7, 16])?;
            emu.write_bytes(address + 16, &[8; 16])?;
            emu.free(address);
            assert_eq!(emu.malloc(32)?, address);
            let extra = emu.malloc(8192)?;
            emu.rewind(&checkpoint)?;
            assert_eq!(emu.heap_high_water_mark(), high_water);
            assert_eq!(emu.register(register)?, 123);
            assert_eq!(emu.register("d5")?, 0x1234_5678_9abc_def0);
            let diff = before.diff(&emu.snapshot()?);
            assert!(
                diff.registers.is_empty()
                    && diff.changes.is_empty()
                    && diff.mapped.is_empty()
                    && diff.unmapped.is_empty(),
                "{}",
                diff.report()
            );
            emu.free(extra); // A post-checkpoint allocation no longer exists.
            assert_eq!(emu.malloc(8192)?, extra);
            emu.rewind(&checkpoint)?;
        }
    }
    Ok(())
}
