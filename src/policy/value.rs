use crate::{Emulator, Handle, JniValue, PrimitiveType, Result};
use base64::{Engine, prelude::BASE64_STANDARD};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum Object {
    Null,
    String(String),
    Instance { object: String },
    Array { array: String, base64: String },
}

pub(super) fn decode(
    emu: &mut Emulator,
    kind: u8,
    value: Option<&Value>,
    context: &str,
) -> Result<JniValue> {
    let value = match value {
        Some(value) => value,
        None if kind == b'V' => return Ok(JniValue::Void),
        None => return Err(format!("{context} needs a value").into()),
    };
    if kind != b'L' {
        return crate::serve::wire::decode(kind, value, context);
    }
    let object = Object::deserialize(value)
        .map_err(|_| format!("{context} must be null, a string, an object or a primitive array"))?;
    Ok(JniValue::Object(match object {
        Object::Null => Handle(0),
        Object::String(value) => emu.new_string(&value),
        Object::Instance { object } => emu.new_object(&object, ()),
        Object::Array { array, base64 } => {
            new_array(emu, &array, &base64).map_err(|error| format!("{context}: {error}"))?
        }
    }))
}

pub(crate) fn new_array(emu: &mut Emulator, kind: &str, base64: &str) -> Result<Handle> {
    let kind = match kind {
        "Z" => PrimitiveType::Boolean,
        "B" => PrimitiveType::Byte,
        "C" => PrimitiveType::Char,
        "S" => PrimitiveType::Short,
        "I" => PrimitiveType::Int,
        "J" => PrimitiveType::Long,
        "F" => PrimitiveType::Float,
        "D" => PrimitiveType::Double,
        _ => return Err(format!("unknown array type: {kind}").into()),
    };
    Ok(emu.new_primitive_array(kind, &BASE64_STANDARD.decode(base64)?))
}
