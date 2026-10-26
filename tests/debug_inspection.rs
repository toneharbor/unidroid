use unidroid::{Emulator, Result};

#[test]
fn resolve_accepts_locations_hex_and_exports_and_names_bad_parts() -> Result<()> {
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let mut emu = Emulator::builder().build()?;
        let path = format!("tests/fixtures/{abi}/libjnidispatch.so");
        let module = emu.load_library(&path)?;
        let modules = emu.modules();
        assert_eq!(modules[module.0].name, "libjnidispatch.so");
        assert_eq!(modules[module.0].path, std::path::PathBuf::from(&path));
        assert!(modules[module.0].size > 0x1000);
        assert_eq!(
            emu.resolve("libjnidispatch.so+0x1234")?,
            modules[module.0].base + 0x1234
        );
        assert_eq!(emu.resolve("0x50001000")?, 0x50001000);
        assert_eq!(
            emu.resolve("JNI_OnLoad")?,
            emu.find_symbol(module, "JNI_OnLoad")?
        );
        assert_eq!(emu.resolve("strlen")?, emu.find_symbol(module, "strlen")?);
        for (text, error) in [
            ("missing.so+0x1", "unknown module missing.so"),
            ("libjnidispatch.so+wat", "bad number wat"),
            ("libjnidispatch.so+0xg", "bad number 0xg"),
            ("0xno", "bad number 0xno"),
            ("0x10000000000000000", "bad number 0x10000000000000000"),
            ("missing_symbol", "unknown symbol missing_symbol"),
        ] {
            assert!(emu.resolve(text).unwrap_err().to_string().contains(error));
        }
        for (index, module) in modules.iter().enumerate() {
            assert_eq!(emu.load_library(&module.path)?.0, index);
        }
    }
    Ok(())
}

#[test]
fn named_registers_match_dumps_and_preserve_float_bits() -> Result<()> {
    for (dir, gp) in [("lib64", "x0"), ("lib", "r0")] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        emu.set_register(gp, 42)?;
        assert_eq!(emu.register(gp)?, 42);
        for n in 0..32 {
            let name = format!("d{n}");
            let bits = 0xfff8000000000000 | n;
            emu.set_register(&name, bits)?;
            assert_eq!(emu.register(&name)?, bits);
        }
        let registers = emu.registers()?;
        assert_eq!(registers.len(), if dir == "lib64" { 34 } else { 17 });
        assert!(registers.iter().all(|r| !r.0.starts_with('d')));
        for (name, value) in &registers {
            assert_eq!(emu.register(name)?, *value);
        }
        for error in [
            emu.register("oops").unwrap_err(),
            emu.set_register("oops", 1).unwrap_err(),
        ] {
            assert!(error.to_string().contains("unknown register oops"));
            assert!(error.to_string().contains("d0-d31"));
            for name in if dir == "lib64" {
                &["x0-x28", "fp", "lr", "sp", "pc", "nzcv"][..]
            } else {
                &["r0-r12", "sp", "lr", "pc", "cpsr"][..]
            } {
                assert!(error.to_string().contains(name));
            }
        }
    }
    Ok(())
}
