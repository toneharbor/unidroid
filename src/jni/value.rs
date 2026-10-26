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
    pub fn type_code(self) -> u8 {
        match self {
            Self::Object(_) => b'L',
            Self::Boolean(_) => b'Z',
            Self::Byte(_) => b'B',
            Self::Char(_) => b'C',
            Self::Short(_) => b'S',
            Self::Int(_) => b'I',
            Self::Long(_) => b'J',
            Self::Float(_) => b'F',
            Self::Double(_) => b'D',
            Self::Void => b'V',
        }
    }
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
pub(crate) fn decode(kind: u8, bits: u64) -> JniValue {
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
pub(crate) fn method_types(signature: &str) -> Option<(Vec<u8>, u8)> {
    let mut bytes = signature.as_bytes().strip_prefix(b"(")?;
    let mut parameters = Vec::new();
    while bytes.first() != Some(&b')') {
        parameters.push(take_field(&mut bytes)?);
    }
    bytes = &bytes[1..];
    let result = if bytes == b"V" {
        bytes = &[];
        b'V'
    } else {
        take_field(&mut bytes)?
    };
    bytes.is_empty().then_some((parameters, result))
}

pub(super) fn field_type(signature: &str) -> Option<u8> {
    let mut bytes = signature.as_bytes();
    let kind = take_field(&mut bytes)?;
    bytes.is_empty().then_some(kind)
}

fn take_field(bytes: &mut &[u8]) -> Option<u8> {
    let (&kind, rest) = bytes.split_first()?;
    *bytes = rest;
    let mut element = kind;
    while element == b'[' {
        let (&next, rest) = bytes.split_first()?;
        *bytes = rest;
        element = next;
    }
    if element == b'L' {
        let end = bytes.iter().position(|&b| b == b';')?;
        if end == 0 {
            return None;
        }
        *bytes = &bytes[end + 1..];
    } else {
        super::PrimitiveType::from_code(element)?;
    }
    Some(kind)
}
