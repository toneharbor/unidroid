use crate::{Architecture, Emulator};

const IOC_DIRSHIFT: u32 = 30;
const IOC_TYPESHIFT: u32 = 8;
const IOC_NRSHIFT: u32 = 0;
const IOC_SIZESHIFT: u32 = 16;

const IOC_T: u32 = b'T' as u32;
const IOC_F: u32 = b'f' as u32;

fn ioc_dir(cmd: u32) -> u32 {
    (cmd >> IOC_DIRSHIFT) & 0x3
}
fn ioc_type(cmd: u32) -> u32 {
    (cmd >> IOC_TYPESHIFT) & 0xFF
}
fn ioc_nr(cmd: u32) -> u32 {
    (cmd >> IOC_NRSHIFT) & 0xFF
}
fn ioc_size(cmd: u32) -> u32 {
    (cmd >> IOC_SIZESHIFT) & 0x3FFF
}

impl Emulator {
    pub(super) fn ioctl(&mut self, _fd: u64, request: u64, arg: u64) -> u64 {
        let req = request as u32;
        let typ = ioc_type(req);
        let nr = ioc_nr(req);
        let size = ioc_size(req) as usize;
        let errno = |n: i64| (-n) as u64;
        match (typ, nr) {
            (IOC_T, 1) => {
                let termios_size = match self.architecture() {
                    Architecture::Arm32 => 36,
                    Architecture::Arm64 => 60,
                };
                let n = if size > 0 { size } else { termios_size };
                let mut buf = vec![0u8; n];
                if n >= 16 {
                    buf[8..12].copy_from_slice(&0x00B0u32.to_le_bytes());
                    buf[12..16].copy_from_slice(&0x000Bu32.to_le_bytes());
                }
                self.write_bytes(arg, &buf).map(|_| 0).unwrap_or(errno(14))
            }
            (IOC_T, 15) => {
                let pgid = (crate::threads::MAIN_TID as i32).to_le_bytes();
                self.write_bytes(arg, &pgid).map(|_| 0).unwrap_or(errno(14))
            }
            (IOC_T, 16) => 0,
            (IOC_T, 19) => {
                let ws_size = if size > 0 { size } else { 8 };
                let mut ws = vec![0u8; ws_size];
                ws[0..2].copy_from_slice(&24u16.to_le_bytes());
                ws[2..4].copy_from_slice(&80u16.to_le_bytes());
                self.write_bytes(arg, &ws).map(|_| 0).unwrap_or(errno(14))
            }
            (IOC_T, 27) => {
                let zero = 0i32.to_le_bytes();
                self.write_bytes(arg, &zero).map(|_| 0).unwrap_or(errno(14))
            }
            (IOC_T, 33) => 0,
            (IOC_F, 126) => 0,
            (IOC_F, 127) => {
                let zero = 0i32.to_le_bytes();
                self.write_bytes(arg, &zero).map(|_| 0).unwrap_or(errno(14))
            }
            _ => {
                if ioc_dir(req) & 2 != 0 && size > 0 && arg != 0 {
                    let zeros = vec![0u8; size.min(4096)];
                    let _ = self.write_bytes(arg, &zeros);
                }
                0
            }
        }
    }

    pub(super) fn sysinfo(&mut self, address: u64) -> u64 {
        let ptr_size = self.architecture().pointer_size() as usize;
        let struct_size = if ptr_size == 8 { 104 } else { 64 };
        let mem_unit: u64 = if ptr_size == 8 { 1 } else { 4096 };
        let mut buf = vec![0u8; struct_size];
        let mut offset = 0;
        let write_long = |buf: &mut [u8], off: &mut usize, val: u64, ps: usize| {
            debug_assert!(ps == 8 || val <= u32::MAX as u64);
            if ps == 8 {
                buf[*off..*off + 8].copy_from_slice(&val.to_le_bytes());
            } else {
                buf[*off..*off + 4].copy_from_slice(&(val as u32).to_le_bytes());
            }
            *off += ps;
        };
        // Deterministic stand-ins for emulation; not scraped from the host.
        // They only need to be self-consistent so that guest code branching
        // on sysinfo sees a plausible environment.
        write_long(&mut buf, &mut offset, 3600, ptr_size);
        for &load in &[128u64, 64, 32] {
            write_long(&mut buf, &mut offset, load << 16, ptr_size);
        }
        let totalram = 2u64 * 1024 * 1024 * 1024;
        write_long(&mut buf, &mut offset, totalram / mem_unit, ptr_size);
        write_long(&mut buf, &mut offset, totalram / 4 * 3 / mem_unit, ptr_size);
        write_long(&mut buf, &mut offset, 0, ptr_size);
        write_long(&mut buf, &mut offset, totalram / 10 / mem_unit, ptr_size);
        write_long(&mut buf, &mut offset, 0, ptr_size);
        write_long(&mut buf, &mut offset, 0, ptr_size);
        buf[offset..offset + 2].copy_from_slice(&(120u16).to_le_bytes());
        offset += 2;
        buf[offset..offset + 2].copy_from_slice(&0u16.to_le_bytes());
        offset += 2;
        write_long(&mut buf, &mut offset, 0, ptr_size);
        write_long(&mut buf, &mut offset, 0, ptr_size);
        debug_assert!(mem_unit <= u32::MAX as u64);
        buf[offset..offset + 4].copy_from_slice(&(mem_unit as u32).to_le_bytes());
        offset += 4;
        if ptr_size == 4 {
            buf[offset..offset + 8].copy_from_slice(&[0u8; 8]);
            offset += 8;
        }
        debug_assert_eq!(offset, struct_size);
        if self.write_bytes(address, &buf).is_err() {
            (-14i64) as u64
        } else {
            0
        }
    }

    pub(super) fn uname(&mut self, address: u64) -> u64 {
        let machine = match self.architecture() {
            Architecture::Arm32 => "armv7l",
            Architecture::Arm64 => "aarch64",
        };
        let mut bytes = [0; 390];
        for (field, value) in bytes.as_chunks_mut::<65>().0.iter_mut().zip([
            "Linux",
            "localhost",
            "4.14.190-g5f07fba0b5da",
            "#1 SMP PREEMPT Thu Apr 19 14:36:58 CST 2018",
            machine,
            "",
        ]) {
            field[..value.len()].copy_from_slice(value.as_bytes());
        }
        if self.write_bytes(address, &bytes).is_err() {
            (-14i64) as u64
        } else {
            0
        }
    }
}
