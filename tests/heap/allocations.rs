use super::*;

#[test]
fn repeated_address_reuse_keeps_live_lookups_and_history_consistent() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        caller(&mut emu, arm64)?;
        let reused = guest(&mut emu, "malloc", &[100])?;
        let held = guest(&mut emu, "malloc", &[100])?;
        assert!(reused < held);
        guest(&mut emu, "free", &[reused])?;
        for generation in 0..128 {
            assert_eq!(guest(&mut emu, "malloc", &[100])?, reused);
            let allocation = emu.allocation(reused + 99).unwrap();
            assert!(allocation.live);
            assert_eq!((allocation.address, allocation.size), (reused, 100));
            assert!(allocation.freed.is_none());
            assert_eq!(guest(&mut emu, "realloc", &[reused, 104])?, reused);
            let allocation = emu.allocation(reused + 103).unwrap();
            assert!(allocation.live);
            assert_eq!(allocation.size, 104);
            guest(&mut emu, "free", &[reused])?;
            let allocation = emu.allocation(reused + 103).unwrap();
            assert!(!allocation.live);
            assert_eq!(allocation.size, 104);
            assert!(allocation.freed.is_some());
            assert!(emu.allocation(held + 99).unwrap().live);
            let records = emu.heap();
            assert_eq!(records.len(), generation + 3);
            assert_eq!(
                (records[0].address, records[0].size, records[0].live),
                (reused, 100, false)
            );
            assert_eq!(
                (records[1].address, records[1].size, records[1].live),
                (held, 100, true)
            );
            for record in &records[2..] {
                assert_eq!(
                    (record.address, record.size, record.live),
                    (reused, 104, false)
                );
                assert!(record.freed.is_some());
            }
        }
        assert_eq!(guest(&mut emu, "malloc", &[100])?, reused);
        let checkpoint = emu.checkpoint()?;
        let saved = serde_json::to_value(emu.heap())?;
        emu.reset_heap();
        assert!(emu.heap().is_empty());
        assert!(emu.allocation(reused).is_none());
        assert!(emu.allocation(held).is_none());
        assert!(
            emu.snapshot()?
                .heap
                .iter()
                .all(|record| record.function == "host")
        );
        emu.rewind(&checkpoint)?;
        let mut copy = Emulator::builder()
            .trace(Trace {
                heap: true,
                ..Trace::default()
            })
            .build()?;
        copy.rewind(&checkpoint)?;
        for restored in [&mut emu, &mut copy] {
            assert_eq!(serde_json::to_value(restored.heap())?, saved);
            assert!(restored.allocation(reused + 99).unwrap().live);
            assert!(restored.allocation(held + 99).unwrap().live);
            assert!(!restored.allocation(reused + 103).unwrap().live);
            let live = restored
                .snapshot()?
                .heap
                .into_iter()
                .filter(|record| record.function != "host")
                .map(|record| (record.address, record.size))
                .collect::<Vec<_>>();
            assert_eq!(live, [(held, 100), (reused, 100)]);
            assert_eq!(guest(restored, "realloc", &[reused, 104])?, reused);
            assert!(restored.allocation(reused + 103).unwrap().live);
            guest(restored, "free", &[held])?;
            assert!(!restored.allocation(held + 99).unwrap().live);
        }
    }
    Ok(())
}

#[test]
fn allocation_lookup_prefers_live_then_most_recently_freed_then_host() -> Result<()> {
    struct Return(u64);
    impl unidroid::JniHandler for Return {
        fn stopped(&mut self, _: &mut Emulator, _: &unidroid::Stop) -> unidroid::Resume {
            unidroid::Resume::Return(self.0)
        }
    }
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        let storage = emu.malloc(64)?;
        let malloc = emu.resolve("malloc")?;
        let realloc = emu.resolve("realloc")?;
        let free = emu.resolve("free")?;
        for address in [malloc, realloc, free] {
            emu.add_breakpoint(address)?;
        }
        // Public return hooks model an allocator growing its older block into a freed neighbor.
        emu.call_function_with(&mut Return(storage), malloc, &[16])?;
        emu.call_function_with(&mut Return(storage + 16), malloc, &[16])?;
        emu.call_function_with(&mut Return(0), free, &[storage + 16])?;
        emu.call_function_with(&mut Return(storage), realloc, &[storage, 32])?;
        assert_eq!(emu.allocation(storage + 17).unwrap().address, storage);
        assert!(emu.allocation(storage + 17).unwrap().live);
        emu.call_function_with(&mut Return(0), free, &[storage])?;
        assert_eq!(emu.allocation(storage + 17).unwrap().address, storage);
        assert!(!emu.allocation(storage + 17).unwrap().live);
        let checkpoint = emu.checkpoint()?;
        emu.reset_heap();
        assert_eq!(emu.allocation(storage + 17).unwrap().function, "host");
        let mut copy = Emulator::builder().build()?;
        copy.rewind(&checkpoint)?;
        assert_eq!(copy.allocation(storage + 17).unwrap().address, storage);
        assert!(!copy.allocation(storage + 17).unwrap().live);
    }
    Ok(())
}

#[test]
fn caller_addresses_clear_the_thumb_bit() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        let entry = if arm64 {
            caller(&mut emu, true)?
        } else {
            emu.write_bytes(0x20100, &[0x80, 0xb5, 0x6f, 0x46, 0x98, 0x47, 0x80, 0xbd])?;
            0x20101
        };
        let malloc = emu.resolve("malloc")?;
        let address = emu.call_function(entry, &[16, 0, 0, malloc])?;
        assert_eq!(
            emu.allocation(address).unwrap().backtrace[0].address,
            if arm64 { 0x2010c } else { 0x20106 }
        );
    }
    Ok(())
}

#[test]
fn guest_allocators_capture_sizes_callers_and_release_history() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        caller(&mut emu, arm64)?;
        let a = guest(&mut emu, "malloc", &[100])?;
        let b = guest(&mut emu, "calloc", &[3, 17])?;
        assert_eq!(emu.read_bytes(b, 51)?, [0; 51]);
        let aligned = guest(&mut emu, "memalign", &[64, 80])?;
        assert_eq!(aligned % 64, 0);
        let out = emu.malloc(8)?;
        assert_eq!(guest(&mut emu, "posix_memalign", &[out, 64, 96])?, 0);
        let posix = pointer(&emu, out, arm64)?;
        assert_eq!(posix % 64, 0);
        for (address, size, function) in [
            (a, 100, "malloc"),
            (b, 51, "calloc"),
            (aligned, 80, "memalign"),
            (posix, 96, "posix_memalign"),
        ] {
            let allocation = emu.allocation(address + 1).unwrap();
            assert_eq!((allocation.address, allocation.size), (address, size));
            assert_eq!(allocation.function, function);
            assert!(allocation.live);
            assert_eq!(allocation.thread, 1234);
            assert_eq!(allocation.backtrace[0].address, 0x2010c);
            assert!(allocation.freed.is_none());
        }
        emu.write_bytes(a, b"preserved")?;
        assert_eq!(guest(&mut emu, "realloc", &[a, 104])?, a);
        assert_eq!(emu.allocation(a).unwrap().size, 104);
        let moved = guest(&mut emu, "realloc", &[a, 1024 * 1024])?;
        assert_ne!(moved, a);
        assert_eq!(emu.read_bytes(moved, 9)?, b"preserved");
        let old = emu.allocation(a).unwrap();
        assert!(!old.live);
        assert_eq!(old.freed.unwrap()[0].address, 0x2010c);
        let new = emu.allocation(moved).unwrap();
        assert_eq!(new.function, "realloc");
        assert_eq!(new.size, 1024 * 1024);
        let count = emu.heap().len();
        let largest = if arm64 { u64::MAX } else { u32::MAX as u64 };
        assert_eq!(guest(&mut emu, "malloc", &[largest])?, 0);
        assert_eq!(guest(&mut emu, "calloc", &[largest, 2])?, 0);
        assert_eq!(guest(&mut emu, "realloc", &[moved, largest])?, 0);
        assert_ne!(guest(&mut emu, "posix_memalign", &[out, 3, 20])?, 0);
        guest(&mut emu, "free", &[0])?;
        assert_eq!(emu.heap().len(), count);
        assert!(emu.allocation(moved).unwrap().live);
        for address in [b, aligned, posix, moved] {
            guest(&mut emu, "free", &[address])?;
            let allocation = emu.allocation(address).unwrap();
            assert!(!allocation.live);
            assert_eq!(allocation.freed.unwrap()[0].address, 0x2010c);
        }
        assert!(emu.heap().iter().all(|allocation| !allocation.live));
        assert_eq!(
            emu.heap().iter().map(|a| a.address).collect::<Vec<_>>(),
            [a, b, aligned, posix, moved]
        );
    }
    Ok(())
}

#[test]
fn disabling_preserves_records_and_reset_does_not_forget_host_blocks() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        caller(&mut emu, arm64)?;
        let a = guest(&mut emu, "malloc", &[32])?;
        let saved = serde_json::to_value(emu.heap())?;
        emu.set_trace(Trace::default())?;
        let b = guest(&mut emu, "malloc", &[48])?;
        guest(&mut emu, "free", &[b])?;
        assert_eq!(serde_json::to_value(emu.heap())?, saved);
        assert!(emu.snapshot()?.heap.is_empty());
        emu.set_trace(Trace {
            heap: true,
            ..Trace::default()
        })?;
        guest(&mut emu, "free", &[a])?;
        assert!(!emu.allocation(a).unwrap().live);
        let host = emu.new_cstring("host string")?;
        emu.reset_heap();
        assert!(emu.heap().is_empty());
        assert_eq!(emu.allocation(host).unwrap().function, "host");
        assert!(emu.allocation(a).is_none());
        assert!(guest(&mut emu, "malloc", &[64])? != 0);
        assert_eq!(emu.heap().len(), 1);
    }
    Ok(())
}
