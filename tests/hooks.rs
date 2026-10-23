use unidroid::{Emulator, ImportCall, JniHandler, Result};

#[path = "hooks/behavior.rs"]
mod behavior;
#[path = "hooks/state.rs"]
mod state;

struct Answer {
    seen: Vec<(String, Vec<u64>, Option<u64>, u64)>,
    value: Option<u64>,
}
impl JniHandler for Answer {
    fn import(&mut self, _emu: &mut Emulator, call: &ImportCall<'_>) -> Option<u64> {
        self.seen.push((
            call.name.into(),
            call.args.to_vec(),
            call.original,
            call.return_address,
        ));
        self.value
    }
}

#[test]
fn hook_after_loading_rebinds_existing_got_calls() -> Result<()> {
    for (directory, code) in [
        ("arm64-v8a", &[0xf9400010u32, 0xaa0103e0, 0xd61f0200][..]),
        ("armeabi-v7a", &[0xe590c000, 0xe1a00001, 0xe12fff1c][..]),
    ] {
        let path = format!("tests/fixtures/{directory}/libjnidispatch.so");
        let bytes = std::fs::read(&path)?;
        let image = goblin::elf::Elf::parse(&bytes)?;
        let relocation = image
            .dynrelas
            .iter()
            .chain(image.dynrels.iter())
            .chain(image.pltrelocs.iter())
            .find(|r| {
                image
                    .dynstrtab
                    .get_at(image.dynsyms.get(r.r_sym).unwrap().st_name)
                    == Some("strlen")
            })
            .unwrap();
        let mut emu = Emulator::builder().build()?;
        let module = emu.load_library(path)?;
        let slot = emu.modules()[module.0].base + relocation.r_offset;
        emu.write_bytes(
            0x20100,
            &code
                .iter()
                .flat_map(|word| word.to_le_bytes())
                .collect::<Vec<_>>(),
        )?;
        let text = emu.new_cstring("hello")?;
        assert_eq!(emu.call_function(0x20100, &[slot, text])?, 5);
        emu.hook_import("strlen")?;
        let mut handler = Answer {
            seen: Vec::new(),
            value: Some(73),
        };
        assert_eq!(
            emu.call_function_with(&mut handler, 0x20100, &[slot, text])?,
            73
        );
        assert_eq!(handler.seen[0].0, "strlen");
        handler.value = None;
        assert_eq!(
            emu.call_function_with(&mut handler, 0x20100, &[slot, text])?,
            5
        );
    }
    Ok(())
}

#[test]
fn hook_before_loading_replaces_and_passes_through() -> Result<()> {
    for (directory, registers) in [("lib64", 8), ("lib", 4)] {
        let mut emu = Emulator::builder().build()?;
        emu.hook_import("strlen")?;
        emu.hook_import("strlen")?;
        let module = emu.load_library(format!("rootfs/{directory}/libc.so"))?;
        let function = emu.find_symbol(module, "strlen")?;
        let text = emu.new_cstring("hello")?;
        let mut handler = Answer {
            seen: Vec::new(),
            value: Some(73),
        };
        assert_eq!(
            emu.call_function_with(&mut handler, function, &[text, 19])?,
            73
        );
        let (name, args, original, caller) = &handler.seen[0];
        assert_eq!(name, "strlen");
        assert_eq!(args.len(), registers);
        assert_eq!(&args[..2], &[text, 19]);
        assert_ne!(original.unwrap(), function);
        assert_eq!(*caller, 0x20000);
        handler.value = None;
        assert_eq!(emu.call_function_with(&mut handler, function, &[text])?, 5);
        assert_eq!(emu.call_function(function, &[text])?, 5);
        emu.hook_import("strlen")?;
        assert_eq!(emu.find_symbol(module, "strlen")?, function);
    }
    Ok(())
}
