use super::*;

macro_rules! handlers {
    ($(($name:ident, $op:literal, $static:literal, $kind:literal, $variant:ident, $result:ty)),* $(,)?) => {
        $(fn $name(&mut self, emu: &mut Emulator, call: &JniCall<'_>) -> Option<$result> {
            match self.member(emu, call, $op, $static, $kind) {
                Some(JniValue::$variant(value)) => Some(value),
                _ => None,
            }
        })*
    };
}
macro_rules! void_handlers {
    ($(($name:ident, $op:literal, $static:literal)),* $(,)?) => {
        $(fn $name(&mut self, emu: &mut Emulator, call: &JniCall<'_>) -> Option<()> {
            self.member(emu, call, $op, $static, b'V').map(|_| ())
        })*
    };
}

impl<R: BufRead, W: Write> JniHandler for Session<R, W> {
    fn stopped(&mut self, emu: &mut Emulator, stop: &crate::Stop) -> crate::Resume {
        let registers = match emu.registers() {
            Ok(registers) => response::registers_value(&registers),
            Err(error) => {
                self.error = Some(error);
                return crate::Resume::Abort;
            }
        };
        let question = response::break_value(stop, emu, registers);
        match self.ask(emu, Response::Break(question), Question::Stop) {
            Some(Reply::Resume(resume)) => resume,
            _ => crate::Resume::Abort,
        }
    }
    fn is_assignable(&mut self, emu: &mut Emulator, from: &str, to: &str) -> Option<bool> {
        match self.question(
            emu,
            json!({"op":"is_assignable", "from":from, "to":to}),
            b'Z',
            &format!("answer for {from}->{to}"),
        ) {
            Some(JniValue::Boolean(value)) => Some(value),
            _ => None,
        }
    }
    handlers! {
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
    void_handlers! {
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
}
