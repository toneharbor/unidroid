use unidroid::{Emulator, Handle, JniCall, JniHandler, Probe, Result};

#[test]
fn probe_defaults_cover_members_constructors_and_assignability() -> Result<()> {
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(format!("tests/fixtures/{abi}/libjnidispatch.so"))?;
        let mut probe = Probe::default();
        let call = |signature| JniCall {
            class: "test/Defaults",
            name: "value",
            signature,
            receiver: Handle(0),
            args: &[],
        };
        assert_eq!(probe.call_int_method(&mut emu, &call("()I")), Some(0));
        assert_eq!(
            probe.call_static_int_method(&mut emu, &call("()I")),
            Some(0)
        );
        assert_eq!(probe.get_int_field(&mut emu, &call("I")), Some(0));
        assert_eq!(probe.set_int_field(&mut emu, &call("I")), Some(()));
        assert_eq!(
            probe.new_object(
                &mut emu,
                &JniCall {
                    name: "<init>",
                    ..call("()V")
                }
            ),
            Some(Handle(0))
        );
        assert_eq!(
            probe.call_boolean_method(&mut emu, &call("()Z")),
            Some(false)
        );
        assert_eq!(probe.call_long_method(&mut emu, &call("()J")), Some(0));
        assert_eq!(probe.call_double_method(&mut emu, &call("()D")), Some(0.0));
        assert_eq!(probe.call_void_method(&mut emu, &call("()V")), Some(()));
        assert_eq!(
            probe.is_assignable(&mut emu, "test/A", "test/B"),
            Some(false)
        );
        assert_eq!(probe.calls().len(), 10);
    }
    Ok(())
}

#[test]
fn probe_returns_defaults_and_counts_calls_in_first_seen_order() -> Result<()> {
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let mut emu = Emulator::builder().build()?;
        let m = emu.load_library(format!("tests/fixtures/{abi}/libjnidispatch.so"))?;
        let mut probe = Probe::default();
        assert_eq!(emu.call_jni_onload(m, &mut probe)?, 0);
        assert_eq!(probe.calls()[0].class, "java/lang/Void");
        assert_eq!(probe.calls()[0].op, "get");
        assert!(probe.calls()[0].is_static);
        assert_eq!(probe.calls()[0].count, 1);
        assert!(probe.calls()[0].from.starts_with("libjnidispatch.so+"));
        let first = probe.calls()[0].from.clone();
        let count = probe.calls().len();
        let _ = emu.call_jni_onload(m, &mut probe);
        assert_eq!(probe.calls().len(), count);
        assert!(probe.calls().iter().all(|c| c.count == 2));
        assert_eq!(probe.calls()[0].from, first);
    }
    Ok(())
}
