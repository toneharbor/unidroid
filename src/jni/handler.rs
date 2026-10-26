use super::{Handle, JniValue};
use crate::{Emulator, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JniOp {
    Call,
    Get,
    Set,
    New,
}
impl JniOp {
    pub fn name(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::Get => "get",
            Self::Set => "set",
            Self::New => "new",
        }
    }
}

/// Decoded member access delivered to a handler. Objects are handles into this emulator.
pub struct JniCall<'a> {
    pub op: JniOp,
    pub is_static: bool,
    /// Expected answer: Z B C S I J F D, L for references/new, V for void/set.
    pub type_code: u8,
    pub class: &'a str,
    pub name: &'a str,
    pub signature: &'a str,
    pub receiver: Handle,
    pub args: &'a [JniValue],
}

pub trait JniHandler {
    fn import(&mut self, _emu: &mut Emulator, _call: &crate::ImportCall<'_>) -> Option<u64> {
        None
    }
    fn stopped(&mut self, _emu: &mut Emulator, _stop: &crate::Stop) -> crate::Resume {
        crate::Resume::Continue
    }
    fn is_assignable(&mut self, _emu: &mut Emulator, _from: &str, _to: &str) -> Option<bool> {
        None
    }
    fn member(&mut self, _emu: &mut Emulator, _call: &JniCall<'_>) -> Option<JniValue> {
        None
    }
}
impl JniHandler for () {}

pub(super) fn check_answer(
    emu: &Emulator,
    call: &JniCall<'_>,
    function: &str,
    answer: Option<JniValue>,
) -> Result<Option<JniValue>> {
    if let Some(value) = answer
        && value.type_code() != call.type_code
    {
        return Err(emu
            .jni_message(format!(
                "wrong JNI answer type: {function} {}->{}{} expected {}, got {}",
                call.class,
                call.name,
                call.signature,
                char::from(call.type_code),
                char::from(value.type_code())
            ))
            .into());
    }
    Ok(answer)
}
