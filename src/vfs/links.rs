use super::*;

impl Emulator {
    pub(crate) fn read_link(&self, path: &str) -> FsResult<Vec<u8>> {
        let resolved = self.resolve_path(path, false)?;
        let path = resolved.as_str();
        if let Some(node) = self.vfs.overlay.get(path) {
            return match &node.borrow().data {
                Data::Symlink(target, _) => Ok(target.clone()),
                _ => Err(Error(22)),
            };
        }
        if self.vfs.hidden(path) {
            return Err(Error(2));
        }
        self.require_parent(path)?;
        let pid = crate::threads::MAIN_TID;
        for prefix in ["/proc/self/fd/".to_owned(), format!("/proc/{pid}/fd/")] {
            if let Some(fd) = path.strip_prefix(&prefix) {
                let fd = fd.parse::<u64>().map_err(|_| Error(2))?;
                return Ok(self
                    .vfs
                    .descriptor(fd)
                    .map_err(|_| Error(2))?
                    .file
                    .borrow()
                    .path
                    .as_bytes()
                    .to_vec());
            }
        }
        if path == "/proc/self/exe" {
            let bits = if self.architecture() == crate::Architecture::Arm64 {
                64
            } else {
                32
            };
            return Ok(format!("/system/bin/app_process{bits}").into_bytes());
        }
        match &self.node_at(path)?.borrow().data {
            Data::Symlink(target, _) => Ok(target.clone()),
            _ => Err(Error(22)),
        }
    }
}
