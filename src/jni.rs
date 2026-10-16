use crate::{Emulator, HostFunction, Module, RETURN, Reg, Result};
use std::{any::Any, cell::RefCell, collections::HashMap, rc::Rc};

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
enum Object {
    Class(String),
    String(Vec<u16>),
    Array(PrimitiveType, Vec<u8>),
    User(String, Rc<dyn Any>),
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
    buffers: HashMap<u64, usize>,
    free_buffers: Vec<(u64, usize)>,
}

impl Emulator {
    fn insert_object(&mut self, object: Object) -> Handle {
        self.jni.next_handle += 1;
        let handle = Handle(self.jni.next_handle);
        self.jni.objects.insert(
            handle,
            Reference {
                object: Rc::new(RefCell::new(object)),
                scope: self.jni.scope,
            },
        );
        handle
    }
    fn insert_api_object(&mut self, object: Object) -> Handle {
        let handle = self.insert_object(object);
        self.jni.objects.get_mut(&handle).unwrap().scope = 0;
        handle
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
        }
    }
    fn alias(&mut self, handle: Handle, scope: usize) -> Handle {
        if handle.0 == 0 {
            return handle;
        }
        let object = self.object(handle);
        self.jni.next_handle += 1;
        let handle = Handle(self.jni.next_handle);
        self.jni.objects.insert(handle, Reference { object, scope });
        handle
    }
    pub(crate) fn setup_jni(&mut self) -> Result<()> {
        let table = self.malloc(233 * 8)?;
        for index in 0..233 {
            let stub = self.stub(HostFunction::Jni(index))?;
            self.write_u64(table + index as u64 * 8, stub)?;
        }
        self.jni.env = self.malloc(8)?;
        self.write_u64(self.jni.env, table)?;
        let table = self.malloc(8 * 8)?;
        for index in 0..8 {
            let stub = self.stub(HostFunction::Vm(index))?;
            self.write_u64(table + index as u64 * 8, stub)?;
        }
        self.jni.vm = self.malloc(8)?;
        self.write_u64(self.jni.vm, table)?;
        Ok(())
    }
    pub fn call_jni_onload(&mut self, module: Module, handler: &mut dyn JniHandler) -> Result<u64> {
        let function = self.find_symbol(module, "JNI_OnLoad")?;
        self.invoke_jni(
            function,
            &[JniValue::Long(self.jni.vm as i64), JniValue::Long(0)],
            b'J',
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
        let mut values = vec![JniValue::Long(self.jni.env as i64), receiver.into()];
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
        let sp = self.uc.reg_read(Reg::SP)?;
        let mut gp = 0;
        let mut fp = 0;
        let mut stack = Vec::new();
        for &value in args {
            let floating = matches!(value, JniValue::Float(_) | JniValue::Double(_));
            if floating && fp < 8 {
                self.uc.reg_write(Reg::D0 as i32 + fp, value.bits())?;
                fp += 1;
            } else if !floating && gp < 8 {
                self.uc.reg_write(Reg::X0 as i32 + gp, value.bits())?;
                gp += 1;
            } else {
                stack.push(value.bits());
            }
        }
        let call_sp = (sp - stack.len() as u64 * 8) & !15;
        for (i, value) in stack.iter().enumerate() {
            self.write_u64(call_sp + i as u64 * 8, *value)?;
        }
        self.uc.reg_write(Reg::SP, call_sp)?;
        self.uc.reg_write(Reg::LR, RETURN)?;
        let result = self.run(function, handler).and_then(|bits| {
            Ok(decode(
                kind,
                if kind == b'F' || kind == b'D' {
                    self.uc.reg_read(Reg::D0)?
                } else {
                    bits
                },
            ))
        });
        self.uc.reg_write(Reg::SP, sp)?;
        if let Ok(JniValue::Object(handle)) = result
            && let Some(reference) = self.jni.objects.get_mut(&handle)
        {
            reference.scope = old_scope;
        }
        self.jni
            .objects
            .retain(|_, reference| reference.scope <= old_scope.max(1));
        self.jni.scope = old_scope;
        result
    }
    fn scratch(&mut self, bytes: &[u8]) -> Result<u64> {
        let (ptr, size) = if let Some(i) = self
            .jni
            .free_buffers
            .iter()
            .position(|(_, size)| *size >= bytes.len())
        {
            self.jni.free_buffers.swap_remove(i)
        } else {
            (self.malloc(bytes.len())?, bytes.len())
        };
        self.write_bytes(ptr, bytes)?;
        self.jni.buffers.insert(ptr, size);
        Ok(ptr)
    }
    fn release_scratch(&mut self, ptr: u64) {
        if let Some(size) = self.jni.buffers.remove(&ptr) {
            self.jni.free_buffers.push((ptr, size));
        }
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
            self.location(self.uc.reg_read(Reg::LR).unwrap_or(0))
        )
        .into()
    }
    fn jni_arguments(
        &self,
        signature: &str,
        variant: usize,
        start: usize,
    ) -> Result<Vec<JniValue>> {
        let mut gp = start;
        let mut fp = 0;
        let mut stack = self.uc.reg_read(Reg::SP)?;
        let ptr = if variant != 0 { self.reg(start)? } else { 0 };
        let mut gr_top = 0;
        let mut vr_top = 0;
        let mut gr_off = 0i32;
        let mut vr_off = 0i32;
        if variant == 1 {
            stack = self.read_u64(ptr)?;
            gr_top = self.read_u64(ptr + 8)?;
            vr_top = self.read_u64(ptr + 16)?;
            gr_off = i32::from_le_bytes(self.read_bytes(ptr + 24, 4)?.try_into().unwrap());
            vr_off = i32::from_le_bytes(self.read_bytes(ptr + 28, 4)?.try_into().unwrap());
        }
        let mut args = Vec::new();
        for (i, kind) in argument_types(signature).into_iter().enumerate() {
            let floating = kind == b'F' || kind == b'D';
            let bits = if variant == 2 {
                self.read_u64(ptr + i as u64 * 8)?
            } else if variant == 1 {
                if floating && vr_off < 0 {
                    let p = vr_top.wrapping_add_signed(vr_off as i64);
                    vr_off += 16;
                    self.read_u64(p)?
                } else if !floating && gr_off < 0 {
                    let p = gr_top.wrapping_add_signed(gr_off as i64);
                    gr_off += 8;
                    self.read_u64(p)?
                } else {
                    let value = self.read_u64(stack)?;
                    stack += 8;
                    value
                }
            } else if floating && fp < 8 {
                let value = self.uc.reg_read(Reg::D0 as i32 + fp)?;
                fp += 1;
                value
            } else if !floating && gp < 8 {
                let value = self.reg(gp)?;
                gp += 1;
                value
            } else {
                let value = self.read_u64(stack)?;
                stack += 8;
                value
            };
            args.push(if kind == b'F' && variant != 2 {
                JniValue::Float(f64::from_bits(bits) as f32)
            } else {
                decode(kind, bits)
            });
        }
        Ok(args)
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
                self.location(self.uc.reg_read(Reg::LR)?)
            );
        }
        match index {
            4 => Ok(0x10006),
            6 => {
                let name = self.cstring(a)?;
                Ok(self.find_class(&name, self.jni.scope).0)
            }
            15 | 228 => Ok(0),
            17 | 26 => Ok(0),
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
                    self.jni.next_handle += 1;
                    let handle = Handle(self.jni.next_handle);
                    self.jni.objects.insert(
                        handle,
                        Reference {
                            object,
                            scope: self.jni.scope,
                        },
                    );
                    Ok(handle.0)
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
                let id = self.jni.members.len() as u64 + 0x100000000;
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
                let member = self.jni.members[(member_id - 0x100000000) as usize].clone();
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
                    let bits = if kind == b'F' || kind == b'D' {
                        self.uc.reg_read(Reg::D0)?
                    } else {
                        c
                    };
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
                        self.location(self.uc.reg_read(Reg::LR)?)
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
                if matches!(value, JniValue::Float(_) | JniValue::Double(_)) {
                    self.uc.reg_write(Reg::D0, value.bits())?;
                }
                Ok(value.bits())
            }
            163 => {
                let bytes = self.read_bytes(a, b as usize * 2)?;
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
                self.scratch(&bytes)
            }
            166 | 170 | 225 => {
                self.release_scratch(b);
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
                let object = self.object(Handle(a));
                let Object::Array(kind, bytes) = &*object.borrow() else {
                    unreachable!()
                };
                Ok((bytes.len() / kind.size()) as u64)
            }
            175..=182 => {
                let kind = ARRAY_TYPES[index - 175];
                Ok(self
                    .insert_object(Object::Array(kind, vec![0; a as usize * kind.size()]))
                    .0)
            }
            183..=190 | 222 => {
                let bytes = self.primitive_array(Handle(a));
                if b != 0 {
                    self.write_bytes(b, &[1])?;
                }
                self.scratch(&bytes)
            }
            191..=198 | 223 => {
                if c == 0 || c == 1 {
                    let object = self.object(Handle(a));
                    let mut object = object.borrow_mut();
                    let Object::Array(_, bytes) = &mut *object else {
                        unreachable!()
                    };
                    self.uc.mem_read(b, bytes)?;
                }
                if c != 1 {
                    self.release_scratch(b);
                }
                Ok(0)
            }
            199..=214 => {
                let object = self.object(Handle(a));
                let mut object = object.borrow_mut();
                let Object::Array(kind, bytes) = &mut *object else {
                    unreachable!()
                };
                let start = b as usize * kind.size();
                let length = c as usize * kind.size();
                let ptr = self.reg(4)?;
                if index < 207 {
                    self.write_bytes(ptr, &bytes[start..start + length])?;
                } else {
                    self.uc.mem_read(ptr, &mut bytes[start..start + length])?;
                }
                Ok(0)
            }
            215 => {
                let class = self.class_name(Handle(a));
                for i in 0..c {
                    let entry = b + i * 24;
                    let name = self.cstring(self.read_u64(entry)?)?;
                    let signature = self.cstring(self.read_u64(entry + 8)?)?;
                    let ptr = self.read_u64(entry + 16)?;
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
                self.write_u64(a, self.jni.vm)?;
                Ok(0)
            }
            220 | 221 => {
                let object = self.object(Handle(a));
                let Object::String(value) = &*object.borrow() else {
                    unreachable!()
                };
                let units = &value[b as usize..(b + c) as usize];
                let bytes = if index == 221 {
                    modified_utf8(units)
                } else {
                    units.iter().flat_map(|c| c.to_le_bytes()).collect()
                };
                self.write_bytes(self.reg(4)?, &bytes)?;
                Ok(0)
            }
            232 => Ok(self
                .jni
                .objects
                .get(&Handle(a))
                .map_or(0, |v| if v.scope == 1 { 2 } else { 1 })),
            _ => Err(self.jni_error(&function, "JNIEnv", "", "")),
        }
    }
    pub(crate) fn vm_function(&mut self, index: usize) -> Result<u64> {
        match index {
            4 | 6 | 7 => {
                self.write_u64(self.reg(1)?, self.jni.env)?;
                Ok(0)
            }
            5 => Ok(0),
            _ => Err(self.jni_error(
                [
                    "reserved0",
                    "reserved1",
                    "reserved2",
                    "DestroyJavaVM",
                    "AttachCurrentThread",
                    "DetachCurrentThread",
                    "GetEnv",
                    "AttachCurrentThreadAsDaemon",
                ][index],
                "JavaVM",
                "",
                "",
            )),
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
        let mut emu = Emulator::builder().build()?;
        let class = emu.class("example/Math");
        let array = emu.new_byte_array(&[1, 2]);
        let name = emu.new_cstring("calculate")?;
        let sig = emu.new_cstring("(I[BFJ)D")?;
        let table = emu.read_u64(emu.jni.env)?;
        let id_function = emu.read_u64(table + 113 * 8)?;
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
        for variant in 0..3 {
            let function = emu.read_u64(table + (138 + variant) * 8)?;
            let mut args = vec![
                JniValue::Long(emu.jni.env as i64),
                class.into(),
                JniValue::Long(id as i64),
            ];
            if variant == 0 {
                args.extend([
                    (-7).into(),
                    array.into(),
                    JniValue::Double(3.5),
                    JniValue::Long(-2),
                ]);
            } else {
                args.push(JniValue::Long(if variant == 1 { va } else { slots } as i64));
            }
            assert_eq!(
                emu.invoke_jni(function, &args, b'D', &mut Handler { array })?,
                JniValue::Double(9.5)
            );
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
        let mut emu = Emulator::builder().build()?;
        let class = emu.class("example/Factory");
        let name = emu.new_cstring("create")?;
        let sig = emu.new_cstring("()V")?;
        let table = emu.read_u64(emu.jni.env)?;
        let get_id = emu.read_u64(table + 113 * 8)?;
        let method = emu.call_function(get_id, &[emu.jni.env, class.0, name, sig])?;
        let function = emu.read_u64(table + 141 * 8)?;
        let mut handler = Handler::default();
        emu.invoke_jni(
            function,
            &[
                JniValue::Long(emu.jni.env as i64),
                class.into(),
                JniValue::Long(method as i64),
            ],
            b'V',
            &mut handler,
        )?;
        assert_eq!(*emu.user_value::<u32>(handler.0).unwrap(), 42);
        emu.release(handler.0);
        let reference_type = emu.read_u64(table + 232 * 8)?;
        assert_eq!(
            emu.call_function(reference_type, &[emu.jni.env, handler.0.0])?,
            0
        );
        Ok(())
    }

    #[test]
    fn class_identity_survives_local_reference_cleanup() -> Result<()> {
        let mut emu = Emulator::builder().build()?;
        emu.jni.scope = 2;
        let local = emu.class("example/Test");
        let global = emu.alias(local, 1);
        emu.release(local);
        let again = emu.class("example/Test");
        let table = emu.read_u64(emu.jni.env)?;
        let is_same = emu.read_u64(table + 24 * 8)?;
        assert_eq!(
            emu.call_function(is_same, &[emu.jni.env, again.0, global.0])?,
            1
        );
        Ok(())
    }

    #[test]
    fn primitive_arrays_commit_abort_and_regions_use_element_width() -> Result<()> {
        let mut emu = Emulator::builder().build()?;
        let table = emu.read_u64(emu.jni.env)?;
        for (index, kind) in ARRAY_TYPES.iter().enumerate() {
            let width = kind.size();
            let array = emu.new_primitive_array(*kind, &vec![0; 3 * width]);
            let get = emu.read_u64(table + (183 + index as u64) * 8)?;
            let release = emu.read_u64(table + (191 + index as u64) * 8)?;
            let ptr = emu.call_function(get, &[emu.jni.env, array.0, 0])?;
            emu.write_bytes(ptr, &vec![0x42; 3 * width])?;
            emu.call_function(release, &[emu.jni.env, array.0, ptr, 1])?;
            assert_eq!(emu.primitive_array(array), vec![0x42; 3 * width]);
            emu.write_bytes(ptr, &vec![0x55; 3 * width])?;
            emu.call_function(release, &[emu.jni.env, array.0, ptr, 2])?;
            assert_eq!(emu.primitive_array(array), vec![0x42; 3 * width]);
            let set_region = emu.read_u64(table + (207 + index as u64) * 8)?;
            emu.call_function(set_region, &[emu.jni.env, array.0, 1, 1, ptr])?;
            let mut expected = vec![0x42; 3 * width];
            expected[width..2 * width].fill(0x55);
            assert_eq!(emu.primitive_array(array), expected);
            let ptr = emu.call_function(get, &[emu.jni.env, array.0, 0])?;
            emu.write_bytes(ptr, &vec![0x66; 3 * width])?;
            emu.call_function(release, &[emu.jni.env, array.0, ptr, 0])?;
            assert_eq!(emu.primitive_array(array), vec![0x66; 3 * width]);
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
    fn signature_counts_arrays_and_classes_as_one_argument() {
        assert_eq!(argument_types("([B[[ILjava/lang/String;JFD)V"), b"[[LJFD");
        assert_eq!(decode(b'B', 255), JniValue::Byte(-1));
        assert_eq!(decode(b'I', 0xffff_ffff), JniValue::Int(-1));
    }
}
