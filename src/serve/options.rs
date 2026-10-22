use crate::{Emulator, EmulatorBuilder, Result};
use clap::Args;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Emulator settings shared by the CLI and recording header.
#[derive(Args, Serialize, Deserialize)]
pub struct Options {
    /// Android system library directory.
    #[arg(long, default_value = "rootfs")]
    pub rootfs: PathBuf,
    /// Guest process name.
    #[arg(long, default_value = "unidroid")]
    pub process_name: String,
    /// Set a system property.
    #[arg(long, value_name = "KEY=VALUE", value_parser = pair)]
    pub prop: Vec<(String, String)>,
    /// Resolve a symbol under another name.
    #[arg(long, value_name = "NAME=TARGET", value_parser = pair)]
    pub alias: Vec<(String, String)>,
    /// Declare that CLASS extends SUPERCLASS.
    #[arg(long, value_name = "CLASS=SUPERCLASS", value_parser = pair)]
    pub superclass: Vec<(String, String)>,
    /// Translation cache size in MB. Large libraries run faster with a bigger cache.
    #[arg(long, value_name = "MB", default_value = "8", value_parser = clap::value_parser!(u32).range(1..4096))]
    pub translation_cache_mb: u32,
}

fn pair(value: &str) -> std::result::Result<(String, String), String> {
    value
        .split_once('=')
        .map(|(key, value)| (key.into(), value.into()))
        .ok_or_else(|| "expected KEY=VALUE".into())
}

impl Options {
    pub fn build(&self, builder: EmulatorBuilder) -> Result<Emulator> {
        let mut builder = builder
            .rootfs(&self.rootfs)
            .process_name(&self.process_name)
            .translation_cache_size(self.translation_cache_mb * 1024 * 1024);
        for (key, value) in &self.prop {
            builder = builder.system_property(key, value);
        }
        let mut emu = builder.build()?;
        for (name, target) in &self.alias {
            emu.symbol_alias(name, target);
        }
        for (class, superclass) in &self.superclass {
            emu.superclass(class, superclass);
        }
        Ok(emu)
    }
}
