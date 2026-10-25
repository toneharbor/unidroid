use super::*;
use support::SPIN;

#[test]
fn local_frames_do_not_cross_threads() -> Result<()> {
    for arch in ARCHES {
        let (mut emu, libc) = libc_emulator(arch)?;
        let table = emu.read_pointer(emu.jni.env)?;
        let alias = emu.read_pointer(table + 25 * emu.pointer_size())?;
        register_native(&mut emu, "()Ljava/lang/Object;", alias)?;
        let worker = Worker::spawn(&mut emu, libc)?;
        let name = emu.new_cstring("example/Frames")?;
        let main_return = SPIN + if arch == Architecture::Arm32 { 28 } else { 24 };
        emu.add_breakpoint(SPIN)?;
        emu.add_breakpoint(main_return)?;
        let (mut main, mut main_pushed, mut base, mut pushed) = (0, 0, 0, 0);
        worker.wait(
            &mut emu,
            &mut AtStop(|emu: &mut Emulator, stop: &Stop| {
                match stop.address {
                    SPIN => {
                        assert_eq!(stop.thread, MAIN_TID);
                        main = jni_call(emu, 6, &[name])?;
                        jni_call(emu, 19, &[16])?;
                        jni_call(emu, 19, &[16])?;
                        main_pushed = jni_call(emu, 6, &[name])?;
                        assert_eq!(emu.jni.scope, 4);
                    }
                    CODE => {
                        assert_ne!(stop.thread, MAIN_TID);
                        assert_eq!((emu.jni.scope, emu.jni.frame_floor), (2, 2));
                        base = jni_call(emu, 6, &[name])?;
                        jni_call(emu, 19, &[16])?;
                        let local = jni_call(emu, 6, &[name])?;
                        jni_call(emu, 20, &[0])?;
                        assert_eq!(jni_call(emu, 232, &[local])?, 0);
                        assert_eq!(jni_call(emu, 232, &[main_pushed])?, 1);
                        jni_call(emu, 23, &[main])?;
                        assert_eq!(jni_call(emu, 232, &[main])?, 1);
                        // A JNI call nested in a worker stop returns into the worker's base frame.
                        let nested = emu
                            .call_jni_method(
                                &mut (),
                                "example/Frame",
                                Handle(base),
                                "run()Ljava/lang/Object;",
                                &[],
                            )?
                            .object();
                        assert_eq!(emu.jni.objects[&nested].scope, 2);
                        assert_eq!((emu.jni.scope, emu.jni.frame_floor), (2, 2));
                        jni_call(emu, 19, &[16])?;
                        pushed = jni_call(emu, 6, &[name])?;
                    }
                    address if address == main_return => {
                        assert_eq!(stop.thread, MAIN_TID);
                        assert_eq!((emu.jni.scope, emu.jni.frame_floor), (4, 2));
                        assert_eq!(jni_call(emu, 232, &[main])?, 1);
                        jni_call(emu, 20, &[0])?;
                        assert_eq!(jni_call(emu, 232, &[main_pushed])?, 0);
                        jni_call(emu, 20, &[0])?;
                        assert_eq!(emu.jni.scope, 2);
                        jni_call(emu, 19, &[16])?;
                        let local = jni_call(emu, 6, &[name])?;
                        jni_call(emu, 20, &[0])?;
                        assert_eq!(jni_call(emu, 232, &[local])?, 0);
                        for handle in [main, base, pushed] {
                            assert_eq!(jni_call(emu, 232, &[handle])?, 1);
                        }
                    }
                    _ => panic!("unexpected stop: {stop:?}"),
                }
                Ok(Resume::Continue)
            }),
        )?;
        assert_eq!(emu.jni.scope, 0);
        assert_eq!(jni_call(&mut emu, 232, &[main])?, 0);
        for handle in [base, pushed] {
            assert_eq!(jni_call(&mut emu, 232, &[handle])?, 1);
        }
        worker.join(
            &mut emu,
            libc,
            &mut AtStop(|emu: &mut Emulator, stop: &Stop| {
                assert_eq!(stop.address, worker.finish);
                worker.released(emu)?;
                assert_eq!((emu.jni.scope, emu.jni.frame_floor), (3, 2));
                jni_call(emu, 20, &[0])?;
                assert_eq!(jni_call(emu, 232, &[pushed])?, 0);
                assert_eq!(jni_call(emu, 232, &[base])?, 1);
                // Extra pops may clear the base frame, but cannot move below its floor.
                jni_call(emu, 20, &[0])?;
                jni_call(emu, 20, &[0])?;
                assert_eq!((emu.jni.scope, emu.jni.frame_floor), (2, 2));
                Ok(Resume::Continue)
            }),
        )?;
    }
    Ok(())
}

#[test]
fn returning_another_threads_local_keeps_its_owner() -> Result<()> {
    for arch in ARCHES {
        let (mut emu, libc) = libc_emulator(arch)?;
        let worker = Worker::spawn(&mut emu, libc)?;
        let name = emu.new_cstring("example/ReturnedWorker")?;
        let main_return = SPIN + if arch == Architecture::Arm32 { 28 } else { 24 };
        emu.add_breakpoint(main_return)?;
        let mut local = 0;
        let result = emu.call_function_with(
            &mut AtStop(|emu: &mut Emulator, stop: &Stop| {
                if stop.address == CODE {
                    local = jni_call(emu, 6, &[name])?;
                } else {
                    assert_eq!((stop.address, stop.thread), (main_return, MAIN_TID));
                    emu.write_reg(0, local)?;
                }
                Ok(Resume::Continue)
            }),
            SPIN,
            &[worker.flags],
        )?;
        assert_eq!(result, local);
        assert_eq!(emu.jni.objects[&Handle(local)].scope, 2);
        jni_call(&mut emu, 23, &[local])?;
        assert_eq!(jni_call(&mut emu, 232, &[local])?, 1);
        worker.join(
            &mut emu,
            libc,
            &mut AtStop(|emu: &mut Emulator, stop: &Stop| {
                assert_eq!(stop.address, worker.finish);
                worker.released(emu)?;
                assert_eq!(emu.class_name(Handle(local))?, "example/ReturnedWorker");
                Ok(Resume::Continue)
            }),
        )?;
        assert_eq!(jni_call(&mut emu, 232, &[local])?, 0);
    }
    Ok(())
}
