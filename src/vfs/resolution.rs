use super::*;
use std::os::unix::fs::MetadataExt;

pub(super) fn normalize(path: &str) -> String {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    format!("/{}", parts.join("/"))
}
pub(super) fn absolute(path: &str) -> Result<String> {
    if !path.starts_with('/') {
        return Err(format!("{path}: {} (path must be absolute)", Error(22)).into());
    }
    if path.contains('\0') {
        return Err(format!("{path:?}: {} (path contains NUL)", Error(22)).into());
    }
    Ok(normalize(path))
}
pub(super) fn parent(path: &str) -> &str {
    let index = path.rfind('/').unwrap();
    if index == 0 { "/" } else { &path[..index] }
}
pub(super) fn under(path: &str, ancestor: &str) -> bool {
    path == ancestor
        || ancestor == "/"
        || path
            .strip_prefix(ancestor)
            .is_some_and(|tail| tail.starts_with('/'))
}

pub(super) fn generated_paths() -> Vec<String> {
    let pid = crate::threads::MAIN_TID;
    [
        "/dev",
        "/dev/random",
        "/dev/urandom",
        "/proc",
        "/proc/self",
        "/proc/self/maps",
        "/proc/self/cmdline",
        "/proc/self/task",
    ]
    .into_iter()
    .map(String::from)
    .chain([
        format!("/proc/{pid}"),
        format!("/proc/{pid}/maps"),
        format!("/proc/{pid}/cmdline"),
        format!("/proc/self/task/{pid}"),
        format!("/proc/self/task/{pid}/maps"),
    ])
    .collect()
}
impl Emulator {
    pub(super) fn rootfs_path(&self, path: &str) -> FsResult<PathBuf> {
        let root = self.rootfs.canonicalize()?;
        let host = root.join(path.trim_start_matches('/')).canonicalize()?;
        if !host.starts_with(root) {
            return Err(Error(2));
        }
        Ok(host)
    }
    pub(crate) fn guest_node(&self, path: &str) -> FsResult<NodeRef> {
        if path != "/" && !self.guest_node(parent(path))?.borrow().is_dir() {
            return Err(Error(20));
        }
        if let Some(node) = self.vfs.overlay.get(path) {
            return Ok(node.clone());
        }
        if self.vfs.hidden(path) {
            return Err(Error(2));
        }
        let pid = crate::threads::MAIN_TID;
        if path == "/dev/urandom" || path == "/dev/random" {
            return Ok(Node::new(Data::Random, 0o020666));
        }
        if path == "/proc/self/cmdline" || path == format!("/proc/{pid}/cmdline") {
            let mut bytes = self.process_name.as_bytes().to_vec();
            bytes.push(0);
            return Ok(Node::new(Data::Generated(Rc::new(bytes)), 0o100000));
        }
        if path == "/proc/self/maps"
            || path == format!("/proc/{pid}/maps")
            || path == format!("/proc/self/task/{pid}/maps")
        {
            let bytes = self.proc_maps().map_err(|_| Error(5))?;
            return Ok(Node::new(Data::Generated(Rc::new(bytes)), 0o100000));
        }
        if [
            "/",
            "/dev",
            "/proc",
            "/proc/self",
            "/proc/self/task",
            "/proc/self/fd",
            &format!("/proc/{pid}/fd"),
            &format!("/proc/{pid}"),
            &format!("/proc/self/task/{pid}"),
        ]
        .contains(&path)
        {
            return Ok(Node::new(Data::Directory, 0o040755));
        }
        for descriptor in self.vfs.fds.values() {
            let file = descriptor.file.borrow();
            if file.path == path && matches!(file.node.borrow().data, Data::Host(_)) {
                return Ok(file.node.clone());
            }
        }
        let host = self.rootfs_path(path)?;
        let metadata = std::fs::metadata(&host)?;
        Ok(Node::new(Data::Host(host), metadata.mode()))
    }
}
