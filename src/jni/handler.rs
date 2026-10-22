use super::{Handle, JniValue};
use crate::Emulator;

/// Decoded member access delivered to a handler. Objects are handles into this emulator.
pub struct JniCall<'a> {
    pub class: &'a str,
    pub name: &'a str,
    pub signature: &'a str,
    pub receiver: Handle,
    pub args: &'a [JniValue],
}

pub trait JniHandler {
    fn stopped(&mut self, _emu: &mut Emulator, _stop: &crate::Stop) -> crate::Resume {
        crate::Resume::Continue
    }
    fn is_assignable(&mut self, _emu: &mut Emulator, _from: &str, _to: &str) -> Option<bool> {
        None
    }
    fn new_object(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<Handle> {
        None
    }
    fn call_object_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<Handle> {
        None
    }
    fn call_static_object_method(
        &mut self,
        _emu: &mut Emulator,
        _call: &JniCall<'_>,
    ) -> Option<Handle> {
        None
    }
    fn get_object_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<Handle> {
        None
    }
    fn get_static_object_field(
        &mut self,
        _emu: &mut Emulator,
        _call: &JniCall<'_>,
    ) -> Option<Handle> {
        None
    }
    fn set_object_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn set_static_object_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn call_boolean_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<bool> {
        None
    }
    fn call_static_boolean_method(
        &mut self,
        _emu: &mut Emulator,
        _call: &JniCall<'_>,
    ) -> Option<bool> {
        None
    }
    fn get_boolean_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<bool> {
        None
    }
    fn get_static_boolean_field(
        &mut self,
        _emu: &mut Emulator,
        _call: &JniCall<'_>,
    ) -> Option<bool> {
        None
    }
    fn set_boolean_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn set_static_boolean_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn call_byte_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i8> {
        None
    }
    fn call_static_byte_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i8> {
        None
    }
    fn get_byte_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i8> {
        None
    }
    fn get_static_byte_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i8> {
        None
    }
    fn set_byte_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn set_static_byte_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn call_char_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<u16> {
        None
    }
    fn call_static_char_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<u16> {
        None
    }
    fn get_char_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<u16> {
        None
    }
    fn get_static_char_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<u16> {
        None
    }
    fn set_char_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn set_static_char_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn call_short_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i16> {
        None
    }
    fn call_static_short_method(
        &mut self,
        _emu: &mut Emulator,
        _call: &JniCall<'_>,
    ) -> Option<i16> {
        None
    }
    fn get_short_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i16> {
        None
    }
    fn get_static_short_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i16> {
        None
    }
    fn set_short_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn set_static_short_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn call_int_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i32> {
        None
    }
    fn call_static_int_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i32> {
        None
    }
    fn get_int_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i32> {
        None
    }
    fn get_static_int_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i32> {
        None
    }
    fn set_int_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn set_static_int_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn call_long_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i64> {
        None
    }
    fn call_static_long_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i64> {
        None
    }
    fn get_long_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i64> {
        None
    }
    fn get_static_long_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<i64> {
        None
    }
    fn set_long_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn set_static_long_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn call_float_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<f32> {
        None
    }
    fn call_static_float_method(
        &mut self,
        _emu: &mut Emulator,
        _call: &JniCall<'_>,
    ) -> Option<f32> {
        None
    }
    fn get_float_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<f32> {
        None
    }
    fn get_static_float_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<f32> {
        None
    }
    fn set_float_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn set_static_float_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn call_double_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<f64> {
        None
    }
    fn call_static_double_method(
        &mut self,
        _emu: &mut Emulator,
        _call: &JniCall<'_>,
    ) -> Option<f64> {
        None
    }
    fn get_double_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<f64> {
        None
    }
    fn get_static_double_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<f64> {
        None
    }
    fn set_double_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn set_static_double_field(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn call_void_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
    fn call_static_void_method(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<()> {
        None
    }
}
impl JniHandler for () {}
pub(super) fn dispatch(
    handler: &mut dyn JniHandler,
    emu: &mut Emulator,
    call: &JniCall<'_>,
    operation: u8,
    kind: usize,
) -> Option<JniValue> {
    match (operation, kind) {
        (0, 0) => handler.call_object_method(emu, call).map(JniValue::Object),
        (1, 0) => handler
            .call_static_object_method(emu, call)
            .map(JniValue::Object),
        (2, 0) => handler.get_object_field(emu, call).map(JniValue::Object),
        (3, 0) => handler
            .get_static_object_field(emu, call)
            .map(JniValue::Object),
        (4, 0) => handler.set_object_field(emu, call).map(|_| JniValue::Void),
        (5, 0) => handler
            .set_static_object_field(emu, call)
            .map(|_| JniValue::Void),
        (0, 1) => handler
            .call_boolean_method(emu, call)
            .map(JniValue::Boolean),
        (1, 1) => handler
            .call_static_boolean_method(emu, call)
            .map(JniValue::Boolean),
        (2, 1) => handler.get_boolean_field(emu, call).map(JniValue::Boolean),
        (3, 1) => handler
            .get_static_boolean_field(emu, call)
            .map(JniValue::Boolean),
        (4, 1) => handler.set_boolean_field(emu, call).map(|_| JniValue::Void),
        (5, 1) => handler
            .set_static_boolean_field(emu, call)
            .map(|_| JniValue::Void),
        (0, 2) => handler.call_byte_method(emu, call).map(JniValue::Byte),
        (1, 2) => handler
            .call_static_byte_method(emu, call)
            .map(JniValue::Byte),
        (2, 2) => handler.get_byte_field(emu, call).map(JniValue::Byte),
        (3, 2) => handler.get_static_byte_field(emu, call).map(JniValue::Byte),
        (4, 2) => handler.set_byte_field(emu, call).map(|_| JniValue::Void),
        (5, 2) => handler
            .set_static_byte_field(emu, call)
            .map(|_| JniValue::Void),
        (0, 3) => handler.call_char_method(emu, call).map(JniValue::Char),
        (1, 3) => handler
            .call_static_char_method(emu, call)
            .map(JniValue::Char),
        (2, 3) => handler.get_char_field(emu, call).map(JniValue::Char),
        (3, 3) => handler.get_static_char_field(emu, call).map(JniValue::Char),
        (4, 3) => handler.set_char_field(emu, call).map(|_| JniValue::Void),
        (5, 3) => handler
            .set_static_char_field(emu, call)
            .map(|_| JniValue::Void),
        (0, 4) => handler.call_short_method(emu, call).map(JniValue::Short),
        (1, 4) => handler
            .call_static_short_method(emu, call)
            .map(JniValue::Short),
        (2, 4) => handler.get_short_field(emu, call).map(JniValue::Short),
        (3, 4) => handler
            .get_static_short_field(emu, call)
            .map(JniValue::Short),
        (4, 4) => handler.set_short_field(emu, call).map(|_| JniValue::Void),
        (5, 4) => handler
            .set_static_short_field(emu, call)
            .map(|_| JniValue::Void),
        (0, 5) => handler.call_int_method(emu, call).map(JniValue::Int),
        (1, 5) => handler.call_static_int_method(emu, call).map(JniValue::Int),
        (2, 5) => handler.get_int_field(emu, call).map(JniValue::Int),
        (3, 5) => handler.get_static_int_field(emu, call).map(JniValue::Int),
        (4, 5) => handler.set_int_field(emu, call).map(|_| JniValue::Void),
        (5, 5) => handler
            .set_static_int_field(emu, call)
            .map(|_| JniValue::Void),
        (0, 6) => handler.call_long_method(emu, call).map(JniValue::Long),
        (1, 6) => handler
            .call_static_long_method(emu, call)
            .map(JniValue::Long),
        (2, 6) => handler.get_long_field(emu, call).map(JniValue::Long),
        (3, 6) => handler.get_static_long_field(emu, call).map(JniValue::Long),
        (4, 6) => handler.set_long_field(emu, call).map(|_| JniValue::Void),
        (5, 6) => handler
            .set_static_long_field(emu, call)
            .map(|_| JniValue::Void),
        (0, 7) => handler.call_float_method(emu, call).map(JniValue::Float),
        (1, 7) => handler
            .call_static_float_method(emu, call)
            .map(JniValue::Float),
        (2, 7) => handler.get_float_field(emu, call).map(JniValue::Float),
        (3, 7) => handler
            .get_static_float_field(emu, call)
            .map(JniValue::Float),
        (4, 7) => handler.set_float_field(emu, call).map(|_| JniValue::Void),
        (5, 7) => handler
            .set_static_float_field(emu, call)
            .map(|_| JniValue::Void),
        (0, 8) => handler.call_double_method(emu, call).map(JniValue::Double),
        (1, 8) => handler
            .call_static_double_method(emu, call)
            .map(JniValue::Double),
        (2, 8) => handler.get_double_field(emu, call).map(JniValue::Double),
        (3, 8) => handler
            .get_static_double_field(emu, call)
            .map(JniValue::Double),
        (4, 8) => handler.set_double_field(emu, call).map(|_| JniValue::Void),
        (5, 8) => handler
            .set_static_double_field(emu, call)
            .map(|_| JniValue::Void),
        (0, 9) => handler.call_void_method(emu, call).map(|_| JniValue::Void),
        (1, 9) => handler
            .call_static_void_method(emu, call)
            .map(|_| JniValue::Void),
        _ => None,
    }
}
