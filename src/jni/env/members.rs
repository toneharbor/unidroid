use crate::jni::handler::check_answer;
use crate::jni::names::member_function;
use crate::jni::value::{decode, field_type};
use crate::jni::{Handle, JniCall, JniHandler, JniOp, MEMBER_ID_BASE, Member, ReflectedMemberKind};
use crate::{Emulator, Result};

pub(super) fn member_id(index: usize, b: u64, c: u64) -> Option<u64> {
    match index {
        9 | 12 => Some(b),
        _ => member_function(index).map(|member| if member.member_argument == 3 { c } else { b }),
    }
}

impl Emulator {
    pub(in crate::jni) fn intern_member(
        &mut self,
        class: &str,
        name: &str,
        signature: &str,
        is_static: bool,
        is_field: bool,
    ) -> u64 {
        let key = (
            class.into(),
            name.into(),
            signature.into(),
            is_static,
            is_field,
        );
        if let Some(id) = self.jni.member_ids.get(&key) {
            return *id;
        }
        let id = self.jni.members.len() as u64 + MEMBER_ID_BASE;
        self.jni.members.push(Member {
            class: class.into(),
            name: name.into(),
            signature: signature.into(),
            kind: if is_field {
                ReflectedMemberKind::Field
            } else if name == "<init>" {
                ReflectedMemberKind::Constructor
            } else {
                ReflectedMemberKind::Method
            },
        });
        self.jni.member_ids.insert(key, id);
        id
    }

    pub(super) fn jni_members_function(
        &mut self,
        index: usize,
        function: String,
        a: u64,
        b: u64,
        c: u64,
        handler: &mut dyn JniHandler,
    ) -> Result<u64> {
        match index {
            33 | 94 | 113 | 144 => {
                let class = self.class_name(Handle(a))?;
                let name = self.cstring(b)?;
                let signature = self.cstring(c)?;
                Ok(self.intern_member(
                    &class,
                    &name,
                    &signature,
                    index >= 113,
                    index == 94 || index == 144,
                ))
            }
            _ if member_function(index).is_some() => {
                let decoded = member_function(index).unwrap();
                let member = self.require_member(&function, member_id(index, b, c).unwrap())?;
                let field = if matches!(decoded.op, JniOp::Get | JniOp::Set) {
                    Some(
                        field_type(&member.signature)
                            .ok_or_else(|| self.signature_error(&function, &member.signature))?,
                    )
                } else {
                    None
                };
                let args = match decoded.op {
                    JniOp::Set => {
                        let kind = field.unwrap();
                        let bits = self.argument(3, kind)?;
                        vec![decode(kind, bits)]
                    }
                    JniOp::Get => Vec::new(),
                    _ => self.jni_arguments(
                        &function,
                        &member.signature,
                        decoded.variant,
                        if decoded.nonvirtual { 4 } else { 3 },
                    )?,
                };
                let call = JniCall {
                    op: decoded.op,
                    is_static: decoded.is_static,
                    type_code: decoded.type_code(),
                    class: &member.class,
                    name: &member.name,
                    signature: &member.signature,
                    receiver: Handle(a),
                    args: &args,
                };
                if self.trace.jni {
                    self.events.borrow_mut().emit(crate::Event::JniCall {
                        function: function.clone(),
                        class: call.class.into(),
                        name: call.name.into(),
                        signature: call.signature.into(),
                        args: call.args.iter().map(|arg| format!("{arg:?}")).collect(),
                        from: self.location(self.lr()?),
                    });
                }
                let result = match crate::jni::java::dispatch(self, &call, &function, handler)? {
                    Some(value) => Some(value),
                    None => {
                        let answer = handler.member(self, &call);
                        check_answer(self, &call, &function, answer)?
                    }
                };
                let value = result.ok_or_else(|| {
                    self.jni_error(&function, &member.class, &member.name, &member.signature)
                })?;
                self.write_jni_result(value)?;
                Ok(value.bits())
            }
            _ => unreachable!(),
        }
    }
}
