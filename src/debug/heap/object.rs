use super::Allocation;
use crate::{Emulator, Mapping, Result};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize)]
pub struct Field {
    pub offset: u64,
    pub value: u64,
    pub target: Option<Target>,
    pub string: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    Block {
        address: u64,
        offset: u64,
        object: Option<Box<HeapObject>>,
    },
    Location {
        location: String,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct Referrer {
    pub address: u64,
    pub location: String,
    pub block: Option<u64>,
    pub object: Option<Box<HeapObject>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct HeapObject {
    pub address: u64,
    pub size: u64,
    pub allocation: Allocation,
    pub fields: Vec<Field>,
    pub referrers: Vec<Referrer>,
}

struct Browser<'a> {
    emu: &'a Emulator,
    runs: Vec<(Mapping, Vec<u8>)>,
    blocks: Vec<Allocation>,
    visited: HashSet<u64>,
}

pub(super) fn build(emu: &Emulator, address: u64, depth: usize) -> Result<HeapObject> {
    if depth > 64 {
        return Err("depth must be at most 64".into());
    }
    let blocks = emu.known_allocations();
    let maps = emu.maps()?;
    let allocation = blocks
        .iter()
        .find(|allocation| allocation.contains(address))
        .cloned()
        .ok_or_else(|| {
            format!(
                "{} is not inside a known allocation",
                crate::debug::memory_location(
                    address,
                    &emu.modules(),
                    maps.iter().find(|m| m.start <= address && address < m.end)
                )
            )
        })?;
    // All recursion uses this one copy, including strings and inbound pointers.
    let runs = maps
        .into_iter()
        .map(|mapping| {
            let bytes = emu.read_bytes(mapping.start, (mapping.end - mapping.start) as usize)?;
            Ok((mapping, bytes))
        })
        .collect::<Result<_>>()?;
    let mut browser = Browser {
        emu,
        runs,
        blocks,
        visited: HashSet::from([allocation.address]),
    };
    browser.expand(allocation, depth)
}

impl Browser<'_> {
    fn block(&self, address: u64) -> Option<&Allocation> {
        self.blocks
            .iter()
            .find(|allocation| allocation.contains(address))
    }

    fn location(&self, address: u64) -> String {
        if self
            .emu
            .modules
            .iter()
            .any(|module| (module.base..module.base + module.image.size).contains(&address))
        {
            self.emu.symbolize(address)
        } else {
            crate::debug::memory_location(
                address,
                &self.emu.modules(),
                self.runs
                    .iter()
                    .map(|(mapping, _)| mapping)
                    .find(|mapping| mapping.start <= address && address < mapping.end),
            )
        }
    }

    fn byte(&self, address: u64) -> Option<u8> {
        let index = self
            .runs
            .partition_point(|(mapping, _)| mapping.start <= address)
            .checked_sub(1)?;
        let (mapping, bytes) = &self.runs[index];
        bytes.get((address - mapping.start) as usize).copied()
    }

    fn string(&self, address: u64) -> Option<String> {
        let mut bytes = Vec::new();
        for offset in 0..64 {
            let byte = self.byte(address.checked_add(offset)?)?;
            if byte == 0 {
                return (bytes.len() >= 4).then(|| String::from_utf8(bytes).unwrap());
            }
            if !(b' '..=b'~').contains(&byte) {
                return None;
            }
            bytes.push(byte);
        }
        None
    }

    fn nested(&mut self, allocation: &Allocation, depth: usize) -> Result<Option<Box<HeapObject>>> {
        if depth > 0 && self.visited.insert(allocation.address) {
            Ok(Some(Box::new(self.expand(allocation.clone(), depth - 1)?)))
        } else {
            Ok(None)
        }
    }

    fn expand(&mut self, allocation: Allocation, depth: usize) -> Result<HeapObject> {
        let width = self.emu.pointer_size();
        let mut fields = Vec::new();
        let first = allocation.address.next_multiple_of(width) - allocation.address;
        for offset in (first..allocation.size).step_by(width as usize) {
            if allocation.size - offset < width {
                break;
            }
            let mut bytes = [0; 8];
            for i in 0..width {
                bytes[i as usize] =
                    self.byte(allocation.address + offset + i).ok_or_else(|| {
                        format!(
                            "cannot read allocation at {}",
                            self.location(allocation.address + offset)
                        )
                    })?;
            }
            let value = u64::from_le_bytes(bytes);
            let target = if let Some(block) = self.block(value).cloned() {
                Some(Target::Block {
                    address: block.address,
                    offset: value - block.address,
                    object: self.nested(&block, depth)?,
                })
            } else if self
                .runs
                .iter()
                .any(|(mapping, _)| mapping.start <= value && value < mapping.end)
            {
                Some(Target::Location {
                    location: self.location(value),
                })
            } else {
                None
            };
            fields.push(Field {
                offset,
                value,
                target,
                string: self.string(value),
            });
        }
        let mut addresses = Vec::new();
        'mappings: for (mapping, bytes) in &self.runs {
            for (offset, bytes) in bytes.chunks_exact(width as usize).enumerate() {
                let address = mapping.start + offset as u64 * width;
                if allocation.contains(address) {
                    continue;
                }
                let mut word = [0; 8];
                word[..bytes.len()].copy_from_slice(bytes);
                if allocation.contains(u64::from_le_bytes(word)) {
                    addresses.push(address);
                    if addresses.len() == crate::SEARCH_LIMIT {
                        break 'mappings;
                    }
                }
            }
        }
        let mut referrers = Vec::new();
        for address in addresses {
            let block = self.block(address).cloned();
            let object = match &block {
                Some(block) => self.nested(block, depth)?,
                None => None,
            };
            referrers.push(Referrer {
                address,
                location: self.location(address),
                block: block.map(|allocation| allocation.address),
                object,
            });
        }
        Ok(HeapObject {
            address: allocation.address,
            size: allocation.size,
            allocation,
            fields,
            referrers,
        })
    }
}
