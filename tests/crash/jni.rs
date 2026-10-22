use unidroid::{Crash, Emulator, JniHandler, Result, Resume, Stop};

const CODE: u64 = 0x20100;

struct Vm(u64);
impl JniHandler for Vm {
    fn stopped(&mut self, emu: &mut Emulator, _: &Stop) -> Resume {
        self.0 = emu.registers().unwrap()[0].1;
        Resume::Return(0x10006)
    }
}

fn pointer(emu: &Emulator, at: u64, width: usize) -> Result<u64> {
    let mut bytes = [0; 8];
    bytes[..width].copy_from_slice(&emu.read_bytes(at, width)?);
    Ok(u64::from_le_bytes(bytes))
}

fn setup(arm64: bool) -> Result<(Emulator, u64)> {
    let mut emu = Emulator::builder().build()?;
    let abi = if arm64 { "arm64-v8a" } else { "armeabi-v7a" };
    let module = emu.load_library(format!("tests/fixtures/{abi}/libjnidispatch.so"))?;
    let onload = emu.find_symbol(module, "JNI_OnLoad")?;
    emu.add_breakpoint(onload)?;
    let mut vm = Vm(0);
    emu.call_jni_onload(module, &mut vm)?;
    emu.remove_breakpoint(onload)?;
    let width = if arm64 { 8 } else { 4 };
    let table = pointer(&emu, vm.0, width)?;
    let get_env = pointer(&emu, table + 6 * width as u64, width)?;
    let out = emu.malloc(width)?;
    assert_eq!(emu.call_function(get_env, &[vm.0, out, 0x10006])?, 0);
    let env = pointer(&emu, out, width)?;
    Ok((emu, env))
}

fn call(
    emu: &mut Emulator,
    env: u64,
    arm64: bool,
    code: u64,
    index: u64,
    args: [u64; 3],
) -> Result<u64> {
    let width = if arm64 { 8 } else { 4 };
    let table = pointer(emu, env, width)?;
    let function = pointer(emu, table + index * width as u64, width)?;
    // Call the JNI function passed as argument 5, preserving the guest return address.
    let words = if arm64 {
        [0xa9bf7bfdu32, 0xd63f0080, 0xa8c17bfd, 0xd65f03c0]
    } else {
        [0xe59dc000, 0xe92d4010, 0xe12fff3c, 0xe8bd8010]
    };
    emu.write_bytes(
        code,
        &words
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>(),
    )?;
    emu.call_function(code, &[env, args[0], args[1], args[2], function])
}

#[test]
fn crash_jni_tail_describes_lookups_strings_and_class_handles_without_tracing() -> Result<()> {
    for arm64 in [true, false] {
        let (mut emu, env) = setup(arm64)?;
        assert!(!emu.trace().jni);
        let class_name = emu.new_cstring("test/Tail")?;
        let class = call(&mut emu, env, arm64, CODE, 6, [class_name, 0, 0])?;
        call(&mut emu, env, arm64, CODE, 21, [class, 0, 0])?;
        let mut expected = vec![
            "JNIEnv->FindClass(test/Tail)".to_string(),
            format!("JNIEnv->NewGlobalRef(test/Tail handle=0x{class:x}, arg2=0x0, arg3=0x0)"),
        ];
        for (index, function, name, signature) in [
            (33, "GetMethodID", "method", "()V"),
            (94, "GetFieldID", "field", "I"),
            (113, "GetStaticMethodID", "method", "()V"),
            (144, "GetStaticFieldID", "field", "I"),
        ] {
            let name_ptr = emu.new_cstring(name)?;
            let signature_ptr = emu.new_cstring(signature)?;
            call(
                &mut emu,
                env,
                arm64,
                CODE,
                index,
                [class, name_ptr, signature_ptr],
            )?;
            expected.push(format!("JNIEnv->{function}(test/Tail->{name}{signature})"));
        }
        let text = emu.new_cstring("text")?;
        let string = call(&mut emu, env, arm64, CODE, 167, [text, 0, 0])?;
        call(&mut emu, env, arm64, CODE, 168, [string, 0, 0])?;
        call(&mut emu, env, arm64, CODE, 4, [1, 2, 3])?;
        expected.extend([
            "JNIEnv->NewStringUTF(text)".to_string(),
            format!("JNIEnv->GetStringUTFLength(java/lang/String handle=0x{string:x}, arg2=0x0, arg3=0x0)"),
            "JNIEnv->GetVersion".to_string(),
            "JNIEnv->DefineClass".to_string(),
        ]);
        let error = call(&mut emu, env, arm64, CODE, 5, [0; 3]).unwrap_err();
        let crash = error.downcast_ref::<Crash>().unwrap();
        let lr = CODE + if arm64 { 8 } else { 12 };
        let expected = expected
            .into_iter()
            .map(|line| format!("{line} from 0x{lr:x}"))
            .collect::<Vec<_>>();
        assert_eq!(crash.jni, expected);
    }
    Ok(())
}

#[test]
fn crash_jni_details_read_guest_strings_at_the_crash() -> Result<()> {
    for arm64 in [true, false] {
        let (mut emu, env) = setup(arm64)?;
        let class_name = emu.new_cstring("test/Before")?;
        let class = call(&mut emu, env, arm64, CODE, 6, [class_name, 0, 0])?;
        let name = emu.new_cstring("before")?;
        let signature = emu.new_cstring("I")?;
        let member = call(&mut emu, env, arm64, CODE, 144, [class, name, signature])?;
        emu.write_bytes(class_name, b"test/After\0")?;
        emu.write_bytes(name, b"after\0")?;
        emu.write_bytes(signature, b"J\0")?;
        let error = call(&mut emu, env, arm64, CODE, 150, [class, member, 0]).unwrap_err();
        let crash = error.downcast_ref::<Crash>().unwrap();
        let lr = CODE + if arm64 { 8 } else { 12 };
        assert_eq!(
            crash.jni,
            [
                format!("JNIEnv->FindClass(test/After) from 0x{lr:x}"),
                format!("JNIEnv->GetStaticFieldID(test/Before->afterJ) from 0x{lr:x}"),
                format!("JNIEnv->GetStaticIntField test/Before->beforeI from 0x{lr:x}"),
            ]
        );
    }
    Ok(())
}

#[test]
fn crash_jni_details_are_omitted_when_a_guest_string_is_unmapped() -> Result<()> {
    for arm64 in [true, false] {
        for (index, function, argument, text) in [
            (6, "FindClass", 0, "test/Tail"),
            (167, "NewStringUTF", 0, "text"),
            (144, "GetStaticFieldID", 1, "field"),
            (144, "GetStaticFieldID", 2, "I"),
        ] {
            let (mut emu, env) = setup(arm64)?;
            let mmap = emu.resolve("mmap")?;
            let munmap = emu.resolve("munmap")?;
            let memory = emu.call_function(mmap, &[0, 4096, 3, 0x22, u64::MAX, 0])?;
            emu.write_bytes(memory, text.as_bytes())?;
            let class = emu.class("test/Tail").0;
            let name = emu.new_cstring("field")?;
            let signature = emu.new_cstring("I")?;
            let mut args = [class, name, signature];
            args[argument] = memory;
            call(&mut emu, env, arm64, CODE, index, args)?;
            assert_eq!(emu.call_function(munmap, &[memory, 4096])?, 0);
            assert!(emu.read_bytes(memory, 1).is_err());
            let error = call(&mut emu, env, arm64, CODE, 5, [0; 3]).unwrap_err();
            let crash = error.downcast_ref::<Crash>().unwrap();
            let lr = CODE + if arm64 { 8 } else { 12 };
            assert_eq!(
                crash.jni,
                [
                    format!("JNIEnv->{function} from 0x{lr:x}"),
                    format!("JNIEnv->DefineClass from 0x{lr:x}"),
                ]
            );
        }
    }
    Ok(())
}

#[test]
fn crash_jni_tail_includes_member_calls_and_failures_before_dispatch() -> Result<()> {
    for arm64 in [true, false] {
        let (mut emu, env) = setup(arm64)?;
        let class = emu.class("test/Tail");
        let object = emu.new_object("test/Tail", ());
        for (index, function, id_slot, name, signature) in [
            (28, "NewObject", 33, "<init>", "()V"),
            (34, "CallObjectMethod", 33, "method", "()Ljava/lang/Object;"),
            (
                64,
                "CallNonvirtualObjectMethod",
                33,
                "method",
                "()Ljava/lang/Object;",
            ),
            (
                114,
                "CallStaticObjectMethod",
                113,
                "method",
                "()Ljava/lang/Object;",
            ),
            (95, "GetObjectField", 94, "field", "Ljava/lang/Object;"),
            (104, "SetObjectField", 94, "field", "Ljava/lang/Object;"),
            (
                145,
                "GetStaticObjectField",
                144,
                "field",
                "Ljava/lang/Object;",
            ),
            (
                154,
                "SetStaticObjectField",
                144,
                "field",
                "Ljava/lang/Object;",
            ),
        ] {
            let name_ptr = emu.new_cstring(name)?;
            let signature_ptr = emu.new_cstring(signature)?;
            let member = call(
                &mut emu,
                env,
                arm64,
                CODE,
                id_slot,
                [class.0, name_ptr, signature_ptr],
            )?;
            let receiver = if index == 28 || index >= 114 {
                class.0
            } else {
                object.0
            };
            let args = if index == 64 {
                [receiver, class.0, member]
            } else {
                [receiver, member, 0]
            };
            let error = call(&mut emu, env, arm64, CODE, index, args).unwrap_err();
            let crash = error.downcast_ref::<Crash>().unwrap();
            let lr = CODE + if arm64 { 8 } else { 12 };
            assert_eq!(
                crash.jni.last().unwrap(),
                &format!("JNIEnv->{function} test/Tail->{name}{signature} from 0x{lr:x}")
            );
        }
        let error = call(&mut emu, env, arm64, CODE, 31, [999, 0, 0]).unwrap_err();
        let crash = error.downcast_ref::<Crash>().unwrap();
        let lr = CODE + if arm64 { 8 } else { 12 };
        assert_eq!(
            crash.jni.last().unwrap(),
            &format!("JNIEnv->GetObjectClass from 0x{lr:x}")
        );
        assert!(crash.message.contains("invalid JNI reference"));
    }
    Ok(())
}

#[test]
fn crash_jni_tail_keeps_calls_from_other_guest_threads() -> Result<()> {
    for arm64 in [true, false] {
        let (mut emu, env) = setup(arm64)?;
        // The child calls GetVersion through its JNIEnv table and returns.
        let words = if arm64 {
            vec![
                0xa9bf7bfdu32,
                0xf9400001,
                0xf9401021,
                0xd63f0020,
                0xa8c17bfd,
                0xd65f03c0,
            ]
        } else {
            vec![0xe92d4010, 0xe5901000, 0xe5911010, 0xe12fff31, 0xe8bd8010]
        };
        emu.write_bytes(
            0x20800,
            &words
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        let create = emu.resolve("pthread_create")?;
        let join = emu.resolve("pthread_join")?;
        let out = emu.malloc(16)?;
        assert_eq!(emu.call_function(create, &[out, 0, 0x20800, env])?, 0);
        let thread = pointer(&emu, out, if arm64 { 8 } else { 4 })?;
        assert_eq!(emu.call_function(join, &[thread, 0])?, 0);
        let error = call(&mut emu, env, arm64, CODE, 5, [0; 3]).unwrap_err();
        let crash = error.downcast_ref::<Crash>().unwrap();
        assert_eq!(crash.thread, 1234);
        let lr = CODE + if arm64 { 8 } else { 12 };
        assert_eq!(
            crash.jni,
            [
                "JNIEnv->GetVersion from 0x20810 in thread 1235".to_string(),
                format!("JNIEnv->DefineClass from 0x{lr:x}"),
            ]
        );
    }
    Ok(())
}

#[test]
fn crash_keeps_the_last_sixteen_jni_calls_in_order_without_tracing() -> Result<()> {
    for arm64 in [true, false] {
        let (mut emu, env) = setup(arm64)?;
        assert!(!emu.trace().jni);
        let class = emu.class("java/lang/Void");
        let name = emu.new_cstring("TYPE")?;
        let signature = emu.new_cstring("Ljava/lang/Class;")?;
        let member = call(&mut emu, env, arm64, CODE, 144, [class.0, name, signature])?;
        let mut expected = Vec::new();
        for i in 0..20 {
            let code = CODE + 0x20 * (i + 1);
            let (index, name) = if i % 2 == 0 {
                (4, "GetVersion")
            } else {
                (228, "ExceptionCheck")
            };
            call(&mut emu, env, arm64, code, index, [0; 3])?;
            let lr = code + if arm64 { 8 } else { 12 };
            expected.push(format!("JNIEnv->{name} from 0x{lr:x}"));
        }
        let code = CODE + 0x20 * 21;
        let error = call(&mut emu, env, arm64, code, 145, [class.0, member, 0]).unwrap_err();
        let lr = code + if arm64 { 8 } else { 12 };
        expected.push(format!(
            "JNIEnv->GetStaticObjectField java/lang/Void->TYPELjava/lang/Class; from 0x{lr:x}"
        ));
        let crash = error.downcast_ref::<Crash>().unwrap();
        assert_eq!(crash.jni, expected[5..]);
        assert_eq!(crash.jni.len(), 16);
        let report = crash.report();
        assert!(report.ends_with(&crash.jni.join("\n")));
        assert!(
            report.find("JNIEnv->").unwrap()
                > report
                    .find(&format!(
                        "0x{:x}:",
                        crash.stack.last().unwrap().0 - 3 * if arm64 { 8 } else { 4 }
                    ))
                    .unwrap()
        );
        assert!(!report.ends_with('\n'));
    }
    Ok(())
}
