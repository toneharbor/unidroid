use crate::{Emulator, Result};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
};
use zip::CompressionMethod::{Deflated, Stored};
#[path = "../../tests/support/apk.rs"]
mod support;
use support::{
    TestApk,
    manifest::{self, package},
};

thread_local! {
    static PARSES: RefCell<HashMap<PathBuf, usize>> = RefCell::default();
}
pub(super) fn parsed(path: &Path) {
    PARSES.with_borrow_mut(|counts| *counts.entry(path.to_owned()).or_default() += 1);
}
fn parses(path: &Path) -> usize {
    PARSES.with_borrow(|counts| counts.get(path).copied().unwrap_or(0))
}

#[test]
fn dependency_loads_and_rewinds_parse_each_archive_once() -> Result<()> {
    for (abi, dir, split_name) in [
        ("arm64-v8a", "lib64", "config.arm64_v8a.apk"),
        ("armeabi-v7a", "lib", "config.armeabi_v7a.apk"),
    ] {
        let xml = manifest::xml(true, &[package()]);
        let jna = std::fs::read(format!("tests/fixtures/{abi}/libjnidispatch.so"))?;
        let libm = std::fs::read(format!("rootfs/{dir}/libm.so"))?;
        let base = TestApk::new(&[
            ("AndroidManifest.xml", &xml, Stored, 1),
            (&format!("lib/{abi}/libjnidispatch.so"), &jna, Deflated, 1),
        ])?;
        let split = TestApk::new(&[(&format!("lib/{abi}/libm.so"), &libm, Stored, 4096)])?;
        let split_path = base.path.with_file_name(split_name);
        std::fs::copy(&split.path, &split_path)?;
        let mut emu = Emulator::builder().apk(&base.path).build()?;
        let base_path = base.path.canonicalize()?;
        let split_path = split_path.canonicalize()?;
        assert_eq!(parses(&base_path), 1);
        assert_eq!(parses(&split_path), 0);
        emu.load_apk_library("libjnidispatch.so")?;
        assert!(emu.modules().len() >= 4);
        assert_eq!(parses(&base_path), 1);
        assert_eq!(parses(&split_path), 1);
        let saved = emu.checkpoint()?;
        emu.apk_mut().unwrap().package = "changed".into();
        emu.load_apk_library_without_constructors("libm.so")?;
        let mut fresh = Emulator::builder().build()?;
        fresh.rewind(&saved)?;
        let name = fresh.new_cstring("libm.so")?;
        let dlopen = fresh.resolve("dlopen")?;
        assert_ne!(fresh.call_function(dlopen, &[name, 2])?, 0);
        assert_eq!(parses(&base_path), 1);
        assert_eq!(parses(&split_path), 1);
    }
    Ok(())
}
