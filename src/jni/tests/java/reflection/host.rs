use super::*;
use crate::{ReflectedMember, ReflectedMemberKind};

#[test]
fn host_reflection_shares_ids_and_exposes_member_details() -> Result<()> {
    for arch in ARCHES {
        for lookup_first in [false, true] {
            let mut emu = Emulator::builder().build()?;
            let early = emu.new_reflected_method("example/Early", "run", "()V", false);
            assert_eq!(emu.reflected_member(early)?.class, "example/Early");
            emu.setup_cpu(arch)?;
            let class = emu.class("example/Owner");
            let other_class = emu.class("example/Other");
            let mut ids = Vec::new();
            for (lookup, name, signature, kind) in [
                (33, "run", "()V", ReflectedMemberKind::Method),
                (113, "run", "()V", ReflectedMemberKind::Method),
                (33, "run", "(I)V", ReflectedMemberKind::Method),
                (33, "other", "()V", ReflectedMemberKind::Method),
                (33, "<init>", "()V", ReflectedMemberKind::Constructor),
                (94, "count", "I", ReflectedMemberKind::Field),
                (144, "count", "I", ReflectedMemberKind::Field),
            ] {
                let is_static = lookup >= 113;
                let (to, from) = if kind == ReflectedMemberKind::Field {
                    (12, 8)
                } else {
                    (9, 7)
                };
                let before = if lookup_first {
                    Some(get_id(&mut emu, lookup, class, name, signature)?)
                } else {
                    None
                };
                jni_call(&mut emu, 19, &[8])?;
                let host = if kind == ReflectedMemberKind::Field {
                    emu.new_reflected_field("example/Owner", name, signature, is_static)
                } else {
                    emu.new_reflected_method("example/Owner", name, signature, is_static)
                };
                jni_call(&mut emu, 20, &[0])?;
                let id = jni_call(&mut emu, from, &[host.0])?;
                assert_eq!(get_id(&mut emu, lookup, class, name, signature)?, id);
                if let Some(before) = before {
                    assert_eq!(before, id);
                }
                assert!(!ids.contains(&id));
                ids.push(id);
                let expected = ReflectedMember {
                    class: "example/Owner".into(),
                    name: name.into(),
                    signature: signature.into(),
                    is_static,
                    kind,
                };
                assert_eq!(emu.reflected_member(host)?, expected);
                // ToReflected uses the ID's declaring class, and the supplied static flag.
                let guest = Handle(jni_call(
                    &mut emu,
                    to,
                    &[other_class.0, id, u64::from(!is_static)],
                )?);
                assert_eq!(
                    emu.reflected_member(guest)?,
                    ReflectedMember {
                        is_static: !is_static,
                        ..expected
                    }
                );
                assert_eq!(jni_call(&mut emu, from, &[guest.0])?, id);
                assert_ne!(get_id(&mut emu, lookup, other_class, name, signature)?, id);
            }
            for handle in [
                emu.new_string("plain"),
                emu.new_object("java/lang/reflect/Method", ()),
            ] {
                let class = if emu.string(handle).is_ok() {
                    "java/lang/String"
                } else {
                    "java/lang/reflect/Method"
                };
                assert_eq!(
                    emu.reflected_member(handle).unwrap_err().to_string(),
                    format!(
                        "invalid handle {}: {class}, not a reflected member",
                        handle.0
                    )
                );
            }
            for handle in [Handle(0), Handle(0x123456)] {
                assert_eq!(
                    emu.reflected_member(handle).unwrap_err().to_string(),
                    format!(
                        "invalid handle {}: no live object (never created, or already released)",
                        handle.0
                    )
                );
            }
            emu.release(early);
            assert!(emu.reflected_member(early).is_err());
        }
    }
    Ok(())
}

struct DeclarationHandler;
impl JniHandler for DeclarationHandler {
    fn call_object_method(&mut self, emu: &mut Emulator, call: &JniCall<'_>) -> Option<Handle> {
        assert_eq!(call.class, "java/lang/Class");
        assert_eq!(call.name, "getDeclaredMethod");
        assert_eq!(
            call.signature,
            "(Ljava/lang/String;[Ljava/lang/Class;)Ljava/lang/reflect/Method;"
        );
        assert_eq!(emu.string(call.args[0].object()).unwrap(), "run");
        assert!(emu.object_array(call.args[1].object()).unwrap().is_empty());
        Some(emu.new_reflected_method("example/Owner", "run", "()V", false))
    }
    fn call_int_method(&mut self, emu: &mut Emulator, call: &JniCall<'_>) -> Option<i32> {
        assert_eq!(call.class, "java/lang/reflect/Method");
        assert_eq!(call.name, "getModifiers");
        let member = emu.reflected_member(call.receiver).unwrap();
        assert_eq!(member.class, "example/Owner");
        assert_eq!(member.name, "run");
        assert_eq!(member.signature, "()V");
        assert_eq!(member.kind, ReflectedMemberKind::Method);
        Some(if member.is_static { 8 } else { 0 })
    }
}

#[test]
fn handler_returns_reflected_method_before_and_after_guest_lookup() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let class = emu.class("example/Owner");
        let name = emu.new_string("run");
        let parameters = emu.new_object_array("java/lang/Class", &[])?;
        let mut ids = Vec::new();
        for _ in 0..2 {
            let reflected = member(
                &mut emu,
                &mut DeclarationHandler,
                class,
                "java/lang/Class",
                "getDeclaredMethod(Ljava/lang/String;[Ljava/lang/Class;)Ljava/lang/reflect/Method;",
                &[name.into(), parameters.into()],
                0,
            )?
            .object();
            let id = jni_call(&mut emu, 7, &[reflected.0])?;
            assert_eq!(get_id(&mut emu, 33, class, "run", "()V")?, id);
            ids.push(id);
            assert_eq!(
                member(
                    &mut emu,
                    &mut DeclarationHandler,
                    reflected,
                    "java/lang/reflect/Method",
                    "getModifiers()I",
                    &[],
                    0,
                )?,
                JniValue::Int(0)
            );
        }
        assert_eq!(ids[0], ids[1]);
    }
    Ok(())
}
