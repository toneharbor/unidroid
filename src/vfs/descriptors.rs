use super::*;
use std::{fs::File, os::unix::fs::FileExt};

const O_DIRECTORY: u32 = 0x4000;

pub(super) struct Descriptor {
    pub file: Rc<RefCell<OpenFile>>,
    pub cloexec: bool,
}
pub(super) struct OpenFile {
    pub node: NodeRef,
    pub path: String,
    pub offset: u64,
    pub flags: u32,
    pub host: Option<File>,
    pub entries: Option<Rc<Vec<super::directories::DirEntry>>>,
}
impl OpenFile {
    pub fn writable(&self) -> FsResult<()> {
        if self.flags & 3 == 0 {
            Err(Error(9))
        } else {
            Ok(())
        }
    }
    pub fn length(&self) -> FsResult<u64> {
        let node = self.node.borrow();
        Ok(match &node.data {
            Data::Bytes(bytes) | Data::Generated(bytes) => bytes.len() as u64,
            Data::Host(_) => self.host.as_ref().unwrap().metadata()?.len(),
            _ => 0,
        })
    }
    pub fn read_length(&self, count: u64, offset: u64) -> FsResult<usize> {
        if self.flags & 3 == 1 {
            return Err(Error(9));
        }
        let node = self.node.borrow();
        if node.is_dir() {
            return Err(Error(21));
        }
        let available = if matches!(node.data, Data::Random) {
            IO_LIMIT
        } else {
            self.length()?.saturating_sub(offset)
        };
        Ok(count.min(available) as usize)
    }
    pub fn read(&self, buffer: &mut [u8], offset: u64) -> FsResult<usize> {
        let node = self.node.borrow();
        match &node.data {
            Data::Bytes(bytes) | Data::Generated(bytes) => {
                let start = offset.min(bytes.len() as u64) as usize;
                let count = buffer.len().min(bytes.len() - start);
                buffer[..count].copy_from_slice(&bytes[start..start + count]);
                Ok(count)
            }
            Data::Host(_) => Ok(self.host.as_ref().unwrap().read_at(buffer, offset)?),
            Data::Standard(_) => Ok(0),
            _ => Err(Error(22)),
        }
    }
    pub fn write(&mut self, bytes: &[u8], position: Option<u64>) -> FsResult<usize> {
        self.writable()?;
        let mut node = self.node.borrow_mut();
        let Data::Bytes(data) = &mut node.data else {
            return Err(Error(22));
        };
        let offset = if self.flags & 0x400 != 0 {
            data.len() as u64
        } else {
            position.unwrap_or(self.offset)
        };
        if offset >= FILE_LIMIT {
            return Err(Error(27));
        }
        let count = bytes.len().min((FILE_LIMIT - offset) as usize);
        let end = offset as usize + count;
        if count != 0 {
            let data = Rc::make_mut(data);
            data.resize(data.len().max(end), 0);
            data[offset as usize..end].copy_from_slice(&bytes[..count]);
            node.changed = None;
        }
        if position.is_none() {
            self.offset = end as u64;
        }
        Ok(count)
    }
    pub fn seek(&mut self, offset: u64, whence: u64) -> FsResult<u64> {
        if matches!(self.node.borrow().data, Data::Random) {
            return Ok(0);
        }
        let base = match whence {
            0 => 0,
            1 => self.offset,
            2 => self.length()?,
            _ => return Err(Error(22)),
        };
        self.offset = base
            .checked_add_signed(offset as i64)
            .filter(|offset| *offset <= i64::MAX as u64)
            .ok_or(Error(22))?;
        Ok(self.offset)
    }
}
impl Vfs {
    pub(super) fn descriptor(&self, fd: u64) -> FsResult<&Descriptor> {
        self.fds.get(&fd).ok_or(Error(9))
    }
    pub(super) fn lowest_fd(&self, minimum: u64) -> FsResult<u64> {
        (minimum..=i32::MAX as u64)
            .find(|fd| !self.fds.contains_key(fd))
            .ok_or(Error(24))
    }
    pub(crate) fn close(&mut self, fd: u64) -> FsResult<u64> {
        self.fds.remove(&fd).map(|_| 0).ok_or(Error(9))
    }
}
impl Emulator {
    pub(crate) fn guest_path(&self, dirfd: u64, path: &str) -> FsResult<String> {
        if path.is_empty() {
            return Err(Error(2));
        }
        if path.starts_with('/') {
            return Ok(resolution::normalize(path));
        }
        let base = if dirfd as i32 == -100 {
            "/".to_owned()
        } else {
            let file = self.vfs.descriptor(dirfd)?.file.borrow();
            if !file.node.borrow().is_dir() {
                return Err(Error(20));
            }
            file.path.clone()
        };
        Ok(resolution::normalize(&format!("{base}/{path}")))
    }
    pub(crate) fn open_file(&mut self, path: &str, flags: u32, mode: u32) -> FsResult<u64> {
        let exclusive = flags & 0xc0 == 0xc0;
        let resolved = self.resolve_path(path, !exclusive)?;
        let path = resolved.as_str();
        let existing = self.node_at(path);
        let node = match existing {
            Ok(_) if exclusive => return Err(Error(17)),
            Ok(node) => node,
            Err(Error(2)) if flags & 0x40 != 0 => {
                self.require_parent(path)?;
                let node = Node::new(Data::Bytes(Rc::new(Vec::new())), mode & 0o7777 | 0o100000);
                self.vfs.overlay.insert(path.into(), node.clone());
                node
            }
            Err(error) => return Err(error),
        };
        if flags & O_DIRECTORY != 0 && !node.borrow().is_dir() {
            return Err(Error(20));
        }
        if flags & (3 | 0x200) != 0 {
            if node.borrow().is_dir() {
                return Err(Error(21));
            }
            if matches!(node.borrow().data, Data::Generated(_) | Data::Random) {
                return Err(Error(13));
            }
            if matches!(node.borrow().data, Data::Host(_)) {
                let bytes = if flags & 0x200 != 0 {
                    Vec::new()
                } else {
                    node.borrow().copy_bytes()?
                };
                node.borrow_mut().data = Data::Bytes(Rc::new(bytes));
                node.borrow_mut().changed = None;
                self.vfs.overlay.insert(path.into(), node.clone());
            }
        }
        if flags & 0x200 != 0 {
            let mut node = node.borrow_mut();
            if let Data::Bytes(bytes) = &mut node.data {
                *bytes = Rc::new(Vec::new());
                node.changed = None;
            }
        }
        let host = match &node.borrow().data {
            Data::Host(path) => Some(File::open(path)?),
            _ => None,
        };
        let entries = if node.borrow().is_dir() {
            Some(Rc::new(self.directory_entries(path)?))
        } else {
            None
        };
        let fd = self.vfs.lowest_fd(0)?;
        self.vfs.fds.insert(
            fd,
            Descriptor {
                file: Rc::new(RefCell::new(OpenFile {
                    node,
                    path: path.into(),
                    offset: 0,
                    flags: flags & (3 | 0x400 | 0x800),
                    host,
                    entries,
                })),
                cloexec: flags & 0x80000 != 0,
            },
        );
        Ok(fd)
    }
    pub(super) fn require_parent(&self, path: &str) -> FsResult<()> {
        if self.guest_node(resolution::parent(path))?.borrow().is_dir() {
            Ok(())
        } else {
            Err(Error(20))
        }
    }
}
