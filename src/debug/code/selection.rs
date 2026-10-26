use crate::debug::modules::SharedModules;
use crate::{Emulator, Result};
use unicorn_engine::UcHookId;

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
pub(crate) struct Selection {
    pub ranges: Vec<(u64, u64)>,
    modules: SharedModules,
}
impl Selection {
    pub fn new(entries: &[String], modules: SharedModules) -> Result<Self> {
        let specs = entries
            .iter()
            .map(|entry| Spec::parse(entry))
            .collect::<Result<Vec<_>>>()?;
        let mut ranges = Vec::new();
        for module in &modules.borrow().entries {
            let module = &module.info;
            for spec in specs.iter().filter(|s| s.name == module.name) {
                let (start, end) = spec.offsets.unwrap_or((0, module.size));
                let end = end.min(module.size);
                if start < end {
                    ranges.push((module.base + start, module.base + end));
                }
            }
        }
        Ok(Self {
            ranges: merge(ranges),
            modules,
        })
    }
    pub fn instruction_hooks(&self) -> Vec<(u64, u64)> {
        // Observe the next instruction even when the exclusive end falls
        // inside a four-byte instruction. Merge overlaps to avoid double hooks.
        merge(
            self.ranges
                .iter()
                .map(|&(start, end)| (start, end + 4))
                .collect(),
        )
    }
    pub fn contains(&self, address: u64) -> bool {
        self.ranges
            .iter()
            .any(|&(start, end)| (start..end).contains(&address))
    }
    pub fn location(&self, address: u64) -> String {
        self.modules.borrow().symbolize(address)
    }
}

impl Emulator {
    pub(crate) fn prepare_range_trace(
        &mut self,
        entries: &[String],
        take_hooks: impl FnOnce(&mut Self) -> Vec<UcHookId>,
    ) -> Result<Option<Selection>> {
        let selection = Selection::new(entries, self.module_table.clone())?;
        if self.uc.is_none() {
            return Ok(None);
        }
        let hooks = take_hooks(self);
        crate::debug::hooks::remove_hooks(self.cpu_mut()?, hooks)?;
        Ok((!selection.ranges.is_empty()).then_some(selection))
    }
}

fn merge(mut ranges: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    ranges.sort_unstable();
    let mut merged: Vec<(u64, u64)> = Vec::new();
    for (start, end) in ranges {
        if let Some(last) = merged.last_mut()
            && start <= last.1
        {
            last.1 = last.1.max(end);
        } else {
            merged.push((start, end));
        }
    }
    merged
}
