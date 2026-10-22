use crate::{Emulator, Handle, JniCall, JniHandler, JniValue, Module, PrimitiveType, Result};
use base64::{Engine, prelude::BASE64_STANDARD};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

mod options;
pub use options::Options;
mod journal;
mod recording;
pub use recording::{record, replay};
mod response;
use response::{Response, crash_value};
mod wire;
use wire::{decode, encode};
mod command;
use command::Command;
mod handler;
mod waiting;
use waiting::{Question, Reply};

/// Serve JSON Lines commands, including reentrant JNI questions, until stdin closes.
pub fn serve(emu: &mut Emulator, input: impl BufRead, output: impl Write) -> Result<()> {
    let mut session = Session {
        input,
        output,
        error: None,
        fatal: false,
        journal: None,
    };
    session.run(emu)
}

struct Session<R, W> {
    input: R,
    output: W,
    // JniHandler returns Option, so errors must cross the handler boundary here.
    error: Option<Box<dyn std::error::Error>>,
    fatal: bool,
    journal: Option<journal::Shared>,
}
impl<R: BufRead, W: Write> Session<R, W> {
    fn run(&mut self, emu: &mut Emulator) -> Result<()> {
        while let Some(command) = self.read()? {
            self.respond(emu, command)?;
        }
        Ok(())
    }
    fn read(&mut self) -> Result<Option<std::result::Result<Command, serde_json::Error>>> {
        if let Some(journal) = &self.journal
            && journal.borrow().is_replay()
        {
            return Ok(journal
                .borrow_mut()
                .replay_input()?
                .map(|line| serde_json::from_str(&line)));
        }
        let mut line = String::new();
        if self.input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if let Some(journal) = &self.journal {
            let line = line
                .strip_suffix("\r\n")
                .or_else(|| line.strip_suffix('\n'))
                .unwrap_or(&line);
            journal.borrow_mut().input(line)?;
        }
        Ok(Some(serde_json::from_str(&line)))
    }
    fn write(&mut self, response: Response) -> Result<()> {
        if let Some(journal) = &self.journal {
            journal
                .borrow_mut()
                .output(serde_json::to_value(&response)?)?;
        }
        serde_json::to_writer(&mut self.output, &response)?;
        self.output.write_all(b"\n")?;
        self.output.flush()?;
        Ok(())
    }
    fn respond(
        &mut self,
        emu: &mut Emulator,
        command: std::result::Result<Command, serde_json::Error>,
    ) -> Result<()> {
        let mut result = match command {
            Ok(command) => self.execute(emu, command),
            Err(error) => Err(format!("bad message: {error}").into()),
        };
        let crash = result
            .as_ref()
            .err()
            .and_then(|error| error.downcast_ref::<crate::Crash>())
            .map(crash_value);
        if let Some(error) = self.error.take() {
            result = Err(error);
        }
        if self.fatal {
            return result.map(|_| ());
        }
        self.write(match result {
            Ok(response) => response,
            Err(error) => Response::Error {
                error: error.to_string(),
                crash,
            },
        })
    }
    fn execute(&mut self, emu: &mut Emulator, command: Command) -> Result<Response> {
        Ok(Response::Ok(match command {
            Command::Break { at } => {
                let address = emu.resolve(&at)? & !1;
                emu.add_breakpoint(address)?;
                json!(address.to_string())
            }
            Command::Unbreak { at } => {
                emu.remove_breakpoint(emu.resolve(&at)?)?;
                Value::Null
            }
            Command::Watch { at, size, kind } => {
                let address = emu.resolve(&at)?;
                emu.add_watchpoint(address, size, kind)?;
                json!(address.to_string())
            }
            Command::Unwatch { at } => {
                emu.remove_watchpoint(emu.resolve(&at)?)?;
                Value::Null
            }
            Command::Resolve { at } => json!(emu.resolve(&at)?.to_string()),
            Command::Registers => return Ok(Response::Registers(response::registers_value(&emu.registers()?))),
            Command::SetRegister { name, value } => {
                emu.set_register(&name, value.0)?;
                Value::Null
            }
            Command::Backtrace => response::frames_value(&emu.backtrace()?),
            Command::Modules => json!(emu.modules().iter().map(|m| json!({
                "name": m.name, "path": m.path, "base": m.base.to_string(), "size": m.size.to_string(),
            })).collect::<Vec<_>>()),
            Command::Maps => json!(emu.maps()?.iter().map(|m| json!({
                "start": m.start.to_string(), "end": m.end.to_string(), "perms": m.perms, "name": m.name,
            })).collect::<Vec<_>>()),
            Command::Search { base64 } => {
                let addresses = emu.search(&BASE64_STANDARD.decode(base64)?)?;
                let maps = emu.maps()?;
                let modules = emu.modules();
                json!(addresses.into_iter().map(|address| json!({
                    "address": address.to_string(),
                    "location": crate::debug::memory_location(address, &modules, maps.iter().find(|m| m.start <= address && address < m.end)),
                })).collect::<Vec<_>>())
            }
            Command::Crypto => json!(emu
                .crypto_constants()?
                .into_iter()
                .map(|constant| json!({
                    "name": constant.name,
                    "address": constant.address.to_string(),
                    "location": constant.location,
                }))
                .collect::<Vec<_>>()),
            Command::Snapshot { path } => {
                emu.snapshot()?.save(path)?;
                Value::Null
            }
            Command::Continue | Command::Step | Command::Finish | Command::Return { .. } | Command::Abort => {
                return Err("no breakpoint stop is waiting".into());
            }
            Command::Trace {
                jni,
                syscalls,
                imports,
                code,
                calls,
            } => {
                emu.set_trace(crate::Trace {
                    jni,
                    syscalls,
                    imports,
                    code,
                    calls,
                })?;
                Value::Null
            }
            Command::Load { path } => json!(emu.load_library(path)?.0),
            Command::Alias { name, target } => {
                emu.symbol_alias(name, target);
                Value::Null
            }
            Command::Superclass { class, superclass } => {
                emu.superclass(class, superclass);
                Value::Null
            }
            Command::FindSymbol { module, name } => {
                json!(emu.find_symbol(Module(module), &name)?.to_string())
            }
            Command::Onload { module } => json!(emu.call_jni_onload(Module(module), self)?),
            Command::CallStatic {
                class,
                method,
                args,
            } => {
                let args = method.arguments(&args)?;
                encode(emu.call_static_jni_method(self, &class, &method.name, &args)?)
            }
            Command::Call {
                class,
                receiver,
                method,
                args,
            } => {
                let args = method.arguments(&args)?;
                encode(emu.call_jni_method(self, &class, Handle(receiver), &method.name, &args)?)
            }
            Command::CallFunction { address, args } => {
                json!(
                    emu.call_function_with(self, address.0, &args.iter().map(|arg| arg.0).collect::<Vec<_>>())?
                        .to_string()
                )
            }
            Command::Malloc { size } => json!(emu.malloc(size)?.to_string()),
            Command::Free { address } => {
                emu.free(address.0);
                Value::Null
            }
            Command::NewCstring { value } => json!(emu.new_cstring(&value)?.to_string()),
            Command::Read { address, size } => {
                json!(BASE64_STANDARD.encode(emu.read_bytes(address.0, size)?))
            }
            Command::Write { address, base64 } => {
                emu.write_bytes(address.0, &BASE64_STANDARD.decode(base64)?)?;
                Value::Null
            }
            Command::NewString { value } => json!(emu.new_string(&value).0),
            Command::String { handle } => json!(emu.string(Handle(handle))?),
            Command::NewArray { r#type, base64 } => {
                let kind = match r#type.as_str() {
                    "Z" => PrimitiveType::Boolean,
                    "B" => PrimitiveType::Byte,
                    "C" => PrimitiveType::Char,
                    "S" => PrimitiveType::Short,
                    "I" => PrimitiveType::Int,
                    "J" => PrimitiveType::Long,
                    "F" => PrimitiveType::Float,
                    "D" => PrimitiveType::Double,
                    _ => return Err(format!("unknown array type: {type}").into()),
                };
                json!(
                    emu.new_primitive_array(kind, &BASE64_STANDARD.decode(base64)?)
                        .0
                )
            }
            Command::Array { handle } => {
                json!(BASE64_STANDARD.encode(emu.primitive_array(Handle(handle))?))
            }
            Command::NewObjectArray { class, items } => json!(
                emu.new_object_array(&class, &items.into_iter().map(Handle).collect::<Vec<_>>())?
                    .0
            ),
            Command::ObjectArray { handle } => json!(
                emu.object_array(Handle(handle))?
                    .into_iter()
                    .map(|handle| handle.0)
                    .collect::<Vec<_>>()
            ),
            Command::NewObject { class } => json!(emu.new_object(&class, ()).0),
            Command::Class { name } => json!(emu.class(&name).0),
            Command::Release { handle } => {
                emu.release(Handle(handle));
                Value::Null
            }
            Command::Answer { .. } | Command::Unhandled => {
                return Err("no JNI question is waiting for an answer".into());
            }
        }))
    }
    fn question(
        &mut self,
        emu: &mut Emulator,
        question: Value,
        kind: u8,
        context: &str,
    ) -> Option<JniValue> {
        match self.ask(
            emu,
            Response::Jni(question),
            Question::Jni { kind, context },
        ) {
            Some(Reply::Jni(value)) => value,
            _ => None,
        }
    }
    fn member(
        &mut self,
        emu: &mut Emulator,
        call: &JniCall<'_>,
        op: &str,
        is_static: bool,
        kind: u8,
    ) -> Option<JniValue> {
        let question = json!({
            "op":op, "static":is_static, "type":char::from(kind).to_string(),
            "class":call.class, "name":call.name, "signature":call.signature,
            "receiver":call.receiver.0, "args":call.args.iter().copied().map(encode).collect::<Vec<_>>()
        });
        self.question(
            emu,
            question,
            kind,
            &format!("answer for {}->{}{}", call.class, call.name, call.signature),
        )
    }
}
