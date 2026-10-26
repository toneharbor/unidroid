use super::Policy;
use crate::{Emulator, JniCall, JniHandler, JniValue, Result};

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

impl JniHandler for PolicyHandler<'_> {
    fn import(&mut self, emu: &mut Emulator, call: &crate::ImportCall<'_>) -> Option<u64> {
        self.fallback.import(emu, call)
    }
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
    fn member(&mut self, emu: &mut Emulator, call: &JniCall<'_>) -> Option<JniValue> {
        match self.policy.member(emu, call) {
            Some(result) => self.answer(result),
            None => self.fallback.member(emu, call),
        }
    }
}
