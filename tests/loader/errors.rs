use super::*;

#[test]
fn relocation_outside_segments_names_library_offset_and_dlopen_caller() -> Result<()> {
    for arm64 in [false, true] {
        let library = elf::TestLibrary::new(arm64, false);
        let mut bytes = std::fs::read(&library.0)?;
        let width = if arm64 { 8 } else { 4 };
        bytes[0x1400..0x1400 + width].copy_from_slice(&0x8000u64.to_le_bytes()[..width]);
        std::fs::write(&library.0, bytes)?;
        for constructors in [false, true] {
            let mut emu = Emulator::builder().build()?;
            let error = if constructors {
                emu.load_library(&library.0)
            } else {
                emu.load_library_without_constructors(&library.0)
            }
            .unwrap_err()
            .to_string();
            assert!(
                error.contains(library.0.file_name().unwrap().to_str().unwrap()),
                "{error}"
            );
            assert!(error.contains("0x8000"), "{error}");
        }
        let main = elf::TestLibrary::new(arm64, true);
        let mut emu = Emulator::builder().build()?;
        let module = emu.load_library(&main.0)?;
        let base = emu.modules()[module.0].base;
        let code = if arm64 {
            vec![0xa9bf7bfeu32, 0xd63f0040, 0xa8c17bfe, 0xd65f03c0]
        } else {
            vec![0xe92d4010, 0xe12fff32, 0xe8bd8010]
        };
        emu.write_bytes(
            base + 0x280,
            &code
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        let open = emu.find_symbol(module, "dlopen")?;
        let name = emu.new_cstring(library.0.file_name().unwrap().to_str().unwrap())?;
        let error = emu
            .call_function(base + 0x280, &[name, 0, open])
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(library.0.file_name().unwrap().to_str().unwrap()),
            "{error}"
        );
        assert!(error.contains("0x8000"), "{error}");
        assert!(
            error.contains(&format!("from {}", emu.location(base + 0x288))),
            "{error}"
        );
    }
    Ok(())
}
