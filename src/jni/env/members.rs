use crate::jni::handler::dispatch;
use crate::jni::value::decode;
use crate::jni::{Handle, JniCall, JniHandler, JniValue, MEMBER_ID_BASE, Member, Object};
use crate::{Emulator, Result};

pub(super) fn member_id(index: usize, b: u64, c: u64) -> Option<u64> {
    match index {
        64..=93 => Some(c),
        9 | 12 | 28..=30 | 34..=63 | 95..=112 | 114..=143 | 145..=162 => Some(b),
        _ => None,
    }
}

impl Emulator {
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
                let member = self.require_member(&function, member_id(index, b, c).unwrap())?;
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
                let result = if constructor {
                    if member.class == "java/lang/String"
                        && member.signature == "([BLjava/lang/String;)V"
                    {
                        self.validate_ref(&function, args[0].object().0, 3)?;
                        self.validate_ref(&function, args[1].object().0, 2)?;
                        let bytes = self.byte_array(args[0].object())?;
                        let charset = self
                            .string(args[1].object())?
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
            _ => unreachable!(),
        }
    }
}
