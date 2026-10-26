use super::*;

pub(in crate::jni) fn class(value: JniValue) -> Option<&'static str> {
    Some(match value {
        JniValue::Boolean(_) => "java/lang/Boolean",
        JniValue::Byte(_) => "java/lang/Byte",
        JniValue::Char(_) => "java/lang/Character",
        JniValue::Short(_) => "java/lang/Short",
        JniValue::Int(_) => "java/lang/Integer",
        JniValue::Long(_) => "java/lang/Long",
        JniValue::Float(_) => "java/lang/Float",
        JniValue::Double(_) => "java/lang/Double",
        _ => return None,
    })
}
pub(super) fn integer(value: JniValue) -> Option<i64> {
    Some(match value {
        JniValue::Byte(v) => v as i64,
        JniValue::Short(v) => v as i64,
        JniValue::Int(v) => v as i64,
        JniValue::Long(v) => v,
        _ => return None,
    })
}
pub(super) fn hash(value: JniValue) -> i32 {
    match value {
        JniValue::Boolean(v) => {
            if v {
                1231
            } else {
                1237
            }
        }
        JniValue::Char(v) => v as i32,
        JniValue::Float(v) => float_bits(v) as i32,
        JniValue::Double(v) => {
            let bits = double_bits(v);
            (bits ^ (bits >> 32)) as i32
        }
        JniValue::Long(v) => (v ^ ((v as u64 >> 32) as i64)) as i32,
        _ => integer(value).unwrap() as i32,
    }
}
fn float_bits(v: f32) -> u32 {
    if v.is_nan() { 0x7fc00000 } else { v.to_bits() }
}
fn double_bits(v: f64) -> u64 {
    if v.is_nan() {
        0x7ff8000000000000
    } else {
        v.to_bits()
    }
}
pub(super) fn equal(a: JniValue, b: JniValue) -> bool {
    match (a, b) {
        (JniValue::Float(a), JniValue::Float(b)) => float_bits(a) == float_bits(b),
        (JniValue::Double(a), JniValue::Double(b)) => double_bits(a) == double_bits(b),
        _ => a == b,
    }
}
pub(super) fn text(value: JniValue) -> Vec<u16> {
    if let JniValue::Char(c) = value {
        return vec![c];
    }
    let value = match value {
        JniValue::Boolean(v) => v.to_string(),
        JniValue::Float(v) => floating(v as f64, true),
        JniValue::Double(v) => floating(v, false),
        _ => integer(value).unwrap().to_string(),
    };
    value.encode_utf16().collect()
}
fn floating(value: f64, single: bool) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-Infinity"
        } else {
            "Infinity"
        }
        .into();
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0"
        } else {
            "0.0"
        }
        .into();
    }
    let mut scientific = String::new();
    for precision in 2..=if single { 9 } else { 17 } {
        scientific = format!("{:.*e}", precision - 1, value);
        let parsed = if single {
            scientific.parse::<f32>().unwrap() as f64
        } else {
            scientific.parse::<f64>().unwrap()
        };
        if parsed == value {
            break;
        }
    }
    let (mantissa, exponent) = scientific.split_once('e').unwrap();
    let exponent: i32 = exponent.parse().unwrap();
    let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
    if !(-3..7).contains(&exponent) {
        return format!(
            "{}{}E{exponent}",
            mantissa,
            if mantissa.contains('.') { "" } else { ".0" }
        );
    }
    let negative = mantissa.starts_with('-');
    let mut digits = mantissa.trim_start_matches('-').replace('.', "");
    let point = exponent + 1;
    let result = if point <= 0 {
        format!("0.{}{}", "0".repeat(-point as usize), digits)
    } else if point as usize >= digits.len() {
        format!("{}{}.0", digits, "0".repeat(point as usize - digits.len()))
    } else {
        digits.insert(point as usize, '.');
        digits
    };
    format!("{}{result}", if negative { "-" } else { "" })
}
pub(super) fn static_call(
    emu: &mut Emulator,
    call: &JniCall<'_>,
    constructor: bool,
) -> Result<Option<JniValue>> {
    if let Some(&value) = call.args.first()
        && class(value) == Some(call.class)
        && ((constructor
            && call.name == "<init>"
            && call.signature == format!("({})V", value.type_code() as char))
            || (!constructor
                && call.name == "valueOf"
                && call.signature == format!("({})L{};", value.type_code() as char, call.class)))
    {
        if !constructor
            && match value {
                JniValue::Boolean(_) => true,
                JniValue::Char(value) => value <= 127,
                _ => integer(value).is_some_and(|value| (-128..=127).contains(&value)),
            }
        {
            let object = emu
                .jni
                .boxed
                .entry((value.type_code(), value.bits()))
                .or_insert_with(|| Rc::new(RefCell::new(Object::Boxed(value))))
                .clone();
            return Ok(Some(return_ref(emu, Some(object))));
        }
        return Ok(Some(emu.insert_object(Object::Boxed(value)).into()));
    }
    if constructor {
        return Ok(None);
    }
    let a = call.args;
    let result = match (call.class, call.name, call.signature) {
        ("java/lang/Integer", "parseInt", "(Ljava/lang/String;)I" | "(Ljava/lang/String;I)I")
        | ("java/lang/Long", "parseLong", "(Ljava/lang/String;)J") => {
            if a[0].object().0 == 0 {
                return Err(exception(
                    "java/lang/NumberFormatException",
                    Some("Cannot parse null string".into()),
                ));
            }
            let text = String::from_utf16_lossy(&units(emu, a[0].object())?);
            let radix = a.get(1).map_or(10, |v| v.int());
            let parsed = (2..=36)
                .contains(&radix)
                .then(|| parse_integer(&text, radix as u32))
                .flatten();
            let parsed =
                parsed.filter(|&v| call.class != "java/lang/Integer" || i32::try_from(v).is_ok());
            let v = parsed.ok_or_else(|| {
                exception(
                    "java/lang/NumberFormatException",
                    Some(format!("For input string: \"{text}\"")),
                )
            })?;
            if call.class == "java/lang/Integer" {
                JniValue::Int(v as i32)
            } else {
                JniValue::Long(v)
            }
        }
        ("java/lang/Integer", "toString", "(I)Ljava/lang/String;")
        | ("java/lang/Long", "toString", "(J)Ljava/lang/String;") => string(emu, text(a[0])),
        ("java/lang/Integer", "toHexString", "(I)Ljava/lang/String;") => {
            string_utf8(emu, &format!("{:x}", a[0].int() as u32))
        }
        _ => return Ok(None),
    };
    Ok(Some(result))
}
fn parse_integer(text: &str, radix: u32) -> Option<i64> {
    let (negative, digits) = if let Some(s) = text.strip_prefix('-') {
        (true, s)
    } else {
        (false, text.strip_prefix('+').unwrap_or(text))
    };
    if digits.is_empty() {
        return None;
    }
    let mut value = 0i64;
    for c in digits.chars() {
        let digit = c.to_digit(radix).or_else(|| {
            // Java Character.digit accepts BMP decimal digits and full-width Latin letters.
            let c = c as u32;
            let decimal = [
                0x660, 0x6f0, 0x7c0, 0x966, 0x9e6, 0xa66, 0xae6, 0xb66, 0xbe6, 0xc66, 0xce6, 0xd66,
                0xde6, 0xe50, 0xed0, 0xf20, 0x1040, 0x1090, 0x17e0, 0x1810, 0x1946, 0x19d0, 0x1a80,
                0x1a90, 0x1b50, 0x1bb0, 0x1c40, 0x1c50, 0xa620, 0xa8d0, 0xa900, 0xa9d0, 0xa9f0,
                0xaa50, 0xabf0, 0xff10,
            ]
            .into_iter()
            .find_map(|base| (base..base + 10).contains(&c).then(|| c - base));
            decimal
                .or_else(|| {
                    if (0xff21..=0xff3a).contains(&c) {
                        Some(c - 0xff21 + 10)
                    } else if (0xff41..=0xff5a).contains(&c) {
                        Some(c - 0xff41 + 10)
                    } else {
                        None
                    }
                })
                .filter(|&d| d < radix)
        })?;
        value = value.checked_mul(radix as i64)?.checked_sub(digit as i64)?;
    }
    if negative {
        Some(value)
    } else {
        value.checked_neg()
    }
}
pub(super) fn call(call: &JniCall<'_>, value: JniValue) -> Option<JniValue> {
    let matching = match value {
        JniValue::Boolean(_) => ("booleanValue", "()Z"),
        JniValue::Byte(_) => ("byteValue", "()B"),
        JniValue::Char(_) => ("charValue", "()C"),
        JniValue::Short(_) => ("shortValue", "()S"),
        JniValue::Int(_) => ("intValue", "()I"),
        JniValue::Long(_) => ("longValue", "()J"),
        JniValue::Float(_) => ("floatValue", "()F"),
        JniValue::Double(_) => ("doubleValue", "()D"),
        _ => unreachable!(),
    };
    if (call.name, call.signature) == matching {
        return Some(value);
    }
    match (call.name, call.signature) {
        ("intValue", "()I") => integer(value).map(|v| JniValue::Int(v as i32)),
        ("longValue", "()J") => integer(value).map(JniValue::Long),
        _ => None,
    }
}
