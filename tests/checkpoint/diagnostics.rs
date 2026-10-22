use super::{code, emulator};
use std::{cell::RefCell, rc::Rc};
use unidroid::{
    Checkpoint, Emulator, Handle, JniCall, JniHandler, Policy, Result, Resume, Stop, Trace, Watch,
    WatchAction,
};

struct DuringCall<'a> {
    saved: &'a Checkpoint,
    checked: usize,
}
impl DuringCall<'_> {
    fn check(&mut self, emu: &mut Emulator) {
        let expected = "checkpoint and rewind work only between calls";
        assert_eq!(emu.checkpoint().err().unwrap().to_string(), expected);
        assert_eq!(emu.rewind(self.saved).unwrap_err().to_string(), expected);
        self.checked += 1;
    }
}
impl JniHandler for DuringCall<'_> {
    fn stopped(&mut self, emu: &mut Emulator, _: &Stop) -> Resume {
        self.check(emu);
        Resume::Return(42)
    }
    fn get_static_object_field(&mut self, emu: &mut Emulator, _: &JniCall<'_>) -> Option<Handle> {
        self.check(emu);
        None
    }
}

#[test]
fn calls_and_other_architectures_reject_rewind_without_changing_the_checkpoint() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        let abi = if arm64 { "arm64-v8a" } else { "armeabi-v7a" };
        let module = emu.load_library(format!("tests/fixtures/{abi}/libjnidispatch.so"))?;
        let checkpoint = emu.checkpoint()?;
        let mut handler = DuringCall {
            saved: &checkpoint,
            checked: 0,
        };
        let entry = emu.find_symbol(module, "Java_com_sun_jna_Native_getNativeVersion")?;
        emu.add_breakpoint(entry)?;
        assert_eq!(
            emu.call_static_jni_method(
                &mut handler,
                "com/sun/jna/Native",
                "getNativeVersion()I",
                &[]
            )?
            .int(),
            42
        );
        emu.remove_breakpoint(entry)?;
        let error = emu.call_jni_onload(module, &mut handler).unwrap_err();
        assert!(error.to_string().contains("java/lang/Void->TYPE"));
        assert_eq!(handler.checked, 2);
        emu.rewind(&checkpoint)?;
        // Crash history stays with the emulator, even though handles were rewound.
        let error = emu.call_function(0x30000000, &[]).unwrap_err();
        assert!(
            !error
                .downcast_ref::<unidroid::Crash>()
                .unwrap()
                .jni
                .is_empty()
        );
        emu.checkpoint()?;
        let mut other = emulator(!arm64)?;
        let before = other.snapshot()?;
        assert_eq!(
            other.rewind(&checkpoint).unwrap_err().to_string(),
            if arm64 {
                "cannot rewind arm64 checkpoint into arm32 emulator"
            } else {
                "cannot rewind arm32 checkpoint into arm64 emulator"
            }
        );
        let diff = before.diff(&other.snapshot()?);
        assert!(diff.registers.is_empty() && diff.changes.is_empty());
    }
    let mut fresh = Emulator::builder().build()?;
    assert!(
        fresh
            .checkpoint()
            .err()
            .unwrap()
            .to_string()
            .contains("no library loaded yet")
    );
    Ok(())
}

#[test]
fn copy_keeps_the_targets_debugger_events_and_coverage_counts() -> Result<()> {
    for arm64 in [true, false] {
        let mut source = emulator(arm64)?;
        let address = source.malloc(4)?;
        let function = code(
            &mut source,
            0x20200,
            if arm64 {
                &[0xb9000001, 0xd65f03c0] // str w1,[x0]; ret
            } else {
                &[0xe5801000, 0xe12fff1e] // str r1,[r0]; bx lr
            },
        )?;
        let checkpoint = source.checkpoint()?;
        let events = Rc::new(RefCell::new(Vec::new()));
        let capture = events.clone();
        let mut target = Emulator::builder()
            .translation_cache_size(16 * 1024 * 1024)
            .trace(Trace {
                coverage: vec!["libc.so".into()],
                ..Trace::default()
            })
            .on_event(move |event| {
                capture
                    .borrow_mut()
                    .push(serde_json::to_value(event).unwrap())
            })
            .build()?;
        target.load_library(format!(
            "rootfs/{}/libc.so",
            if arm64 { "lib64" } else { "lib" }
        ))?;
        let getpid = target.resolve("getpid")?;
        target.call_function(getpid, &[])?;
        let counts = serde_json::to_value(target.coverage())?;
        target.add_breakpoint(function)?;
        target.add_watchpoint(address, 4, Watch::Write, WatchAction::Log)?;
        target.rewind(&checkpoint)?;
        assert_eq!(serde_json::to_value(target.coverage())?, counts);
        assert_eq!(target.trace().coverage, ["libc.so"]);
        struct Stops(usize);
        impl JniHandler for Stops {
            fn stopped(&mut self, _: &mut Emulator, _: &Stop) -> Resume {
                self.0 += 1;
                Resume::Continue
            }
        }
        let mut stops = Stops(0);
        target.call_function_with(&mut stops, function, &[address, 99])?;
        assert_eq!(stops.0, 1);
        assert!(
            events
                .borrow()
                .iter()
                .any(|event| event["event"] == "watch")
        );
        assert_eq!(target.read_bytes(address, 4)?, 99u32.to_le_bytes());
        assert_eq!(source.read_bytes(address, 4)?, [0; 4]);
        target.remove_breakpoint(function)?;
        target.remove_watchpoint(address)?;
    }
    Ok(())
}

#[test]
fn policy_clone_preserves_the_current_sequence_position() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        let mut policy =
            Policy::parse(r#"{"rules":[{"op":"is_assignable","values":[true,false,true]}]}"#)?;
        assert_eq!(
            policy.handler(&mut ()).is_assignable(&mut emu, "A", "B"),
            Some(true)
        );
        let saved = policy.clone();
        for _ in 0..2 {
            policy = saved.clone();
            assert_eq!(
                policy.handler(&mut ()).is_assignable(&mut emu, "A", "B"),
                Some(false)
            );
            assert_eq!(
                policy.handler(&mut ()).is_assignable(&mut emu, "A", "B"),
                Some(true)
            );
            assert_eq!(
                policy.handler(&mut ()).is_assignable(&mut emu, "A", "B"),
                None
            );
        }
    }
    Ok(())
}
