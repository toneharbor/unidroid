use minidbg::{Emulator, Result};
#[path = "support/elf.rs"]
mod elf;

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
    let mut emu = Emulator::builder().build()?;
    let module = emu.load_library("tests/fixtures/arm64-v8a/libjnidispatch.so")?;
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
    emu.call_function(error, &[])?;
    let missing = emu.new_cstring("missing_symbol")?;
    assert_eq!(emu.call_function(lookup, &[handle, missing])?, 0);
    assert_ne!(emu.call_function(error, &[])?, 0);
    assert_eq!(emu.call_function(error, &[])?, 0);
    assert_eq!(emu.call_function(close, &[handle])?, 0);
    assert_eq!(emu.call_function(close, &[handle])? as i64, -1);
    assert_eq!(emu.call_function(close, &[999999])? as i64, -1);
    Ok(())
}

#[test]
fn android_packed_library_initializes() -> Result<()> {
    let mut emu = Emulator::builder().build()?;
    let module = emu.load_library("rootfs/lib64/libc++.so")?;
    assert_ne!(emu.find_symbol(module, "malloc")?, 0);
    Ok(())
}

#[test]
fn bionic_initializes_and_dependency_symbols_are_callable() -> Result<()> {
    let mut emu = Emulator::builder()
        .system_property("ro.product.model", "Pixel")
        .build()?;
    let module = emu.load_library("tests/fixtures/arm64-v8a/libjnidispatch.so")?;
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
    Ok(())
}
