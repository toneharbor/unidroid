use unidroid::{Mapping, Snapshot};

fn snapshot(regions: &[(u64, &[u8])]) -> Snapshot {
    Snapshot {
        thread: 1234,
        registers: Vec::new(),
        modules: Vec::new(),
        memory: regions
            .iter()
            .map(|&(start, bytes)| {
                (
                    Mapping {
                        start,
                        end: start + bytes.len() as u64,
                        perms: "rw-".into(),
                        name: "[mmap]".into(),
                    },
                    bytes.to_vec(),
                )
            })
            .collect(),
    }
}

#[test]
fn changes_merge_across_adjacent_mappings_but_not_eight_unchanged_bytes() {
    for gap in [7, 8] {
        let before = snapshot(&[(0x1000, &[0; 32])]);
        let mut bytes = [0; 32];
        bytes[1] = 1;
        bytes[gap + 2] = 2;
        let mut after = snapshot(&[(0x1000, &bytes[..5]), (0x1005, &bytes[5..])]);
        after.memory[1].0.perms = "---".into();
        let diff = before.diff(&after);
        assert!(diff.mapped.is_empty() && diff.unmapped.is_empty());
        assert_eq!(diff.changes.len(), if gap == 7 { 1 } else { 2 });
        assert_eq!(diff.changes[0].address, 0x1001);
        if gap == 7 {
            assert_eq!(diff.changes[0].before, [0; 9]);
            assert_eq!(diff.changes[0].after, [1, 0, 0, 0, 0, 0, 0, 0, 2]);
        } else {
            assert_eq!(diff.changes[0].after, [1]);
            assert_eq!(diff.changes[1].address, 0x100a);
            assert_eq!(diff.changes[1].after, [2]);
        }
    }
    let before = snapshot(&[(0x1000, &[0; 4]), (0x1004, &[0; 4])]);
    let after = snapshot(&[(0x1000, &[1; 8])]);
    assert_eq!(before.diff(&after).changes[0].after, [1; 8]);
    // A hole is never an unchanged-byte bridge, even when it is only one byte.
    let before = snapshot(&[(0x1000, &[0; 4]), (0x1005, &[0; 4])]);
    let after = snapshot(&[(0x1000, &[1; 9])]);
    let diff = before.diff(&after);
    assert_eq!(diff.changes.len(), 2);
    assert_eq!((diff.mapped[0].start, diff.mapped[0].end), (0x1004, 0x1005));
    // Different mapping boundaries and permissions alone are not a change.
    let before = snapshot(&[(0x1000, &[0; 9])]);
    let mut after = snapshot(&[(0x1000, &[0; 3]), (0x1003, &[0; 6])]);
    after.memory[1].0.perms = "r-x".into();
    let diff = before.diff(&after);
    assert!(diff.changes.is_empty() && diff.mapped.is_empty() && diff.unmapped.is_empty());
}

#[test]
fn coverage_differences_keep_source_names_permissions_and_boundaries() {
    let mut before = snapshot(&[(0x1002, &[0; 2]), (0x1006, &[0; 4]), (0x100a, &[0; 2])]);
    before.memory[2].0.name.clear();
    let mut after = snapshot(&[
        (0x1000, &[0; 7]),
        (0x1007, &[0; 1]),
        (0x1010, &[0; 1]),
        (0x1011, &[0; 1]),
    ]);
    after.memory[0].0.perms = "r-x".into();
    after.memory[3].0.name = "[heap]".into();
    let diff = before.diff(&after);
    assert_eq!(
        diff.mapped
            .iter()
            .map(|m| (m.start, m.end, m.perms.as_str(), m.name.as_str()))
            .collect::<Vec<_>>(),
        [
            (0x1000, 0x1002, "r-x", "[mmap]"),
            (0x1004, 0x1006, "r-x", "[mmap]"),
            (0x1010, 0x1011, "rw-", "[mmap]"),
            (0x1011, 0x1012, "rw-", "[heap]"),
        ]
    );
    assert_eq!(
        diff.unmapped
            .iter()
            .map(|m| (m.start, m.end, m.name.as_str()))
            .collect::<Vec<_>>(),
        [(0x1008, 0x100a, "[mmap]"), (0x100a, 0x100c, ""),]
    );
    assert!(diff.changes.is_empty());
    assert_eq!(after.diff(&before).mapped, diff.unmapped);
}

#[test]
fn report_has_all_sections_offsets_and_no_trailing_newline() {
    let mut before = snapshot(&[(0x40000000, &[0; 17]), (0x60000000, &[0; 1])]);
    before.registers = vec![("x1".into(), 5), ("x0".into(), 0), ("old".into(), 1)];
    let mut after = snapshot(&[(0x40000000, &[1; 17]), (0x50010000, &[0; 1])]);
    before.memory[0].0.name = "[host malloc]".into();
    before.memory[1].0.name.clear();
    after.memory[0].0.name = "[host malloc]".into();
    after.registers = vec![("x0".into(), 7), ("x1".into(), 6), ("new".into(), 2)];
    let diff = before.diff(&after);
    assert_eq!(diff.registers, [("x0".into(), 0, 7), ("x1".into(), 5, 6)]);
    assert_eq!(
        diff.report(),
        concat!(
            "registers:\n  x0 0x0 -> 0x7\n  x1 0x5 -> 0x6\n",
            "mapped:\n  0x50010000-0x50010001 rw- [mmap]\n",
            "unmapped:\n  0x60000000-0x60000001 rw-\n",
            "changes:\n  0x40000000 [host malloc], 17 bytes\n",
            "    0000 - 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00\n",
            "    0000 + 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01\n",
            "    0010 - 00\n    0010 + 01"
        )
    );
    assert_eq!(
        after.diff(&after).report(),
        "registers:\n  (none)\nmapped:\n  (none)\nunmapped:\n  (none)\nchanges:\n  (none)"
    );
    assert!(serde_json::to_value(diff).unwrap()["changes"].is_array());
}
