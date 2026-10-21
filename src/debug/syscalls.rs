use crate::{Emulator, Event, arch::Architecture};

fn description(number: u64) -> Option<(&'static str, usize)> {
    Some(match number {
        56 => ("openat", 4),
        57 => ("close", 1),
        62 => ("lseek", 3),
        63 => ("read", 3),
        67 => ("pread64", 4),
        79 => ("newfstatat", 4),
        80 => ("fstat", 2),
        93 => ("exit", 1),
        94 => ("exit_group", 1),
        96 => ("set_tid_address", 1),
        98 => ("futex", 6),
        99 => ("set_robust_list", 2),
        101 => ("nanosleep", 2),
        113 => ("clock_gettime", 2),
        115 => ("clock_nanosleep", 4),
        124 => ("sched_yield", 0),
        132 => ("sigaltstack", 2),
        135 => ("rt_sigprocmask", 4),
        167 => ("prctl", 5),
        169 => ("gettimeofday", 2),
        172 => ("getpid", 0),
        174 => ("getuid", 0),
        175 => ("geteuid", 0),
        176 => ("getgid", 0),
        177 => ("getegid", 0),
        178 => ("gettid", 0),
        214 => ("brk", 1),
        215 => ("munmap", 2),
        220 => ("clone", 5),
        222 => ("mmap", 6),
        226 => ("mprotect", 3),
        233 => ("madvise", 3),
        _ => return None,
    })
}

// Linux guest errno values, independent of the host OS.
fn errno(result: i64) -> Option<String> {
    if !(-4095..=-1).contains(&result) {
        return None;
    }
    let name = match -result {
        1 => "EPERM",
        2 => "ENOENT",
        3 => "ESRCH",
        4 => "EINTR",
        5 => "EIO",
        9 => "EBADF",
        11 => "EAGAIN",
        12 => "ENOMEM",
        13 => "EACCES",
        14 => "EFAULT",
        16 => "EBUSY",
        17 => "EEXIST",
        19 => "ENODEV",
        20 => "ENOTDIR",
        21 => "EISDIR",
        22 => "EINVAL",
        24 => "EMFILE",
        28 => "ENOSPC",
        30 => "EROFS",
        32 => "EPIPE",
        38 => "ENOSYS",
        110 => "ETIMEDOUT",
        _ => return None,
    };
    Some(name.into())
}

impl Emulator {
    pub(crate) fn syscall_event(
        &self,
        guest: u64,
        number: u64,
        args: [u64; 6],
        pc: u64,
        lr: u64,
    ) -> Event {
        let (name, count) = description(number).map_or_else(
            || (format!("syscall_{guest}"), 6),
            |(name, count)| (name.into(), count),
        );
        let args = args[..count]
            .iter()
            .enumerate()
            .map(|(i, &value)| {
                if matches!(number, 56 | 79) && i == 1 {
                    self.cstring(value)
                        .map(|s| format!("{s:?}"))
                        .unwrap_or_else(|_| format!("0x{value:x}"))
                } else {
                    format!("0x{value:x}")
                }
            })
            .collect();
        Event::Syscall {
            name,
            number: guest,
            args,
            result: None,
            errno: None,
            at: self.symbolize(pc),
            from: self.symbolize(lr & !1),
        }
    }

    pub(crate) fn emit_syscall(&mut self, mut event: Event, result: Option<u64>) {
        let signed = result.map(|value| match self.architecture() {
            Architecture::Arm32 => value as i32 as i64,
            Architecture::Arm64 => value as i64,
        });
        if let Event::Syscall {
            result,
            errno: error,
            ..
        } = &mut event
        {
            *result = signed;
            *error = signed.and_then(errno);
        }
        self.events.borrow_mut().emit(event);
    }
}
