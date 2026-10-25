use super::{Handle, JniCall, JniValue, MEMBER_ID_BASE, Object};
use crate::{Emulator, Result};

/// The Java reflection class representing a member.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReflectedMemberKind {
    Method,
    Constructor,
    Field,
}

/// A copy of a built-in reflected member's metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReflectedMember {
    pub class: String,
    pub name: String,
    pub signature: String,
    pub is_static: bool,
    pub kind: ReflectedMemberKind,
}

impl ReflectedMemberKind {
    pub(super) fn class(self) -> &'static str {
        match self {
            Self::Method => "java/lang/reflect/Method",
            Self::Constructor => "java/lang/reflect/Constructor",
            Self::Field => "java/lang/reflect/Field",
        }
    }
}

impl Emulator {
    /// Create an API-owned Method or Constructor, sharing IDs with JNI GetMethodID/GetStaticMethodID.
    pub fn new_reflected_method(
        &mut self,
        class: &str,
        name: &str,
        signature: &str,
        is_static: bool,
    ) -> Handle {
        self.new_reflected(class, name, signature, is_static, false)
    }

    /// Create an API-owned Field, sharing IDs with JNI GetFieldID/GetStaticFieldID.
    pub fn new_reflected_field(
        &mut self,
        class: &str,
        name: &str,
        signature: &str,
        is_static: bool,
    ) -> Handle {
        self.new_reflected(class, name, signature, is_static, true)
    }

    /// Inspect a built-in Method, Constructor or Field.
    pub fn reflected_member(&self, handle: Handle) -> Result<ReflectedMember> {
        match &*self.jni_object(handle)?.borrow() {
            Object::Reflected {
                id,
                is_static,
                kind,
            } => {
                let member = &self.jni.members[(id - MEMBER_ID_BASE) as usize];
                Ok(ReflectedMember {
                    class: member.class.clone(),
                    name: member.name.clone(),
                    signature: member.signature.clone(),
                    is_static: *is_static,
                    kind: *kind,
                })
            }
            object => Err(format!(
                "invalid handle {}: {}, not a reflected member",
                handle.0,
                object.java_type()
            )
            .into()),
        }
    }

    fn new_reflected(
        &mut self,
        class: &str,
        name: &str,
        signature: &str,
        is_static: bool,
        is_field: bool,
    ) -> Handle {
        let id = self.intern_member(class, name, signature, is_static, is_field);
        self.insert_api_object(self.reflected_object(id, is_static))
    }

    fn reflected_object(&self, id: u64, is_static: bool) -> Object {
        Object::Reflected {
            id,
            is_static,
            kind: self.jni.members[(id - MEMBER_ID_BASE) as usize].kind,
        }
    }

    pub(super) fn reflected_call(
        &mut self,
        call: &JniCall<'_>,
        id: u64,
    ) -> Result<Option<JniValue>> {
        let member = &self.jni.members[(id - MEMBER_ID_BASE) as usize];
        Ok(Some(match (call.name, call.signature) {
            ("getName", "()Ljava/lang/String;") => {
                let name = if member.kind == ReflectedMemberKind::Constructor {
                    member.class.replace('/', ".")
                } else {
                    member.name.clone()
                };
                super::java::string(self, name.encode_utf16().collect::<Vec<_>>())
            }
            ("getDeclaringClass", "()Ljava/lang/Class;") => {
                let class = member.class.clone();
                self.find_class(&class).into()
            }
            _ => return Ok(None),
        }))
    }

    pub(super) fn jni_to_reflected(
        &mut self,
        function: &str,
        id: u64,
        is_static: bool,
    ) -> Result<u64> {
        let member = self.require_member(function, id)?;
        if (member.kind == ReflectedMemberKind::Field) != (function == "ToReflectedField") {
            return Err(self.member_error(function, id));
        }
        Ok(self.insert_object(self.reflected_object(id, is_static)).0)
    }

    pub(super) fn jni_from_reflected(&self, function: &str, handle: u64) -> Result<u64> {
        let object = self
            .jni
            .objects
            .get(&Handle(handle))
            .map(|r| r.object.borrow());
        if let Some(Object::Reflected { id, kind, .. }) = object.as_deref()
            && (*kind == ReflectedMemberKind::Field) == (function == "FromReflectedField")
        {
            return Ok(*id);
        }
        let class = object.as_ref().map_or_else(
            || {
                if handle == 0 {
                    "null"
                } else {
                    "invalid reference"
                }
                .into()
            },
            |object| object.java_type(),
        );
        Err(format!(
            "invalid reflected member 0x{handle:x}: {class} in {function} from {}",
            self.location(self.lr().unwrap_or(0))
        )
        .into())
    }
}
