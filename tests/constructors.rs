use unidroid::{Emulator, JniHandler, Module, Result, Resume, Stop};
#[path = "support/elf.rs"]
mod elf;

#[test]
fn guest_dlopen_constructors_use_the_callers_handler() -> Result<()> {
    let main = elf::TestLibrary::new(false);
    let library = elf::TestLibrary::new(false);
    let mut bytes = std::fs::read(&library.0)?;
    // AArch64 constructor: tail-call answer through its relocated GOT slot.
    let code = [
        0x10000010u32 | ((0x1610 - 0x220) >> 2) << 5,
        0xf9400210,
        0xd61f0200,
    ];
    for (i, word) in code.iter().enumerate() {
        bytes[0x220 + i * 4..0x224 + i * 4].copy_from_slice(&word.to_le_bytes());
    }
    std::fs::write(&library.0, bytes)?;
    let mut emu = Emulator::builder().build()?;
    emu.hook_import("answer")?;
    let module = emu.load_library(&main.0)?;
    let open = emu.find_symbol(module, "dlopen")?;
    let name = emu.new_cstring(library.0.file_name().unwrap().to_str().unwrap())?;
    struct Handler(usize);
    impl JniHandler for Handler {
        fn import(&mut self, emu: &mut Emulator, call: &unidroid::ImportCall<'_>) -> Option<u64> {
            assert_eq!(call.name, "answer");
            assert!(emu.location(call.return_address).starts_with("0x"));
            self.0 += 1;
            Some(73)
        }
    }
    let mut handler = Handler(0);
    assert_eq!(emu.call_function_with(&mut handler, open, &[name, 0])?, 2);
    assert_eq!(handler.0, 1);
    assert_eq!(emu.call_function_with(&mut handler, open, &[name, 0])?, 2);
    assert_eq!(handler.0, 1);
    Ok(())
}

#[test]
fn dependency_cycles_run_init_then_init_array_depth_first_once() -> Result<()> {
    let main = elf::TestLibrary::new(false);
    let dependency = elf::TestLibrary::new(false);
    for (library, needed) in [(&main, &dependency), (&dependency, &main)] {
        let mut bytes = std::fs::read(&library.0)?;
        let name = needed.0.file_name().unwrap().to_str().unwrap().as_bytes();
        bytes[0x1a00..0x1a00 + name.len()].copy_from_slice(name);
        bytes[0x1038..0x1040].copy_from_slice(&(0x900 + name.len() as u64 + 1).to_le_bytes());
        for (offset, value) in [
            (0x10a0, 1u64),
            (0x10a8, 0x900),
            (0x10b0, 12),
            (0x10b8, 0x220),
        ] {
            bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        std::fs::write(&library.0, bytes)?;
    }
    let mut emu = Emulator::builder().build()?;
    let module = emu.load_library_without_constructors(&main.0)?;
    let bases: Vec<_> = emu.modules().iter().map(|m| m.base).collect();
    for base in &bases {
        assert_eq!(emu.read_bytes(base + 0x1600, 8)?, [0; 8]);
        emu.add_breakpoint(base + 0x220)?;
    }
    struct Handler(Vec<u64>);
    impl JniHandler for Handler {
        fn stopped(&mut self, _emu: &mut Emulator, stop: &Stop) -> Resume {
            self.0.push(stop.address);
            Resume::Continue
        }
    }
    let mut handler = Handler(Vec::new());
    emu.run_constructors(module, &mut handler)?;
    assert_eq!(
        handler.0,
        [
            bases[1] + 0x220,
            bases[1] + 0x220,
            bases[0] + 0x220,
            bases[0] + 0x220
        ]
    );
    emu.run_constructors(Module(1), &mut handler)?;
    assert_eq!(handler.0.len(), 4);
    for base in bases {
        assert_eq!(emu.read_bytes(base + 0x1600, 8)?, 42u64.to_le_bytes());
    }
    Ok(())
}

#[test]
fn deferred_constructors_stop_in_handler_and_run_once() -> Result<()> {
    // The synthetic ELF supplies AArch64 constructor code.
    let library = elf::TestLibrary::new(false);
    let mut emu = Emulator::builder().build()?;
    assert_eq!(
        emu.run_constructors(Module(0), &mut ())
            .unwrap_err()
            .to_string(),
        "no module 0 is loaded"
    );
    let module = emu.load_library_without_constructors(&library.0)?;
    let data = emu.find_symbol(module, "data")?;
    let constructor = emu.modules()[module.0].base + 0x220;
    assert_eq!(emu.read_bytes(data, 8)?, [0; 8]);
    emu.add_breakpoint(constructor)?;
    struct Handler(Vec<u64>);
    impl JniHandler for Handler {
        fn stopped(&mut self, emu: &mut Emulator, stop: &Stop) -> Resume {
            self.0.push(stop.address);
            // Reentrant initialization skips the module already being initialized.
            emu.run_constructors(Module(0), self).unwrap();
            Resume::Continue
        }
    }
    let mut handler = Handler(Vec::new());
    emu.run_constructors(module, &mut handler)?;
    assert_eq!(handler.0, [constructor]);
    assert_eq!(emu.read_bytes(data, 8)?, 42u64.to_le_bytes());
    emu.write_bytes(data, &91u64.to_le_bytes())?;
    emu.run_constructors(module, &mut handler)?;
    emu.load_library(&library.0)?;
    assert_eq!(handler.0, [constructor]);
    assert_eq!(emu.read_bytes(data, 8)?, 91u64.to_le_bytes());
    Ok(())
}
