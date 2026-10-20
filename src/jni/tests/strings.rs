use super::*;
use crate::jni::env::strings::{decode_modified_utf8, modified_utf8};

fn string_chars_use_libc_heap(get: u64, release: u64, expected: &[u8]) -> Result<()> {
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        let (mut emu, libc) = libc_emulator(arch)?;
        let string = emu.new_string("a\0é😀");
        let copy = emu.malloc(1)?;
        let ptr = jni_call(&mut emu, get, &[string.0, copy])?;
        assert_eq!(emu.read_bytes(ptr, expected.len())?, expected);
        assert_eq!(emu.read_bytes(copy, 1)?, [1]);
        let usable = emu.find_symbol(libc, "malloc_usable_size")?;
        assert!(emu.call_function(usable, &[ptr])? >= expected.len() as u64);
        jni_call(&mut emu, release, &[string.0, ptr])?;
        let malloc = emu.find_symbol(libc, "malloc")?;
        assert_eq!(emu.call_function(malloc, &[expected.len() as u64])?, ptr);
        let free = emu.find_symbol(libc, "free")?;
        emu.call_function(free, &[ptr])?;
    }
    Ok(())
}

#[test]
fn get_string_utf_chars_uses_libc_heap() -> Result<()> {
    string_chars_use_libc_heap(169, 170, b"a\xc0\x80\xc3\xa9\xed\xa0\xbd\xed\xb8\x80\0")
}

#[test]
fn get_string_chars_uses_libc_heap() -> Result<()> {
    string_chars_use_libc_heap(165, 166, &[0x61, 0, 0, 0, 0xe9, 0, 0x3d, 0xd8, 0, 0xde])
}

#[test]
fn get_string_critical_uses_libc_heap() -> Result<()> {
    string_chars_use_libc_heap(224, 225, &[0x61, 0, 0, 0, 0xe9, 0, 0x3d, 0xd8, 0, 0xde])
}

#[test]
fn native_free_accepts_string_utf_chars() -> Result<()> {
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        let (mut emu, libc) = libc_emulator(arch)?;
        let string = emu.new_string("359250054370919||1551086094");
        let ptr = jni_call(&mut emu, 169, &[string.0, 0])?;
        let free = emu.find_symbol(libc, "free")?;
        emu.call_function(free, &[ptr])?;
        let malloc = emu.find_symbol(libc, "malloc")?;
        let next = emu.call_function(malloc, &[26])?;
        assert_eq!(next, ptr);
        emu.write_bytes(next, &[0x42; 26])?;
        assert_eq!(emu.read_bytes(next, 26)?, [0x42; 26]);
        emu.call_function(free, &[next])?;
    }
    Ok(())
}

#[test]
fn new_string_utf_decodes_modified_utf8() -> Result<()> {
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        let (mut emu, _) = libc_emulator(arch)?;
        let bytes = b"a\xc0\x80\xc3\xa9\xed\xa0\xbd\xed\xb8\x80\0";
        let ptr = emu.malloc(bytes.len())?;
        emu.write_bytes(ptr, bytes)?;
        let string = jni_call(&mut emu, 167, &[ptr])?;
        assert_eq!(emu.string(Handle(string))?, "a\0é😀");
        emu.free(ptr);
    }
    Ok(())
}

#[test]
fn jni_buffers_require_guest_malloc_and_free() -> Result<()> {
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(arch)?;
        let string = emu.new_string("hello");
        let array = emu.new_byte_array(&[1]);
        for (get, release, handle) in [
            (165, 166, string),
            (169, 170, string),
            (224, 225, string),
            (184, 192, array),
            (222, 223, array),
        ] {
            assert_eq!(
                jni_call(&mut emu, get, &[handle.0, 0])
                    .unwrap_err()
                    .to_string(),
                "JNI buffer requires malloc: no loaded library exports it"
            );
            assert_eq!(
                jni_call(&mut emu, release, &[handle.0, 0, 2])
                    .unwrap_err()
                    .to_string(),
                "JNI buffer requires free: no loaded library exports it"
            );
        }
    }
    Ok(())
}

#[test]
fn string_regions_use_a_fifth_argument_and_signed_jni_lengths() -> Result<()> {
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(arch)?;
        let table = emu.read_pointer(emu.jni.env)?;
        let string = emu.new_string("aéz");
        let out = emu.malloc(8)?;
        for (index, expected) in [(220, [0xe9, 0]), (221, [0xc3, 0xa9])] {
            emu.write_bytes(out, &[0xa5; 8])?;
            let function = emu.read_pointer(table + index * emu.pointer_size())?;
            emu.call_function(function, &[emu.jni.env, string.0, 1, 1, out])?;
            assert_eq!(emu.read_bytes(out, 2)?, expected);
            assert_eq!(emu.read_bytes(out + 2, 6)?, [0xa5; 6]);
        }
        let new_string = emu.read_pointer(table + 163 * emu.pointer_size())?;
        assert_eq!(
            emu.call_function(new_string, &[emu.jni.env, 0, u64::MAX])?,
            0
        );
        let check = emu.read_pointer(table + 228 * emu.pointer_size())?;
        assert_eq!(emu.call_function(check, &[emu.jni.env])?, 1);
    }
    Ok(())
}

#[test]
fn modified_utf8_preserves_nul_and_surrogate_pairs() {
    let units = [0, 0x41, 0xd83d, 0xde00];
    let expected = [0xc0, 0x80, 0x41, 0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80];
    assert_eq!(modified_utf8(&units), expected);
    assert_eq!(decode_modified_utf8(&expected), units);
}
