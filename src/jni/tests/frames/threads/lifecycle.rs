use super::*;
use support::SPIN;

fn detach(emu: &mut Emulator) -> Result<u64> {
    let table = emu.read_pointer(emu.jni.vm)?;
    let function = emu.read_pointer(table + 5 * emu.pointer_size())?;
    emu.call_function(function, &[emu.jni.vm])
}

#[test]
fn detach_clears_worker_locals_but_preserves_main_locals() -> Result<()> {
    for arch in ARCHES {
        let (mut emu, libc) = libc_emulator(arch)?;
        let worker = Worker::spawn(&mut emu, libc)?;
        let name = emu.new_cstring("example/Detach")?;
        let host = emu.class("example/Host");
        emu.add_breakpoint(SPIN)?;
        let (mut main, mut fresh, mut global) = (0, 0, 0);
        worker.wait(
            &mut emu,
            &mut AtStop(|emu: &mut Emulator, stop: &Stop| {
                if stop.address == SPIN {
                    assert_eq!(stop.thread, MAIN_TID);
                    main = jni_call(emu, 6, &[name])?;
                    jni_call(emu, 19, &[16])?;
                    let pushed = jni_call(emu, 6, &[name])?;
                    assert_eq!(detach(emu)?, 0);
                    assert_eq!((emu.jni.scope, emu.jni.frame_floor), (3, 2));
                    for local in [main, pushed] {
                        assert_eq!(jni_call(emu, 232, &[local])?, 1);
                    }
                } else {
                    assert_eq!(stop.address, CODE);
                    assert_ne!(stop.thread, MAIN_TID);
                    let base = jni_call(emu, 6, &[name])?;
                    jni_call(emu, 19, &[16])?;
                    let pushed = jni_call(emu, 6, &[name])?;
                    global = jni_call(emu, 21, &[pushed])?;
                    jni_call(emu, 19, &[16])?;
                    let deep = jni_call(emu, 6, &[name])?;
                    assert_eq!(detach(emu)?, 0);
                    for local in [base, pushed, deep] {
                        assert_eq!(jni_call(emu, 232, &[local])?, 0);
                    }
                    assert_eq!((emu.jni.scope, emu.jni.frame_floor), (2, 2));
                    assert_eq!(jni_call(emu, 232, &[main])?, 1);
                    assert_eq!(jni_call(emu, 232, &[global])?, 2);
                    assert_eq!(emu.class_name(host)?, "example/Host");
                    fresh = jni_call(emu, 6, &[name])?;
                }
                Ok(Resume::Continue)
            }),
        )?;
        assert_eq!(jni_call(&mut emu, 232, &[main])?, 0);
        assert_eq!(jni_call(&mut emu, 232, &[fresh])?, 1);
        assert_eq!((emu.jni.scope, emu.jni.frame_floor), (0, 0));
        worker.join(
            &mut emu,
            libc,
            &mut AtStop(|emu: &mut Emulator, stop: &Stop| {
                assert_eq!(stop.address, worker.finish);
                worker.released(emu)?;
                assert_eq!(emu.class_name(Handle(fresh))?, "example/Detach");
                assert_eq!((emu.jni.scope, emu.jni.frame_floor), (2, 2));
                Ok(Resume::Continue)
            }),
        )?;
        assert_eq!(jni_call(&mut emu, 232, &[fresh])?, 0);
        assert_eq!(jni_call(&mut emu, 232, &[global])?, 2);
    }
    Ok(())
}

#[test]
fn worker_error_clears_only_the_discarded_threads_locals() -> Result<()> {
    for arch in ARCHES {
        let (mut emu, libc) = libc_emulator(arch)?;
        let worker = Worker::spawn(&mut emu, libc)?;
        let name = emu.new_cstring("example/Error")?;
        let mut kept = 0;
        worker.wait(
            &mut emu,
            &mut AtStop(|emu: &mut Emulator, stop: &Stop| {
                assert_eq!(stop.address, CODE);
                kept = jni_call(emu, 6, &[name])?;
                Ok(Resume::Continue)
            }),
        )?;
        let discarded = Worker::spawn(&mut emu, libc)?;
        emu.add_breakpoint(SPIN)?;
        let (mut main, mut local, mut global) = (0, 0, 0);
        let error = discarded
            .wait(
                &mut emu,
                &mut AtStop(|emu: &mut Emulator, stop: &Stop| {
                    if stop.address == SPIN {
                        assert_eq!(stop.thread, MAIN_TID);
                        jni_call(emu, 19, &[16])?;
                        main = jni_call(emu, 6, &[name])?;
                        return Ok(Resume::Continue);
                    }
                    assert_eq!(stop.address, CODE);
                    jni_call(emu, 19, &[16])?;
                    jni_call(emu, 19, &[16])?;
                    local = jni_call(emu, 6, &[name])?;
                    global = jni_call(emu, 21, &[local])?;
                    Ok(Resume::Abort)
                }),
            )
            .unwrap_err();
        assert!(error.to_string().contains("stopped by the debugger"));
        assert_eq!(emu.threads.tid(), MAIN_TID);
        assert_eq!((emu.jni.scope, emu.jni.frame_floor), (0, 0));
        for dead in [main, local] {
            assert_eq!(jni_call(&mut emu, 232, &[dead])?, 0);
        }
        assert_eq!(jni_call(&mut emu, 232, &[kept])?, 1);
        assert_eq!(jni_call(&mut emu, 232, &[global])?, 2);
        worker.join(
            &mut emu,
            libc,
            &mut AtStop(|emu: &mut Emulator, stop: &Stop| {
                assert_eq!(stop.address, worker.finish);
                worker.released(emu)?;
                assert_eq!(emu.class_name(Handle(kept))?, "example/Error");
                Ok(Resume::Continue)
            }),
        )?;
        assert_eq!(jni_call(&mut emu, 232, &[kept])?, 0);
    }
    Ok(())
}
