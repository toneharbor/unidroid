use super::*;

#[test]
fn get_superclass_uses_declarations_and_requires_unknown_parents() -> Result<()> {
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(arch)?;
        emu.superclass("example/Child", "example/Old");
        emu.superclass("example/Child", "example/Parent");
        let child = emu.class("example/Child");
        let parent = jni_call(&mut emu, 10, &[child.0])?;
        assert_eq!(emu.class_name(Handle(parent))?, "example/Parent");
        let object = emu.class("java/lang/Object");
        assert_eq!(jni_call(&mut emu, 10, &[object.0])?, 0);
        emu.superclass("example/Loop", "example/Loop");
        let looping = emu.class("example/Loop");
        let superclass = jni_call(&mut emu, 10, &[looping.0])?;
        assert_eq!(jni_call(&mut emu, 24, &[superclass, looping.0])?, 1);
        let unknown = emu.class("example/Unknown");
        assert_eq!(
            jni_call(&mut emu, 10, &[unknown.0])
                .unwrap_err()
                .to_string(),
            "GetSuperclass example/Unknown from 0x20000: superclass not declared; declare it with Emulator::superclass, --superclass, or the serve command superclass"
        );
        let string = emu.new_string("wrong kind");
        assert_eq!(
            jni_call(&mut emu, 10, &[string.0]).unwrap_err().to_string(),
            format!(
                "invalid JNI reference 0x{:x} in GetSuperclass from 0x20000",
                string.0
            )
        );
    }
    Ok(())
}

#[test]
fn declared_superclasses_answer_assignability_without_a_handler() -> Result<()> {
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(arch)?;
        emu.superclass("com/example/Child", "com/example/Parent");
        emu.superclass("com/example/Parent", "com/example/Base");
        let child = emu.class("com/example/Child");
        let parent = emu.class("com/example/Base");
        assert_eq!(jni_call(&mut emu, 11, &[child.0, parent.0])?, 1);
        let instance = emu.new_object("com/example/Child", ());
        assert_eq!(jni_call(&mut emu, 32, &[instance.0, parent.0])?, 1);
        assert!(jni_call(&mut emu, 11, &[parent.0, child.0]).is_err());
    }
    Ok(())
}

#[test]
fn inherited_registered_natives_receive_the_declaring_class_or_receiver() -> Result<()> {
    for arch in [crate::Architecture::Arm32, crate::Architecture::Arm64] {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(arch)?;
        let code = crate::RETURN + 0x100;
        let words = match arch {
            crate::Architecture::Arm32 => [0xe1a00001u32, 0xe12fff1e], // mov r0,r1; bx lr
            crate::Architecture::Arm64 => [0xaa0103e0, 0xd65f03c0],    // mov x0,x1; ret
        };
        emu.write_bytes(
            code,
            &words
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )?;
        let declaring = emu.class("com/example/Base");
        let entry = emu.malloc(3 * emu.pointer_size() as usize)?;
        let name = emu.new_cstring("a")?;
        let signature = emu.new_cstring("()Ljava/lang/Class;")?;
        emu.write_pointer(entry, name)?;
        emu.write_pointer(entry + emu.pointer_size(), signature)?;
        emu.write_pointer(entry + 2 * emu.pointer_size(), code)?;
        jni_call(&mut emu, 215, &[declaring.0, entry, 1])?;
        let class = "com/example/Child";
        let method = "a()Ljava/lang/Class;";
        assert_eq!(
            emu.call_static_jni_method(&mut (), class, method, &[])
                .unwrap_err()
                .to_string(),
            "native method com/example/Child.a()Ljava/lang/Class; not found: no RegisterNatives entry or Java_ export on com/example/Child (an inherited method needs its superclasses declared)"
        );
        emu.superclass(class, "com/example/Parent");
        emu.superclass("com/example/Parent", "com/example/Base");
        assert_eq!(
            emu.call_static_jni_method(&mut (), class, method, &[])?
                .object(),
            declaring
        );
        let receiver = emu.new_object(class, ());
        assert_eq!(
            emu.call_jni_method(&mut (), class, receiver, method, &[])?
                .object(),
            receiver
        );
        emu.superclass("A", "B");
        emu.superclass("B", "A");
        assert_eq!(
            emu.call_static_jni_method(&mut (), "A", "missing()V", &[])
                .unwrap_err()
                .to_string(),
            "native method A.missing()V not found: no RegisterNatives entry or Java_ export on A, B (an inherited method needs its superclasses declared)"
        );
    }
    Ok(())
}

#[test]
fn inherited_java_exports_precede_ancestor_registrations_and_use_short_names_first() -> Result<()> {
    let library = crate::test_elf::TestLibrary::new(true, false);
    let mut bytes = std::fs::read(&library.0)?;
    let names = b"Java_example_Parent_a\0Java_example_Parent_a__\0";
    bytes[0x1800..0x1800 + names.len()].copy_from_slice(names);
    bytes[0x1218..0x121c].copy_from_slice(&0x700u32.to_le_bytes());
    bytes[0x1230..0x1234].copy_from_slice(&0x716u32.to_le_bytes());
    bytes[0x1038..0x1040].copy_from_slice(&0x800u64.to_le_bytes());
    bytes[0x200..0x204].copy_from_slice(&0xaa0103e0u32.to_le_bytes()); // mov x0,x1
    std::fs::write(&library.0, &bytes)?;
    let mut emu = Emulator::builder().build()?;
    emu.load_library(&library.0)?;
    emu.superclass("example/Child", "example/Parent");
    emu.superclass("example/Parent", "example/Grandparent");
    let parent = emu.class("example/Parent");
    let grandparent = emu.class("example/Grandparent");
    let entry = emu.malloc(24)?;
    let name = emu.new_cstring("a")?;
    let signature = emu.new_cstring("()Ljava/lang/Class;")?;
    emu.write_pointer(entry, name)?;
    emu.write_pointer(entry + 8, signature)?;
    emu.write_pointer(entry + 16, crate::RETURN + 0x100)?;
    jni_call(&mut emu, 215, &[grandparent.0, entry, 1])?;
    assert_eq!(
        emu.call_static_jni_method(&mut (), "example/Child", "a()Ljava/lang/Class;", &[])?
            .object(),
        parent
    );
    // Removing the short name leaves the long Java_ export on the same class.
    bytes[0x1218..0x121c].copy_from_slice(&1u32.to_le_bytes());
    bytes[0x1238..0x1240].copy_from_slice(&0x200u64.to_le_bytes());
    let long_library = crate::test_elf::TestLibrary::new(true, false);
    std::fs::write(&long_library.0, bytes)?;
    let mut emu = Emulator::builder().build()?;
    emu.load_library(&long_library.0)?;
    emu.superclass("example/Child", "example/Parent");
    let parent = emu.class("example/Parent");
    assert_eq!(
        emu.call_static_jni_method(&mut (), "example/Child", "a()Ljava/lang/Class;", &[])?
            .object(),
        parent
    );
    Ok(())
}

#[test]
fn arm32_vm_pointers_write_four_bytes_and_native_entries_keep_thumb_bits() -> Result<()> {
    let mut emu = Emulator::builder().build()?;
    emu.setup_cpu(crate::Architecture::Arm32)?;
    let env_table = emu.read_pointer(emu.jni.env)?;
    let vm_table = emu.read_pointer(emu.jni.vm)?;
    let out = emu.malloc(8)?;
    for index in [4, 6, 7] {
        emu.write_bytes(out, &[0xa5; 8])?;
        let function = emu.read_pointer(vm_table + index * 4)?;
        assert_eq!(emu.call_function(function, &[emu.jni.vm, out, 0x10006])?, 0);
        assert_eq!(emu.read_pointer(out)?, emu.jni.env);
        assert_eq!(emu.read_bytes(out + 4, 4)?, [0xa5; 4]);
    }
    emu.write_bytes(out, &[0xa5; 8])?;
    let get_vm = emu.read_pointer(env_table + 219 * 4)?;
    assert_eq!(emu.call_function(get_vm, &[emu.jni.env, out])?, 0);
    assert_eq!(emu.read_pointer(out)?, emu.jni.vm);
    assert_eq!(emu.read_bytes(out + 4, 4)?, [0xa5; 4]);

    let class = emu.class("example/Native");
    let entries = emu.malloc(24)?;
    let sig = emu.new_cstring("()I")?;
    let code = crate::RETURN + 0x100;
    // Thumb: movs r0,#42; bx lr; movs r0,#7; bx lr.
    emu.write_bytes(code, &[0x2a, 0x20, 0x70, 0x47, 7, 0x20, 0x70, 0x47])?;
    for (i, name) in ["one", "two"].into_iter().enumerate() {
        let name = emu.new_cstring(name)?;
        let entry = entries + i as u64 * 12;
        emu.write_pointer(entry, name)?;
        emu.write_pointer(entry + 4, sig)?;
        emu.write_pointer(entry + 8, (code + i as u64 * 4) | 1)?;
    }
    let register = emu.read_pointer(env_table + 215 * 4)?;
    assert_eq!(
        emu.call_function(register, &[emu.jni.env, class.0, entries, 2])?,
        0
    );
    assert_eq!(
        emu.call_static_jni_method(&mut (), "example/Native", "one()I", &[])?
            .int(),
        42
    );
    assert_eq!(
        emu.call_static_jni_method(&mut (), "example/Native", "two()I", &[])?
            .int(),
        7
    );
    Ok(())
}
