use super::Snapshot;
use crate::{Mapping, debug::memory_location};
use serde::Serialize;
use std::fmt::Write;

const CHANGE_GAP: usize = 8;

#[derive(Debug, Serialize)]
pub struct Diff {
    pub registers: Vec<(String, u64, u64)>,
    pub mapped: Vec<Mapping>,
    pub unmapped: Vec<Mapping>,
    pub changes: Vec<Change>,
}

#[derive(Debug, Serialize)]
pub struct Change {
    pub address: u64,
    pub location: String,
    pub before: Vec<u8>,
    pub after: Vec<u8>,
}

impl Diff {
    pub fn report(&self) -> String {
        let mut text = String::from("registers:\n");
        if self.registers.is_empty() {
            text.push_str("  (none)\n");
        }
        for (name, before, after) in &self.registers {
            let _ = writeln!(text, "  {name} 0x{before:x} -> 0x{after:x}");
        }
        for (section, mappings) in [("mapped", &self.mapped), ("unmapped", &self.unmapped)] {
            let _ = writeln!(text, "{section}:");
            if mappings.is_empty() {
                text.push_str("  (none)\n");
            }
            for mapping in mappings {
                let _ = write!(
                    text,
                    "  0x{:x}-0x{:x} {}",
                    mapping.start, mapping.end, mapping.perms
                );
                if !mapping.name.is_empty() {
                    let _ = write!(text, " {}", mapping.name);
                }
                text.push('\n');
            }
        }
        text.push_str("changes:\n");
        if self.changes.is_empty() {
            text.push_str("  (none)\n");
        }
        for change in &self.changes {
            let _ = writeln!(text, "  {}, {} bytes", change.location, change.before.len());
            for (row, (before, after)) in change
                .before
                .chunks(16)
                .zip(change.after.chunks(16))
                .enumerate()
            {
                for (sign, bytes) in [('-', before), ('+', after)] {
                    let _ = write!(text, "    {:04x} {sign}", row * 16);
                    for byte in bytes {
                        let _ = write!(text, " {byte:02x}");
                    }
                    text.push('\n');
                }
            }
        }
        text.pop();
        text
    }
}

impl Snapshot {
    pub fn diff(&self, after: &Self) -> Diff {
        let mut diff = Diff {
            registers: after
                .registers
                .iter()
                .filter_map(|(name, value)| {
                    let (_, old) = self.registers.iter().find(|(n, _)| n == name)?;
                    (old != value).then(|| (name.clone(), *old, *value))
                })
                .collect(),
            mapped: Vec::new(),
            unmapped: Vec::new(),
            changes: Vec::new(),
        };
        let mut boundaries: Vec<_> = self
            .memory
            .iter()
            .chain(&after.memory)
            .flat_map(|(m, _)| [m.start, m.end])
            .collect();
        boundaries.sort_unstable();
        boundaries.dedup();
        let mut old: Vec<_> = self.memory.iter().collect();
        let mut new: Vec<_> = after.memory.iter().collect();
        old.sort_unstable_by_key(|(m, _)| m.start);
        new.sort_unstable_by_key(|(m, _)| m.start);
        let mut old = old.into_iter().peekable();
        let mut new = new.into_iter().peekable();
        let mut gap = Vec::new();
        for pair in boundaries.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            while old.peek().is_some_and(|(m, _)| m.end <= start) {
                old.next();
            }
            while new.peek().is_some_and(|(m, _)| m.end <= start) {
                new.next();
            }
            let before = old.peek().copied().filter(|(m, _)| m.start <= start);
            let current = new.peek().copied().filter(|(m, _)| m.start <= start);
            match (before, current) {
                (Some((mapping_a, bytes_a)), Some((mapping_b, bytes_b))) => {
                    let bytes_a = &bytes_a
                        [(start - mapping_a.start) as usize..(end - mapping_a.start) as usize];
                    let bytes_b = &bytes_b
                        [(start - mapping_b.start) as usize..(end - mapping_b.start) as usize];
                    for (offset, (&a, &b)) in bytes_a.iter().zip(bytes_b).enumerate() {
                        if a == b {
                            if gap.len() < CHANGE_GAP {
                                gap.push(a);
                            }
                            continue;
                        }
                        let address = start + offset as u64;
                        if let Some(change) = diff.changes.last_mut().filter(|change| {
                            gap.len() < CHANGE_GAP
                                && change.address + (change.before.len() + gap.len()) as u64
                                    == address
                        }) {
                            change.before.extend_from_slice(&gap);
                            change.after.extend_from_slice(&gap);
                            change.before.push(a);
                            change.after.push(b);
                        } else {
                            diff.changes.push(Change {
                                address,
                                location: memory_location(address, &after.modules, Some(mapping_b)),
                                before: vec![a],
                                after: vec![b],
                            });
                        }
                        gap.clear();
                    }
                }
                (Some((mapping, _)), None) => diff.unmapped.push(Mapping {
                    start,
                    end,
                    ..mapping.clone()
                }),
                (None, Some((mapping, _))) => diff.mapped.push(Mapping {
                    start,
                    end,
                    ..mapping.clone()
                }),
                (None, None) => {}
            }
        }
        diff
    }
}
