use crate::debug::registers::{indexed, resolve};
use crate::{CpuData, Result, arch::Architecture};
use unicorn_engine::{RegisterARM as A, RegisterARM64 as X, Unicorn};

pub(super) fn value(cpu: &Unicorn<'_, CpuData>, arch: Architecture, name: &str) -> Result<u128> {
    let indexed = |prefix, count, first| indexed(name, prefix, count, first);
    let unavailable = || format!("register {name} is not exposed by Unicorn");
    if arch == Architecture::Arm32 {
        // These IDs exist in Unicorn's enum, but its reader leaves the value untouched.
        if matches!(name, "fpinst" | "fpinst2" | "mvfr0" | "mvfr1" | "mvfr2") {
            return Err(unavailable().into());
        }
        // Unicorn's long register API supports A64 only. ARM Q registers alias two D registers.
        if let Some(n) = indexed("q", 16, 0) {
            let low = cpu.reg_read(A::D0 as i32 + n * 2)? as u128;
            let high = cpu.reg_read(A::D0 as i32 + n * 2 + 1)? as u128;
            return Ok(low | high << 64);
        }
        if name == "itstate" {
            let cpsr = cpu.reg_read(A::CPSR)?;
            return Ok((((cpsr >> 25) & 3) | ((cpsr >> 8) & 0xfc)) as u128);
        }
        if name == "fpscr_nzcv" {
            return Ok((cpu.reg_read(A::FPSCR)? & 0xf0000000) as u128);
        }
    }
    if arch == Architecture::Arm64 {
        // Capstone also names SVE/SME registers. Unicorn exposes no read API for them.
        if matches!(name, "ffr" | "vg" | "za")
            || [
                ("p", 16),
                ("z", 32),
                ("zab", 1),
                ("zad", 8),
                ("zah", 2),
                ("zaq", 16),
                ("zas", 4),
            ]
            .iter()
            .any(|&(prefix, limit)| indexed(prefix, limit, 0).is_some())
        {
            return Err(unavailable().into());
        }
        if matches!(name, "xzr" | "wzr") {
            return Ok(0);
        }
        if let Some(reg) = indexed("v", 32, X::Q0 as i32).or_else(|| indexed("q", 32, X::Q0 as i32))
        {
            return Ok(u128::from_le_bytes(
                cpu.reg_read_long(reg)?.as_ref().try_into()?,
            ));
        }
    }
    let reg = resolve(arch, name).ok_or_else(|| format!("unsupported trace register {name}"))?;
    Ok(cpu.reg_read(reg)? as u128)
}
