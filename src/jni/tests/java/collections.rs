use super::*;

#[test]
fn collections_handle_nulls_mutation_and_snapshot_iteration() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let a = emu.new_string("a");
        let b = emu.new_string("b");
        let map = emu.new_java_map("java/util/HashMap", &[(a, Handle(0))])?;
        assert_eq!(
            call(
                &mut emu,
                map,
                "containsKey(Ljava/lang/Object;)Z",
                &[a.into()]
            )?,
            JniValue::Boolean(true)
        );
        assert_eq!(
            call(
                &mut emu,
                map,
                "getOrDefault(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
                &[a.into(), b.into()]
            )?,
            Handle(0).into()
        );
        let value = call(
            &mut emu,
            map,
            "getOrDefault(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
            &[b.into(), b.into()],
        )?
        .object();
        assert_eq!(emu.string(value)?, "b");
        call(
            &mut emu,
            map,
            "put(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
            &[Handle(0).into(), b.into()],
        )?;
        let value = call(
            &mut emu,
            map,
            "get(Ljava/lang/Object;)Ljava/lang/Object;",
            &[Handle(0).into()],
        )?
        .object();
        assert_eq!(emu.string(value)?, "b");
        for class in ["java/util/ArrayList", "java/util/LinkedList"] {
            let list = emu.new_java_list(class, &[a, Handle(0), b])?;
            let iter = call(&mut emu, list, "iterator()Ljava/util/Iterator;", &[])?.object();
            assert_eq!(
                call(&mut emu, list, "indexOf(Ljava/lang/Object;)I", &[b.into()])?,
                JniValue::Int(2)
            );
            assert_eq!(
                call(
                    &mut emu,
                    list,
                    "contains(Ljava/lang/Object;)Z",
                    &[Handle(0).into()]
                )?,
                JniValue::Boolean(true)
            );
            assert_eq!(
                call(
                    &mut emu,
                    list,
                    "set(ILjava/lang/Object;)Ljava/lang/Object;",
                    &[1.into(), a.into()]
                )?,
                Handle(0).into()
            );
            let removed =
                call(&mut emu, list, "remove(I)Ljava/lang/Object;", &[0.into()])?.object();
            assert_eq!(emu.string(removed)?, "a");
            assert_eq!(call(&mut emu, list, "size()I", &[])?, JniValue::Int(2));
            call(&mut emu, list, "clear()V", &[])?;
            assert_eq!(
                call(&mut emu, list, "isEmpty()Z", &[])?,
                JniValue::Boolean(true)
            );
            let value = call(&mut emu, iter, "next()Ljava/lang/Object;", &[])?.object();
            assert_eq!(emu.string(value)?, "a");
            assert_eq!(
                call(&mut emu, iter, "next()Ljava/lang/Object;", &[])?,
                Handle(0).into()
            );
            let value = call(&mut emu, iter, "next()Ljava/lang/Object;", &[])?.object();
            assert_eq!(emu.string(value)?, "b");
            assert_eq!(
                call(&mut emu, iter, "hasNext()Z", &[])?,
                JniValue::Boolean(false)
            );
        }
        let identity = call(&mut emu, map, "hashCode()I", &[])?;
        call(
            &mut emu,
            map,
            "put(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
            &[map.into(), map.into()],
        )?;
        let saved = emu.checkpoint()?;
        call(&mut emu, map, "clear()V", &[])?;
        assert_eq!(
            call(&mut emu, map, "isEmpty()Z", &[])?,
            JniValue::Boolean(true)
        );
        emu.rewind(&saved)?;
        assert_eq!(call(&mut emu, map, "hashCode()I", &[])?, identity);
        let value = call(
            &mut emu,
            map,
            "get(Ljava/lang/Object;)Ljava/lang/Object;",
            &[map.into()],
        )?
        .object();
        assert_eq!(jni_call(&mut emu, 24, &[value.0, map.0])?, 1);
    }
    Ok(())
}

#[test]
fn maps_lists_views_and_iterators_are_guest_state() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let map = construct(&mut emu, "java/util/HashMap", "()V", &[])?;
        let key = emu.new_string("key");
        let old = emu.new_string("old");
        assert_eq!(
            call(
                &mut emu,
                map,
                "put(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
                &[key.into(), old.into()]
            )?,
            Handle(0).into()
        );
        let view = call(&mut emu, map, "values()Ljava/util/Collection;", &[])?.object();
        let iterator = call(&mut emu, view, "iterator()Ljava/util/Iterator;", &[])?.object();
        let saved = emu.checkpoint()?;
        let new = emu.new_string("new");
        call(
            &mut emu,
            map,
            "put(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
            &[key.into(), new.into()],
        )?;
        assert_eq!(text(&mut emu, map)?, "{key=new}");
        let value = call(&mut emu, iterator, "next()Ljava/lang/Object;", &[])?.object();
        assert_eq!(emu.string(value)?, "old");
        emu.rewind(&saved)?;
        assert_eq!(text(&mut emu, map)?, "{key=old}");
        assert_eq!(
            call(&mut emu, iterator, "hasNext()Z", &[])?,
            JniValue::Boolean(true)
        );
        for class in ["java/util/ArrayList", "java/util/LinkedList"] {
            let list = construct(&mut emu, class, "()V", &[])?;
            assert_eq!(
                call(&mut emu, list, "add(Ljava/lang/Object;)Z", &[key.into()])?,
                JniValue::Boolean(true)
            );
            call(
                &mut emu,
                list,
                "add(ILjava/lang/Object;)V",
                &[0.into(), old.into()],
            )?;
            assert_eq!(text(&mut emu, list)?, "[old, key]");
            let array = call(&mut emu, list, "toArray()[Ljava/lang/Object;", &[])?.object();
            let items = emu.object_array(array)?;
            assert_eq!(emu.string(items[0])?, "old");
            assert_eq!(emu.string(items[1])?, "key");
        }
    }
    Ok(())
}
