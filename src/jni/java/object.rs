use super::*;
use crate::jni::android::Android;

pub(super) fn hash(emu: &mut Emulator, object: &ObjectRef) -> i32 {
    let Some(object) = object else {
        return 0;
    };
    match &*object.borrow() {
        Object::String(value) => hash_string(value),
        Object::Boxed(value) => boxing::hash(*value),
        Object::Android(Android::Signature(bytes)) => bytes.iter().fold(1i32, |hash, &byte| {
            hash.wrapping_mul(31).wrapping_add(byte as i8 as i32)
        }),
        _ => emu.identity_hash(object),
    }
}
pub(super) fn equal(a: &ObjectRef, b: &ObjectRef) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            Rc::ptr_eq(a, b)
                || match (&*a.borrow(), &*b.borrow()) {
                    (Object::String(a), Object::String(b)) => a == b,
                    (Object::Boxed(a), Object::Boxed(b)) => boxing::equal(*a, *b),
                    (
                        Object::Android(Android::Signature(a)),
                        Object::Android(Android::Signature(b)),
                    ) => a == b,
                    _ => false,
                }
        }
        _ => false,
    }
}
pub(super) fn text(object: &ObjectRef) -> Option<Vec<u16>> {
    text_inner(object, &mut Vec::new())
}
fn text_inner(object: &ObjectRef, seen: &mut Vec<*const RefCell<Object>>) -> Option<Vec<u16>> {
    let Some(object) = object else {
        return Some("null".encode_utf16().collect());
    };
    let address = Rc::as_ptr(object);
    if seen.contains(&address) {
        return None;
    }
    seen.push(address);
    let result = match &*object.borrow() {
        Object::String(value) | Object::StringBuilder(value) => Some(value.clone()),
        Object::Boxed(value) => Some(boxing::text(*value)),
        Object::List(_, items) => {
            let mut values = Vec::new();
            for item in items {
                values.push(element_text(item, object, seen, "(this Collection)")?);
            }
            Some(join(&values, "[", ", ", "]"))
        }
        Object::Map(map) => {
            let mut values = Vec::new();
            for (key, value) in map.ordered() {
                let key = element_text(&key, object, seen, "(this Map)")?;
                let value = element_text(&value, object, seen, "(this Map)")?;
                values.push(join(&[key, value], "", "=", ""));
            }
            Some(join(&values, "{", ", ", "}"))
        }
        _ => None,
    };
    seen.pop();
    result
}
fn element_text(
    item: &ObjectRef,
    owner: &Rc<RefCell<Object>>,
    seen: &mut Vec<*const RefCell<Object>>,
    recursive: &str,
) -> Option<Vec<u16>> {
    if item.as_ref().is_some_and(|item| Rc::ptr_eq(item, owner)) {
        Some(recursive.encode_utf16().collect())
    } else {
        text_inner(item, seen)
    }
}
fn join(items: &[Vec<u16>], start: &str, separator: &str, end: &str) -> Vec<u16> {
    let mut value: Vec<_> = start.encode_utf16().collect();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            value.extend(separator.encode_utf16());
        }
        value.extend(item);
    }
    value.extend(end.encode_utf16());
    value
}
pub(super) fn call(
    emu: &mut Emulator,
    call: &JniCall<'_>,
    receiver: &Rc<RefCell<Object>>,
) -> Result<Option<JniValue>> {
    let answer = match (call.name, call.signature) {
        ("getClass", "()Ljava/lang/Class;") => {
            let class = emu.find_class(&receiver.borrow().java_type(), emu.jni.scope);
            emu.alias(class, emu.jni.scope)?.into()
        }
        ("hashCode", "()I") => JniValue::Int(hash(emu, &Some(receiver.clone()))),
        ("equals", "(Ljava/lang/Object;)Z") => JniValue::Boolean(equal(
            &Some(receiver.clone()),
            &reference(emu, call.args[0].object())?,
        )),
        ("toString", "()Ljava/lang/String;") => {
            if matches!(&*receiver.borrow(), Object::Android(_)) {
                let hash = hash(emu, &Some(receiver.clone()));
                let value = format!(
                    "{}@{:x}",
                    receiver.borrow().java_type().replace('/', "."),
                    hash as u32
                );
                return Ok(Some(string(emu, value.encode_utf16().collect::<Vec<_>>())));
            }
            let Some(value) = text(&Some(receiver.clone())) else {
                return Ok(None);
            };
            if matches!(&*receiver.borrow(), Object::String(_)) {
                return_ref(emu, Some(receiver.clone()))
            } else {
                string(emu, value)
            }
        }
        ("getName", "()Ljava/lang/String;") => {
            let Object::Class(name) = &*receiver.borrow() else {
                return Ok(None);
            };
            string(
                emu,
                name.replace('/', ".").encode_utf16().collect::<Vec<_>>(),
            )
        }
        _ => return Ok(None),
    };
    Ok(Some(answer))
}
