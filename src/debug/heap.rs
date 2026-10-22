use crate::{Emulator, Frame, Result};
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

mod object;
mod tracking;
pub use object::{Field, HeapObject, Referrer, Target};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Allocation {
    pub address: u64,
    pub size: u64,
    pub live: bool,
    pub function: String,
    pub thread: u32,
    pub backtrace: Vec<Frame>,
    pub freed: Option<Vec<Frame>>,
}

impl Allocation {
    pub(crate) fn contains(&self, address: u64) -> bool {
        self.address <= address && address - self.address < self.size
    }

    fn host(address: u64, size: u64) -> Self {
        Self {
            address,
            size,
            live: true,
            function: "host".into(),
            thread: crate::threads::MAIN_TID,
            backtrace: Vec::new(),
            freed: None,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct SavedHeap {
    records: BTreeMap<u64, Allocation>,
    next_order: u64,
    freed_order: Vec<u64>,
}

#[derive(Default)]
pub(crate) struct Heap {
    state: Option<Rc<RefCell<tracking::State>>>,
}

impl Emulator {
    /// Completed libc allocations in entry order, including freed blocks.
    pub fn heap(&self) -> Vec<Allocation> {
        self.heap_trace
            .state
            .as_ref()
            .map_or_else(Vec::new, |state| {
                state.borrow().records.values().cloned().collect()
            })
    }

    pub fn reset_heap(&mut self) {
        if let Some(state) = &self.heap_trace.state {
            let mut state = state.borrow_mut();
            state.records.clear();
            state.live.clear();
            state.freed_order.clear();
        }
    }

    pub fn allocation(&self, address: u64) -> Option<Allocation> {
        if let Some(state) = &self.heap_trace.state {
            let state = state.borrow();
            if let Some(allocation) = state
                .live
                .range(..=address)
                .next_back()
                .map(|(_, order)| &state.records[order])
                .filter(|allocation| allocation.contains(address))
                .or_else(|| {
                    state
                        .freed_order
                        .iter()
                        .rev()
                        .map(|order| &state.records[order])
                        .find(|allocation| allocation.contains(address))
                })
            {
                return Some(allocation.clone());
            }
        }
        self.allocations.iter().find_map(|(&start, &size)| {
            (start <= address && address - start < size).then(|| Allocation::host(start, size))
        })
    }

    pub fn object(&self, address: u64, depth: usize) -> Result<HeapObject> {
        object::build(self, address, depth)
    }

    pub(crate) fn known_allocations(&self) -> Vec<Allocation> {
        let mut blocks: Vec<_> = self
            .heap_trace
            .state
            .as_ref()
            .map_or_else(Vec::new, |state| {
                let state = state.borrow();
                let mut orders: Vec<_> = state.live.values().copied().collect();
                orders.sort_unstable();
                orders
                    .into_iter()
                    .map(|order| state.records[&order].clone())
                    .collect()
            });
        let mut host: Vec<_> = self
            .allocations
            .iter()
            .map(|(&address, &size)| Allocation::host(address, size))
            .collect();
        host.sort_unstable_by_key(|allocation| allocation.address);
        blocks.extend(host);
        blocks
    }

    pub(crate) fn trace_heap(&mut self, enabled: bool) -> Result<()> {
        tracking::configure(self, enabled)
    }

    pub(crate) fn finish_heap(&mut self, failed: bool) -> Result<()> {
        if self.trace.heap
            && let Some(state) = self.heap_trace.state.clone()
        {
            let arch = self.architecture();
            let pc = self.pc()? & !1;
            if failed {
                let thread = self.threads.tid();
                let depth = self.threads.depth;
                let mut state = state.borrow_mut();
                if let Some(stack) = state.pending.get_mut(&thread) {
                    stack.retain(|pending| pending.depth < depth);
                    if stack.is_empty() {
                        state.pending.remove(&thread);
                    }
                }
            } else if pc == crate::RETURN {
                tracking::complete(self.cpu_mut()?, &state, arch, pc);
            }
        }
        Ok(())
    }

    pub(crate) fn saved_heap(&self) -> SavedHeap {
        self.heap_trace
            .state
            .as_ref()
            .map_or_else(SavedHeap::default, |state| {
                let state = state.borrow();
                SavedHeap {
                    records: state.records.clone(),
                    next_order: state.next_order,
                    freed_order: state.freed_order.clone(),
                }
            })
    }

    pub(crate) fn restore_heap_records(&mut self, saved: &SavedHeap) {
        if saved.records.is_empty() && self.heap_trace.state.is_none() {
            return;
        }
        let state = self.heap_trace.state.get_or_insert_with(Default::default);
        let mut state = state.borrow_mut();
        state.records = saved.records.clone();
        state.live = state
            .records
            .iter()
            .filter(|(_, allocation)| allocation.live)
            .map(|(&order, allocation)| (allocation.address, order))
            .collect();
        state.next_order = saved.next_order;
        state.freed_order = saved.freed_order.clone();
        state.pending.clear();
    }
}
