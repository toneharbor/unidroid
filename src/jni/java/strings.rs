use super::{charset::Charset, *};
use crate::PrimitiveType;

pub(super) fn construct(
    emu: &mut Emulator,
    call: &JniCall<'_>,
    function: &str,
) -> Result<Option<JniValue>> {
    if call.class != "java/lang/String" || call.name != "<init>" {
        return Ok(None);
    }
    let a = call.args;
    let value = match call.signature {
        "()V" => Vec::new(),
        "(Ljava/lang/String;)V" => units(emu, a[0].object())?,
        "([C)V" => array(emu, a[0].object(), function)?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&b| u16::from_le_bytes(b))
            .collect(),
        "([B)V" | "([BLjava/lang/String;)V" | "([BII)V" | "([BIILjava/lang/String;)V" => {
            let bytes = array(emu, a[0].object(), function)?;
            let charset = if call.signature.contains("Ljava/lang/String;") {
                let Some(charset) = named_charset(emu, a.last().unwrap().object(), function)?
                else {
                    return Ok(None);
                };
                charset
            } else {
                Charset::Utf8
            };
            let bytes = if call.signature.contains("II") {
                range(&bytes, a[1].int(), a[1].int() as i64 + a[2].int() as i64)?
            } else {
                &bytes
            };
            charset.decode(bytes)
        }
        _ => return Ok(None),
    };
    Ok(Some(string(emu, value)))
}
fn named_charset(emu: &Emulator, handle: Handle, function: &str) -> Result<Option<Charset>> {
    if handle.0 != 0 {
        emu.validate_ref(function, handle.0, 2)?;
    }
    Ok(Charset::parse(&String::from_utf16_lossy(&units(
        emu, handle,
    )?)))
}
fn range<T>(value: &[T], start: i32, end: i64) -> Result<&[T]> {
    if start < 0 || end < start as i64 || end > value.len() as i64 {
        return Err(exception("java/lang/StringIndexOutOfBoundsException", None));
    }
    Ok(&value[start as usize..end as usize])
}
pub(super) fn call(
    emu: &mut Emulator,
    call: &JniCall<'_>,
    value: &[u16],
    function: &str,
) -> Result<Option<JniValue>> {
    let a = call.args;
    let answer = match (call.name, call.signature) {
        ("length", "()I") => JniValue::Int(value.len() as i32),
        ("isEmpty", "()Z") => JniValue::Boolean(value.is_empty()),
        ("charAt", "(I)C") => JniValue::Char(range(value, a[0].int(), a[0].int() as i64 + 1)?[0]),
        ("substring", "(I)Ljava/lang/String;") | ("substring", "(II)Ljava/lang/String;") => {
            let end = a.get(1).map_or(value.len() as i64, |v| v.int() as i64);
            string(emu, range(value, a[0].int(), end)?)
        }
        ("toCharArray", "()[C") => emu
            .insert_object(Object::Array(
                PrimitiveType::Char,
                value.iter().flat_map(|c| c.to_le_bytes()).collect(),
            ))
            .into(),
        ("getBytes", "()[B") | ("getBytes", "(Ljava/lang/String;)[B") => {
            let charset = if a.is_empty() {
                Charset::Utf8
            } else {
                let Some(charset) = named_charset(emu, a[0].object(), function)? else {
                    return Ok(None);
                };
                charset
            };
            emu.insert_object(Object::Array(PrimitiveType::Byte, charset.encode(value)))
                .into()
        }
        ("getBytes", "(Ljava/nio/charset/Charset;)[B") => {
            let charset = reference(emu, a[0].object())?
                .ok_or_else(|| exception("java/lang/NullPointerException", None))?;
            let Object::Charset(charset) = *charset.borrow() else {
                return Ok(None);
            };
            emu.insert_object(Object::Array(PrimitiveType::Byte, charset.encode(value)))
                .into()
        }
        ("contains", "(Ljava/lang/CharSequence;)Z") => {
            let Some(needle) = sequence(emu, a[0].object())? else {
                return Ok(None);
            };
            JniValue::Boolean(index_of(value, &needle) >= 0)
        }
        ("replace", "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Ljava/lang/String;") => {
            let Some(target) = sequence(emu, a[0].object())? else {
                return Ok(None);
            };
            let Some(replacement) = sequence(emu, a[1].object())? else {
                return Ok(None);
            };
            let mut result = Vec::new();
            if target.is_empty() {
                result.extend(&replacement);
                for &unit in value {
                    result.push(unit);
                    result.extend(&replacement);
                }
            } else {
                let mut rest = value;
                loop {
                    let index = index_of(rest, &target);
                    if index < 0 {
                        result.extend(rest);
                        break;
                    }
                    result.extend(&rest[..index as usize]);
                    result.extend(&replacement);
                    rest = &rest[index as usize + target.len()..];
                }
            }
            string(emu, result)
        }
        ("startsWith", "(Ljava/lang/String;)Z") => {
            JniValue::Boolean(value.starts_with(&units(emu, a[0].object())?))
        }
        ("endsWith", "(Ljava/lang/String;)Z") => {
            JniValue::Boolean(value.ends_with(&units(emu, a[0].object())?))
        }
        ("indexOf", "(Ljava/lang/String;)I") => {
            JniValue::Int(index_of(value, &units(emu, a[0].object())?))
        }
        ("indexOf", "(I)I") => {
            let code = a[0].int();
            let needle = if (0..=0xffff).contains(&code) {
                vec![code as u16]
            } else if let Some(c) = char::from_u32(code as u32) {
                c.encode_utf16(&mut [0; 2]).to_vec()
            } else {
                return Ok(Some(JniValue::Int(-1)));
            };
            JniValue::Int(index_of(value, &needle))
        }
        ("compareTo", "(Ljava/lang/String;)I") => {
            let other = units(emu, a[0].object())?;
            JniValue::Int(
                value
                    .iter()
                    .zip(&other)
                    .find(|(x, y)| x != y)
                    .map_or(value.len() as i32 - other.len() as i32, |(&x, &y)| {
                        x as i32 - y as i32
                    }),
            )
        }
        ("equalsIgnoreCase", "(Ljava/lang/String;)Z") => {
            if a[0].object().0 == 0 {
                JniValue::Boolean(false)
            } else {
                let other = units(emu, a[0].object())?;
                JniValue::Boolean(
                    value.len() == other.len() && case_fold(value) == case_fold(&other),
                )
            }
        }
        ("toLowerCase", "()Ljava/lang/String;") => string(emu, change_case(value, false)),
        ("toUpperCase", "()Ljava/lang/String;") => string(emu, change_case(value, true)),
        ("trim", "()Ljava/lang/String;") => {
            let start = value.iter().position(|&c| c > 32).unwrap_or(value.len());
            let end = value.iter().rposition(|&c| c > 32).map_or(start, |i| i + 1);
            string(emu, &value[start..end])
        }
        ("concat", "(Ljava/lang/String;)Ljava/lang/String;") => {
            let mut result = value.to_vec();
            result.extend(units(emu, a[0].object())?);
            string(emu, result)
        }
        _ => return Ok(None),
    };
    Ok(Some(answer))
}
fn sequence(emu: &Emulator, handle: Handle) -> Result<Option<Vec<u16>>> {
    let object =
        reference(emu, handle)?.ok_or_else(|| exception("java/lang/NullPointerException", None))?;
    Ok(match &*object.borrow() {
        Object::String(s) | Object::StringBuilder(s) => Some(s.clone()),
        _ => None,
    })
}
fn index_of(haystack: &[u16], needle: &[u16]) -> i32 {
    if needle.is_empty() {
        return 0;
    }
    haystack
        .windows(needle.len())
        .position(|part| part == needle)
        .map_or(-1, |i| i as i32)
}
fn case_fold(value: &[u16]) -> Vec<u32> {
    char::decode_utf16(value.iter().copied())
        .map(|c| match c {
            Ok(c) => {
                let mut upper = c.to_uppercase();
                let first = upper.next().unwrap();
                let upper = if upper.next().is_none() { first } else { c };
                upper.to_lowercase().next().unwrap() as u32
            }
            Err(e) => e.unpaired_surrogate() as u32,
        })
        .collect()
}
fn change_case(value: &[u16], upper: bool) -> Vec<u16> {
    let mut result = Vec::new();
    let mut run = String::new();
    let flush = |run: &mut String, result: &mut Vec<u16>| {
        let mapped = if upper {
            run.to_uppercase()
        } else {
            run.to_lowercase()
        };
        result.extend(mapped.encode_utf16());
        run.clear();
    };
    for c in char::decode_utf16(value.iter().copied()) {
        match c {
            Ok(c) => run.push(c),
            Err(e) => {
                flush(&mut run, &mut result);
                result.push(e.unpaired_surrogate());
            }
        }
    }
    flush(&mut run, &mut result);
    result
}
