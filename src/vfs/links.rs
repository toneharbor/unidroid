use super::*;
use std::os::unix::ffi::OsStrExt;

impl Emulator {
    pub(crate) fn read_link(&self, path: &str) -> FsResult<Vec<u8>> {
        if self.vfs.overlay.contains_key(path) {
            return Err(Error(22));
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
        // Resolve the parent only: dangling links still have a readable target.
        let parent = resolution::parent(path);
        if !self.vfs.hidden(parent) {
            let host = self
                .rootfs_path(parent)
                .map(|parent| parent.join(path.rsplit('/').next().unwrap()));
            if let Ok(host) = host
                && let Ok(metadata) = std::fs::symlink_metadata(&host)
                && metadata.is_symlink()
            {
                return Ok(std::fs::read_link(host)?.as_os_str().as_bytes().to_vec());
            }
        }
        self.guest_node(path)?;
        Err(Error(22))
    }
}
