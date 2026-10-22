use crate::{Emulator, Module, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub name: String,
    pub path: PathBuf,
    pub base: u64,
    pub size: u64,
}

pub(crate) fn module_location<'a>(
    address: u64,
    mut modules: impl Iterator<Item = (&'a str, u64, u64)>,
) -> Option<String> {
    modules
        .find(|&(_, base, size)| address >= base && address < base + size)
        .map(|(name, base, _)| format!("{name}+0x{:x}", address - base))
}

impl Emulator {
    pub fn modules(&self) -> Vec<ModuleInfo> {
        self.modules
            .iter()
            .map(|m| ModuleInfo {
                name: m.name.clone(),
                path: m.path.clone(),
                base: m.base,
                size: m.image.size,
            })
            .collect()
    }

    pub fn resolve(&self, text: &str) -> Result<u64> {
        fn number(text: &str) -> Result<u64> {
            text.strip_prefix("0x")
                .and_then(|digits| u64::from_str_radix(digits, 16).ok())
                .ok_or_else(|| format!("bad number {text}").into())
        }
        if let Some((name, offset)) = text.split_once('+') {
            let module = self
                .modules
                .iter()
                .find(|m| m.name == name)
                .ok_or_else(|| format!("unknown module {name}"))?;
            return module
                .base
                .checked_add(number(offset)?)
                .ok_or_else(|| format!("bad number {offset}: address overflow").into());
        }
        if text.starts_with("0x") {
            return number(text);
        }
        (0..self.modules.len())
            .find_map(|i| self.local_symbol(Module(i), text))
            .ok_or_else(|| format!("unknown symbol {text}").into())
    }
}
