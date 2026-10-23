use super::{Handle, JniCall, JniValue, Object};
use crate::{Emulator, Result};
use std::{cell::RefCell, rc::Rc};

mod boxing;
mod builder;
mod charset;
mod collections;
mod hierarchy;
mod host;
mod maps;
mod object;
mod strings;
pub(super) use hierarchy::assignable;
mod base64;
mod digest;
pub(super) use boxing::class as boxed_class;
pub(super) use charset::Charset;
pub(super) use digest::Digest;
pub(super) use maps::Map;

pub(super) type ObjectRef = Option<Rc<RefCell<Object>>>;

#[derive(Debug)]
struct Exception(&'static str, Option<String>);
impl std::fmt::Display for Exception {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for Exception {}
fn exception(class: &'static str, message: Option<String>) -> Box<dyn std::error::Error> {
    Box::new(Exception(class, message))
}

pub(super) fn dispatch(
    emu: &mut Emulator,
    call: &JniCall<'_>,
    op: u8,
    function: &str,
) -> Result<Option<JniValue>> {
    let result = dispatch_inner(emu, call, op, function);
    match result {
        Err(error) => match error.downcast::<Exception>() {
            Ok(error) => {
                emu.pending_exception(error.0, error.1);
                let kind = if op == 6 {
                    b'L'
                } else {
                    *call
                        .signature
                        .as_bytes()
                        .get(call.signature.find(')').map_or(0, |i| i + 1))
                        .unwrap_or(&b'V')
                };
                Ok(Some(super::value::decode(kind, 0)))
            }
            Err(error) => {
                let from = emu.location(emu.lr().unwrap_or(0));
                if error.to_string().ends_with(&format!(" from {from}")) {
                    Err(error)
                } else {
                    Err(format!("{error} from {from}").into())
                }
            }
        },
        result => result,
    }
}
fn dispatch_inner(
    emu: &mut Emulator,
    call: &JniCall<'_>,
    op: u8,
    function: &str,
) -> Result<Option<JniValue>> {
    if op == 6 {
        if let Some(value) = strings::construct(emu, call, function)? {
            return Ok(Some(value));
        }
        if let Some(value) = builder::construct(emu, call)? {
            return Ok(Some(value));
        }
        if let Some(value) = maps::construct(emu, call)? {
            return Ok(Some(value));
        }
        if let Some(value) = collections::construct(emu, call)? {
            return Ok(Some(value));
        }
        return boxing::static_call(emu, call, true);
    }
    if op == 1 {
        if call.class == "java/lang/String"
            && call.name == "valueOf"
            && matches!(
                call.signature,
                "(I)Ljava/lang/String;"
                    | "(J)Ljava/lang/String;"
                    | "(Z)Ljava/lang/String;"
                    | "(C)Ljava/lang/String;"
                    | "(Ljava/lang/Object;)Ljava/lang/String;"
            )
        {
            return Ok(builder::argument(emu, call.args[0])?.map(|value| string(emu, value)));
        }
        if call.class == "java/nio/charset/Charset"
            && call.name == "forName"
            && call.signature == "(Ljava/lang/String;)Ljava/nio/charset/Charset;"
        {
            return Ok(Charset::parse(&String::from_utf16_lossy(&units(
                emu,
                call.args[0].object(),
            )?))
            .map(|charset| emu.insert_object(Object::Charset(charset)).into()));
        }
        if let Some(value) = digest::static_call(emu, call)? {
            return Ok(Some(value));
        }
        if let Some(value) = base64::call(emu, call)? {
            return Ok(Some(value));
        }
        return boxing::static_call(emu, call, false);
    }
    if op == 3
        && call.class == "java/nio/charset/StandardCharsets"
        && call.signature == "Ljava/nio/charset/Charset;"
    {
        return Ok(match call.name {
            "UTF_8" | "US_ASCII" | "ISO_8859_1" | "UTF_16LE" | "UTF_16BE" => {
                Charset::parse(call.name)
                    .map(|charset| emu.insert_object(Object::Charset(charset)).into())
            }
            _ => None,
        });
    }
    if op != 0 {
        return Ok(None);
    }
    let receiver = emu.jni_object(call.receiver)?;
    if matches!(&*receiver.borrow(), Object::User(..)) {
        return Ok(None);
    }
    if let Some(value) = object::call(emu, call, &receiver)? {
        return Ok(Some(value));
    }
    match &*receiver.borrow() {
        Object::String(value) => return strings::call(emu, call, value),
        Object::Boxed(value) => return Ok(boxing::call(call, *value)),
        Object::Charset(charset)
            if call.name == "name" && call.signature == "()Ljava/lang/String;" =>
        {
            return Ok(Some(string(
                emu,
                charset.name().encode_utf16().collect::<Vec<_>>(),
            )));
        }
        _ => {}
    }
    if matches!(&*receiver.borrow(), Object::StringBuilder(_)) {
        return builder::call(emu, call, &receiver);
    }
    if matches!(&*receiver.borrow(), Object::Digest(_)) {
        return digest::call(emu, call, &receiver);
    }
    if matches!(&*receiver.borrow(), Object::Map(_)) {
        return maps::call(emu, call, &receiver);
    }
    collections::call(emu, call, &receiver)
}
fn return_ref(emu: &mut Emulator, object: ObjectRef) -> JniValue {
    object
        .map_or(Handle(0), |object| emu.insert_ref(object, emu.jni.scope))
        .into()
}
fn reference(emu: &Emulator, handle: Handle) -> Result<ObjectRef> {
    if handle.0 == 0 {
        Ok(None)
    } else {
        emu.jni_object(handle).map(Some)
    }
}
fn units(emu: &Emulator, handle: Handle) -> Result<Vec<u16>> {
    let object =
        reference(emu, handle)?.ok_or_else(|| exception("java/lang/NullPointerException", None))?;
    match &*object.borrow() {
        Object::String(value) => Ok(value.clone()),
        object => Err(format!("expected a string, got {}", object.java_type()).into()),
    }
}
fn string(emu: &mut Emulator, value: impl Into<Vec<u16>>) -> JniValue {
    emu.insert_object(Object::String(value.into())).into()
}
fn hash_string(value: &[u16]) -> i32 {
    value
        .iter()
        .fold(0i32, |h, &c| h.wrapping_mul(31).wrapping_add(c as i32))
}
