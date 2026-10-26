use super::Allocation;
use crate::debug::modules::SharedModules;
use crate::{CpuData, Emulator, Frame, Result, arch::Architecture};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    ops::Bound::{Excluded, Included, Unbounded},
    rc::Rc,
};
use unicorn_engine::{UcHookId, Unicorn};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Function {
    Malloc,
    Calloc,
    Realloc,
    Free,
    Memalign,
    PosixMemalign,
}

pub(super) struct Pending {
    function: Function,
    return_address: u64,
    sp: u64,
    args: [u64; 3],
    allocation: Option<u64>,
    thread: u32,
    pub depth: usize,
    order: u64,
    backtrace: Vec<Frame>,
}

#[derive(Default)]
pub(super) struct State {
    pub records: BTreeMap<u64, Allocation>,
    pub live: BTreeMap<u64, u64>,
    pub next_order: u64,
    pub freed_order: Vec<u64>,
    pub pending: HashMap<u32, Vec<Pending>>,
    entry_hooks: HashMap<u64, UcHookId>,
    return_hooks: HashMap<u64, UcHookId>,
}

pub(super) fn configure(emu: &mut Emulator, enabled: bool) -> Result<()> {
    if emu.uc.is_none() || (!enabled && emu.heap_trace.state.is_none()) {
        return Ok(());
    }
    let state = emu
        .heap_trace
        .state
        .get_or_insert_with(Default::default)
        .clone();
    if !enabled {
        let mut data = state.borrow_mut();
        data.pending.clear();
        let hooks: Vec<_> = data.entry_hooks.drain().map(|(_, hook)| hook).collect();
        let hooks: Vec<_> = hooks
            .into_iter()
            .chain(data.return_hooks.drain().map(|(_, hook)| hook))
            .collect();
        crate::debug::hooks::remove_hooks(emu.cpu_mut()?, hooks)?;
        return Ok(());
    }
    let mut targets = BTreeMap::new();
    if let Some(module) = emu.modules.iter().find(|module| module.name == "libc.so") {
        for (name, function) in [
            ("malloc", Function::Malloc),
            ("calloc", Function::Calloc),
            ("realloc", Function::Realloc),
            ("free", Function::Free),
            ("memalign", Function::Memalign),
            ("posix_memalign", Function::PosixMemalign),
        ] {
            if let Some(&index) = module.image.exports.get(name) {
                let symbol = &module.image.symbols[index];
                let address = if symbol.absolute {
                    symbol.value
                } else {
                    module.base + symbol.value
                };
                targets.insert(address & !1, function);
            }
        }
    }
    let obsolete: Vec<_> = state
        .borrow()
        .entry_hooks
        .keys()
        .filter(|address| !targets.contains_key(address))
        .copied()
        .collect();
    let mut changed = false;
    for address in obsolete {
        let hook = state.borrow_mut().entry_hooks.remove(&address).unwrap();
        emu.cpu_mut()?.remove_hook(hook)?;
        changed = true;
    }
    let arch = emu.architecture();
    for (address, function) in targets {
        if state.borrow().entry_hooks.contains_key(&address) {
            continue;
        }
        let callback = state.clone();
        let modules = emu.module_table.clone();
        let hook = emu
            .cpu_mut()?
            .add_code_hook(address, address, move |cpu, _, _| {
                enter(cpu, &callback, &modules, arch, function);
            })?;
        state.borrow_mut().entry_hooks.insert(address, hook);
        changed = true;
    }
    if changed {
        emu.cpu_mut()?.ctl_flush_tb()?;
    }
    Ok(())
}

fn enter(
    cpu: &mut Unicorn<'_, CpuData>,
    state: &Rc<RefCell<State>>,
    modules: &SharedModules,
    arch: Architecture,
    function: Function,
) {
    let thread = cpu.get_data().thread;
    let depth = cpu.get_data().depth;
    let return_address = cpu.reg_read(arch.lr()).expect("link register") & !1;
    let sp = cpu.reg_read(arch.sp()).expect("stack pointer");
    let args = std::array::from_fn(|i| cpu.reg_read(arch.gp(i)).expect("argument register"));
    let mut data = state.borrow_mut();
    let allocation = if matches!(function, Function::Free | Function::Realloc) && args[0] != 0 {
        data.live.get(&args[0]).copied()
    } else {
        None
    };
    // A debugger stop can replay this entry before its first instruction executes.
    if let Some(pending) = data
        .pending
        .get_mut(&thread)
        .and_then(|stack| stack.last_mut())
        .filter(|pending| {
            pending.function == function
                && pending.sp == sp
                && pending.return_address == return_address
                && pending.depth == depth
        })
    {
        if pending.args[0] != args[0] {
            pending.allocation = allocation;
        }
        pending.args = args;
        return;
    }
    let backtrace = crate::debug::backtrace_addresses(cpu, arch, true)
        .expect("heap backtrace")
        .into_iter()
        .map(|address| modules.borrow().frame(address))
        .collect();
    let order = data.next_order;
    data.next_order += 1;
    data.pending.entry(thread).or_default().push(Pending {
        function,
        return_address,
        sp,
        args,
        allocation,
        thread,
        depth,
        order,
        backtrace,
    });
    if data.return_hooks.contains_key(&return_address) {
        return;
    }
    drop(data);
    let callback = state.clone();
    let hook = cpu
        .add_code_hook(return_address, return_address, move |cpu, pc, _| {
            complete(cpu, &callback, arch, pc);
        })
        .expect("heap return hook");
    cpu.ctl_remove_cache(return_address, return_address + 1)
        .expect("heap return hook cache");
    state.borrow_mut().return_hooks.insert(return_address, hook);
}

// Both guest return hooks and the host return boundary use this completion path.
pub(super) fn complete(
    cpu: &Unicorn<'_, CpuData>,
    state: &Rc<RefCell<State>>,
    arch: Architecture,
    pc: u64,
) {
    let thread = cpu.get_data().thread;
    let sp = cpu.reg_read(arch.sp()).expect("stack pointer");
    let result = cpu.reg_read(arch.gp(0)).expect("result register");
    let mut data = state.borrow_mut();
    while let Some(stack) = data.pending.get_mut(&thread) {
        let Some(pending) = stack.last() else {
            break;
        };
        if pending.return_address != pc || sp < pending.sp || pending.depth != cpu.get_data().depth
        {
            break;
        }
        let pending = stack.pop().unwrap();
        if stack.is_empty() {
            data.pending.remove(&thread);
        }
        record(cpu, arch, &mut data, pending, result);
    }
}

fn record(
    cpu: &Unicorn<'_, CpuData>,
    arch: Architecture,
    state: &mut State,
    pending: Pending,
    result: u64,
) {
    let (address, size, function) = match pending.function {
        Function::Free => {
            release(state, pending.allocation, &pending.backtrace);
            return;
        }
        Function::Malloc => (result, pending.args[0], "malloc"),
        Function::Calloc => (
            result,
            pending.args[0].wrapping_mul(pending.args[1]),
            "calloc",
        ),
        Function::Memalign => (result, pending.args[1], "memalign"),
        Function::Realloc => {
            if result == 0 {
                return;
            }
            if result == pending.args[0] {
                if let Some(order) = pending.allocation {
                    if state.records.get(&order).is_some_and(|a| a.live) {
                        retire_overlaps(state, result, pending.args[1], order);
                        state.records.get_mut(&order).unwrap().size = pending.args[1];
                    }
                    return;
                }
            } else {
                release(state, pending.allocation, &pending.backtrace);
            }
            (result, pending.args[1], "realloc")
        }
        Function::PosixMemalign => {
            if result != 0 {
                return;
            }
            let mut bytes = [0; 8];
            if cpu
                .mem_read(pending.args[0], &mut bytes[..arch.pointer_size() as usize])
                .is_err()
            {
                return;
            }
            (u64::from_le_bytes(bytes), pending.args[2], "posix_memalign")
        }
    };
    if address == 0 {
        return;
    }
    retire_overlaps(state, address, size, pending.order);
    state.live.insert(address, pending.order);
    state.records.insert(
        pending.order,
        Allocation {
            address,
            size,
            live: true,
            function: function.into(),
            thread: pending.thread,
            backtrace: pending.backtrace,
            freed: None,
        },
    );
}

fn retire_overlaps(state: &mut State, address: u64, size: u64, keep: u64) {
    // Include the same start even for a zero-size block; an overflowing end
    // covers all remaining addresses, as the subtraction-based check did.
    let end = address.checked_add(size.max(1)).map_or(Unbounded, Excluded);
    let overlaps: Vec<_> = state
        .live
        .range(..address)
        .next_back()
        .filter(|&(_, order)| state.records[order].contains(address))
        .into_iter()
        .chain(state.live.range((Included(address), end)))
        .filter(|&(_, &order)| order != keep)
        .map(|(&start, &order)| (start, order))
        .collect();
    for (start, order) in overlaps {
        let allocation = state.records.get_mut(&order).unwrap();
        allocation.live = false;
        allocation.freed = None;
        state.live.remove(&start);
    }
}

fn release(state: &mut State, order: Option<u64>, backtrace: &[Frame]) {
    if let Some(order) = order
        && let Some(allocation) = state.records.get_mut(&order).filter(|a| a.live)
    {
        allocation.live = false;
        allocation.freed = Some(backtrace.to_vec());
        if state.live.get(&allocation.address) == Some(&order) {
            state.live.remove(&allocation.address);
        }
        state.freed_order.push(order);
    }
}
