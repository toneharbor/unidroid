use super::{Handle, JniState, MEMBER_ID_BASE, Object, PrimitiveType, Reference};
use crate::{Emulator, Result};
use std::{
    any::Any,
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

impl Clone for JniState {
    fn clone(&self) -> Self {
        fn copy(
            object: &Rc<RefCell<Object>>,
            seen: &mut HashMap<*const RefCell<Object>, Rc<RefCell<Object>>>,
            pending: &mut Vec<Rc<RefCell<Object>>>,
        ) -> Rc<RefCell<Object>> {
            let address = Rc::as_ptr(object);
            if let Some(copied) = seen.get(&address) {
                return copied.clone();
            }
            // Register a placeholder before queuing children so cycles keep their identity.
            let copied = Rc::new(RefCell::new(Object::ObjectArray(Vec::new(), String::new())));
            seen.insert(address, copied.clone());
            pending.push(object.clone());
            copied
        }
        let mut seen = HashMap::new();
        let mut pending = Vec::new();
        let mut copied = Self {
            objects: self
                .objects
                .iter()
                .map(|(&handle, reference)| {
                    (
                        handle,
                        Reference {
                            object: copy(&reference.object, &mut seen, &mut pending),
                            scope: reference.scope,
                        },
                    )
                })
                .collect(),
            classes: self
                .classes
                .iter()
                .map(|(name, (handle, object))| {
                    (
                        name.clone(),
                        (*handle, copy(object, &mut seen, &mut pending)),
                    )
                })
                .collect(),
            superclasses: self.superclasses.clone(),
            members: self.members.clone(),
            member_ids: self.member_ids.clone(),
            natives: self.natives.clone(),
            next_handle: self.next_handle,
            identities: HashMap::new(),
            next_identity: self.next_identity,
            scope: self.scope,
            env: self.env,
            vm: self.vm,
            weak: self.weak.clone(),
            pending: self.pending,
            history: Default::default(),
        };
        while let Some(object) = pending.pop() {
            let mut value = object.borrow().clone();
            value.references(|item| {
                if let Some(item) = item {
                    *item = copy(item, &mut seen, &mut pending);
                }
            });
            *seen[&Rc::as_ptr(&object)].borrow_mut() = value;
        }
        copied.identities = seen
            .iter()
            .filter_map(|(old, new)| {
                self.identities
                    .get(old)
                    .map(|(_, id)| (Rc::as_ptr(new), (Rc::downgrade(new), *id)))
            })
            .collect();
        copied
    }
}
impl JniState {
    pub(crate) fn rewind(&mut self, saved: &Self) {
        *self = saved.clone();
    }
}
impl Drop for JniState {
    fn drop(&mut self) {
        // Each state owns its graph. Break object cycles when replacing or dropping it.
        let mut pending: Vec<_> = self
            .objects
            .values()
            .map(|r| r.object.clone())
            .chain(self.classes.values().map(|(_, object)| object.clone()))
            .collect();
        let mut seen = HashSet::new();
        while let Some(object) = pending.pop() {
            if seen.insert(Rc::as_ptr(&object)) {
                object
                    .borrow_mut()
                    .references(|item| pending.extend(item.take()));
            }
        }
    }
}

impl Emulator {
    pub(super) fn identity_hash(&mut self, object: &Rc<RefCell<Object>>) -> i32 {
        if let Some((_, id)) = self.jni.identities.get(&Rc::as_ptr(object)) {
            return *id;
        }
        // Weak references prevent address reuse; discard dead records before assigning another hash.
        self.jni
            .identities
            .retain(|_, (object, _)| object.strong_count() != 0);
        self.jni.next_identity = self.jni.next_identity.wrapping_add(1);
        let id = self.jni.next_identity;
        self.jni
            .identities
            .insert(Rc::as_ptr(object), (Rc::downgrade(object), id));
        id
    }
    pub(super) fn insert_ref(&mut self, object: Rc<RefCell<Object>>, scope: usize) -> Handle {
        let handle = loop {
            self.jni.next_handle = self.jni.next_handle % (MEMBER_ID_BASE - 1) + 1;
            let handle = Handle(self.jni.next_handle);
            if !self.jni.objects.contains_key(&handle)
                && !self
                    .jni
                    .classes
                    .values()
                    .any(|(remembered, _)| *remembered == handle)
            {
                break handle;
            }
        };
        self.jni.objects.insert(handle, Reference { object, scope });
        handle
    }
    pub(super) fn insert_object(&mut self, object: Object) -> Handle {
        let scope = self.jni.scope;
        self.insert_ref(Rc::new(RefCell::new(object)), scope)
    }
    pub(super) fn insert_api_object(&mut self, object: Object) -> Handle {
        self.insert_ref(Rc::new(RefCell::new(object)), 0)
    }
    pub(super) fn jni_object(&self, handle: Handle) -> Result<Rc<RefCell<Object>>> {
        self.jni
            .objects
            .get(&handle)
            .map(|reference| reference.object.clone())
            .ok_or_else(|| {
                format!(
                    "invalid handle {}: no live object (never created, or already released)",
                    handle.0
                )
                .into()
            })
    }
    pub fn release(&mut self, handle: Handle) {
        self.jni.objects.remove(&handle);
    }
    pub fn class(&mut self, name: &str) -> Handle {
        self.find_class(name, 0)
    }
    pub(super) fn find_class(&mut self, name: &str, scope: usize) -> Handle {
        if let Some((handle, object)) = self.jni.classes.get(name).cloned() {
            let reference = self
                .jni
                .objects
                .entry(handle)
                .or_insert(Reference { object, scope });
            reference.scope = reference.scope.min(scope);
            return handle;
        }
        let object = Rc::new(RefCell::new(Object::Class(name.into())));
        let handle = self.insert_ref(object.clone(), scope);
        self.jni.classes.insert(name.into(), (handle, object));
        handle
    }
    /// Declare that a class directly extends a superclass.
    pub fn superclass(&mut self, class: impl Into<String>, superclass: impl Into<String>) {
        self.jni
            .superclasses
            .insert(class.into(), superclass.into());
    }
    pub(super) fn class_chain<'a>(&'a self, class: &'a str) -> Vec<&'a str> {
        let mut chain = Vec::new();
        let mut next = Some(class);
        while let Some(class) = next {
            if chain.contains(&class) {
                break;
            }
            chain.push(class);
            next = self.jni.superclasses.get(class).map(String::as_str);
        }
        chain
    }
    pub fn new_string(&mut self, value: &str) -> Handle {
        self.insert_api_object(Object::String(value.encode_utf16().collect()))
    }
    pub fn string(&self, handle: Handle) -> Result<String> {
        match &*self.jni_object(handle)?.borrow() {
            Object::String(s) => Ok(String::from_utf16_lossy(s)),
            object => Err(format!(
                "invalid handle {}: {}, not a string",
                handle.0,
                object.java_type()
            )
            .into()),
        }
    }
    pub fn new_byte_array(&mut self, bytes: &[u8]) -> Handle {
        self.new_primitive_array(PrimitiveType::Byte, bytes)
    }
    /// Primitive array bytes use Android's little-endian representation.
    pub fn new_primitive_array(&mut self, kind: PrimitiveType, bytes: &[u8]) -> Handle {
        self.insert_api_object(Object::Array(kind, bytes.to_vec()))
    }
    pub fn byte_array(&self, handle: Handle) -> Result<Vec<u8>> {
        self.primitive_array(handle)
    }
    pub fn primitive_array(&self, handle: Handle) -> Result<Vec<u8>> {
        match &*self.jni_object(handle)?.borrow() {
            Object::Array(_, bytes) => Ok(bytes.clone()),
            object => Err(format!(
                "invalid handle {}: {}, not a primitive array",
                handle.0,
                object.java_type()
            )
            .into()),
        }
    }
    pub fn new_object<T: Any>(&mut self, class: &str, value: T) -> Handle {
        self.insert_api_object(Object::User(class.into(), Rc::new(value)))
    }
    pub fn new_object_array(&mut self, class: &str, values: &[Handle]) -> Result<Handle> {
        let values = values
            .iter()
            .map(|handle| {
                (handle.0 != 0)
                    .then(|| self.jni_object(*handle))
                    .transpose()
            })
            .collect::<Result<_>>()?;
        Ok(self.insert_api_object(Object::ObjectArray(values, class.into())))
    }
    pub fn object_array(&mut self, handle: Handle) -> Result<Vec<Handle>> {
        let values = match &*self.jni_object(handle)?.borrow() {
            Object::ObjectArray(values, _) => values.clone(),
            object => {
                return Err(format!(
                    "invalid handle {}: {}, not an object array",
                    handle.0,
                    object.java_type()
                )
                .into());
            }
        };
        Ok(values
            .into_iter()
            .map(|object| object.map_or(Handle(0), |object| self.insert_ref(object, 0)))
            .collect())
    }
    pub(super) fn pending_exception(&mut self, class: &str, message: Option<String>) {
        if let Some(old) = self.jni.pending.take() {
            self.release(old);
        }
        let handle = self.insert_object(Object::Exception(class.into(), message));
        self.jni.objects.get_mut(&handle).unwrap().scope = 0;
        self.jni.pending = Some(handle);
    }
    pub(super) fn exception_result(&mut self) -> Result<()> {
        let Some(handle) = self.jni.pending.take() else {
            return Ok(());
        };
        let object = self.jni_object(handle)?;
        let (class, message) = match &*object.borrow() {
            Object::Exception(class, message) => (class.clone(), message.clone()),
            _ => ("java/lang/Throwable".into(), None),
        };
        self.release(handle);
        Err(match message {
            Some(message) => format!("java exception thrown by native code: {class}: {message}"),
            None => format!("java exception thrown by native code: {class}"),
        }
        .into())
    }

    pub fn user_value<T: Any>(&self, handle: Handle) -> Option<Rc<T>> {
        match &*self.jni_object(handle).ok()?.borrow() {
            Object::User(_, v) => v.clone().downcast().ok(),
            _ => None,
        }
    }
    pub(super) fn class_name(&self, handle: Handle) -> Result<String> {
        Ok(match &*self.jni_object(handle)?.borrow() {
            Object::Class(name) => name.clone(),
            object => object.java_type(),
        })
    }
    pub(super) fn alias(&mut self, handle: Handle, scope: usize) -> Result<Handle> {
        if handle.0 == 0 {
            return Ok(handle);
        }
        let object = self.jni_object(handle)?;
        Ok(self.insert_ref(object, scope))
    }
}
