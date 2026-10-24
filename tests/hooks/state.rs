use super::*;
use std::{cell::RefCell, rc::Rc};
use unidroid::{Event, Trace};
#[path = "../support/elf.rs"]
mod elf;

#[test]
fn rebinding_preserves_explicit_and_implicit_addends() -> Result<()> {
    for (directory, kind, width) in [("lib64", 257, 8), ("lib", 2, 4)] {
        let path = format!("rootfs/{directory}/libc.so");
        let bytes = std::fs::read(&path)?;
        let image = goblin::elf::Elf::parse(&bytes)?;
        let relocation = image
            .dynrelas
            .iter()
            .chain(image.dynrels.iter())
            .find(|r| r.r_type == kind && r.r_sym != 0)
            .unwrap();
        let name = image
            .dynstrtab
            .get_at(image.dynsyms.get(relocation.r_sym).unwrap().st_name)
            .unwrap();
        let addend = if width == 8 {
            relocation.r_addend.unwrap() as u64
        } else {
            let segment = image
                .program_headers
                .iter()
                .find(|s| {
                    s.p_type == 1
                        && s.p_vaddr <= relocation.r_offset
                        && relocation.r_offset < s.p_vaddr + s.p_filesz
                })
                .unwrap();
            let offset = (segment.p_offset + relocation.r_offset - segment.p_vaddr) as usize;
            u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as u64
        };
        let mut emu = Emulator::builder().build()?;
        let module = emu.load_library_without_constructors(path)?;
        let slot = emu.modules()[module.0].base + relocation.r_offset;
        // Guest edits must not become the addend when rebinding an existing slot.
        emu.write_bytes(slot, &[0xaa; 8][..width])?;
        emu.hook_import(name)?;
        let hook = emu.find_symbol(module, name)?;
        assert_eq!(
            emu.read_bytes(slot, width)?,
            hook.wrapping_add(addend).to_le_bytes()[..width]
        );
    }
    for arm64 in [true, false] {
        let width = if arm64 { 8 } else { 4 };
        let library = elf::TestLibrary::new(arm64, false);
        let mut emu = Emulator::builder().build()?;
        let module = emu.load_library(&library.0)?;
        let data = emu.find_symbol(module, "data")?;
        emu.hook_import("answer")?;
        let hook = emu.find_symbol(module, "answer")?;
        assert_eq!(
            emu.read_bytes(data + 8, width)?,
            (hook + 4).to_le_bytes()[..width]
        );
        assert_eq!(
            emu.read_bytes(data + 16, width)?,
            hook.to_le_bytes()[..width]
        );
        assert_eq!(
            emu.read_bytes(data + 24, width)?,
            hook.to_le_bytes()[..width]
        );
    }
    Ok(())
}

#[test]
fn missing_original_becomes_available_after_another_load() -> Result<()> {
    for arm64 in [true, false] {
        let importer = elf::TestLibrary::new(arm64, false);
        let exporter = elf::TestLibrary::new(arm64, false);
        let mut bytes = std::fs::read(&exporter.0)?;
        let (info, section, value, width) = if arm64 {
            (0x1264, 0x1266, 0x1268, 8)
        } else {
            (0x124c, 0x124e, 0x1244, 4)
        };
        bytes[info] = 0x12;
        bytes[section] = 1;
        bytes[value..value + width].copy_from_slice(&0x200u64.to_le_bytes()[..width]);
        std::fs::write(&exporter.0, bytes)?;
        let mut emu = Emulator::builder().build()?;
        emu.hook_import("weak")?;
        let module = emu.load_library(&importer.0)?;
        let hook = emu.find_symbol(module, "weak")?;
        let mut handler = Answer {
            seen: Vec::new(),
            value: Some(73),
        };
        assert_eq!(emu.call_function_with(&mut handler, hook, &[])?, 73);
        assert_eq!(handler.seen[0].2, None);
        let defined = emu.load_library(&exporter.0)?;
        handler.value = None;
        assert_eq!(emu.call_function_with(&mut handler, hook, &[])?, 42);
        assert_eq!(
            handler.seen[1].2,
            Some(emu.modules()[defined.0].base + 0x200)
        );
    }
    Ok(())
}

#[test]
fn originals_resolve_later_exports_in_load_order() -> Result<()> {
    for arm64 in [true, false] {
        let first = elf::TestLibrary::new(arm64, false);
        let second = elf::TestLibrary::new(arm64, false);
        let mut bytes = std::fs::read(&second.0)?;
        bytes[0x200..0x204]
            .copy_from_slice(&(if arm64 { 0xd28000e0u32 } else { 0xe3a00007 }).to_le_bytes()); // mov x0/r0,#7
        std::fs::write(&second.0, bytes)?;
        let mut emu = Emulator::builder().build()?;
        emu.hook_import("answer")?;
        let first_module = emu.load_library(&first.0)?;
        emu.load_library(&second.0)?;
        let hook = emu.find_symbol(first_module, "answer")?;
        let mut handler = Answer {
            seen: Vec::new(),
            value: None,
        };
        assert_eq!(emu.call_function_with(&mut handler, hook, &[])?, 42);
        assert_eq!(handler.seen[0].2, Some(emu.modules()[0].base + 0x200));
    }
    Ok(())
}

#[test]
fn tracing_restores_hook_names_when_rewinding_without_module_changes() -> Result<()> {
    for arm64 in [true, false] {
        let library = elf::TestLibrary::new(arm64, true);
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut emu = Emulator::builder()
            .trace(Trace {
                imports: true,
                ..Trace::default()
            })
            .on_event(move |event| {
                if let Event::Call {
                    function, module, ..
                } = event
                {
                    sink.borrow_mut().push((function.clone(), module.clone()));
                }
            })
            .build()?;
        let module = emu.load_library(&library.0)?;
        let base = emu.modules()[module.0].base;
        // Save lr, call the address in x0/r0, restore lr, return.
        let code = if arm64 {
            vec![0xa9bf7bfeu32, 0xd63f0000, 0xa8c17bfe, 0xd65f03c0]
        } else {
            vec![0xe92d4010, 0xe12fff30, 0xe8bd8010]
        };
        emu.write_bytes(
            base + 0x280,
            &code
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        let empty = emu.checkpoint()?;
        emu.hook_import("answer")?;
        let hooked = emu.checkpoint()?;
        emu.rewind(&empty)?;
        emu.hook_import("other_name")?;
        emu.rewind(&hooked)?;
        events.borrow_mut().clear();
        let hook = emu.find_symbol(module, "answer")?;
        let mut handler = Answer {
            seen: Vec::new(),
            value: Some(73),
        };
        assert_eq!(
            emu.call_function_with(&mut handler, base + 0x280, &[hook])?,
            73
        );
        assert_eq!(&*events.borrow(), &[("answer".into(), "<hook>".into())]);
    }
    Ok(())
}
