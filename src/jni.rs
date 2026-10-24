mod android;
mod calls;
mod env;
mod handler;
mod java;
mod names;
mod objects;
mod value;

pub use handler::{JniCall, JniHandler};
pub use value::{Handle, JniValue};

use std::{
    any::Any,
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::{Rc, Weak},
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
#[derive(Clone)]
enum Object {
    Android(android::Android),
    Class(String),
    String(Vec<u16>),
    Boxed(JniValue),
    StringBuilder(Vec<u16>),
    Charset(java::Charset),
    Digest(Box<java::Digest>),
    Map(java::Map),
    Set(java::Map),
    List(String, Vec<java::ObjectRef>),
    View(String, Vec<java::ObjectRef>),
    Entry(String, java::ObjectRef, java::ObjectRef),
    Iterator(String, Vec<java::ObjectRef>, usize),
    Array(PrimitiveType, Vec<u8>),
    User(String, Rc<dyn Any>),
    ObjectArray(Vec<Option<Rc<RefCell<Object>>>>, String),
    DirectBuffer(u64, i64),
    Exception(String, Option<String>),
}
impl Object {
    fn is_container(&self) -> bool {
        matches!(
            self,
            Self::Map(_)
                | Self::Set(_)
                | Self::List(..)
                | Self::View(..)
                | Self::Entry(..)
                | Self::Iterator(..)
                | Self::ObjectArray(..)
        )
    }
    fn references(&mut self, mut visit: impl FnMut(&mut java::ObjectRef)) {
        match self {
            Self::ObjectArray(items, _)
            | Self::List(_, items)
            | Self::View(_, items)
            | Self::Iterator(_, items, _) => items.iter_mut().for_each(visit),
            Self::Map(map) | Self::Set(map) => map.references(visit),
            Self::Entry(_, key, value) => {
                visit(key);
                visit(value);
            }
            _ => {}
        }
    }
    fn java_type(&self) -> String {
        match self {
            Self::Android(value) => value.class().into(),
            Self::Class(_) => "java/lang/Class".into(),
            Self::String(_) => "java/lang/String".into(),
            Self::Boxed(value) => java::boxed_class(*value).unwrap().into(),
            Self::StringBuilder(_) => "java/lang/StringBuilder".into(),
            Self::Charset(_) => "java/nio/charset/Charset".into(),
            Self::Digest(_) => "java/security/MessageDigest".into(),
            Self::Map(map) => map.class().into(),
            Self::Set(_) => "java/util/HashSet".into(),
            Self::List(class, _)
            | Self::View(class, _)
            | Self::Entry(class, ..)
            | Self::Iterator(class, ..) => class.clone(),
            Self::Array(kind, _) => kind.signature().into(),
            Self::User(class, _) | Self::Exception(class, _) => class.clone(),
            Self::ObjectArray(_, class) => format!("[L{class};"),
            Self::DirectBuffer(_, _) => "java/nio/ByteBuffer".into(),
        }
    }
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
#[derive(Clone)]
struct PendingException {
    handle: Handle,
    from: String,
}
#[derive(Default)]
pub(crate) struct JniState {
    application: java::ObjectRef,
    boxed: HashMap<(u8, u64), Rc<RefCell<Object>>>,
    charsets: HashMap<&'static str, Rc<RefCell<Object>>>,
    containers: Vec<Weak<RefCell<Object>>>,
    objects: HashMap<Handle, Reference>,
    classes: HashMap<String, (Handle, Rc<RefCell<Object>>)>,
    superclasses: HashMap<String, String>,
    members: Vec<Member>,
    member_ids: HashMap<(String, String, String, bool, bool), u64>,
    natives: HashMap<(String, String), u64>,
    next_handle: u64,
    identities: HashMap<*const RefCell<Object>, (Weak<RefCell<Object>>, i32)>,
    next_identity: i32,
    scope: usize,
    env: u64,
    vm: u64,
    weak: HashSet<Handle>,
    pending: Option<PendingException>,
    history: env::history::History,
}

#[cfg(test)]
mod tests;
