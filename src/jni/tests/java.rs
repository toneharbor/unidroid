use super::*;
use crate::Architecture;

mod android;
mod boxing;
mod caches;
mod collections;
mod crypto;
mod cycles;
mod dispatch;
mod exceptions;
mod host;
mod order;
mod origins;
mod policy;
mod references;
mod strings;
mod text;

const ARCHES: [Architecture; 2] = [Architecture::Arm32, Architecture::Arm64];

fn emulator(arch: Architecture) -> Result<Emulator> {
    let mut emu = Emulator::builder().build()?;
    emu.setup_cpu(arch)?;
    Ok(emu)
}

// Drive the actual guest JNIEnv table with jvalue[] on both ABIs.
fn member(
    emu: &mut Emulator,
    handler: &mut dyn JniHandler,
    receiver: Handle,
    class: &str,
    method: &str,
    args: &[JniValue],
    operation: u8,
) -> Result<JniValue> {
    let (name, signature) = method.split_at(method.find('(').unwrap());
    let class = emu.class(class);
    let name_ptr = emu.new_cstring(name)?;
    let sig_ptr = emu.new_cstring(signature)?;
    let id = jni_call(
        emu,
        if operation == 1 { 113 } else { 33 },
        &[class.0, name_ptr, sig_ptr],
    )?;
    let slots = emu.malloc(args.len().max(1) * 8)?;
    for (i, arg) in args.iter().enumerate() {
        emu.write_u64(slots + i as u64 * 8, arg.bits())?;
    }
    let kind = if operation == 6 {
        b'L'
    } else {
        signature.as_bytes()[signature.find(')').unwrap() + 1]
    };
    let slot = b"LZBCSIJFDV"
        .iter()
        .position(|&b| b == if kind == b'[' { b'L' } else { kind })
        .unwrap();
    let index = if operation == 6 {
        30
    } else {
        (if operation == 1 { 116 } else { 36 }) + slot * 3
    };
    let table = emu.read_pointer(emu.jni.env)?;
    let function = emu.read_pointer(table + index as u64 * emu.pointer_size())?;
    let result = emu.invoke_jni(
        function,
        &[
            Handle(emu.jni.env).into(),
            if operation == 0 { receiver } else { class }.into(),
            Handle(id).into(),
            Handle(slots).into(),
        ],
        kind,
        handler,
    );
    emu.free(name_ptr);
    emu.free(sig_ptr);
    emu.free(slots);
    result
}

fn call(emu: &mut Emulator, receiver: Handle, method: &str, args: &[JniValue]) -> Result<JniValue> {
    member(emu, &mut (), receiver, "java/lang/Object", method, args, 0)
}
fn static_call(
    emu: &mut Emulator,
    class: &str,
    method: &str,
    args: &[JniValue],
) -> Result<JniValue> {
    member(emu, &mut (), Handle(0), class, method, args, 1)
}
fn construct(
    emu: &mut Emulator,
    class: &str,
    signature: &str,
    args: &[JniValue],
) -> Result<Handle> {
    Ok(member(
        emu,
        &mut (),
        Handle(0),
        class,
        &format!("<init>{signature}"),
        args,
        6,
    )?
    .object())
}
fn text(emu: &mut Emulator, receiver: Handle) -> Result<String> {
    let value = call(emu, receiver, "toString()Ljava/lang/String;", &[])?.object();
    emu.string(value)
}
