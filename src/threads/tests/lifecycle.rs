use super::*;

#[test]
fn child_exit_inside_nested_callback_does_not_resume_the_callback() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        let (mut emu, libc) = libc_emulator(arch)?;
        let worker = code(
            &mut emu,
            0x100,
            if arch == Architecture::Arm32 {
                // Tail-call dl_iterate_phdr(callback, data) from [function, callback, data].
                &[
                    0xe1a03000, 0xe5932008, 0xe5931004, 0xe5933000, 0xe1a00001, 0xe1a01002,
                    0xe12fff13,
                ]
            } else {
                &[0xf9400003, 0xa9408801, 0xaa0103e0, 0xaa0203e1, 0xd61f0060]
            },
        )?;
        let callback = code(
            &mut emu,
            0x200,
            if arch == Architecture::Arm32 {
                // exit(0); *data = 1 (must never execute); return.
                &[
                    0xe3a00000, 0xe3a07001, 0xef000000, 0xe3a01001, 0xe5821000, 0xe12fff1e,
                ]
            } else {
                &[
                    0xd2800000, 0xd2800ba8, 0xd4000001, 0x52800021, 0xb9000041, 0xd65f03c0,
                ]
            },
        )?;
        let flag = emu.malloc(4)?;
        let args = emu.malloc(24)?;
        let iterate = emu.find_symbol(libc, "dl_iterate_phdr")?;
        let p = emu.pointer_size();
        for (i, value) in [iterate, callback, flag].into_iter().enumerate() {
            emu.write_pointer(args + i as u64 * p, value)?;
        }
        let thread = spawn(&mut emu, libc, worker, args)?;
        assert_eq!(call(&mut emu, libc, "pthread_join", &[thread, 0])?, 0);
        assert_eq!(emu.read_bytes(flag, 4)?, [0; 4]);
    }
    Ok(())
}

#[test]
fn main_exit_and_exit_group_report_status_and_location() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        for (arm32, arm64, name) in [(1, 93, "exit"), (248, 94, "exit_group")] {
            let (mut emu, libc) = libc_emulator(arch)?;
            let exit = syscall_code(&mut emu, 0x100, arm32, arm64)?;
            let at = exit + if arch == Architecture::Arm32 { 16 } else { 4 };
            let error = emu
                .call_function(exit, &[7, 0, 0, 0, 0, 0])
                .unwrap_err()
                .to_string();
            assert_eq!(error, format!("emulation stopped: {name}(7) at 0x{at:x}"));
            if name == "exit_group" {
                spawn(&mut emu, libc, exit, 9)?;
                let error = call(&mut emu, libc, "sched_yield", &[]).unwrap_err();
                let crash = error.downcast_ref::<crate::Crash>().unwrap();
                assert_eq!(crash.thread, 1235);
                assert_eq!(crash.backtrace[0].address, at + 4);
                assert_eq!(
                    error.to_string(),
                    format!("emulation stopped: exit_group(9) at 0x{at:x}")
                );
            }
            assert_eq!(call(&mut emu, libc, "gettid", &[])?, 1234);
        }
    }
    Ok(())
}

#[test]
fn clone_copies_tls_and_stack_sets_tids_and_can_replace_or_clear_ctid() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        let (mut emu, libc) = libc_emulator(arch)?;
        let clone = code(
            &mut emu,
            0x100,
            if arch == Architecture::Arm32 {
                // Parent returns normally. Child records r0, TLS, tid and pid on
                // its supplied stack, sets clear_tid, records its tid, and exits.
                &[
                    0xe92d40b0, 0xe59d4010, 0xe3a07078, 0xef000000, 0xe3500000, 0x18bd80b0,
                    0xe1a0600d, 0xe5860000, 0xee1d5f70, 0xe5865004, 0xe3a070e0, 0xef000000,
                    0xe5860008, 0xe3a07014, 0xef000000, 0xe586000c, 0xe5960010, 0xe3007100,
                    0xef000000, 0xe5860014, 0xe3a00000, 0xe3a07001, 0xef000000,
                ]
            } else {
                &[
                    0xd2801b88, 0xd4000001, 0xb4000040, 0xd65f03c0, 0x910003f3, 0xf9000260,
                    0xd53bd045, 0xf9000665, 0xd2801648, 0xd4000001, 0xf9000a60, 0xd2801588,
                    0xd4000001, 0xf9000e60, 0xf9401260, 0xd2800c08, 0xd4000001, 0xf9001660,
                    0xd2800000, 0xd2800ba8, 0xd4000001,
                ]
            },
        )?;
        for flags in [0, 0x100, 0x10000] {
            assert_eq!(
                emu.call_function(clone, &[flags, 0, 0, 0, 0])?,
                (-38i64) as u64 & arch.pointer_mask()
            );
        }
        let parent_tls = emu.cpu()?.reg_read(arch.tls())?;
        let p = emu.pointer_size();
        for (i, set_tls) in [true, false].into_iter().enumerate() {
            let stack = emu.malloc(64)?;
            let tls = emu.malloc(64)?;
            let ptid = emu.malloc(8)?;
            let ctid = emu.malloc(8)?;
            let replacement = emu.malloc(8)?;
            emu.write_bytes(stack, &[0xa5; 64])?;
            emu.write_bytes(ptid, &[0xa5; 8])?;
            emu.write_bytes(ctid, &[0xa5; 8])?;
            emu.write_bytes(replacement, &[0xa5; 8])?;
            emu.write_pointer(stack + 4 * p, if set_tls { replacement } else { 0 })?;
            let flags = 0x100
                | 0x10000
                | 0x100000
                | 0x1000000
                | 0x200000
                | if set_tls { 0x80000 } else { 0 };
            let tid = 1235 + i as u64;
            assert_eq!(
                emu.call_function(clone, &[flags, stack, ptid, tls, ctid])?,
                tid
            );
            assert_eq!(emu.read_bytes(ptid, 4)?, (tid as u32).to_le_bytes());
            assert_eq!(emu.read_bytes(ctid, 4)?, (tid as u32).to_le_bytes());
            assert_eq!(emu.read_bytes(ptid + 4, 4)?, [0xa5; 4]);
            assert_eq!(call(&mut emu, libc, "sched_yield", &[])?, 0);
            for (slot, expected) in [
                (0, 0),
                (1, if set_tls { tls } else { parent_tls }),
                (2, tid),
                (3, 1234),
                (5, tid),
            ] {
                assert_eq!(emu.read_pointer(stack + slot * p)?, expected);
            }
            assert_eq!(emu.read_bytes(ctid, 4)?, (tid as u32).to_le_bytes());
            assert_eq!(
                emu.read_bytes(replacement, 4)?,
                if set_tls { [0; 4] } else { [0xa5; 4] }
            );
            assert_eq!(emu.read_bytes(replacement + 4, 4)?, [0xa5; 4]);
            assert_eq!(emu.cpu()?.reg_read(arch.tls())?, parent_tls);
        }
    }
    Ok(())
}

#[test]
fn disabled_clone_keeps_flag_checks_tid_writes_and_single_thread_futex_results() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        let mut emu = Emulator::builder().threads(false).build()?;
        emu.load_library(format!("rootfs/{}/libc.so", arch.library_dir()))?;
        let clone = syscall_code(&mut emu, 0x100, 120, 220)?;
        let futex = syscall_code(&mut emu, 0x200, 240, 98)?;
        let ptid = emu.malloc(8)?;
        let ctid = emu.malloc(8)?;
        emu.write_bytes(ptid, &[0xa5; 8])?;
        emu.write_bytes(ctid, &[0xa5; 8])?;
        for flags in [0, 0x100, 0x10000] {
            assert_eq!(
                emu.call_function(clone, &[flags | 0x1100000, 0, ptid, 0, ctid, 0])?,
                (-38i64) as u64 & arch.pointer_mask()
            );
            assert_eq!(emu.read_bytes(ptid, 8)?, [0xa5; 8]);
            assert_eq!(emu.read_bytes(ctid, 8)?, [0xa5; 8]);
        }
        for (i, (parent, child)) in [(true, false), (false, true), (true, true), (false, false)]
            .into_iter()
            .enumerate()
        {
            emu.write_bytes(ptid, &[0xa5; 8])?;
            emu.write_bytes(ctid, &[0xa5; 8])?;
            let flags = 0x10100
                | 0x280000 // CLONE_SETTLS | CLONE_CHILD_CLEARTID
                | if parent { 0x100000 } else { 0 }
                | if child { 0x1000000 } else { 0 };
            let tid = 1235 + i as u32;
            assert_eq!(
                emu.call_function(clone, &[flags, 0, ptid, 0, ctid, 0])?,
                u64::from(tid)
            );
            for (address, written) in [(ptid, parent), (ctid, child)] {
                assert_eq!(
                    emu.read_bytes(address, 4)?,
                    if written {
                        tid.to_le_bytes()
                    } else {
                        [0xa5; 4]
                    }
                );
                assert_eq!(emu.read_bytes(address + 4, 4)?, [0xa5; 4]);
            }
            assert_eq!(
                emu.call_function(futex, &[0, 0, 0, 0, 0, 0])?,
                (-11i64) as u64 & arch.pointer_mask()
            );
        }
    }
    Ok(())
}

#[test]
fn pthread_join_returns_value_and_clears_tid() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        let (mut emu, libc) = libc_emulator(arch)?;
        let entry = code(
            &mut emu,
            0x100,
            &[if arch == Architecture::Arm32 {
                0xe12fff1e // bx lr (return argument)
            } else {
                0xd65f03c0 // ret
            }],
        )?;
        // A caller-owned stack keeps pthread_internal_t readable after join.
        let stack = crate::page_up(emu.malloc(0x21000)?);
        let attr = emu.malloc(64)?;
        assert_eq!(call(&mut emu, libc, "pthread_attr_init", &[attr])?, 0);
        assert_eq!(
            call(
                &mut emu,
                libc,
                "pthread_attr_setstack",
                &[attr, stack, 0x20000]
            )?,
            0
        );
        let thread = emu.malloc(8)?;
        assert_eq!(
            call(&mut emu, libc, "pthread_create", &[thread, attr, entry, 42])?,
            0
        );
        let handle = emu.read_pointer(thread)?;
        let result = emu.malloc(8)?;
        assert_eq!(call(&mut emu, libc, "pthread_join", &[handle, result])?, 0);
        assert_eq!(emu.read_pointer(result)?, 42);
        assert_eq!(emu.read_bytes(handle + 2 * emu.pointer_size(), 4)?, [0; 4]);
    }
    Ok(())
}
