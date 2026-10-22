use crate::{Image, Loaded, Result};
use std::sync::Arc;

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
    modules: Vec<(u64, String, Arc<Image>)>,
}
impl Selection {
    pub fn new(entries: &[String], modules: &[Loaded]) -> Result<Self> {
        let specs = entries
            .iter()
            .map(|entry| Spec::parse(entry))
            .collect::<Result<Vec<_>>>()?;
        let mut ranges = Vec::new();
        for module in modules {
            for spec in specs.iter().filter(|s| s.name == module.name) {
                let (start, end) = spec.offsets.unwrap_or((0, module.image.size));
                let end = end.min(module.image.size);
                if start < end {
                    ranges.push((module.base + start, module.base + end));
                }
            }
        }
        let merged = merge(ranges);
        if merged.is_empty() {
            return Ok(Self::default());
        }
        Ok(Self {
            ranges: merged,
            modules: modules
                .iter()
                .map(|m| (m.base, m.name.clone(), m.image.clone()))
                .collect(),
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
        crate::debug::symbolize(
            address,
            self.modules
                .iter()
                .map(|(base, name, image)| (*base, name.as_str(), image.as_ref())),
        )
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
