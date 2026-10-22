use unidroid::{Emulator, Result, SEARCH_LIMIT};

#[test]
fn search_keeps_overlaps_address_order_and_the_limit() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let address = emu.malloc(4096)?;
        emu.write_bytes(address, &[0xa7; 1010])?;
        assert_eq!(
            emu.search(&[0xa7; 8])?,
            (address..address + 1000).collect::<Vec<_>>()
        );
        assert_eq!(SEARCH_LIMIT, 1000);
        emu.write_bytes(address, &[0; 4096])?;
        emu.write_bytes(address + 100, b"abababa")?;
        emu.write_bytes(address + 10, b"abababa")?;
        assert_eq!(
            emu.search(b"ababa")?,
            [address + 10, address + 12, address + 100, address + 102]
        );
        assert_eq!(
            emu.search(&[]).unwrap_err().to_string(),
            "empty search pattern"
        );
    }
    Ok(())
}

#[test]
fn search_crosses_adjacent_protected_mappings_but_not_holes() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let mmap = emu.resolve("mmap")?;
        let mprotect = emu.resolve("mprotect")?;
        let munmap = emu.resolve("munmap")?;
        let address = emu.call_function(mmap, &[0, 0x3000, 3, 0x22, u64::MAX, 0])?;
        assert_eq!(
            emu.call_function(mprotect, &[address + 0x1000, 0x1000, 0])?,
            0
        );
        let pattern = b"search-across-boundaries";
        emu.write_bytes(address + 0xffb, pattern)?;
        emu.write_bytes(address + 0x1ffb, pattern)?;
        assert_eq!(emu.search(pattern)?, [address + 0xffb, address + 0x1ffb]);
        assert_eq!(emu.call_function(munmap, &[address + 0x1000, 0x1000])?, 0);
        assert!(emu.search(pattern)?.is_empty());
    }
    Ok(())
}

#[test]
fn search_reads_current_guest_memory() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let address = emu.malloc(32)?;
        let pattern = b"unidroid-search-input";
        emu.write_bytes(address, pattern)?;
        assert_eq!(emu.search(pattern)?, [address]);
        emu.write_bytes(address, &[0; 32])?;
        assert!(emu.search(pattern)?.is_empty());
    }
    Ok(())
}
