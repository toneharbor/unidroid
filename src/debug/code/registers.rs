use crate::{CpuData, Result, arch::Architecture};
use unicorn_engine::{RegisterARM as A, RegisterARM64 as X, Unicorn};

pub(super) fn value(cpu: &Unicorn<'_, CpuData>, arch: Architecture, name: &str) -> Result<u128> {
    let indexed = |prefix: &str, limit: i32, base: i32| {
        name.strip_prefix(prefix)?
            .parse::<i32>()
            .ok()
            .filter(|&n| (0..limit).contains(&n))
            .map(|n| base + n)
    };
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
        let reg = match name {
            "sp" => Some(A::SP as i32),
            "lr" => Some(A::LR as i32),
            "pc" => Some(A::PC as i32),
            "cpsr" => Some(A::CPSR as i32),
            "apsr" => Some(A::APSR as i32),
            "apsr_nzcv" => Some(A::APSR_NZCV as i32),
            "fpscr" => Some(A::FPSCR as i32),
            "fpexc" => Some(A::FPEXC as i32),
            "fpscr_nzcv" => Some(A::FPSCR as i32),
            "sb" => Some(A::R9 as i32),
            "sl" => Some(A::R10 as i32),
            "fp" => Some(A::R11 as i32),
            "ip" => Some(A::R12 as i32),
            "fpsid" => Some(A::FPSID as i32),
            "spsr" => Some(A::SPSR as i32),
            _ => indexed("r", 13, A::R0 as i32)
                .or_else(|| indexed("s", 32, A::S0 as i32))
                .or_else(|| indexed("d", 32, A::D0 as i32)),
        }
        .ok_or_else(|| format!("unsupported trace register {name}"))?;
        let value = cpu.reg_read(reg)? as u128;
        return Ok(if name == "fpscr_nzcv" {
            value & 0xf0000000
        } else {
            value
        });
    }
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
    if let Some(reg) = indexed("v", 32, X::Q0 as i32).or_else(|| indexed("q", 32, X::Q0 as i32)) {
        return Ok(u128::from_le_bytes(
            cpu.reg_read_long(reg)?.as_ref().try_into()?,
        ));
    }
    let reg = match name {
        "fp" | "x29" => Some(X::FP as i32),
        "lr" | "x30" => Some(X::LR as i32),
        "sp" => Some(X::SP as i32),
        "wsp" => Some(X::WSP as i32),
        "pc" => Some(X::PC as i32),
        "nzcv" => Some(X::NZCV as i32),
        "fpcr" => Some(X::FPCR as i32),
        "fpsr" => Some(X::FPSR as i32),
        _ => indexed("x", 29, X::X0 as i32)
            .or_else(|| indexed("w", 31, X::W0 as i32))
            .or_else(|| indexed("b", 32, X::B0 as i32))
            .or_else(|| indexed("h", 32, X::H0 as i32))
            .or_else(|| indexed("s", 32, X::S0 as i32))
            .or_else(|| indexed("d", 32, X::D0 as i32)),
    }
    .ok_or_else(|| format!("unsupported trace register {name}"))?;
    Ok(cpu.reg_read(reg)? as u128)
}
