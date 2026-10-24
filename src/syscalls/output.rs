use crate::{
    Emulator,
    vfs::{Error, IO_LIMIT},
};

impl Emulator {
    pub(super) fn write_output(&mut self, fd: u64, address: u64, length: u64) -> u64 {
        if let Err(error) = self.vfs.writable(fd) {
            return error.result();
        }
        let Ok(bytes) = self.read_bytes(address, length.min(IO_LIMIT) as usize) else {
            return (-14i64) as u64;
        };
        self.write_file_bytes(fd, &bytes, None)
            .unwrap_or_else(Error::result)
    }

    pub(super) fn writev_output(&mut self, fd: u64, address: u64, count: u64) -> u64 {
        if let Err(error) = self.vfs.writable(fd) {
            return error.result();
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
        self.write_file_bytes(fd, &bytes, None)
            .unwrap_or_else(Error::result)
    }
}
