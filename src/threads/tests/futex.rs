use super::*;

fn waiter_code(emu: &mut Emulator) -> Result<u64> {
    // arg = [address, operation, value, timeout, result, counter, wake_order].
    // Wait, record the raw return value and completion order, then return 42.
    code(
        emu,
        0x100,
        if emu.architecture() == Architecture::Arm32 {
            &[
                0xe92d41f0, 0xe1a06000, 0xe896000f, 0xe3a070f0, 0xef000000, 0xe5860010, 0xe5964014,
                0xe5945000, 0xe2855001, 0xe5845000, 0xe5865018, 0xe3a0002a, 0xe8bd81f0,
            ]
        } else {
            &[
                0xa9bf7bf3, 0xaa0003f3, 0xa9400660, 0xa9410e62, 0xd2800c48, 0xd4000001, 0xf9001260,
                0xf9401664, 0xb9400085, 0x110004a5, 0xb9000085, 0xb9003265, 0xd2800540, 0xa8c17bf3,
                0xd65f03c0,
            ]
        },
    )
}

fn waiter_args(
    emu: &mut Emulator,
    address: u64,
    operation: u64,
    timeout: u64,
    counter: u64,
) -> Result<u64> {
    let p = emu.pointer_size();
    let args = emu.malloc((7 * p) as usize)?;
    for (i, value) in [address, operation, 0, timeout, 99, counter, 0]
        .into_iter()
        .enumerate()
    {
        emu.write_pointer(args + i as u64 * p, value)?;
    }
    Ok(args)
}

#[test]
fn futex_wakes_only_the_requested_waiters_in_wait_order() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        let (mut emu, libc) = libc_emulator(arch)?;
        let worker = waiter_code(&mut emu)?;
        let futex = syscall_code(&mut emu, 0x300, 240, 98)?;
        let address = emu.malloc(4)?;
        let counter = emu.malloc(4)?;
        let mut waiters = Vec::new();
        for operation in [0, 128, 9 | 128] {
            let args = waiter_args(&mut emu, address, operation, 0, counter)?;
            waiters.push((spawn(&mut emu, libc, worker, args)?, args));
        }
        assert_eq!(call(&mut emu, libc, "sched_yield", &[])?, 0);
        assert_eq!(emu.call_function(futex, &[address, 1, 0, 0, 0, 0])?, 0);
        assert_eq!(emu.call_function(futex, &[address + 4, 1, 3, 0, 0, 0])?, 0);
        assert_eq!(
            emu.call_function(futex, &[address, 1 | 128, 2, 0, 0, 0])?,
            2
        );
        assert_eq!(call(&mut emu, libc, "sched_yield", &[])?, 0);
        let p = emu.pointer_size();
        for (i, (_, args)) in waiters.iter().enumerate() {
            assert_eq!(emu.read_pointer(args + 4 * p)?, if i < 2 { 0 } else { 99 });
            assert_eq!(
                emu.read_pointer(args + 6 * p)?,
                if i < 2 { i as u64 + 1 } else { 0 }
            );
        }
        assert_eq!(
            emu.call_function(futex, &[address, 10 | 128, 5, 0, 0, 0])?,
            1
        );
        for (thread, _) in &waiters {
            assert_eq!(call(&mut emu, libc, "pthread_join", &[*thread, 0])?, 0);
        }
        assert_eq!(emu.read_pointer(waiters[2].1 + 6 * p)?, 3);
        assert_eq!(emu.call_function(futex, &[address, 1, 5, 0, 0, 0])?, 0);
    }
    Ok(())
}

#[test]
fn futex_changed_value_does_not_wait_and_other_commands_stay_enosys() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        let (mut emu, libc) = libc_emulator(arch)?;
        let worker = waiter_code(&mut emu)?;
        let futex = syscall_code(&mut emu, 0x300, 240, 98)?;
        let address = emu.malloc(4)?;
        let counter = emu.malloc(4)?;
        let args = waiter_args(&mut emu, address, 0, 0, counter)?;
        let thread = spawn(&mut emu, libc, worker, args)?;
        for op in [0, 9 | 128] {
            assert_eq!(
                emu.call_function(futex, &[address, op, 1, 0, 0, 0])?,
                (-11i64) as u64 & arch.pointer_mask()
            );
        }
        for op in [3, 4, 5] {
            assert_eq!(
                emu.call_function(futex, &[address, op, 1, 0, 0, 0])?,
                (-38i64) as u64 & arch.pointer_mask()
            );
        }
        emu.write_bytes(address, &1u32.to_le_bytes())?;
        assert_eq!(call(&mut emu, libc, "pthread_join", &[thread, 0])?, 0);
        assert_eq!(
            emu.read_pointer(args + 4 * emu.pointer_size())?,
            (-11i64) as u64 & arch.pointer_mask()
        );
    }
    Ok(())
}

#[test]
fn futex_timeouts_advance_guest_time_in_deadline_order() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        let (mut emu, libc) = libc_emulator(arch)?;
        let worker = waiter_code(&mut emu)?;
        let address = emu.malloc(4)?;
        let counter = emu.malloc(4)?;
        let p = emu.pointer_size();
        let mut waiters = Vec::new();
        for nanos in [30, 10, 20] {
            let timeout = emu.malloc(16)?;
            emu.write_pointer(timeout + p, nanos)?;
            let args = waiter_args(&mut emu, address, 0, timeout, counter)?;
            waiters.push((spawn(&mut emu, libc, worker, args)?, args));
        }
        assert_eq!(call(&mut emu, libc, "sched_yield", &[])?, 0);
        assert_eq!(emu.read_bytes(counter, 4)?, [0; 4]); // Runnable main prevents timeout.
        let futex = syscall_code(&mut emu, 0x300, 240, 98)?;
        let timeout = emu.malloc(16)?;
        emu.write_pointer(timeout + p, 5)?;
        assert_eq!(
            emu.call_function(futex, &[address, 0, 0, timeout, 0, 0])?,
            (-110i64) as u64 & arch.pointer_mask()
        );
        assert_eq!(emu.read_bytes(counter, 4)?, [0; 4]);
        for (thread, _) in &waiters {
            assert_eq!(call(&mut emu, libc, "pthread_join", &[*thread, 0])?, 0);
        }
        for ((_, args), order) in waiters.iter().zip([3, 1, 2]) {
            assert_eq!(
                emu.read_pointer(args + 4 * p)?,
                (-110i64) as u64 & arch.pointer_mask()
            );
            assert_eq!(emu.read_pointer(args + 6 * p)?, order);
        }
    }
    Ok(())
}

#[test]
fn futex_deadlock_names_every_waiting_thread_and_address() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        let (mut emu, libc) = libc_emulator(arch)?;
        let worker = waiter_code(&mut emu)?;
        let futex = syscall_code(&mut emu, 0x300, 240, 98)?;
        let address = emu.malloc(8)?;
        let counter = emu.malloc(4)?;
        let args = waiter_args(&mut emu, address, 0, 0, counter)?;
        spawn(&mut emu, libc, worker, args)?;
        let error = emu
            .call_function(futex, &[address + 4, 0, 0, 0, 0, 0])
            .unwrap_err()
            .to_string();
        assert!(error.contains("all guest threads are waiting"), "{error}");
        assert!(
            error.contains(&format!("tid 1234 futex 0x{:x}", address + 4)),
            "{error}"
        );
        assert!(
            error.contains(&format!("tid 1235 futex 0x{address:x}")),
            "{error}"
        );
    }
    Ok(())
}

#[test]
fn blocking_futex_in_nested_run_reports_address_and_location() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        let (mut emu, libc) = libc_emulator(arch)?;
        let worker = code(
            &mut emu,
            0x100,
            &[if arch == Architecture::Arm32 {
                0xe12fff1e
            } else {
                0xd65f03c0
            }],
        )?;
        let thread = spawn(&mut emu, libc, worker, 42)?;
        let callback = code(
            &mut emu,
            0x200,
            if arch == Architecture::Arm32 {
                // dl_iterate_phdr(info, size, data): wait on *data.
                &[
                    0xe1a00002, 0xe3a01000, 0xe3a02000, 0xe3a03000, 0xe3a070f0, 0xef000000,
                    0xe12fff1e,
                ]
            } else {
                &[
                    0xaa0203e0, 0xd2800001, 0xd2800002, 0xd2800003, 0xd2800c48, 0xd4000001,
                    0xd65f03c0,
                ]
            },
        )?;
        let address = emu.malloc(4)?;
        let error = call(&mut emu, libc, "dl_iterate_phdr", &[callback, address]).unwrap_err();
        let crash = error.downcast_ref::<crate::Crash>().unwrap();
        assert_eq!(crash.thread, 1234);
        assert_eq!(crash.backtrace[0].address, callback + 24);
        assert_eq!(
            crash.registers.iter().find(|r| r.0 == "pc").unwrap().1,
            callback + 24
        );
        assert_eq!(
            error.to_string(),
            format!(
                "futex wait at 0x{address:x} would block in nested run at 0x{:x}",
                callback + 20
            )
        );
        // The error restores the host call's context and leaves the child saved.
        assert_eq!(call(&mut emu, libc, "pthread_join", &[thread, 0])?, 0);
    }
    Ok(())
}

#[test]
fn single_thread_futex_keeps_old_results_without_reading_guest_memory() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        let (mut emu, _) = libc_emulator(arch)?;
        let futex = syscall_code(&mut emu, 0x100, 240, 98)?;
        for op in [0, 9, 128, 137] {
            assert_eq!(
                emu.call_function(futex, &[0, op, 0, 1, 0, 0])?,
                (-11i64) as u64 & arch.pointer_mask()
            );
        }
        for op in [1, 10, 129, 138] {
            assert_eq!(emu.call_function(futex, &[0, op, 9, 0, 0, 0])?, 0);
        }
        // A call longer than a slice still completes without another thread.
        let countdown = code(
            &mut emu,
            0x200,
            if arch == Architecture::Arm32 {
                &[0xe30c0350, 0xe2500001, 0x1afffffd, 0xe12fff1e]
            } else {
                &[0x52986a00, 0x71000400, 0x54ffffe1, 0xd65f03c0]
            },
        )?;
        assert_eq!(emu.call_function(countdown, &[])?, 0);
    }
    Ok(())
}
