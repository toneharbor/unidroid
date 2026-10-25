use super::*;

fn stopped_native(emu: &mut Emulator) -> Result<()> {
    let ret = match emu.architecture() {
        Architecture::Arm32 => 0xe12fff1e, // bx lr
        Architecture::Arm64 => 0xd65f03c0, // ret
    };
    write_code(emu, &[ret])?;
    emu.add_breakpoint(CODE)?;
    register_native(emu, "()Ljava/lang/Object;", CODE)
}

struct ExceptionResult {
    class: Handle,
    message: u64,
    exception_result: bool,
    abort: bool,
}
impl JniHandler for ExceptionResult {
    fn stopped(&mut self, emu: &mut Emulator, _: &Stop) -> Resume {
        jni_call(emu, 14, &[self.class.0, self.message]).unwrap();
        let result = if self.exception_result {
            jni_call(emu, 15, &[]).unwrap()
        } else {
            jni_call(emu, 167, &[self.message]).unwrap()
        };
        emu.write_reg(0, result).unwrap();
        if self.abort {
            Resume::Abort
        } else {
            Resume::Continue
        }
    }
}

#[test]
fn pending_exceptions_discard_native_results_and_win_over_invoke_errors() -> Result<()> {
    for arch in ARCHES {
        for exception_result in [false, true] {
            for abort in [false, true] {
                let mut emu = emulator(arch)?;
                stopped_native(&mut emu)?;
                let mut handler = ExceptionResult {
                    class: emu.class("java/lang/IllegalStateException"),
                    message: emu.new_cstring("failed")?,
                    exception_result,
                    abort,
                };
                let before = emu.jni.objects.len();
                for _ in 0..2 {
                    let error = emu
                        .call_static_jni_method(
                            &mut handler,
                            "example/Frame",
                            "run()Ljava/lang/Object;",
                            &[],
                        )
                        .unwrap_err()
                        .to_string();
                    assert_eq!(
                        error,
                        "java exception thrown by native code: java/lang/IllegalStateException: failed from 0x20000"
                    );
                    assert_eq!(
                        emu.jni.objects.len(),
                        before,
                        "{arch:?}, exception result {exception_result}, abort {abort}"
                    );
                    assert_eq!(jni_call(&mut emu, 228, &[])?, 0);
                    assert_eq!(emu.jni.scope, 0);
                }
            }
        }
    }
    Ok(())
}

struct ConstructorResult {
    text: u64,
    calls: usize,
}
impl JniHandler for ConstructorResult {
    fn stopped(&mut self, emu: &mut Emulator, _: &Stop) -> Resume {
        let local = jni_call(emu, 167, &[self.text]).unwrap();
        emu.write_reg(0, local).unwrap();
        self.calls += 1;
        Resume::Continue
    }
}

#[test]
fn constructors_discard_local_handles_left_in_return_registers() -> Result<()> {
    for arch in ARCHES {
        let library = crate::test_elf::TestLibrary::new(arch == Architecture::Arm64, false);
        let mut bytes = std::fs::read(&library.0)?;
        let width = arch.pointer_size() as usize;
        // Add DT_INIT alongside the synthetic ELF's existing DT_INIT_ARRAY.
        for (index, value) in [(20, 12u64), (21, 0x220)] {
            let offset = 0x1000 + index * width;
            bytes[offset..offset + width].copy_from_slice(&value.to_le_bytes()[..width]);
        }
        std::fs::write(&library.0, bytes)?;
        let mut emu = Emulator::builder().build()?;
        let module = emu.load_library_without_constructors(&library.0)?;
        let ret = emu.modules[module.0].base
            + match arch {
                Architecture::Arm32 => 0x230,
                Architecture::Arm64 => 0x22c,
            };
        emu.add_breakpoint(ret)?;
        let mut handler = ConstructorResult {
            text: emu.new_cstring("constructor local")?,
            calls: 0,
        };
        let before = emu.jni.objects.len();
        emu.run_constructors(module, &mut handler)?;
        assert_eq!(handler.calls, 2);
        assert_eq!(emu.jni.objects.len(), before, "{arch:?}");
        assert_eq!(emu.jni.scope, 0);
    }
    Ok(())
}

struct ExtraPops(bool);
impl JniHandler for ExtraPops {
    fn stopped(&mut self, emu: &mut Emulator, _: &Stop) -> Resume {
        // B's frame starts at 3; nested call C starts at 4 and must restore B's floor.
        emu.call_static_jni_method(&mut (), "example/Frame", "run()Ljava/lang/Object;", &[])
            .unwrap();
        jni_call(emu, 20, &[0]).unwrap();
        jni_call(emu, 20, &[0]).unwrap();
        emu.write_reg(0, 0).unwrap();
        if self.0 {
            Resume::Abort
        } else {
            Resume::Continue
        }
    }
}

#[test]
fn extra_pops_preserve_outer_locals_and_restore_the_callers_floor() -> Result<()> {
    for arch in ARCHES {
        for abort in [false, true] {
            let mut emu = emulator(arch)?;
            stopped_native(&mut emu)?;
            let text = emu.new_cstring("outer")?;
            // Act as code inside a native method's frame.
            emu.jni.scope = 2;
            let local = Handle(jni_call(&mut emu, 167, &[text])?);
            let result = emu.call_static_jni_method(
                &mut ExtraPops(abort),
                "example/Frame",
                "run()Ljava/lang/Object;",
                &[],
            );
            if abort {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("stopped by the debugger")
                );
            } else {
                assert_eq!(result?, JniValue::Object(Handle(0)));
            }
            assert_eq!(emu.jni.scope, 2);
            assert_eq!(emu.string(local)?, "outer");
            jni_call(&mut emu, 20, &[0])?;
            assert_eq!(jni_call(&mut emu, 232, &[local.0])?, 0);
            assert_eq!(emu.jni.scope, 2);
        }
    }
    Ok(())
}
