use crate::{Emulator, Result, arch::Architecture};
use unicorn_engine::{RegisterARM as A, RegisterARM64 as X};

impl Emulator {
    pub fn registers(&self) -> Result<Vec<(&'static str, u64)>> {
        let cpu = self.cpu()?;
        let table: &[(&str, i32)] = match self.architecture() {
            Architecture::Arm64 => &[
                ("x0", X::X0 as i32),
                ("x1", X::X1 as i32),
                ("x2", X::X2 as i32),
                ("x3", X::X3 as i32),
                ("x4", X::X4 as i32),
                ("x5", X::X5 as i32),
                ("x6", X::X6 as i32),
                ("x7", X::X7 as i32),
                ("x8", X::X8 as i32),
                ("x9", X::X9 as i32),
                ("x10", X::X10 as i32),
                ("x11", X::X11 as i32),
                ("x12", X::X12 as i32),
                ("x13", X::X13 as i32),
                ("x14", X::X14 as i32),
                ("x15", X::X15 as i32),
                ("x16", X::X16 as i32),
                ("x17", X::X17 as i32),
                ("x18", X::X18 as i32),
                ("x19", X::X19 as i32),
                ("x20", X::X20 as i32),
                ("x21", X::X21 as i32),
                ("x22", X::X22 as i32),
                ("x23", X::X23 as i32),
                ("x24", X::X24 as i32),
                ("x25", X::X25 as i32),
                ("x26", X::X26 as i32),
                ("x27", X::X27 as i32),
                ("x28", X::X28 as i32),
                ("fp", X::FP as i32),
                ("lr", X::LR as i32),
                ("sp", X::SP as i32),
                ("pc", X::PC as i32),
                ("nzcv", X::NZCV as i32),
            ],
            Architecture::Arm32 => &[
                ("r0", A::R0 as i32),
                ("r1", A::R1 as i32),
                ("r2", A::R2 as i32),
                ("r3", A::R3 as i32),
                ("r4", A::R4 as i32),
                ("r5", A::R5 as i32),
                ("r6", A::R6 as i32),
                ("r7", A::R7 as i32),
                ("r8", A::R8 as i32),
                ("r9", A::R9 as i32),
                ("r10", A::R10 as i32),
                ("r11", A::R11 as i32),
                ("r12", A::R12 as i32),
                ("sp", A::SP as i32),
                ("lr", A::LR as i32),
                ("pc", A::PC as i32),
                ("cpsr", A::CPSR as i32),
            ],
        };
        table
            .iter()
            .map(|&(name, reg)| Ok((name, cpu.reg_read(reg)?)))
            .collect()
    }
}
