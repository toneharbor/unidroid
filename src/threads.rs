use crate::{CpuData, Emulator, Result, jni::FramePosition};
use std::collections::VecDeque;
use unicorn_engine::{Context, Unicorn};

pub(crate) const MAIN_TID: u32 = 1234;
// About 10,000 A64 or ARM instructions; Thumb code runs more per slice.
pub(crate) const SLICE_CODE_BYTES: u64 = 40_000;

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

#[derive(Clone, Copy, Default)]
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
    jni_frame: FramePosition,
    pc: u64,
    wait: Option<Wait>,
}

pub(crate) struct Threads {
    run: bool,
    saved: VecDeque<SavedThread>,
    current: Identity,
    next_tid: u32,
    time: u128,
    wait_order: u64,
    pub(crate) slice_hook_installed: bool,
    pub depth: usize,
    pub action: Action,
}

impl Threads {
    pub(crate) fn new(run: bool) -> Self {
        Self {
            run,
            saved: VecDeque::new(),
            current: Identity {
                tid: MAIN_TID,
                clear_tid: 0,
            },
            next_tid: MAIN_TID + 1,
            time: 0,
            wait_order: 0,
            slice_hook_installed: false,
            depth: 0,
            action: Action::Continue,
        }
    }

    pub(crate) fn copy(&self, cpu: &mut Unicorn<'_, CpuData>) -> Result<Self> {
        let current = cpu.context_init()?;
        let saved = self
            .saved
            .iter()
            .map(|thread| {
                cpu.context_restore(&thread.context)?;
                Ok(SavedThread {
                    identity: thread.identity,
                    context: cpu.context_init()?,
                    jni_frame: thread.jni_frame,
                    pc: thread.pc,
                    wait: thread.wait,
                })
            })
            .collect::<Result<_>>();
        // Restore even if copying a context failed.
        cpu.context_restore(&current)?;
        Ok(Self {
            run: self.run,
            saved: saved?,
            current: self.current,
            next_tid: self.next_tid,
            time: self.time,
            wait_order: self.wait_order,
            slice_hook_installed: false,
            depth: self.depth,
            action: self.action,
        })
    }

    pub(crate) fn enabled(&self) -> bool {
        self.run
    }

    pub(crate) fn has_saved(&self) -> bool {
        !self.saved.is_empty()
    }

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
    pub(crate) fn install_thread_slice_hook(&mut self) -> Result<()> {
        if !self.threads.slice_hook_installed {
            let uc = self.cpu_mut()?;
            uc.add_block_hook(1, 0, |uc, _, size| {
                let remaining = &mut uc.get_data_mut().slice_remaining;
                if *remaining == 0 {
                    return;
                }
                *remaining = remaining.saturating_sub(u64::from(size));
                if *remaining == 0 {
                    let _ = uc.emu_stop();
                }
            })?;
            uc.ctl_flush_tb()?;
            self.threads.slice_hook_installed = true;
        }
        Ok(())
    }

    pub(crate) fn clone_thread(&mut self, a: [u64; 6]) -> Result<u64> {
        const CLONE_VM: u64 = 0x100;
        const CLONE_THREAD: u64 = 0x10000;
        if a[0] & (CLONE_VM | CLONE_THREAD) != CLONE_VM | CLONE_THREAD {
            return Ok((-38i64) as u64);
        }
        let tid = self.threads.next_tid;
        if a[0] & 0x100000 != 0 {
            // CLONE_PARENT_SETTID
            self.write_bytes(a[2], &tid.to_le_bytes())?;
        }
        if a[0] & 0x1000000 != 0 {
            // CLONE_CHILD_SETTID
            self.write_bytes(a[4], &tid.to_le_bytes())?;
        }
        if self.threads.run {
            let arch = self.architecture();
            let mut context = self.cpu()?.context_init()?;
            context.reg_write(arch.gp(0), 0)?;
            context.reg_write(arch.sp(), a[1])?;
            if a[0] & 0x80000 != 0 {
                // CLONE_SETTLS
                context.reg_write(arch.tls(), a[3])?;
            }
            let pc = self.restart_pc()?.0;
            self.install_thread_slice_hook()?;
            self.threads.saved.push_back(SavedThread {
                identity: Identity {
                    tid,
                    clear_tid: if a[0] & 0x200000 != 0 { a[4] } else { 0 },
                },
                context,
                jni_frame: FramePosition::BASE,
                pc,
                wait: None,
            });
        }
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
        if status == 1 && group && self.threads.tid() == MAIN_TID {
            self.events.borrow_mut().emit(crate::Event::Warning {
                message: format!("suppressed exit_group(1) at {}", self.location(pc)),
            });
            return Ok(0);
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
        let context = if matches!(self.threads.action, Action::Exit) {
            self.jni.clear_thread_locals(self.threads.tid());
            None
        } else {
            Some(self.cpu()?.context_init()?)
        };
        self.cpu_mut()?.context_restore(&next.context)?;
        let jni_frame = self.jni.swap_frame(next.jni_frame);
        if let Some(context) = context {
            self.threads.saved.push_back(SavedThread {
                identity: self.threads.current,
                context,
                jni_frame,
                pc,
                wait,
            });
        }
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
            self.jni.clear_thread_locals(self.threads.tid());
            self.jni.swap_frame(main.jni_frame);
            self.threads.current = main.identity;
        }
        self.threads.action = Action::Continue;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
