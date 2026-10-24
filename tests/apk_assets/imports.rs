use super::*;
use std::{cell::RefCell, rc::Rc};
use unidroid::{Event, ImportCall, JniHandler, Trace};

#[test]
fn asset_import_hooks_wrap_builtins_before_and_after_loading_and_survive_rewind() -> Result<()> {
    #[derive(Default)]
    struct Wrapper {
        original: Option<u64>,
        calls: usize,
    }
    impl JniHandler for Wrapper {
        fn import(&mut self, emu: &mut Emulator, call: &ImportCall<'_>) -> Option<u64> {
            assert_eq!(call.name, "AAsset_getLength");
            self.original = call.original;
            self.calls += 1;
            Some(
                emu.call_function(call.original.unwrap(), call.args)
                    .unwrap()
                    + 10,
            )
        }
    }
    for dir in ["lib", "lib64"] {
        for before_load in [false, true] {
            let file = TestApk::assets(&[("file", b"abc", Stored)])?;
            let mut emu = Emulator::builder().apk(&file.path).build()?;
            if before_load {
                emu.hook_import("AAsset_getLength")?;
            }
            let module = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
            if !before_load {
                emu.hook_import("AAsset_getLength")?;
            }
            let asset = open(&mut emu, module, "file", 0)?;
            let function = emu.find_symbol(module, "AAsset_getLength")?;
            let saved = emu.checkpoint()?;
            let mut fresh = Emulator::builder().build()?;
            fresh.rewind(&saved)?;
            for target in [&mut emu, &mut fresh] {
                let mut wrapper = Wrapper::default();
                assert_eq!(
                    target.call_function_with(&mut wrapper, function, &[asset])?,
                    13
                );
                assert_eq!(wrapper.calls, 1);
                assert_ne!(wrapper.original.unwrap(), function);
                // A handler returning None jumps into the same original.
                assert_eq!(target.call_function(function, &[asset])?, 3);
            }
        }
    }
    Ok(())
}

#[test]
fn import_traces_name_asset_builtins_as_libandroid() -> Result<()> {
    for (abi, words) in [
        (
            "arm64-v8a",
            vec![0xa9bf7bfdu32, 0xd63f0060, 0xa8c17bfd, 0xd65f03c0],
        ),
        ("armeabi-v7a", vec![0xe92d4010, 0xe12fff33, 0xe8bd8010]),
    ] {
        let file = TestApk::assets(&[("file", b"abc", Stored)])?;
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .apk(&file.path)
            .trace(Trace {
                imports: true,
                ..Trace::default()
            })
            .on_event(move |event| {
                if let Event::Call {
                    function,
                    module,
                    from,
                    ..
                } = event
                {
                    sink.borrow_mut()
                        .push((function.clone(), module.clone(), from.clone()));
                }
            })
            .build()?;
        let module = emu.load_library(format!("tests/fixtures/{abi}/libjnidispatch.so"))?;
        let asset = open(&mut emu, module, "file", 0)?;
        let code = emu.find_symbol(module, "JNI_OnLoad")? & !1;
        call(&mut emu, module, "mprotect", &[code & !4095, 4096, 7])?;
        emu.write_bytes(
            code,
            &words
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        let length = emu.find_symbol(module, "AAsset_getLength")?;
        events.borrow_mut().clear();
        assert_eq!(emu.call_function(code, &[asset, 0, 0, length])?, 3);
        let events = events.borrow();
        let event = events
            .iter()
            .find(|event| event.0 == "AAsset_getLength")
            .unwrap();
        assert_eq!(event.1, "libandroid.so");
        assert!(event.2.starts_with("libjnidispatch.so+"), "{}", event.2);
    }
    Ok(())
}

#[test]
fn missing_libandroid_warning_distinguishes_builtin_assets_from_unresolved_symbols() -> Result<()> {
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let file = TestApk::assets(&[])?;
        for with_apk in [false, true] {
            let warnings = Rc::new(RefCell::new(Vec::new()));
            let sink = warnings.clone();
            let mut builder = Emulator::builder().on_event(move |event| {
                if let Event::Warning { message } = event {
                    sink.borrow_mut().push(message.clone());
                }
            });
            if with_apk {
                builder = builder.apk(&file.path);
            }
            let mut emu = builder.build()?;
            emu.load_library_without_constructors(format!(
                "tests/fixtures/{abi}/libtmessages.29.so"
            ))?;
            for library in ["libandroid.so", "libEGL.so"] {
                let symbols = if with_apk && library == "libandroid.so" {
                    "its asset functions are built in; the rest stay unresolved"
                } else {
                    "its symbols stay unresolved"
                };
                let expected = format!(
                    "{library} needed by libtmessages.29.so not found in the APK, next to the loaded libraries or in rootfs; {symbols}"
                );
                assert!(
                    warnings.borrow().contains(&expected),
                    "{abi}, APK={with_apk}: {:?}",
                    warnings.borrow()
                );
            }
        }
    }
    Ok(())
}

#[test]
fn asset_symbols_are_not_bound_without_an_apk() -> Result<()> {
    for dir in ["lib", "lib64"] {
        let mut emu = Emulator::builder().build()?;
        let module = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        for name in [
            "AAssetManager_fromJava",
            "AAssetManager_open",
            "AAsset_read",
            "AAsset_seek",
            "AAsset_seek64",
            "AAsset_getLength",
            "AAsset_getLength64",
            "AAsset_getRemainingLength",
            "AAsset_getRemainingLength64",
            "AAsset_getBuffer",
            "AAsset_close",
            "AAssetManager_openDir",
            "AAssetDir_getNextFileName",
            "AAssetDir_rewind",
            "AAssetDir_close",
        ] {
            assert!(emu.find_symbol(module, name).is_err(), "{name}");
        }
    }
    Ok(())
}
