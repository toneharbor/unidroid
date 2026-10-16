use minidbg::{Emulator, Handle, Result};
#[path = "support/elf.rs"]
mod elf;

#[test]
fn loader_service_calls_do_not_grow_the_api_heap() -> Result<()> {
    let library = elf::TestLibrary::new(false);
    let mut emu = Emulator::builder().build()?;
    let module = emu.load_library(&library.0)?;
    let open = emu.find_symbol(module, "dlopen")?;
    let sym = emu.find_symbol(module, "dlsym")?;
    let error = emu.find_symbol(module, "dlerror")?;
    let addr = emu.find_symbol(module, "dladdr")?;
    let iterate = emu.find_symbol(module, "dl_iterate_phdr")?;
    let callback = emu.find_symbol(module, "answer")?;
    let path = emu.new_cstring(&library.0.to_string_lossy())?;
    let handle = emu.call_function(open, &[path, 0])?;
    let missing = emu.new_cstring("missing_every_time")?;
    let info = emu.malloc(32)?;
    let address = emu.find_symbol(module, "answer")?;

    for _ in 0..10 {
        assert_eq!(emu.call_function(sym, &[handle, missing])?, 0);
        assert_ne!(emu.call_function(error, &[])?, 0);
        assert_eq!(emu.call_function(addr, &[address, info])?, 1);
        assert_eq!(emu.call_function(iterate, &[callback, 0])?, 42);
    }
    let high_water = emu.heap_high_water_mark();
    for _ in 0..10_000 {
        assert_eq!(emu.call_function(sym, &[handle, missing])?, 0);
        assert_ne!(emu.call_function(error, &[])?, 0);
        assert_eq!(emu.call_function(addr, &[address, info])?, 1);
        assert_eq!(emu.call_function(iterate, &[callback, 0])?, 42);
    }
    assert_eq!(emu.heap_high_water_mark(), high_water);
    Ok(())
}

#[test]
fn api_heap_reuses_and_coalesces_freed_blocks() -> Result<()> {
    let mut emu = Emulator::builder().build()?;
    let first = emu.malloc(32)?;
    let second = emu.malloc(48)?;
    emu.free(first);
    assert_eq!(emu.malloc(16)?, first);
    emu.free(second);
    emu.free(first);
    assert_eq!(emu.malloc(80)?, first);
    Ok(())
}

#[test]
fn weak_undefined_imports_resolve_to_zero() -> Result<()> {
    let library = elf::TestLibrary::new(false);
    let mut emu = Emulator::builder().build()?;
    let module = emu.load_library(&library.0)?;
    assert_eq!(emu.read_bytes(0x10000000 + 0x1620, 8)?, [0; 8]);
    assert_ne!(emu.find_symbol(module, "answer")?, 0);
    Ok(())
}

#[test]
fn api_object_arrays_are_stable_handles() -> Result<()> {
    let mut emu = Emulator::builder().build()?;
    let a = emu.new_string("a");
    let b = emu.new_string("b");
    let array = emu.new_object_array("java/lang/String", &[a, Handle(0), b]);
    let actual = emu.object_array(array);
    assert_eq!(emu.string(actual[0]), "a");
    assert_eq!(actual[1], Handle(0));
    assert_eq!(emu.string(actual[2]), "b");
    emu.release(a);
    emu.release(b);
    let retained = emu.object_array(array);
    assert_eq!(emu.string(retained[0]), "a");
    assert_eq!(emu.string(retained[2]), "b");
    Ok(())
}
