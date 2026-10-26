use crate::{CpuData, Emulator, Event, Result, arch::Architecture};
use capstone::prelude::*;
use std::{cell::RefCell, rc::Rc};
use unicorn_engine::{
    RegisterARM, UcHookId, Unicorn,
    unicorn_const::{HookType, MemType},
};

mod registers;
#[cfg(test)]
mod tests;

mod selection;
pub(crate) use selection::{Selection, Spec};

#[derive(Default)]
pub(crate) struct Code {
    hooks: Vec<UcHookId>,
    block: Option<UcHookId>,
    memory: Option<UcHookId>,
    state: Rc<RefCell<State>>,
}
#[derive(Default)]
struct State {
    disassembler: Option<Capstone>,
    selection: Selection,
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
    fn capture(
        &mut self,
        cpu: &Unicorn<'_, CpuData>,
        arch: Architecture,
        address: u64,
        size: u32,
    ) -> Result<()> {
        let at = self.selection.location(address);
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
                memory: Vec::new(),
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
        let Some(selection) = self.prepare_range_trace(entries, |emu| {
            let mut hooks = std::mem::take(&mut emu.code.hooks);
            hooks.extend(emu.code.block.take());
            hooks.extend(emu.code.memory.take());
            hooks
        })?
        else {
            *self.code.state.borrow_mut() = State::default();
            return Ok(());
        };
        let arch = self.architecture();
        let mut state = self.code.state.borrow_mut();
        let ranges = selection.instruction_hooks();
        state.selection = selection;
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
        self.code.memory = Some(self.memory_hook(
            HookType::MEM_READ_AFTER | HookType::MEM_WRITE,
            1,
            0,
            move |_, kind, address, size, value| {
                if let Some(Pending {
                    event: Event::Instruction { memory, .. },
                    ..
                }) = &mut state.borrow_mut().pending
                {
                    memory.push(crate::MemoryAccess {
                        write: kind == MemType::WRITE,
                        address,
                        size,
                        value: value as u64,
                    });
                }
                true
            },
        )?);
        let state = self.code.state.clone();
        let events = self.events.clone();
        self.code.block = Some(
            self.cpu_mut()?
                .add_block_hook(1, 0, move |cpu, address, _| {
                    let mut state = state.borrow_mut();
                    state.block_start = address;
                    if !state.selection.contains(address)
                        && let Some(event) = state.finish(cpu, arch, false)
                    {
                        events.borrow_mut().emit(event);
                    }
                })?,
        );
        for (start, end) in ranges {
            let state = self.code.state.clone();
            let events = self.events.clone();
            let hook = self.add_code_hook(start, end - 1, move |cpu, address, size| {
                let mut state = state.borrow_mut();
                if let Some(event) = state.finish(cpu, arch, false) {
                    events.borrow_mut().emit(event);
                }
                if state.selection.contains(address)
                    && let Err(error) = state.capture(cpu, arch, address, size)
                {
                    events.borrow_mut().emit(State::warning(
                        &state.selection.location(address),
                        address,
                        error,
                    ));
                }
            })?;
            self.code.hooks.push(hook);
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
