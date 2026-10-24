use super::*;
use descriptors::{Descriptor, OpenFile};
use std::{collections::HashMap, fs::File};

impl Vfs {
    pub(crate) fn copy(&self, reopen: bool) -> FsResult<Self> {
        let mut nodes = HashMap::new();
        let mut node = |old: &NodeRef| -> NodeRef {
            nodes
                .entry(Rc::as_ptr(old))
                .or_insert_with(|| Rc::new(RefCell::new(old.borrow().clone())))
                .clone()
        };
        let overlay = self
            .overlay
            .iter()
            .map(|(path, old)| (path.clone(), node(old)))
            .collect();
        let mut descriptions = HashMap::new();
        let mut fds = BTreeMap::new();
        for (&fd, old) in &self.fds {
            let key = Rc::as_ptr(&old.file);
            let file = if let Some(file) = descriptions.get(&key) {
                Rc::clone(file)
            } else {
                let old = old.file.borrow();
                let host = match &old.node.borrow().data {
                    Data::Host(path) if reopen => Some(File::open(path)?),
                    _ => None,
                };
                let file = Rc::new(RefCell::new(OpenFile {
                    node: node(&old.node),
                    path: old.path.clone(),
                    offset: old.offset,
                    flags: old.flags,
                    host,
                    entries: old.entries.clone(),
                }));
                descriptions.insert(key, file.clone());
                file
            };
            fds.insert(
                fd,
                Descriptor {
                    file,
                    cloexec: old.cloexec,
                },
            );
        }
        Ok(Self {
            overlay,
            whiteouts: self.whiteouts.clone(),
            fds,
        })
    }
}
