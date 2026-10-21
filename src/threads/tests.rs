use crate::{Architecture, Emulator, Module, RETURN, Result};

mod futex;
mod lifecycle;
mod scheduling;
mod tracing;

fn libc_emulator(arch: Architecture) -> Result<(Emulator, Module)> {
    let mut emu = Emulator::builder().build()?;
    let libc = emu.load_library(format!("rootfs/{}/libc.so", arch.library_dir()))?;
    Ok((emu, libc))
}

fn code(emu: &mut Emulator, offset: u64, words: &[u32]) -> Result<u64> {
    let address = RETURN + offset;
    let bytes: Vec<_> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    emu.write_bytes(address, &bytes)?;
    Ok(address)
}

fn call(emu: &mut Emulator, libc: Module, name: &str, args: &[u64]) -> Result<u64> {
    emu.call_function(emu.find_symbol(libc, name)?, args)
}

fn spawn(emu: &mut Emulator, libc: Module, entry: u64, argument: u64) -> Result<u64> {
    let out = emu.malloc(8)?;
    assert_eq!(
        call(emu, libc, "pthread_create", &[out, 0, entry, argument])?,
        0
    );
    emu.read_pointer(out)
}

fn syscall_code(emu: &mut Emulator, offset: u64, arm32: u32, arm64: u32) -> Result<u64> {
    if emu.architecture() == Architecture::Arm32 {
        // Preserve callee-saved registers and fetch syscall arguments 5 and 6.
        code(
            emu,
            offset,
            &[
                0xe92d40b0,
                0xe59d4010,
                0xe59d5014,
                0xe3007000 | (arm32 & 0xfff) | ((arm32 & 0xf000) << 4),
                0xef000000,
                0xe8bd80b0,
            ],
        )
    } else {
        code(
            emu,
            offset,
            &[0xd2800008 | (arm64 << 5), 0xd4000001, 0xd65f03c0],
        )
    }
}
