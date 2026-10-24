use super::*;

fn add(emu: &mut Emulator, list: Handle, item: Handle) -> Result<()> {
    call(emu, list, "add(Ljava/lang/Object;)Z", &[item.into()])?;
    Ok(())
}

fn cycle(emu: &mut Emulator, kind: &str) -> Result<Vec<Handle>> {
    let list = emu.new_java_list("java/util/ArrayList", &[])?;
    let mut handles = vec![list];
    match kind {
        "list" => add(emu, list, list)?,
        "map" | "mutual" | "view" | "entry" => {
            let map = emu.new_java_map("java/util/HashMap", &[])?;
            handles.push(map);
            let value = if kind == "map" { map } else { list };
            call(
                emu,
                map,
                "put(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
                &[Handle(0).into(), value.into()],
            )?;
            match kind {
                "mutual" => add(emu, list, map)?,
                "view" => {
                    let view = call(emu, map, "values()Ljava/util/Collection;", &[])?.object();
                    add(emu, list, view)?;
                    handles.push(view);
                }
                "entry" => {
                    let view = call(emu, map, "entrySet()Ljava/util/Set;", &[])?.object();
                    let iterator = call(emu, view, "iterator()Ljava/util/Iterator;", &[])?.object();
                    let entry = call(emu, iterator, "next()Ljava/lang/Object;", &[])?.object();
                    add(emu, list, entry)?;
                    handles.extend([view, iterator, entry]);
                }
                _ => {}
            }
        }
        "set" => {
            let set = construct(emu, "java/util/HashSet", "()V", &[])?;
            add(emu, set, set)?;
            handles.push(set);
        }
        "iterator" => {
            add(emu, list, list)?;
            let iterator = call(emu, list, "iterator()Ljava/util/Iterator;", &[])?.object();
            call(emu, list, "clear()V", &[])?;
            add(emu, list, iterator)?;
            handles.push(iterator);
        }
        "array" => {
            let array = emu.new_object_array("java/lang/Object", &[Handle(0)])?;
            jni_call(emu, 174, &[array.0, 0, array.0])?;
            handles.push(array);
        }
        _ => unreachable!(),
    }
    Ok(handles)
}

#[test]
fn orphaned_container_cycles_are_freed_on_drop_and_rewind_including_copies() -> Result<()> {
    for arch in ARCHES {
        for kind in [
            "list", "map", "mutual", "set", "view", "entry", "iterator", "array",
        ] {
            for copied in [false, true] {
                for rewind in [false, true] {
                    let mut emu = emulator(arch)?;
                    let empty = emu.checkpoint()?;
                    let handles = cycle(&mut emu, kind)?;
                    if copied {
                        let saved = emu.checkpoint()?;
                        emu.rewind(&saved)?;
                    }
                    let objects = handles
                        .iter()
                        .map(|&handle| emu.jni_object(handle).map(|object| Rc::downgrade(&object)))
                        .collect::<Result<Vec<_>>>()?;
                    for handle in handles {
                        emu.release(handle);
                    }
                    assert!(
                        objects.iter().any(|object| object.strong_count() > 0),
                        "{kind} must form an orphaned cycle"
                    );
                    if rewind {
                        emu.rewind(&empty)?;
                    } else {
                        drop(emu);
                    }
                    assert!(
                        objects.iter().all(|object| object.strong_count() == 0),
                        "leaked {kind}, copied={copied}, rewind={rewind}"
                    );
                }
            }
        }
    }
    Ok(())
}
