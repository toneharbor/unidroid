use crate::{CpuData, Emulator, Event, Image, Result, arch::Architecture};
use capstone::prelude::*;
use std::{cell::RefCell, rc::Rc, sync::Arc};
use unicorn_engine::{RegisterARM, UcHookId, Unicorn};

mod registers;
#[cfg(test)]
mod tests;

pub(crate) struct Spec {
    name: String,
    offsets: Option<(u64, u64)>,
}
impl Spec {
    pub fn parse(spec: &str) -> Result<Self> {
        let invalid = || {
            format!("invalid code trace {spec:?}: expected libfoo.so or libfoo.so+0x1000-0x2000")
        };
        let (name, offsets) = if let Some((name, range)) = spec.split_once('+') {
            let (start, end) = range.split_once('-').ok_or_else(invalid)?;
            let hex = |value: &str| -> Result<u64> {
                let value = value.strip_prefix("0x").ok_or_else(invalid)?;
                if value.is_empty() || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(invalid().into());
                }
                Ok(u64::from_str_radix(value, 16).map_err(|_| invalid())?)
            };
            let (start, end) = (hex(start)?, hex(end)?);
            if start >= end {
                return Err(invalid().into());
            }
            (name, Some((start, end)))
        } else {
            (spec, None)
        };
        if name.is_empty() || name.contains(['/', '\\', '+']) {
            return Err(invalid().into());
        }
        Ok(Self {
            name: name.into(),
            offsets,
        })
    }
}

#[derive(Default)]
pub(crate) struct Code {
    hooks: Vec<(u64, u64, UcHookId)>,
    block: Option<UcHookId>,
    state: Rc<RefCell<State>>,
}
#[derive(Default)]
struct State {
    disassembler: Option<Capstone>,
    ranges: Vec<(u64, u64)>,
    modules: Vec<(u64, String, Arc<Image>)>,
    pending: Option<Pending>,
    block_start: u64,
}
struct Pending {
    event: Event,
    writes: Vec<String>,
    address: u64,
    supervisor: bool,
    itstate: Option<u8>,
}
impl State {
    fn contains(&self, address: u64) -> bool {
        self.ranges
            .iter()
            .any(|&(start, end)| (start..end).contains(&address))
    }
    fn finish(
        &mut self,
        cpu: &Unicorn<'_, CpuData>,
        arch: Architecture,
        fault: bool,
    ) -> Option<Event> {
        let mut pending = self.pending.take()?;
        if !fault && let Event::Instruction { at, written, .. } = &mut pending.event {
            let values = pending
                .writes
                .iter()
                .map(|name| {
                    let value = if name == "itstate" {
                        u128::from(pending.itstate.ok_or("missing IT state")?)
                    } else {
                        registers::value(cpu, arch, name)?
                    };
                    Ok((name.clone(), value))
                })
                .collect::<Result<Vec<_>>>();
            match values {
                Ok(values) => *written = values,
                Err(error) => return Some(Self::warning(at, pending.address, error)),
            }
        }
        Some(pending.event)
    }
    fn warning(at: &str, address: u64, error: impl std::fmt::Display) -> Event {
        Event::Warning {
            message: format!("instruction trace at {at} (0x{address:x}): {error}"),
        }
    }
    fn location(&self, address: u64) -> String {
        let Some((base, name, image)) = self
            .modules
            .iter()
            .find(|(base, _, image)| (*base..*base + image.size).contains(&address))
        else {
            return format!("0x{address:x}");
        };
        let offset = address - base;
        let mut at = format!("{name}+0x{offset:x}");
        if let Some(symbol) = image.function(offset) {
            at.push_str(&format!(" ({symbol})"));
        }
        at
    }
    fn capture(
        &mut self,
        cpu: &Unicorn<'_, CpuData>,
        arch: Architecture,
        address: u64,
        size: u32,
    ) -> Result<()> {
        let at = self.location(address);
        let cs = self
            .disassembler
            .as_mut()
            .ok_or("code disassembler missing")?;
        let cpsr = if arch == Architecture::Arm32 {
            cpu.reg_read(RegisterARM::CPSR)?
        } else {
            0
        };
        if arch == Architecture::Arm32 {
            cs.set_mode(if cpsr & 32 != 0 {
                capstone::Mode::Thumb
            } else {
                capstone::Mode::Arm
            })?;
        }
        let bytes = cpu.mem_read_as_vec(address, size.min(4) as usize)?;
        // Decode from the Thumb block's start: Unicorn keeps IT state in the
        // translated block, so CPSR alone cannot describe a conditional MOV.
        let (decode, start) = if cpsr & 32 != 0 {
            (
                cpu.mem_read_as_vec(
                    self.block_start,
                    (address
                        .checked_sub(self.block_start)
                        .ok_or("instruction precedes its block")?) as usize
                        + bytes.len(),
                )?,
                self.block_start,
            )
        } else {
            (bytes.clone(), address)
        };
        let instructions = cs.disasm_all(&decode, start)?;
        let mut read = Vec::new();
        let mut writes = Vec::new();
        let (text, supervisor) =
            if let Some(instruction) = instructions.last().filter(|i| i.address() == address) {
                let detail = cs.insn_detail(instruction)?;
                let register_name = |id: capstone::RegId| {
                    cs.reg_name(id)
                        .ok_or_else(|| format!("unknown Capstone register id {}", id.0))
                };
                for &id in detail.regs_read() {
                    let name = register_name(id)?;
                    read.push((name.clone(), registers::value(cpu, arch, &name)?));
                }
                for &id in detail.regs_write() {
                    writes.push(register_name(id)?);
                }
                if let Some(arm) = detail.arch_detail().arm()
                    && arm.update_flags()
                    && !writes
                        .iter()
                        .any(|name| matches!(name.as_str(), "cpsr" | "apsr" | "apsr_nzcv"))
                {
                    writes.push("cpsr".into());
                }
                let mnemonic = instruction.mnemonic().unwrap_or_default();
                let operands = instruction.op_str().unwrap_or_default();
                let supervisor = mnemonic == "svc";
                if supervisor {
                    // The emulated supervisor returns through the ABI result register.
                    writes.push(
                        if arch == Architecture::Arm64 {
                            "x0"
                        } else {
                            "r0"
                        }
                        .into(),
                    );
                }
                (
                    format!("{mnemonic} {operands}").trim_end().to_owned(),
                    supervisor,
                )
            } else {
                ("invalid".into(), false)
            };
        let itstate = bytes
            .first()
            .copied()
            .filter(|_| writes.iter().any(|name| name == "itstate"));
        self.pending = Some(Pending {
            event: Event::Instruction {
                at,
                address,
                bytes: bytes.iter().map(|b| format!("{b:02x}")).collect(),
                text,
                read,
                written: Vec::new(),
            },
            writes,
            address,
            supervisor,
            itstate,
        });
        Ok(())
    }
}

impl Emulator {
    pub(crate) fn trace_code(&mut self, entries: &[String]) -> Result<()> {
        let specs = entries
            .iter()
            .map(|entry| Spec::parse(entry))
            .collect::<Result<Vec<_>>>()?;
        if self.uc.is_none() {
            return Ok(());
        }
        for (start, end, hook) in std::mem::take(&mut self.code.hooks) {
            self.cpu_mut()?.remove_hook(hook)?;
            self.cpu_mut()?.ctl_remove_cache(start, end + 4)?;
        }
        if let Some(hook) = self.code.block.take() {
            self.cpu_mut()?.remove_hook(hook)?;
            self.cpu_mut()?.ctl_flush_tb()?;
        }
        let arch = self.architecture();
        let mut state = self.code.state.borrow_mut();
        state.ranges.clear();
        state.modules.clear();
        for module in &self.modules {
            for spec in specs.iter().filter(|s| s.name == module.name) {
                let (start, end) = spec.offsets.unwrap_or((0, module.image.size));
                let end = end.min(module.image.size);
                if start < end {
                    state.ranges.push((module.base + start, module.base + end));
                }
            }
            state
                .modules
                .push((module.base, module.name.clone(), module.image.clone()));
        }
        state.ranges.sort_unstable();
        let mut ranges: Vec<(u64, u64)> = Vec::new();
        for &(start, end) in &state.ranges {
            if let Some(last) = ranges.last_mut()
                && start <= last.1
            {
                last.1 = last.1.max(end);
            } else {
                ranges.push((start, end));
            }
        }
        state.ranges = ranges.clone();
        if ranges.is_empty() {
            return Ok(());
        }
        if state.disassembler.is_none() {
            state.disassembler = Some(match arch {
                Architecture::Arm32 => Capstone::new()
                    .arm()
                    .mode(capstone::arch::arm::ArchMode::Arm)
                    .detail(true)
                    .build()?,
                Architecture::Arm64 => Capstone::new()
                    .arm64()
                    .mode(capstone::arch::arm64::ArchMode::Arm)
                    .detail(true)
                    .build()?,
            });
        }
        drop(state);
        let state = self.code.state.clone();
        let events = self.events.clone();
        self.code.block = Some(
            self.cpu_mut()?
                .add_block_hook(1, 0, move |cpu, address, _| {
                    let mut state = state.borrow_mut();
                    state.block_start = address;
                    if !state.contains(address)
                        && let Some(event) = state.finish(cpu, arch, false)
                    {
                        events.borrow_mut().emit(event);
                    }
                })?,
        );
        for (start, end) in ranges {
            let state = self.code.state.clone();
            let events = self.events.clone();
            let hook = self
                .cpu_mut()?
                .add_code_hook(start, end, move |cpu, address, size| {
                    let mut state = state.borrow_mut();
                    if let Some(event) = state.finish(cpu, arch, false) {
                        events.borrow_mut().emit(event);
                    }
                    if state.contains(address)
                        && let Err(error) = state.capture(cpu, arch, address, size)
                    {
                        events.borrow_mut().emit(State::warning(
                            &state.location(address),
                            address,
                            error,
                        ));
                    }
                })?;
            self.code.hooks.push((start, end, hook));
            self.cpu_mut()?.ctl_remove_cache(start, end + 4)?;
        }
        self.cpu_mut()?.ctl_flush_tb()?;
        Ok(())
    }

    pub(crate) fn settle_code(&mut self, fault: bool) -> Result<()> {
        if self.trace.code.is_empty() {
            return Ok(());
        }
        let cpu = self.cpu()?;
        let mut state = self.code.state.borrow_mut();
        if !fault && let Some(pending) = &state.pending {
            if cpu.get_data().interrupt == Some(2) && pending.supervisor {
                return Ok(());
            }
            if pending.address == self.pc()? & !1 {
                // Like suspended imports, this hook ran before a stop, not an instruction.
                return Ok(());
            }
        }
        if let Some(event) = state.finish(cpu, self.architecture(), fault) {
            self.events.borrow_mut().emit(event);
        }
        Ok(())
    }

    pub(crate) fn discard_unexecuted_code(&mut self, address: u64) {
        let mut state = self.code.state.borrow_mut();
        if state
            .pending
            .as_ref()
            .is_some_and(|pending| pending.address == address)
        {
            state.pending = None;
        }
    }

    pub(crate) fn finish_code(&mut self, fault: bool) -> Result<()> {
        if !self.trace.code.is_empty()
            && let Some(event) =
                self.code
                    .state
                    .borrow_mut()
                    .finish(self.cpu()?, self.architecture(), fault)
        {
            self.events.borrow_mut().emit(event);
        }
        Ok(())
    }
}
