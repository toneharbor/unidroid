use super::*;

fn array_elements_use_libc_heap(get: u64, release: u64, kind: PrimitiveType) -> Result<()> {
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        let (mut emu, libc) = libc_emulator(arch)?;
        let malloc = emu.find_symbol(libc, "malloc")?;
        let free = emu.find_symbol(libc, "free")?;
        let usable = emu.find_symbol(libc, "malloc_usable_size")?;
        let copy = emu.malloc(1)?;
        let length = 3 * kind.size();
        for mode in [0, 1, 2] {
            let array = emu.new_primitive_array(kind, &vec![1; length]);
            let ptr = jni_call(&mut emu, get, &[array.0, copy])?;
            assert_eq!(emu.read_bytes(ptr, length)?, vec![1; length]);
            assert_eq!(emu.read_bytes(copy, 1)?, [1]);
            assert!(emu.call_function(usable, &[ptr])? >= length as u64);
            emu.write_bytes(ptr, &vec![0; length])?;
            jni_call(&mut emu, release, &[array.0, ptr, mode])?;
            assert_eq!(
                emu.primitive_array(array)?,
                vec![u8::from(mode == 2); length]
            );
            if mode == 1 {
                let other = emu.call_function(malloc, &[length as u64])?;
                assert_ne!(other, ptr);
                emu.write_bytes(ptr, &vec![1; length])?;
                jni_call(&mut emu, release, &[array.0, ptr, 0])?;
                assert_eq!(emu.primitive_array(array)?, vec![1; length]);
                assert_eq!(emu.call_function(malloc, &[length as u64])?, ptr);
                emu.call_function(free, &[other])?;
            } else {
                assert_eq!(emu.call_function(malloc, &[length as u64])?, ptr);
            }
            emu.call_function(free, &[ptr])?;
            emu.release(array);
        }
    }
    Ok(())
}

macro_rules! array_heap_test {
    ($name:ident, $get:expr, $release:expr, $kind:ident) => {
        #[test]
        fn $name() -> Result<()> {
            array_elements_use_libc_heap($get, $release, PrimitiveType::$kind)
        }
    };
}
array_heap_test!(get_boolean_array_elements_uses_libc_heap, 183, 191, Boolean);
array_heap_test!(get_byte_array_elements_uses_libc_heap, 184, 192, Byte);
array_heap_test!(get_char_array_elements_uses_libc_heap, 185, 193, Char);
array_heap_test!(get_short_array_elements_uses_libc_heap, 186, 194, Short);
array_heap_test!(get_int_array_elements_uses_libc_heap, 187, 195, Int);
array_heap_test!(get_long_array_elements_uses_libc_heap, 188, 196, Long);
array_heap_test!(get_float_array_elements_uses_libc_heap, 189, 197, Float);
array_heap_test!(get_double_array_elements_uses_libc_heap, 190, 198, Double);
array_heap_test!(get_primitive_array_critical_uses_libc_heap, 222, 223, Int);

#[test]
fn primitive_arrays_commit_abort_and_regions_use_element_width() -> Result<()> {
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        let (mut emu, _) = libc_emulator(arch)?;
        let p = emu.pointer_size();
        let table = emu.read_pointer(emu.jni.env)?;
        for (index, kind) in ARRAY_TYPES.iter().enumerate() {
            let width = kind.size();
            let array = emu.new_primitive_array(*kind, &vec![0; 3 * width]);
            let get = emu.read_pointer(table + (183 + index as u64) * p)?;
            let release = emu.read_pointer(table + (191 + index as u64) * p)?;
            let ptr = emu.call_function(get, &[emu.jni.env, array.0, 0])?;
            emu.write_bytes(ptr, &vec![0x42; 3 * width])?;
            emu.call_function(release, &[emu.jni.env, array.0, ptr, 1])?;
            assert_eq!(emu.primitive_array(array)?, vec![0x42; 3 * width]);
            emu.write_bytes(ptr, &vec![0x55; 3 * width])?;
            emu.call_function(release, &[emu.jni.env, array.0, ptr, 2])?;
            assert_eq!(emu.primitive_array(array)?, vec![0x42; 3 * width]);
            let set_region = emu.read_pointer(table + (207 + index as u64) * p)?;
            let region = emu.malloc(width)?;
            emu.write_bytes(region, &vec![0x55; width])?;
            emu.call_function(set_region, &[emu.jni.env, array.0, 1, 1, region])?;
            emu.free(region);
            let mut expected = vec![0x42; 3 * width];
            expected[width..2 * width].fill(0x55);
            assert_eq!(emu.primitive_array(array)?, expected);
            let get_region = emu.read_pointer(table + (199 + index as u64) * p)?;
            let out = emu.malloc(width + 4)?;
            emu.write_bytes(out, &vec![0xa5; width + 4])?;
            emu.call_function(get_region, &[emu.jni.env, array.0, 1, 1, out])?;
            assert_eq!(emu.read_bytes(out, width)?, vec![0x55; width]);
            assert_eq!(emu.read_bytes(out + width as u64, 4)?, [0xa5; 4]);
            let ptr = emu.call_function(get, &[emu.jni.env, array.0, 0])?;
            emu.write_bytes(ptr, &vec![0x66; 3 * width])?;
            emu.call_function(release, &[emu.jni.env, array.0, ptr, 0])?;
            assert_eq!(emu.primitive_array(array)?, vec![0x66; 3 * width]);
        }
    }
    Ok(())
}
