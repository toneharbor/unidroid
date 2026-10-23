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
pub(super) fn argument(emu: &Emulator, value: JniValue) -> Result<Option<Vec<u16>>> {
    Ok(match value {
        JniValue::Object(handle) => object::text(&reference(emu, handle)?),
        _ => Some(boxing::text(value)),
    })
}
pub(super) fn call(
    emu: &mut Emulator,
    call: &JniCall<'_>,
    receiver: &Rc<RefCell<Object>>,
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
            let Some(value) = argument(emu, call.args[0])? else {
                return Ok(None);
            };
            let Object::StringBuilder(builder) = &mut *receiver.borrow_mut() else {
                unreachable!()
            };
            builder.extend(value);
            Ok(Some(return_ref(emu, Some(receiver.clone()))))
        }
        _ => Ok(None),
    }
}
