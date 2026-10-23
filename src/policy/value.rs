use crate::{Emulator, Handle, JniValue, PrimitiveType, Result};
use base64::{Engine, prelude::BASE64_STANDARD};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum Object {
    Null,
    String(String),
    Instance {
        object: String,
    },
    Array {
        array: String,
        base64: String,
    },
    Map {
        map: String,
        entries: Vec<[Object; 2]>,
    },
    List {
        list: String,
        items: Vec<Object>,
    },
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
    let object = Object::deserialize(value).map_err(|_| {
        format!("{context} must be null, a string, an object, a primitive array, a map or a list")
    })?;
    Ok(JniValue::Object(
        object
            .create(emu)
            .map_err(|error| format!("{context}: {error}"))?,
    ))
}
impl Object {
    fn create(self, emu: &mut Emulator) -> Result<Handle> {
        Ok(match self {
            Self::Null => Handle(0),
            Self::String(value) => emu.new_string(&value),
            Self::Instance { object } => emu.new_object(&object, ()),
            Self::Array { array, base64 } => new_array(emu, &array, &base64)?,
            Self::Map { map, entries } => {
                let entries = entries
                    .into_iter()
                    .map(|[key, value]| Ok((key.create(emu)?, value.create(emu)?)))
                    .collect::<Result<Vec<_>>>()?;
                let result = emu.new_java_map(&map, &entries);
                for (key, value) in entries {
                    emu.release(key);
                    emu.release(value);
                }
                result?
            }
            Self::List { list, items } => {
                let items = items
                    .into_iter()
                    .map(|item| item.create(emu))
                    .collect::<Result<Vec<_>>>()?;
                let result = emu.new_java_list(&list, &items);
                for item in items {
                    emu.release(item);
                }
                result?
            }
        })
    }
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
