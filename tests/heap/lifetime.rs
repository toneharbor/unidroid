use super::*;
use unidroid::{JniHandler, Resume, Stop};

struct Return(u64);
impl JniHandler for Return {
    fn stopped(&mut self, _: &mut Emulator, _: &Stop) -> Resume {
        Resume::Return(self.0)
    }
}

#[test]
fn unreadable_posix_memalign_output_skips_the_record() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        let function = emu.resolve("posix_memalign")?;
        emu.add_breakpoint(function)?;
        assert_eq!(
            emu.call_function_with(&mut Return(0), function, &[0, 16, 32])?,
            0
        );
        assert!(emu.heap().is_empty());
    }
    Ok(())
}

#[test]
fn reused_ranges_retire_unobserved_releases_and_stay_retired_after_rewind() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        let storage = emu.malloc(128)?;
        let malloc = emu.resolve("malloc")?;
        let free = emu.resolve("free")?;
        emu.add_breakpoint(malloc)?;
        emu.add_breakpoint(free)?;
        for address in [storage, storage + 32, storage + 64] {
            emu.call_function_with(&mut Return(address), malloc, &[32])?;
        }
        emu.set_trace(Trace::default())?;
        emu.call_function_with(&mut Return(0), free, &[storage])?;
        emu.set_trace(Trace {
            heap: true,
            ..Trace::default()
        })?;
        emu.remove_breakpoint(malloc)?;
        emu.add_breakpoint(malloc)?;
        // Replace two overlapping records while preserving the adjacent one.
        emu.call_function_with(&mut Return(storage + 8), malloc, &[56])?;
        let records = emu.heap();
        assert_eq!(
            records.iter().map(|a| a.live).collect::<Vec<_>>(),
            [false, false, true, true]
        );
        assert!(records[..2].iter().all(|a| a.freed.is_none()));
        let checkpoint = emu.checkpoint()?;
        let mut restored = Emulator::builder().build()?;
        restored.rewind(&checkpoint)?;
        for current in [&emu, &restored] {
            assert_eq!(current.allocation(storage).unwrap().function, "host");
            assert_eq!(
                current.allocation(storage + 40).unwrap().address,
                storage + 8
            );
            assert_eq!(
                current.allocation(storage + 64).unwrap().address,
                storage + 64
            );
        }
    }
    Ok(())
}

#[test]
fn in_place_growth_retires_unobserved_neighbors() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        let storage = emu.malloc(64)?;
        let malloc = emu.resolve("malloc")?;
        let realloc = emu.resolve("realloc")?;
        let free = emu.resolve("free")?;
        for address in [malloc, free] {
            emu.add_breakpoint(address)?;
        }
        for address in [storage, storage + 16] {
            emu.call_function_with(&mut Return(address), malloc, &[16])?;
        }
        emu.set_trace(Trace::default())?;
        emu.call_function_with(&mut Return(0), free, &[storage + 16])?;
        emu.set_trace(Trace {
            heap: true,
            ..Trace::default()
        })?;
        emu.add_breakpoint(realloc)?;
        emu.call_function_with(&mut Return(storage), realloc, &[storage, 32])?;
        let records = emu.heap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].size, 32);
        assert!(records[0].live);
        assert!(!records[1].live);
        assert!(records[1].freed.is_none());
        assert_eq!(emu.allocation(storage + 24).unwrap().address, storage);
    }
    Ok(())
}

#[test]
fn releases_and_in_place_realloc_keep_the_entry_record_identity() -> Result<()> {
    struct Reuse {
        storage: u64,
        malloc: u64,
        free: u64,
        result: u64,
    }
    impl JniHandler for Reuse {
        fn stopped(&mut self, emu: &mut Emulator, _: &Stop) -> Resume {
            // A nested call releases the old block and reuses its address before
            // the suspended allocator returns, just as another guest thread can.
            emu.call_function_with(&mut Return(0), self.free, &[self.storage])
                .unwrap();
            emu.call_function_with(&mut Return(self.storage), self.malloc, &[48])
                .unwrap();
            Resume::Return(self.result)
        }
    }
    for arm64 in [true, false] {
        for operation in ["free", "moving", "in_place"] {
            let mut emu = emulator(arm64)?;
            let storage = emu.malloc(128)?;
            let malloc = emu.resolve("malloc")?;
            let free = emu.resolve("free")?;
            let realloc = emu.resolve("realloc")?;
            for address in [malloc, free, realloc] {
                emu.add_breakpoint(address)?;
            }
            emu.call_function_with(&mut Return(storage), malloc, &[16])?;
            let (function, result) = match operation {
                "free" => (free, 0),
                "moving" => (realloc, storage + 64),
                _ => (realloc, storage),
            };
            emu.call_function_with(
                &mut Reuse {
                    storage,
                    malloc,
                    free,
                    result,
                },
                function,
                &[storage, 32],
            )?;
            let current = emu.allocation(storage).unwrap();
            assert!(current.live, "{operation}");
            assert_eq!(current.size, 48, "{operation}");
            let records = emu.heap();
            assert_eq!(records.len(), if operation == "moving" { 3 } else { 2 });
            assert!(!records[0].live);
            assert!(records[0].freed.is_some());
            assert!(
                records
                    .iter()
                    .any(|a| a.address == storage && a.size == 48 && a.live)
            );
        }
    }
    Ok(())
}
