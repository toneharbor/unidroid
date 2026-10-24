use super::*;

impl Vfs {
    pub(crate) fn dup(&mut self, fd: u64, minimum: u64, cloexec: bool) -> FsResult<u64> {
        let file = self.descriptor(fd)?.file.clone();
        let target = self.lowest_fd(minimum)?;
        self.fds.insert(target, Descriptor { file, cloexec });
        Ok(target)
    }
    pub(crate) fn dup3(&mut self, fd: u64, target: u64, flags: u64) -> FsResult<u64> {
        if fd == target || flags & !0x80000 != 0 {
            return Err(Error(22));
        }
        let file = self.descriptor(fd)?.file.clone();
        if target > i32::MAX as u64 {
            return Err(Error(9));
        }
        self.fds.insert(
            target,
            Descriptor {
                file,
                cloexec: flags != 0,
            },
        );
        Ok(target)
    }
    pub(crate) fn fcntl(&mut self, fd: u64, command: u64, arg: u64) -> FsResult<u64> {
        let descriptor = self.descriptor(fd)?;
        match command {
            0 | 1030 => {
                if arg > i32::MAX as u64 {
                    return Err(Error(22));
                }
                self.dup(fd, arg, command == 1030)
            }
            1 => Ok(u64::from(descriptor.cloexec)),
            2 => {
                self.fds.get_mut(&fd).unwrap().cloexec = arg & 1 != 0;
                Ok(0)
            }
            3 => Ok(descriptor.file.borrow().flags as u64),
            4 => {
                let mut file = descriptor.file.borrow_mut();
                file.flags = (file.flags & 3) | (arg as u32 & (0x400 | 0x800));
                Ok(0)
            }
            _ => Err(Error(22)),
        }
    }
}
