use crate::jni::{ARRAY_TYPES, Handle, Object, PrimitiveType};
use crate::{Emulator, Result};

impl Emulator {
    pub(super) fn jni_arrays_function(
        &mut self,
        index: usize,
        function: String,
        a: u64,
        b: u64,
        c: u64,
    ) -> Result<u64> {
        match index {
            172 => {
                let length = a as i32 as i64;
                if length < 0 {
                    self.pending_exception("java/lang/NegativeArraySizeException", None);
                    Ok(0)
                } else {
                    let initial = (c != 0).then(|| self.jni_object(Handle(c))).transpose()?;
                    let class = self.class_name(Handle(b))?;
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
                let value = (c != 0).then(|| self.jni_object(Handle(c))).transpose()?;
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
                let bytes = self.primitive_array(Handle(a))?;
                if b != 0 {
                    self.write_bytes(b, &[1])?;
                }
                self.jni_buffer(&bytes)
            }
            191..=198 | 223 => {
                if c == 0 || c == 1 {
                    let object = self.jni_object(Handle(a))?;
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
            _ => unreachable!(),
        }
    }
}
