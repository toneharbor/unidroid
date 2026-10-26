use crate::{Emulator, Frame, Image, Loaded, Mapping, Module, RETURN, Result};
use serde::{Deserialize, Serialize};
use std::{
    cell::{Ref, RefCell},
    fmt::Write,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub name: String,
    pub path: PathBuf,
    pub base: u64,
    pub size: u64,
}

impl ModuleInfo {
    pub(super) fn location(&self, address: u64) -> String {
        format!("{}+0x{:x}", self.name, address - self.base)
    }
}

fn containing<'a>(
    address: u64,
    modules: impl Iterator<Item = &'a ModuleInfo>,
) -> Option<(usize, &'a ModuleInfo)> {
    modules
        .enumerate()
        .find(|(_, m)| (m.base..m.base + m.size).contains(&address))
}

pub(crate) type SharedModules = Rc<RefCell<ModuleTable>>;

#[derive(Default)]
pub(crate) struct ModuleTable {
    pub entries: Vec<Entry>,
}

pub(crate) struct Entry {
    pub info: ModuleInfo,
    pub image: Arc<Image>,
    pub app: bool,
}

impl Entry {
    pub fn symbolize(&self, address: u64) -> String {
        let mut at = self.info.location(address);
        if let Some(symbol) = self.image.function(address - self.info.base) {
            let _ = write!(at, " ({symbol})");
        }
        at
    }

    fn frame(&self, address: u64) -> Frame {
        Frame {
            address,
            location: self.info.location(address),
            symbol: self.image.function(address - self.info.base),
        }
    }
}

impl ModuleTable {
    fn rebuild(&mut self, modules: &[Loaded]) {
        self.entries = modules
            .iter()
            .map(|m| Entry {
                info: ModuleInfo {
                    name: m.name.clone(),
                    path: m.path.clone(),
                    base: m.base,
                    size: m.image.size,
                },
                image: m.image.clone(),
                app: m.app,
            })
            .collect();
    }

    pub fn at(&self, address: u64) -> Option<(usize, &Entry)> {
        containing(address, self.entries.iter().map(|entry| &entry.info))
            .map(|(index, _)| (index, &self.entries[index]))
    }

    pub fn location(&self, address: u64) -> String {
        self.at(address).map_or_else(
            || format!("0x{address:x}"),
            |(_, m)| m.info.location(address),
        )
    }

    pub fn symbolize(&self, address: u64) -> String {
        self.at(address)
            .map_or_else(|| format!("0x{address:x}"), |(_, m)| m.symbolize(address))
    }

    pub fn frame(&self, address: u64) -> Frame {
        self.at(address).map_or_else(
            || Frame {
                address,
                location: format!("0x{address:x}"),
                symbol: (address == RETURN).then(|| "host return".into()),
            },
            |(_, m)| m.frame(address),
        )
    }
}

pub(super) fn memory_location(
    address: u64,
    modules: &[ModuleInfo],
    mapping: Option<&Mapping>,
) -> String {
    containing(address, modules.iter()).map_or_else(
        || mapping_location(address, mapping),
        |(_, m)| m.location(address),
    )
}

pub(super) fn mapping_location(address: u64, mapping: Option<&Mapping>) -> String {
    match mapping.filter(|m| !m.name.is_empty()) {
        Some(mapping) => format!("0x{address:x} {}", mapping.name),
        None => format!("0x{address:x}"),
    }
}

impl Emulator {
    pub(crate) fn module_table(&self) -> Ref<'_, ModuleTable> {
        self.module_table.borrow()
    }

    pub(crate) fn rebuild_module_table(&mut self) {
        self.module_table.borrow_mut().rebuild(&self.modules);
    }
    /// Original ELF bytes, before relocation and guest writes, borrowed from the image cache.
    pub fn module_bytes(&self, module: Module) -> Result<&[u8]> {
        self.modules
            .get(module.0)
            .map(|loaded| loaded.image.bytes.as_ref())
            .ok_or_else(|| format!("no module {} is loaded", module.0).into())
    }

    pub fn modules(&self) -> Vec<ModuleInfo> {
        self.module_table()
            .entries
            .iter()
            .map(|m| m.info.clone())
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
        self.global_symbol(text, 0)
            .ok_or_else(|| format!("unknown symbol {text}").into())
    }
}
