use super::*;
use unidroid::{HeapObject, Target};

fn expanded(object: &HeapObject, index: usize) -> Option<&HeapObject> {
    match object.fields[index].target.as_ref().unwrap() {
        Target::Block { object, .. } => object.as_deref(),
        _ => panic!("expected a block target"),
    }
}

#[test]
fn fields_strings_referrers_and_cycles_obey_depth_and_expand_once() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        caller(&mut emu, arm64)?;
        let width = if arm64 { 8 } else { 4 };
        let a = guest(&mut emu, "malloc", &[3 * width + 1])?;
        let b = guest(&mut emu, "calloc", &[1, 80])?;
        let malloc = emu.resolve("malloc")?;
        write_pointer(&mut emu, a, b, arm64)?;
        write_pointer(&mut emu, a + width, b + 16, arm64)?;
        write_pointer(&mut emu, a + 2 * width, malloc, arm64)?;
        emu.write_bytes(b + 16, b"hello object\0")?;
        let stack = emu.register("sp")? - 0x100;
        write_pointer(&mut emu, stack, b + 3, arm64)?;
        let object = emu.object(a, 1)?;
        assert_eq!(object.fields.len(), 3);
        assert_eq!(object.fields[1].string.as_deref(), Some("hello object"));
        assert_eq!(expanded(&object, 0).unwrap().address, b);
        assert!(expanded(&object, 1).is_none());
        match object.fields[2].target.as_ref().unwrap() {
            Target::Location { location } => assert!(location.contains("malloc")),
            _ => panic!("expected a symbolized location"),
        }
        let object = emu.object(b + 17, 1)?;
        assert_eq!((object.address, object.size), (b, 80));
        let reference = object.referrers.iter().find(|r| r.address == a).unwrap();
        assert_eq!(reference.block, Some(a));
        assert_eq!(reference.object.as_deref().unwrap().address, a);
        assert!(
            object
                .referrers
                .iter()
                .any(|r| r.address == stack && r.block.is_none() && r.location.contains("[stack]"))
        );
        assert!(
            object
                .referrers
                .windows(2)
                .all(|r| r[0].address < r[1].address)
        );
        let shallow = emu.object(a, 0)?;
        assert!(expanded(&shallow, 0).is_none());
        assert!(shallow.referrers.iter().all(|r| r.object.is_none()));
        write_pointer(&mut emu, a + 2 * width, 0x20000, arm64)?;
        match emu.object(a, 0)?.fields[2].target.as_ref().unwrap() {
            Target::Location { location } => assert_eq!(location, "0x20000 [trampoline]"),
            _ => panic!("expected a mapping location"),
        }
        let c = guest(&mut emu, "calloc", &[1, width])?;
        write_pointer(&mut emu, b, c, arm64)?;
        let one = emu.object(a, 1)?;
        assert!(expanded(expanded(&one, 0).unwrap(), 0).is_none());
        let two = emu.object(a, 2)?;
        assert_eq!(expanded(expanded(&two, 0).unwrap(), 0).unwrap().address, c);
        write_pointer(&mut emu, b, a, arm64)?;
        let cyclic = emu.object(a, usize::MAX)?;
        assert!(expanded(expanded(&cyclic, 0).unwrap(), 0).is_none());
        guest(&mut emu, "free", &[b])?;
        assert!(
            emu.object(b, 0)
                .unwrap_err()
                .to_string()
                .contains("is not inside a known allocation")
        );
        assert_eq!(
            emu.object(0xdeadbeef, 0).unwrap_err().to_string(),
            "0xdeadbeef is not inside a known allocation"
        );
    }
    Ok(())
}

#[test]
fn string_detection_requires_printable_ascii_and_a_nul_within_64_bytes() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        let width = if arm64 { 8 } else { 4 };
        let fields = emu.malloc(6 * width)?;
        let cases = [
            (b"abcd\0".to_vec(), Some("abcd".to_owned())),
            (b"abc\0".to_vec(), None),
            (b"abc\n\0".to_vec(), None),
            (b"abcd\xff\0".to_vec(), None),
            ([vec![b'x'; 63], vec![0]].concat(), Some("x".repeat(63))),
            ([vec![b'x'; 64], vec![0]].concat(), None),
        ];
        for (index, (bytes, _)) in cases.iter().enumerate() {
            let string = emu.malloc(bytes.len())?;
            emu.write_bytes(string, bytes)?;
            write_pointer(&mut emu, fields + (index * width) as u64, string, arm64)?;
        }
        let object = emu.object(fields, 0)?;
        for (field, (_, expected)) in object.fields.iter().zip(cases) {
            assert_eq!(field.string, expected);
        }
    }
    Ok(())
}

#[test]
fn host_objects_and_strings_cross_mapping_boundaries_and_limit_referrers() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        emu.set_trace(Trace::default())?;
        let block = emu.malloc(3 * 4096)?;
        emu.write_bytes(block, &vec![0; 3 * 4096])?;
        let boundary = (block + 4095) & !4095;
        let boundary = boundary + 4096;
        emu.write_bytes(boundary - 4, b"cross pages\0")?;
        write_pointer(&mut emu, block, boundary - 4, arm64)?;
        let mprotect = emu.resolve("mprotect")?;
        assert_eq!(emu.call_function(mprotect, &[boundary, 4096, 1])?, 0);
        let refs = emu.malloc(8 * 1100)?;
        for index in 0..1100 {
            write_pointer(
                &mut emu,
                refs + index * if arm64 { 8 } else { 4 },
                block + 2,
                arm64,
            )?;
        }
        let object = emu.object(block + 7, 0)?;
        assert_eq!(object.allocation.function, "host");
        assert_eq!(object.fields[0].string.as_deref(), Some("cross pages"));
        assert_eq!(object.fields.len(), 3 * 4096 / if arm64 { 8 } else { 4 });
        assert_eq!(object.referrers.len(), unidroid::SEARCH_LIMIT);
        assert!(
            object
                .referrers
                .windows(2)
                .all(|r| r[0].address < r[1].address)
        );
        assert!(
            object
                .referrers
                .iter()
                .all(|r| r.address < block || r.address >= block + 3 * 4096)
        );
    }
    Ok(())
}
