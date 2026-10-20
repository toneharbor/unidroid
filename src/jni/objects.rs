use super::{Handle, MEMBER_ID_BASE, Object, PrimitiveType, Reference};
use crate::{Emulator, Result};
use std::{any::Any, cell::RefCell, rc::Rc};

impl Emulator {
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
    fn insert_api_object(&mut self, object: Object) -> Handle {
        self.insert_ref(Rc::new(RefCell::new(object)), 0)
    }
    pub(super) fn object(&self, handle: Handle) -> Result<Rc<RefCell<Object>>> {
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
        match &*self.object(handle)?.borrow() {
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
        match &*self.object(handle)?.borrow() {
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
            .map(|handle| (handle.0 != 0).then(|| self.object(*handle)).transpose())
            .collect::<Result<_>>()?;
        Ok(self.insert_api_object(Object::ObjectArray(values, class.into())))
    }
    pub fn object_array(&mut self, handle: Handle) -> Result<Vec<Handle>> {
        let values = match &*self.object(handle)?.borrow() {
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
        let object = self.object(handle)?;
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
        match &*self.object(handle).ok()?.borrow() {
            Object::User(_, v) => v.clone().downcast().ok(),
            _ => None,
        }
    }
    pub(super) fn class_name(&self, handle: Handle) -> Result<String> {
        Ok(match &*self.object(handle)?.borrow() {
            Object::Class(name) => name.clone(),
            object => object.java_type(),
        })
    }
    pub(super) fn alias(&mut self, handle: Handle, scope: usize) -> Result<Handle> {
        if handle.0 == 0 {
            return Ok(handle);
        }
        let object = self.object(handle)?;
        Ok(self.insert_ref(object, scope))
    }
}
