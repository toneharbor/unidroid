use crate::{Emulator, Result};
use serde::Serialize;
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use unicorn_engine::UcHookId;

#[derive(Debug, Clone, Serialize)]
pub struct BlockCount {
    pub address: u64,
    pub location: String,
    pub count: u64,
}

#[derive(Default)]
pub(crate) struct Coverage {
    hooks: Vec<UcHookId>,
    counts: Option<Rc<RefCell<HashMap<u64, u64>>>>,
}

impl Emulator {
    pub fn coverage(&self) -> Vec<BlockCount> {
        let Some(counts) = &self.coverage.counts else {
            return Vec::new();
        };
        let mut blocks: Vec<_> = counts
            .borrow()
            .iter()
            .map(|(&address, &count)| BlockCount {
                address,
                location: self.symbolize(address),
                count,
            })
            .collect();
        blocks.sort_unstable_by_key(|block| block.address);
        blocks
    }

    pub fn reset_coverage(&mut self) {
        if let Some(counts) = &self.coverage.counts {
            *counts.borrow_mut() = HashMap::new();
        }
    }

    pub(crate) fn trace_coverage(&mut self, entries: &[String]) -> Result<()> {
        let Some(selection) =
            self.prepare_range_trace(entries, |emu| std::mem::take(&mut emu.coverage.hooks))?
        else {
            return Ok(());
        };
        let counts = self
            .coverage
            .counts
            .get_or_insert_with(Default::default)
            .clone();
        for (start, end) in selection.ranges {
            let counts = counts.clone();
            let hook = self
                .cpu_mut()?
                .add_block_hook(start, end - 1, move |_, address, _| {
                    *counts.borrow_mut().entry(address & !1).or_default() += 1;
                })?;
            self.coverage.hooks.push(hook);
        }
        self.cpu_mut()?.ctl_flush_tb()?;
        Ok(())
    }
}
