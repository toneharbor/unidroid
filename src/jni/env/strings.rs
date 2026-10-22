use crate::jni::{Handle, Object};
use crate::{Emulator, Result};

impl Emulator {
    pub(super) fn jni_strings_function(
        &mut self,
        index: usize,
        function: String,
        a: u64,
        b: u64,
        c: u64,
    ) -> Result<u64> {
        match index {
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
                let object = self.jni_object(Handle(a))?;
                let Object::String(value) = &*object.borrow() else {
                    unreachable!()
                };
                Ok(value.len() as u64)
            }
            165 | 169 | 224 => {
                let object = self.jni_object(Handle(a))?;
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
                let object = self.jni_object(Handle(a))?;
                let Object::String(value) = &*object.borrow() else {
                    unreachable!()
                };
                Ok(modified_utf8(value).len() as u64)
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
            _ => unreachable!(),
        }
    }
}

pub(crate) fn modified_utf8(units: &[u16]) -> Vec<u8> {
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
pub(crate) fn decode_modified_utf8(bytes: &[u8]) -> Vec<u16> {
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
