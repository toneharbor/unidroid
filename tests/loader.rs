use minidbg::{Emulator, Result};
#[path = "support/elf.rs"]
mod elf;

#[test]
fn cpu_calls_require_a_library_but_host_objects_do_not() -> Result<()> {
    let mut emu = Emulator::builder().build()?;
    let text = emu.new_string("before loading");
    assert_eq!(emu.string(text)?, "before loading");
    emu.release(text);
    let expected = "no library loaded yet: the first load_library picks arm64 or arm32";
    assert_eq!(emu.malloc(16).unwrap_err().to_string(), expected);
    assert_eq!(emu.new_cstring("x").unwrap_err().to_string(), expected);
    assert_eq!(emu.read_bytes(0, 1).unwrap_err().to_string(), expected);
    assert_eq!(emu.write_bytes(0, &[0]).unwrap_err().to_string(), expected);
    assert_eq!(emu.call_function(0, &[]).unwrap_err().to_string(), expected);
    assert_eq!(
        emu.call_static_jni_method(&mut (), "example/Native", "value()I", &[])
            .unwrap_err()
            .to_string(),
        expected
    );
    assert_eq!(
        emu.call_jni_onload(minidbg::Module(0), &mut ())
            .unwrap_err()
            .to_string(),
        "no module 0 is loaded"
    );
    Ok(())
}

#[test]
fn loading_the_other_architecture_reports_the_filename() -> Result<()> {
    for (first, second, expected) in [
        (
            "arm64-v8a",
            "armeabi-v7a",
            "cannot load libjnidispatch.so: it is arm32 but this emulator runs arm64",
        ),
        (
            "armeabi-v7a",
            "arm64-v8a",
            "cannot load libjnidispatch.so: it is arm64 but this emulator runs arm32",
        ),
    ] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(format!("tests/fixtures/{first}/libjnidispatch.so"))?;
        assert_eq!(
            emu.load_library(format!("tests/fixtures/{second}/libjnidispatch.so"))
                .unwrap_err()
                .to_string(),
            expected
        );
    }
    Ok(())
}

#[test]
fn jni_onload_without_an_export_returns_not_found() -> Result<()> {
    let library = elf::TestLibrary::new(false);
    let mut emu = Emulator::builder().build()?;
    let module = emu.load_library(&library.0)?;
    let error = emu.call_jni_onload(module, &mut ()).unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::NotFound
    );
    Ok(())
}

#[test]
fn relocation_constructors_bss_and_shared_code_isolation() -> Result<()> {
    for relocate_code in [false, true] {
        let library = elf::TestLibrary::new(relocate_code);
        let mut first = Emulator::builder().build()?;
        let mut second = Emulator::builder().build()?;
        let module = first.load_library(&library.0)?;
        let other = second.load_library(&library.0)?;
        let answer = first.find_symbol(module, "answer")?;
        let data = first.find_symbol(module, "data")?;
        let protect = first.find_symbol(module, "protect")?;
        assert_eq!(first.call_function(answer, &[])?, 42);
        let values = first.read_bytes(data, 32)?;
        let words: Vec<_> = values
            .as_chunks::<8>()
            .0
            .iter()
            .map(|b| u64::from_le_bytes(*b))
            .collect();
        assert_eq!(words, [42, answer + 4, answer, answer]);
        assert_eq!(first.read_bytes(data + 0xa00, 4096)?, vec![0; 4096]);
        if relocate_code {
            assert_eq!(first.read_bytes(answer + 0x100, 8)?, data.to_le_bytes());
        }
        assert_eq!(first.call_function(protect, &[answer & !4095, 4096, 7])?, 0);
        first.write_bytes(answer, &0xd28000e0u32.to_le_bytes())?; // mov x0,#7
        assert_eq!(first.call_function(answer, &[])?, 7);
        assert_eq!(
            second.call_function(second.find_symbol(other, "answer")?, &[])?,
            42
        );
        assert_eq!(first.load_library(&library.0)?, module);
        first.write_bytes(answer, &0xd4200000u32.to_le_bytes())?; // brk #0
        let error = first.call_function(answer, &[]).unwrap_err().to_string();
        let filename = library.0.file_name().unwrap().to_string_lossy();
        assert_eq!(
            error,
            format!(
                "emulation stopped: Unhandled CPU exception (UC_ERR_EXCEPTION) at {filename}+0x200 (pc=0x{answer:x}, lr=0x20000)"
            )
        );
    }
    Ok(())
}

#[test]
fn libdl_resolves_dependencies_and_clears_errors() -> Result<()> {
    for (directory, p, default, next) in [
        ("arm64-v8a", 8u64, 0, u64::MAX),
        ("armeabi-v7a", 4, 0xffff_ffff, 0xffff_fffe),
    ] {
        let mut emu = Emulator::builder().build()?;
        let module = emu.load_library(format!("tests/fixtures/{directory}/libjnidispatch.so"))?;
        let open = emu.find_symbol(module, "dlopen")?;
        let lookup = emu.find_symbol(module, "dlsym")?;
        let error = emu.find_symbol(module, "dlerror")?;
        let close = emu.find_symbol(module, "dlclose")?;
        let name = emu.new_cstring("libjnidispatch.so")?;
        let handle = emu.call_function(open, &[name, 0])?;
        let symbol = emu.new_cstring("__system_property_get")?;
        assert_eq!(
            emu.call_function(lookup, &[handle, symbol])?,
            emu.find_symbol(module, "__system_property_get")?
        );
        let property = emu.find_symbol(module, "__system_property_get")?;
        assert_eq!(emu.call_function(lookup, &[default, symbol])?, property);
        assert_eq!(emu.call_function(lookup, &[next, symbol])?, property);
        if p == 4 {
            assert_eq!(emu.call_function(lookup, &[0, symbol])?, 0);
        }
        let addr = emu.find_symbol(module, "dladdr")?;
        let info = emu.malloc(40)?;
        emu.write_bytes(info, &[0xa5; 40])?;
        let native = emu.find_symbol(module, "JNI_OnLoad")?;
        assert_eq!(emu.call_function(addr, &[native, info])?, 1);
        let pointer = |emu: &Emulator, at| -> Result<u64> {
            let mut bytes = [0; 8];
            bytes[..p as usize].copy_from_slice(&emu.read_bytes(at, p as usize)?);
            Ok(u64::from_le_bytes(bytes))
        };
        let base = pointer(&emu, info + p)?;
        assert_eq!(pointer(&emu, info + 3 * p)?, native);
        assert_eq!(
            emu.read_bytes(info + 4 * p, (40 - 4 * p) as usize)?,
            vec![0xa5; (40 - 4 * p) as usize]
        );
        let callback = emu.malloc(16)?;
        let code = if p == 4 {
            [0xe5820000u32, 0xe1a00001, 0xe12fff1e]
        } else {
            [0xf9000040, 0xaa0103e0, 0xd65f03c0]
        }; // save info at data, return size
        emu.write_bytes(
            callback,
            &code
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        let protect = emu.find_symbol(module, "mprotect")?;
        assert_eq!(emu.call_function(protect, &[callback & !4095, 4096, 7])?, 0);
        let iterate = emu.find_symbol(module, "dl_iterate_phdr")?;
        let saved = emu.malloc(p as usize)?;
        assert_eq!(emu.call_function(iterate, &[callback, saved])?, 4 * p);
        let phdr_info = pointer(&emu, saved)?;
        assert_eq!(pointer(&emu, phdr_info)?, base);
        assert_eq!(pointer(&emu, phdr_info + p)?, pointer(&emu, info)?);
        let phoff = pointer(&emu, base + if p == 4 { 28 } else { 32 })?;
        assert_eq!(pointer(&emu, phdr_info + 2 * p)?, base + phoff);
        assert_eq!(
            emu.read_bytes(phdr_info + 3 * p, 2)?,
            emu.read_bytes(base + if p == 4 { 44 } else { 56 }, 2)?
        );
        emu.call_function(error, &[])?;
        let missing = emu.new_cstring("missing_symbol")?;
        assert_eq!(emu.call_function(lookup, &[handle, missing])?, 0);
        assert_ne!(emu.call_function(error, &[])?, 0);
        assert_eq!(emu.call_function(error, &[])?, 0);
        assert_eq!(emu.call_function(close, &[handle])?, 0);
        let invalid_result = if p == 4 { u32::MAX as u64 } else { u64::MAX };
        assert_eq!(emu.call_function(close, &[handle])?, invalid_result);
        assert_eq!(emu.call_function(close, &[999999])?, invalid_result);
        let cpp = emu.new_cstring("libc++.so")?;
        let cpp_handle = emu.call_function(open, &[cpp, 0])?;
        assert_ne!(cpp_handle, 0);
        let exported = emu.new_cstring("__cxa_throw")?;
        assert_ne!(emu.call_function(lookup, &[cpp_handle, exported])?, 0);
    }
    Ok(())
}

#[test]
fn dependencies_and_dlopen_find_siblings_of_public_loads() -> Result<()> {
    let library = elf::TestLibrary::new(false);
    let directory = library.0.with_extension("siblings");
    std::fs::create_dir(&directory)?;
    let main = directory.join("libtmessages.29.so");
    std::fs::copy("tests/fixtures/arm64-v8a/libtmessages.29.so", &main)?;
    std::fs::copy(&library.0, directory.join("libjnigraphics.so"))?;
    std::fs::copy(&library.0, directory.join("libextra.so"))?;
    let mut emu = Emulator::builder().build()?;
    let module = emu.load_library(&main)?;
    let answer = emu.find_symbol(module, "answer")?;
    assert_eq!(emu.call_function(answer, &[])?, 42);
    let open = emu.find_symbol(module, "dlopen")?;
    let lookup = emu.find_symbol(module, "dlsym")?;
    let name = emu.new_cstring("libextra.so")?;
    let handle = emu.call_function(open, &[name, 0])?;
    assert_ne!(handle, 0);
    let symbol = emu.new_cstring("answer")?;
    let extra_answer = emu.call_function(lookup, &[handle, symbol])?;
    assert_ne!(extra_answer, 0);
    assert_ne!(extra_answer, answer);
    assert_eq!(emu.call_function(extra_answer, &[])?, 42);
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn android_packed_library_initializes() -> Result<()> {
    for system in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        let module = emu.load_library(format!("rootfs/{system}/libc++.so"))?;
        assert_ne!(emu.find_symbol(module, "malloc")?, 0);
    }
    Ok(())
}

#[test]
fn bionic_initializes_and_dependency_symbols_are_callable() -> Result<()> {
    for directory in ["arm64-v8a", "armeabi-v7a"] {
        let mut emu = Emulator::builder()
            .system_property("ro.product.model", "Pixel")
            .build()?;
        let module = emu.load_library(format!("tests/fixtures/{directory}/libjnidispatch.so"))?;
        let property = emu.find_symbol(module, "__system_property_get")?;
        let key = emu.new_cstring("ro.build.version.sdk")?;
        let output = emu.malloc(92)?;
        assert_eq!(emu.call_function(property, &[key, output])?, 2);
        assert_eq!(emu.read_bytes(output, 3)?, b"23\0");
        let key = emu.new_cstring("missing")?;
        assert_eq!(emu.call_function(property, &[key, output])?, 0);
        assert_eq!(emu.read_bytes(output, 1)?, [0]);
        let malloc = emu.find_symbol(module, "malloc")?;
        assert_ne!(emu.call_function(malloc, &[32])?, 0);
    }
    Ok(())
}
