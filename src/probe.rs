use crate::{Emulator, Handle, JniCall, JniHandler};

#[derive(Debug)]
pub struct ProbedCall {
    pub op: &'static str,
    pub is_static: bool,
    pub class: String,
    pub name: String,
    pub signature: String,
    pub count: usize,
    pub from: String,
}

#[derive(Default)]
pub struct Probe {
    calls: Vec<ProbedCall>,
}
impl Probe {
    pub fn calls(&self) -> &[ProbedCall] {
        &self.calls
    }

    fn record(&mut self, emu: &Emulator, call: &JniCall<'_>, op: &'static str, is_static: bool) {
        if let Some(seen) = self.calls.iter_mut().find(|c| {
            c.op == op
                && c.is_static == is_static
                && c.class == call.class
                && c.name == call.name
                && c.signature == call.signature
        }) {
            seen.count += 1;
        } else {
            self.calls.push(ProbedCall {
                op,
                is_static,
                class: call.class.into(),
                name: call.name.into(),
                signature: call.signature.into(),
                count: 1,
                from: emu.location(emu.lr().unwrap_or(0)),
            });
        }
    }
}

macro_rules! handlers {
    ($(($name:ident, $op:literal, $static:literal, $result:ty, $value:expr)),* $(,)?) => {
        $(fn $name(&mut self, emu: &mut Emulator, call: &JniCall<'_>) -> Option<$result> {
            self.record(emu, call, $op, $static);
            Some($value)
        })*
    };
}

impl JniHandler for Probe {
    fn is_assignable(&mut self, emu: &mut Emulator, from: &str, to: &str) -> Option<bool> {
        self.record(
            emu,
            &JniCall {
                class: from,
                name: to,
                signature: "",
                receiver: Handle(0),
                args: &[],
            },
            "is_assignable",
            false,
        );
        Some(false)
    }
    handlers! {
        (new_object, "new", false, Handle, Handle(0)),
        (call_object_method, "call", false, Handle, Handle(0)),
        (call_static_object_method, "call", true, Handle, Handle(0)),
        (get_object_field, "get", false, Handle, Handle(0)),
        (get_static_object_field, "get", true, Handle, Handle(0)),
        (set_object_field, "set", false, (), ()),
        (set_static_object_field, "set", true, (), ()),
        (call_boolean_method, "call", false, bool, false),
        (call_static_boolean_method, "call", true, bool, false),
        (get_boolean_field, "get", false, bool, false),
        (get_static_boolean_field, "get", true, bool, false),
        (set_boolean_field, "set", false, (), ()),
        (set_static_boolean_field, "set", true, (), ()),
        (call_byte_method, "call", false, i8, 0),
        (call_static_byte_method, "call", true, i8, 0),
        (get_byte_field, "get", false, i8, 0),
        (get_static_byte_field, "get", true, i8, 0),
        (set_byte_field, "set", false, (), ()),
        (set_static_byte_field, "set", true, (), ()),
        (call_char_method, "call", false, u16, 0),
        (call_static_char_method, "call", true, u16, 0),
        (get_char_field, "get", false, u16, 0),
        (get_static_char_field, "get", true, u16, 0),
        (set_char_field, "set", false, (), ()),
        (set_static_char_field, "set", true, (), ()),
        (call_short_method, "call", false, i16, 0),
        (call_static_short_method, "call", true, i16, 0),
        (get_short_field, "get", false, i16, 0),
        (get_static_short_field, "get", true, i16, 0),
        (set_short_field, "set", false, (), ()),
        (set_static_short_field, "set", true, (), ()),
        (call_int_method, "call", false, i32, 0),
        (call_static_int_method, "call", true, i32, 0),
        (get_int_field, "get", false, i32, 0),
        (get_static_int_field, "get", true, i32, 0),
        (set_int_field, "set", false, (), ()),
        (set_static_int_field, "set", true, (), ()),
        (call_long_method, "call", false, i64, 0),
        (call_static_long_method, "call", true, i64, 0),
        (get_long_field, "get", false, i64, 0),
        (get_static_long_field, "get", true, i64, 0),
        (set_long_field, "set", false, (), ()),
        (set_static_long_field, "set", true, (), ()),
        (call_float_method, "call", false, f32, 0.0),
        (call_static_float_method, "call", true, f32, 0.0),
        (get_float_field, "get", false, f32, 0.0),
        (get_static_float_field, "get", true, f32, 0.0),
        (set_float_field, "set", false, (), ()),
        (set_static_float_field, "set", true, (), ()),
        (call_double_method, "call", false, f64, 0.0),
        (call_static_double_method, "call", true, f64, 0.0),
        (get_double_field, "get", false, f64, 0.0),
        (get_static_double_field, "get", true, f64, 0.0),
        (set_double_field, "set", false, (), ()),
        (set_static_double_field, "set", true, (), ()),
        (call_void_method, "call", false, (), ()),
        (call_static_void_method, "call", true, (), ()),
    }
}
