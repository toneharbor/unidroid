#[path = "support/jna.rs"]
#[allow(dead_code)]
mod jna;
#[path = "support/apk.rs"]
mod support;
use support::{
    TestApk,
    manifest::{self, package},
};
use unidroid::{Emulator, Result};
use zip::CompressionMethod::{Deflated, Stored};

#[test]
fn abi_selection_prefers_arm64_and_arm32_falls_back_to_armeabi() -> Result<()> {
    let xml = manifest::xml(true, &[package()]);
    let arm64 = std::fs::read("rootfs/lib64/libdl.so")?;
    let arm32 = std::fs::read("rootfs/lib/libdl.so")?;
    let file = TestApk::new(&[
        ("AndroidManifest.xml", &xml, Stored, 1),
        ("lib/arm64-v8a/libdl.so", &arm64, Stored, 1),
        ("lib/armeabi-v7a/libdl.so", &arm32, Stored, 1),
    ])?;
    let mut emu = Emulator::builder().apk(&file.path).build()?;
    let module = emu.load_apk_library("libdl.so")?;
    assert!(
        emu.modules()[module.0]
            .path
            .to_string_lossy()
            .contains("!/lib/arm64-v8a/")
    );
    for abi in ["armeabi-v7a", "armeabi"] {
        let file = TestApk::new(&[
            ("AndroidManifest.xml", &xml, Stored, 1),
            (&format!("lib/{abi}/libdl.so"), &arm32, Stored, 1),
        ])?;
        let mut emu = Emulator::builder().apk(&file.path).build()?;
        let module = emu.load_apk_library("libdl.so")?;
        assert!(
            emu.modules()[module.0]
                .path
                .to_string_lossy()
                .ends_with(&format!("!/lib/{abi}/libdl.so"))
        );
        assert!(emu.registers()?.iter().any(|(name, _)| *name == "r0"));
    }
    Ok(())
}

#[test]
fn libraries_resolve_base_then_split_then_host_then_rootfs() -> Result<()> {
    for (abi, dir, split_name) in [
        ("arm64-v8a", "lib64", "config.arm64_v8a.apk"),
        ("armeabi-v7a", "lib", "config.armeabi_v7a.apk"),
    ] {
        let xml = manifest::xml(true, &[package()]);
        let libc = std::fs::read(format!("rootfs/{dir}/libc.so"))?;
        let libdl = std::fs::read(format!("rootfs/{dir}/libdl.so"))?;
        let libm = std::fs::read(format!("rootfs/{dir}/libm.so"))?;
        let entry = format!("lib/{abi}/libdl.so");
        for base_has_library in [false, true] {
            let mut entries = vec![("AndroidManifest.xml", xml.as_slice(), Stored, 1)];
            if base_has_library {
                entries.push((&entry, &libdl, Stored, 1));
            }
            let base = TestApk::new(&entries)?;
            let split = TestApk::new(&[
                (&entry, &libdl, Stored, 1),
                (&format!("lib/{abi}/libc.so"), &libc, Deflated, 1),
            ])?;
            let split_path = base.path.with_file_name(split_name);
            std::fs::copy(&split.path, &split_path)?;
            let host = base.path.with_file_name("libm.so");
            std::fs::write(&host, &libm)?;
            // Host libdl would win over rootfs, but both APK locations must win over it.
            std::fs::write(base.path.with_file_name("libdl.so"), &libdl)?;
            let mut emu = Emulator::builder().apk(&base.path).build()?;
            emu.load_library_without_constructors(&host)?;
            let expected = if base_has_library {
                &base.path
            } else {
                &split_path
            };
            let module = emu
                .modules()
                .into_iter()
                .find(|m| m.name == "libdl.so")
                .unwrap();
            assert_eq!(
                module.path.to_string_lossy(),
                format!("{}!/{entry}", expected.canonicalize()?.display())
            );
            assert!(
                emu.modules()
                    .iter()
                    .any(|m| m.name == "libc.so" && m.path.to_string_lossy().contains(split_name))
            );
            // A named APK load uses the same host/rootfs fallback when neither APK holds it.
            assert_eq!(emu.load_apk_library_without_constructors("libm.so")?.0, 0);
        }
        let base = TestApk::manifest(&xml)?;
        let split = TestApk::new(&[(&entry, &libdl, Stored, 1)])?;
        std::fs::copy(&split.path, base.path.with_file_name(split_name))?;
        let mut emu = Emulator::builder().apk(&base.path).build()?;
        let module = emu.load_apk_library("libdl.so")?; // also chooses architecture from a split
        assert!(
            emu.modules()[module.0]
                .path
                .to_string_lossy()
                .contains(split_name)
        );
        emu.load_apk_library("libm.so")?; // rootfs fallback
    }
    Ok(())
}

#[test]
fn apk_load_errors_name_the_library_and_apk() -> Result<()> {
    let mut emu = Emulator::builder().build()?;
    assert_eq!(
        emu.module_bytes(unidroid::Module(7))
            .unwrap_err()
            .to_string(),
        "no module 7 is loaded"
    );
    let error = emu.load_apk_library("missing.so").unwrap_err().to_string();
    assert!(error.contains("missing.so") && error.contains("no APK"));
    let file = TestApk::manifest(&manifest::xml(true, &[package()]))?;
    let mut emu = Emulator::builder().apk(&file.path).build()?;
    let error = emu.load_apk_library("missing.so").unwrap_err().to_string();
    assert!(error.contains("missing.so") && error.contains(file.path.to_str().unwrap()));
    Ok(())
}

#[test]
fn apk_jna_matches_unidbg_for_compressed_aligned_and_unaligned_entries() -> Result<()> {
    for (abi, width) in [("arm64-v8a", 8), ("armeabi-v7a", 4)] {
        let xml = manifest::xml(true, &[package()]);
        let library = std::fs::read(format!("tests/fixtures/{abi}/libjnidispatch.so"))?;
        for (method, alignment) in [(Deflated, 1), (Stored, 4096), (Stored, 1)] {
            let entry = format!("lib/{abi}/libjnidispatch.so");
            let file = TestApk::new(&[
                ("AndroidManifest.xml", &xml, Stored, 1),
                (&entry, &library, method, alignment),
            ])?;
            let mut emu = Emulator::builder().apk(&file.path).build()?;
            let module = emu.load_apk_library("libjnidispatch.so")?;
            let out = jna::check(&mut emu, module)?;
            assert_eq!(emu.module_bytes(module)?, library);
            let base = emu.modules()[module.0].base;
            let elf = goblin::elf::Elf::parse(&library)?;
            let data = elf
                .program_headers
                .iter()
                .find(|p| p.p_type == 1 && p.p_flags & 2 != 0)
                .unwrap();
            emu.write_bytes(base + data.p_vaddr, b"changed")?;
            assert_eq!(emu.module_bytes(module)?, library);
            assert_eq!(out.sdk, "23");
            assert!(out.malloc_nonzero);
            assert_eq!(out.version, "4.0.1");
            assert_eq!(out.checksum, "1a6047467b59e8748f975e03016ce3d9");
            assert_eq!(out.size, width);
            assert_eq!(
                emu.modules()[module.0].path.to_string_lossy(),
                format!("{}!/{entry}", file.path.canonicalize()?.display())
            );
        }
    }
    Ok(())
}
