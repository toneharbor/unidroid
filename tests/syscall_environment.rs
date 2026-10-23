use unidroid::{Emulator, Result};

#[test]
fn access_resolves_rootfs_paths_and_ignores_permission_modes() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let access = emu.find_symbol(libc, "access")?;
        let path = emu.new_cstring(&format!("/{dir}/libc.so"))?;
        for mode in [0, 1, 2, 4, 7] {
            assert_eq!(emu.call_function(access, &[path, mode])?, 0);
        }
        for missing in ["/system/bin/su", "/../Cargo.toml"] {
            let path = emu.new_cstring(missing)?;
            assert_eq!(emu.call_function(access, &[path, 0])? as u32, u32::MAX);
            let errno = emu.find_symbol(libc, "__errno")?;
            let address = emu.call_function(errno, &[])?;
            assert_eq!(emu.read_bytes(address, 4)?, 2u32.to_le_bytes());
        }
    }
    Ok(())
}

#[test]
fn uname_writes_all_six_padded_fields_and_reports_bad_memory() -> Result<()> {
    for (dir, machine) in [("lib64", "armv8l"), ("lib", "armv7l")] {
        let mut emu = Emulator::builder().build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let uname = emu.find_symbol(libc, "uname")?;
        let buffer = emu.malloc(398)?;
        emu.write_bytes(buffer, &[0xff; 398])?;
        assert_eq!(emu.call_function(uname, &[buffer])?, 0);
        let bytes = emu.read_bytes(buffer, 398)?;
        for (field, expected) in bytes[..390].as_chunks::<65>().0.iter().zip([
            "Linux",
            "localhost",
            "1.0.0-unidbg",
            "#1 SMP PREEMPT Thu Apr 19 14:36:58 CST 2018",
            machine,
            "localdomain",
        ]) {
            assert_eq!(&field[..expected.len()], expected.as_bytes());
            assert!(field[expected.len()..].iter().all(|byte| *byte == 0));
        }
        assert_eq!(&bytes[390..], &[0xff; 8]);
        assert_eq!(emu.call_function(uname, &[0])? as u32, u32::MAX);
        let errno = emu.find_symbol(libc, "__errno")?;
        let address = emu.call_function(errno, &[])?;
        assert_eq!(emu.read_bytes(address, 4)?, 14u32.to_le_bytes());
    }
    Ok(())
}
