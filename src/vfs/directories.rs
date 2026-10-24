use super::*;
use std::os::unix::ffi::OsStrExt;

#[derive(Clone)]
pub(super) struct DirEntry {
    pub name: Vec<u8>,
    pub inode: u64,
    pub kind: u8,
}
impl Emulator {
    pub(super) fn directory_entries(&self, path: &str) -> FsResult<Vec<DirEntry>> {
        if !self.guest_node(path)?.borrow().is_dir() {
            return Err(Error(20));
        }
        let mut entries = BTreeMap::new();
        if !self.vfs.hidden(path) {
            let host = self.rootfs.join(path.trim_start_matches('/'));
            if host.is_dir() {
                let host = self.rootfs_path(path)?;
                for entry in std::fs::read_dir(host)? {
                    let entry = entry?;
                    let name = entry.file_name().as_bytes().to_vec();
                    let child = resolution::normalize(&format!(
                        "{path}/{}",
                        entry.file_name().to_string_lossy()
                    ));
                    if self.vfs.hidden(&child) {
                        continue;
                    }
                    let kind = entry.file_type()?;
                    entries.insert(
                        name,
                        if kind.is_symlink() {
                            10
                        } else if kind.is_dir() {
                            4
                        } else {
                            8
                        },
                    );
                }
            }
            for generated in resolution::generated_paths() {
                if resolution::parent(&generated) == path && !self.vfs.hidden(&generated) {
                    let name = generated.rsplit('/').next().unwrap().as_bytes().to_vec();
                    entries.insert(
                        name,
                        if self.guest_node(&generated)?.borrow().is_dir() {
                            4
                        } else {
                            8
                        },
                    );
                }
            }
        }
        for (child, node) in &self.vfs.overlay {
            if child != path && resolution::parent(child) == path {
                entries.insert(
                    child.rsplit('/').next().unwrap().as_bytes().to_vec(),
                    if node.borrow().is_dir() { 4 } else { 8 },
                );
            }
        }
        let mut result = vec![
            DirEntry {
                name: b".".to_vec(),
                inode: inode(path.as_bytes()),
                kind: 4,
            },
            DirEntry {
                name: b"..".to_vec(),
                inode: inode(resolution::parent(path).as_bytes()),
                kind: 4,
            },
        ];
        for (name, kind) in entries {
            let mut full = path.trim_end_matches('/').as_bytes().to_vec();
            full.push(b'/');
            full.extend_from_slice(&name);
            result.push(DirEntry {
                name,
                inode: inode(&full),
                kind,
            });
        }
        Ok(result)
    }
}
impl Vfs {
    pub(crate) fn getdents(&mut self, fd: u64, count: u64) -> FsResult<Vec<u8>> {
        let mut file = self.descriptor(fd)?.file.borrow_mut();
        let entries = file.entries.clone().ok_or(Error(20))?;
        let mut bytes = Vec::new();
        for (index, entry) in entries.iter().enumerate().skip(file.offset as usize) {
            let length = (19 + entry.name.len() + 1).next_multiple_of(8);
            if bytes.len() as u64 + length as u64 > count.min(IO_LIMIT) {
                if bytes.is_empty() {
                    return Err(Error(22));
                }
                break;
            }
            let start = bytes.len();
            bytes.resize(start + length, 0);
            bytes[start..start + 8].copy_from_slice(&entry.inode.to_le_bytes());
            bytes[start + 8..start + 16].copy_from_slice(&(index as i64 + 1).to_le_bytes());
            bytes[start + 16..start + 18].copy_from_slice(&(length as u16).to_le_bytes());
            bytes[start + 18] = entry.kind;
            bytes[start + 19..start + 19 + entry.name.len()].copy_from_slice(&entry.name);
            file.offset = index as u64 + 1;
        }
        Ok(bytes)
    }
}
