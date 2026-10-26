use super::code::Selection;
use crate::{CpuData, Emulator, Event, Result, arch::Architecture};
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use unicorn_engine::{RegisterARM, RegisterARM64, UcHookId, Unicorn};

pub(crate) struct Calls {
    hooks: Vec<UcHookId>,
    state: Rc<RefCell<State>>,
}
struct State {
    selection: Selection,
    pending: HashMap<(usize, u32), Pending>,
}
struct Pending {
    from: u64,
    target: u64,
    link: u64,
}

impl Emulator {
    pub(crate) fn trace_calls(&mut self, entries: &[String]) -> Result<()> {
        let Some(selection) = self.prepare_range_trace(entries, |emu| {
            emu.calls.take().map_or_else(Vec::new, |calls| calls.hooks)
        })?
        else {
            return Ok(());
        };
        let ranges = selection.ranges.clone();
        let state = Rc::new(RefCell::new(State {
            selection,
            pending: HashMap::new(),
        }));
        let arch = self.architecture();
        let mut hooks = Vec::new();
        let targets = state.clone();
        let events = self.events.clone();
        hooks.push(
            self.cpu_mut()?
                .add_block_hook(1, 0, move |cpu, address, _| {
                    let key = (cpu.get_data().depth, cpu.get_data().thread);
                    let mut state = targets.borrow_mut();
                    let Some(pending) = state.pending.remove(&key) else {
                        return;
                    };
                    if pending.target != address
                        || cpu.reg_read(arch.lr()).expect("link register") != pending.link
                    {
                        return;
                    }
                    events.borrow_mut().emit(Event::FunctionCall {
                        to: state.selection.location(address),
                        address,
                        args: super::arguments::register_arguments(cpu, arch, 0),
                        from: state.selection.location(pending.from),
                    });
                })?,
        );
        for (start, end) in ranges {
            let sites = state.clone();
            let events = self.events.clone();
            hooks.push(self.cpu_mut()?.add_code_hook(
                start,
                end - 1,
                move |cpu, address, size| {
                    let mut state = sites.borrow_mut();
                    match call(cpu, arch, address, size) {
                        Ok(Some(pending)) => {
                            state
                                .pending
                                .insert((cpu.get_data().depth, cpu.get_data().thread), pending);
                        }
                        Ok(None) => {}
                        Err(error) => events.borrow_mut().emit(Event::Warning {
                            message: format!(
                                "function call trace from {}: {error}",
                                state.selection.location(address)
                            ),
                        }),
                    }
                },
            )?);
        }
        self.calls = Some(Calls { hooks, state });
        self.cpu_mut()?.ctl_flush_tb()?;
        Ok(())
    }

    pub(crate) fn suspend_calls(&mut self, address: u64) {
        if let Some(calls) = &self.calls {
            let mut state = calls.state.borrow_mut();
            let key = (self.threads.depth, self.threads.tid());
            if state
                .pending
                .get(&key)
                .is_some_and(|pending| pending.from == address)
            {
                state.pending.remove(&key);
            }
        }
    }

    pub(crate) fn finish_calls(&mut self) {
        if let Some(calls) = &self.calls {
            calls
                .state
                .borrow_mut()
                .pending
                .retain(|&(depth, _), _| depth != self.threads.depth);
        }
    }
}

fn call(
    cpu: &Unicorn<'_, CpuData>,
    arch: Architecture,
    address: u64,
    size: u32,
) -> Result<Option<Pending>> {
    let mut bytes = [0; 4];
    cpu.mem_read(address, &mut bytes[..size.min(4) as usize])?;
    let word = u32::from_le_bytes(bytes);
    let cpsr = if arch == Architecture::Arm32 {
        cpu.reg_read(RegisterARM::CPSR)?
    } else {
        0
    };
    let thumb = cpsr & 32 != 0;
    let register = |n: u32| -> Result<u64> {
        let id = match (arch, n) {
            (Architecture::Arm64, 29) => RegisterARM64::FP as i32,
            (Architecture::Arm64, 30) => RegisterARM64::LR as i32,
            (Architecture::Arm64, 31) => RegisterARM64::XZR as i32,
            (Architecture::Arm32, 13) => RegisterARM::SP as i32,
            (Architecture::Arm32, 14) => RegisterARM::LR as i32,
            (Architecture::Arm32, 15) => RegisterARM::PC as i32,
            _ => arch.gp(n as usize),
        };
        Ok(cpu.reg_read(id)?)
    };
    let target = if arch == Architecture::Arm64 {
        if word & 0xfc00_0000 == 0x9400_0000 {
            address.wrapping_add_signed(((word << 6) as i32 >> 4) as i64)
        } else if word & 0xffff_fc1f == 0xd63f_0000 {
            register((word >> 5) & 31)?
        } else {
            return Ok(None);
        }
    } else if thumb {
        let first = word & 0xffff;
        let second = word >> 16;
        if first & 0xff87 == 0x4780 {
            register((first >> 3) & 15)?
        } else if first & 0xf800 == 0xf000 && second & 0xc000 == 0xc000 {
            let exchange = second & 0x1000 == 0;
            if exchange && second & 1 != 0 {
                return Ok(None);
            }
            let sign = (first >> 10) & 1;
            let i1 = 1 ^ ((second >> 13) & 1) ^ sign;
            let i2 = 1 ^ ((second >> 11) & 1) ^ sign;
            let offset = (sign << 24)
                | (i1 << 23)
                | (i2 << 22)
                | ((first & 0x3ff) << 12)
                | ((second & 0x7ff) << 1);
            let base = if exchange {
                (address + 4) & !3
            } else {
                address + 4
            };
            base.wrapping_add_signed(((offset << 7) as i32 >> 7) as i64)
        } else {
            return Ok(None);
        }
    } else if word & 0xfe00_0000 == 0xfa00_0000 {
        // BLX immediate is unconditional; bit 24 supplies the halfword offset.
        (address + 8).wrapping_add_signed(((word << 8) as i32 >> 6) as i64)
            + ((word >> 23) & 2) as u64
    } else {
        if !condition(word >> 28, cpsr) {
            return Ok(None);
        }
        if word & 0x0f00_0000 == 0x0b00_0000 {
            (address + 8).wrapping_add_signed(((word << 8) as i32 >> 6) as i64)
        } else if word & 0x0fff_fff0 == 0x012f_ff30 {
            register(word & 15)?
        } else {
            return Ok(None);
        }
    };
    Ok(Some(Pending {
        from: address,
        target: target & arch.pointer_mask() & !1,
        link: (address + u64::from(size)) | u64::from(thumb),
    }))
}

fn condition(condition: u32, cpsr: u64) -> bool {
    let n = cpsr & (1 << 31) != 0;
    let z = cpsr & (1 << 30) != 0;
    let c = cpsr & (1 << 29) != 0;
    let v = cpsr & (1 << 28) != 0;
    match condition {
        0 => z,
        1 => !z,
        2 => c,
        3 => !c,
        4 => n,
        5 => !n,
        6 => v,
        7 => !v,
        8 => c && !z,
        9 => !c || z,
        10 => n == v,
        11 => n != v,
        12 => !z && n == v,
        13 => z || n != v,
        14 => true,
        _ => false,
    }
}
