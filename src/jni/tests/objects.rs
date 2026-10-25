use super::*;

#[test]
fn class_lookups_return_independent_local_references() -> Result<()> {
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        for index in [6, 31, 10] {
            let mut emu = Emulator::builder().build()?;
            emu.setup_cpu(arch)?;
            let name = "example/Bridge";
            let argument = match index {
                6 => emu.new_cstring(name)?,
                31 => emu.new_object(name, ()).0,
                _ => {
                    emu.superclass("example/Child", name);
                    emu.class("example/Child").0
                }
            };
            jni_call(&mut emu, 19, &[16])?;
            jni_call(&mut emu, 19, &[16])?;
            let first = jni_call(&mut emu, index, &[argument])?;
            let second = jni_call(&mut emu, index, &[argument])?;
            assert_ne!(first, second, "JNIEnv index {index}, {arch:?}");
            assert_eq!(jni_call(&mut emu, 24, &[first, second])?, 1);
            jni_call(&mut emu, 23, &[second])?;
            assert_eq!(jni_call(&mut emu, 232, &[second])?, 0);
            let method = emu.new_cstring("call")?;
            let signature = emu.new_cstring("()V")?;
            assert_ne!(jni_call(&mut emu, 113, &[first, method, signature])?, 0);
            let third = jni_call(&mut emu, index, &[argument])?;
            assert_eq!(jni_call(&mut emu, 24, &[first, third])?, 1);
            jni_call(&mut emu, 20, &[0])?;
            assert_eq!(jni_call(&mut emu, 232, &[first])?, 0);
            assert_eq!(jni_call(&mut emu, 232, &[third])?, 0);
        }
    }
    Ok(())
}

#[test]
fn guest_class_lookups_leave_api_handles_stable() -> Result<()> {
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        for api_first in [false, true] {
            let mut emu = Emulator::builder().build()?;
            emu.setup_cpu(arch)?;
            let name = "example/Bridge";
            let stable = api_first.then(|| emu.class(name));
            let text = emu.new_cstring(name)?;
            jni_call(&mut emu, 19, &[16])?;
            jni_call(&mut emu, 19, &[16])?;
            let local = jni_call(&mut emu, 6, &[text])?;
            let api = emu.class(name);
            if let Some(stable) = stable {
                assert_eq!(api, stable);
            }
            assert_ne!(api.0, local);
            assert_eq!(jni_call(&mut emu, 24, &[api.0, local])?, 1);
            jni_call(&mut emu, 23, &[local])?;
            assert_eq!(jni_call(&mut emu, 232, &[local])?, 0);
            emu.release(api);
            let local = jni_call(&mut emu, 6, &[text])?;
            assert_eq!(jni_call(&mut emu, 232, &[api.0])?, 0);
            assert_eq!(emu.class(name), api);
            assert_eq!(jni_call(&mut emu, 24, &[api.0, local])?, 1);
            jni_call(&mut emu, 23, &[local])?;
            jni_call(&mut emu, 20, &[0])?;
            assert_eq!(jni_call(&mut emu, 232, &[api.0])?, 1);
            assert_eq!(emu.class(name), api);
        }
    }
    Ok(())
}

#[test]
fn checkpoints_preserve_guest_and_api_class_identity() -> Result<()> {
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(arch)?;
        let name = emu.new_cstring("example/GuestOnly")?;
        let guest = jni_call(&mut emu, 6, &[name])?;
        let api = emu.class("example/Api");
        let api_name = emu.new_cstring("example/Api")?;
        let alias = jni_call(&mut emu, 6, &[api_name])?;
        emu.release(api);
        let saved = emu.checkpoint()?;
        let mut copy = Emulator::builder().build()?;
        copy.rewind(&saved)?;
        emu.rewind(&saved)?;
        for current in [&mut emu, &mut copy] {
            let again = jni_call(current, 6, &[name])?;
            assert_ne!(again, guest);
            assert_eq!(jni_call(current, 24, &[guest, again])?, 1);
            let api_guest = current.class("example/GuestOnly");
            assert_ne!(api_guest.0, guest);
            assert_eq!(jni_call(current, 24, &[guest, api_guest.0])?, 1);
            assert_eq!(current.class("example/Api"), api);
            assert_eq!(jni_call(current, 24, &[api.0, alias])?, 1);
        }
    }
    Ok(())
}

#[test]
fn local_frames_and_string_constructors_reject_invalid_guest_references() -> Result<()> {
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(arch)?;
        assert_eq!(
            jni_call(&mut emu, 20, &[999]).unwrap_err().to_string(),
            "invalid JNI reference 0x3e7 in PopLocalFrame from 0x20000"
        );
        let class = emu.class("java/lang/String");
        let name = emu.new_cstring("<init>")?;
        let signature = emu.new_cstring("([BLjava/lang/String;)V")?;
        let id = jni_call(&mut emu, 33, &[class.0, name, signature])?;
        let bytes = emu.new_byte_array(b"abc");
        let charset = emu.new_string("UTF-8");
        for (first, second, invalid) in [
            (999, charset.0, 999),
            (charset.0, charset.0, charset.0),
            (bytes.0, 999, 999),
            (bytes.0, bytes.0, bytes.0),
        ] {
            assert_eq!(
                jni_call(&mut emu, 28, &[class.0, id, first, second])
                    .unwrap_err()
                    .to_string(),
                format!("invalid JNI reference 0x{invalid:x} in NewObject from 0x20000")
            );
        }
        let string = jni_call(&mut emu, 28, &[class.0, id, bytes.0, charset.0])?;
        assert_eq!(emu.string(Handle(string))?, "abc");
    }
    Ok(())
}

#[test]
fn arm32_direct_buffers_use_a_register_pair_for_capacity() -> Result<()> {
    let mut emu = Emulator::builder().build()?;
    emu.setup_cpu(crate::Architecture::Arm32)?;
    let table = emu.read_pointer(emu.jni.env)?;
    let memory = emu.malloc(16)?;
    let buffer = emu.call_function(
        emu.read_pointer(table + 229 * 4)?,
        &[emu.jni.env, memory, 0x89ab_cdef, 0x1234_5678],
    )?;
    let capacity = emu.read_pointer(table + 231 * 4)?;
    assert_eq!(
        emu.invoke_jni(
            capacity,
            &[Handle(emu.jni.env).into(), Handle(buffer).into()],
            b'J',
            &mut ()
        )?,
        JniValue::Long(0x1234_5678_89ab_cdef)
    );
    assert_eq!(emu.reg(0)?, 0x89ab_cdef);
    assert_eq!(emu.reg(1)?, 0x1234_5678);
    Ok(())
}

#[test]
fn handles_wrap_without_reusing_live_or_remembered_class_ids() -> Result<()> {
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(arch)?;
        let live = emu.new_string("live");
        let class = emu.class("example/Remembered");
        emu.release(class);
        let free = emu.new_string("released");
        emu.release(free);
        emu.jni.next_handle = 0x7fff_fffe;
        assert_eq!(emu.new_string("last"), Handle(0x7fff_ffff));
        assert_eq!(emu.new_string("wrapped"), free);
        assert_eq!(emu.string(live)?, "live");
        assert_eq!(emu.class("example/Remembered"), class);

        let table = emu.read_pointer(emu.jni.env)?;
        let name = emu.new_cstring("field")?;
        let signature = emu.new_cstring("J")?;
        let id = emu.call_function(
            emu.read_pointer(table + 94 * emu.pointer_size())?,
            &[emu.jni.env, class.0, name, signature],
        )?;
        assert_eq!(id, 0x8000_0000);
        let push = emu.read_pointer(table + 19 * emu.pointer_size())?;
        let pop = emu.read_pointer(table + 20 * emu.pointer_size())?;
        emu.call_function(push, &[emu.jni.env, 16])?;
        emu.jni.next_handle = 0x7fff_ffff;
        let result = emu.call_function(pop, &[emu.jni.env, live.0])?;
        assert_eq!(result, 4);
        assert_eq!(emu.string(Handle(result))?, "live");
    }
    Ok(())
}

#[test]
fn api_handles_created_by_a_handler_live_until_release() -> Result<()> {
    #[derive(Default)]
    struct Handler(Handle);
    impl JniHandler for Handler {
        fn call_static_void_method(&mut self, emu: &mut Emulator, _: &JniCall<'_>) -> Option<()> {
            self.0 = emu.new_object("example/Value", 42u32);
            Some(())
        }
    }
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(arch)?;
        let p = emu.pointer_size();
        let class = emu.class("example/Factory");
        let name = emu.new_cstring("create")?;
        let sig = emu.new_cstring("()V")?;
        let table = emu.read_pointer(emu.jni.env)?;
        let get_id = emu.read_pointer(table + 113 * p)?;
        let method = emu.call_function(get_id, &[emu.jni.env, class.0, name, sig])?;
        let function = emu.read_pointer(table + 141 * p)?;
        let mut handler = Handler::default();
        emu.invoke_jni(
            function,
            &[
                JniValue::Object(Handle(emu.jni.env)),
                class.into(),
                JniValue::Object(Handle(method)),
            ],
            b'V',
            &mut handler,
        )?;
        assert_eq!(*emu.user_value::<u32>(handler.0).unwrap(), 42);
        emu.release(handler.0);
        let reference_type = emu.read_pointer(table + 232 * p)?;
        assert_eq!(
            emu.call_function(reference_type, &[emu.jni.env, handler.0.0])?,
            0
        );
    }
    Ok(())
}

#[test]
fn functions_taking_pointers_or_array_references_are_not_rejected() -> Result<()> {
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        let (mut emu, _) = libc_emulator(arch)?;
        let p = emu.pointer_size();
        let table = emu.read_pointer(emu.jni.env)?;
        let text = emu.new_cstring("hello")?;
        let string = emu.call_function(emu.read_pointer(table + 167 * p)?, &[emu.jni.env, text])?;
        assert_eq!(emu.string(Handle(string))?, "hello");
        let array = emu.new_byte_array(&[1, 2, 3]);
        let ptr = emu.call_function(
            emu.read_pointer(table + 222 * p)?,
            &[emu.jni.env, array.0, 0],
        )?;
        emu.write_bytes(ptr, &[9])?;
        emu.call_function(
            emu.read_pointer(table + 223 * p)?,
            &[emu.jni.env, array.0, ptr, 0],
        )?;
        assert_eq!(emu.byte_array(array)?, [9, 2, 3]);
        let memory = emu.malloc(16)?;
        let buffer = emu
            .invoke_jni(
                emu.read_pointer(table + 229 * p)?,
                &[
                    Handle(emu.jni.env).into(),
                    Handle(memory).into(),
                    JniValue::Long(16),
                ],
                b'L',
                &mut (),
            )?
            .object()
            .0;
        assert_eq!(
            emu.call_function(emu.read_pointer(table + 230 * p)?, &[emu.jni.env, buffer])?,
            memory
        );
        assert_eq!(
            emu.call_function(emu.read_pointer(table + 231 * p)?, &[emu.jni.env, buffer])?,
            16
        );
    }
    Ok(())
}

#[test]
fn class_identity_survives_local_reference_cleanup() -> Result<()> {
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(arch)?;
        let p = emu.pointer_size();
        emu.jni.scope = 2;
        let local = emu.class("example/Test");
        let global = emu.alias(local, 1)?;
        emu.release(local);
        let again = emu.class("example/Test");
        let table = emu.read_pointer(emu.jni.env)?;
        let is_same = emu.read_pointer(table + 24 * p)?;
        assert_eq!(
            emu.call_function(is_same, &[emu.jni.env, again.0, global.0])?,
            1
        );
    }
    Ok(())
}

#[test]
fn object_arrays_exceptions_and_bad_ids_are_safe() -> Result<()> {
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(arch)?;
        let p = emu.pointer_size();
        let table = emu.read_pointer(emu.jni.env)?;
        let class = emu.class("java/lang/String");
        let initial = emu.new_string("x");
        let array = emu.call_function(
            emu.read_pointer(table + 172 * p)?,
            &[emu.jni.env, 3, class.0, initial.0],
        )?;
        assert_eq!(
            emu.call_function(emu.read_pointer(table + 171 * p)?, &[emu.jni.env, array])?,
            3
        );
        let second = emu.new_string("y");
        emu.call_function(
            emu.read_pointer(table + 174 * p)?,
            &[emu.jni.env, array, 1, second.0],
        )?;
        let element =
            emu.call_function(emu.read_pointer(table + 173 * p)?, &[emu.jni.env, array, 1])?;
        assert_eq!(emu.string(Handle(element))?, "y");
        let throw_new = emu.read_pointer(table + 14 * p)?;
        let message = emu.new_cstring("bad input")?;
        assert_eq!(
            emu.call_function(throw_new, &[emu.jni.env, class.0, message])?,
            0
        );
        assert_eq!(
            emu.call_function(emu.read_pointer(table + 228 * p)?, &[emu.jni.env])?,
            1
        );
        let exception = emu.call_function(emu.read_pointer(table + 15 * p)?, &[emu.jni.env])?;
        assert_ne!(exception, 0);
        emu.call_function(emu.read_pointer(table + 17 * p)?, &[emu.jni.env])?;
        assert_eq!(
            emu.call_function(emu.read_pointer(table + 228 * p)?, &[emu.jni.env])?,
            0
        );
        let invalid = emu
            .call_function(emu.read_pointer(table + 171 * p)?, &[emu.jni.env, 0])
            .unwrap_err()
            .to_string();
        assert!(invalid.starts_with("invalid JNI reference 0x0 in GetArrayLength"));
        let invalid_id = emu
            .call_function(
                emu.read_pointer(table + 141 * p)?,
                &[emu.jni.env, class.0, 0x1234],
            )
            .unwrap_err()
            .to_string();
        assert!(invalid_id.starts_with("invalid JNI id 0x1234 in CallStaticVoidMethod"));
        let weak =
            emu.call_function(emu.read_pointer(table + 226 * p)?, &[emu.jni.env, class.0])?;
        assert_eq!(
            emu.call_function(emu.read_pointer(table + 232 * p)?, &[emu.jni.env, weak])?,
            3
        );
    }
    Ok(())
}

#[test]
fn unsupported_jni_functions_have_stable_errors() -> Result<()> {
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(arch)?;
        let p = emu.pointer_size();
        let table = emu.read_pointer(emu.jni.env)?;
        let error = emu
            .call_function(emu.read_pointer(table + 5 * p)?, &[emu.jni.env, 0, 0, 0])
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("unsupported JNIEnv function DefineClass"));
    }
    Ok(())
}
