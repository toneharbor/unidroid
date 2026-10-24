use super::*;

#[test]
fn reflected_members_keep_ids_and_identity_across_checkpoint_copies() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let class = emu.class("example/Owner");
        let mut members = Vec::new();
        let mut entries = Vec::new();
        for (lookup, name, signature, to, from) in [
            (33, "run", "()V", 9, 7),
            (113, "run", "()V", 9, 7),
            (33, "<init>", "()V", 9, 7),
            (94, "count", "I", 12, 8),
            (144, "count", "I", 12, 8),
        ] {
            let is_static = lookup >= 113;
            let host = if to == 9 {
                emu.new_reflected_method("example/Owner", name, signature, is_static)
            } else {
                emu.new_reflected_field("example/Owner", name, signature, is_static)
            };
            let id = get_id(&mut emu, lookup, class, name, signature)?;
            let guest = Handle(jni_call(
                &mut emu,
                to,
                &[class.0, id, u64::from(is_static)],
            )?);
            let alias = Handle(jni_call(&mut emu, 21, &[guest.0])?);
            assert_eq!(jni_call(&mut emu, 24, &[host.0, guest.0])?, 0);
            let value = emu.new_string(name);
            entries.push((guest, value));
            members.push((
                host,
                guest,
                alias,
                id,
                from,
                lookup,
                emu.reflected_member(guest)?,
            ));
        }
        let map = emu.new_java_map("java/util/HashMap", &entries)?;
        let saved = emu.checkpoint()?;
        for &(host, guest, alias, ..) in &members {
            emu.release(host);
            emu.release(guest);
            emu.release(alias);
        }
        call(&mut emu, map, "clear()V", &[])?;
        emu.new_reflected_method("example/Later", "later", "()V", true);
        let mut fresh = Emulator::builder().build()?;
        for target in [&mut emu, &mut fresh] {
            target.rewind(&saved)?;
            for (host, guest, alias, id, from, lookup, expected) in &members {
                assert_eq!(target.reflected_member(*host)?, *expected);
                assert_eq!(target.reflected_member(*guest)?, *expected);
                assert_eq!(target.reflected_member(*alias)?, *expected);
                assert_eq!(jni_call(target, *from, &[host.0])?, *id);
                assert_eq!(jni_call(target, *from, &[guest.0])?, *id);
                assert_eq!(
                    get_id(target, *lookup, class, &expected.name, &expected.signature)?,
                    *id
                );
                assert_eq!(jni_call(target, 24, &[guest.0, alias.0])?, 1);
                assert_eq!(jni_call(target, 24, &[host.0, guest.0])?, 0);
                let value = call(
                    target,
                    map,
                    "get(Ljava/lang/Object;)Ljava/lang/Object;",
                    &[(*alias).into()],
                )?
                .object();
                assert_eq!(target.string(value)?, expected.name);
            }
        }
    }
    Ok(())
}

#[test]
fn to_reflected_returns_local_references_and_host_handles_outlive_frames() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let class = emu.class("example/Owner");
        // Enter scope 2, the local scope used by guest JNI invocations.
        jni_call(&mut emu, 19, &[8])?;
        jni_call(&mut emu, 19, &[8])?;
        let host_method = emu.new_reflected_method("example/Owner", "run", "()V", false);
        let host_field = emu.new_reflected_field("example/Owner", "count", "I", true);
        let mut locals = Vec::new();
        for (host, from, to, is_static) in [(host_method, 7, 9, 0), (host_field, 8, 12, 1)] {
            let id = jni_call(&mut emu, from, &[host.0])?;
            let local = jni_call(&mut emu, to, &[class.0, id, is_static])?;
            assert_eq!(jni_call(&mut emu, 232, &[local])?, 1);
            locals.push((local, from));
        }
        jni_call(&mut emu, 20, &[0])?;
        assert_eq!(emu.reflected_member(host_method)?.name, "run");
        assert_eq!(emu.reflected_member(host_field)?.name, "count");
        for (local, from) in locals {
            let error = jni_call(&mut emu, from, &[local]).unwrap_err().to_string();
            assert!(error.contains("invalid reference"));
            assert!(error.ends_with("from 0x20000"));
        }
    }
    Ok(())
}
