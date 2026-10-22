mod calls;
mod env;
mod handler;
mod names;
mod objects;
mod value;

pub use handler::{JniCall, JniHandler};
pub use value::{Handle, JniValue};

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
impl Object {
    fn java_type(&self) -> String {
        match self {
            Self::Class(_) => "java/lang/Class".into(),
            Self::String(_) => "java/lang/String".into(),
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
#[derive(Default)]
pub(crate) struct JniState {
    objects: HashMap<Handle, Reference>,
    classes: HashMap<String, (Handle, Rc<RefCell<Object>>)>,
    superclasses: HashMap<String, String>,
    members: Vec<Member>,
    member_ids: HashMap<(String, String, String, bool, bool), u64>,
    natives: HashMap<(String, String), u64>,
    next_handle: u64,
    scope: usize,
    env: u64,
    vm: u64,
    weak: HashSet<Handle>,
    pending: Option<Handle>,
    history: env::history::History,
}

#[cfg(test)]
mod tests;
