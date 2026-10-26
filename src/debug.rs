use crate::{Emulator, RETURN, Result, TRAMPOLINE, arch::Architecture};
use serde::{Deserialize, Serialize};
use std::fmt::{self, Write};
use unicorn_engine::{RegisterARM, RegisterARM64};

mod arguments;
pub(crate) mod calls;
pub(crate) mod code;
pub(crate) mod control;
pub(crate) mod coverage;
pub(crate) mod crypto;
pub(crate) mod events;
mod expr;
pub(crate) mod heap;
mod hooks;
pub(crate) mod imports;
pub(crate) mod maps;
pub(crate) mod modules;
mod registers;
pub(crate) mod search;
pub(crate) mod snapshot;
mod syscalls;
pub(crate) mod tracepoint;
pub(crate) mod watch;

const STACK_WORDS: usize = 32;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Frame {
    pub address: u64,
    pub location: String,
    pub symbol: Option<String>,
}

impl fmt::Display for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.location)?;
        if let Some(symbol) = &self.symbol {
            write!(f, " ({symbol})")?;
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct Crash {
    pub message: String,
    pub thread: u32,
    pub registers: Vec<(&'static str, u64)>,
    pub backtrace: Vec<Frame>,
    pub stack: Vec<(u64, u64)>,
    pub jni: Vec<String>,
}

impl fmt::Display for Crash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Crash {}
impl Crash {
    pub fn report(&self) -> String {
        let mut text = format!("{}\nthread {}\n", self.message, self.thread);
        for (i, frame) in self.backtrace.iter().enumerate() {
            let _ = writeln!(text, "#{i} {frame}");
        }
        for row in self.registers.chunks(4) {
            for (i, (name, value)) in row.iter().enumerate() {
                if i > 0 {
                    text.push_str("  ");
                }
                let _ = write!(text, "{name}=0x{value:x}");
            }
            text.push('\n');
        }
        for row in self.stack.chunks(4) {
            let _ = write!(text, "0x{:x}:", row[0].0);
            for (_, value) in row {
                let _ = write!(text, " 0x{value:x}");
            }
            text.push('\n');
        }
        for call in &self.jni {
            let _ = writeln!(text, "{call}");
        }
        text.pop();
        text
    }
}

impl Emulator {
    pub(crate) fn crash(&self, error: Box<dyn std::error::Error>) -> Box<dyn std::error::Error> {
        if error.is::<Crash>() {
            return error;
        }
        Box::new(Crash {
            message: error.to_string(),
            thread: self.threads.tid(),
            registers: self.registers().unwrap_or_default(),
            backtrace: self.backtrace().unwrap_or_default(),
            stack: self.stack_window().unwrap_or_default(),
            jni: self.recent_jni(),
        })
    }

    fn stack_window(&self) -> Result<Vec<(u64, u64)>> {
        let sp = self.sp()?;
        let mut words = Vec::new();
        for i in 0..STACK_WORDS {
            let address = sp + i as u64 * self.pointer_size();
            let Ok(value) = self.read_pointer(address) else {
                break;
            };
            words.push((address, value));
        }
        Ok(words)
    }

    pub fn backtrace(&self) -> Result<Vec<Frame>> {
        let modules = self.module_table();
        Ok(
            backtrace_addresses(self.cpu()?, self.architecture(), false)?
                .into_iter()
                .map(|address| modules.frame(address))
                .collect(),
        )
    }

    pub fn symbolize(&self, address: u64) -> String {
        self.module_table().symbolize(address)
    }
}

// Allocator entry starts at its caller, including host-started calls at RETURN.
pub(crate) fn backtrace_addresses(
    cpu: &unicorn_engine::Unicorn<'_, crate::CpuData>,
    arch: Architecture,
    allocator: bool,
) -> Result<Vec<u64>> {
    let pc = cpu.reg_read(arch.pc())?;
    let lr = cpu.reg_read(arch.lr())?;
    let host = (TRAMPOLINE..RETURN).contains(&pc);
    let mut addresses = Vec::new();
    let mut push = |address: u64, is_return: bool| {
        let address = address & !1;
        if (address == RETURN && !(allocator && addresses.is_empty()))
            || (is_return && address == 0)
            || addresses.len() == 32
        {
            return false;
        }
        if addresses.last() != Some(&address) {
            addresses.push(address);
        }
        true
    };
    let caller = allocator || host;
    if push(if caller { lr } else { pc }, caller) && push(lr, true) {
        let register = match arch {
            Architecture::Arm64 => RegisterARM64::FP as i32,
            Architecture::Arm32 => {
                let thumb = if caller {
                    lr & 1 != 0
                } else {
                    cpu.reg_read(RegisterARM::CPSR)? & 32 != 0
                };
                if thumb {
                    RegisterARM::R7 as i32
                } else {
                    RegisterARM::R11 as i32
                }
            }
        };
        let read = |address| -> Result<u64> {
            let mut bytes = [0; 8];
            cpu.mem_read(address, &mut bytes[..arch.pointer_size() as usize])?;
            Ok(u64::from_le_bytes(bytes))
        };
        let mut fp = cpu.reg_read(register)?;
        while fp != 0 {
            let (Ok(previous), Ok(address)) =
                (read(fp), read(fp.saturating_add(arch.pointer_size())))
            else {
                break;
            };
            if !push(address, true) || previous <= fp {
                break;
            }
            fp = previous;
        }
    }
    Ok(addresses)
}
