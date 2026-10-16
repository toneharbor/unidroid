use crate::{Emulator, Result};
use std::collections::VecDeque;
use unicorn_engine::Context;

pub(crate) const MAIN_TID: u32 = 1234;
pub(crate) const SLICE_INSTRUCTIONS: usize = 10000;

#[derive(Clone, Copy)]
struct Identity {
    tid: u32,
    clear_tid: u64,
}

#[derive(Clone, Copy)]
pub(crate) struct Wait {
    address: u64,
    deadline: Option<u128>,
    order: u64,
}

#[derive(Default)]
pub(crate) enum Action {
    #[default]
    Continue,
    Yield,
    Wait(Wait),
    Exit,
}

struct SavedThread {
    identity: Identity,
    context: Context,
    pc: u64,
    wait: Option<Wait>,
}

pub(crate) struct Threads {
    saved: VecDeque<SavedThread>,
    current: Identity,
    next_tid: u32,
    time: u128,
    wait_order: u64,
    pub depth: usize,
    pub action: Action,
}

impl Default for Threads {
    fn default() -> Self {
        Self {
            saved: VecDeque::new(),
            current: Identity {
                tid: MAIN_TID,
                clear_tid: 0,
            },
            next_tid: MAIN_TID + 1,
            time: 0,
            wait_order: 0,
            depth: 0,
            action: Action::Continue,
        }
    }
}

impl Threads {
    pub fn tid(&self) -> u32 {
        self.current.tid
    }

    pub fn has_runnable(&self) -> bool {
        self.saved.iter().any(|thread| thread.wait.is_none())
    }

    pub fn set_tid_address(&mut self, address: u64) -> u64 {
        self.current.clear_tid = address;
        self.tid() as u64
    }
}

impl Emulator {
    pub(crate) fn clone_thread(&mut self, a: [u64; 6]) -> Result<u64> {
        const CLONE_VM: u64 = 0x100;
        const CLONE_THREAD: u64 = 0x10000;
        if a[0] & (CLONE_VM | CLONE_THREAD) != CLONE_VM | CLONE_THREAD {
            return Ok((-38i64) as u64);
        }
        let arch = self.architecture();
        let tid = self.threads.next_tid;
        let mut context = self.cpu()?.context_init()?;
        context.reg_write(arch.gp(0), 0)?;
        context.reg_write(arch.sp(), a[1])?;
        if a[0] & 0x80000 != 0 {
            // CLONE_SETTLS
            context.reg_write(arch.tls(), a[3])?;
        }
        if a[0] & 0x100000 != 0 {
            // CLONE_PARENT_SETTID
            self.write_bytes(a[2], &tid.to_le_bytes())?;
        }
        if a[0] & 0x1000000 != 0 {
            // CLONE_CHILD_SETTID
            self.write_bytes(a[4], &tid.to_le_bytes())?;
        }
        let pc = self.restart_pc()?.0;
        self.threads.saved.push_back(SavedThread {
            identity: Identity {
                tid,
                clear_tid: if a[0] & 0x200000 != 0 { a[4] } else { 0 },
            },
            context,
            pc,
            wait: None,
        });
        self.threads.next_tid += 1;
        Ok(tid as u64)
    }

    pub(crate) fn futex(&mut self, a: [u64; 6], pc: u64) -> Result<u64> {
        match a[1] & 127 {
            1 | 10 => Ok(self.wake_futex(a[0], a[2])),
            0 | 9 => {
                if self.threads.saved.is_empty() {
                    return Ok((-11i64) as u64);
                }
                let mut word = [0; 4];
                self.cpu()?.mem_read(a[0], &mut word)?;
                if u32::from_le_bytes(word) != a[2] as u32 {
                    return Ok((-11i64) as u64);
                }
                if self.threads.depth > 1 {
                    return Err(format!(
                        "futex wait at 0x{:x} would block in nested run at {}",
                        a[0],
                        self.location(pc)
                    )
                    .into());
                }
                let deadline = if a[3] == 0 {
                    None
                } else {
                    let seconds = self.read_pointer(a[3])? as u128;
                    let nanos = self.read_pointer(a[3] + self.pointer_size())? as u128;
                    Some(self.threads.time + seconds * 1_000_000_000 + nanos)
                };
                self.threads.action = Action::Wait(Wait {
                    address: a[0],
                    deadline,
                    order: self.threads.wait_order,
                });
                self.threads.wait_order += 1;
                Ok(0)
            }
            _ => Ok((-38i64) as u64),
        }
    }

    fn wake_futex(&mut self, address: u64, count: u64) -> u64 {
        let mut woke = 0;
        while woke < count {
            let next = self
                .threads
                .saved
                .iter()
                .enumerate()
                .filter_map(|(i, thread)| {
                    thread
                        .wait
                        .filter(|wait| wait.address == address)
                        .map(|wait| (i, wait.order))
                })
                .min_by_key(|(_, order)| *order);
            let Some((index, _)) = next else { break };
            self.threads.saved[index].wait = None;
            woke += 1;
        }
        woke
    }

    pub(crate) fn exit_thread(&mut self, status: u64, group: bool, pc: u64) -> Result<u64> {
        if !group {
            let address = self.threads.current.clear_tid;
            if address != 0 {
                self.write_bytes(address, &0u32.to_le_bytes())?;
                self.wake_futex(address, 1);
            }
            if self.threads.tid() != MAIN_TID {
                self.threads.action = Action::Exit;
            }
        }
        if group || self.threads.tid() == MAIN_TID || self.threads.depth > 1 {
            return Err(format!(
                "emulation stopped: {}({}) at {}",
                if group { "exit_group" } else { "exit" },
                status as i32,
                self.location(pc)
            )
            .into());
        }
        Ok(0)
    }

    pub(crate) fn switch_thread(&mut self, pc: u64) -> Result<u64> {
        let wait = match self.threads.action {
            Action::Wait(wait) => Some(wait),
            _ => None,
        };
        if !self.threads.has_runnable() {
            if matches!(self.threads.action, Action::Continue | Action::Yield) {
                self.threads.action = Action::Continue;
                return Ok(pc);
            }
            let deadline = self
                .threads
                .saved
                .iter()
                .filter_map(|thread| thread.wait?.deadline)
                .chain(wait.and_then(|wait| wait.deadline))
                .min();
            let Some(deadline) = deadline else {
                let mut waiting: Vec<_> = self
                    .threads
                    .saved
                    .iter()
                    .filter_map(|thread| {
                        thread.wait.map(|wait| {
                            format!("tid {} futex 0x{:x}", thread.identity.tid, wait.address)
                        })
                    })
                    .collect();
                if let Some(wait) = wait {
                    waiting.push(format!(
                        "tid {} futex 0x{:x}",
                        self.threads.tid(),
                        wait.address
                    ));
                }
                return Err(
                    format!("all guest threads are waiting: {}", waiting.join(", ")).into(),
                );
            };
            self.threads.time = deadline;
            if wait.is_some_and(|wait| wait.deadline == Some(deadline)) {
                self.write_reg(0, (-110i64) as u64)?;
                self.threads.action = Action::Continue;
                return Ok(pc);
            }
            let index = self
                .threads
                .saved
                .iter()
                .position(|thread| {
                    thread
                        .wait
                        .is_some_and(|wait| wait.deadline == Some(deadline))
                })
                .unwrap();
            let reg = self.architecture().gp(0);
            let bits = (-110i64) as u64 & self.architecture().pointer_mask();
            self.threads.saved[index].context.reg_write(reg, bits)?;
            self.threads.saved[index].wait = None;
        }
        let index = self
            .threads
            .saved
            .iter()
            .position(|thread| thread.wait.is_none())
            .unwrap();
        let next = self.threads.saved.remove(index).unwrap();
        if !matches!(self.threads.action, Action::Exit) {
            let context = self.cpu()?.context_init()?;
            self.threads.saved.push_back(SavedThread {
                identity: self.threads.current,
                context,
                pc,
                wait,
            });
        }
        self.cpu_mut()?.context_restore(&next.context)?;
        self.threads.current = next.identity;
        self.threads.action = Action::Continue;
        Ok(next.pc)
    }

    pub(crate) fn restore_main_thread(&mut self) -> Result<()> {
        if self.threads.tid() != MAIN_TID {
            let index = self
                .threads
                .saved
                .iter()
                .position(|thread| thread.identity.tid == MAIN_TID)
                .unwrap();
            let main = self.threads.saved.remove(index).unwrap();
            self.cpu_mut()?.context_restore(&main.context)?;
            self.threads.current = main.identity;
        }
        self.threads.action = Action::Continue;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{Architecture, Emulator, Module, RETURN, Result};

    fn libc_emulator(arch: Architecture) -> Result<(Emulator, Module)> {
        let mut emu = Emulator::builder().build()?;
        let libc = emu.load_library(format!("rootfs/{}/libc.so", arch.library_dir()))?;
        Ok((emu, libc))
    }

    fn code(emu: &mut Emulator, offset: u64, words: &[u32]) -> Result<u64> {
        let address = RETURN + offset;
        let bytes: Vec<_> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        emu.write_bytes(address, &bytes)?;
        Ok(address)
    }

    fn call(emu: &mut Emulator, libc: Module, name: &str, args: &[u64]) -> Result<u64> {
        emu.call_function(emu.find_symbol(libc, name)?, args)
    }

    fn spawn(emu: &mut Emulator, libc: Module, entry: u64, argument: u64) -> Result<u64> {
        let out = emu.malloc(8)?;
        assert_eq!(
            call(emu, libc, "pthread_create", &[out, 0, entry, argument])?,
            0
        );
        emu.read_pointer(out)
    }

    fn syscall_code(emu: &mut Emulator, offset: u64, arm32: u32, arm64: u32) -> Result<u64> {
        if emu.architecture() == Architecture::Arm32 {
            // Preserve callee-saved registers and fetch syscall arguments 5 and 6.
            code(
                emu,
                offset,
                &[
                    0xe92d40b0,
                    0xe59d4010,
                    0xe59d5014,
                    0xe3007000 | (arm32 & 0xfff) | ((arm32 & 0xf000) << 4),
                    0xef000000,
                    0xe8bd80b0,
                ],
            )
        } else {
            code(
                emu,
                offset,
                &[0xd2800008 | (arm64 << 5), 0xd4000001, 0xd65f03c0],
            )
        }
    }

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

    fn waiter_code(emu: &mut Emulator) -> Result<u64> {
        // arg = [address, operation, value, timeout, result, counter, wake_order].
        // Wait, record the raw return value and completion order, then return 42.
        code(
            emu,
            0x100,
            if emu.architecture() == Architecture::Arm32 {
                &[
                    0xe92d41f0, 0xe1a06000, 0xe896000f, 0xe3a070f0, 0xef000000, 0xe5860010,
                    0xe5964014, 0xe5945000, 0xe2855001, 0xe5845000, 0xe5865018, 0xe3a0002a,
                    0xe8bd81f0,
                ]
            } else {
                &[
                    0xa9bf7bf3, 0xaa0003f3, 0xa9400660, 0xa9410e62, 0xd2800c48, 0xd4000001,
                    0xf9001260, 0xf9401664, 0xb9400085, 0x110004a5, 0xb9000085, 0xb9003265,
                    0xd2800540, 0xa8c17bf3, 0xd65f03c0,
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
            let error = call(&mut emu, libc, "dl_iterate_phdr", &[callback, address])
                .unwrap_err()
                .to_string();
            assert_eq!(
                error,
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
                    let error = call(&mut emu, libc, "sched_yield", &[])
                        .unwrap_err()
                        .to_string();
                    assert_eq!(
                        error,
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
}
