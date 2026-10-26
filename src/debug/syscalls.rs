use crate::{Emulator, Event, arch::Architecture};

#[cfg(test)]
mod tests;

fn description(arch: Architecture, number: u64) -> Option<(&'static str, usize)> {
    Some(match number {
        23 => ("dup", 1),
        24 => ("dup3", 3),
        25 => ("fcntl", 3),
        34 => ("mkdirat", 3),
        35 => ("unlinkat", 3),
        38 => ("renameat", 4),
        46 => ("ftruncate", 2),
        48 => ("faccessat", 4),
        56 => ("openat", 4),
        57 => ("close", 1),
        61 => ("getdents64", 3),
        62 => ("lseek", 3),
        63 => ("read", 3),
        64 => ("write", 3),
        66 => ("writev", 3),
        67 => ("pread64", 4),
        68 => ("pwrite64", 4),
        78 => ("readlinkat", 4),
        79 => ("newfstatat", 4),
        80 => ("fstat", 2),
        82 => ("fsync", 1),
        83 => ("fdatasync", 1),
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
        140 if arch == Architecture::Arm32 => ("_llseek", 5),
        160 => ("uname", 1),
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
        276 => ("renameat2", 5),
        _ => return None,
    })
}

impl Emulator {
    // Linux guest errno values, independent of the host OS.
    pub(crate) fn errno(result: i64) -> Option<&'static str> {
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
            27 => "EFBIG",
            28 => "ENOSPC",
            29 => "ESPIPE",
            30 => "EROFS",
            32 => "EPIPE",
            38 => "ENOSYS",
            39 => "ENOTEMPTY",
            40 => "ELOOP",
            75 => "EOVERFLOW",
            110 => "ETIMEDOUT",
            _ => return None,
        };
        Some(name)
    }

    pub(crate) fn syscall_event(
        &self,
        guest: u64,
        number: u64,
        args: [u64; 6],
        pc: u64,
        lr: u64,
    ) -> Event {
        let (name, count) = description(self.architecture(), number).map_or_else(
            || (format!("syscall_{guest}"), 6),
            |(name, count)| (name.into(), count),
        );
        let strings = u8::from(matches!(number, 34 | 35 | 38 | 56 | 78 | 79 | 276)) << 1
            | u8::from(matches!(number, 38 | 276)) << 3;
        let args = super::arguments::arguments(
            self.cpu().expect("syscall CPU"),
            args[..count].iter().copied(),
            strings,
        );
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
            *error = signed.and_then(Self::errno).map(str::to_owned);
        }
        self.events.borrow_mut().emit(event);
    }
}
