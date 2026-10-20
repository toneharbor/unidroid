use crate::{Emulator, Handle, JniCall, JniHandler, JniValue, Module, PrimitiveType, Result};
use base64::{Engine, prelude::BASE64_STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

mod wire;
use wire::{Address, Method, decode, encode};

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Command {
    Load {
        path: String,
    },
    Alias {
        name: String,
        target: String,
    },
    Superclass {
        class: String,
        superclass: String,
    },
    FindSymbol {
        module: usize,
        name: String,
    },
    Onload {
        module: usize,
    },
    CallStatic {
        class: String,
        method: Method,
        args: Vec<Value>,
    },
    Call {
        class: String,
        receiver: u64,
        method: Method,
        args: Vec<Value>,
    },
    CallFunction {
        address: Address,
        args: Vec<Address>,
    },
    Malloc {
        size: usize,
    },
    Free {
        address: Address,
    },
    NewCstring {
        value: String,
    },
    Read {
        address: Address,
        size: usize,
    },
    Write {
        address: Address,
        base64: String,
    },
    NewString {
        value: String,
    },
    String {
        handle: u64,
    },
    NewArray {
        r#type: String,
        base64: String,
    },
    Array {
        handle: u64,
    },
    NewObjectArray {
        class: String,
        items: Vec<u64>,
    },
    ObjectArray {
        handle: u64,
    },
    NewObject {
        class: String,
    },
    Class {
        name: String,
    },
    Release {
        handle: u64,
    },
    Answer {
        #[serde(default)]
        value: Value,
    },
    Unhandled,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Response {
    Ok(Value),
    Error(String),
    Jni(Value),
}

/// Serve JSON Lines commands, including reentrant JNI questions, until stdin closes.
pub fn serve(emu: &mut Emulator, input: impl BufRead, output: impl Write) -> Result<()> {
    let mut session = Session {
        input,
        output,
        error: None,
        fatal: false,
    };
    while let Some(command) = session.read()? {
        session.respond(emu, command)?;
    }
    Ok(())
}

struct Session<R, W> {
    input: R,
    output: W,
    // JniHandler returns Option, so errors must cross the handler boundary here.
    error: Option<Box<dyn std::error::Error>>,
    fatal: bool,
}
impl<R: BufRead, W: Write> Session<R, W> {
    fn read(&mut self) -> Result<Option<std::result::Result<Command, serde_json::Error>>> {
        let mut line = String::new();
        if self.input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        Ok(Some(serde_json::from_str(&line)))
    }
    fn write(&mut self, response: Response) -> Result<()> {
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
        if let Some(error) = self.error.take() {
            result = Err(error);
        }
        if self.fatal {
            return result.map(|_| ());
        }
        self.write(match result {
            Ok(value) => Response::Ok(value),
            Err(error) => Response::Error(error.to_string()),
        })
    }
    fn execute(&mut self, emu: &mut Emulator, command: Command) -> Result<Value> {
        Ok(match command {
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
                    emu.invoke(address.0, args.iter().map(|arg| (b'L', arg.0)), b'L', self)?
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
        })
    }
    fn wait(
        &mut self,
        emu: &mut Emulator,
        question: Value,
        kind: u8,
        context: &str,
    ) -> Result<Option<JniValue>> {
        self.write(Response::Jni(question))?;
        loop {
            let command = self
                .read()?
                .ok_or("stdin closed while a JNI question was waiting for an answer")?;
            match command {
                Ok(Command::Answer { value }) => {
                    return match decode(kind, &value, context) {
                        Ok(value) => Ok(Some(value)),
                        Err(error) => {
                            self.error = Some(error);
                            Ok(None)
                        }
                    };
                }
                Ok(Command::Unhandled) => return Ok(None),
                command => self.respond(emu, command)?,
            }
        }
    }
    fn question(
        &mut self,
        emu: &mut Emulator,
        question: Value,
        kind: u8,
        context: &str,
    ) -> Option<JniValue> {
        match self.wait(emu, question, kind, context) {
            Ok(value) => value,
            Err(error) => {
                self.error = Some(error);
                self.fatal = true;
                None
            }
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
