use crate::{Emulator, Result};
use serde::{Serialize, Serializer};
use std::{fmt, io::Write};

#[derive(Clone, Default)]
pub struct Trace {
    pub jni: bool,
    pub syscalls: bool,
    pub imports: bool,
    pub heap: bool,
    pub code: Vec<String>,
    pub calls: Vec<String>,
    pub coverage: Vec<String>,
}

impl Trace {
    pub(crate) fn validate(&self) -> Result<()> {
        for entry in self.code.iter().chain(&self.calls).chain(&self.coverage) {
            super::code::Spec::parse(entry)?;
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct MemoryAccess {
    #[serde(rename = "access", serialize_with = "access_kind")]
    pub write: bool,
    #[serde(serialize_with = "hex_address")]
    pub address: u64,
    pub size: usize,
    #[serde(serialize_with = "hex_address")]
    pub value: u64,
}

fn access_kind<S: Serializer>(write: &bool, serializer: S) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_str(if *write { "write" } else { "read" })
}

#[derive(Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Output {
        fd: u64,
        text: String,
    },
    Jni {
        function: String,
        details: String,
        from: String,
    },
    JniCall {
        function: String,
        class: String,
        name: String,
        signature: String,
        args: Vec<String>,
        from: String,
    },
    Syscall {
        name: String,
        number: u64,
        args: Vec<String>,
        result: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        errno: Option<String>,
        at: String,
        from: String,
    },
    Call {
        function: String,
        module: String,
        args: Vec<String>,
        from: String,
    },
    FunctionCall {
        to: String,
        #[serde(serialize_with = "hex_address")]
        address: u64,
        args: Vec<String>,
        from: String,
    },
    Instruction {
        at: String,
        #[serde(serialize_with = "hex_address")]
        address: u64,
        bytes: String,
        text: String,
        #[serde(serialize_with = "hex_registers")]
        read: Vec<(String, u128)>,
        #[serde(serialize_with = "hex_registers")]
        written: Vec<(String, u128)>,
        memory: Vec<MemoryAccess>,
    },
    Watch {
        at: String,
        #[serde(serialize_with = "hex_address")]
        pc: u64,
        #[serde(serialize_with = "hex_address")]
        address: u64,
        size: usize,
        write: bool,
        #[serde(serialize_with = "hex_address")]
        value: u64,
        thread: u32,
        callers: Vec<String>,
    },
    Tracepoint {
        at: String,
        #[serde(serialize_with = "hex_address")]
        address: u64,
        thread: u32,
        #[serde(serialize_with = "hex_values")]
        values: Vec<(String, Option<u64>)>,
    },
    Warning {
        message: String,
    },
}

fn hex_address<S: Serializer>(value: &u64, serializer: S) -> std::result::Result<S::Ok, S::Error> {
    serializer.collect_str(&format_args!("0x{value:x}"))
}

fn hex_registers<S: Serializer>(
    registers: &[(String, u128)],
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.collect_seq(
        registers
            .iter()
            .map(|(name, value)| (name, format!("0x{value:x}"))),
    )
}

fn hex_values<S: Serializer>(
    values: &[(String, Option<u64>)],
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.collect_seq(
        values
            .iter()
            .map(|(name, value)| (name, value.map(|value| format!("0x{value:x}")))),
    )
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Output { fd, text } => write!(
                f,
                "{}: {}",
                if *fd == 1 { "stdout" } else { "stderr" },
                text.strip_suffix('\n').unwrap_or(text)
            ),
            Self::Jni {
                function,
                details,
                from,
            } => write!(f, "JNIEnv->{function}({details}) from {from}"),
            Self::JniCall {
                function,
                class,
                name,
                signature,
                args,
                from,
            } => write!(
                f,
                "JNIEnv->{function}({class}->{name}{signature}, [{}]) from {from}",
                args.join(", ")
            ),
            Self::Syscall {
                name,
                args,
                result,
                errno,
                at,
                from,
                ..
            } => {
                write!(f, "syscall {name}({})", args.join(", "))?;
                if let Some(result) = result {
                    write!(f, " = {result}")?;
                    if let Some(errno) = errno {
                        write!(f, " {errno}")?;
                    }
                    write!(f, " at {at} from {from}")
                } else {
                    write!(f, " at {at} from {from} = (no return)")
                }
            }
            Self::Call {
                function,
                module,
                args,
                from,
            } => write!(
                f,
                "call {function}({}) in {module} from {from}",
                args.join(", ")
            ),
            Self::FunctionCall { to, args, from, .. } => {
                write!(f, "function {to}({}) from {from}", args.join(", "))
            }
            Self::Instruction {
                at,
                bytes,
                text,
                read,
                written,
                memory,
                ..
            } => {
                write!(f, "[{at}] [{bytes}] {text}")?;
                for (separator, registers) in [(" ; ", read), (" => ", written)] {
                    if !registers.is_empty() {
                        f.write_str(separator)?;
                        for (i, (name, value)) in registers.iter().enumerate() {
                            if i > 0 {
                                f.write_str(", ")?;
                            }
                            write!(f, "{name}=0x{value:x}")?;
                        }
                    }
                }
                for (i, access) in memory.iter().enumerate() {
                    f.write_str(if i == 0 { " | " } else { ", " })?;
                    write!(
                        f,
                        "{} 0x{:x}:{}=0x{:x}",
                        if access.write { "w" } else { "r" },
                        access.address,
                        access.size,
                        access.value
                    )?;
                }
                Ok(())
            }
            Self::Watch {
                at,
                address,
                size,
                write,
                value,
                thread,
                callers,
                ..
            } => {
                write!(
                    f,
                    "watch {} 0x{address:x}:{size}=0x{value:x} at {at} thread {thread}",
                    if *write { "write" } else { "read" }
                )?;
                if !callers.is_empty() {
                    write!(f, " from {}", callers.join(" <- "))?;
                }
                Ok(())
            }
            Self::Tracepoint {
                at, thread, values, ..
            } => {
                write!(f, "tracepoint {at} thread {thread}")?;
                for (name, value) in values {
                    write!(f, " {name}=")?;
                    match value {
                        Some(value) => write!(f, "0x{value:x}")?,
                        None => f.write_str("?")?,
                    }
                }
                Ok(())
            }
            Self::Warning { message } => write!(f, "warning: {message}"),
        }
    }
}

pub(crate) type EventHandler = Box<dyn FnMut(&Event)>;
pub(crate) struct Events {
    pub handler: Option<EventHandler>,
}
impl Events {
    pub fn emit(&mut self, event: Event) {
        if let Some(handler) = &mut self.handler {
            handler(&event);
        } else {
            let _ = writeln!(std::io::stderr().lock(), "{event}");
        }
    }
}
impl Emulator {
    pub fn trace(&self) -> &Trace {
        &self.trace
    }
    pub fn set_trace(&mut self, trace: Trace) -> Result<()> {
        trace.validate()?;
        self.apply_trace_selections(&trace)?;
        self.trace_imports(trace.imports)?;
        self.trace = trace;
        Ok(())
    }

    pub(crate) fn apply_trace_selections(&mut self, trace: &Trace) -> Result<()> {
        self.trace_code(&trace.code)?;
        self.trace_calls(&trace.calls)?;
        self.trace_coverage(&trace.coverage)?;
        self.trace_heap(trace.heap)
    }
}
