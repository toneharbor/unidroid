use super::*;
use crate::{Emulator, Module, Result};

mod arrays;
mod calls;
mod frames;
mod java;
mod natives;
mod objects;
mod signatures;
mod strings;

fn libc_emulator(arch: crate::Architecture) -> Result<(Emulator, Module)> {
    let mut emu = Emulator::builder().build()?;
    let libc = emu.load_library(format!("rootfs/{}/libc.so", arch.library_dir()))?;
    Ok((emu, libc))
}

fn jni_call(emu: &mut Emulator, index: u64, args: &[u64]) -> Result<u64> {
    let table = emu.read_pointer(emu.jni.env)?;
    let function = emu.read_pointer(table + index * emu.pointer_size())?;
    let mut words = vec![emu.jni.env];
    words.extend_from_slice(args);
    emu.call_function(function, &words)
}
