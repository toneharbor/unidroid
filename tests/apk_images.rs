#[path = "support/elf.rs"]
mod elf;
#[path = "support/apk.rs"]
mod support;
use std::{
    io::Write,
    process::{Command, Stdio},
};
use support::{TestApk, manifest};
use unidroid::{Emulator, Result};
use zip::CompressionMethod::Stored;

#[test]
fn unusable_images_error_with_source_and_are_not_cached() -> Result<()> {
    for arm64 in [true, false] {
        let abi = if arm64 { "arm64-v8a" } else { "armeabi-v7a" };
        let width = if arm64 { 8 } else { 4 };
        for defect in [
            "no dynamic",
            "no load",
            "packed outside",
            "packed length",
            "packed file range",
            "packed count",
            "segment file range",
            "segment file size",
            "segment file overflow",
            "segment memory overflow",
            "segment page overflow",
        ] {
            if !arm64 && defect.ends_with("overflow") {
                continue; // ELF32 fields cannot overflow the loader's u64 arithmetic.
            }
            let library = elf::TestLibrary::new(arm64, false);
            let valid = std::fs::read(&library.0)?;
            let mut bytes = valid.clone();
            let (phoff, phsize) = if arm64 { (64, 56) } else { (52, 32) };
            let reason = match defect {
                "segment file range" | "packed file range" => {
                    Some("PT_LOAD file range exceeds the file")
                }
                "segment file size" => Some("PT_LOAD file size exceeds memory size"),
                "segment file overflow" => Some("PT_LOAD file range overflows"),
                "segment memory overflow" => Some("PT_LOAD virtual range overflows"),
                "segment page overflow" => Some("PT_LOAD page-aligned virtual range overflows"),
                _ => None,
            };
            match defect {
                name if name.starts_with("segment ") => {
                    // Add a separate PT_LOAD so malformed ranges do not affect ELF parsing.
                    let (offset, address, filesz, memsz) = match defect {
                        "segment file range" => (0x2000, 0x4000, 0x1000, 0x1000),
                        "segment file size" => (0x1000, 0x4000, 0x1000, 0x800),
                        "segment file overflow" => (u64::MAX - 0xfff, 0x4000, 0x1000, 0x1000),
                        "segment memory overflow" => (0x1000, u64::MAX - 0xfff, 0x1000, 0x1000),
                        _ => (0x1000, 0x4000, 0x1000, u64::MAX - 0x4000),
                    };
                    let count = if arm64 { 56 } else { 44 };
                    bytes[count..count + 2].copy_from_slice(&4u16.to_le_bytes());
                    let p = phoff + 3 * phsize;
                    bytes[p..p + 4].copy_from_slice(&1u32.to_le_bytes());
                    let flags = p + if arm64 { 4 } else { 24 };
                    bytes[flags..flags + 4].copy_from_slice(&6u32.to_le_bytes());
                    for (field, value) in if arm64 {
                        [
                            (8, offset),
                            (16, address),
                            (32, filesz),
                            (40, memsz),
                            (48, 4096),
                        ]
                    } else {
                        [
                            (4, offset),
                            (8, address),
                            (16, filesz),
                            (20, memsz),
                            (28, 4096),
                        ]
                    } {
                        bytes[p + field..p + field + width]
                            .copy_from_slice(&value.to_le_bytes()[..width]);
                    }
                }
                "no dynamic" => bytes[phoff + 2 * phsize..phoff + 2 * phsize + 4].fill(0),
                "no load" => {
                    bytes[0x1000..0x1100].fill(0);
                    bytes[phoff..phoff + 4].fill(0);
                    bytes[phoff + phsize..phoff + phsize + 4].fill(0);
                }
                _ => {
                    let tag: u64 = if arm64 { 0x60000011 } else { 0x6000000f };
                    let (address, size) = match defect {
                        "packed outside" => (0x8000u64, 8u64),
                        "packed length" => (0x1ff8, u64::MAX),
                        "packed count" => (0x1800, 15),
                        _ => (0x1ff8, 16),
                    };
                    if defect == "packed file range" {
                        let offset = phoff + phsize + if arm64 { 32 } else { 16 };
                        bytes[offset..offset + width]
                            .copy_from_slice(&0x2000u64.to_le_bytes()[..width]);
                    }
                    if defect == "packed count" {
                        bytes[0x1800..0x1804].copy_from_slice(b"APS2");
                        bytes[0x1804..0x180d].fill(0xff); // i64::MAX in SLEB128, then offset zero
                    }
                    for (index, value) in [(20, tag), (21, address), (22, tag + 1), (23, size)] {
                        let offset = 0x1000 + index * width;
                        bytes[offset..offset + width]
                            .copy_from_slice(&value.to_le_bytes()[..width]);
                    }
                }
            }
            // These images parse as ELF; failure belongs to image construction.
            goblin::elf::Elf::parse(&bytes)?;
            std::fs::write(&library.0, &bytes)?;
            let xml = manifest::xml(true, &[manifest::package()]);
            let entry = format!("lib/{abi}/libbroken.so");
            let apk = TestApk::new(&[
                ("AndroidManifest.xml", &xml, Stored, 1),
                (&entry, &bytes, Stored, 1),
            ])?;
            for _ in 0..2 {
                let mut emu = Emulator::builder().build()?;
                let error = emu.load_library(&library.0).unwrap_err().to_string();
                assert!(
                    error.contains(library.0.to_str().unwrap()),
                    "{defect}: {error}"
                );
                if let Some(reason) = reason {
                    assert_eq!(error, format!("ELF {}: {reason}", library.0.display()));
                }
                let mut emu = Emulator::builder().apk(&apk.path).build()?;
                let error = emu
                    .load_apk_library("libbroken.so")
                    .unwrap_err()
                    .to_string();
                assert!(
                    error.contains(apk.path.to_str().unwrap()) && error.contains(&entry),
                    "{defect}: {error}"
                );
                if let Some(reason) = reason {
                    assert_eq!(
                        error,
                        format!("ELF {}!/{entry}: {reason}", apk.path.display())
                    );
                }
            }
            // A failed image must leave both the cache and this key usable.
            std::fs::write(&library.0, &valid)?;
            let mut emu = Emulator::builder().build()?;
            emu.load_library(&library.0)?;
            apk.write(&[
                ("AndroidManifest.xml", &xml, Stored, 1),
                (&entry, &valid, Stored, 1),
            ])?;
            let mut emu = Emulator::builder().apk(&apk.path).build()?;
            emu.load_apk_library("libbroken.so")?;
        }
    }
    Ok(())
}

#[test]
fn malformed_stored_entries_fail_without_faulting_the_process() -> Result<()> {
    for arm64 in [true, false] {
        let library = elf::TestLibrary::new(arm64, false);
        let bytes = std::fs::read(&library.0)?;
        let abi = if arm64 { "arm64-v8a" } else { "armeabi-v7a" };
        let entry = format!("lib/{abi}/libbroken.so");
        let xml = manifest::xml(true, &[manifest::package()]);
        for (compressed, size) in [
            (bytes.len() as u32 - 1, bytes.len() as u32),
            (0x1000000, 0x1000000),
        ] {
            let apk = TestApk::new(&[
                ("AndroidManifest.xml", &xml, Stored, 1),
                (&entry, &bytes, Stored, 4096),
            ])?;
            let mut archive = std::fs::read(&apk.path)?;
            let central = archive
                .windows(4)
                .rposition(|bytes| bytes == b"PK\x01\x02")
                .unwrap();
            archive[central + 20..central + 24].copy_from_slice(&compressed.to_le_bytes());
            archive[central + 24..central + 28].copy_from_slice(&size.to_le_bytes());
            std::fs::write(&apk.path, archive)?;
            // Keep a malformed mapping fault confined to the child during the regression test.
            let mut child = Command::new(env!("CARGO_BIN_EXE_unidroid"))
                .args(["serve", "--apk"])
                .arg(&apk.path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"{\"cmd\":\"load_apk_library\",\"name\":\"libbroken.so\"}\n")?;
            let output = child.wait_with_output()?;
            assert!(
                output.status.success(),
                "status: {}; stderr: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
            let response: serde_json::Value = serde_json::from_slice(&output.stdout)?;
            let error = response["error"]
                .as_str()
                .expect("malformed stored entry must fail");
            assert!(
                error.contains(apk.path.to_str().unwrap()) && error.contains(&entry),
                "{error}"
            );
        }
    }
    Ok(())
}
