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

impl Emulator {
    pub fn maps(&self) -> Result<Vec<Mapping>> {
        let mut regions = self.cpu()?.mem_regions()?;
        regions.sort_unstable_by_key(|region| region.begin);
        Ok(regions
            .into_iter()
            .map(|region| {
                let start = region.begin;
                let end = region.end + 1;
                let name = self
                    .modules
                    .iter()
                    .find(|m| start >= m.base && end <= m.base + m.image.size)
                    .map(|m| m.name.as_str())
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
