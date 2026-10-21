use crate::{Emulator, Result};
use serde::Serialize;
use std::{fmt, io::Write};

#[derive(Clone, Default)]
pub struct Trace {
    pub jni: bool,
    pub syscalls: bool,
    pub imports: bool,
}

#[derive(Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
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
    Warning {
        message: String,
    },
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
        self.trace_imports(trace.imports)?;
        self.trace = trace;
        Ok(())
    }
}
