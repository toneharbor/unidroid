use crate::{Emulator, Result, vfs::Error};

impl Emulator {
    fn file_path(&self, dirfd: u64, address: u64) -> Result<String> {
        Ok(self.guest_path(dirfd, &self.cstring(address)?)?)
    }
    pub(super) fn file_syscall(&mut self, nr: u64, a: [u64; 6]) -> Result<u64> {
        match self.file_operation(nr, a) {
            Ok(result) => Ok(result),
            Err(error) => match error.downcast::<Error>() {
                Ok(error) => Ok(error.result()),
                Err(error) => Err(error),
            },
        }
    }
    fn file_operation(&mut self, nr: u64, a: [u64; 6]) -> Result<u64> {
        Ok(match nr {
            23 => self.vfs.dup(a[0], 0, false)?,
            24 => self.vfs.dup3(a[0], a[1], a[2])?,
            25 => self.vfs.fcntl(a[0], a[1], a[2])?,
            34 => {
                let path = self.file_path(a[0], a[1])?;
                self.mkdir(&path, a[2] as u32)?
            }
            35 => {
                let path = self.file_path(a[0], a[1])?;
                self.unlink(&path, a[2])?
            }
            38 | 276 => {
                if nr == 276 && a[4] != 0 {
                    return Err(Error(22).into());
                }
                let from = self.file_path(a[0], a[1])?;
                let to = self.file_path(a[2], a[3])?;
                self.rename(&from, &to)?
            }
            46 => self.vfs.truncate(a[0], a[1])?,
            48 => {
                self.guest_node(&self.file_path(a[0], a[1])?)?;
                0
            }
            56 => {
                let path = self.file_path(a[0], a[1])?;
                self.open_file(&path, a[2] as u32, a[3] as u32)?
            }
            57 => self.vfs.close(a[0])?,
            61 => {
                let bytes = self.vfs.getdents(a[0], a[2])?;
                self.write_bytes(a[1], &bytes).map_err(|_| Error(14))?;
                bytes.len() as u64
            }
            62 => self.vfs.seek(a[0], a[1], a[2])?,
            63 | 67 => {
                let position = (nr == 67).then_some(a[3]);
                if position.is_some_and(|offset| (offset as i64) < 0) {
                    return Err(Error(22).into());
                }
                let bytes = self.read_file_bytes(a[0], a[2], position)?;
                self.write_bytes(a[1], &bytes).map_err(|_| Error(14))?;
                bytes.len() as u64
            }
            68 => {
                self.vfs.writable(a[0])?;
                if (a[3] as i64) < 0 {
                    return Err(Error(22).into());
                }
                let bytes = self
                    .read_bytes(a[1], a[2].min(crate::vfs::IO_LIMIT) as usize)
                    .map_err(|_| Error(14))?;
                self.write_file_bytes(a[0], &bytes, Some(a[3]))?
            }
            78 => {
                if a[3] == 0 {
                    return Err(Error(22).into());
                }
                let bytes = self.read_link(&self.file_path(a[0], a[1])?)?;
                let count = bytes.len().min(a[3] as usize);
                self.write_bytes(a[2], &bytes[..count])
                    .map_err(|_| Error(14))?;
                count as u64
            }
            79 | 80 => {
                let (bytes, size) = if nr == 80 {
                    self.stat_file(a[0])?
                } else {
                    self.stat_path(&self.file_path(a[0], a[1])?)?
                };
                self.write_bytes(if nr == 80 { a[1] } else { a[2] }, &bytes[..size])
                    .map_err(|_| Error(14))?;
                0
            }
            82 | 83 => self.vfs.sync(a[0])?,
            _ => unreachable!(),
        })
    }
}
