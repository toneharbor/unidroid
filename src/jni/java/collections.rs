use super::*;

pub(super) fn construct(emu: &mut Emulator, call: &JniCall<'_>) -> Result<Option<JniValue>> {
    if call.name != "<init>"
        || !matches!(call.class, "java/util/ArrayList" | "java/util/LinkedList")
    {
        return Ok(None);
    }
    match call.signature {
        "()V" => {}
        "(I)V" if call.class == "java/util/ArrayList" => {
            if call.args[0].int() < 0 {
                return Err(exception(
                    "java/lang/IllegalArgumentException",
                    Some(format!("Illegal Capacity: {}", call.args[0].int())),
                ));
            }
        }
        _ => return Ok(None),
    }
    Ok(Some(
        emu.insert_object(Object::List(call.class.into(), Vec::new()))
            .into(),
    ))
}
fn index(index: i32, length: usize, insert: bool) -> Result<usize> {
    if index < 0 || index as usize > length || (!insert && index as usize == length) {
        Err(exception(
            "java/lang/IndexOutOfBoundsException",
            Some(format!("Index: {index}, Size: {length}")),
        ))
    } else {
        Ok(index as usize)
    }
}
pub(super) fn call(
    emu: &mut Emulator,
    call: &JniCall<'_>,
    receiver: &Rc<RefCell<Object>>,
) -> Result<Option<JniValue>> {
    let a = call.args;
    if matches!(&*receiver.borrow(), Object::List(..)) {
        match (call.name, call.signature) {
            ("add", "(Ljava/lang/Object;)Z")
            | ("add", "(ILjava/lang/Object;)V")
            | ("set", "(ILjava/lang/Object;)Ljava/lang/Object;") => {
                let value = reference(emu, a.last().unwrap().object())?;
                let Object::List(_, items) = &mut *receiver.borrow_mut() else {
                    unreachable!()
                };
                return Ok(Some(if a.len() == 1 {
                    items.push(value);
                    JniValue::Boolean(true)
                } else {
                    let i = index(a[0].int(), items.len(), call.name == "add")?;
                    if call.name == "add" {
                        items.insert(i, value);
                        JniValue::Void
                    } else {
                        return_ref(emu, std::mem::replace(&mut items[i], value))
                    }
                }));
            }
            ("get", "(I)Ljava/lang/Object;") | ("remove", "(I)Ljava/lang/Object;") => {
                let Object::List(_, items) = &mut *receiver.borrow_mut() else {
                    unreachable!()
                };
                let i = index(a[0].int(), items.len(), false)?;
                let value = if call.name == "remove" {
                    items.remove(i)
                } else {
                    items[i].clone()
                };
                return Ok(Some(return_ref(emu, value)));
            }
            ("clear", "()V") => {
                let Object::List(_, items) = &mut *receiver.borrow_mut() else {
                    unreachable!()
                };
                items.clear();
                return Ok(Some(JniValue::Void));
            }
            _ => {}
        }
    }
    if matches!(&*receiver.borrow(), Object::Set(_))
        && let ("add", "(Ljava/lang/Object;)Z") = (call.name, call.signature)
    {
        let key = reference(emu, a[0].object())?;
        let hash = object::hash(emu, &key);
        let index = match &*receiver.borrow() {
            Object::Set(map) => map.position(&key).unwrap(),
            _ => unreachable!(),
        };
        let Object::Set(map) = &mut *receiver.borrow_mut() else {
            unreachable!()
        };
        map.put_at(key, None, hash, index);
        return Ok(Some(JniValue::Boolean(index.is_none())));
    }
    if matches!(&*receiver.borrow(), Object::Iterator(..)) {
        let Object::Iterator(_, items, position) = &mut *receiver.borrow_mut() else {
            unreachable!()
        };
        return Ok(Some(match (call.name, call.signature) {
            ("hasNext", "()Z") => JniValue::Boolean(*position < items.len()),
            ("next", "()Ljava/lang/Object;") => {
                let value = items
                    .get(*position)
                    .ok_or_else(|| exception("java/util/NoSuchElementException", None))?
                    .clone();
                *position += 1;
                return_ref(emu, value)
            }
            _ => return Ok(None),
        }));
    }
    if let Object::Entry(_, key, value) = &*receiver.borrow() {
        return Ok(match (call.name, call.signature) {
            ("getKey", "()Ljava/lang/Object;") => Some(return_ref(emu, key.clone())),
            ("getValue", "()Ljava/lang/Object;") => Some(return_ref(emu, value.clone())),
            _ => None,
        });
    }
    let (class, items, list) = match &*receiver.borrow() {
        Object::List(class, items) => (class.clone(), items.clone(), true),
        Object::View(class, items) => (class.clone(), items.clone(), false),
        Object::Set(map) => (
            "java/util/HashMap$KeySet".into(),
            map.ordered().into_iter().map(|(key, _)| key).collect(),
            false,
        ),
        _ => return Ok(None),
    };
    let result = match (call.name, call.signature) {
        ("size", "()I") => JniValue::Int(items.len() as i32),
        ("isEmpty", "()Z") => JniValue::Boolean(items.is_empty()),
        ("contains", "(Ljava/lang/Object;)Z") | ("indexOf", "(Ljava/lang/Object;)I")
            if call.name == "contains" || list =>
        {
            let value = reference(emu, a[0].object())?;
            let found = items.iter().position(|v| object::equal(v, &value));
            if call.name == "contains" {
                JniValue::Boolean(found.is_some())
            } else {
                JniValue::Int(found.map_or(-1, |i| i as i32))
            }
        }
        ("toArray", "()[Ljava/lang/Object;") => emu
            .insert_object(Object::ObjectArray(items, "java/lang/Object".into()))
            .into(),
        ("iterator", "()Ljava/util/Iterator;") => {
            let name = if class == "java/util/LinkedList" {
                "java/util/LinkedList$ListItr".into()
            } else if list {
                format!("{class}$Itr")
            } else {
                let (base, kind) = class.split_once('$').unwrap();
                let suffix = if kind.ends_with("KeySet") {
                    "KeyIterator"
                } else if kind.ends_with("Values") {
                    "ValueIterator"
                } else {
                    "EntryIterator"
                };
                format!(
                    "{base}${}{}",
                    if base == "java/util/LinkedHashMap" {
                        "Linked"
                    } else {
                        ""
                    },
                    suffix
                )
            };
            emu.insert_object(Object::Iterator(name, items, 0)).into()
        }
        _ => return Ok(None),
    };
    Ok(Some(result))
}
