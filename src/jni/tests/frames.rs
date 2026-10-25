use super::*;
use crate::{Architecture, Resume, Stop, StopReason};

mod lifecycle;

const ARCHES: [Architecture; 2] = [Architecture::Arm32, Architecture::Arm64];
const CODE: u64 = crate::RETURN + 0x100;

fn emulator(arch: Architecture) -> Result<Emulator> {
    let mut emu = Emulator::builder().build()?;
    emu.setup_cpu(arch)?;
    Ok(emu)
}

fn write_code(emu: &mut Emulator, words: &[u32]) -> Result<()> {
    emu.write_bytes(
        CODE,
        &words
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect::<Vec<_>>(),
    )
}

// (JNIEnv*, class name, count): FindClass through the table, optionally delete, repeat.
// The returned address is after the loop and before the zero result is written.
fn class_loop(emu: &mut Emulator, delete: bool) -> Result<u64> {
    let (mut words, start) = match emu.architecture() {
        Architecture::Arm32 => (
            vec![
                0xe92d41f0, // push {r4-r8,lr}
                0xe1a04000, // mov r4,r0
                0xe1a05001, // mov r5,r1
                0xe1a06002, // mov r6,r2
                0xe1a00004, // loop: mov r0,r4
                0xe1a01005, // mov r1,r5
                0xe5947000, // ldr r7,[r4]
                0xe5978018, // ldr r8,[r7,#6*4]
                0xe12fff38, // blx r8
            ],
            4,
        ),
        Architecture::Arm64 => (
            vec![
                0xa9bd53f3, // stp x19,x20,[sp,#-48]!
                0xa9015bf5, // stp x21,x22,[sp,#16]
                0xa9027bf7, // stp x23,x30,[sp,#32]
                0xaa0003f3, // mov x19,x0
                0xaa0103f4, // mov x20,x1
                0xaa0203f5, // mov x21,x2
                0xaa1303e0, // loop: mov x0,x19
                0xaa1403e1, // mov x1,x20
                0xf9400276, // ldr x22,[x19]
                0xf9401ad7, // ldr x23,[x22,#6*8]
                0xd63f02e0, // blr x23
            ],
            6,
        ),
    };
    match emu.architecture() {
        Architecture::Arm32 => {
            if delete {
                words.extend([
                    0xe1a01000, // mov r1,r0
                    0xe1a00004, // mov r0,r4
                    0xe597805c, // ldr r8,[r7,#23*4]
                    0xe12fff38, // blx r8
                ]);
            }
            words.push(0xe2566001); // subs r6,r6,#1
            let offset = start - words.len() as i32 - 2;
            words.push(0x1a000000 | (offset as u32 & 0x00ffffff)); // bne loop
        }
        Architecture::Arm64 => {
            if delete {
                words.extend([
                    0xaa0003e1, // mov x1,x0
                    0xaa1303e0, // mov x0,x19
                    0xf9405ed7, // ldr x23,[x22,#23*8]
                    0xd63f02e0, // blr x23
                ]);
            }
            words.push(0xf10006b5); // subs x21,x21,#1
            let offset = start - words.len() as i32;
            words.push(0x54000001 | ((offset as u32 & 0x7ffff) << 5)); // b.ne loop
        }
    }
    let stop = CODE + words.len() as u64 * 4;
    match emu.architecture() {
        Architecture::Arm32 => words.extend([0xe3a00000, 0xe8bd81f0]), // mov r0,#0; pop {r4-r8,pc}
        Architecture::Arm64 => words.extend([
            0xd2800000, // mov x0,#0
            0xa9415bf5, // ldp x21,x22,[sp,#16]
            0xa9427bf7, // ldp x23,x30,[sp,#32]
            0xa8c353f3, // ldp x19,x20,[sp],#48
            0xd65f03c0, // ret
        ]),
    }
    write_code(emu, &words)?;
    Ok(stop)
}

#[derive(Default)]
struct CountAtStop(Option<usize>);
impl JniHandler for CountAtStop {
    fn stopped(&mut self, emu: &mut Emulator, stop: &Stop) -> Resume {
        assert_eq!(stop.reason, StopReason::Breakpoint);
        self.0 = Some(emu.jni.objects.len());
        Resume::Continue
    }
}

#[test]
fn raw_call_deletes_locals_during_guest_loop() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let stop = class_loop(&mut emu, true)?;
        emu.add_breakpoint(stop)?;
        let name = emu.new_cstring("example/Loop")?;
        let before = emu.jni.objects.len();
        let mut handler = CountAtStop::default();
        assert_eq!(
            emu.call_function_with(&mut handler, CODE, &[emu.jni.env, name, 1000])?,
            0
        );
        assert_eq!(handler.0, Some(before), "{arch:?}");
        assert_eq!(emu.jni.objects.len(), before, "{arch:?}");
        assert_eq!(emu.jni.scope, 0);
    }
    Ok(())
}

#[test]
fn raw_call_frees_locals_on_return() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let stop = class_loop(&mut emu, false)?;
        emu.add_breakpoint(stop)?;
        let name = emu.new_cstring("example/Loop")?;
        let before = emu.jni.objects.len();
        let mut handler = CountAtStop::default();
        assert_eq!(
            emu.call_function_with(&mut handler, CODE, &[emu.jni.env, name, 1000])?,
            0
        );
        assert_eq!(handler.0, Some(before + 1000), "{arch:?}");
        assert_eq!(emu.jni.objects.len(), before, "{arch:?}");
        assert_eq!(emu.jni.scope, 0);
    }
    Ok(())
}

#[test]
fn raw_call_results_keep_host_and_global_ownership() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let text = emu.new_cstring("kept")?;
        let local = Handle(jni_call(&mut emu, 167, &[text])?);
        jni_call(&mut emu, 4, &[])?;
        jni_call(&mut emu, 23, &[local.0])?;
        assert_eq!(emu.string(local)?, "kept");
        assert_eq!(emu.jni.objects[&local].scope, 0);
        for (new, delete, kind) in [(21, 22, 2), (226, 227, 3)] {
            let global = jni_call(&mut emu, new, &[local.0])?;
            assert_eq!(jni_call(&mut emu, 232, &[global])?, kind);
            jni_call(&mut emu, delete, &[global])?;
            assert_eq!(jni_call(&mut emu, 232, &[global])?, 0);
        }
        emu.release(local);
        assert!(emu.string(local).is_err());
        assert_eq!(emu.jni.scope, 0);
    }
    Ok(())
}

fn register_native(emu: &mut Emulator, signature: &str, function: u64) -> Result<()> {
    let class = emu.class("example/Frame");
    let name = emu.new_cstring("run")?;
    let signature = emu.new_cstring(signature)?;
    let entry = emu.malloc(3 * emu.pointer_size() as usize)?;
    emu.write_pointer(entry, name)?;
    emu.write_pointer(entry + emu.pointer_size(), signature)?;
    emu.write_pointer(entry + 2 * emu.pointer_size(), function)?;
    jni_call(emu, 215, &[class.0, entry, 1])?;
    Ok(())
}

#[test]
fn native_global_results_stay_global() -> Result<()> {
    for arch in ARCHES {
        for scope in [0, 2] {
            let mut emu = emulator(arch)?;
            let table = emu.read_pointer(emu.jni.env)?;
            let function = emu.read_pointer(table + 21 * emu.pointer_size())?;
            register_native(&mut emu, "()Ljava/lang/Object;", function)?;
            let receiver = emu.new_object("example/Frame", ());
            emu.jni.scope = scope;
            let global = emu
                .call_jni_method(
                    &mut (),
                    "example/Frame",
                    receiver,
                    "run()Ljava/lang/Object;",
                    &[],
                )?
                .object();
            assert_eq!(emu.jni.scope, scope);
            assert_eq!(
                jni_call(&mut emu, 232, &[global.0])?,
                2,
                "{arch:?}, scope {scope}"
            );
            jni_call(&mut emu, 22, &[global.0])?;
            assert_eq!(jni_call(&mut emu, 232, &[global.0])?, 0);
        }
    }
    Ok(())
}

#[test]
fn raw_calls_join_the_active_frame() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let text = emu.new_cstring("local")?;
        // Act as code inside a native method's frame.
        emu.jni.scope = 2;
        let local = Handle(jni_call(&mut emu, 167, &[text])?);
        assert_eq!(emu.jni.objects[&local].scope, 2);
        jni_call(&mut emu, 19, &[16])?;
        assert_eq!(emu.jni.scope, 3);
        let nested = jni_call(&mut emu, 167, &[text])?;
        assert_eq!(emu.jni.objects[&local].scope, 2);
        jni_call(&mut emu, 20, &[0])?;
        assert_eq!(emu.jni.scope, 2);
        assert_eq!(jni_call(&mut emu, 232, &[nested])?, 0);
        assert_eq!(emu.string(local)?, "local");
        jni_call(&mut emu, 23, &[local.0])?;
        assert_eq!(jni_call(&mut emu, 232, &[local.0])?, 0);
    }
    Ok(())
}

struct PushedFrames {
    text: u64,
    locals: Vec<Handle>,
    abort: bool,
}
impl JniHandler for PushedFrames {
    fn stopped(&mut self, emu: &mut Emulator, _: &Stop) -> Resume {
        for _ in 0..2 {
            jni_call(emu, 19, &[16]).unwrap();
            self.locals
                .push(Handle(jni_call(emu, 167, &[self.text]).unwrap()));
        }
        emu.write_reg(0, self.locals[1].0).unwrap();
        if self.abort {
            Resume::Abort
        } else {
            Resume::Continue
        }
    }
}

#[test]
fn raw_calls_clean_unpopped_frames_after_success_and_error() -> Result<()> {
    for arch in ARCHES {
        for abort in [false, true] {
            let mut emu = emulator(arch)?;
            let ret = match arch {
                Architecture::Arm32 => 0xe12fff1e,
                Architecture::Arm64 => 0xd65f03c0,
            };
            write_code(&mut emu, &[ret])?;
            emu.add_breakpoint(CODE)?;
            let host = emu.new_string("host");
            let global = jni_call(&mut emu, 21, &[host.0])?;
            let before = emu.jni.objects.len();
            let mut handler = PushedFrames {
                text: emu.new_cstring("kept")?,
                locals: Vec::new(),
                abort,
            };
            let result = emu.call_function_with(&mut handler, CODE, &[]);
            assert_eq!(emu.jni.scope, 0, "{arch:?}, abort {abort}");
            assert_eq!(emu.jni.objects.len(), before + usize::from(!abort));
            assert_eq!(jni_call(&mut emu, 232, &[handler.locals[0].0])?, 0);
            assert_eq!(jni_call(&mut emu, 232, &[global])?, 2);
            assert_eq!(emu.string(host)?, "host");
            if abort {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("stopped by the debugger")
                );
                assert!(emu.string(handler.locals[1]).is_err());
            } else {
                assert_eq!(result?, handler.locals[1].0);
                jni_call(&mut emu, 23, &[handler.locals[1].0])?;
                assert_eq!(emu.string(handler.locals[1])?, "kept");
                emu.release(handler.locals[1]);
                assert!(emu.string(handler.locals[1]).is_err());
            }
        }
    }
    Ok(())
}

#[test]
fn native_results_preserve_host_and_outer_local_ownership() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let words = match arch {
            Architecture::Arm32 => [0xe1a00001, 0xe12fff1e], // mov r0,r1; bx lr
            Architecture::Arm64 => [0xaa0103e0, 0xd65f03c0], // mov x0,x1; ret
        };
        write_code(&mut emu, &words)?;
        register_native(&mut emu, "()Ljava/lang/Object;", CODE)?;
        let host = emu.new_string("host");
        emu.jni.scope = 2;
        let outer = Handle(jni_call(&mut emu, 25, &[host.0])?);
        emu.jni.scope = 3;
        for (handle, scope) in [(host, 0), (outer, 2)] {
            let result = emu
                .call_jni_method(
                    &mut (),
                    "example/Frame",
                    handle,
                    "run()Ljava/lang/Object;",
                    &[],
                )?
                .object();
            assert_eq!(result, handle);
            assert_eq!(emu.jni.objects[&handle].scope, scope, "{arch:?}");
            assert_eq!(emu.jni.scope, 3);
        }
        jni_call(&mut emu, 20, &[0])?;
        assert_eq!(emu.string(host)?, "host");
        assert_eq!(emu.string(outer)?, "host");
    }
    Ok(())
}

#[test]
fn native_keeps_new_locals_only_for_object_results() -> Result<()> {
    for arch in ARCHES {
        for (signature, object) in [("()Ljava/lang/Object;", true), ("()I", false)] {
            let mut emu = emulator(arch)?;
            let table = emu.read_pointer(emu.jni.env)?;
            let function = emu.read_pointer(table + 25 * emu.pointer_size())?;
            register_native(&mut emu, signature, function)?;
            let before = emu.jni.objects.len();
            let result = emu.call_static_jni_method(
                &mut (),
                "example/Frame",
                &format!("run{signature}"),
                &[],
            )?;
            assert_eq!(emu.jni.objects.len(), before + usize::from(object));
            assert_eq!(
                jni_call(&mut emu, 232, &[result.bits()])?,
                u64::from(object)
            );
            if object {
                assert_eq!(emu.jni.objects[&result.object()].scope, 0);
            }
            assert_eq!(emu.jni.scope, 0);
        }
    }
    Ok(())
}
