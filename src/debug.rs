use crate::{Emulator, RETURN, Result, TRAMPOLINE, arch::Architecture};
use serde::Serialize;
use std::fmt::{self, Write};
use unicorn_engine::{RegisterARM, RegisterARM64};

pub(crate) mod code;
pub(crate) mod control;
pub(crate) mod events;
pub(crate) mod imports;
pub(crate) mod maps;
pub(crate) mod modules;
mod registers;
pub(crate) mod snapshot;
mod syscalls;

const STACK_WORDS: usize = 32;

#[derive(Debug, Serialize)]
pub struct Frame {
    pub address: u64,
    pub location: String,
    pub symbol: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Crash {
    pub message: String,
    pub thread: u32,
    pub registers: Vec<(&'static str, u64)>,
    pub backtrace: Vec<Frame>,
    pub stack: Vec<(u64, u64)>,
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
            let _ = write!(text, "#{i} {}", frame.location);
            if let Some(symbol) = &frame.symbol {
                let _ = write!(text, " ({symbol})");
            }
            text.push('\n');
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
        let pc = self.pc()?;
        let lr = self.lr()?;
        let host = (TRAMPOLINE..RETURN).contains(&pc);
        let mut addresses = Vec::new();
        let mut push = |address: u64, is_return: bool| {
            let address = address & !1;
            if address == RETURN || (is_return && address == 0) || addresses.len() == 32 {
                return false;
            }
            if addresses.last() != Some(&address) {
                addresses.push(address);
            }
            true
        };
        if push(if host { lr } else { pc }, host) && push(lr, true) {
            let reg = match self.architecture() {
                Architecture::Arm64 => RegisterARM64::FP as i32,
                Architecture::Arm32 => {
                    let thumb = if host {
                        lr & 1 != 0
                    } else {
                        self.cpu()?.reg_read(RegisterARM::CPSR)? & 32 != 0
                    };
                    if thumb {
                        RegisterARM::R7 as i32
                    } else {
                        RegisterARM::R11 as i32
                    }
                }
            };
            let mut fp = self.cpu()?.reg_read(reg)?;
            while fp != 0 {
                let (Ok(previous), Ok(address)) = (
                    self.read_pointer(fp),
                    self.read_pointer(fp.saturating_add(self.pointer_size())),
                ) else {
                    break;
                };
                if !push(address, true) || previous <= fp {
                    break;
                }
                fp = previous;
            }
        }
        Ok(addresses
            .into_iter()
            .map(|address| Frame {
                address,
                location: self.location(address),
                symbol: self.symbol(address),
            })
            .collect())
    }

    fn symbol(&self, address: u64) -> Option<String> {
        self.modules
            .iter()
            .find(|m| address >= m.base && address < m.base + m.image.size)
            .and_then(|m| m.image.function(address - m.base))
    }

    pub fn symbolize(&self, address: u64) -> String {
        let location = self.location(address);
        match self.symbol(address) {
            Some(symbol) => format!("{location} ({symbol})"),
            None => location,
        }
    }
}
