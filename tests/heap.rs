use unidroid::{Emulator, Result, Trace};

#[path = "heap/allocations.rs"]
mod allocations;
#[path = "heap/execution.rs"]
mod execution;
#[path = "heap/objects.rs"]
mod objects;
#[path = "heap/sample.rs"]
mod sample;
#[path = "heap/serve.rs"]
mod serve;
#[path = "heap/snapshots.rs"]
mod snapshots;

fn emulator(arm64: bool) -> Result<Emulator> {
    let mut emu = Emulator::builder()
        .trace(Trace {
            heap: true,
            ..Trace::default()
        })
        .build()?;
    emu.load_library(if arm64 {
        "rootfs/lib64/libc.so"
    } else {
        "rootfs/lib/libc.so"
    })?;
    emu.reset_heap();
    Ok(emu)
}

fn code(emu: &mut Emulator, at: u64, words: &[u32]) -> Result<u64> {
    emu.write_bytes(
        at,
        &words
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect::<Vec<_>>(),
    )?;
    Ok(at)
}

fn caller(emu: &mut Emulator, arm64: bool) -> Result<u64> {
    code(
        emu,
        0x20100,
        if arm64 {
            // stp fp,lr,[sp,#-16]!; mov fp,sp; blr x3; ldp fp,lr,[sp],#16; ret
            &[0xa9bf7bfd, 0x910003fd, 0xd63f0060, 0xa8c17bfd, 0xd65f03c0]
        } else {
            // push {fp,lr}; mov fp,sp; blx r3; pop {fp,pc}
            &[0xe92d4800, 0xe1a0b00d, 0xe12fff33, 0xe8bd8800]
        },
    )
}

fn guest(emu: &mut Emulator, name: &str, args: &[u64]) -> Result<u64> {
    let mut words = [0; 4];
    words[..args.len()].copy_from_slice(args);
    words[3] = emu.resolve(name)?;
    emu.call_function(0x20100, &words)
}

fn pointer(emu: &Emulator, at: u64, arm64: bool) -> Result<u64> {
    let mut bytes = [0; 8];
    let width = if arm64 { 8 } else { 4 };
    bytes[..width].copy_from_slice(&emu.read_bytes(at, width)?);
    Ok(u64::from_le_bytes(bytes))
}

fn write_pointer(emu: &mut Emulator, at: u64, value: u64, arm64: bool) -> Result<()> {
    emu.write_bytes(at, &value.to_le_bytes()[..if arm64 { 8 } else { 4 }])
}

#[test]
fn host_started_calls_complete_at_the_return_trampoline() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        let malloc = emu.resolve("malloc")?;
        let address = emu.call_function(malloc, &[32])?;
        let records = emu.heap();
        assert_eq!(records.len(), 1);
        let allocation = &records[0];
        assert_eq!(
            (allocation.address, allocation.size, allocation.thread),
            (address, 32, 1234)
        );
        assert!(allocation.live);
        assert_eq!(allocation.function, "malloc");
        assert_eq!(allocation.backtrace[0].address, 0x20000);
        assert_eq!(
            allocation.backtrace[0].symbol.as_deref(),
            Some("host return")
        );
        let host = emu.malloc(16)?;
        assert_eq!(emu.heap().len(), 1);
        let known = emu.allocation(host + 1).unwrap();
        assert_eq!(known.function, "host");
        assert_eq!(known.thread, 1234);
        assert!(known.backtrace.is_empty());
        emu.free(host);
        assert!(emu.allocation(host).is_none());
    }
    Ok(())
}
