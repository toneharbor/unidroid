use super::*;
use unidroid::{Policy, Probe};

struct Wrapper(usize);
impl JniHandler for Wrapper {
    fn import(&mut self, emu: &mut Emulator, call: &ImportCall<'_>) -> Option<u64> {
        self.0 += 1;
        let value = emu
            .call_function_with(self, call.original.unwrap(), call.args)
            .unwrap();
        Some(value + 10)
    }
}

#[test]
fn wrappers_call_the_original_and_policy_forwards_imports() -> Result<()> {
    for directory in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.hook_import("strlen")?;
        let module = emu.load_library(format!("rootfs/{directory}/libc.so"))?;
        let function = emu.find_symbol(module, "strlen")?;
        let text = emu.new_cstring("hello")?;
        let mut fallback = Wrapper(0);
        let mut policy = Policy::parse(r#"{"rules":[]}"#)?;
        let mut handler = policy.handler(&mut fallback);
        let result = emu.call_function_with(&mut handler, function, &[text]);
        assert_eq!(handler.finish(result)?, 15);
        assert_eq!(fallback.0, 1);
        assert_eq!(
            emu.call_function_with(&mut Probe::default(), function, &[text])?,
            5
        );
    }
    Ok(())
}

#[test]
fn passthrough_preserves_floating_arguments_and_results() -> Result<()> {
    for directory in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        let module = emu.load_library(format!("rootfs/{directory}/libm.so"))?;
        let original = emu.find_symbol(module, "sin")?;
        let bits = 0.5f64.to_bits();
        let args = if directory == "lib" {
            vec![bits as u32 as u64, bits >> 32]
        } else {
            Vec::new()
        };
        emu.set_register("d0", bits)?;
        let low = emu.call_function(original, &args)?;
        let expected = if directory == "lib" {
            low | emu.register("r1")? << 32
        } else {
            emu.register("d0")?
        };
        emu.hook_import("sin")?;
        let hook = emu.find_symbol(module, "sin")?;
        let mut handler = Answer {
            seen: Vec::new(),
            value: None,
        };
        emu.set_register("d0", bits)?;
        let low = emu.call_function_with(&mut handler, hook, &args)?;
        let result = if directory == "lib" {
            low | emu.register("r1")? << 32
        } else {
            emu.register("d0")?
        };
        assert_eq!(result, expected);
        assert_eq!(handler.seen[0].2, Some(original));
    }
    Ok(())
}

#[test]
fn passthrough_preserves_variadic_stack_arguments() -> Result<()> {
    for directory in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        let module = emu.load_library(format!("rootfs/{directory}/libc.so"))?;
        let original = emu.find_symbol(module, "snprintf")?;
        let output = emu.malloc(128)?;
        let format = emu.new_cstring("%d %d %d %d %d %d %d %d %d %d")?;
        let args = [output, 128, format, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        assert_eq!(emu.call_function(original, &args)?, 20);
        emu.write_bytes(output, &[0; 128])?;
        emu.hook_import("snprintf")?;
        let hook = emu.find_symbol(module, "snprintf")?;
        let mut handler = Answer {
            seen: Vec::new(),
            value: None,
        };
        assert_eq!(emu.call_function_with(&mut handler, hook, &args)?, 20);
        assert_eq!(emu.read_bytes(output, 21)?, b"1 2 3 4 5 6 7 8 9 10\0");
        assert_eq!(handler.seen.len(), 1);
    }
    Ok(())
}

#[test]
fn missing_original_can_be_answered_or_names_symbol_and_caller() -> Result<()> {
    for directory in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.hook_import("missing_import")?;
        let module = emu.load_library(format!("rootfs/{directory}/libc.so"))?;
        let function = emu.find_symbol(module, "missing_import")?;
        let mut handler = Answer {
            seen: Vec::new(),
            value: Some(0x123456789abcdef0),
        };
        let value = emu.call_function_with(&mut handler, function, &[9])?;
        if directory == "lib" {
            assert_eq!(value, 0x9abcdef0);
            assert_eq!(emu.register("r1")?, 0x12345678);
        } else {
            assert_eq!(value, 0x123456789abcdef0);
        }
        assert_eq!(handler.seen[0].2, None);
        handler.value = None;
        assert_eq!(
            emu.call_function_with(&mut handler, function, &[])
                .unwrap_err()
                .to_string(),
            "call to unresolved symbol missing_import from 0x20000"
        );
    }
    Ok(())
}

#[test]
fn hooks_keep_builtin_originals_before_and_after_loading() -> Result<()> {
    for directory in ["lib64", "lib"] {
        for early in [true, false] {
            let mut emu = Emulator::builder().build()?;
            if early {
                emu.hook_import("__system_property_get")?;
                emu.hook_import("dlopen")?;
            }
            let module = emu.load_library(format!("rootfs/{directory}/libc.so"))?;
            if !early {
                emu.hook_import("__system_property_get")?;
                emu.hook_import("dlopen")?;
            }
            let key = emu.new_cstring("ro.build.version.sdk")?;
            let output = emu.malloc(92)?;
            let property = emu.find_symbol(module, "__system_property_get")?;
            let open = emu.find_symbol(module, "dlopen")?;
            let mut handler = Wrapper(0);
            assert_eq!(
                emu.call_function_with(&mut handler, property, &[key, output])?,
                12
            );
            assert_eq!(emu.read_bytes(output, 3)?, b"23\0");
            assert_eq!(emu.call_function_with(&mut handler, open, &[0, 0])?, 11);
            assert_eq!(handler.0, 2);
            let lookup = emu.find_symbol(module, "dlsym")?;
            let name = emu.new_cstring("__system_property_get")?;
            assert_eq!(emu.call_function(lookup, &[1, name])?, property);
        }
    }
    Ok(())
}

#[test]
fn checkpoint_restores_hooks_into_existing_and_fresh_emulators() -> Result<()> {
    for directory in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.hook_import("strlen")?;
        emu.hook_import("__system_property_get")?;
        let module = emu.load_library(format!("rootfs/{directory}/libc.so"))?;
        let text = emu.new_cstring("hello")?;
        let checkpoint = emu.checkpoint()?;
        let mut fresh = Emulator::builder().build()?;
        fresh.hook_import("unused_pending_hook")?;
        emu.hook_import("strchr")?;
        for target in [&mut emu, &mut fresh] {
            target.rewind(&checkpoint)?;
            let hook = target.find_symbol(module, "strlen")?;
            let mut handler = Wrapper(0);
            assert_eq!(target.call_function_with(&mut handler, hook, &[text])?, 15);
            let key = target.new_cstring("ro.build.version.sdk")?;
            let output = target.malloc(92)?;
            let property = target.find_symbol(module, "__system_property_get")?;
            assert_eq!(
                target.call_function_with(&mut handler, property, &[key, output])?,
                12
            );
            assert_eq!(handler.0, 2);
        }
    }
    Ok(())
}
