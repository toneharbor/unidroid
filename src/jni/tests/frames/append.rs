use super::*;

// (JNIEnv*, builder, append method ID, object): append 1,000 times and delete each result.
pub(super) fn append_loop(emu: &mut Emulator) -> Result<u64> {
    let mut words = match emu.architecture() {
        Architecture::Arm32 => vec![
            0xe92d47f0, // push {r4-r10,lr}
            0xe1a04000, // mov r4,r0
            0xe1a05001, // mov r5,r1
            0xe1a06002, // mov r6,r2
            0xe1a07003, // mov r7,r3
            0xe30083e8, // movw r8,#1000
            0xe5949000, // ldr r9,[r4]
        ],
        Architecture::Arm64 => vec![
            0xa9bc53f3, // stp x19,x20,[sp,#-64]!
            0xa9015bf5, // stp x21,x22,[sp,#16]
            0xa90263f7, // stp x23,x24,[sp,#32]
            0xa9037bf9, // stp x25,x30,[sp,#48]
            0xaa0003f3, // mov x19,x0
            0xaa0103f4, // mov x20,x1
            0xaa0203f5, // mov x21,x2
            0xaa0303f6, // mov x22,x3
            0xd2807d17, // mov x23,#1000
            0xf9400278, // ldr x24,[x19]
        ],
    };
    let start = words.len() as i32;
    match emu.architecture() {
        Architecture::Arm32 => {
            words.extend([
                0xe1a00004, // loop: mov r0,r4
                0xe1a01005, // mov r1,r5
                0xe1a02006, // mov r2,r6
                0xe1a03007, // mov r3,r7
                0xe599a088, // ldr r10,[r9,#34*4]: CallObjectMethod
                0xe12fff3a, // blx r10
                0xe1a01000, // mov r1,r0
                0xe1a00004, // mov r0,r4
                0xe599a05c, // ldr r10,[r9,#23*4]: DeleteLocalRef
                0xe12fff3a, // blx r10
                0xe2588001, // subs r8,r8,#1
            ]);
            let offset = start - words.len() as i32 - 2;
            words.push(0x1a000000 | (offset as u32 & 0x00ffffff)); // bne loop
        }
        Architecture::Arm64 => {
            words.extend([
                0xaa1303e0, // loop: mov x0,x19
                0xaa1403e1, // mov x1,x20
                0xaa1503e2, // mov x2,x21
                0xaa1603e3, // mov x3,x22
                0xf9408b19, // ldr x25,[x24,#34*8]: CallObjectMethod
                0xd63f0320, // blr x25
                0xaa0003e1, // mov x1,x0
                0xaa1303e0, // mov x0,x19
                0xf9405f19, // ldr x25,[x24,#23*8]: DeleteLocalRef
                0xd63f0320, // blr x25
                0xf10006f7, // subs x23,x23,#1
            ]);
            let offset = start - words.len() as i32;
            words.push(0x54000001 | ((offset as u32 & 0x7ffff) << 5)); // b.ne loop
        }
    }
    let stop = CODE + words.len() as u64 * 4;
    match emu.architecture() {
        Architecture::Arm32 => words.extend([0xe3a00000, 0xe8bd87f0]), // mov r0,#0; pop {r4-r10,pc}
        Architecture::Arm64 => words.extend([
            0xd2800000, // mov x0,#0
            0xa9415bf5, // ldp x21,x22,[sp,#16]
            0xa94263f7, // ldp x23,x24,[sp,#32]
            0xa9437bf9, // ldp x25,x30,[sp,#48]
            0xa8c453f3, // ldp x19,x20,[sp],#64
            0xd65f03c0, // ret
        ]),
    }
    write_code(emu, &words)?;
    Ok(stop)
}

#[test]
fn append_builtin_objects_does_not_grow_the_local_frame() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let stop = append_loop(&mut emu)?;
        emu.add_breakpoint(stop)?;
        let class = emu.class("java/lang/StringBuilder");
        let mut methods = Vec::new();
        for (name, signature) in [
            ("<init>", "()V"),
            ("append", "(Ljava/lang/Object;)Ljava/lang/StringBuilder;"),
            ("toString", "()Ljava/lang/String;"),
        ] {
            let name = emu.new_cstring(name)?;
            let signature = emu.new_cstring(signature)?;
            methods.push(jni_call(&mut emu, 33, &[class.0, name, signature])?);
        }
        let string = emu.new_string("item");
        let list = emu.new_java_list("java/util/ArrayList", &[string])?;
        for (object, expected) in [(list, "[item]"), (string, "item"), (Handle(0), "null")] {
            let builder = jni_call(&mut emu, 28, &[class.0, methods[0]])?;
            let before = emu.jni.objects.len();
            let mut handler = CountAtStop::default();
            assert_eq!(
                emu.call_function_with(
                    &mut handler,
                    CODE,
                    &[emu.jni.env, builder, methods[1], object.0]
                )?,
                0
            );
            assert_eq!(handler.0, Some(before), "{arch:?}: {expected}");
            let text = Handle(jni_call(&mut emu, 34, &[builder, methods[2]])?);
            assert_eq!(emu.string(text)?, expected.repeat(1000));
        }
    }
    Ok(())
}
