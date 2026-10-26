use crate::{Emulator, Result, arch::Architecture, vfs::Error};

impl Emulator {
    fn path_argument(&self, address: u64) -> crate::vfs::FsResult<String> {
        self.cstring(address).map_err(|_| Error(14))
    }
    fn file_path(&self, dirfd: u64, address: u64) -> Result<String> {
        Ok(self.guest_path(dirfd, &self.path_argument(address)?)?)
    }
    pub(super) fn file_syscall(&mut self, nr: u64, a: [u64; 6]) -> Result<u64> {
        self.file_operation(nr, a).or_else(super::file_error)
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
            61 => self.getdents(a[0], a[1], a[2])?,
            62 => {
                let position = self.vfs.seek(a[0], a[1], a[2])?;
                if self.architecture() == Architecture::Arm32 && position > i32::MAX as u64 {
                    return Err(Error(75).into());
                }
                position
            }
            63 | 67 => {
                let position = (nr == 67).then_some(a[3]);
                if position.is_some_and(|offset| (offset as i64) < 0) {
                    return Err(Error(22).into());
                }
                self.read_file(a[0], a[1], a[2], position)?
            }
            64 | 68 => self.write_file(a[0], a[1], a[2], (nr == 68).then_some(a[3]))?,
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
                    let path = self.path_argument(a[1])?;
                    if path.is_empty() && a[3] & 0x1000 != 0 {
                        if a[0] as i32 == -100 {
                            self.stat_path("/", true)?
                        } else {
                            self.stat_file(a[0])?
                        }
                    } else {
                        self.stat_path(&self.guest_path(a[0], &path)?, a[3] & 0x100 == 0)?
                    }
                };
                self.write_bytes(if nr == 80 { a[1] } else { a[2] }, &bytes[..size])
                    .map_err(|_| Error(14))?;
                0
            }
            82 | 83 => self.vfs.sync(a[0])?,
            140 => {
                let offset = ((a[1] as u32 as u64) << 32) | a[2] as u32 as u64;
                let position = self.vfs.seek(a[0], offset, a[4])?;
                self.write_u64(a[3], position).map_err(|_| Error(14))?;
                0
            }
            _ => unreachable!(),
        })
    }
}
