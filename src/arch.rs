use crate::{Emulator, JniValue, Result};
use unicorn_engine::{
    RegisterARM as Arm, RegisterARM64 as Arm64, Unicorn,
    unicorn_const::{Arch, Mode},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Architecture {
    Arm32,
    Arm64,
}
impl Architecture {
    pub fn name(self) -> &'static str {
        match self {
            Self::Arm32 => "arm32",
            Self::Arm64 => "arm64",
        }
    }
    pub fn pointer_size(self) -> u64 {
        match self {
            Self::Arm32 => 4,
            Self::Arm64 => 8,
        }
    }
    pub fn library_dir(self) -> &'static str {
        match self {
            Self::Arm32 => "lib",
            Self::Arm64 => "lib64",
        }
    }
    pub fn cpu(self) -> (Arch, Mode) {
        match self {
            Self::Arm32 => (Arch::ARM, Mode::ARM | Mode::LITTLE_ENDIAN),
            Self::Arm64 => (Arch::ARM64, Mode::LITTLE_ENDIAN),
        }
    }
    pub fn enable_fp(self, uc: &mut Unicorn<'_, crate::CpuData>) -> Result<()> {
        match self {
            Self::Arm32 => {
                uc.reg_write(Arm::C1_C0_2, 0xf << 20)?;
                uc.reg_write(Arm::FPEXC, 0x40000000)?;
            }
            Self::Arm64 => uc.reg_write(Arm64::CPACR_EL1, 3 << 20)?,
        }
        Ok(())
    }
    pub fn gp(self, index: usize) -> i32 {
        (match self {
            Self::Arm32 => Arm::R0 as i32,
            Self::Arm64 => Arm64::X0 as i32,
        }) + index as i32
    }
    pub fn sp(self) -> i32 {
        match self {
            Self::Arm32 => Arm::SP as i32,
            Self::Arm64 => Arm64::SP as i32,
        }
    }
    pub fn lr(self) -> i32 {
        match self {
            Self::Arm32 => Arm::LR as i32,
            Self::Arm64 => Arm64::LR as i32,
        }
    }
    pub fn pc(self) -> i32 {
        match self {
            Self::Arm32 => Arm::PC as i32,
            Self::Arm64 => Arm64::PC as i32,
        }
    }
    pub fn tls(self) -> i32 {
        match self {
            Self::Arm32 => Arm::C13_C0_3 as i32,
            Self::Arm64 => Arm64::TPIDR_EL0 as i32,
        }
    }
    pub fn syscall_register(self) -> usize {
        match self {
            Self::Arm32 => 7,
            Self::Arm64 => 8,
        }
    }
    pub fn stub(self) -> [u8; 8] {
        match self {
            Self::Arm32 => [0, 0, 0, 0xef, 0x1e, 0xff, 0x2f, 0xe1],
            Self::Arm64 => [1, 0, 0, 0xd4, 0xc0, 3, 0x5f, 0xd6],
        }
    }
    pub fn rtld_default(self) -> u64 {
        match self {
            Self::Arm32 => 0xffffffff,
            Self::Arm64 => 0,
        }
    }
    pub fn rtld_next(self) -> u64 {
        match self {
            Self::Arm32 => 0xfffffffe,
            Self::Arm64 => u64::MAX,
        }
    }
    pub fn pointer_mask(self) -> u64 {
        match self {
            Self::Arm32 => u32::MAX as u64,
            Self::Arm64 => u64::MAX,
        }
    }
    pub fn packed_rel_tag(self) -> u64 {
        match self {
            Self::Arm32 => 0x6000000f,
            Self::Arm64 => 0x60000011,
        }
    }
    pub fn relocation_info(self, info: u64) -> u64 {
        match self {
            Self::Arm32 => (info >> 8) << 32 | (info & 0xff),
            Self::Arm64 => info,
        }
    }
    // (relative, implicit addend). None is an unsupported relocation.
    pub fn relocation(self, kind: u32) -> Option<(bool, bool)> {
        match (self, kind) {
            (Self::Arm32, 23) => Some((true, true)),
            (Self::Arm32, 2) => Some((false, true)),
            (Self::Arm32, 21 | 22) => Some((false, false)),
            (Self::Arm64, 1027) => Some((true, false)),
            (Self::Arm64, 257 | 1025 | 1026) => Some((false, false)),
            _ => None,
        }
    }
    pub fn stat(self, m: &std::fs::Metadata) -> ([u8; 128], usize) {
        use std::os::unix::fs::MetadataExt;
        let mut bytes = [0; 128];
        let (size, inode, time_width) = match self {
            Self::Arm32 => (104, 96, 4),
            Self::Arm64 => (128, 8, 8),
        };
        for (offset, width, value) in [
            (0, 8, m.dev()),
            (inode, 8, m.ino()),
            (32, 8, m.rdev()),
            (48, 8, m.size()),
            (64, 8, m.blocks()),
            (16, 4, m.mode() as u64),
            (20, 4, m.nlink()),
            (24, 4, m.uid() as u64),
            (28, 4, m.gid() as u64),
            (56, 4, 4096),
        ] {
            bytes[offset..offset + width].copy_from_slice(&value.to_le_bytes()[..width]);
        }
        if self == Self::Arm32 {
            bytes[12..16].copy_from_slice(&(m.ino() as u32).to_le_bytes());
        }
        for (i, value) in [
            m.atime(),
            m.atime_nsec(),
            m.mtime(),
            m.mtime_nsec(),
            m.ctime(),
            m.ctime_nsec(),
        ]
        .into_iter()
        .enumerate()
        {
            let offset = 72 + i * time_width;
            bytes[offset..offset + time_width].copy_from_slice(&value.to_le_bytes()[..time_width]);
        }
        (bytes, size)
    }
    pub fn syscall(self, nr: u64, a: &mut [u64; 6]) -> Option<u64> {
        if self == Self::Arm64 {
            return Some(nr);
        }
        Some(match nr {
            322 => 56,
            6 => 57,
            19 => {
                a[1] = a[1] as i32 as i64 as u64;
                62
            }
            3 => 63,
            180 => {
                a[3] = a[4] as u32 as u64 | (a[5] << 32);
                67
            }
            327 => 79,
            197 => 80,
            1 => 93,
            248 => 94,
            256 => 96,
            240 => 98,
            338 => 99,
            162 => 101,
            265 => 115,
            158 => 124,
            186 => 132,
            175 => 135,
            20 => 172,
            224 => 178,
            120 => 220,
            263 => 113,
            78 => 169,
            172 => 167,
            199 => 174,
            201 => 175,
            200 => 176,
            202 => 177,
            45 => 214,
            91 => 215,
            192 => {
                a[5] *= 4096;
                222
            }
            125 => 226,
            220 => 233,
            _ => return None,
        })
    }
}

// One cursor owns register allocation and stack alignment for both directions.
struct Arguments {
    arch: Architecture,
    gp: usize,
    fp: usize,
    stack: u64,
}
enum Argument {
    General(usize, usize),
    Float(usize),
    Stack(u64, usize),
}
impl Arguments {
    fn new(arch: Architecture, start: usize, stack: u64) -> Self {
        Self {
            arch,
            gp: start,
            fp: 0,
            stack,
        }
    }
    fn next(&mut self, kind: u8) -> Argument {
        let floating = matches!(kind, b'F' | b'D');
        if self.arch == Architecture::Arm64 {
            if floating && self.fp < 8 {
                let index = self.fp;
                self.fp += 1;
                return Argument::Float(index);
            }
            if !floating && self.gp < 8 {
                let index = self.gp;
                self.gp += 1;
                return Argument::General(index, 1);
            }
            let address = self.stack;
            self.stack += 8;
            return Argument::Stack(address, 8);
        }
        let words = if matches!(kind, b'J' | b'D') { 2 } else { 1 };
        self.gp = (self.gp + words - 1) & !(words - 1);
        if self.gp + words <= 4 {
            let index = self.gp;
            self.gp += words;
            Argument::General(index, words)
        } else {
            self.gp = 4;
            let size = words * 4;
            self.stack = (self.stack + size as u64 - 1) & !(size as u64 - 1);
            let address = self.stack;
            self.stack += size as u64;
            Argument::Stack(address, size)
        }
    }
}
impl Emulator {
    pub(crate) fn architecture(&self) -> Architecture {
        self.arch.unwrap()
    }
    pub(crate) fn pointer_size(&self) -> u64 {
        self.architecture().pointer_size()
    }
    pub(crate) fn read_pointer(&self, address: u64) -> Result<u64> {
        let mut bytes = [0; 8];
        let size = self.pointer_size() as usize;
        self.cpu()?.mem_read(address, &mut bytes[..size])?;
        Ok(u64::from_le_bytes(bytes))
    }
    pub(crate) fn write_pointer(&mut self, address: u64, value: u64) -> Result<()> {
        self.write_bytes(
            address,
            &value.to_le_bytes()[..self.pointer_size() as usize],
        )
    }
    pub(crate) fn sp(&self) -> Result<u64> {
        Ok(self.cpu()?.reg_read(self.architecture().sp())?)
    }
    pub(crate) fn set_sp(&mut self, value: u64) -> Result<()> {
        let reg = self.architecture().sp();
        Ok(self.cpu_mut()?.reg_write(reg, value)?)
    }
    pub(crate) fn lr(&self) -> Result<u64> {
        Ok(self.cpu()?.reg_read(self.architecture().lr())?)
    }
    pub(crate) fn set_lr(&mut self, value: u64) -> Result<()> {
        let reg = self.architecture().lr();
        Ok(self.cpu_mut()?.reg_write(reg, value)?)
    }
    pub(crate) fn pc(&self) -> Result<u64> {
        Ok(self.cpu()?.reg_read(self.architecture().pc())?)
    }
    pub(crate) fn restart_pc(&self) -> Result<(u64, u64)> {
        let pc = self.pc()?;
        let thumb = self.architecture() == Architecture::Arm32
            && self.cpu()?.reg_read(Arm::CPSR)? & 32 != 0;
        Ok((
            pc | u64::from(thumb),
            pc.wrapping_sub(if thumb { 2 } else { 4 }),
        ))
    }
    pub(crate) fn reg(&self, index: usize) -> Result<u64> {
        Ok(self.cpu()?.reg_read(self.architecture().gp(index))?)
    }
    pub(crate) fn write_reg(&mut self, index: usize, bits: u64) -> Result<()> {
        let arch = self.architecture();
        Ok(self
            .cpu_mut()?
            .reg_write(arch.gp(index), bits & arch.pointer_mask())?)
    }
    fn read_argument(&self, argument: Argument) -> Result<u64> {
        match argument {
            Argument::General(index, 1) => self.reg(index),
            Argument::General(index, _) => Ok(self.reg(index)? | (self.reg(index + 1)? << 32)),
            Argument::Float(index) => Ok(self.cpu()?.reg_read(Arm64::D0 as i32 + index as i32)?),
            Argument::Stack(address, size) => {
                let mut bytes = [0; 8];
                self.cpu()?.mem_read(address, &mut bytes[..size])?;
                Ok(u64::from_le_bytes(bytes))
            }
        }
    }
    pub(crate) fn argument(&self, start: usize, kind: u8) -> Result<u64> {
        let mut cursor = Arguments::new(self.architecture(), start, self.sp()?);
        self.read_argument(cursor.next(kind))
    }
    pub(crate) fn place_arguments(
        &mut self,
        args: impl IntoIterator<Item = (u8, u64)>,
    ) -> Result<u64> {
        let sp = self.sp()?;
        let mut cursor = Arguments::new(self.architecture(), 0, 0);
        let mut stack = Vec::new();
        for (kind, bits) in args {
            match cursor.next(kind) {
                Argument::General(index, words) => {
                    self.write_reg(index, bits)?;
                    if words == 2 {
                        self.write_reg(index + 1, bits >> 32)?;
                    }
                }
                Argument::Float(index) => self
                    .cpu_mut()?
                    .reg_write(Arm64::D0 as i32 + index as i32, bits)?,
                Argument::Stack(offset, size) => stack.push((offset, size, bits)),
            }
        }
        let call_sp = (sp - cursor.stack) & !15;
        for (offset, size, bits) in stack {
            self.write_bytes(call_sp + offset, &bits.to_le_bytes()[..size])?;
        }
        self.set_sp(call_sp)?;
        self.set_lr(crate::RETURN)?;
        Ok(sp)
    }
    pub(crate) fn result_bits(&self, kind: u8) -> Result<u64> {
        match (self.architecture(), kind) {
            (Architecture::Arm64, b'F' | b'D') => Ok(self.cpu()?.reg_read(Arm64::D0)?),
            (Architecture::Arm32, b'J' | b'D') => Ok(self.reg(0)? | (self.reg(1)? << 32)),
            _ => self.reg(0),
        }
    }
    pub(crate) fn write_host_result(&mut self, bits: u64) -> Result<()> {
        self.write_reg(0, bits)?;
        if self.architecture() == Architecture::Arm32 {
            self.write_reg(1, bits >> 32)?;
        }
        Ok(())
    }
    pub(crate) fn write_jni_result(&mut self, value: JniValue) -> Result<()> {
        if self.architecture() == Architecture::Arm64
            && matches!(value, JniValue::Float(_) | JniValue::Double(_))
        {
            self.cpu_mut()?.reg_write(Arm64::D0, value.bits())?;
        }
        Ok(())
    }
    pub(crate) fn argument_bits(
        &self,
        kinds: &[u8],
        variant: usize,
        start: usize,
    ) -> Result<Vec<u64>> {
        let arch = self.architecture();
        let ptr = if variant == 0 {
            0
        } else {
            self.argument(start, b'L')?
        };
        let mut cursor = Arguments::new(arch, start, self.sp()?);
        let (mut gr_top, mut vr_top, mut gr_off, mut vr_off) = (0, 0, 0i32, 0i32);
        if variant == 1 {
            if arch == Architecture::Arm32 {
                cursor.gp = 4;
                cursor.stack = ptr;
            } else {
                cursor.stack = self.read_u64(ptr)?;
                gr_top = self.read_u64(ptr + 8)?;
                vr_top = self.read_u64(ptr + 16)?;
                gr_off = i32::from_le_bytes(self.read_bytes(ptr + 24, 4)?.try_into().unwrap());
                vr_off = i32::from_le_bytes(self.read_bytes(ptr + 28, 4)?.try_into().unwrap());
            }
        }
        kinds
            .iter()
            .enumerate()
            .map(|(i, &kind)| {
                if variant == 2 {
                    return self.read_u64(ptr + i as u64 * 8);
                }
                let promoted = if kind == b'F' { b'D' } else { kind };
                if variant == 1 && arch == Architecture::Arm64 {
                    let floating = matches!(kind, b'F' | b'D');
                    let address = if floating && vr_off < 0 {
                        let address = vr_top.wrapping_add_signed(vr_off as i64);
                        vr_off += 16;
                        address
                    } else if !floating && gr_off < 0 {
                        let address = gr_top.wrapping_add_signed(gr_off as i64);
                        gr_off += 8;
                        address
                    } else {
                        let address = cursor.stack;
                        cursor.stack += 8;
                        address
                    };
                    self.read_u64(address)
                } else {
                    self.read_argument(cursor.next(promoted))
                }
            })
            .collect()
    }
}
