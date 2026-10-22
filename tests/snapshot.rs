use unidroid::{Emulator, Result, Snapshot};

#[path = "snapshot/diff.rs"]
mod diff;

fn temp_path(label: &str) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("unidroid-{label}-{}-{id}.snap", std::process::id()))
}

#[test]
fn snapshot_saves_all_memory_registers_and_modules() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let address = emu.malloc(16)?;
        emu.write_bytes(address, &[1, 2, 3, 4])?;
        let register = if dir == "lib64" { "x0" } else { "r0" };
        let value = if dir == "lib64" {
            u64::MAX
        } else {
            u32::MAX as u64
        };
        emu.set_register(register, value)?;
        let snapshot = emu.snapshot()?;
        assert_eq!(snapshot.thread, 1234);
        assert_eq!(snapshot.registers[0], (register.into(), value));
        assert_eq!(
            snapshot
                .memory
                .iter()
                .map(|(m, _)| m.clone())
                .collect::<Vec<_>>(),
            emu.maps()?
        );
        for (mapping, bytes) in &snapshot.memory {
            assert_eq!(
                *bytes,
                emu.read_bytes(mapping.start, (mapping.end - mapping.start) as usize)?
            );
        }
        assert_eq!(
            serde_json::to_value(&snapshot.modules)?,
            serde_json::to_value(emu.modules())?
        );
        let path = temp_path("roundtrip");
        snapshot.save(&path)?;
        let loaded = Snapshot::load(&path)?;
        assert_eq!(loaded.thread, snapshot.thread);
        assert_eq!(loaded.registers, snapshot.registers);
        assert_eq!(loaded.memory, snapshot.memory);
        assert_eq!(
            serde_json::to_value(&loaded.modules)?,
            serde_json::to_value(&snapshot.modules)?
        );
        let file = std::fs::read(&path)?;
        let newline = file.iter().position(|&b| b == b'\n').unwrap();
        let mut header: serde_json::Value = serde_json::from_slice(&file[..newline])?;
        assert_eq!(header["snapshot"], 1);
        assert_eq!(header["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(header["registers"][0][1].as_u64(), Some(value));
        let raw: Vec<u8> = snapshot
            .memory
            .iter()
            .flat_map(|(_, bytes)| bytes.iter().copied())
            .collect();
        assert_eq!(&file[newline + 1..], raw);
        std::fs::write(&path, &file[..file.len() - 1])?;
        assert!(Snapshot::load(&path).is_err());
        header["snapshot"] = 2.into();
        std::fs::write(&path, format!("{header}\n"))?;
        assert_eq!(
            Snapshot::load(&path).err().unwrap().to_string(),
            "unsupported snapshot format 2"
        );
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn snapshot_diff_finds_guest_bytes_and_registers() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let address = emu.malloc(32)?;
        emu.write_bytes(address, &[0; 32])?;
        let register = if dir == "lib64" { "x0" } else { "r0" };
        emu.set_register(register, 0)?;
        let before = emu.snapshot()?;
        emu.write_bytes(address, &[1, 2, 3, 4])?;
        emu.set_register(register, 7)?;
        let after = emu.snapshot()?;
        let diff = before.diff(&after);
        assert_eq!(diff.registers, [(register.into(), 0, 7)]);
        assert!(diff.mapped.is_empty());
        assert!(diff.unmapped.is_empty());
        assert_eq!(diff.changes.len(), 1);
        let change = &diff.changes[0];
        assert_eq!(change.address, address);
        assert_eq!(change.location, format!("0x{address:x} [host malloc]"));
        assert_eq!(change.before, [0; 4]);
        assert_eq!(change.after, [1, 2, 3, 4]);
        let path = temp_path("diff-roundtrip");
        after.save(&path)?;
        let loaded = Snapshot::load(&path)?;
        let equal = after.diff(&loaded);
        assert!(
            equal.registers.is_empty()
                && equal.mapped.is_empty()
                && equal.unmapped.is_empty()
                && equal.changes.is_empty()
        );
        std::fs::remove_file(path)?;
        emu.malloc(8192)?;
        let allocated = emu.snapshot()?;
        let diff = after.diff(&allocated);
        assert!(!diff.mapped.is_empty());
        assert!(diff.mapped.iter().all(|m| m.name == "[host malloc]"));
        assert!(diff.unmapped.is_empty() && diff.changes.is_empty());
        let reverse = allocated.diff(&after);
        assert_eq!(reverse.unmapped, diff.mapped);
        let data = emu
            .maps()?
            .into_iter()
            .find(|m| m.name == "libc.so" && m.perms == "rw-")
            .unwrap();
        let byte = emu.read_bytes(data.start, 1)?[0];
        emu.write_bytes(data.start, &[byte ^ 0xff])?;
        let changed = emu.snapshot()?;
        let diff = allocated.diff(&changed);
        assert_eq!(diff.changes.len(), 1);
        assert_eq!(diff.changes[0].location, emu.location(data.start));
        assert!(diff.changes[0].location.starts_with("libc.so+0x"));
        assert_eq!(emu.resolve(&diff.changes[0].location)?, data.start);
    }
    Ok(())
}

#[test]
fn cli_prints_snapshot_diff_and_uses_the_main_error_path() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let address = emu.malloc(4)?;
        emu.write_bytes(address, &[0; 4])?;
        let before = temp_path("cli-before");
        let after = temp_path("cli-after");
        emu.snapshot()?.save(&before)?;
        emu.write_bytes(address, &[1, 2, 3, 4])?;
        emu.snapshot()?.save(&after)?;
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_unidroid"))
            .arg("diff")
            .arg(&before)
            .arg(&after)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        let stdout = String::from_utf8(output.stdout)?;
        assert_eq!(
            stdout,
            format!(
                "registers:\n  (none)\nmapped:\n  (none)\nunmapped:\n  (none)\nchanges:\n  0x{address:x} [host malloc], 4 bytes\n    0000 - 00 00 00 00\n    0000 + 01 02 03 04\n"
            )
        );
        std::fs::write(
            &before,
            "{\"snapshot\":9,\"version\":\"test\",\"thread\":1,\"registers\":[],\"modules\":[],\"maps\":[]}\n",
        )?;
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_unidroid"))
            .arg("diff")
            .arg(&before)
            .arg(&after)
            .output()?;
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr)?,
            "error: unsupported snapshot format 9\n"
        );
        std::fs::remove_file(before)?;
        std::fs::remove_file(after)?;
    }
    Ok(())
}

#[test]
fn snapshots_read_protected_memory_and_diff_across_live_region_splits() -> Result<()> {
    for (dir, brk) in [("lib64", 214), ("lib", 45)] {
        let mut emu = Emulator::builder().build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let mmap = emu.find_symbol(libc, "mmap")?;
        let mprotect = emu.find_symbol(libc, "mprotect")?;
        let munmap = emu.find_symbol(libc, "munmap")?;
        let syscall = emu.find_symbol(libc, "syscall")?;
        assert_eq!(emu.call_function(syscall, &[brk, 0x60001000])?, 0x60001000);
        assert!(
            emu.maps()?
                .iter()
                .any(|m| m.name == "[heap]" && m.start == 0x60000000 && m.end == 0x60001000)
        );
        let address = emu.call_function(mmap, &[0, 0x3000, 7, 0x22, u64::MAX, 0])?;
        assert!((0x50000000..0x60000000).contains(&address));
        let mapping = emu
            .maps()?
            .into_iter()
            .find(|m| m.start == address)
            .unwrap();
        assert_eq!(mapping.perms, "rwx");
        assert_eq!(mapping.name, "[mmap]");
        assert_eq!(mapping.end, address + 0x3000);
        emu.write_bytes(address + 0xfff, &[0; 10])?;
        let before = emu.snapshot()?;
        assert_eq!(
            emu.call_function(mprotect, &[address + 0x1000, 0x1000, 0])?,
            0
        );
        let maps = emu.maps()?;
        let protected = maps.iter().find(|m| m.start == address + 0x1000).unwrap();
        assert_eq!(protected.perms, "---");
        assert_eq!(protected.end, address + 0x2000);
        emu.write_bytes(address + 0xfff, &[1])?;
        emu.write_bytes(address + 0x1007, &[2])?;
        // The return page is executable and read-only to guest code.
        emu.write_bytes(0x20f00, &[0xab])?;
        let after = emu.snapshot()?;
        let diff = before.diff(&after);
        assert!(diff.mapped.is_empty() && diff.unmapped.is_empty());
        let changes: Vec<_> = diff
            .changes
            .iter()
            .filter(|c| c.address >= address && c.address < address + 0x3000)
            .collect();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].address, address + 0xfff);
        assert_eq!(changes[0].before, [0; 9]);
        assert_eq!(changes[0].after, [1, 0, 0, 0, 0, 0, 0, 0, 2]);
        assert!(
            diff.changes
                .iter()
                .any(|c| c.address == 0x20f00 && c.after == [0xab])
        );
        assert_eq!(emu.call_function(munmap, &[address + 0x1000, 0x1000])?, 0);
        let diff = after.diff(&emu.snapshot()?);
        assert_eq!(diff.unmapped, std::slice::from_ref(protected));
        assert!(diff.mapped.is_empty());
    }
    Ok(())
}

#[test]
fn maps_name_live_regions_with_exclusive_ends() -> Result<()> {
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let allocation = emu.malloc(8192)?;
        let maps = emu.maps()?;
        assert!(maps.iter().all(|m| m.start < m.end));
        assert!(maps.windows(2).all(|m| m[0].end <= m[1].start));
        assert!(maps.iter().any(|m| m.name == "libc.so" && m.perms == "r-x"));
        let stack = maps.iter().find(|m| m.name == "[stack]").unwrap();
        assert_eq!(
            (stack.start, stack.end, stack.perms.as_str()),
            (0x70000000, 0x70200000, "rw-")
        );
        assert!(
            maps.iter()
                .any(|m| m.name == "[trampoline]" && m.start == 0x10000)
        );
        assert!(
            maps.iter()
                .any(|m| m.name == "[trampoline]" && m.end == 0x21000)
        );
        assert_eq!(maps.iter().any(|m| m.name == "[vectors]"), dir == "lib");
        if dir == "lib64" {
            let mmap = emu.resolve("mmap")?;
            assert_eq!(
                emu.call_function(mmap, &[0xffff0000, 4096, 3, 0x32, u64::MAX, 0])?,
                0xffff0000
            );
            let fixed = emu
                .maps()?
                .into_iter()
                .find(|m| m.start == 0xffff0000)
                .unwrap();
            assert_eq!(fixed.name, "[vectors]");
        }
        assert!(
            maps.iter()
                .any(|m| m.name == "[host malloc]" && m.start <= allocation && m.end > allocation)
        );
    }
    Ok(())
}
