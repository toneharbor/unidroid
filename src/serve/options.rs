use crate::{Emulator, EmulatorBuilder, Policy, Result};
use clap::Args;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Emulator settings shared by the CLI and recording header.
#[derive(Clone, Args, Serialize, Deserialize)]
pub struct Options {
    /// Answer matching Java calls from a JSON policy file.
    #[arg(long, value_name = "FILE")]
    #[serde(skip)]
    pub policy: Option<PathBuf>,
    /// Android system library directory.
    #[arg(long, default_value = "rootfs")]
    pub rootfs: PathBuf,
    /// APK containing the app's libraries and metadata.
    #[arg(long, value_name = "FILE")]
    pub apk: Option<PathBuf>,
    /// Guest process name (default: APK package, otherwise unidroid).
    #[arg(long)]
    pub process_name: Option<String>,
    /// Set a system property.
    #[arg(long, value_name = "KEY=VALUE", value_parser = pair)]
    pub prop: Vec<(String, String)>,
    /// Resolve a symbol under another name.
    #[arg(long, value_name = "NAME=TARGET", value_parser = pair)]
    pub alias: Vec<(String, String)>,
    /// Make a host file available at an absolute guest path, with writes kept in memory.
    #[arg(long, value_name = "GUEST=HOST", value_parser = pair)]
    pub file: Vec<(String, String)>,
    /// Declare that CLASS extends SUPERCLASS.
    #[arg(long, value_name = "CLASS=SUPERCLASS", value_parser = pair)]
    pub superclass: Vec<(String, String)>,
    /// Start guest realtime at these Unix seconds; guest clock syscalls and arm64 counter reads advance by one microsecond per read.
    #[arg(long, value_name = "SECONDS", allow_negative_numbers = true)]
    pub fixed_clock: Option<i64>,
    /// Seed the guest random byte stream with SplitMix64.
    #[arg(long, value_name = "N")]
    pub random_seed: Option<u64>,
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
    pub fn load_policy(&self) -> Result<Option<Policy>> {
        self.policy.as_ref().map(Policy::load).transpose()
    }

    pub fn build(&self, builder: EmulatorBuilder) -> Result<Emulator> {
        let mut builder = builder
            .rootfs(&self.rootfs)
            .translation_cache_size(self.translation_cache_mb * 1024 * 1024);
        if let Some(path) = &self.apk {
            builder = builder.apk(path);
        }
        if let Some(name) = &self.process_name {
            builder = builder.process_name(name);
        }
        if let Some(seconds) = self.fixed_clock {
            builder = builder.fixed_clock(seconds);
        }
        if let Some(seed) = self.random_seed {
            builder = builder.random_seed(seed);
        }
        for (key, value) in &self.prop {
            builder = builder.system_property(key, value);
        }
        let mut emu = builder.build()?;
        for (name, target) in &self.alias {
            emu.symbol_alias(name, target);
        }
        for (path, host_path) in &self.file {
            emu.add_host_file(path, host_path)?;
        }
        for (class, superclass) in &self.superclass {
            emu.superclass(class, superclass);
        }
        Ok(emu)
    }
}
