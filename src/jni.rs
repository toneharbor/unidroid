use crate::{Emulator, HostFunction, Module, Result};
use std::{
    any::Any,
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

#[derive(Clone, Copy, Debug)]
pub enum PrimitiveType {
    Boolean,
    Byte,
    Char,
    Short,
    Int,
    Long,
    Float,
    Double,
}
impl PrimitiveType {
    fn size(self) -> usize {
        match self {
            Self::Boolean | Self::Byte => 1,
            Self::Char | Self::Short => 2,
            Self::Int | Self::Float => 4,
            Self::Long | Self::Double => 8,
        }
    }
    fn signature(self) -> &'static str {
        match self {
            Self::Boolean => "[Z",
            Self::Byte => "[B",
            Self::Char => "[C",
            Self::Short => "[S",
            Self::Int => "[I",
            Self::Long => "[J",
            Self::Float => "[F",
            Self::Double => "[D",
        }
    }
}
const MEMBER_ID_BASE: u64 = 0x8000_0000;
const ARRAY_TYPES: [PrimitiveType; 8] = [
    PrimitiveType::Boolean,
    PrimitiveType::Byte,
    PrimitiveType::Char,
    PrimitiveType::Short,
    PrimitiveType::Int,
    PrimitiveType::Long,
    PrimitiveType::Float,
    PrimitiveType::Double,
];
#[allow(clippy::enum_variant_names)]
enum Object {
    Class(String),
    String(Vec<u16>),
    Array(PrimitiveType, Vec<u8>),
    User(String, Rc<dyn Any>),
    ObjectArray(Vec<Option<Rc<RefCell<Object>>>>, String),
    DirectBuffer(u64, i64),
    Exception(String, Option<String>),
}
struct Reference {
    object: Rc<RefCell<Object>>,
    scope: usize,
}
#[derive(Clone)]
struct Member {
    class: String,
    name: String,
    signature: String,
}
#[derive(Default)]
pub(crate) struct JniState {
    objects: HashMap<Handle, Reference>,
    classes: HashMap<String, (Handle, Rc<RefCell<Object>>)>,
    members: Vec<Member>,
    member_ids: HashMap<(String, String, String, bool, bool), u64>,
    natives: HashMap<(String, String), u64>,
    next_handle: u64,
    scope: usize,
    env: u64,
    vm: u64,
    weak: HashSet<Handle>,
    pending: Option<Handle>,
}

impl Emulator {
    fn insert_ref(&mut self, object: Rc<RefCell<Object>>, scope: usize) -> Handle {
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
    fn insert_object(&mut self, object: Object) -> Handle {
        let scope = self.jni.scope;
        self.insert_ref(Rc::new(RefCell::new(object)), scope)
    }
    fn insert_api_object(&mut self, object: Object) -> Handle {
        self.insert_ref(Rc::new(RefCell::new(object)), 0)
    }
    fn object(&self, handle: Handle) -> Rc<RefCell<Object>> {
        self.jni.objects[&handle].object.clone()
    }
    pub fn release(&mut self, handle: Handle) {
        self.jni.objects.remove(&handle);
    }
    pub fn class(&mut self, name: &str) -> Handle {
        self.find_class(name, 0)
    }
    fn find_class(&mut self, name: &str, scope: usize) -> Handle {
        if let Some((handle, object)) = self.jni.classes.get(name).cloned() {
            let reference = self
                .jni
                .objects
                .entry(handle)
                .or_insert(Reference { object, scope });
            reference.scope = reference.scope.min(scope);
            return handle;
        }
        let handle = self.insert_object(Object::Class(name.into()));
        self.jni.objects.get_mut(&handle).unwrap().scope = scope;
        self.jni
            .classes
            .insert(name.into(), (handle, self.object(handle)));
        handle
    }
    pub fn new_string(&mut self, value: &str) -> Handle {
        self.insert_api_object(Object::String(value.encode_utf16().collect()))
    }
    pub fn string(&self, handle: Handle) -> String {
        match &*self.object(handle).borrow() {
            Object::String(s) => String::from_utf16_lossy(s),
            _ => panic!(),
        }
    }
    pub fn new_byte_array(&mut self, bytes: &[u8]) -> Handle {
        self.new_primitive_array(PrimitiveType::Byte, bytes)
    }
    /// Primitive array bytes use Android's little-endian representation.
    pub fn new_primitive_array(&mut self, kind: PrimitiveType, bytes: &[u8]) -> Handle {
        self.insert_api_object(Object::Array(kind, bytes.to_vec()))
    }
    pub fn byte_array(&self, handle: Handle) -> Vec<u8> {
        self.primitive_array(handle)
    }
    pub fn primitive_array(&self, handle: Handle) -> Vec<u8> {
        match &*self.object(handle).borrow() {
            Object::Array(_, bytes) => bytes.clone(),
            _ => panic!(),
        }
    }
    pub fn new_object<T: Any>(&mut self, class: &str, value: T) -> Handle {
        self.insert_api_object(Object::User(class.into(), Rc::new(value)))
    }
    pub fn new_object_array(&mut self, class: &str, values: &[Handle]) -> Handle {
        let values = values
            .iter()
            .map(|handle| (handle.0 != 0).then(|| self.object(*handle)))
            .collect();
        self.insert_api_object(Object::ObjectArray(values, class.into()))
    }
    pub fn object_array(&mut self, handle: Handle) -> Vec<Handle> {
        let values = match &*self.object(handle).borrow() {
            Object::ObjectArray(values, _) => values.clone(),
            _ => panic!(),
        };
        values
            .into_iter()
            .map(|object| object.map_or(Handle(0), |object| self.insert_ref(object, 0)))
            .collect()
    }
    fn reference_error(&self, function: &str, handle: u64) -> Box<dyn std::error::Error> {
        format!(
            "invalid JNI reference 0x{handle:x} in {function} from {}",
            self.location(self.lr().unwrap_or(0))
        )
        .into()
    }
    fn require_object(&self, function: &str, handle: u64) -> Result<Rc<RefCell<Object>>> {
        if handle == 0 {
            return Err(self.reference_error(function, handle));
        }
        self.jni
            .objects
            .get(&Handle(handle))
            .map(|reference| reference.object.clone())
            .ok_or_else(|| self.reference_error(function, handle))
    }
    fn require_member(&self, function: &str, id: u64) -> Result<Member> {
        id.checked_sub(MEMBER_ID_BASE)
            .and_then(|index| self.jni.members.get(index as usize))
            .cloned()
            .ok_or_else(|| {
                format!(
                    "invalid JNI id 0x{id:x} in {function} from {}",
                    self.location(self.lr().unwrap_or(0))
                )
                .into()
            })
    }
    fn pending_exception(&mut self, class: &str, message: Option<String>) {
        if let Some(old) = self.jni.pending.take() {
            self.release(old);
        }
        let handle = self.insert_object(Object::Exception(class.into(), message));
        self.jni.objects.get_mut(&handle).unwrap().scope = 0;
        self.jni.pending = Some(handle);
    }
    fn exception_result(&mut self) -> Result<()> {
        let Some(handle) = self.jni.pending.take() else {
            return Ok(());
        };
        let object = self.object(handle);
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
        match &*self.object(handle).borrow() {
            Object::User(_, v) => v.clone().downcast().ok(),
            _ => None,
        }
    }
    fn class_name(&self, handle: Handle) -> String {
        match &*self.object(handle).borrow() {
            Object::Class(name) | Object::User(name, _) => name.clone(),
            Object::String(_) => "java/lang/String".into(),
            Object::Array(kind, _) => kind.signature().into(),
            Object::ObjectArray(_, class) => format!("[L{class};"),
            Object::DirectBuffer(_, _) => "java/nio/ByteBuffer".into(),
            Object::Exception(class, _) => class.clone(),
        }
    }
    fn alias(&mut self, handle: Handle, scope: usize) -> Handle {
        if handle.0 == 0 {
            return handle;
        }
        let object = self.object(handle);
        self.insert_ref(object, scope)
    }
    pub(crate) fn setup_jni(&mut self) -> Result<()> {
        let p = self.pointer_size();
        let table = self.malloc(233 * p as usize)?;
        for index in 0..233 {
            let stub = self.stub(HostFunction::Jni(index))?;
            self.write_pointer(table + index as u64 * p, stub)?;
        }
        self.jni.env = self.malloc(p as usize)?;
        self.write_pointer(self.jni.env, table)?;
        let table = self.malloc(8 * p as usize)?;
        for index in 0..8 {
            let stub = self.stub(HostFunction::Vm(index))?;
            self.write_pointer(table + index as u64 * p, stub)?;
        }
        self.jni.vm = self.malloc(p as usize)?;
        self.write_pointer(self.jni.vm, table)?;
        Ok(())
    }
    pub fn registered_natives(&self) -> Vec<(String, String, u64)> {
        let mut natives: Vec<_> = self
            .jni
            .natives
            .iter()
            .map(|((class, method), &address)| (class.clone(), method.clone(), address))
            .collect();
        natives.sort_unstable_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        natives
    }
    pub fn call_jni_onload(&mut self, module: Module, handler: &mut dyn JniHandler) -> Result<u64> {
        self.cpu()?;
        let function = self.find_symbol(module, "JNI_OnLoad")?;
        self.invoke_jni(
            function,
            &[
                JniValue::Object(Handle(self.jni.vm)),
                JniValue::Object(Handle(0)),
            ],
            b'I',
            handler,
        )
        .map(JniValue::bits)
    }
    pub fn call_static_jni_method(
        &mut self,
        handler: &mut dyn JniHandler,
        class: &str,
        method: &str,
        args: &[JniValue],
    ) -> Result<JniValue> {
        let class_handle = self.class(class);
        self.call_jni_method(handler, class, class_handle, method, args)
    }
    pub fn call_jni_method(
        &mut self,
        handler: &mut dyn JniHandler,
        class: &str,
        receiver: Handle,
        method: &str,
        args: &[JniValue],
    ) -> Result<JniValue> {
        self.cpu()?;
        let (name, signature) = method.split_at(method.find('(').unwrap());
        let key = (class.to_owned(), method.to_owned());
        let function = if let Some(function) = self.jni.natives.get(&key) {
            *function
        } else {
            let short = format!("Java_{}_{}", mangle(class), mangle(name));
            let args_sig = signature[1..].split(')').next().unwrap();
            let long = format!("{short}__{}", mangle(args_sig));
            [short, long]
                .iter()
                .find_map(|symbol| {
                    (0..self.modules.len()).find_map(|i| self.local_symbol(Module(i), symbol))
                })
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?
        };
        let mut values = vec![JniValue::Object(Handle(self.jni.env)), receiver.into()];
        values.extend_from_slice(args);
        let result_type = signature.as_bytes()[signature.find(')').unwrap() + 1];
        self.invoke_jni(function, &values, result_type, handler)
    }
    fn invoke_jni(
        &mut self,
        function: u64,
        args: &[JniValue],
        kind: u8,
        handler: &mut dyn JniHandler,
    ) -> Result<JniValue> {
        let old_scope = self.jni.scope;
        self.jni.scope = old_scope.max(1) + 1;
        let result = self
            .invoke(
                function,
                args.iter().map(|&value| (value.kind(), value.bits())),
                kind,
                handler,
            )
            .map(|bits| decode(kind, bits));
        if let Ok(JniValue::Object(handle)) = result
            && let Some(reference) = self.jni.objects.get_mut(&handle)
        {
            reference.scope = old_scope;
        }
        self.jni
            .objects
            .retain(|_, reference| reference.scope <= old_scope.max(1));
        self.jni.scope = old_scope;
        self.exception_result()?;
        result
    }
    fn jni_allocator(&mut self, name: &str, argument: u64) -> Result<u64> {
        let function = (0..self.modules.len())
            .find_map(|i| self.local_symbol(Module(i), name))
            .ok_or_else(|| format!("JNI buffer requires {name}: no loaded library exports it"))?;
        self.call_function(function, &[argument])
    }
    fn jni_buffer(&mut self, bytes: &[u8]) -> Result<u64> {
        let ptr = self.jni_allocator("malloc", bytes.len() as u64)?;
        self.write_bytes(ptr, bytes)?;
        Ok(ptr)
    }
    fn jni_error(
        &self,
        function: &str,
        class: &str,
        name: &str,
        signature: &str,
    ) -> Box<dyn std::error::Error> {
        format!(
            "unhandled JNI call: {function} {class}->{name}{signature} from {}",
            self.location(self.lr().unwrap_or(0))
        )
        .into()
    }
    fn unsupported_jni(&self, name: &str) -> Box<dyn std::error::Error> {
        format!(
            "unsupported JNIEnv function {name} from {}",
            self.location(self.lr().unwrap_or(0))
        )
        .into()
    }
    fn unsupported_vm(&self, name: &str) -> Box<dyn std::error::Error> {
        format!(
            "unsupported JavaVM function {name} from {}",
            self.location(self.lr().unwrap_or(0))
        )
        .into()
    }
    fn validate_ref(&self, function: &str, handle: u64, kind: u8) -> Result<()> {
        if handle == 0 && kind == 6 {
            return Ok(());
        }
        if handle == 0 {
            return Err(self.reference_error(function, handle));
        }
        let object = self.require_object(function, handle)?;
        let valid = matches!(
            (&*object.borrow(), kind),
            (_, 0 | 6)
                | (Object::Class(_), 1)
                | (Object::String(_), 2)
                | (Object::Array(_, _), 3)
                | (Object::ObjectArray(_, _), 4)
                | (Object::Array(_, _) | Object::ObjectArray(_, _), 5)
        );
        if valid {
            Ok(())
        } else {
            Err(self.reference_error(function, handle))
        }
    }
    fn jni_arguments(
        &self,
        signature: &str,
        variant: usize,
        start: usize,
    ) -> Result<Vec<JniValue>> {
        let kinds = argument_types(signature);
        let bits = self.argument_bits(&kinds, variant, start)?;
        Ok(kinds
            .into_iter()
            .zip(bits)
            .map(|(kind, bits)| {
                if kind == b'F' && variant != 2 {
                    JniValue::Float(f64::from_bits(bits) as f32)
                } else {
                    decode(kind, bits)
                }
            })
            .collect())
    }
    pub(crate) fn jni_function(
        &mut self,
        index: usize,
        handler: &mut dyn JniHandler,
    ) -> Result<u64> {
        let function = jni_name(index);
        let a = self.reg(1)?;
        let b = self.reg(2)?;
        let c = self.reg(3)?;
        // The kind of reference each function takes as its first argument.
        // NewStringUTF, NewDirectByteBuffer and GetObjectRefType take a
        // pointer or any value there, so they are not checked.
        let kind = match index {
            21..=25 | 226 | 227 => Some(6),
            164..=166 | 168..=170 | 220 | 221 | 224 | 225 => Some(2),
            171 => Some(5),
            183..=214 | 222 | 223 => Some(3),
            27..=30 | 33 | 94 | 113..=162 | 215 | 216 => Some(1),
            31 | 32 | 34..=93 | 95..=112 | 230 | 231 => Some(0),
            _ => None,
        };
        if let Some(kind) = kind {
            self.validate_ref(&function, a, kind)?;
        }
        if index == 11 {
            self.validate_ref(&function, a, 1)?;
            self.validate_ref(&function, b, 1)?;
        }
        if index == 13 {
            self.validate_ref(&function, a, 0)?;
        }
        if index == 14 {
            self.validate_ref(&function, a, 1)?;
        }
        if index == 32 {
            self.validate_ref(&function, b, 1)?;
        }
        if index == 172 {
            self.validate_ref(&function, b, 1)?;
            if c != 0 {
                self.validate_ref(&function, c, 0)?;
            }
        }
        if matches!(index, 173 | 174) {
            self.validate_ref(&function, a, 4)?;
        }
        if index == 174 && c != 0 {
            self.validate_ref(&function, c, 0)?;
        }
        if index == 24 {
            self.validate_ref(&function, b, 6)?;
        }
        if matches!(index, 226 | 227) {
            self.validate_ref(&function, a, 6)?;
        }
        if self.verbose && !matches!(index,28..=30 | 34..=93 | 95..=112 | 114..=143 | 145..=162) {
            let details = match index {
                6 | 167 => self.cstring(a)?,
                33 | 94 | 113 | 144 => format!(
                    "{}->{}{}",
                    self.class_name(Handle(a)),
                    self.cstring(b)?,
                    self.cstring(c)?
                ),
                21..=25 | 31 | 164..=166 | 168..=171 | 183..=216 | 220..=225 if a != 0 => format!(
                    "{} handle=0x{a:x}, arg2=0x{b:x}, arg3=0x{c:x}",
                    self.class_name(Handle(a))
                ),
                _ => format!("arg1=0x{a:x}, arg2=0x{b:x}, arg3=0x{c:x}"),
            };
            eprintln!(
                "JNIEnv->{function}({details}) from {}",
                self.location(self.lr()?)
            );
        }
        match index {
            5 => Err(self.unsupported_jni("DefineClass")),
            7 => Err(self.unsupported_jni("FromReflectedMethod")),
            8 => Err(self.unsupported_jni("FromReflectedField")),
            9 => Err(self.unsupported_jni("ToReflectedMethod")),
            10 => Err(self.unsupported_jni("GetSuperclass")),
            12 => Err(self.unsupported_jni("ToReflectedField")),
            11 | 32 => {
                let from = self.class_name(Handle(a));
                let to = self.class_name(Handle(b));
                Ok(if from == to || to == "java/lang/Object" {
                    1
                } else {
                    handler
                        .is_assignable(self, &from, &to)
                        .ok_or_else(|| self.jni_error(&function, &from, &to, ""))?
                        as u64
                })
            }
            13 => {
                if self.jni.objects.contains_key(&Handle(a)) {
                    self.jni.pending = Some(self.alias(Handle(a), 0));
                    Ok(0)
                } else {
                    Err(self.reference_error(&function, a))
                }
            }
            14 => {
                let class = self.class_name(Handle(a));
                self.pending_exception(&class, Some(self.cstring(b)?));
                Ok(0)
            }
            15 => Ok(self
                .jni
                .pending
                .map(|handle| self.alias(handle, self.jni.scope).0)
                .unwrap_or(0)),
            16 => Ok(0),
            17 => {
                if let Some(handle) = self.jni.pending.take() {
                    self.release(handle);
                }
                Ok(0)
            }
            18 => Err(format!(
                "FatalError from native code: {} at {}",
                self.cstring(a)?,
                self.location(self.lr()?)
            )
            .into()),
            27 => Ok(self
                .insert_object(Object::User(self.class_name(Handle(a)), Rc::new(())))
                .0),
            217 | 218 => {
                self.validate_ref(&function, a, 0)?;
                Ok(0)
            }
            226 => {
                if a == 0 {
                    return Ok(0);
                }
                let handle = self.alias(Handle(a), 1);
                self.jni.weak.insert(handle);
                Ok(handle.0)
            }
            227 => {
                self.jni.weak.remove(&Handle(a));
                self.release(Handle(a));
                Ok(0)
            }
            228 => Ok(if self.jni.pending.is_some() { 1 } else { 0 }),
            172 => {
                let length = a as i32 as i64;
                if length < 0 {
                    self.pending_exception("java/lang/NegativeArraySizeException", None);
                    Ok(0)
                } else {
                    let initial = (c != 0).then(|| self.object(Handle(c)));
                    let class = self.class_name(Handle(b));
                    Ok(self
                        .insert_object(Object::ObjectArray(vec![initial; length as usize], class))
                        .0)
                }
            }
            173 => {
                let object = self.require_object(&function, a)?;
                let value = {
                    let Object::ObjectArray(values, _) = &*object.borrow() else {
                        return Err(self.reference_error(&function, a));
                    };
                    let index = b as i32 as i64;
                    if index < 0 || index as usize >= values.len() {
                        self.pending_exception("java/lang/ArrayIndexOutOfBoundsException", None);
                        return Ok(0);
                    }
                    values[index as usize].clone()
                };
                Ok(value.map_or(0, |object| self.insert_ref(object, self.jni.scope).0))
            }
            174 => {
                let value = (c != 0).then(|| self.object(Handle(c)));
                let object = self.require_object(&function, a)?;
                let Object::ObjectArray(values, _) = &mut *object.borrow_mut() else {
                    return Err(self.reference_error(&function, a));
                };
                let index = b as i32 as i64;
                if index < 0 || index as usize >= values.len() {
                    self.pending_exception("java/lang/ArrayIndexOutOfBoundsException", None);
                } else {
                    values[index as usize] = value;
                }
                Ok(0)
            }
            229 => {
                let capacity = self.argument(2, b'J')? as i64;
                Ok(self.insert_object(Object::DirectBuffer(a, capacity)).0)
            }
            230 => {
                let object = self.require_object(&function, a)?;
                let Object::DirectBuffer(ptr, _) = *object.borrow() else {
                    return Err(self.reference_error(&function, a));
                };
                Ok(ptr)
            }
            231 => {
                let object = self.require_object(&function, a)?;
                let Object::DirectBuffer(_, capacity) = *object.borrow() else {
                    return Err(self.reference_error(&function, a));
                };
                Ok(capacity as u64)
            }
            4 => Ok(0x10006),
            6 => {
                let name = self.cstring(a)?;
                Ok(self.find_class(&name, self.jni.scope).0)
            }
            26 => Ok(0),
            19 => {
                self.jni.scope += 1;
                Ok(0)
            }
            20 => {
                let object = if a != 0 {
                    Some(self.object(Handle(a)))
                } else {
                    None
                };
                let scope = self.jni.scope;
                self.jni.objects.retain(|_, v| v.scope < scope);
                self.jni.scope = scope.saturating_sub(1).max(2);
                if let Some(object) = object {
                    Ok(self.insert_ref(object, self.jni.scope).0)
                } else {
                    Ok(0)
                }
            }
            21 => Ok(self.alias(Handle(a), 1).0),
            22 => {
                if self
                    .jni
                    .objects
                    .get(&Handle(a))
                    .is_some_and(|v| v.scope == 1)
                {
                    self.release(Handle(a));
                }
                Ok(0)
            }
            23 => {
                if self
                    .jni
                    .objects
                    .get(&Handle(a))
                    .is_some_and(|v| v.scope >= 2)
                {
                    self.release(Handle(a));
                }
                Ok(0)
            }
            24 => Ok(if a == 0 || b == 0 {
                a == b
            } else {
                Rc::ptr_eq(&self.object(Handle(a)), &self.object(Handle(b)))
            } as u64),
            25 => Ok(self.alias(Handle(a), self.jni.scope).0),
            31 => {
                let name = match &*self.object(Handle(a)).borrow() {
                    Object::Class(_) => "java/lang/Class".to_owned(),
                    _ => self.class_name(Handle(a)),
                };
                Ok(self.find_class(&name, self.jni.scope).0)
            }
            33 | 94 | 113 | 144 => {
                let class = self.class_name(Handle(a));
                let name = self.cstring(b)?;
                let signature = self.cstring(c)?;
                let key = (
                    class.clone(),
                    name.clone(),
                    signature.clone(),
                    index >= 113,
                    index == 94 || index == 144,
                );
                if let Some(id) = self.jni.member_ids.get(&key) {
                    return Ok(*id);
                }
                let id = self.jni.members.len() as u64 + MEMBER_ID_BASE;
                self.jni.members.push(Member {
                    class,
                    name,
                    signature,
                });
                self.jni.member_ids.insert(key, id);
                Ok(id)
            }
            28..=30 | 34..=93 | 95..=112 | 114..=143 | 145..=162 => {
                let constructor = (28..=30).contains(&index);
                let nonvirtual = (64..=93).contains(&index);
                let member_id = if nonvirtual { c } else { b };
                let member = self.require_member(&function, member_id)?;
                let (operation, kind, variant) = match index {
                    28..=30 => (6, 0, index - 28),
                    34..=63 => (0, (index - 34) / 3, (index - 34) % 3),
                    64..=93 => (0, (index - 64) / 3, (index - 64) % 3),
                    95..=103 => (2, index - 95, 0),
                    104..=112 => (4, index - 104, 0),
                    114..=143 => (1, (index - 114) / 3, (index - 114) % 3),
                    145..=153 => (3, index - 145, 0),
                    154..=162 => (5, index - 154, 0),
                    _ => unreachable!(),
                };
                let args = if operation == 4 || operation == 5 {
                    let kind = member.signature.as_bytes()[0];
                    let bits = self.argument(3, kind)?;
                    vec![decode(kind, bits)]
                } else if operation == 2 || operation == 3 {
                    Vec::new()
                } else {
                    self.jni_arguments(&member.signature, variant, if nonvirtual { 4 } else { 3 })?
                };
                let call = JniCall {
                    class: &member.class,
                    name: &member.name,
                    signature: &member.signature,
                    receiver: Handle(a),
                    args: &args,
                };
                if self.verbose {
                    eprintln!(
                        "JNIEnv->{function}({}->{}{}, {:?}) from {}",
                        call.class,
                        call.name,
                        call.signature,
                        call.args,
                        self.location(self.lr()?)
                    );
                }
                let result = if constructor {
                    if member.class == "java/lang/String"
                        && member.signature == "([BLjava/lang/String;)V"
                    {
                        let bytes = self.byte_array(args[0].object());
                        let charset = self
                            .string(args[1].object())
                            .to_ascii_lowercase()
                            .replace('-', "");
                        let text = match charset.as_str() {
                            "utf8" => Some(String::from_utf8_lossy(&bytes).into_owned()),
                            "iso88591" => Some(bytes.iter().map(|&b| char::from(b)).collect()),
                            "usascii" => Some(
                                bytes
                                    .iter()
                                    .map(|&b| if b < 128 { char::from(b) } else { '\u{fffd}' })
                                    .collect(),
                            ),
                            _ => None,
                        };
                        text.map(|text| {
                            JniValue::Object(
                                self.insert_object(Object::String(text.encode_utf16().collect())),
                            )
                        })
                    } else {
                        handler.new_object(self, &call).map(JniValue::Object)
                    }
                } else {
                    dispatch(handler, self, &call, operation, kind)
                };
                let value = result.ok_or_else(|| {
                    self.jni_error(&function, &member.class, &member.name, &member.signature)
                })?;
                self.write_jni_result(value)?;
                Ok(value.bits())
            }
            163 => {
                let length = b as i32 as i64;
                if length < 0 {
                    self.pending_exception("java/lang/NegativeArraySizeException", None);
                    return Ok(0);
                }
                let bytes = self.read_bytes(a, length as usize * 2)?;
                Ok(self
                    .insert_object(Object::String(
                        bytes
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|b| u16::from_le_bytes([b[0], b[1]]))
                            .collect(),
                    ))
                    .0)
            }
            164 => {
                let object = self.object(Handle(a));
                let Object::String(value) = &*object.borrow() else {
                    unreachable!()
                };
                Ok(value.len() as u64)
            }
            165 | 169 | 224 => {
                let object = self.object(Handle(a));
                let Object::String(value) = &*object.borrow() else {
                    unreachable!()
                };
                let mut bytes = if index == 169 {
                    modified_utf8(value)
                } else {
                    value.iter().flat_map(|c| c.to_le_bytes()).collect()
                };
                if index == 169 {
                    bytes.push(0);
                }
                if b != 0 {
                    self.write_bytes(b, &[1])?;
                }
                self.jni_buffer(&bytes)
            }
            166 | 170 | 225 => {
                self.jni_allocator("free", b)?;
                Ok(0)
            }
            167 => {
                let mut bytes = Vec::new();
                let mut p = a;
                loop {
                    let byte = self.read_bytes(p, 1)?[0];
                    if byte == 0 {
                        break;
                    }
                    bytes.push(byte);
                    p += 1;
                }
                Ok(self
                    .insert_object(Object::String(decode_modified_utf8(&bytes)))
                    .0)
            }
            168 => {
                let object = self.object(Handle(a));
                let Object::String(value) = &*object.borrow() else {
                    unreachable!()
                };
                Ok(modified_utf8(value).len() as u64)
            }
            171 => {
                let object = self.require_object(&function, a)?;
                match &*object.borrow() {
                    Object::Array(kind, bytes) => Ok((bytes.len() / kind.size()) as u64),
                    Object::ObjectArray(values, _) => Ok(values.len() as u64),
                    _ => Err(self.reference_error(&function, a)),
                }
            }
            175..=182 => {
                let length = a as i32 as i64;
                if length < 0 {
                    self.pending_exception("java/lang/NegativeArraySizeException", None);
                    Ok(0)
                } else {
                    let kind = ARRAY_TYPES[index - 175];
                    Ok(self
                        .insert_object(Object::Array(kind, vec![0; length as usize * kind.size()]))
                        .0)
                }
            }
            183..=190 | 222 => {
                let bytes = self.primitive_array(Handle(a));
                if b != 0 {
                    self.write_bytes(b, &[1])?;
                }
                self.jni_buffer(&bytes)
            }
            191..=198 | 223 => {
                if c == 0 || c == 1 {
                    let object = self.object(Handle(a));
                    let mut object = object.borrow_mut();
                    let Object::Array(_, bytes) = &mut *object else {
                        unreachable!()
                    };
                    self.cpu()?.mem_read(b, bytes)?;
                }
                if c != 1 {
                    self.jni_allocator("free", b)?;
                }
                Ok(0)
            }
            199..=214 => {
                let object = self.require_object(&function, a)?;
                let (width, total) = match &*object.borrow() {
                    Object::Array(kind, bytes) => (*kind as PrimitiveType, bytes.len()),
                    _ => return Err(self.reference_error(&function, a)),
                };
                let start = b as i32 as i64;
                let count = c as i32 as i64;
                if start < 0
                    || count < 0
                    || (start as usize).saturating_add(count as usize) * width.size() > total
                {
                    self.pending_exception("java/lang/ArrayIndexOutOfBoundsException", None);
                    return Ok(0);
                }
                let start = start as usize * width.size();
                let length = count as usize * width.size();
                let ptr = self.argument(4, b'L')?;
                let mut object = object.borrow_mut();
                let Object::Array(_, bytes) = &mut *object else {
                    unreachable!()
                };
                if index < 207 {
                    self.write_bytes(ptr, &bytes[start..start + length])?;
                } else {
                    self.cpu()?
                        .mem_read(ptr, &mut bytes[start..start + length])?;
                }
                Ok(0)
            }
            215 => {
                let class = self.class_name(Handle(a));
                for i in 0..c {
                    let p = self.pointer_size();
                    let entry = b + i * 3 * p;
                    let name = self.cstring(self.read_pointer(entry)?)?;
                    let signature = self.cstring(self.read_pointer(entry + p)?)?;
                    let ptr = self.read_pointer(entry + 2 * p)?;
                    self.jni
                        .natives
                        .insert((class.clone(), format!("{name}{signature}")), ptr);
                }
                Ok(0)
            }
            216 => {
                let class = self.class_name(Handle(a));
                self.jni.natives.retain(|(c, _), _| *c != class);
                Ok(0)
            }
            219 => {
                self.write_pointer(a, self.jni.vm)?;
                Ok(0)
            }
            220 | 221 => {
                let object = self.require_object(&function, a)?;
                let Object::String(value) = &*object.borrow() else {
                    return Err(self.reference_error(&function, a));
                };
                let start = b as i32 as i64;
                let length = c as i32 as i64;
                if start < 0
                    || length < 0
                    || (start as usize).saturating_add(length as usize) > value.len()
                {
                    self.pending_exception("java/lang/StringIndexOutOfBoundsException", None);
                    return Ok(0);
                }
                let units = &value[start as usize..start as usize + length as usize];
                let bytes = if index == 221 {
                    modified_utf8(units)
                } else {
                    units.iter().flat_map(|c| c.to_le_bytes()).collect()
                };
                self.write_bytes(self.argument(4, b'L')?, &bytes)?;
                Ok(0)
            }
            232 => Ok(self.jni.objects.get(&Handle(a)).map_or(0, |v| {
                if self.jni.weak.contains(&Handle(a)) {
                    3
                } else if v.scope == 1 {
                    2
                } else {
                    1
                }
            })),
            _ => Err(self.jni_error(&function, "JNIEnv", "", "")),
        }
    }
    pub(crate) fn vm_function(&mut self, index: usize) -> Result<u64> {
        match index {
            3 => Err(self.unsupported_vm("DestroyJavaVM")),
            4 | 6 | 7 => {
                self.write_pointer(self.reg(1)?, self.jni.env)?;
                Ok(0)
            }
            5 => Ok(0),
            _ => Err(self.unsupported_vm("reserved")),
        }
    }
}
fn modified_utf8(units: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for &c in units {
        if c != 0 && c < 128 {
            bytes.push(c as u8);
        } else if c < 2048 {
            bytes.extend([(0xc0 | (c >> 6)) as u8, (0x80 | (c & 63)) as u8]);
        } else {
            bytes.extend([
                (0xe0 | (c >> 12)) as u8,
                (0x80 | ((c >> 6) & 63)) as u8,
                (0x80 | (c & 63)) as u8,
            ]);
        }
    }
    bytes
}
fn decode_modified_utf8(bytes: &[u8]) -> Vec<u16> {
    let mut units = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let a = bytes[i] as u16;
        i += 1;
        let value = if a < 128 {
            a
        } else if a & 0xe0 == 0xc0 {
            let b = bytes[i] as u16;
            i += 1;
            ((a & 31) << 6) | (b & 63)
        } else {
            let b = bytes[i] as u16;
            let c = bytes[i + 1] as u16;
            i += 2;
            ((a & 15) << 12) | ((b & 63) << 6) | (c & 63)
        };
        units.push(value);
    }
    units
}
fn jni_name(index: usize) -> String {
    const TYPES: [&str; 10] = [
        "Object", "Boolean", "Byte", "Char", "Short", "Int", "Long", "Float", "Double", "Void",
    ];
    const VARIANTS: [&str; 3] = ["", "V", "A"];
    match index {
        28..=30 => format!("NewObject{}", VARIANTS[index - 28]),
        34..=93 => {
            let base = if index < 64 { 34 } else { 64 };
            format!(
                "Call{}{}Method{}",
                if base == 64 { "Nonvirtual" } else { "" },
                TYPES[(index - base) / 3],
                VARIANTS[(index - base) % 3]
            )
        }
        114..=143 => format!(
            "CallStatic{}Method{}",
            TYPES[(index - 114) / 3],
            VARIANTS[(index - 114) % 3]
        ),
        95..=112 | 145..=162 => {
            let base = if index < 145 { 95 } else { 145 };
            let set = index - base >= 9;
            format!(
                "{}{}{}Field",
                if set { "Set" } else { "Get" },
                if base == 145 { "Static" } else { "" },
                TYPES[(index - base) % 9]
            )
        }
        175..=214 => {
            let block = (index - 175) / 8;
            format!(
                "{}{}Array{}",
                ["New", "Get", "Release", "Get", "Set"][block],
                TYPES[(index - 175) % 8 + 1],
                ["", "Elements", "Elements", "Region", "Region"][block]
            )
        }
        _ => match index {
            0..=3 => "reserved",
            4 => "GetVersion",
            5 => "DefineClass",
            6 => "FindClass",
            7 => "FromReflectedMethod",
            8 => "FromReflectedField",
            9 => "ToReflectedMethod",
            10 => "GetSuperclass",
            11 => "IsAssignableFrom",
            12 => "ToReflectedField",
            13 => "Throw",
            14 => "ThrowNew",
            15 => "ExceptionOccurred",
            16 => "ExceptionDescribe",
            17 => "ExceptionClear",
            18 => "FatalError",
            19 => "PushLocalFrame",
            20 => "PopLocalFrame",
            21 => "NewGlobalRef",
            22 => "DeleteGlobalRef",
            23 => "DeleteLocalRef",
            24 => "IsSameObject",
            25 => "NewLocalRef",
            26 => "EnsureLocalCapacity",
            27 => "AllocObject",
            31 => "GetObjectClass",
            32 => "IsInstanceOf",
            33 => "GetMethodID",
            94 => "GetFieldID",
            113 => "GetStaticMethodID",
            144 => "GetStaticFieldID",
            163 => "NewString",
            164 => "GetStringLength",
            165 => "GetStringChars",
            166 => "ReleaseStringChars",
            167 => "NewStringUTF",
            168 => "GetStringUTFLength",
            169 => "GetStringUTFChars",
            170 => "ReleaseStringUTFChars",
            171 => "GetArrayLength",
            172 => "NewObjectArray",
            173 => "GetObjectArrayElement",
            174 => "SetObjectArrayElement",
            215 => "RegisterNatives",
            216 => "UnregisterNatives",
            217 => "MonitorEnter",
            218 => "MonitorExit",
            219 => "GetJavaVM",
            220 => "GetStringRegion",
            221 => "GetStringUTFRegion",
            222 => "GetPrimitiveArrayCritical",
            223 => "ReleasePrimitiveArrayCritical",
            224 => "GetStringCritical",
            225 => "ReleaseStringCritical",
            226 => "NewWeakGlobalRef",
            227 => "DeleteWeakGlobalRef",
            228 => "ExceptionCheck",
            229 => "NewDirectByteBuffer",
            230 => "GetDirectBufferAddress",
            231 => "GetDirectBufferCapacity",
            232 => "GetObjectRefType",
            _ => unreachable!(),
        }
        .into(),
    }
}
fn mangle(value: &str) -> String {
    let mut out = String::new();
    for c in value.encode_utf16() {
        match c {
            47 => out.push('_'),
            95 => out.push_str("_1"),
            59 => out.push_str("_2"),
            91 => out.push_str("_3"),
            48..=57 | 65..=90 | 97..=122 => out.push(char::from_u32(c as u32).unwrap()),
            _ => out.push_str(&format!("_0{c:04x}")),
        }
    }
    out
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Handle(pub u64);
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JniValue {
    Object(Handle),
    Boolean(bool),
    Byte(i8),
    Char(u16),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Void,
}
impl From<Handle> for JniValue {
    fn from(v: Handle) -> Self {
        Self::Object(v)
    }
}
impl From<i32> for JniValue {
    fn from(v: i32) -> Self {
        Self::Int(v)
    }
}
impl From<i64> for JniValue {
    fn from(v: i64) -> Self {
        Self::Long(v)
    }
}
impl JniValue {
    pub(crate) fn kind(self) -> u8 {
        match self {
            Self::Long(_) => b'J',
            Self::Float(_) => b'F',
            Self::Double(_) => b'D',
            _ => b'L',
        }
    }
    pub fn object(self) -> Handle {
        if let Self::Object(v) = self {
            v
        } else {
            panic!()
        }
    }
    pub fn long(self) -> i64 {
        if let Self::Long(v) = self {
            v
        } else {
            panic!()
        }
    }
    pub fn int(self) -> i32 {
        if let Self::Int(v) = self { v } else { panic!() }
    }
    pub(crate) fn bits(self) -> u64 {
        match self {
            Self::Object(v) => v.0,
            Self::Boolean(v) => v as u64,
            Self::Byte(v) => v as u64,
            Self::Char(v) => v as u64,
            Self::Short(v) => v as u64,
            Self::Int(v) => v as u64,
            Self::Long(v) => v as u64,
            Self::Float(v) => v.to_bits() as u64,
            Self::Double(v) => v.to_bits(),
            Self::Void => 0,
        }
    }
}
fn decode(kind: u8, bits: u64) -> JniValue {
    match kind {
        b'L' | b'[' => JniValue::Object(Handle(bits)),
        b'Z' => JniValue::Boolean(bits != 0),
        b'B' => JniValue::Byte(bits as i8),
        b'C' => JniValue::Char(bits as u16),
        b'S' => JniValue::Short(bits as i16),
        b'I' => JniValue::Int(bits as i32),
        b'J' => JniValue::Long(bits as i64),
        b'F' => JniValue::Float(f32::from_bits(bits as u32)),
        b'D' => JniValue::Double(f64::from_bits(bits)),
        _ => JniValue::Void,
    }
}
fn argument_types(signature: &str) -> Vec<u8> {
    let mut types = Vec::new();
    let mut bytes = signature.bytes().skip(1);
    while let Some(mut kind) = bytes.next() {
        if kind == b')' {
            break;
        }
        types.push(kind);
        while kind == b'[' {
            kind = bytes.next().unwrap();
        }
        if kind == b'L' {
            for c in bytes.by_ref() {
                if c == b';' {
                    break;
                }
            }
        }
    }
    types
}

/// Decoded member access delivered to a handler. Objects are handles into this emulator.
pub struct JniCall<'a> {
    pub class: &'a str,
    pub name: &'a str,
    pub signature: &'a str,
    pub receiver: Handle,
    pub args: &'a [JniValue],
}

pub trait JniHandler {
    fn is_assignable(&mut self, _emu: &mut Emulator, _from: &str, _to: &str) -> Option<bool> {
        None
    }
    fn new_object(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<Handle> {
        None
    }
    fn call_object_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<Handle> {
        None
    }
    fn call_static_object_method(
        &mut self,
        _emu: &mut Emulator,
        _call: &JniCall<'_>,
    ) -> Option<Handle> {
        None
    }
    fn get_object_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<Handle> {
        None
    }
    fn get_static_object_field(
        &mut self,
        _emu: &mut Emulator,
        _call: &JniCall<'_>,
    ) -> Option<Handle> {
        None
    }
    fn set_object_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn set_static_object_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn call_boolean_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<bool> {
        None
    }
    fn call_static_boolean_method(
        &mut self,
        _emu: &mut Emulator,
        _call: &JniCall<'_>,
    ) -> Option<bool> {
        None
    }
    fn get_boolean_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<bool> {
        None
    }
    fn get_static_boolean_field(
        &mut self,
        _emu: &mut Emulator,
        _call: &JniCall<'_>,
    ) -> Option<bool> {
        None
    }
    fn set_boolean_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn set_static_boolean_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn call_byte_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i8> {
        None
    }
    fn call_static_byte_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i8> {
        None
    }
    fn get_byte_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i8> {
        None
    }
    fn get_static_byte_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i8> {
        None
    }
    fn set_byte_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn set_static_byte_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn call_char_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<u16> {
        None
    }
    fn call_static_char_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<u16> {
        None
    }
    fn get_char_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<u16> {
        None
    }
    fn get_static_char_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<u16> {
        None
    }
    fn set_char_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn set_static_char_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn call_short_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i16> {
        None
    }
    fn call_static_short_method(
        &mut self,
        _emu: &mut Emulator,
        _call: &JniCall<'_>,
    ) -> Option<i16> {
        None
    }
    fn get_short_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i16> {
        None
    }
    fn get_static_short_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i16> {
        None
    }
    fn set_short_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn set_static_short_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn call_int_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i32> {
        None
    }
    fn call_static_int_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i32> {
        None
    }
    fn get_int_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i32> {
        None
    }
    fn get_static_int_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i32> {
        None
    }
    fn set_int_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn set_static_int_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn call_long_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i64> {
        None
    }
    fn call_static_long_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i64> {
        None
    }
    fn get_long_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i64> {
        None
    }
    fn get_static_long_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i64> {
        None
    }
    fn set_long_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn set_static_long_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn call_float_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<f32> {
        None
    }
    fn call_static_float_method(
        &mut self,
        _emu: &mut Emulator,
        _call: &JniCall<'_>,
    ) -> Option<f32> {
        None
    }
    fn get_float_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<f32> {
        None
    }
    fn get_static_float_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<f32> {
        None
    }
    fn set_float_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn set_static_float_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn call_double_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<f64> {
        None
    }
    fn call_static_double_method(
        &mut self,
        _emu: &mut Emulator,
        _call: &JniCall<'_>,
    ) -> Option<f64> {
        None
    }
    fn get_double_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<f64> {
        None
    }
    fn get_static_double_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<f64> {
        None
    }
    fn set_double_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn set_static_double_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn call_void_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn call_static_void_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
}
impl JniHandler for () {}
fn dispatch(
    handler: &mut dyn JniHandler,
    emu: &mut Emulator,
    call: &JniCall<'_>,
    operation: u8,
    kind: usize,
) -> Option<JniValue> {
    match (operation, kind) {
        (0, 0) => handler.call_object_method(emu, call).map(JniValue::Object),
        (1, 0) => handler
            .call_static_object_method(emu, call)
            .map(JniValue::Object),
        (2, 0) => handler.get_object_field(emu, call).map(JniValue::Object),
        (3, 0) => handler
            .get_static_object_field(emu, call)
            .map(JniValue::Object),
        (4, 0) => handler.set_object_field(emu, call).map(|_| JniValue::Void),
        (5, 0) => handler
            .set_static_object_field(emu, call)
            .map(|_| JniValue::Void),
        (0, 1) => handler
            .call_boolean_method(emu, call)
            .map(JniValue::Boolean),
        (1, 1) => handler
            .call_static_boolean_method(emu, call)
            .map(JniValue::Boolean),
        (2, 1) => handler.get_boolean_field(emu, call).map(JniValue::Boolean),
        (3, 1) => handler
            .get_static_boolean_field(emu, call)
            .map(JniValue::Boolean),
        (4, 1) => handler.set_boolean_field(emu, call).map(|_| JniValue::Void),
        (5, 1) => handler
            .set_static_boolean_field(emu, call)
            .map(|_| JniValue::Void),
        (0, 2) => handler.call_byte_method(emu, call).map(JniValue::Byte),
        (1, 2) => handler
            .call_static_byte_method(emu, call)
            .map(JniValue::Byte),
        (2, 2) => handler.get_byte_field(emu, call).map(JniValue::Byte),
        (3, 2) => handler.get_static_byte_field(emu, call).map(JniValue::Byte),
        (4, 2) => handler.set_byte_field(emu, call).map(|_| JniValue::Void),
        (5, 2) => handler
            .set_static_byte_field(emu, call)
            .map(|_| JniValue::Void),
        (0, 3) => handler.call_char_method(emu, call).map(JniValue::Char),
        (1, 3) => handler
            .call_static_char_method(emu, call)
            .map(JniValue::Char),
        (2, 3) => handler.get_char_field(emu, call).map(JniValue::Char),
        (3, 3) => handler.get_static_char_field(emu, call).map(JniValue::Char),
        (4, 3) => handler.set_char_field(emu, call).map(|_| JniValue::Void),
        (5, 3) => handler
            .set_static_char_field(emu, call)
            .map(|_| JniValue::Void),
        (0, 4) => handler.call_short_method(emu, call).map(JniValue::Short),
        (1, 4) => handler
            .call_static_short_method(emu, call)
            .map(JniValue::Short),
        (2, 4) => handler.get_short_field(emu, call).map(JniValue::Short),
        (3, 4) => handler
            .get_static_short_field(emu, call)
            .map(JniValue::Short),
        (4, 4) => handler.set_short_field(emu, call).map(|_| JniValue::Void),
        (5, 4) => handler
            .set_static_short_field(emu, call)
            .map(|_| JniValue::Void),
        (0, 5) => handler.call_int_method(emu, call).map(JniValue::Int),
        (1, 5) => handler.call_static_int_method(emu, call).map(JniValue::Int),
        (2, 5) => handler.get_int_field(emu, call).map(JniValue::Int),
        (3, 5) => handler.get_static_int_field(emu, call).map(JniValue::Int),
        (4, 5) => handler.set_int_field(emu, call).map(|_| JniValue::Void),
        (5, 5) => handler
            .set_static_int_field(emu, call)
            .map(|_| JniValue::Void),
        (0, 6) => handler.call_long_method(emu, call).map(JniValue::Long),
        (1, 6) => handler
            .call_static_long_method(emu, call)
            .map(JniValue::Long),
        (2, 6) => handler.get_long_field(emu, call).map(JniValue::Long),
        (3, 6) => handler.get_static_long_field(emu, call).map(JniValue::Long),
        (4, 6) => handler.set_long_field(emu, call).map(|_| JniValue::Void),
        (5, 6) => handler
            .set_static_long_field(emu, call)
            .map(|_| JniValue::Void),
        (0, 7) => handler.call_float_method(emu, call).map(JniValue::Float),
        (1, 7) => handler
            .call_static_float_method(emu, call)
            .map(JniValue::Float),
        (2, 7) => handler.get_float_field(emu, call).map(JniValue::Float),
        (3, 7) => handler
            .get_static_float_field(emu, call)
            .map(JniValue::Float),
        (4, 7) => handler.set_float_field(emu, call).map(|_| JniValue::Void),
        (5, 7) => handler
            .set_static_float_field(emu, call)
            .map(|_| JniValue::Void),
        (0, 8) => handler.call_double_method(emu, call).map(JniValue::Double),
        (1, 8) => handler
            .call_static_double_method(emu, call)
            .map(JniValue::Double),
        (2, 8) => handler.get_double_field(emu, call).map(JniValue::Double),
        (3, 8) => handler
            .get_static_double_field(emu, call)
            .map(JniValue::Double),
        (4, 8) => handler.set_double_field(emu, call).map(|_| JniValue::Void),
        (5, 8) => handler
            .set_static_double_field(emu, call)
            .map(|_| JniValue::Void),
        (0, 9) => handler.call_void_method(emu, call).map(|_| JniValue::Void),
        (1, 9) => handler
            .call_static_void_method(emu, call)
            .map(|_| JniValue::Void),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn libc_emulator(arch: crate::Architecture) -> Result<(Emulator, Module)> {
        let mut emu = Emulator::builder().build()?;
        let libc = emu.load_library(format!("rootfs/{}/libc.so", arch.library_dir()))?;
        Ok((emu, libc))
    }

    fn jni_call(emu: &mut Emulator, index: u64, args: &[u64]) -> Result<u64> {
        let table = emu.read_pointer(emu.jni.env)?;
        let function = emu.read_pointer(table + index * emu.pointer_size())?;
        let mut words = vec![emu.jni.env];
        words.extend_from_slice(args);
        emu.call_function(function, &words)
    }

    fn string_chars_use_libc_heap(get: u64, release: u64, expected: &[u8]) -> Result<()> {
        for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
            let (mut emu, libc) = libc_emulator(arch)?;
            let string = emu.new_string("a\0é😀");
            let copy = emu.malloc(1)?;
            let ptr = jni_call(&mut emu, get, &[string.0, copy])?;
            assert_eq!(emu.read_bytes(ptr, expected.len())?, expected);
            assert_eq!(emu.read_bytes(copy, 1)?, [1]);
            let usable = emu.find_symbol(libc, "malloc_usable_size")?;
            assert!(emu.call_function(usable, &[ptr])? >= expected.len() as u64);
            jni_call(&mut emu, release, &[string.0, ptr])?;
            let malloc = emu.find_symbol(libc, "malloc")?;
            assert_eq!(emu.call_function(malloc, &[expected.len() as u64])?, ptr);
            let free = emu.find_symbol(libc, "free")?;
            emu.call_function(free, &[ptr])?;
        }
        Ok(())
    }

    #[test]
    fn get_string_utf_chars_uses_libc_heap() -> Result<()> {
        string_chars_use_libc_heap(169, 170, b"a\xc0\x80\xc3\xa9\xed\xa0\xbd\xed\xb8\x80\0")
    }

    #[test]
    fn get_string_chars_uses_libc_heap() -> Result<()> {
        string_chars_use_libc_heap(165, 166, &[0x61, 0, 0, 0, 0xe9, 0, 0x3d, 0xd8, 0, 0xde])
    }

    #[test]
    fn get_string_critical_uses_libc_heap() -> Result<()> {
        string_chars_use_libc_heap(224, 225, &[0x61, 0, 0, 0, 0xe9, 0, 0x3d, 0xd8, 0, 0xde])
    }

    fn array_elements_use_libc_heap(get: u64, release: u64, kind: PrimitiveType) -> Result<()> {
        for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
            let (mut emu, libc) = libc_emulator(arch)?;
            let malloc = emu.find_symbol(libc, "malloc")?;
            let free = emu.find_symbol(libc, "free")?;
            let usable = emu.find_symbol(libc, "malloc_usable_size")?;
            let copy = emu.malloc(1)?;
            let length = 3 * kind.size();
            for mode in [0, 1, 2] {
                let array = emu.new_primitive_array(kind, &vec![1; length]);
                let ptr = jni_call(&mut emu, get, &[array.0, copy])?;
                assert_eq!(emu.read_bytes(ptr, length)?, vec![1; length]);
                assert_eq!(emu.read_bytes(copy, 1)?, [1]);
                assert!(emu.call_function(usable, &[ptr])? >= length as u64);
                emu.write_bytes(ptr, &vec![0; length])?;
                jni_call(&mut emu, release, &[array.0, ptr, mode])?;
                assert_eq!(
                    emu.primitive_array(array),
                    vec![u8::from(mode == 2); length]
                );
                if mode == 1 {
                    let other = emu.call_function(malloc, &[length as u64])?;
                    assert_ne!(other, ptr);
                    emu.write_bytes(ptr, &vec![1; length])?;
                    jni_call(&mut emu, release, &[array.0, ptr, 0])?;
                    assert_eq!(emu.primitive_array(array), vec![1; length]);
                    assert_eq!(emu.call_function(malloc, &[length as u64])?, ptr);
                    emu.call_function(free, &[other])?;
                } else {
                    assert_eq!(emu.call_function(malloc, &[length as u64])?, ptr);
                }
                emu.call_function(free, &[ptr])?;
                emu.release(array);
            }
        }
        Ok(())
    }

    macro_rules! array_heap_test {
        ($name:ident, $get:expr, $release:expr, $kind:ident) => {
            #[test]
            fn $name() -> Result<()> {
                array_elements_use_libc_heap($get, $release, PrimitiveType::$kind)
            }
        };
    }
    array_heap_test!(get_boolean_array_elements_uses_libc_heap, 183, 191, Boolean);
    array_heap_test!(get_byte_array_elements_uses_libc_heap, 184, 192, Byte);
    array_heap_test!(get_char_array_elements_uses_libc_heap, 185, 193, Char);
    array_heap_test!(get_short_array_elements_uses_libc_heap, 186, 194, Short);
    array_heap_test!(get_int_array_elements_uses_libc_heap, 187, 195, Int);
    array_heap_test!(get_long_array_elements_uses_libc_heap, 188, 196, Long);
    array_heap_test!(get_float_array_elements_uses_libc_heap, 189, 197, Float);
    array_heap_test!(get_double_array_elements_uses_libc_heap, 190, 198, Double);
    array_heap_test!(get_primitive_array_critical_uses_libc_heap, 222, 223, Int);

    #[test]
    fn native_free_accepts_string_utf_chars() -> Result<()> {
        for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
            let (mut emu, libc) = libc_emulator(arch)?;
            let string = emu.new_string("359250054370919||1551086094");
            let ptr = jni_call(&mut emu, 169, &[string.0, 0])?;
            let free = emu.find_symbol(libc, "free")?;
            emu.call_function(free, &[ptr])?;
            let malloc = emu.find_symbol(libc, "malloc")?;
            let next = emu.call_function(malloc, &[26])?;
            assert_eq!(next, ptr);
            emu.write_bytes(next, &[0x42; 26])?;
            assert_eq!(emu.read_bytes(next, 26)?, [0x42; 26]);
            emu.call_function(free, &[next])?;
        }
        Ok(())
    }

    #[test]
    fn new_string_utf_decodes_modified_utf8() -> Result<()> {
        for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
            let (mut emu, _) = libc_emulator(arch)?;
            let bytes = b"a\xc0\x80\xc3\xa9\xed\xa0\xbd\xed\xb8\x80\0";
            let ptr = emu.malloc(bytes.len())?;
            emu.write_bytes(ptr, bytes)?;
            let string = jni_call(&mut emu, 167, &[ptr])?;
            assert_eq!(emu.string(Handle(string)), "a\0é😀");
            emu.free(ptr);
        }
        Ok(())
    }

    #[test]
    fn jni_buffers_require_guest_malloc_and_free() -> Result<()> {
        for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
            let mut emu = Emulator::builder().build()?;
            emu.setup_cpu(arch)?;
            let string = emu.new_string("hello");
            let array = emu.new_byte_array(&[1]);
            for (get, release, handle) in [
                (165, 166, string),
                (169, 170, string),
                (224, 225, string),
                (184, 192, array),
                (222, 223, array),
            ] {
                assert_eq!(
                    jni_call(&mut emu, get, &[handle.0, 0])
                        .unwrap_err()
                        .to_string(),
                    "JNI buffer requires malloc: no loaded library exports it"
                );
                assert_eq!(
                    jni_call(&mut emu, release, &[handle.0, 0, 2])
                        .unwrap_err()
                        .to_string(),
                    "JNI buffer requires free: no loaded library exports it"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn arm32_mixed_variadic_v_and_a_calls_include_nonvirtual_stack_arguments() -> Result<()> {
        struct Handler;
        impl JniHandler for Handler {
            fn call_long_method(&mut self, _: &mut Emulator, call: &JniCall<'_>) -> Option<i64> {
                assert_eq!(
                    call.args,
                    [
                        JniValue::Long(0x1122_3344_5566_7788),
                        JniValue::Int(-7),
                        JniValue::Long(-2),
                        JniValue::Float(3.5),
                        JniValue::Double(9.5)
                    ]
                );
                Some(0x8877_6655_4433_2211u64 as i64)
            }
            fn call_static_long_method(
                &mut self,
                emu: &mut Emulator,
                call: &JniCall<'_>,
            ) -> Option<i64> {
                self.call_long_method(emu, call)
            }
        }
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(crate::Architecture::Arm32)?;
        let class = emu.class("example/Mixed");
        let name = emu.new_cstring("mixed")?;
        let sig = emu.new_cstring("(JIJFD)J")?;
        let table = emu.read_pointer(emu.jni.env)?;
        let memory = emu.malloc(64)?;
        let va = memory + 4; // The first jlong must align this address up to 8.
        for (offset, value) in [
            (8, 0x1122_3344_5566_7788),
            (24, (-2i64) as u64),
            (32, 3.5f64.to_bits()),
            (40, 9.5f64.to_bits()),
        ] {
            emu.write_u64(memory + offset, value)?;
        }
        emu.write_pointer(memory + 16, (-7i32) as u64)?;
        let slots = emu.malloc(40)?;
        for (i, value) in [
            0x1122_3344_5566_7788,
            (-7i32) as u64,
            (-2i64) as u64,
            3.5f32.to_bits() as u64,
            9.5f64.to_bits(),
        ]
        .into_iter()
        .enumerate()
        {
            emu.write_u64(slots + i as u64 * 8, value)?;
        }
        for base in [52, 82, 132] {
            let nonvirtual = base == 82;
            let get_id = emu.read_pointer(table + if base == 132 { 113 * 4 } else { 33 * 4 })?;
            let id = emu.call_function(get_id, &[emu.jni.env, class.0, name, sig])?;
            for variant in 0..3 {
                let mut words = vec![emu.jni.env, class.0];
                if nonvirtual {
                    words.push(class.0);
                }
                words.push(id);
                if variant == 0 {
                    if !nonvirtual {
                        words.push(0xdead_beef);
                    } // r3 is skipped for jlong.
                    words.extend([
                        0x5566_7788,
                        0x1122_3344,
                        0xffff_fff9,
                        0xdead_beef,
                        0xffff_fffe,
                        0xffff_ffff,
                        0,
                        0x400c_0000,
                        0,
                        0x4023_0000,
                    ]);
                } else {
                    words.push(if variant == 1 { va } else { slots });
                }
                let args: Vec<_> = words
                    .into_iter()
                    .map(|word| JniValue::Int(word as i32))
                    .collect();
                let function = emu.read_pointer(table + (base + variant) * 4)?;
                assert_eq!(
                    emu.invoke_jni(function, &args, b'J', &mut Handler)?,
                    JniValue::Long(0x8877_6655_4433_2211u64 as i64)
                );
                assert_eq!(emu.reg(0)?, 0x4433_2211);
                assert_eq!(emu.reg(1)?, 0x8877_6655);
            }
        }
        Ok(())
    }

    #[test]
    fn arm32_native_arguments_use_even_register_pairs_and_aligned_stack_slots() -> Result<()> {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(crate::Architecture::Arm32)?;
        let code = crate::RETURN + 0x100;
        // mov r0,r2; mov r1,r3; bx lr
        emu.write_bytes(
            code,
            &[0xe1a00002u32, 0xe1a01003, 0xe12fff1e]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        assert_eq!(
            emu.invoke_jni(
                code,
                &[JniValue::Int(9), JniValue::Long(0x1122_3344_5566_7788)],
                b'J',
                &mut ()
            )?,
            JniValue::Long(0x1122_3344_5566_7788)
        );
        let args = [
            JniValue::Int(1),
            JniValue::Int(2),
            JniValue::Int(3),
            JniValue::Long(0x1122_3344_5566_7788),
            JniValue::Float(3.5),
            JniValue::Double(9.5),
            JniValue::Int(7),
        ];
        // ldr r0,[sp,#offset]; ldr r1,[sp,#offset+4]; bx lr
        for (offset, kind, expected) in [
            (0, b'J', args[3]),
            (8, b'F', args[4]),
            (16, b'D', args[5]),
            (24, b'I', args[6]),
        ] {
            emu.write_bytes(
                code,
                &[
                    0xe59d0000u32 | offset,
                    0xe59d1000 | (offset + 4),
                    0xe12fff1e,
                ]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
            )?;
            assert_eq!(emu.invoke_jni(code, &args, kind, &mut ())?, expected);
        }
        // ldr r0,[sp]; bx lr: the fifth raw argument is one truncated word.
        emu.write_bytes(
            code,
            &[0xe59d0000u32, 0xe12fff1e]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        assert_eq!(
            emu.call_function(code, &[1, 2, 3, 4, 0x1234_5678_ffff_ffff])?,
            0xffff_ffff
        );
        Ok(())
    }

    #[test]
    fn arm32_field_values_and_double_results_follow_softfp() -> Result<()> {
        struct Handler(JniValue);
        impl Handler {
            fn check(&self, call: &JniCall<'_>) -> Option<()> {
                assert_eq!(call.args, [self.0]);
                Some(())
            }
        }
        impl JniHandler for Handler {
            fn set_long_field(&mut self, _: &mut Emulator, c: &JniCall<'_>) -> Option<()> {
                self.check(c)
            }
            fn set_static_long_field(&mut self, _: &mut Emulator, c: &JniCall<'_>) -> Option<()> {
                self.check(c)
            }
            fn set_float_field(&mut self, _: &mut Emulator, c: &JniCall<'_>) -> Option<()> {
                self.check(c)
            }
            fn set_static_float_field(&mut self, _: &mut Emulator, c: &JniCall<'_>) -> Option<()> {
                self.check(c)
            }
            fn set_double_field(&mut self, _: &mut Emulator, c: &JniCall<'_>) -> Option<()> {
                self.check(c)
            }
            fn set_static_double_field(&mut self, _: &mut Emulator, c: &JniCall<'_>) -> Option<()> {
                self.check(c)
            }
            fn get_double_field(&mut self, _: &mut Emulator, _: &JniCall<'_>) -> Option<f64> {
                Some(9.5)
            }
            fn get_static_double_field(
                &mut self,
                _: &mut Emulator,
                _: &JniCall<'_>,
            ) -> Option<f64> {
                Some(9.5)
            }
        }
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(crate::Architecture::Arm32)?;
        let class = emu.class("example/Fields");
        let table = emu.read_pointer(emu.jni.env)?;
        let name = emu.new_cstring("value")?;
        for static_field in [false, true] {
            for (signature, index, value) in [
                ("J", 110, JniValue::Long(0x1122_3344_5566_7788)),
                ("F", 111, JniValue::Float(3.5)),
                ("D", 112, JniValue::Double(9.5)),
            ] {
                let signature = emu.new_cstring(signature)?;
                let get_id =
                    emu.read_pointer(table + if static_field { 144 * 4 } else { 94 * 4 })?;
                let id = emu.call_function(get_id, &[emu.jni.env, class.0, name, signature])?;
                let mut words = vec![emu.jni.env, class.0, id];
                if index != 111 {
                    words.push(0xdead_beef);
                }
                words.push(value.bits() & 0xffff_ffff);
                if index != 111 {
                    words.push(value.bits() >> 32);
                }
                let args: Vec<_> = words
                    .into_iter()
                    .map(|word| JniValue::Int(word as i32))
                    .collect();
                let setter =
                    emu.read_pointer(table + (index + if static_field { 50 } else { 0 }) * 4)?;
                emu.invoke_jni(setter, &args, b'V', &mut Handler(value))?;
                if index == 112 {
                    let getter =
                        emu.read_pointer(table + if static_field { 153 * 4 } else { 103 * 4 })?;
                    assert_eq!(
                        emu.invoke_jni(
                            getter,
                            &[Handle(emu.jni.env).into(), class.into(), Handle(id).into()],
                            b'D',
                            &mut Handler(value)
                        )?,
                        value
                    );
                    assert_eq!(emu.reg(0)?, 0);
                    assert_eq!(emu.reg(1)?, 0x4023_0000);
                }
            }
        }
        Ok(())
    }

    #[test]
    fn arm32_direct_buffers_use_a_register_pair_for_capacity() -> Result<()> {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(crate::Architecture::Arm32)?;
        let table = emu.read_pointer(emu.jni.env)?;
        let memory = emu.malloc(16)?;
        let buffer = emu.call_function(
            emu.read_pointer(table + 229 * 4)?,
            &[emu.jni.env, memory, 0x89ab_cdef, 0x1234_5678],
        )?;
        let capacity = emu.read_pointer(table + 231 * 4)?;
        assert_eq!(
            emu.invoke_jni(
                capacity,
                &[Handle(emu.jni.env).into(), Handle(buffer).into()],
                b'J',
                &mut ()
            )?,
            JniValue::Long(0x1234_5678_89ab_cdef)
        );
        assert_eq!(emu.reg(0)?, 0x89ab_cdef);
        assert_eq!(emu.reg(1)?, 0x1234_5678);
        Ok(())
    }

    #[test]
    fn arm32_vm_pointers_write_four_bytes_and_native_entries_keep_thumb_bits() -> Result<()> {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(crate::Architecture::Arm32)?;
        let env_table = emu.read_pointer(emu.jni.env)?;
        let vm_table = emu.read_pointer(emu.jni.vm)?;
        let out = emu.malloc(8)?;
        for index in [4, 6, 7] {
            emu.write_bytes(out, &[0xa5; 8])?;
            let function = emu.read_pointer(vm_table + index * 4)?;
            assert_eq!(emu.call_function(function, &[emu.jni.vm, out, 0x10006])?, 0);
            assert_eq!(emu.read_pointer(out)?, emu.jni.env);
            assert_eq!(emu.read_bytes(out + 4, 4)?, [0xa5; 4]);
        }
        emu.write_bytes(out, &[0xa5; 8])?;
        let get_vm = emu.read_pointer(env_table + 219 * 4)?;
        assert_eq!(emu.call_function(get_vm, &[emu.jni.env, out])?, 0);
        assert_eq!(emu.read_pointer(out)?, emu.jni.vm);
        assert_eq!(emu.read_bytes(out + 4, 4)?, [0xa5; 4]);

        let class = emu.class("example/Native");
        let entries = emu.malloc(24)?;
        let sig = emu.new_cstring("()I")?;
        let code = crate::RETURN + 0x100;
        // Thumb: movs r0,#42; bx lr; movs r0,#7; bx lr.
        emu.write_bytes(code, &[0x2a, 0x20, 0x70, 0x47, 7, 0x20, 0x70, 0x47])?;
        for (i, name) in ["one", "two"].into_iter().enumerate() {
            let name = emu.new_cstring(name)?;
            let entry = entries + i as u64 * 12;
            emu.write_pointer(entry, name)?;
            emu.write_pointer(entry + 4, sig)?;
            emu.write_pointer(entry + 8, (code + i as u64 * 4) | 1)?;
        }
        let register = emu.read_pointer(env_table + 215 * 4)?;
        assert_eq!(
            emu.call_function(register, &[emu.jni.env, class.0, entries, 2])?,
            0
        );
        assert_eq!(
            emu.call_static_jni_method(&mut (), "example/Native", "one()I", &[])?
                .int(),
            42
        );
        assert_eq!(
            emu.call_static_jni_method(&mut (), "example/Native", "two()I", &[])?
                .int(),
            7
        );
        Ok(())
    }

    #[test]
    fn string_regions_use_a_fifth_argument_and_signed_jni_lengths() -> Result<()> {
        for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
            let mut emu = Emulator::builder().build()?;
            emu.setup_cpu(arch)?;
            let table = emu.read_pointer(emu.jni.env)?;
            let string = emu.new_string("aéz");
            let out = emu.malloc(8)?;
            for (index, expected) in [(220, [0xe9, 0]), (221, [0xc3, 0xa9])] {
                emu.write_bytes(out, &[0xa5; 8])?;
                let function = emu.read_pointer(table + index * emu.pointer_size())?;
                emu.call_function(function, &[emu.jni.env, string.0, 1, 1, out])?;
                assert_eq!(emu.read_bytes(out, 2)?, expected);
                assert_eq!(emu.read_bytes(out + 2, 6)?, [0xa5; 6]);
            }
            let new_string = emu.read_pointer(table + 163 * emu.pointer_size())?;
            assert_eq!(
                emu.call_function(new_string, &[emu.jni.env, 0, u64::MAX])?,
                0
            );
            let check = emu.read_pointer(table + 228 * emu.pointer_size())?;
            assert_eq!(emu.call_function(check, &[emu.jni.env])?, 1);
        }
        Ok(())
    }

    #[test]
    fn handles_wrap_without_reusing_live_or_remembered_class_ids() -> Result<()> {
        for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
            let mut emu = Emulator::builder().build()?;
            emu.setup_cpu(arch)?;
            let live = emu.new_string("live");
            let class = emu.class("example/Remembered");
            emu.release(class);
            let free = emu.new_string("released");
            emu.release(free);
            emu.jni.next_handle = 0x7fff_fffe;
            assert_eq!(emu.new_string("last"), Handle(0x7fff_ffff));
            assert_eq!(emu.new_string("wrapped"), free);
            assert_eq!(emu.string(live), "live");
            assert_eq!(emu.class("example/Remembered"), class);

            let table = emu.read_pointer(emu.jni.env)?;
            let name = emu.new_cstring("field")?;
            let signature = emu.new_cstring("J")?;
            let id = emu.call_function(
                emu.read_pointer(table + 94 * emu.pointer_size())?,
                &[emu.jni.env, class.0, name, signature],
            )?;
            assert_eq!(id, 0x8000_0000);
            let push = emu.read_pointer(table + 19 * emu.pointer_size())?;
            let pop = emu.read_pointer(table + 20 * emu.pointer_size())?;
            emu.call_function(push, &[emu.jni.env, 16])?;
            emu.jni.next_handle = 0x7fff_ffff;
            let result = emu.call_function(pop, &[emu.jni.env, live.0])?;
            assert_eq!(result, 4);
            assert_eq!(emu.string(Handle(result)), "live");
        }
        Ok(())
    }

    #[test]
    fn jni_a_and_v_and_variadic_calls_decode_the_same_arguments() -> Result<()> {
        struct Handler {
            array: Handle,
        }
        impl JniHandler for Handler {
            fn call_static_double_method(
                &mut self,
                _: &mut Emulator,
                call: &JniCall<'_>,
            ) -> Option<f64> {
                assert_eq!(call.class, "example/Math");
                assert_eq!(
                    call.args,
                    [
                        JniValue::Int(-7),
                        self.array.into(),
                        JniValue::Float(3.5),
                        JniValue::Long(-2)
                    ]
                );
                Some(9.5)
            }
        }
        for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
            let mut emu = Emulator::builder().build()?;
            emu.setup_cpu(arch)?;
            let p = emu.pointer_size();
            let class = emu.class("example/Math");
            let array = emu.new_byte_array(&[1, 2]);
            let name = emu.new_cstring("calculate")?;
            let sig = emu.new_cstring("(I[BFJ)D")?;
            let table = emu.read_pointer(emu.jni.env)?;
            let id_function = emu.read_pointer(table + 113 * p)?;
            let id = emu.call_function(id_function, &[emu.jni.env, class.0, name, sig])?;
            let slots = emu.malloc(32)?;
            for (i, bits) in [
                (-7i32) as u64,
                array.0,
                3.5f32.to_bits() as u64,
                (-2i64) as u64,
            ]
            .iter()
            .enumerate()
            {
                emu.write_u64(slots + i as u64 * 8, *bits)?;
            }
            let va = if arch == crate::Architecture::Arm32 {
                let va = emu.malloc(24)?;
                emu.write_pointer(va, (-7i32) as u64)?;
                emu.write_pointer(va + 4, array.0)?;
                emu.write_u64(va + 8, 3.5f64.to_bits())?;
                emu.write_u64(va + 16, (-2i64) as u64)?;
                va
            } else {
                let saved = emu.malloc(40)?;
                emu.write_u64(saved, (-7i32) as u64)?;
                emu.write_u64(saved + 8, array.0)?;
                emu.write_u64(saved + 16, (-2i64) as u64)?;
                emu.write_u64(saved + 24, 3.5f64.to_bits())?;
                let va = emu.malloc(32)?;
                emu.write_u64(va, 0)?;
                emu.write_u64(va + 8, saved + 24)?;
                emu.write_u64(va + 16, saved + 40)?;
                emu.write_bytes(va + 24, &(-24i32).to_le_bytes())?;
                emu.write_bytes(va + 28, &(-16i32).to_le_bytes())?;
                va
            };
            for variant in 0..3 {
                let function = emu.read_pointer(table + (138 + variant) * p)?;
                let mut args = vec![
                    JniValue::Object(Handle(emu.jni.env)),
                    class.into(),
                    JniValue::Object(Handle(id)),
                ];
                if variant == 0 {
                    args.extend([
                        (-7).into(),
                        array.into(),
                        JniValue::Double(3.5),
                        JniValue::Long(-2),
                    ]);
                } else {
                    args.push(JniValue::Object(Handle(if variant == 1 {
                        va
                    } else {
                        slots
                    })));
                }
                assert_eq!(
                    emu.invoke_jni(function, &args, b'D', &mut Handler { array })?,
                    JniValue::Double(9.5)
                );
            }
        }
        Ok(())
    }

    #[test]
    fn api_handles_created_by_a_handler_live_until_release() -> Result<()> {
        #[derive(Default)]
        struct Handler(Handle);
        impl JniHandler for Handler {
            fn call_static_void_method(
                &mut self,
                emu: &mut Emulator,
                _: &JniCall<'_>,
            ) -> Option<()> {
                self.0 = emu.new_object("example/Value", 42u32);
                Some(())
            }
        }
        for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
            let mut emu = Emulator::builder().build()?;
            emu.setup_cpu(arch)?;
            let p = emu.pointer_size();
            let class = emu.class("example/Factory");
            let name = emu.new_cstring("create")?;
            let sig = emu.new_cstring("()V")?;
            let table = emu.read_pointer(emu.jni.env)?;
            let get_id = emu.read_pointer(table + 113 * p)?;
            let method = emu.call_function(get_id, &[emu.jni.env, class.0, name, sig])?;
            let function = emu.read_pointer(table + 141 * p)?;
            let mut handler = Handler::default();
            emu.invoke_jni(
                function,
                &[
                    JniValue::Object(Handle(emu.jni.env)),
                    class.into(),
                    JniValue::Object(Handle(method)),
                ],
                b'V',
                &mut handler,
            )?;
            assert_eq!(*emu.user_value::<u32>(handler.0).unwrap(), 42);
            emu.release(handler.0);
            let reference_type = emu.read_pointer(table + 232 * p)?;
            assert_eq!(
                emu.call_function(reference_type, &[emu.jni.env, handler.0.0])?,
                0
            );
        }
        Ok(())
    }

    #[test]
    fn functions_taking_pointers_or_array_references_are_not_rejected() -> Result<()> {
        for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
            let (mut emu, _) = libc_emulator(arch)?;
            let p = emu.pointer_size();
            let table = emu.read_pointer(emu.jni.env)?;
            let text = emu.new_cstring("hello")?;
            let string =
                emu.call_function(emu.read_pointer(table + 167 * p)?, &[emu.jni.env, text])?;
            assert_eq!(emu.string(Handle(string)), "hello");
            let array = emu.new_byte_array(&[1, 2, 3]);
            let ptr = emu.call_function(
                emu.read_pointer(table + 222 * p)?,
                &[emu.jni.env, array.0, 0],
            )?;
            emu.write_bytes(ptr, &[9])?;
            emu.call_function(
                emu.read_pointer(table + 223 * p)?,
                &[emu.jni.env, array.0, ptr, 0],
            )?;
            assert_eq!(emu.byte_array(array), [9, 2, 3]);
            let memory = emu.malloc(16)?;
            let buffer = emu
                .invoke_jni(
                    emu.read_pointer(table + 229 * p)?,
                    &[
                        Handle(emu.jni.env).into(),
                        Handle(memory).into(),
                        JniValue::Long(16),
                    ],
                    b'L',
                    &mut (),
                )?
                .object()
                .0;
            assert_eq!(
                emu.call_function(emu.read_pointer(table + 230 * p)?, &[emu.jni.env, buffer])?,
                memory
            );
            assert_eq!(
                emu.call_function(emu.read_pointer(table + 231 * p)?, &[emu.jni.env, buffer])?,
                16
            );
        }
        Ok(())
    }

    #[test]
    fn class_identity_survives_local_reference_cleanup() -> Result<()> {
        for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
            let mut emu = Emulator::builder().build()?;
            emu.setup_cpu(arch)?;
            let p = emu.pointer_size();
            emu.jni.scope = 2;
            let local = emu.class("example/Test");
            let global = emu.alias(local, 1);
            emu.release(local);
            let again = emu.class("example/Test");
            let table = emu.read_pointer(emu.jni.env)?;
            let is_same = emu.read_pointer(table + 24 * p)?;
            assert_eq!(
                emu.call_function(is_same, &[emu.jni.env, again.0, global.0])?,
                1
            );
        }
        Ok(())
    }

    #[test]
    fn primitive_arrays_commit_abort_and_regions_use_element_width() -> Result<()> {
        for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
            let (mut emu, _) = libc_emulator(arch)?;
            let p = emu.pointer_size();
            let table = emu.read_pointer(emu.jni.env)?;
            for (index, kind) in ARRAY_TYPES.iter().enumerate() {
                let width = kind.size();
                let array = emu.new_primitive_array(*kind, &vec![0; 3 * width]);
                let get = emu.read_pointer(table + (183 + index as u64) * p)?;
                let release = emu.read_pointer(table + (191 + index as u64) * p)?;
                let ptr = emu.call_function(get, &[emu.jni.env, array.0, 0])?;
                emu.write_bytes(ptr, &vec![0x42; 3 * width])?;
                emu.call_function(release, &[emu.jni.env, array.0, ptr, 1])?;
                assert_eq!(emu.primitive_array(array), vec![0x42; 3 * width]);
                emu.write_bytes(ptr, &vec![0x55; 3 * width])?;
                emu.call_function(release, &[emu.jni.env, array.0, ptr, 2])?;
                assert_eq!(emu.primitive_array(array), vec![0x42; 3 * width]);
                let set_region = emu.read_pointer(table + (207 + index as u64) * p)?;
                let region = emu.malloc(width)?;
                emu.write_bytes(region, &vec![0x55; width])?;
                emu.call_function(set_region, &[emu.jni.env, array.0, 1, 1, region])?;
                emu.free(region);
                let mut expected = vec![0x42; 3 * width];
                expected[width..2 * width].fill(0x55);
                assert_eq!(emu.primitive_array(array), expected);
                let get_region = emu.read_pointer(table + (199 + index as u64) * p)?;
                let out = emu.malloc(width + 4)?;
                emu.write_bytes(out, &vec![0xa5; width + 4])?;
                emu.call_function(get_region, &[emu.jni.env, array.0, 1, 1, out])?;
                assert_eq!(emu.read_bytes(out, width)?, vec![0x55; width]);
                assert_eq!(emu.read_bytes(out + width as u64, 4)?, [0xa5; 4]);
                let ptr = emu.call_function(get, &[emu.jni.env, array.0, 0])?;
                emu.write_bytes(ptr, &vec![0x66; 3 * width])?;
                emu.call_function(release, &[emu.jni.env, array.0, ptr, 0])?;
                assert_eq!(emu.primitive_array(array), vec![0x66; 3 * width]);
            }
        }
        Ok(())
    }

    #[test]
    fn modified_utf8_preserves_nul_and_surrogate_pairs() {
        let units = [0, 0x41, 0xd83d, 0xde00];
        let expected = [0xc0, 0x80, 0x41, 0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80];
        assert_eq!(modified_utf8(&units), expected);
        assert_eq!(decode_modified_utf8(&expected), units);
    }

    #[test]
    fn object_arrays_exceptions_and_bad_ids_are_safe() -> Result<()> {
        for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
            let mut emu = Emulator::builder().build()?;
            emu.setup_cpu(arch)?;
            let p = emu.pointer_size();
            let table = emu.read_pointer(emu.jni.env)?;
            let class = emu.class("java/lang/String");
            let initial = emu.new_string("x");
            let array = emu.call_function(
                emu.read_pointer(table + 172 * p)?,
                &[emu.jni.env, 3, class.0, initial.0],
            )?;
            assert_eq!(
                emu.call_function(emu.read_pointer(table + 171 * p)?, &[emu.jni.env, array])?,
                3
            );
            let second = emu.new_string("y");
            emu.call_function(
                emu.read_pointer(table + 174 * p)?,
                &[emu.jni.env, array, 1, second.0],
            )?;
            let element =
                emu.call_function(emu.read_pointer(table + 173 * p)?, &[emu.jni.env, array, 1])?;
            assert_eq!(emu.string(Handle(element)), "y");
            let throw_new = emu.read_pointer(table + 14 * p)?;
            let message = emu.new_cstring("bad input")?;
            assert_eq!(
                emu.call_function(throw_new, &[emu.jni.env, class.0, message])?,
                0
            );
            assert_eq!(
                emu.call_function(emu.read_pointer(table + 228 * p)?, &[emu.jni.env])?,
                1
            );
            let exception = emu.call_function(emu.read_pointer(table + 15 * p)?, &[emu.jni.env])?;
            assert_ne!(exception, 0);
            emu.call_function(emu.read_pointer(table + 17 * p)?, &[emu.jni.env])?;
            assert_eq!(
                emu.call_function(emu.read_pointer(table + 228 * p)?, &[emu.jni.env])?,
                0
            );
            let invalid = emu
                .call_function(emu.read_pointer(table + 171 * p)?, &[emu.jni.env, 0])
                .unwrap_err()
                .to_string();
            assert!(invalid.starts_with("invalid JNI reference 0x0 in GetArrayLength"));
            let invalid_id = emu
                .call_function(
                    emu.read_pointer(table + 141 * p)?,
                    &[emu.jni.env, class.0, 0x1234],
                )
                .unwrap_err()
                .to_string();
            assert!(invalid_id.starts_with("invalid JNI id 0x1234 in CallStaticVoidMethod"));
            let weak =
                emu.call_function(emu.read_pointer(table + 226 * p)?, &[emu.jni.env, class.0])?;
            assert_eq!(
                emu.call_function(emu.read_pointer(table + 232 * p)?, &[emu.jni.env, weak])?,
                3
            );
        }
        Ok(())
    }

    #[test]
    fn unsupported_jni_functions_have_stable_errors() -> Result<()> {
        for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
            let mut emu = Emulator::builder().build()?;
            emu.setup_cpu(arch)?;
            let p = emu.pointer_size();
            let table = emu.read_pointer(emu.jni.env)?;
            let error = emu
                .call_function(emu.read_pointer(table + 5 * p)?, &[emu.jni.env, 0, 0, 0])
                .unwrap_err()
                .to_string();
            assert!(error.starts_with("unsupported JNIEnv function DefineClass"));
        }
        Ok(())
    }

    #[test]
    fn signature_counts_arrays_and_classes_as_one_argument() {
        assert_eq!(argument_types("([B[[ILjava/lang/String;JFD)V"), b"[[LJFD");
        assert_eq!(decode(b'B', 255), JniValue::Byte(-1));
        assert_eq!(decode(b'I', 0xffff_ffff), JniValue::Int(-1));
    }
}
