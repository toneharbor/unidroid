use crate::{Handle, JniValue, Result};
use serde::{
    Deserialize,
    de::{Error, Unexpected},
};
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(try_from = "String")]
pub(super) struct Address(pub(super) u64);
impl TryFrom<String> for Address {
    type Error = std::num::ParseIntError;
    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        value.parse().map(Self)
    }
}

#[derive(Deserialize)]
#[serde(try_from = "String")]
pub(super) struct Method {
    pub(super) name: String,
    parameters: Vec<u8>,
}
impl TryFrom<String> for Method {
    type Error = serde_json::Error;
    fn try_from(name: String) -> std::result::Result<Self, Self::Error> {
        fn field(bytes: &mut &[u8]) -> Option<u8> {
            let (&kind, rest) = bytes.split_first()?;
            *bytes = rest;
            match kind {
                b'Z' | b'B' | b'C' | b'S' | b'I' | b'J' | b'F' | b'D' => {}
                b'L' => {
                    let end = bytes.iter().position(|&b| b == b';')?;
                    if end == 0 {
                        return None;
                    }
                    *bytes = &bytes[end + 1..];
                }
                b'[' => {
                    field(bytes)?;
                }
                _ => return None,
            }
            Some(kind)
        }
        let parse = || {
            let start = name.find('(')?;
            if start == 0 {
                return None;
            }
            let mut bytes = &name.as_bytes()[start + 1..];
            let mut parameters = Vec::new();
            while bytes.first() != Some(&b')') {
                parameters.push(field(&mut bytes)?);
            }
            bytes = &bytes[1..];
            if bytes == b"V" {
                bytes = &[];
            } else {
                field(&mut bytes)?;
            }
            bytes.is_empty().then_some(parameters)
        };
        let parameters = parse().ok_or_else(|| {
            <serde_json::Error as Error>::invalid_value(
                Unexpected::Str(&name),
                &"a JNI method signature",
            )
        })?;
        Ok(Self { name, parameters })
    }
}
impl Method {
    pub(super) fn arguments(&self, args: &[Value]) -> Result<Vec<JniValue>> {
        if args.len() != self.parameters.len() {
            return Err(format!(
                "{} takes {} arguments, got {}",
                self.name,
                self.parameters.len(),
                args.len()
            )
            .into());
        }
        self.parameters
            .iter()
            .zip(args)
            .enumerate()
            .map(|(index, (&kind, value))| {
                decode(kind, value, &format!("argument {index} of {}", self.name))
            })
            .collect()
    }
}

pub(super) fn encode(value: JniValue) -> Value {
    match value {
        JniValue::Object(v) => json!(v.0),
        JniValue::Boolean(v) => json!(v),
        JniValue::Byte(v) => json!(v),
        JniValue::Char(v) => json!(v),
        JniValue::Short(v) => json!(v),
        JniValue::Int(v) => json!(v),
        JniValue::Long(v) => json!(v.to_string()),
        JniValue::Float(v) => json!(v),
        JniValue::Double(v) => json!(v),
        JniValue::Void => Value::Null,
    }
}

pub(super) fn decode(kind: u8, value: &Value, context: &str) -> Result<JniValue> {
    let decoded = match kind {
        b'L' | b'[' => value.as_u64().map(|v| JniValue::Object(Handle(v))),
        b'Z' => value.as_bool().map(JniValue::Boolean),
        b'B' => value
            .as_i64()
            .and_then(|v| v.try_into().ok())
            .map(JniValue::Byte),
        b'C' => value
            .as_u64()
            .and_then(|v| v.try_into().ok())
            .map(JniValue::Char),
        b'S' => value
            .as_i64()
            .and_then(|v| v.try_into().ok())
            .map(JniValue::Short),
        b'I' => value
            .as_i64()
            .and_then(|v| v.try_into().ok())
            .map(JniValue::Int),
        b'J' => value
            .as_str()
            .and_then(|v| v.parse().ok())
            .map(JniValue::Long),
        b'F' => value
            .as_f64()
            .map(|v| v as f32)
            .filter(|v| v.is_finite())
            .map(JniValue::Float),
        b'D' => value.as_f64().map(JniValue::Double),
        b'V' => Some(JniValue::Void),
        _ => unreachable!(),
    };
    decoded.ok_or_else(|| {
        let expected = match kind {
            b'L' | b'[' => "a handle",
            b'Z' => "true or false",
            b'J' => "a decimal string",
            _ => "a number",
        };
        format!("{context} must be {expected}").into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_reject_wrong_types_and_out_of_range_numbers() {
        for (kind, value, expected) in [
            (b'L', json!(-1), "a handle"),
            (b'[', Value::Null, "a handle"),
            (b'Z', json!(1), "true or false"),
            (b'B', json!(128), "a number"),
            (b'C', json!(-1), "a number"),
            (b'S', json!(32768), "a number"),
            (b'I', json!(2147483648u64), "a number"),
            (b'J', json!("9223372036854775808"), "a decimal string"),
            (b'F', json!(1e40), "a number"),
            (b'D', json!("1.5"), "a number"),
        ] {
            assert_eq!(
                decode(kind, &value, "answer for test/Value->get()I")
                    .unwrap_err()
                    .to_string(),
                format!("answer for test/Value->get()I must be {expected}")
            );
        }
    }

    #[test]
    fn values_follow_the_wire_table() {
        let cases = [
            (b'L', JniValue::Object(Handle(7)), json!(7)),
            (b'[', JniValue::Object(Handle(0)), json!(0)),
            (b'Z', JniValue::Boolean(true), json!(true)),
            (b'B', JniValue::Byte(-128), json!(-128)),
            (b'C', JniValue::Char(0xd800), json!(55296)),
            (b'S', JniValue::Short(-32768), json!(-32768)),
            (b'I', JniValue::Int(-2147483648), json!(-2147483648i64)),
            (
                b'J',
                JniValue::Long(-9007199254740993),
                json!("-9007199254740993"),
            ),
            (b'F', JniValue::Float(1.5), json!(1.5)),
            (b'D', JniValue::Double(-2.25), json!(-2.25)),
            (b'V', JniValue::Void, Value::Null),
        ];
        for (kind, native, wire) in cases {
            assert_eq!(encode(native), wire);
            assert_eq!(decode(kind, &wire, "argument 0 of f()V").unwrap(), native);
        }
        assert_eq!(
            decode(b'J', &json!(9007199254740993u64), "argument 0 of f(J)V")
                .unwrap_err()
                .to_string(),
            "argument 0 of f(J)V must be a decimal string"
        );
    }
}
