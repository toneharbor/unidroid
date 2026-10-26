use crate::{Emulator, Result, arch::Architecture};
use unicorn_engine::{RegisterARM as A, RegisterARM64 as X};

#[cfg(test)]
mod tests;

struct Description {
    named: &'static [(&'static str, i32)],
    families: &'static [(&'static str, i32, i32)],
    general: &'static [&'static str],
}

fn description(arch: Architecture) -> &'static Description {
    match arch {
        Architecture::Arm64 => &Description {
            named: &[
                ("fp", X::FP as i32),
                ("lr", X::LR as i32),
                ("sp", X::SP as i32),
                ("pc", X::PC as i32),
                ("nzcv", X::NZCV as i32),
                ("x29", X::FP as i32),
                ("x30", X::LR as i32),
                ("wsp", X::WSP as i32),
                ("fpcr", X::FPCR as i32),
                ("fpsr", X::FPSR as i32),
            ],
            families: &[
                ("x", 29, X::X0 as i32),
                ("w", 31, X::W0 as i32),
                ("b", 32, X::B0 as i32),
                ("h", 32, X::H0 as i32),
                ("s", 32, X::S0 as i32),
                ("d", 32, X::D0 as i32),
            ],
            general: &[
                "x0", "x1", "x2", "x3", "x4", "x5", "x6", "x7", "x8", "x9", "x10", "x11", "x12",
                "x13", "x14", "x15", "x16", "x17", "x18", "x19", "x20", "x21", "x22", "x23", "x24",
                "x25", "x26", "x27", "x28", "fp", "lr", "sp", "pc", "nzcv",
            ],
        },
        Architecture::Arm32 => &Description {
            named: &[
                ("sp", A::SP as i32),
                ("lr", A::LR as i32),
                ("pc", A::PC as i32),
                ("cpsr", A::CPSR as i32),
                ("apsr", A::APSR as i32),
                ("apsr_nzcv", A::APSR_NZCV as i32),
                ("fpscr", A::FPSCR as i32),
                ("fpexc", A::FPEXC as i32),
                ("sb", A::R9 as i32),
                ("sl", A::R10 as i32),
                ("fp", A::R11 as i32),
                ("ip", A::R12 as i32),
                ("fpsid", A::FPSID as i32),
                ("spsr", A::SPSR as i32),
            ],
            families: &[
                ("r", 13, A::R0 as i32),
                ("s", 32, A::S0 as i32),
                ("d", 32, A::D0 as i32),
            ],
            general: &[
                "r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10", "r11", "r12",
                "sp", "lr", "pc", "cpsr",
            ],
        },
    }
}

// W writes zero-extend into X/SP; Unicorn's W writer leaves the upper half intact.
fn write_target(arch: Architecture, reg: i32, value: u64) -> (i32, u64) {
    if arch != Architecture::Arm64 {
        return (reg, value);
    }
    let parent = if (X::W0 as i32..=X::W30 as i32).contains(&reg) {
        match reg - X::W0 as i32 {
            29 => X::FP as i32,
            30 => X::LR as i32,
            n => X::X0 as i32 + n,
        }
    } else if reg == X::WSP as i32 {
        X::SP as i32
    } else {
        return (reg, value);
    };
    (parent, u64::from(value as u32))
}

pub(super) fn indexed(name: &str, prefix: &str, count: i32, first: i32) -> Option<i32> {
    name.strip_prefix(prefix)?
        .parse::<i32>()
        .ok()
        .filter(|&n| (0..count).contains(&n))
        .map(|n| first + n)
}

pub(super) fn resolve(arch: Architecture, name: &str) -> Option<i32> {
    let description = description(arch);
    description
        .named
        .iter()
        .find(|r| r.0 == name)
        .map(|r| r.1)
        .or_else(|| {
            description
                .families
                .iter()
                .find_map(|&(prefix, count, first)| indexed(name, prefix, count, first))
        })
}

impl Emulator {
    pub fn registers(&self) -> Result<Vec<(&'static str, u64)>> {
        let cpu = self.cpu()?;
        let arch = self.architecture();
        description(arch)
            .general
            .iter()
            .map(|&name| Ok((name, cpu.reg_read(resolve(arch, name).unwrap())?)))
            .collect()
    }

    pub fn register(&self, name: &str) -> Result<u64> {
        let reg = self.named_register(name)?;
        Ok(self.cpu()?.reg_read(reg)?)
    }

    pub fn set_register(&mut self, name: &str, value: u64) -> Result<()> {
        let reg = self.named_register(name)?;
        let (reg, value) = write_target(self.architecture(), reg, value);
        Ok(self.cpu_mut()?.reg_write(reg, value)?)
    }

    pub(super) fn named_register(&self, name: &str) -> Result<i32> {
        self.cpu()?;
        let arch = self.architecture();
        resolve(arch, name).ok_or_else(|| {
            let description = description(arch);
            let names = description
                .families
                .iter()
                .map(|(prefix, count, _)| format!("{prefix}0-{prefix}{}", count - 1))
                .chain(description.named.iter().map(|(name, _)| (*name).into()))
                .collect::<Vec<_>>()
                .join(", ");
            format!("unknown register {name}; valid names: {names}").into()
        })
    }
}
