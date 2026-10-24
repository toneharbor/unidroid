use crate::{Emulator, Result};
use std::os::unix::fs::MetadataExt;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    rc::Rc,
};

mod checkpoint;
mod descriptors;
mod directories;
mod fcntl;
mod io;
mod links;
mod overlay;
mod resolution;
mod stat;
use descriptors::{Descriptor, OpenFile};
#[cfg(test)]
mod tests;

pub(crate) const IO_LIMIT: u64 = 33_554_431;
pub(crate) const FILE_LIMIT: u64 = 1 << 30;
pub(crate) type FsResult<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Error(pub u64);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match Emulator::errno(-(self.0 as i64)) {
            Some(name) => f.write_str(name),
            None => write!(f, "guest filesystem errno {}", self.0),
        }
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        use std::io::ErrorKind;
        Self(match error.kind() {
            ErrorKind::NotFound => 2,
            ErrorKind::PermissionDenied => 13,
            ErrorKind::AlreadyExists => 17,
            ErrorKind::NotADirectory => 20,
            ErrorKind::IsADirectory => 21,
            ErrorKind::InvalidInput => 22,
            ErrorKind::DirectoryNotEmpty => 39,
            _ => 5,
        })
    }
}
impl Error {
    pub fn result(self) -> u64 {
        (-(self.0 as i64)) as u64
    }
}

pub(super) type NodeRef = Rc<RefCell<Node>>;
#[derive(Clone)]
pub(super) struct Node {
    data: Data,
    mode: u32,
    changed: (i64, i64),
}
#[derive(Clone)]
pub(super) enum Data {
    Bytes(Rc<Vec<u8>>),
    Host(PathBuf),
    Directory,
    Generated(Rc<Vec<u8>>),
    Random,
    Standard(u64),
}
impl Node {
    fn new(data: Data, mode: u32) -> NodeRef {
        Rc::new(RefCell::new(Self {
            data,
            mode,
            changed: now(),
        }))
    }
    fn is_dir(&self) -> bool {
        self.mode & 0o170000 == 0o040000
    }
    fn bytes(&self) -> FsResult<Vec<u8>> {
        if self.is_dir() {
            return Err(Error(21));
        }
        match &self.data {
            Data::Bytes(bytes) | Data::Generated(bytes) => Ok(bytes.as_ref().clone()),
            Data::Host(path) => Ok(std::fs::read(path)?),
            _ => Err(Error(22)),
        }
    }
}
fn now() -> (i64, i64) {
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    (time.as_secs() as i64, time.subsec_nanos() as i64)
}
pub(super) fn inode(path: &[u8]) -> u64 {
    path.iter().copied().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}
pub(crate) struct Vfs {
    overlay: BTreeMap<String, NodeRef>,
    whiteouts: BTreeSet<String>,
    fds: BTreeMap<u64, Descriptor>,
}
impl Default for Vfs {
    fn default() -> Self {
        let fds = (0..3)
            .map(|fd| {
                (
                    fd,
                    Descriptor {
                        file: Rc::new(RefCell::new(OpenFile {
                            node: Node::new(Data::Standard(fd), 0o020666),
                            path: format!("/dev/{}", ["stdin", "stdout", "stderr"][fd as usize]),
                            offset: 0,
                            flags: u32::from(fd != 0),
                            host: None,
                            entries: None,
                        })),
                        cloexec: false,
                    },
                )
            })
            .collect();
        Self {
            overlay: BTreeMap::new(),
            whiteouts: BTreeSet::new(),
            fds,
        }
    }
}
impl Emulator {
    pub(crate) fn install_apk_files(&mut self, apk: &crate::Apk) -> Result<()> {
        self.add_host_file(&format!("/data/app/{}-1/base.apk", apk.package), &apk.path)?;
        for suffix in ["", "/files", "/cache"] {
            let path = format!("/data/data/{}{suffix}", apk.package);
            self.add_parents(&path)
                .map_err(|error| format!("APK {} directory {path}: {error}", apk.path.display()))?;
            self.vfs
                .overlay
                .insert(path, Node::new(Data::Directory, 0o040755));
        }
        Ok(())
    }

    /// Add or replace an in-memory guest file, creating missing parent directories.
    pub fn add_file(&mut self, path: &str, bytes: &[u8]) -> Result<()> {
        let path = resolution::absolute(path)?;
        self.add_parents(&path)
            .map_err(|error| format!("{path}: {error}"))?;
        self.vfs.overlay.insert(
            path,
            Node::new(Data::Bytes(Rc::new(bytes.to_vec())), 0o100644),
        );
        Ok(())
    }
    /// Link a guest file to a read-only host file. Guest writes use a private copy.
    pub fn add_host_file(&mut self, path: &str, host_path: impl AsRef<Path>) -> Result<()> {
        let path = resolution::absolute(path)?;
        let host_path = host_path.as_ref();
        let host_error = |error: std::io::Error| {
            format!("add_host_file {path}: {}: {error}", host_path.display())
        };
        let host = host_path.canonicalize().map_err(host_error)?;
        let metadata = std::fs::metadata(&host).map_err(host_error)?;
        if !metadata.is_file() {
            return Err(host_error(std::io::ErrorKind::IsADirectory.into()).into());
        }
        self.add_parents(&path)
            .map_err(|error| format!("{path}: {error}"))?;
        self.vfs
            .overlay
            .insert(path, Node::new(Data::Host(host), metadata.mode()));
        Ok(())
    }
    /// Hide a guest path and its descendants, including entries in lower layers.
    pub fn remove_file(&mut self, path: &str) -> Result<()> {
        let path = resolution::absolute(path)?;
        self.vfs.remove(&path);
        Ok(())
    }
    /// Read the current contents of a regular guest file.
    pub fn file(&self, path: &str) -> Result<Vec<u8>> {
        let path = resolution::absolute(path)?;
        self.guest_node(&path)
            .and_then(|node| node.borrow().bytes())
            .map_err(|error| format!("{path}: {error}").into())
    }
    fn add_parents(&mut self, path: &str) -> FsResult<()> {
        let parent = resolution::parent(path);
        if parent == path {
            return Ok(());
        }
        match self.guest_node(parent) {
            Ok(node) if node.borrow().is_dir() => Ok(()),
            Ok(_) => Err(Error(20)),
            Err(Error(2)) => {
                self.add_parents(parent)?;
                self.vfs
                    .overlay
                    .insert(parent.into(), Node::new(Data::Directory, 0o040755));
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}
impl Vfs {
    fn remove(&mut self, path: &str) {
        self.overlay
            .retain(|name, _| !resolution::under(name, path));
        self.whiteouts.insert(path.into());
    }
    fn hidden(&self, mut path: &str) -> bool {
        loop {
            if self.whiteouts.contains(path) {
                return true;
            }
            if path == "/" {
                return false;
            }
            path = resolution::parent(path);
        }
    }
}
