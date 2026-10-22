use super::*;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use unidroid::Snapshot;

struct File(PathBuf);
impl File {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Self(std::env::temp_dir().join(format!(
            "unidroid-heap-{}-{}.snap",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}
impl Drop for File {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn snapshots_save_live_blocks_load_old_headers_and_report_allocations() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        caller(&mut emu, arm64)?;
        let a = guest(&mut emu, "malloc", &[100])?;
        let stable = guest(&mut emu, "calloc", &[1, 256])?;
        let host = emu.new_cstring("known host allocation")?;
        let before = emu.snapshot()?;
        assert!(before.heap.iter().all(|a| a.live));
        let host_record = before.heap.iter().find(|a| a.address == host).unwrap();
        assert_eq!(host_record.function, "host");
        assert_eq!(host_record.thread, 1234);
        assert!(host_record.backtrace.is_empty());
        let b = guest(&mut emu, "malloc", &[1024 * 1024])?;
        guest(&mut emu, "free", &[a])?;
        emu.write_bytes(stable + 16, &[1, 2, 3, 4])?;
        let after = emu.snapshot()?;
        assert!(!after.heap.iter().any(|allocation| allocation.address == a));
        let diff = before.diff(&after);
        assert_eq!(diff.allocated.len(), 1);
        assert_eq!(diff.allocated[0].address, b);
        assert_eq!(diff.freed.len(), 1);
        assert_eq!(diff.freed[0].address, a);
        let change = diff
            .changes
            .iter()
            .find(|c| c.address == stable + 16)
            .unwrap();
        assert_eq!(change.allocation, Some((stable, 256)));
        let report = diff.report();
        assert!(report.find("unmapped:").unwrap() < report.find("allocated:").unwrap());
        assert!(report.find("allocated:").unwrap() < report.find("freed:").unwrap());
        assert!(report.contains("malloc from 0x2010c"));
        assert!(report.contains(&format!("4 bytes in block 0x{stable:x}+0x10")));
        assert!(
            before
                .diff(&before)
                .report()
                .contains("allocated:\n  (none)\nfreed:\n  (none)")
        );
        let file = File::new();
        after.save(&file.0)?;
        let loaded = Snapshot::load(&file.0)?;
        assert_eq!(
            serde_json::to_value(&loaded.heap)?,
            serde_json::to_value(&after.heap)?
        );
        assert_eq!(before.diff(&loaded).report(), report);
        let bytes = std::fs::read(&file.0)?;
        let split = bytes.iter().position(|&byte| byte == b'\n').unwrap();
        let mut header: serde_json::Value = serde_json::from_slice(&bytes[..split])?;
        header.as_object_mut().unwrap().remove("heap");
        let mut legacy = serde_json::to_vec(&header)?;
        legacy.extend_from_slice(&bytes[split..]);
        std::fs::write(&file.0, legacy)?;
        let without = Snapshot::load(&file.0)?;
        assert!(without.heap.is_empty());
        assert!(!without.diff(&without).report().contains("allocated:"));
    }
    Ok(())
}

#[test]
fn allocation_identity_uses_address_and_size_and_changes_fall_back_to_before() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        caller(&mut emu, arm64)?;
        let a = guest(&mut emu, "calloc", &[1, 100])?;
        let before = emu.snapshot()?;
        assert_eq!(guest(&mut emu, "realloc", &[a, 104])?, a);
        let after = emu.snapshot()?;
        let diff = before.diff(&after);
        assert_eq!(
            (diff.allocated[0].address, diff.allocated[0].size),
            (a, 104)
        );
        assert_eq!((diff.freed[0].address, diff.freed[0].size), (a, 100));
        guest(&mut emu, "free", &[a])?;
        emu.write_bytes(a + 32, &[7; 4])?;
        let freed = emu.snapshot()?;
        assert!(
            after
                .diff(&freed)
                .changes
                .iter()
                .any(|c| c.address <= a + 32
                    && c.address + c.after.len() as u64 > a + 32
                    && c.allocation == Some((a, 104)))
        );
    }
    Ok(())
}

#[test]
fn rewind_and_fresh_copies_restore_heap_records_but_keep_local_trace_settings() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        caller(&mut emu, arm64)?;
        let a = guest(&mut emu, "malloc", &[64])?;
        let old = guest(&mut emu, "malloc", &[32])?;
        guest(&mut emu, "free", &[old])?;
        let checkpoint = emu.checkpoint()?;
        let saved = serde_json::to_value(emu.heap())?;
        guest(&mut emu, "malloc", &[256])?;
        guest(&mut emu, "free", &[a])?;
        emu.rewind(&checkpoint)?;
        assert!(emu.trace().heap);
        assert_eq!(serde_json::to_value(emu.heap())?, saved);
        for enabled in [false, true] {
            let mut copy = Emulator::builder()
                .trace(Trace {
                    heap: enabled,
                    ..Trace::default()
                })
                .build()?;
            copy.rewind(&checkpoint)?;
            assert_eq!(copy.trace().heap, enabled);
            assert_eq!(serde_json::to_value(copy.heap())?, saved);
            guest(&mut copy, "malloc", &[4096])?;
            assert_eq!(copy.heap().len(), 2 + usize::from(enabled));
            copy.rewind(&checkpoint)?;
            assert_eq!(serde_json::to_value(copy.heap())?, saved);
            copy.set_trace(Trace {
                heap: true,
                ..Trace::default()
            })?;
            guest(&mut copy, "free", &[a])?;
            assert!(!copy.allocation(a).unwrap().live);
        }
        emu.set_trace(Trace::default())?;
        emu.rewind(&checkpoint)?;
        assert!(!emu.trace().heap);
        assert_eq!(serde_json::to_value(emu.heap())?, saved);
    }
    Ok(())
}
