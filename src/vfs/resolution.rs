use super::*;
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};

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

pub(super) enum GeneratedKind {
    Directory,
    Random,
    Cmdline,
    Maps,
}

pub(super) fn generated_paths() -> &'static [(String, GeneratedKind)] {
    use GeneratedKind::*;
    static PATHS: std::sync::OnceLock<Vec<(String, GeneratedKind)>> = std::sync::OnceLock::new();
    PATHS.get_or_init(|| {
        let pid = crate::threads::MAIN_TID;
        [
            ("/", Directory),
            ("/dev", Directory),
            ("/dev/random", Random),
            ("/dev/urandom", Random),
            ("/proc", Directory),
            ("/proc/self", Directory),
            ("/proc/self/maps", Maps),
            ("/proc/self/cmdline", Cmdline),
            ("/proc/self/task", Directory),
            ("/proc/self/fd", Directory),
        ]
        .into_iter()
        .map(|(path, kind)| (path.into(), kind))
        .chain([
            (format!("/proc/{pid}"), Directory),
            (format!("/proc/{pid}/maps"), Maps),
            (format!("/proc/{pid}/cmdline"), Cmdline),
            (format!("/proc/{pid}/fd"), Directory),
            (format!("/proc/self/task/{pid}"), Directory),
            (format!("/proc/self/task/{pid}/maps"), Maps),
        ])
        .collect()
    })
}
impl Emulator {
    pub(super) fn rootfs_path(&self, path: &str) -> FsResult<PathBuf> {
        let mut host = self.rootfs.clone();
        let mut parts = path.split('/').filter(|part| !part.is_empty()).peekable();
        while let Some(part) = parts.next() {
            host.push(part);
            // A guest overlay directory can cover a host symlink. Never follow
            // that hidden host link when looking for lower-layer children.
            if parts.peek().is_some() {
                let metadata = std::fs::symlink_metadata(&host)?;
                if metadata.is_symlink() {
                    return Err(Error(2));
                }
                if !metadata.is_dir() {
                    return Err(Error(20));
                }
            }
        }
        Ok(host)
    }
    pub(super) fn resolve_path(&self, path: &str, follow_last: bool) -> FsResult<String> {
        let mut pending = normalize(path);
        let mut links = 0;
        'lookup: loop {
            if pending != "/" && !self.node_at("/")?.borrow().is_dir() {
                return Err(Error(20));
            }
            let mut resolved = String::new();
            let mut parts = pending
                .split('/')
                .filter(|part| !part.is_empty())
                .peekable();
            while let Some(part) = parts.next() {
                let parent = resolved.clone();
                resolved.push('/');
                resolved.push_str(part);
                let last = parts.peek().is_none();
                if last && !follow_last {
                    return Ok(resolved);
                }
                let node = match self.node_at(&resolved) {
                    Ok(node) => node,
                    Err(Error(2)) if last => return Ok(resolved),
                    Err(error) => return Err(error),
                };
                let node = node.borrow();
                if let Data::Symlink(target, _) = &node.data {
                    links += 1;
                    if links > 40 {
                        return Err(Error(40));
                    }
                    let target = String::from_utf8_lossy(target);
                    let target = if target.starts_with('/') {
                        target.into_owned()
                    } else {
                        format!("{parent}/{target}")
                    };
                    pending =
                        normalize(&format!("{target}/{}", parts.collect::<Vec<_>>().join("/")));
                    continue 'lookup;
                }
                if !last && !node.is_dir() {
                    return Err(Error(20));
                }
            }
            return Ok(if resolved.is_empty() {
                "/".into()
            } else {
                resolved
            });
        }
    }
    pub(crate) fn guest_node(&self, path: &str) -> FsResult<NodeRef> {
        self.node_at(&self.resolve_path(path, true)?)
    }
    pub(super) fn node_at(&self, path: &str) -> FsResult<NodeRef> {
        if let Some(node) = self.vfs.overlay.get(path) {
            return Ok(node.clone());
        }
        if self.vfs.hidden(path) {
            return Err(Error(2));
        }
        if let Some((_, kind)) = generated_paths().iter().find(|(entry, _)| entry == path) {
            return Ok(match kind {
                GeneratedKind::Directory => Node::new(Data::Directory, 0o040755),
                GeneratedKind::Random => Node::new(Data::Random, 0o020666),
                GeneratedKind::Cmdline => {
                    let mut bytes = self.process_name.as_bytes().to_vec();
                    bytes.push(0);
                    Node::new(Data::Generated(Rc::new(bytes)), 0o100000)
                }
                GeneratedKind::Maps => {
                    let bytes = self.proc_maps().map_err(|_| Error(5))?;
                    Node::new(Data::Generated(Rc::new(bytes)), 0o100000)
                }
            });
        }
        for descriptor in self.vfs.fds.values() {
            let file = descriptor.file.borrow();
            if file.path == path && matches!(file.node.borrow().data, Data::Host(_)) {
                return Ok(file.node.clone());
            }
        }
        let host = self.rootfs_path(path)?;
        let metadata = std::fs::symlink_metadata(&host)?;
        if metadata.is_symlink() {
            let target = std::fs::read_link(host)?.as_os_str().as_bytes().to_vec();
            return Ok(Node::new(Data::Symlink(target, metadata), 0o120777));
        }
        Ok(Node::new(Data::Host(host), metadata.mode()))
    }
}
