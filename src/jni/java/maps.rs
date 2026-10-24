use super::*;
use std::cmp::Ordering;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::jni) enum MapKind {
    Hash,
    Linked,
    Tree,
}
impl MapKind {
    pub(super) fn parse(class: &str) -> Option<Self> {
        Some(match class {
            "java/util/HashMap" => Self::Hash,
            "java/util/LinkedHashMap" => Self::Linked,
            "java/util/TreeMap" => Self::Tree,
            _ => return None,
        })
    }
    pub(in crate::jni) fn class(self) -> &'static str {
        match self {
            Self::Hash => "java/util/HashMap",
            Self::Linked => "java/util/LinkedHashMap",
            Self::Tree => "java/util/TreeMap",
        }
    }
    fn entry_class(self) -> &'static str {
        match self {
            Self::Hash => "java/util/HashMap$Node",
            Self::Linked => "java/util/LinkedHashMap$Entry",
            Self::Tree => "java/util/TreeMap$Entry",
        }
    }
}
#[derive(Clone)]
pub(in crate::jni) struct Map {
    pub(super) kind: MapKind,
    pub(super) entries: Vec<(ObjectRef, ObjectRef, i32)>,
    capacity: u32,
}
impl Map {
    pub(in crate::jni) fn class(&self) -> &'static str {
        self.kind.class()
    }
    pub(in crate::jni) fn references(&mut self, mut visit: impl FnMut(&mut ObjectRef)) {
        for (key, value, _) in &mut self.entries {
            visit(key);
            visit(value);
        }
    }
    pub(super) fn new(kind: MapKind, capacity: i32) -> Self {
        Self {
            kind,
            entries: Vec::new(),
            capacity: (capacity as u32).max(1).next_power_of_two().min(1 << 30),
        }
    }
    pub(super) fn position(&self, key: &ObjectRef) -> Option<Option<usize>> {
        if self.kind == MapKind::Tree {
            compare(key, key)?;
            for (i, (other, _, _)) in self.entries.iter().enumerate() {
                if compare(key, other)? == Ordering::Equal {
                    return Some(Some(i));
                }
            }
            Some(None)
        } else {
            Some(
                self.entries
                    .iter()
                    .position(|(k, _, _)| object::equal(k, key)),
            )
        }
    }
    pub(super) fn put(&mut self, key: ObjectRef, value: ObjectRef, hash: i32) -> Option<ObjectRef> {
        let index = self.position(&key)?;
        Some(self.put_at(key, value, hash, index))
    }
    pub(super) fn put_at(
        &mut self,
        key: ObjectRef,
        value: ObjectRef,
        hash: i32,
        index: Option<usize>,
    ) -> ObjectRef {
        if let Some(index) = index {
            return std::mem::replace(&mut self.entries[index].1, value);
        }
        self.entries.push((key, value, hash));
        if self.kind == MapKind::Tree {
            self.entries.sort_by(|a, b| compare(&a.0, &b.0).unwrap());
        } else {
            self.resize();
        }
        None
    }
    pub(super) fn resize(&mut self) {
        if self.entries.len() > (self.capacity as usize * 3 / 4) && self.capacity < (1 << 30) {
            self.capacity *= 2;
        }
    }
    pub(super) fn ordered(&self) -> Vec<(ObjectRef, ObjectRef)> {
        let mut entries: Vec<_> = self.entries.iter().collect();
        if self.kind == MapKind::Hash {
            // Keep bucket insertion order across splits; Java's treeified buckets are deliberately not modeled.
            entries.sort_by_key(|(_, _, h)| {
                let h = *h as u32;
                (h ^ (h >> 16)) & (self.capacity - 1)
            });
        }
        entries
            .into_iter()
            .map(|(k, v, _)| (k.clone(), v.clone()))
            .collect()
    }
}
fn compare(a: &ObjectRef, b: &ObjectRef) -> Option<Ordering> {
    let (a, b) = (a.as_ref()?.borrow(), b.as_ref()?.borrow());
    match (&*a, &*b) {
        (Object::String(a), Object::String(b)) => Some(a.cmp(b)),
        (Object::Boxed(a), Object::Boxed(b)) if boxing::class(*a) == boxing::class(*b) => {
            Some(match (*a, *b) {
                (JniValue::Float(a), JniValue::Float(b)) => java_float_compare(a as f64, b as f64),
                (JniValue::Double(a), JniValue::Double(b)) => java_float_compare(a, b),
                (JniValue::Char(a), JniValue::Char(b)) => a.cmp(&b),
                (JniValue::Boolean(a), JniValue::Boolean(b)) => a.cmp(&b),
                _ => boxing::integer(*a)?.cmp(&boxing::integer(*b)?),
            })
        }
        _ => None,
    }
}
fn java_float_compare(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        _ => a.total_cmp(&b),
    }
}
pub(super) fn construct(emu: &mut Emulator, call: &JniCall<'_>) -> Result<Option<JniValue>> {
    if call.name != "<init>" {
        return Ok(None);
    }
    let set = call.class == "java/util/HashSet";
    let Some(kind) = (if set {
        Some(MapKind::Hash)
    } else {
        MapKind::parse(call.class)
    }) else {
        return Ok(None);
    };
    let capacity = match call.signature {
        "()V" => 16,
        "(I)V" if !set && kind != MapKind::Tree => call.args[0].int(),
        _ => return Ok(None),
    };
    if capacity < 0 {
        return Err(exception(
            "java/lang/IllegalArgumentException",
            Some(format!("Illegal initial capacity: {capacity}")),
        ));
    }
    let map = Map::new(kind, capacity);
    Ok(Some(
        emu.insert_object(if set {
            Object::Set(map)
        } else {
            Object::Map(map)
        })
        .into(),
    ))
}
pub(super) fn call(
    emu: &mut Emulator,
    call: &JniCall<'_>,
    receiver: &Rc<RefCell<Object>>,
) -> Result<Option<JniValue>> {
    let a = call.args;
    match (call.name, call.signature) {
        ("put", "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;") => {
            let key = reference(emu, a[0].object())?;
            let value = reference(emu, a[1].object())?;
            let hash = object::hash(emu, &key);
            let index = match &*receiver.borrow() {
                Object::Map(map) => map.position(&key),
                _ => return Ok(None),
            };
            let Some(index) = index else {
                return Ok(None);
            };
            let Object::Map(map) = &mut *receiver.borrow_mut() else {
                unreachable!()
            };
            Ok(Some(return_ref(emu, map.put_at(key, value, hash, index))))
        }
        ("get", "(Ljava/lang/Object;)Ljava/lang/Object;")
        | ("getOrDefault", "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;")
        | ("remove", "(Ljava/lang/Object;)Ljava/lang/Object;")
        | ("containsKey", "(Ljava/lang/Object;)Z") => {
            let key = reference(emu, a[0].object())?;
            let index = match &*receiver.borrow() {
                Object::Map(map) => map.position(&key),
                _ => return Ok(None),
            };
            let Some(index) = index else {
                return Ok(None);
            };
            if call.name == "containsKey" {
                return Ok(Some(JniValue::Boolean(index.is_some())));
            }
            let Object::Map(map) = &mut *receiver.borrow_mut() else {
                unreachable!()
            };
            let value = match index {
                Some(i) if call.name == "remove" => map.entries.remove(i).1,
                Some(i) => map.entries[i].1.clone(),
                None if call.name == "getOrDefault" => reference(emu, a[1].object())?,
                None => None,
            };
            Ok(Some(return_ref(emu, value)))
        }
        ("size", "()I")
        | ("isEmpty", "()Z")
        | ("clear", "()V")
        | ("keySet", "()Ljava/util/Set;")
        | ("values", "()Ljava/util/Collection;")
        | ("entrySet", "()Ljava/util/Set;") => {
            let Object::Map(map) = &mut *receiver.borrow_mut() else {
                return Ok(None);
            };
            let value = match call.name {
                "size" => JniValue::Int(map.entries.len() as i32),
                "isEmpty" => JniValue::Boolean(map.entries.is_empty()),
                "clear" => {
                    map.entries.clear();
                    JniValue::Void
                }
                _ => {
                    let items = map
                        .ordered()
                        .into_iter()
                        .map(|(key, value)| match call.name {
                            "keySet" => key,
                            "values" => value,
                            _ => Some(emu.allocate_object(Object::Entry(
                                map.kind.entry_class().into(),
                                key,
                                value,
                            ))),
                        })
                        .collect();
                    let suffix = match (map.kind, call.name) {
                        (MapKind::Linked, "keySet") => "$LinkedKeySet",
                        (MapKind::Linked, "values") => "$LinkedValues",
                        (MapKind::Linked, "entrySet") => "$LinkedEntrySet",
                        (_, "keySet") => "$KeySet",
                        (_, "values") => "$Values",
                        _ => "$EntrySet",
                    };
                    emu.insert_object(Object::View(format!("{}{suffix}", map.kind.class()), items))
                        .into()
                }
            };
            Ok(Some(value))
        }
        _ => Ok(None),
    }
}
