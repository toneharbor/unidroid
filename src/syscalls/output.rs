use super::files::IO_LIMIT;
use crate::{Emulator, Event};

impl Emulator {
    pub(super) fn write_output(&self, fd: u64, address: u64, length: u64) -> u64 {
        if !matches!(fd, 1 | 2) {
            return (-9i64) as u64;
        }
        let Ok(bytes) = self.read_bytes(address, length.min(IO_LIMIT) as usize) else {
            return (-14i64) as u64;
        };
        self.emit_output(fd, &bytes)
    }

    pub(super) fn writev_output(&self, fd: u64, address: u64, count: u64) -> u64 {
        if !matches!(fd, 1 | 2) {
            return (-9i64) as u64;
        }
        if count > 1024 {
            return (-22i64) as u64;
        }
        let width = self.pointer_size() as usize;
        let Ok(iovecs) = self.read_bytes(address, count as usize * 2 * width) else {
            return (-14i64) as u64;
        };
        let word = |bytes: &[u8]| {
            let mut value = [0; 8];
            value[..width].copy_from_slice(bytes);
            u64::from_le_bytes(value)
        };
        let mut bytes = Vec::new();
        for iovec in iovecs.chunks_exact(2 * width) {
            let length = word(&iovec[width..]).min(IO_LIMIT - bytes.len() as u64) as usize;
            if length == 0 {
                continue;
            }
            let Ok(buffer) = self.read_bytes(word(&iovec[..width]), length) else {
                return (-14i64) as u64;
            };
            bytes.extend_from_slice(&buffer);
            if bytes.len() as u64 == IO_LIMIT {
                break;
            }
        }
        self.emit_output(fd, &bytes)
    }

    fn emit_output(&self, fd: u64, bytes: &[u8]) -> u64 {
        self.events.borrow_mut().emit(Event::Output {
            fd,
            text: String::from_utf8_lossy(bytes).into_owned(),
        });
        bytes.len() as u64
    }
}
