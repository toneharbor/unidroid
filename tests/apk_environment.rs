#[path = "support/apk.rs"]
mod support;
use support::{
    TestApk,
    manifest::{self, package},
};
use unidroid::{Emulator, Module, Result};
use zip::CompressionMethod::Stored;

fn call(emu: &mut Emulator, module: Module, name: &str, args: &[u64]) -> Result<u64> {
    let function = emu.find_symbol(module, name)?;
    emu.call_function(function, args)
}

#[test]
fn guest_dlopen_existing_rootfs_path_precedes_apk_name_lookup() -> Result<()> {
    for (abi, dir) in [("arm64-v8a", "lib64"), ("armeabi-v7a", "lib")] {
        let xml = manifest::xml(true, &[package()]);
        let library = std::fs::read(format!("rootfs/{dir}/libdl.so"))?;
        let file = TestApk::new(&[
            ("AndroidManifest.xml", &xml, Stored, 1),
            (&format!("lib/{abi}/libvendor.so"), &library, Stored, 1),
        ])?;
        let root = file.path.with_file_name("rootfs");
        let vendor = root.join(format!("vendor/{dir}"));
        std::fs::create_dir_all(&vendor)?;
        let exact = vendor.join("libvendor.so");
        std::fs::write(&exact, &library)?;
        for with_apk in [false, true] {
            for request in [
                format!("/vendor/{dir}/libvendor.so"),
                format!("/vendor/{dir}/../{dir}/libvendor.so"),
            ] {
                let mut builder = Emulator::builder().rootfs(&root);
                if with_apk {
                    builder = builder.apk(&file.path);
                }
                let mut emu = builder.build()?;
                let libdl = emu.load_library(format!("rootfs/{dir}/libdl.so"))?;
                let name = emu.new_cstring(&request)?;
                let handle = call(&mut emu, libdl, "dlopen", &[name, 2])?;
                if !request.contains("..") {
                    assert_ne!(handle, 0, "{request}");
                    assert_eq!(emu.modules()[(handle - 1) as usize].path, exact);
                } else if with_apk {
                    assert_ne!(handle, 0);
                    assert!(
                        emu.modules()[(handle - 1) as usize]
                            .path
                            .to_string_lossy()
                            .contains("!/lib/")
                    );
                } else {
                    assert_eq!(handle, 0);
                }
            }
        }
    }
    Ok(())
}

#[test]
fn guest_dlopen_apk_errors_include_the_guest_caller() -> Result<()> {
    for (dir, split) in [
        ("lib64", "config.arm64_v8a.apk"),
        ("lib", "config.armeabi_v7a.apk"),
    ] {
        let file = TestApk::manifest(&manifest::xml(true, &[package()]))?;
        let mut emu = Emulator::builder().apk(&file.path).build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        std::fs::write(file.path.with_file_name(split), b"not a zip")?;
        let name = emu.new_cstring("not-loaded.so")?;
        let error = call(&mut emu, libc, "dlopen", &[name, 2])
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(split)
                && error.contains("not-loaded.so")
                && error.contains("from 0x20000"),
            "{error}"
        );
    }
    Ok(())
}

#[test]
fn guest_dlopen_phdr_and_maps_use_installed_library_paths() -> Result<()> {
    for (abi, dir, guest, width) in [
        ("arm64-v8a", "lib64", "arm64", 8),
        ("armeabi-v7a", "lib", "arm", 4),
    ] {
        let xml = manifest::xml(true, &[package()]);
        let libc = std::fs::read(format!("rootfs/{dir}/libc.so"))?;
        let libm = std::fs::read(format!("rootfs/{dir}/libm.so"))?;
        let file = TestApk::new(&[
            ("AndroidManifest.xml", &xml, Stored, 1),
            (&format!("lib/{abi}/libc.so"), &libc, Stored, 4096),
            (&format!("lib/{abi}/libm.so"), &libm, Stored, 1),
        ])?;
        let mut emu = Emulator::builder().apk(&file.path).build()?;
        let libc = emu.load_apk_library("libc.so")?;
        let callback = 0x20100;
        let words: &[u32] = if width == 8 {
            &[0xf9400400, 0xd65f03c0]
        } else {
            &[0xe5900004, 0xe12fff1e]
        }; // return info->dlpi_name
        emu.write_bytes(
            callback,
            &words
                .iter()
                .flat_map(|n| n.to_le_bytes())
                .collect::<Vec<_>>(),
        )?;
        let path = call(&mut emu, libc, "dl_iterate_phdr", &[callback, 0])?;
        let expected = format!("/data/app/com.example.apk-1/lib/{guest}/libc.so");
        assert_eq!(
            emu.read_bytes(path, expected.len() + 1)?,
            format!("{expected}\0").as_bytes()
        );
        let name = emu.new_cstring("libm.so")?;
        let handle = call(&mut emu, libc, "dlopen", &[name, 2])?;
        assert!(handle > 0);
        assert!(
            emu.modules()[(handle - 1) as usize]
                .path
                .to_string_lossy()
                .ends_with(&format!("!/lib/{abi}/libm.so"))
        );
        let buffer = emu.malloc(65536)?;
        let maps = emu.new_cstring("/proc/self/maps")?;
        let fd = call(&mut emu, libc, "open", &[maps, 0])?;
        let count = call(&mut emu, libc, "read", &[fd, buffer, 65536])?;
        let text = String::from_utf8(emu.read_bytes(buffer, count as usize)?)?;
        assert!(text.contains(&format!("b3:19 0          {expected}\n")));
        assert!(text.contains(&format!(
            "/data/app/com.example.apk-1/lib/{guest}/libm.so\n"
        )));
        assert!(text.contains(&format!("/system/{dir}/libdl.so\n")));
    }
    Ok(())
}

#[test]
fn installed_apk_files_directories_and_process_name_are_visible_to_guest() -> Result<()> {
    let xml = manifest::xml(true, &[package()]);
    let file = TestApk::manifest(&xml)?;
    let bytes = std::fs::read(&file.path)?;
    for dir in ["lib64", "lib"] {
        for explicit in [None, Some("custom.process")] {
            let mut builder = Emulator::builder().apk(&file.path);
            if let Some(name) = explicit {
                builder = builder.process_name(name);
            }
            let mut emu = builder.build()?;
            let name = explicit.unwrap_or("com.example.apk");
            assert_eq!(
                emu.file("/proc/self/cmdline")?,
                format!("{name}\0").as_bytes()
            );
            let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
            let buffer = emu.malloc(bytes.len())?;
            let path = emu.new_cstring("/data/app/com.example.apk-1/base.apk")?;
            let fd = call(&mut emu, libc, "open", &[path, 0])?;
            assert_eq!(
                call(&mut emu, libc, "read", &[fd, buffer, bytes.len() as u64])?,
                bytes.len() as u64
            );
            assert_eq!(emu.read_bytes(buffer, bytes.len())?, bytes);
            assert_eq!(call(&mut emu, libc, "close", &[fd])?, 0);
            for directory in [
                "/data/data/com.example.apk",
                "/data/data/com.example.apk/files",
                "/data/data/com.example.apk/cache",
            ] {
                let path = emu.new_cstring(directory)?;
                let fd = call(&mut emu, libc, "open", &[path, 0x10000])?; // O_DIRECTORY
                assert!((3..100).contains(&fd), "{directory}: {fd}");
                assert_eq!(call(&mut emu, libc, "close", &[fd])?, 0);
            }
        }
    }
    assert_eq!(
        Emulator::builder().build()?.file("/proc/self/cmdline")?,
        b"unidroid\0"
    );
    Ok(())
}

#[test]
fn apk_metadata_and_lookup_rewind_into_original_and_fresh_emulators() -> Result<()> {
    for (abi, dir) in [("arm64-v8a", "lib64"), ("armeabi-v7a", "lib")] {
        let xml = manifest::xml(true, &[package()]);
        let library = std::fs::read(format!("rootfs/{dir}/libm.so"))?;
        let entry = format!("lib/{abi}/libm.so");
        let file = TestApk::new(&[
            ("AndroidManifest.xml", &xml, Stored, 1),
            (&entry, &library, Stored, 4096),
        ])?;
        let mut emu = Emulator::builder().apk(&file.path).build()?;
        emu.load_library(format!("rootfs/{dir}/libdl.so"))?;
        let apk = emu.apk_mut().unwrap();
        apk.version_name = Some("override".into());
        apk.version_code = 123;
        apk.signatures = vec![vec![0x30, 1, 42]];
        apk.signers = vec![vec![0x30, 1, 43]];
        apk.signing_history = vec![vec![0x30, 1, 42], vec![0x30, 1, 43]];
        let saved = emu.checkpoint()?;
        let apk = emu.apk_mut().unwrap();
        apk.package = "other.package".into();
        apk.path = "missing.apk".into();
        apk.signatures.clear();
        apk.signers.clear();
        apk.signing_history.clear();
        assert_eq!(
            emu.file("/data/app/com.example.apk-1/base.apk")?,
            std::fs::read(&file.path)?
        );
        assert_eq!(emu.file("/proc/self/cmdline")?, b"com.example.apk\0");
        emu.rewind(&saved)?;
        let mut fresh = Emulator::builder().build()?;
        assert!(fresh.apk().is_none() && fresh.apk_mut().is_none());
        fresh.rewind(&saved)?;
        for restored in [&mut emu, &mut fresh] {
            let apk = restored.apk().unwrap();
            assert_eq!(apk.package, "com.example.apk");
            assert_eq!(apk.version_name.as_deref(), Some("override"));
            assert_eq!(apk.version_code, 123);
            assert_eq!(apk.signatures, [vec![0x30, 1, 42]]);
            assert_eq!(apk.signers, [vec![0x30, 1, 43]]);
            assert_eq!(apk.signing_history, [vec![0x30, 1, 42], vec![0x30, 1, 43]]);
            let module = restored.load_apk_library("libm.so")?;
            assert_eq!(
                restored.modules()[module.0].path.to_string_lossy(),
                format!("{}!/{entry}", file.path.canonicalize()?.display())
            );
            assert_eq!(
                restored.file("/data/app/com.example.apk-1/base.apk")?,
                std::fs::read(&file.path)?
            );
        }
    }
    Ok(())
}
