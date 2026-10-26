use super::{Handle, JniCall, JniHandler, JniOp, JniValue, Object};
use crate::{Emulator, Result};
use std::{cell::RefCell, rc::Rc};

mod boxing;
mod builder;
mod charset;
mod collections;
pub(super) mod hierarchy;
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
enum Exception {
    New(&'static str, Option<String>),
    Pending,
}
impl std::fmt::Display for Exception {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::New(class, _) => class,
            Self::Pending => "pending Java exception",
        })
    }
}
impl std::error::Error for Exception {}
pub(super) fn exception(
    class: &'static str,
    message: Option<String>,
) -> Box<dyn std::error::Error> {
    Box::new(Exception::New(class, message))
}

pub(super) fn dispatch(
    emu: &mut Emulator,
    call: &JniCall<'_>,
    function: &str,
    handler: &mut dyn JniHandler,
) -> Result<Option<JniValue>> {
    let result = dispatch_inner(emu, call, function, handler);
    match result {
        Err(error) => match error.downcast::<Exception>() {
            Ok(error) => {
                if let Exception::New(class, message) = *error {
                    emu.pending_exception(class, message);
                }
                Ok(Some(super::value::decode(call.type_code, 0)))
            }
            Err(error) => Err(emu.jni_message(error).into()),
        },
        result => result,
    }
}
fn dispatch_inner(
    emu: &mut Emulator,
    call: &JniCall<'_>,
    function: &str,
    handler: &mut dyn JniHandler,
) -> Result<Option<JniValue>> {
    if emu.apk().is_some()
        && let Some(value) = super::android::dispatch(emu, call)?
    {
        return Ok(Some(value));
    }
    if call.op == JniOp::New {
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
    if call.op == JniOp::Call && call.is_static {
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
            if call.signature == "(Ljava/lang/Object;)Ljava/lang/String;" {
                return object_string(emu, call.args[0].object(), function, handler).map(Some);
            }
            return Ok(Some(string(emu, boxing::text(call.args[0]))));
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
        if let Some(value) = base64::call(emu, call, function)? {
            return Ok(Some(value));
        }
        return boxing::static_call(emu, call, false);
    }
    if call.op == JniOp::Get
        && call.is_static
        && call.class == "java/nio/charset/StandardCharsets"
        && call.signature == "Ljava/nio/charset/Charset;"
    {
        return Ok(match call.name {
            "UTF_8" | "US_ASCII" | "ISO_8859_1" | "UTF_16LE" | "UTF_16BE" => {
                Charset::parse(call.name).map(|charset| {
                    let object = emu
                        .jni
                        .charsets
                        .entry(charset.name())
                        .or_insert_with(|| Rc::new(RefCell::new(Object::Charset(charset))))
                        .clone();
                    return_ref(emu, Some(object))
                })
            }
            _ => None,
        });
    }
    if call.op != JniOp::Call || call.is_static {
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
        Object::Reflected { id, .. } => return emu.reflected_call(call, *id),
        Object::String(value) => return strings::call(emu, call, value, function),
        Object::Boxed(value) => return Ok(boxing::call(call, *value)),
        Object::Charset(charset)
            if call.name == "name" && call.signature == "()Ljava/lang/String;" =>
        {
            return Ok(Some(string_utf8(emu, charset.name())));
        }
        _ => {}
    }
    if matches!(&*receiver.borrow(), Object::StringBuilder(_)) {
        return builder::call(emu, call, &receiver, function, handler);
    }
    if matches!(&*receiver.borrow(), Object::Digest(_)) {
        return digest::call(emu, call, &receiver, function);
    }
    if matches!(&*receiver.borrow(), Object::Map(_)) {
        return maps::call(emu, call, &receiver);
    }
    collections::call(emu, call, &receiver)
}
enum ObjectText {
    Units(Vec<u16>),
    String(Handle),
}

fn object_text(
    emu: &mut Emulator,
    handle: Handle,
    function: &str,
    handler: &mut dyn JniHandler,
) -> Result<Vec<u16>> {
    match object_content(emu, handle, function, handler)? {
        ObjectText::Units(text) => Ok(text),
        ObjectText::String(Handle(0)) => Ok("null".encode_utf16().collect()),
        ObjectText::String(handle) => units(emu, handle),
    }
}

fn object_string(
    emu: &mut Emulator,
    handle: Handle,
    function: &str,
    handler: &mut dyn JniHandler,
) -> Result<JniValue> {
    let receiver = reference(emu, handle)?;
    if receiver
        .as_ref()
        .is_some_and(|object| matches!(&*object.borrow(), Object::String(_)))
    {
        return Ok(return_ref(emu, receiver));
    }
    match object_content(emu, handle, function, handler)? {
        ObjectText::Units(text) => Ok(string(emu, text)),
        ObjectText::String(handle) => Ok(handle.into()),
    }
}

fn object_content(
    emu: &mut Emulator,
    handle: Handle,
    function: &str,
    handler: &mut dyn JniHandler,
) -> Result<ObjectText> {
    let receiver = reference(emu, handle)?;
    if let Some(text) = object::text(emu, &receiver) {
        return Ok(ObjectText::Units(text));
    }
    let class = receiver.unwrap().borrow().java_type();
    let call = JniCall {
        op: JniOp::Call,
        is_static: false,
        type_code: b'L',
        class: &class,
        name: "toString",
        signature: "()Ljava/lang/String;",
        receiver: handle,
        args: &[],
    };
    let answer = match dispatch_inner(emu, &call, function, handler)? {
        Some(value) => Some(value),
        None => {
            let answer = handler.member(emu, &call);
            super::handler::check_answer(emu, &call, function, answer)?
        }
    };
    if emu.jni.pending.is_some() {
        return Err(Box::new(Exception::Pending));
    }
    answer
        .map(|value| ObjectText::String(value.object()))
        .ok_or_else(|| emu.jni_error(function, &class, call.name, call.signature))
}

pub(super) fn return_ref(emu: &mut Emulator, object: ObjectRef) -> JniValue {
    emu.handle_or_null(object, emu.jni.scope).into()
}
pub(super) fn reference(emu: &Emulator, handle: Handle) -> Result<ObjectRef> {
    if handle.0 == 0 {
        Ok(None)
    } else {
        emu.jni_object(handle).map(Some)
    }
}
fn array(emu: &Emulator, handle: Handle, function: &str) -> Result<Vec<u8>> {
    if handle.0 == 0 {
        return Err(exception("java/lang/NullPointerException", None));
    }
    emu.validate_ref(function, handle.0, 3)?;
    emu.primitive_array(handle)
}
pub(super) fn units(emu: &Emulator, handle: Handle) -> Result<Vec<u16>> {
    let object =
        reference(emu, handle)?.ok_or_else(|| exception("java/lang/NullPointerException", None))?;
    match &*object.borrow() {
        Object::String(value) => Ok(value.clone()),
        object => Err(format!("expected a string, got {}", object.java_type()).into()),
    }
}
pub(super) fn string_utf8(emu: &mut Emulator, value: &str) -> JniValue {
    string(emu, value.encode_utf16().collect::<Vec<_>>())
}
pub(super) fn string(emu: &mut Emulator, value: impl Into<Vec<u16>>) -> JniValue {
    emu.insert_object(Object::String(value.into())).into()
}
fn hash_string(value: &[u16]) -> i32 {
    value
        .iter()
        .fold(0i32, |h, &c| h.wrapping_mul(31).wrapping_add(c as i32))
}
