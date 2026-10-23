use super::names::jni_name;
use super::{Handle, JniHandler, Object};
use crate::{Emulator, Result};
use std::rc::Rc;

mod arrays;
pub(super) mod history;
mod members;
pub(super) mod strings;

impl Emulator {
    fn jni_details(&self, index: usize, [a, b, c]: [u64; 3]) -> Result<Option<String>> {
        Ok(Some(match index {
            6 | 167 => self.cstring(a)?,
            33 | 94 | 113 | 144 => format!(
                "{}->{}{}",
                self.class_name(Handle(a))?,
                self.cstring(b)?,
                self.cstring(c)?
            ),
            10 | 21..=25 | 31 | 164..=166 | 168..=171 | 183..=216 | 220..=225 if a != 0 => {
                format!(
                    "{} handle=0x{a:x}, arg2=0x{b:x}, arg3=0x{c:x}",
                    self.class_name(Handle(a))?
                )
            }
            _ => return Ok(None),
        }))
    }

    pub(crate) fn jni_function(
        &mut self,
        index: usize,
        handler: &mut dyn JniHandler,
    ) -> Result<u64> {
        let a = self.reg(1)?;
        let b = self.reg(2)?;
        let c = self.reg(3)?;
        self.jni.history.record(
            index,
            self.lr()?,
            self.threads.tid(),
            members::member_id(index, b, c),
            [a, b, c],
        );
        let function = jni_name(index);
        // The kind of reference each function takes as its first argument.
        // NewStringUTF, NewDirectByteBuffer and GetObjectRefType take a
        // pointer or any value there, so they are not checked.
        let kind = match index {
            21..=25 | 226 | 227 => Some(6),
            164..=166 | 168..=170 | 220 | 221 | 224 | 225 => Some(2),
            171 => Some(5),
            183..=214 | 222 | 223 => Some(3),
            10 | 27..=30 | 33 | 94 | 113..=162 | 215 | 216 => Some(1),
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
        if self.trace.jni && !matches!(index,28..=30 | 34..=93 | 95..=112 | 114..=143 | 145..=162) {
            let details = self
                .jni_details(index, [a, b, c])?
                .unwrap_or_else(|| format!("arg1=0x{a:x}, arg2=0x{b:x}, arg3=0x{c:x}"));
            self.events.borrow_mut().emit(crate::Event::Jni {
                function: function.clone(),
                details,
                from: self.location(self.lr()?),
            });
        }
        match index {
            5 => Err(self.unsupported_jni("DefineClass")),
            7 => Err(self.unsupported_jni("FromReflectedMethod")),
            8 => Err(self.unsupported_jni("FromReflectedField")),
            9 => Err(self.unsupported_jni("ToReflectedMethod")),
            10 => {
                let class = self.class_name(Handle(a))?;
                if class == "java/lang/Object" {
                    return Ok(0);
                }
                let Some(superclass) = self.jni.superclasses.get(&class).cloned() else {
                    return Err(format!(
                        "GetSuperclass {class} from {}: superclass not declared; declare it with Emulator::superclass, --superclass, or the serve command superclass",
                        self.location(self.lr()?)
                    ).into());
                };
                Ok(self.find_class(&superclass, self.jni.scope).0)
            }
            12 => Err(self.unsupported_jni("ToReflectedField")),
            11 | 32 => {
                let from = self.class_name(Handle(a))?;
                let to = self.class_name(Handle(b))?;
                Ok(
                    if to == "java/lang/Object" || self.class_chain(&from).contains(&to.as_str()) {
                        1
                    } else if let Some(answer) = super::java::assignable(self, &from, &to) {
                        answer as u64
                    } else {
                        handler
                            .is_assignable(self, &from, &to)
                            .ok_or_else(|| self.jni_error(&function, &from, &to, ""))?
                            as u64
                    },
                )
            }
            13 => {
                if self.jni.objects.contains_key(&Handle(a)) {
                    self.jni.pending = Some(self.alias(Handle(a), 0)?);
                    Ok(0)
                } else {
                    Err(self.reference_error(&function, a))
                }
            }
            14 => {
                let class = self.class_name(Handle(a))?;
                self.pending_exception(&class, Some(self.cstring(b)?));
                Ok(0)
            }
            15 => Ok(self
                .jni
                .pending
                .map(|handle| self.alias(handle, self.jni.scope))
                .transpose()?
                .map_or(0, |handle| handle.0)),
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
                .insert_object(Object::User(self.class_name(Handle(a))?, Rc::new(())))
                .0),
            217 | 218 => {
                self.validate_ref(&function, a, 0)?;
                Ok(0)
            }
            226 => {
                if a == 0 {
                    return Ok(0);
                }
                let handle = self.alias(Handle(a), 1)?;
                self.jni.weak.insert(handle);
                Ok(handle.0)
            }
            227 => {
                self.jni.weak.remove(&Handle(a));
                self.release(Handle(a));
                Ok(0)
            }
            228 => Ok(if self.jni.pending.is_some() { 1 } else { 0 }),
            171..=214 | 222 | 223 => self.jni_arrays_function(index, function, a, b, c),
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
                    Some(self.require_object(&function, a)?)
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
            21 => Ok(self.alias(Handle(a), 1)?.0),
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
                Rc::ptr_eq(&self.jni_object(Handle(a))?, &self.jni_object(Handle(b))?)
            } as u64),
            25 => Ok(self.alias(Handle(a), self.jni.scope)?.0),
            31 => {
                let name = self.jni_object(Handle(a))?.borrow().java_type();
                Ok(self.find_class(&name, self.jni.scope).0)
            }
            28..=30 | 33..=162 => self.jni_members_function(index, function, a, b, c, handler),
            163..=170 | 220 | 221 | 224 | 225 => {
                self.jni_strings_function(index, function, a, b, c)
            }
            215 => {
                let class = self.class_name(Handle(a))?;
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
                let class = self.class_name(Handle(a))?;
                self.jni.natives.retain(|(c, _), _| *c != class);
                Ok(0)
            }
            219 => {
                self.write_pointer(a, self.jni.vm)?;
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
