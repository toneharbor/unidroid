use crate::{Emulator, Result, arch::Architecture};
use unicorn_engine::{RegisterARM as A, RegisterARM64 as X};

impl Emulator {
    pub fn registers(&self) -> Result<Vec<(&'static str, u64)>> {
        let cpu = self.cpu()?;
        self.register_table()
            .iter()
            .filter(|r| r.2)
            .map(|&(name, reg, _)| Ok((name, cpu.reg_read(reg)?)))
            .collect()
    }

    pub fn register(&self, name: &str) -> Result<u64> {
        let reg = self.named_register(name)?;
        Ok(self.cpu()?.reg_read(reg)?)
    }

    pub fn set_register(&mut self, name: &str, value: u64) -> Result<()> {
        let reg = self.named_register(name)?;
        Ok(self.cpu_mut()?.reg_write(reg, value)?)
    }

    fn named_register(&self, name: &str) -> Result<i32> {
        self.cpu()?;
        let table = self.register_table();
        table
            .iter()
            .find(|r| r.0 == name)
            .map(|r| r.1)
            .ok_or_else(|| {
                format!(
                    "unknown register {name}; valid names: {}",
                    table.iter().map(|r| r.0).collect::<Vec<_>>().join(", ")
                )
                .into()
            })
    }

    fn register_table(&self) -> &'static [(&'static str, i32, bool)] {
        match self.architecture() {
            Architecture::Arm64 => &[
                ("x0", X::X0 as i32, true),
                ("x1", X::X1 as i32, true),
                ("x2", X::X2 as i32, true),
                ("x3", X::X3 as i32, true),
                ("x4", X::X4 as i32, true),
                ("x5", X::X5 as i32, true),
                ("x6", X::X6 as i32, true),
                ("x7", X::X7 as i32, true),
                ("x8", X::X8 as i32, true),
                ("x9", X::X9 as i32, true),
                ("x10", X::X10 as i32, true),
                ("x11", X::X11 as i32, true),
                ("x12", X::X12 as i32, true),
                ("x13", X::X13 as i32, true),
                ("x14", X::X14 as i32, true),
                ("x15", X::X15 as i32, true),
                ("x16", X::X16 as i32, true),
                ("x17", X::X17 as i32, true),
                ("x18", X::X18 as i32, true),
                ("x19", X::X19 as i32, true),
                ("x20", X::X20 as i32, true),
                ("x21", X::X21 as i32, true),
                ("x22", X::X22 as i32, true),
                ("x23", X::X23 as i32, true),
                ("x24", X::X24 as i32, true),
                ("x25", X::X25 as i32, true),
                ("x26", X::X26 as i32, true),
                ("x27", X::X27 as i32, true),
                ("x28", X::X28 as i32, true),
                ("fp", X::FP as i32, true),
                ("lr", X::LR as i32, true),
                ("sp", X::SP as i32, true),
                ("pc", X::PC as i32, true),
                ("nzcv", X::NZCV as i32, true),
                ("d0", X::D0 as i32, false),
                ("d1", X::D1 as i32, false),
                ("d2", X::D2 as i32, false),
                ("d3", X::D3 as i32, false),
                ("d4", X::D4 as i32, false),
                ("d5", X::D5 as i32, false),
                ("d6", X::D6 as i32, false),
                ("d7", X::D7 as i32, false),
                ("d8", X::D8 as i32, false),
                ("d9", X::D9 as i32, false),
                ("d10", X::D10 as i32, false),
                ("d11", X::D11 as i32, false),
                ("d12", X::D12 as i32, false),
                ("d13", X::D13 as i32, false),
                ("d14", X::D14 as i32, false),
                ("d15", X::D15 as i32, false),
                ("d16", X::D16 as i32, false),
                ("d17", X::D17 as i32, false),
                ("d18", X::D18 as i32, false),
                ("d19", X::D19 as i32, false),
                ("d20", X::D20 as i32, false),
                ("d21", X::D21 as i32, false),
                ("d22", X::D22 as i32, false),
                ("d23", X::D23 as i32, false),
                ("d24", X::D24 as i32, false),
                ("d25", X::D25 as i32, false),
                ("d26", X::D26 as i32, false),
                ("d27", X::D27 as i32, false),
                ("d28", X::D28 as i32, false),
                ("d29", X::D29 as i32, false),
                ("d30", X::D30 as i32, false),
                ("d31", X::D31 as i32, false),
            ],
            Architecture::Arm32 => &[
                ("r0", A::R0 as i32, true),
                ("r1", A::R1 as i32, true),
                ("r2", A::R2 as i32, true),
                ("r3", A::R3 as i32, true),
                ("r4", A::R4 as i32, true),
                ("r5", A::R5 as i32, true),
                ("r6", A::R6 as i32, true),
                ("r7", A::R7 as i32, true),
                ("r8", A::R8 as i32, true),
                ("r9", A::R9 as i32, true),
                ("r10", A::R10 as i32, true),
                ("r11", A::R11 as i32, true),
                ("r12", A::R12 as i32, true),
                ("sp", A::SP as i32, true),
                ("lr", A::LR as i32, true),
                ("pc", A::PC as i32, true),
                ("cpsr", A::CPSR as i32, true),
                ("d0", A::D0 as i32, false),
                ("d1", A::D1 as i32, false),
                ("d2", A::D2 as i32, false),
                ("d3", A::D3 as i32, false),
                ("d4", A::D4 as i32, false),
                ("d5", A::D5 as i32, false),
                ("d6", A::D6 as i32, false),
                ("d7", A::D7 as i32, false),
                ("d8", A::D8 as i32, false),
                ("d9", A::D9 as i32, false),
                ("d10", A::D10 as i32, false),
                ("d11", A::D11 as i32, false),
                ("d12", A::D12 as i32, false),
                ("d13", A::D13 as i32, false),
                ("d14", A::D14 as i32, false),
                ("d15", A::D15 as i32, false),
                ("d16", A::D16 as i32, false),
                ("d17", A::D17 as i32, false),
                ("d18", A::D18 as i32, false),
                ("d19", A::D19 as i32, false),
                ("d20", A::D20 as i32, false),
                ("d21", A::D21 as i32, false),
                ("d22", A::D22 as i32, false),
                ("d23", A::D23 as i32, false),
                ("d24", A::D24 as i32, false),
                ("d25", A::D25 as i32, false),
                ("d26", A::D26 as i32, false),
                ("d27", A::D27 as i32, false),
                ("d28", A::D28 as i32, false),
                ("d29", A::D29 as i32, false),
                ("d30", A::D30 as i32, false),
                ("d31", A::D31 as i32, false),
            ],
        }
    }
}
