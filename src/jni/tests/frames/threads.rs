use super::*;
use crate::threads::MAIN_TID;

mod frames;
mod lifecycle;
mod support;
use support::{AtStop, Worker};

#[test]
fn worker_local_survives_host_calls_and_dies_on_exit() -> Result<()> {
    for arch in ARCHES {
        let (mut emu, libc) = libc_emulator(arch)?;
        let worker = Worker::spawn(&mut emu, libc)?;
        let name = emu.new_cstring("example/Worker")?;
        let mut local = 0;
        let mut tid = 0;
        worker.wait(
            &mut emu,
            &mut AtStop(|emu: &mut Emulator, stop: &Stop| {
                assert_eq!(stop.address, CODE);
                assert_ne!(stop.thread, MAIN_TID);
                tid = stop.thread;
                local = jni_call(emu, 6, &[name])?;
                Ok(Resume::Continue)
            }),
        )?;
        // Check these after the later use, so the old code exposes NewGlobalRef's error.
        let kind = jni_call(&mut emu, 232, &[local])?;
        let delete = jni_call(&mut emu, 23, &[local]);
        let live_after_delete = emu.class_name(Handle(local));
        let mut global = 0;
        worker.join(
            &mut emu,
            libc,
            &mut AtStop(|emu: &mut Emulator, stop: &Stop| {
                assert_eq!((stop.address, stop.thread), (worker.finish, tid));
                worker.released(emu)?;
                global = jni_call(emu, 21, &[local])?;
                assert_eq!(emu.class_name(Handle(global))?, "example/Worker");
                Ok(Resume::Continue)
            }),
        )?;
        assert_eq!(kind, 1);
        assert_eq!(delete?, 0);
        assert_eq!(live_after_delete?, "example/Worker");
        assert_eq!(jni_call(&mut emu, 232, &[local])?, 0);
        assert_eq!(jni_call(&mut emu, 232, &[global])?, 2);
        assert_eq!(emu.class_name(Handle(global))?, "example/Worker");
        assert_eq!(emu.jni.scope, 0);
    }
    Ok(())
}

#[test]
fn checkpoint_restores_worker_locals_and_frames() -> Result<()> {
    for arch in ARCHES {
        let (mut emu, libc) = libc_emulator(arch)?;
        let worker = Worker::spawn(&mut emu, libc)?;
        let name = emu.new_cstring("example/Checkpoint")?;
        let (mut base, mut local, mut tid) = (0, 0, 0);
        worker.wait(
            &mut emu,
            &mut AtStop(|emu: &mut Emulator, stop: &Stop| {
                assert_eq!(stop.address, CODE);
                tid = stop.thread;
                base = jni_call(emu, 6, &[name])?;
                jni_call(emu, 19, &[16])?;
                local = jni_call(emu, 6, &[name])?;
                Ok(Resume::Continue)
            }),
        )?;
        let saved = emu.checkpoint()?;
        let later = Worker::spawn(&mut emu, libc)?;
        let mut later_local = 0;
        later.wait(
            &mut emu,
            &mut AtStop(|emu: &mut Emulator, stop: &Stop| {
                assert_eq!(stop.address, CODE);
                assert_ne!(stop.thread, tid);
                later_local = jni_call(emu, 6, &[name])?;
                Ok(Resume::Continue)
            }),
        )?;
        for rewind in [false, true] {
            if rewind {
                emu.rewind(&saved)?;
                assert_eq!(jni_call(&mut emu, 232, &[later_local])?, 0);
            }
            assert_eq!(jni_call(&mut emu, 232, &[local])?, 1);
            jni_call(&mut emu, 23, &[local])?;
            assert_eq!(emu.class_name(Handle(local))?, "example/Checkpoint");
            let mut global = 0;
            worker.join(
                &mut emu,
                libc,
                &mut AtStop(|emu: &mut Emulator, stop: &Stop| {
                    assert_eq!((stop.address, stop.thread), (worker.finish, tid));
                    worker.released(emu)?;
                    assert_eq!((emu.jni.scope, emu.jni.frame_floor), (3, 2));
                    global = jni_call(emu, 21, &[local])?;
                    jni_call(emu, 20, &[0])?;
                    assert_eq!(jni_call(emu, 232, &[local])?, 0);
                    assert_eq!(jni_call(emu, 232, &[base])?, 1);
                    assert_eq!(emu.jni.scope, 2);
                    Ok(Resume::Continue)
                }),
            )?;
            assert_eq!(jni_call(&mut emu, 232, &[base])?, 0);
            assert_eq!(jni_call(&mut emu, 232, &[local])?, 0);
            assert_eq!(emu.class_name(Handle(global))?, "example/Checkpoint");
            if rewind {
                assert!(!emu.threads.has_saved());
            }
        }
    }
    Ok(())
}
