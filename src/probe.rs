use crate::{Emulator, JniCall, JniHandler, JniValue};

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

    fn record(
        &mut self,
        emu: &Emulator,
        op: &'static str,
        is_static: bool,
        class: &str,
        name: &str,
        signature: &str,
    ) {
        if let Some(seen) = self.calls.iter_mut().find(|c| {
            c.op == op
                && c.is_static == is_static
                && c.class == class
                && c.name == name
                && c.signature == signature
        }) {
            seen.count += 1;
        } else {
            self.calls.push(ProbedCall {
                op,
                is_static,
                class: class.into(),
                name: name.into(),
                signature: signature.into(),
                count: 1,
                from: emu.location(emu.lr().unwrap_or(0)),
            });
        }
    }
}

impl JniHandler for Probe {
    fn member(&mut self, emu: &mut Emulator, call: &JniCall<'_>) -> Option<JniValue> {
        self.record(
            emu,
            call.op.name(),
            call.is_static,
            call.class,
            call.name,
            call.signature,
        );
        Some(crate::jni::value::decode(call.type_code, 0))
    }
    fn is_assignable(&mut self, emu: &mut Emulator, from: &str, to: &str) -> Option<bool> {
        self.record(emu, "is_assignable", false, from, to, "");
        Some(false)
    }
}
