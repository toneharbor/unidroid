use super::*;

impl Vfs {
    pub(crate) fn sync(&self, fd: u64) -> FsResult<u64> {
        self.descriptor(fd).map(|_| 0)
    }
    pub(crate) fn truncate(&mut self, fd: u64, length: u64) -> FsResult<u64> {
        if (length as i64) < 0 {
            return Err(Error(22));
        }
        let file = self.descriptor(fd)?.file.borrow();
        if file.flags & 3 == 0 {
            return Err(Error(22));
        }
        let mut node = file.node.borrow_mut();
        let Data::Bytes(bytes) = &mut node.data else {
            return Err(Error(22));
        };
        if length > FILE_LIMIT {
            return Err(Error(27));
        }
        Rc::make_mut(bytes).resize(length as usize, 0);
        node.changed = now();
        Ok(0)
    }
    pub(crate) fn writable(&self, fd: u64) -> FsResult<()> {
        self.descriptor(fd)?.file.borrow().writable()
    }
    pub(crate) fn seek(&mut self, fd: u64, offset: u64, whence: u64) -> FsResult<u64> {
        self.descriptor(fd)?.file.borrow_mut().seek(offset, whence)
    }
}
impl Emulator {
    pub(crate) fn read_file_bytes(
        &mut self,
        fd: u64,
        count: u64,
        position: Option<u64>,
    ) -> Result<Vec<u8>> {
        let shared = self.vfs.descriptor(fd)?.file.clone();
        let mut file = shared.borrow_mut();
        let offset = position.unwrap_or(file.offset);
        let mut bytes = vec![0; file.read_length(count, offset)?];
        let random = matches!(file.node.borrow().data, Data::Random);
        let count = if random {
            self.source.random(&mut bytes)?
        } else {
            file.read(&mut bytes, offset)?
        };
        if position.is_none() && !random {
            file.offset += count as u64;
        }
        bytes.truncate(count);
        Ok(bytes)
    }
    pub(crate) fn write_file_bytes(
        &mut self,
        fd: u64,
        bytes: &[u8],
        position: Option<u64>,
    ) -> FsResult<u64> {
        let shared = self.vfs.descriptor(fd)?.file.clone();
        let mut file = shared.borrow_mut();
        file.writable()?;
        let standard = match file.node.borrow().data {
            Data::Standard(fd) => Some(fd),
            _ => None,
        };
        if let Some(fd) = standard {
            if position.is_some() {
                return Err(Error(29));
            }
            self.events.borrow_mut().emit(crate::Event::Output {
                fd,
                text: String::from_utf8_lossy(bytes).into_owned(),
            });
            Ok(bytes.len() as u64)
        } else {
            Ok(file.write(bytes, position)? as u64)
        }
    }
}
