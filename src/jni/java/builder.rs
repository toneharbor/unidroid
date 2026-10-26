use super::*;

pub(super) fn construct(emu: &mut Emulator, call: &JniCall<'_>) -> Result<Option<JniValue>> {
    if call.class != "java/lang/StringBuilder" || call.name != "<init>" {
        return Ok(None);
    }
    let value = match call.signature {
        "()V" => Vec::new(),
        "(I)V" => {
            if call.args[0].int() < 0 {
                return Err(exception("java/lang/NegativeArraySizeException", None));
            }
            Vec::new()
        }
        "(Ljava/lang/String;)V" => units(emu, call.args[0].object())?,
        _ => return Ok(None),
    };
    Ok(Some(emu.insert_object(Object::StringBuilder(value)).into()))
}
fn argument(
    emu: &mut Emulator,
    value: JniValue,
    function: &str,
    handler: &mut dyn JniHandler,
) -> Result<Vec<u16>> {
    match value {
        JniValue::Object(handle) => object_text(emu, handle, function, handler),
        _ => Ok(boxing::text(value)),
    }
}
pub(super) fn call(
    emu: &mut Emulator,
    call: &JniCall<'_>,
    receiver: &Rc<RefCell<Object>>,
    function: &str,
    handler: &mut dyn JniHandler,
) -> Result<Option<JniValue>> {
    match (call.name, call.signature) {
        ("length", "()I") => {
            let Object::StringBuilder(value) = &*receiver.borrow() else {
                unreachable!()
            };
            Ok(Some(JniValue::Int(value.len() as i32)))
        }
        (
            "append",
            "(Ljava/lang/String;)Ljava/lang/StringBuilder;"
            | "(Ljava/lang/Object;)Ljava/lang/StringBuilder;"
            | "(I)Ljava/lang/StringBuilder;"
            | "(J)Ljava/lang/StringBuilder;"
            | "(C)Ljava/lang/StringBuilder;"
            | "(Z)Ljava/lang/StringBuilder;",
        ) => {
            let value = argument(emu, call.args[0], function, handler)?;
            let Object::StringBuilder(builder) = &mut *receiver.borrow_mut() else {
                unreachable!()
            };
            builder.extend(value);
            Ok(Some(return_ref(emu, Some(receiver.clone()))))
        }
        _ => Ok(None),
    }
}
