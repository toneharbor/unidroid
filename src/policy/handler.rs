use super::Policy;
use crate::{Emulator, Handle, JniCall, JniHandler, JniValue, Result};

/// Pass to an emulator call, then use `finish` to surface policy answer errors.
pub struct PolicyHandler<'a> {
    pub(super) policy: &'a mut Policy,
    pub(super) fallback: &'a mut dyn JniHandler,
    pub(super) error: Option<Box<dyn std::error::Error>>,
}

impl PolicyHandler<'_> {
    /// Return a policy error, if any, otherwise the emulator call's result.
    pub fn finish<T>(&mut self, result: Result<T>) -> Result<T> {
        match self.error.take() {
            Some(error) => Err(error),
            None => result,
        }
    }

    fn answer(&mut self, result: Result<JniValue>) -> Option<JniValue> {
        match result {
            Ok(value) => Some(value),
            Err(error) => {
                self.error = Some(error);
                None
            }
        }
    }
}

macro_rules! handlers {
    ($(($name:ident, $op:literal, $static:literal, $kind:literal, $variant:ident, $result:ty)),* $(,)?) => {
        $(fn $name(&mut self, emu: &mut Emulator, call: &JniCall<'_>) -> Option<$result> {
            match self.policy.member(emu, call, $op, $static, $kind) {
                None => self.fallback.$name(emu, call),
                Some(result) => match self.answer(result) {
                    Some(JniValue::$variant(value)) => Some(value),
                    _ => None,
                }
            }
        })*
    };
}
macro_rules! void_handlers {
    ($(($name:ident, $op:literal, $static:literal)),* $(,)?) => {
        $(fn $name(&mut self, emu: &mut Emulator, call: &JniCall<'_>) -> Option<()> {
            match self.policy.member(emu, call, $op, $static, b'V') {
                None => self.fallback.$name(emu, call),
                Some(result) => self.answer(result).map(|_| ()),
            }
        })*
    };
}

macro_rules! members {
    ($value:ident, $void:ident) => {
        $value! {
            (new_object, "new", false, b'L', Object, Handle),
            (call_object_method, "call", false, b'L', Object, Handle),
            (call_static_object_method, "call", true, b'L', Object, Handle),
            (get_object_field, "get", false, b'L', Object, Handle),
            (get_static_object_field, "get", true, b'L', Object, Handle),
            (call_boolean_method, "call", false, b'Z', Boolean, bool),
            (call_static_boolean_method, "call", true, b'Z', Boolean, bool),
            (get_boolean_field, "get", false, b'Z', Boolean, bool),
            (get_static_boolean_field, "get", true, b'Z', Boolean, bool),
            (call_byte_method, "call", false, b'B', Byte, i8),
            (call_static_byte_method, "call", true, b'B', Byte, i8),
            (get_byte_field, "get", false, b'B', Byte, i8),
            (get_static_byte_field, "get", true, b'B', Byte, i8),
            (call_char_method, "call", false, b'C', Char, u16),
            (call_static_char_method, "call", true, b'C', Char, u16),
            (get_char_field, "get", false, b'C', Char, u16),
            (get_static_char_field, "get", true, b'C', Char, u16),
            (call_short_method, "call", false, b'S', Short, i16),
            (call_static_short_method, "call", true, b'S', Short, i16),
            (get_short_field, "get", false, b'S', Short, i16),
            (get_static_short_field, "get", true, b'S', Short, i16),
            (call_int_method, "call", false, b'I', Int, i32),
            (call_static_int_method, "call", true, b'I', Int, i32),
            (get_int_field, "get", false, b'I', Int, i32),
            (get_static_int_field, "get", true, b'I', Int, i32),
            (call_long_method, "call", false, b'J', Long, i64),
            (call_static_long_method, "call", true, b'J', Long, i64),
            (get_long_field, "get", false, b'J', Long, i64),
            (get_static_long_field, "get", true, b'J', Long, i64),
            (call_float_method, "call", false, b'F', Float, f32),
            (call_static_float_method, "call", true, b'F', Float, f32),
            (get_float_field, "get", false, b'F', Float, f32),
            (get_static_float_field, "get", true, b'F', Float, f32),
            (call_double_method, "call", false, b'D', Double, f64),
            (call_static_double_method, "call", true, b'D', Double, f64),
            (get_double_field, "get", false, b'D', Double, f64),
            (get_static_double_field, "get", true, b'D', Double, f64),
        }
        $void! {
            (call_void_method, "call", false), (call_static_void_method, "call", true),
            (set_object_field, "set", false), (set_static_object_field, "set", true),
            (set_boolean_field, "set", false), (set_static_boolean_field, "set", true),
            (set_byte_field, "set", false), (set_static_byte_field, "set", true),
            (set_char_field, "set", false), (set_static_char_field, "set", true),
            (set_short_field, "set", false), (set_static_short_field, "set", true),
            (set_int_field, "set", false), (set_static_int_field, "set", true),
            (set_long_field, "set", false), (set_static_long_field, "set", true),
            (set_float_field, "set", false), (set_static_float_field, "set", true),
            (set_double_field, "set", false), (set_static_double_field, "set", true),
        }
    };
}
pub(crate) use members;

impl JniHandler for PolicyHandler<'_> {
    fn stopped(&mut self, emu: &mut Emulator, stop: &crate::Stop) -> crate::Resume {
        self.fallback.stopped(emu, stop)
    }
    fn is_assignable(&mut self, emu: &mut Emulator, from: &str, to: &str) -> Option<bool> {
        match self.policy.is_assignable(emu, from, to) {
            None => self.fallback.is_assignable(emu, from, to),
            Some(result) => match self.answer(result) {
                Some(JniValue::Boolean(value)) => Some(value),
                _ => None,
            },
        }
    }
    members!(handlers, void_handlers);
}
