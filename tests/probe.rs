use unidroid::{Emulator, Handle, JniCall, JniHandler, JniOp, JniValue, Probe, Result};

#[test]
fn probe_defaults_cover_members_constructors_and_assignability() -> Result<()> {
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(format!("tests/fixtures/{abi}/libjnidispatch.so"))?;
        let mut probe = Probe::default();
        for (op, is_static, signature, expected) in [
            (JniOp::Call, false, "()I", JniValue::Int(0)),
            (JniOp::Call, true, "()I", JniValue::Int(0)),
            (JniOp::Get, false, "I", JniValue::Int(0)),
            (JniOp::Set, false, "I", JniValue::Void),
            (JniOp::New, false, "()V", Handle(0).into()),
            (JniOp::Call, false, "()Z", JniValue::Boolean(false)),
            (JniOp::Call, false, "()J", JniValue::Long(0)),
            (JniOp::Call, false, "()D", JniValue::Double(0.0)),
            (JniOp::Call, false, "()V", JniValue::Void),
        ] {
            let call = JniCall {
                op,
                is_static,
                type_code: expected.type_code(),
                class: "test/Defaults",
                name: if op == JniOp::New { "<init>" } else { "value" },
                signature,
                receiver: Handle(0),
                args: &[],
            };
            assert_eq!(probe.member(&mut emu, &call), Some(expected));
        }
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
