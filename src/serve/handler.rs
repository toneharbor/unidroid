use super::*;

impl<R: BufRead, W: Write> JniHandler for Session<R, W> {
    fn import(&mut self, emu: &mut Emulator, call: &crate::ImportCall<'_>) -> Option<u64> {
        let question = json!({
            "name":call.name, "args":call.args.iter().map(u64::to_string).collect::<Vec<_>>(),
            "original":call.original.map(|address| address.to_string()),
            "from":emu.location(call.return_address),
        });
        match self.ask(emu, Response::Import(question), Question::Import) {
            Some(Reply::Import(value)) => value,
            _ => {
                if let Some(error) = &self.error {
                    // Carry the failure across the Option-returning handler boundary.
                    emu.cpu_mut()
                        .expect("import callback has a CPU")
                        .get_data_mut()
                        .source_error = Some((call.return_address, error.to_string().into()));
                }
                None
            }
        }
    }
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
        let result = self
            .policy
            .as_mut()
            .and_then(|policy| policy.is_assignable(emu, from, to));
        let answer = match result {
            Some(result) => self.policy_answer(result),
            None => self.question(
                emu,
                json!({"op":"is_assignable", "from":from, "to":to}),
                b'Z',
                &format!("answer for {from}->{to}"),
            ),
        };
        match answer {
            Some(JniValue::Boolean(value)) => Some(value),
            _ => None,
        }
    }
    fn member(&mut self, emu: &mut Emulator, call: &JniCall<'_>) -> Option<JniValue> {
        Session::member(self, emu, call)
    }
}
