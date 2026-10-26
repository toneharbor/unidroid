use crate::{
    BRK_START, Emulator, HOST_MALLOC_START, MMAP_END, MMAP_START, PAGE, RETURN, Result, STACK,
    STACK_SIZE, TRAMPOLINE,
};
use serde::{Deserialize, Serialize};
use unicorn_engine::unicorn_const::Prot;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mapping {
    pub start: u64,
    pub end: u64,
    pub perms: String,
    pub name: String,
}

pub(super) fn locate<'a>(
    address: u64,
    mut mappings: impl Iterator<Item = &'a Mapping>,
) -> Option<&'a Mapping> {
    mappings.find(|m| (m.start..m.end).contains(&address))
}

impl Emulator {
    pub(crate) fn read_mappings(
        &self,
        mappings: impl IntoIterator<Item = Mapping>,
    ) -> impl Iterator<Item = Result<(Mapping, Vec<u8>)>> {
        mappings.into_iter().map(|mapping| {
            let bytes = self.read_bytes(mapping.start, (mapping.end - mapping.start) as usize)?;
            Ok((mapping, bytes))
        })
    }

    pub(crate) fn memory_location<'a>(
        &self,
        address: u64,
        mappings: impl Iterator<Item = &'a Mapping>,
        symbols: bool,
    ) -> String {
        if let Some((_, module)) = self.module_table().at(address) {
            if symbols {
                module.symbolize(address)
            } else {
                module.info.location(address)
            }
        } else {
            super::modules::mapping_location(address, locate(address, mappings))
        }
    }
    pub fn maps(&self) -> Result<Vec<Mapping>> {
        let mut regions = self.cpu()?.mem_regions()?;
        regions.sort_unstable_by_key(|region| region.begin);
        let modules = self.module_table();
        Ok(regions
            .into_iter()
            .map(|region| {
                let start = region.begin;
                let end = region.end + 1;
                let name = modules
                    .at(start)
                    .filter(|(_, m)| end <= m.info.base + m.info.size)
                    .map(|(_, m)| m.info.name.as_str())
                    .unwrap_or_else(|| {
                        [
                            (TRAMPOLINE, RETURN + PAGE, "[trampoline]"),
                            (HOST_MALLOC_START, MMAP_START, "[host malloc]"),
                            (MMAP_START, MMAP_END, "[mmap]"),
                            (BRK_START, STACK, "[heap]"),
                            (STACK, STACK + STACK_SIZE, "[stack]"),
                            (0xffff0000, 0xffff0000 + PAGE, "[vectors]"),
                        ]
                        .into_iter()
                        .find(|&(lo, hi, _)| start >= lo && end <= hi)
                        .map_or("", |(_, _, name)| name)
                    });
                let perms = [(Prot::READ, 'r'), (Prot::WRITE, 'w'), (Prot::EXEC, 'x')]
                    .into_iter()
                    .map(|(bit, letter)| {
                        if region.perms & bit.0 != 0 {
                            letter
                        } else {
                            '-'
                        }
                    })
                    .collect();
                Mapping {
                    start,
                    end,
                    perms,
                    name: name.into(),
                }
            })
            .collect())
    }
}
