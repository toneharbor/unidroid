use super::*;
use crate::ReflectedMemberKind;

fn reflected(
    emu: &mut Emulator,
    kind: ReflectedMemberKind,
    class: &str,
    name: &str,
    signature: &str,
    is_static: bool,
) -> Handle {
    if kind == ReflectedMemberKind::Field {
        emu.new_reflected_field(class, name, signature, is_static)
    } else {
        emu.new_reflected_method(class, name, signature, is_static)
    }
}

#[test]
fn reflected_equality_and_hashes_follow_java_member_values() -> Result<()> {
    use ReflectedMemberKind::{Constructor, Field, Method};
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let class = emu.class("a/B");
        // Java String hashes: "a.B" = 94709, "m" = 109, "f" = 102.
        for (kind, name, signature, other_signature, lookup, to, from, hash) in [
            (Method, "m", "(I)I", "(J)I", 33, 9, 7, 94616),
            (Constructor, "<init>", "(I)V", "(J)V", 33, 9, 7, 94709),
            (Field, "f", "I", "J", 94, 12, 8, 94611),
        ] {
            let first = reflected(&mut emu, kind, "a/B", name, signature, false);
            let same = reflected(&mut emu, kind, "a/B", name, signature, false);
            let static_member = reflected(&mut emu, kind, "a/B", name, signature, true);
            let id = get_id(&mut emu, lookup, class, name, signature)?;
            let guest = Handle(jni_call(&mut emu, to, &[class.0, id, 1])?);
            assert_ne!(jni_call(&mut emu, from, &[static_member.0])?, id);
            for other in [same, static_member, guest] {
                assert_eq!(jni_call(&mut emu, 24, &[first.0, other.0])?, 0);
                for (a, b) in [(first, other), (other, first)] {
                    assert_eq!(
                        call(&mut emu, a, "equals(Ljava/lang/Object;)Z", &[b.into()])?,
                        JniValue::Boolean(true)
                    );
                    assert_eq!(call(&mut emu, a, "hashCode()I", &[])?, JniValue::Int(hash));
                }
            }
            let other_type = reflected(&mut emu, kind, "a/B", name, other_signature, false);
            let wrong_class = reflected(&mut emu, kind, "a/C", name, signature, false);
            let wrong_name = reflected(&mut emu, kind, "a/B", "other", signature, false);
            let wrong_kind = reflected(
                &mut emu,
                if kind == Field { Method } else { Field },
                "a/B",
                name,
                signature,
                false,
            );
            let opaque_class = match kind {
                Method => "java/lang/reflect/Method",
                Constructor => "java/lang/reflect/Constructor",
                Field => "java/lang/reflect/Field",
            };
            let opaque = emu.new_object(opaque_class, ());
            let text = emu.new_string(name);
            for other in [
                other_type,
                wrong_class,
                wrong_name,
                wrong_kind,
                opaque,
                text,
                Handle(0),
            ] {
                assert_eq!(
                    call(
                        &mut emu,
                        first,
                        "equals(Ljava/lang/Object;)Z",
                        &[other.into()]
                    )?,
                    JniValue::Boolean(false)
                );
            }
            // Overloads and different field types may collide in hashCode, but are not equal.
            assert_eq!(
                call(&mut emu, other_type, "hashCode()I", &[])?,
                JniValue::Int(hash)
            );
            if kind == Method {
                let other_return = emu.new_reflected_method("a/B", name, "(I)J", false);
                assert_eq!(
                    call(
                        &mut emu,
                        first,
                        "equals(Ljava/lang/Object;)Z",
                        &[other_return.into()]
                    )?,
                    JniValue::Boolean(false)
                );
                assert_eq!(
                    call(&mut emu, other_return, "hashCode()I", &[])?,
                    JniValue::Int(hash)
                );
            }
        }
    }
    Ok(())
}

#[test]
fn reflected_members_are_value_keys_in_maps_sets_and_lists() -> Result<()> {
    use ReflectedMemberKind::{Constructor, Field, Method};
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        for (kind, name, signature, other_signature) in [
            (Method, "m", "(I)I", "(J)I"),
            (Constructor, "<init>", "(I)V", "(J)V"),
            (Field, "f", "I", "J"),
        ] {
            let first = reflected(&mut emu, kind, "a/B", name, signature, false);
            let equal = reflected(&mut emu, kind, "a/B", name, signature, true);
            let different = reflected(&mut emu, kind, "a/B", name, other_signature, false);
            let one = emu.new_string("one");
            let two = emu.new_string("two");
            let map = emu.new_java_map("java/util/HashMap", &[(first, one), (equal, two)])?;
            assert_eq!(call(&mut emu, map, "size()I", &[])?, JniValue::Int(1));
            let entries = emu.java_map(map)?;
            assert_eq!(jni_call(&mut emu, 24, &[entries[0].0.0, first.0])?, 1);
            assert_eq!(emu.string(entries[0].1)?, "two");
            let previous = call(
                &mut emu,
                map,
                "put(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
                &[equal.into(), one.into()],
            )?
            .object();
            assert_eq!(emu.string(previous)?, "two");
            assert_eq!(call(&mut emu, map, "size()I", &[])?, JniValue::Int(1));
            call(
                &mut emu,
                map,
                "put(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
                &[different.into(), two.into()],
            )?;
            assert_eq!(call(&mut emu, map, "size()I", &[])?, JniValue::Int(2));
            for (key, expected) in [(first, "one"), (equal, "one"), (different, "two")] {
                let value = call(
                    &mut emu,
                    map,
                    "get(Ljava/lang/Object;)Ljava/lang/Object;",
                    &[key.into()],
                )?
                .object();
                assert_eq!(emu.string(value)?, expected);
            }
            let set = construct(&mut emu, "java/util/HashSet", "()V", &[])?;
            for (key, added) in [(first, true), (equal, false), (different, true)] {
                assert_eq!(
                    call(&mut emu, set, "add(Ljava/lang/Object;)Z", &[key.into()])?,
                    JniValue::Boolean(added)
                );
            }
            assert_eq!(call(&mut emu, set, "size()I", &[])?, JniValue::Int(2));
            assert_eq!(
                call(
                    &mut emu,
                    set,
                    "contains(Ljava/lang/Object;)Z",
                    &[equal.into()]
                )?,
                JniValue::Boolean(true)
            );
            let list = emu.new_java_list("java/util/ArrayList", &[first])?;
            assert_eq!(
                call(
                    &mut emu,
                    list,
                    "indexOf(Ljava/lang/Object;)I",
                    &[equal.into()]
                )?,
                JniValue::Int(0)
            );
            assert_eq!(
                call(
                    &mut emu,
                    list,
                    "contains(Ljava/lang/Object;)Z",
                    &[different.into()]
                )?,
                JniValue::Boolean(false)
            );
            let saved = emu.checkpoint()?;
            let mut fresh = Emulator::builder().build()?;
            fresh.rewind(&saved)?;
            let recreated = reflected(&mut fresh, kind, "a/B", name, signature, false);
            let value = call(
                &mut fresh,
                map,
                "get(Ljava/lang/Object;)Ljava/lang/Object;",
                &[recreated.into()],
            )?
            .object();
            assert_eq!(fresh.string(value)?, "one");
            assert_eq!(
                call(
                    &mut fresh,
                    set,
                    "add(Ljava/lang/Object;)Z",
                    &[recreated.into()]
                )?,
                JniValue::Boolean(false)
            );
        }
    }
    Ok(())
}
