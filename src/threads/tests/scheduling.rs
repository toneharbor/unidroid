use super::*;

#[test]
fn arm32_kuser_cmpxchg_increments_are_atomic_across_thread_slices() -> Result<()> {
    let (mut emu, libc) = libc_emulator(Architecture::Arm32)?;
    let worker = code(
        &mut emu,
        0x100,
        &[
            0xe92d41f0, // push {r4-r8,lr}
            0xe1a04000, // mov r4,r0
            0xe3045e20, // movw r5,#20000
            0xe3006fc0, // movw r6,#0x0fc0
            0xe34f6fff, // movt r6,#0xffff
            0xe5943004, // wait: ldr r3,[r4,#4]
            0xe3530000, // cmp r3,#0
            0x0afffffc, // beq wait
            // Shift slice boundaries through the helper's load/store sequence.
            0xe320f000, // retry: nop
            0xe320f000, // nop
            0xe320f000, // nop
            0xe5940000, // ldr r0,[r4]
            0xe2801001, // add r1,r0,#1
            0xe1a02004, // mov r2,r4
            0xe12fff36, // blx r6 (__kuser_cmpxchg)
            0x3afffff7, // bcc retry
            0xe2555001, // subs r5,r5,#1
            0x1afffff5, // bne retry
            0xe3a00000, // mov r0,#0
            0xe8bd81f0, // pop {r4-r8,pc}
        ],
    )?;
    let counter = emu.malloc(8)?;
    let first = spawn(&mut emu, libc, worker, counter)?;
    let second = spawn(&mut emu, libc, worker, counter)?;
    assert_eq!(emu.read_bytes(counter, 4)?, [0; 4]);
    emu.write_bytes(counter + 4, &1u32.to_le_bytes())?;
    for thread in [first, second] {
        assert_eq!(call(&mut emu, libc, "pthread_join", &[thread, 0])?, 0);
    }
    assert_eq!(emu.read_bytes(counter, 4)?, 40000u32.to_le_bytes());
    Ok(())
}

#[test]
fn instruction_slices_run_a_saved_thread_during_the_next_call() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        let (mut emu, libc) = libc_emulator(arch)?;
        let worker = code(
            &mut emu,
            0x100,
            if arch == Architecture::Arm32 {
                &[0xe3a01001, 0xe5801000, 0xe12fff1e] // *arg = 1; return arg
            } else {
                &[0x52800021, 0xb9000001, 0xd65f03c0]
            },
        )?;
        let spin = code(
            &mut emu,
            0x200,
            if arch == Architecture::Arm32 {
                // Bounded spin: 50,000 tries, no syscall. Return the observed flag.
                &[
                    0xe30c2350, 0xe5901000, 0xe3510000, 0x1a000001, 0xe2522001, 0x1afffffa,
                    0xe1a00001, 0xe12fff1e,
                ]
            } else {
                &[
                    0x52986a02, 0xb9400001, 0x35000061, 0x71000442, 0x54ffffa1, 0x2a0103e0,
                    0xd65f03c0,
                ]
            },
        )?;
        let flag = emu.malloc(4)?;
        let thread = spawn(&mut emu, libc, worker, flag)?;
        assert_eq!(emu.read_bytes(flag, 4)?, [0; 4]);
        assert_eq!(emu.call_function(spin, &[flag])?, 1);
        assert_eq!(call(&mut emu, libc, "pthread_join", &[thread, 0])?, 0);
    }
    Ok(())
}

#[test]
fn a_partly_executed_thread_resumes_after_main_returns() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        let (mut emu, libc) = libc_emulator(arch)?;
        let worker = code(
            &mut emu,
            0x100,
            if arch == Architecture::Arm32 {
                // entered = 1; count down; finished = 1; return.
                &[
                    0xe3a01001, 0xe5801000, 0xe30c2350, 0xe2522001, 0x1afffffd, 0xe5801004,
                    0xe12fff1e,
                ]
            } else {
                &[
                    0x52800021, 0xb9000001, 0x52986a02, 0x71000442, 0x54ffffe1, 0xb9000401,
                    0xd65f03c0,
                ]
            },
        )?;
        let spin = code(
            &mut emu,
            0x200,
            if arch == Architecture::Arm32 {
                &[
                    0xe30c2350, 0xe5901000, 0xe3510000, 0x1a000001, 0xe2522001, 0x1afffffa,
                    0xe1a00001, 0xe12fff1e,
                ]
            } else {
                &[
                    0x52986a02, 0xb9400001, 0x35000061, 0x71000442, 0x54ffffa1, 0x2a0103e0,
                    0xd65f03c0,
                ]
            },
        )?;
        let flags = emu.malloc(8)?;
        let thread = spawn(&mut emu, libc, worker, flags)?;
        assert_eq!(emu.call_function(spin, &[flags])?, 1);
        assert_eq!(emu.read_bytes(flags + 4, 4)?, [0; 4]);
        assert_eq!(emu.call_function(spin, &[flags + 4])?, 1);
        assert_eq!(call(&mut emu, libc, "pthread_join", &[thread, 0])?, 0);
    }
    Ok(())
}

#[test]
fn sleep_syscalls_yield_and_signal_setup_returns_zero() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        for (arm32, arm64) in [(158, 124), (162, 101), (265, 115)] {
            let (mut emu, libc) = libc_emulator(arch)?;
            let worker = code(
                &mut emu,
                0x100,
                if arch == Architecture::Arm32 {
                    &[0xe3a01001, 0xe5801000, 0xe12fff1e]
                } else {
                    &[0x52800021, 0xb9000001, 0xd65f03c0]
                },
            )?;
            let yield_call = syscall_code(&mut emu, 0x200, arm32, arm64)?;
            let flag = emu.malloc(4)?;
            let thread = spawn(&mut emu, libc, worker, flag)?;
            assert_eq!(emu.call_function(yield_call, &[0; 6])?, 0);
            assert_eq!(emu.read_bytes(flag, 4)?, 1u32.to_le_bytes());
            assert_eq!(call(&mut emu, libc, "pthread_join", &[thread, 0])?, 0);
            for (arm32, arm64) in [(186, 132), (175, 135)] {
                let signal = syscall_code(&mut emu, 0x300, arm32, arm64)?;
                assert_eq!(emu.call_function(signal, &[0; 6])?, 0);
            }
        }
    }
    Ok(())
}
