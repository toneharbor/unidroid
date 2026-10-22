use super::{control::emulator, *};
use serde_json::json;

#[test]
fn vector_transfers_are_reported_as_ordered_eight_byte_accesses() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            [0x3dc00000, 0x3d800000, 0xd65f03c0]
        } else {
            [0xec900b04, 0xec800b04, 0xe12fff1e]
        };
        let (mut emu, code, events) = emulator(arm64, &words, false)?;
        let buffer = emu.malloc(16)?;
        let bytes = [0xfedcba9876543210u64, 0x123456789abcdef0]
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect::<Vec<_>>();
        emu.write_bytes(buffer, &bytes)?;
        emu.call_function(code, &[buffer])?;
        let captured = events.borrow();
        assert_eq!(captured.len(), 3);
        for (index, access) in ["read", "write"].into_iter().enumerate() {
            assert_eq!(
                captured[index]["memory"],
                json!([
                    {"access":access, "address":format!("0x{buffer:x}"), "size":8, "value":"0xfedcba9876543210"},
                    {"access":access, "address":format!("0x{:x}", buffer+8), "size":8, "value":"0x123456789abcdef0"}
                ])
            );
        }
    }
    Ok(())
}

#[test]
fn byte_stores_record_only_the_written_bits() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            [0x39000002, 0x39400001, 0xd65f03c0]
        } else {
            [0xe5c02000, 0xe5d01000, 0xe12fff1e]
        };
        let (mut emu, code, events) = emulator(arm64, &words, false)?;
        let buffer = emu.malloc(1)?;
        emu.call_function(code, &[buffer, 0, 0xfedcba98])?;
        let captured = events.borrow();
        assert_eq!(captured.len(), 3);
        for (index, access) in ["write", "read"].into_iter().enumerate() {
            assert_eq!(
                captured[index]["memory"],
                json!([
                    {"access":access, "address":format!("0x{buffer:x}"), "size":1, "value":"0x98"}
                ])
            );
        }
    }
    Ok(())
}

#[test]
fn memory_text_and_json_preserve_full_width_values() -> Result<()> {
    let event = Event::Instruction {
        at: "libfoo.so+0x1234".into(),
        address: 0x1234,
        bytes: "e00740f9".into(),
        text: "ldr x0, [sp, #8]".into(),
        read: vec![("sp".into(), 0x701ffe00)],
        written: vec![("x0".into(), 7)],
        memory: vec![
            unidroid::MemoryAccess {
                write: false,
                address: 0x701ffe08,
                size: 8,
                value: 7,
            },
            unidroid::MemoryAccess {
                write: true,
                address: 0xfedcba9876543210,
                size: 8,
                value: 0xfedcba9876543210,
            },
        ],
    };
    assert_eq!(
        event.to_string(),
        "[libfoo.so+0x1234] [e00740f9] ldr x0, [sp, #8] ; sp=0x701ffe00 => x0=0x7 | r 0x701ffe08:8=0x7, w 0xfedcba9876543210:8=0xfedcba9876543210"
    );
    assert_eq!(
        serde_json::to_value(event)?["memory"],
        json!([
            {"access":"read", "address":"0x701ffe08", "size":8, "value":"0x7"},
            {"access":"write", "address":"0xfedcba9876543210", "size":8, "value":"0xfedcba9876543210"}
        ])
    );
    Ok(())
}

#[test]
fn untraced_memory_is_excluded_even_when_the_range_ends_inside_an_instruction() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            [0xf9400001, 0xf9000002, 0xd65f03c0]
        } else {
            [0xe5901000, 0xe5802000, 0xe12fff1e]
        };
        let (mut emu, code, events) = emulator(arm64, &words, false)?;
        let buffer = emu.malloc(8)?;
        for end in [0x1002, 0x1004] {
            events.borrow_mut().clear();
            emu.write_bytes(buffer, &7u64.to_le_bytes())?;
            emu.set_trace(Trace {
                code: vec![format!("libc.so+0x1000-0x{end:x}")],
                ..Trace::default()
            })?;
            emu.call_function(code, &[buffer, 0, 42])?;
            let captured = events.borrow();
            assert_eq!(captured.len(), 1);
            assert_eq!(
                captured[0]["memory"],
                json!([{
                    "access":"read", "address":format!("0x{buffer:x}"),
                    "size":if arm64 {8} else {4}, "value":"0x7"
                }])
            );
        }
    }
    Ok(())
}

#[test]
fn a_faulting_pair_keeps_the_successful_read_before_the_fault() -> Result<()> {
    for arm64 in [true, false] {
        let (mut emu, code, events) =
            emulator(arm64, &[if arm64 { 0xa9401003 } else { 0xe8900018 }], false)?;
        emu.malloc(4096)?;
        let end = emu
            .maps()?
            .iter()
            .filter(|m| m.name == "[host malloc]")
            .map(|m| m.end)
            .max()
            .unwrap();
        let size = if arm64 { 8 } else { 4 };
        emu.write_bytes(end - size, &7u64.to_le_bytes()[..size as usize])?;
        assert!(emu.call_function(code, &[end - size]).is_err());
        let captured = events.borrow();
        assert_eq!(captured.len(), 1, "arm64={arm64}: {captured:?}");
        assert_eq!(captured[0]["written"], json!([]));
        assert_eq!(
            captured[0]["memory"],
            json!([{
                "access":"read", "address":format!("0x{:x}", end-size), "size":size, "value":"0x7"
            }])
        );
    }
    Ok(())
}

#[test]
fn tracing_before_initialization_keeps_later_thumb_blocks_unconditional() -> Result<()> {
    for arm64 in [true, false] {
        let ret = if arm64 { 0xd65f03c0 } else { 0xe12fff1e };
        let (mut emu, code, _) = emulator(arm64, &[ret], true)?;
        let data = emu.malloc(4)?;
        for load in [true, false] {
            let bytes = if arm64 {
                [
                    if load { 0xb9400022u32 } else { 0xb9000022 },
                    0x14000002,
                    0xd503201f,
                    0xd2800540,
                    0xd65f03c0,
                ]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>()
            } else {
                // cmp r0,#0; it eq; ldr/str r2,[r1]; cmp r0,#1;
                // b target; nop; target: movs r0,#42; bx lr.
                [
                    0x2800u16,
                    0xbf08,
                    if load { 0x680a } else { 0x600a },
                    0x2801,
                    0xe000,
                    0xbf00,
                    0x202a,
                    0x4770,
                ]
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>()
            };
            emu.write_bytes(code, &bytes)?;
            assert_eq!(
                emu.call_function(code | u64::from(!arm64), &[0, data, 7])?,
                42
            );
        }
    }
    Ok(())
}

#[test]
fn loads_and_stores_keep_values_at_access_time_and_pairs_stay_in_order() -> Result<()> {
    for arm64 in [true, false] {
        // ldr r/x1,[r/x0]; str r/x2,[r/x0]; ldp/stp or ldm/stm; return.
        let words = if arm64 {
            [0xf9400001, 0xf9000002, 0xa9401003, 0xa9001003, 0xd65f03c0]
        } else {
            [0xe5901000, 0xe5802000, 0xe8900018, 0xe8800018, 0xe12fff1e]
        };
        let (mut emu, code, events) = emulator(arm64, &words, false)?;
        let size = if arm64 { 8 } else { 4 };
        let buffer = emu.malloc(size * 2)?;
        emu.write_bytes(buffer, &7u64.to_le_bytes()[..size])?;
        emu.write_bytes(buffer + size as u64, &9u64.to_le_bytes()[..size])?;
        emu.call_function(code, &[buffer, 0, 42])?;
        let access = |kind, offset, value| {
            json!({
                "access":kind, "address":format!("0x{:x}", buffer + offset),
                "size":size, "value":value
            })
        };
        let captured = events.borrow();
        assert_eq!(captured.len(), 5);
        assert_eq!(captured[0]["memory"], json!([access("read", 0, "0x7")]));
        assert_eq!(captured[1]["memory"], json!([access("write", 0, "0x2a")]));
        assert_eq!(
            captured[2]["memory"],
            json!([
                access("read", 0, "0x2a"),
                access("read", size as u64, "0x9")
            ])
        );
        assert_eq!(
            captured[3]["memory"],
            json!([
                access("write", 0, "0x2a"),
                access("write", size as u64, "0x9")
            ])
        );
        assert_eq!(captured[4]["memory"], json!([]));
    }
    Ok(())
}
