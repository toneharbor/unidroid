use crate::{CpuData, Emulator, Result};
use unicorn_engine::Unicorn;

pub(super) enum Expr {
    Register(i32),
    Constant(u64),
    Memory {
        base: Box<Expr>,
        offset: u64,
        width: usize,
    },
}

impl Expr {
    pub fn parse(emu: &Emulator, text: &str) -> Result<Self> {
        let mut parser = Parser { emu, rest: text };
        (|| {
            let expr = parser.expr()?;
            parser.end()?;
            Ok(expr)
        })()
        .map_err(|error: Box<dyn std::error::Error>| {
            format!("bad expression {text:?}: {error}").into()
        })
    }

    pub fn value(&self, cpu: &Unicorn<'_, CpuData>) -> Option<u64> {
        match self {
            Self::Register(reg) => cpu.reg_read(*reg).ok(),
            Self::Constant(value) => Some(*value),
            Self::Memory {
                base,
                offset,
                width,
            } => {
                let address = base.value(cpu)?.wrapping_add(*offset);
                let mut bytes = [0; 8];
                cpu.mem_read(address, &mut bytes[..*width]).ok()?;
                Some(u64::from_le_bytes(bytes))
            }
        }
    }
}

pub(super) struct Condition {
    left: Expr,
    right: Expr,
    equal: bool,
}
impl Condition {
    pub fn parse(emu: &Emulator, text: &str) -> Result<Self> {
        let mut parser = Parser { emu, rest: text };
        (|| {
            let left = parser.expr()?;
            let equal = if parser.take("==") {
                true
            } else if parser.take("!=") {
                false
            } else {
                return Err("expected == or !=".into());
            };
            let right = parser.expr()?;
            parser.end()?;
            Ok(Self { left, right, equal })
        })()
        .map_err(|error: Box<dyn std::error::Error>| {
            format!("bad condition {text:?}: {error}").into()
        })
    }

    pub fn holds(&self, cpu: &Unicorn<'_, CpuData>) -> bool {
        match (self.left.value(cpu), self.right.value(cpu)) {
            (Some(left), Some(right)) => (left == right) == self.equal,
            _ => false,
        }
    }
}

struct Parser<'a> {
    emu: &'a Emulator,
    rest: &'a str,
}
impl Parser<'_> {
    fn take(&mut self, token: &str) -> bool {
        self.rest = self.rest.trim_start();
        if let Some(rest) = self.rest.strip_prefix(token) {
            self.rest = rest;
            true
        } else {
            false
        }
    }

    fn hex(&mut self) -> Result<u64> {
        if !self.take("0x") {
            return Err("expected a hex constant".into());
        }
        let n = self.rest.bytes().take_while(u8::is_ascii_hexdigit).count();
        let value = u64::from_str_radix(&self.rest[..n], 16)?;
        self.rest = &self.rest[n..];
        Ok(value)
    }

    fn expr(&mut self) -> Result<Expr> {
        self.rest = self.rest.trim_start();
        if self.take("[") {
            let base = Box::new(self.expr()?);
            let offset = if self.take("+") {
                self.hex()?
            } else if self.take("-") {
                self.hex()?.wrapping_neg()
            } else {
                0
            };
            if !self.take("]") {
                return Err("expected ]".into());
            }
            let width = if self.take(":") {
                let width = match self.rest.as_bytes().first() {
                    Some(b'1') => 1,
                    Some(b'2') => 2,
                    Some(b'4') => 4,
                    Some(b'8') => 8,
                    _ => return Err("expected width 1, 2, 4 or 8".into()),
                };
                self.rest = &self.rest[1..];
                width
            } else {
                self.emu.pointer_size() as usize
            };
            Ok(Expr::Memory {
                base,
                offset,
                width,
            })
        } else if self.rest.starts_with("0x") {
            Ok(Expr::Constant(self.hex()?))
        } else {
            let n = self
                .rest
                .bytes()
                .take_while(u8::is_ascii_alphanumeric)
                .count();
            if n == 0 {
                return Err("expected a register, constant or memory expression".into());
            }
            let register = self.emu.named_register(&self.rest[..n])?;
            self.rest = &self.rest[n..];
            Ok(Expr::Register(register))
        }
    }

    fn end(&self) -> Result<()> {
        if self.rest.trim().is_empty() {
            Ok(())
        } else {
            Err("unexpected trailing text".into())
        }
    }
}
