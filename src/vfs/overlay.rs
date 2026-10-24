use super::*;

impl Emulator {
    pub(crate) fn mkdir(&mut self, path: &str, mode: u32) -> FsResult<u64> {
        let resolved = self.resolve_path(path, false)?;
        let path = resolved.as_str();
        match self.node_at(path) {
            Ok(_) => return Err(Error(17)),
            Err(Error(2)) => {}
            Err(error) => return Err(error),
        }
        self.require_parent(path)?;
        self.vfs.overlay.insert(
            path.into(),
            Node::new(Data::Directory, mode & 0o7777 | 0o040000),
        );
        Ok(0)
    }
    pub(crate) fn unlink(&mut self, path: &str, flags: u64) -> FsResult<u64> {
        if flags & !0x200 != 0 {
            return Err(Error(22));
        }
        let resolved = self.resolve_path(path, false)?;
        let path = resolved.as_str();
        let node = self.node_at(path)?;
        let directory = node.borrow().is_dir();
        if flags & 0x200 != 0 {
            if !directory {
                return Err(Error(20));
            }
            if self.directory_entries(path)?.len() > 2 {
                return Err(Error(39));
            }
        } else if directory {
            return Err(Error(21));
        }
        if path == "/" {
            return Err(Error(16));
        }
        self.vfs.remove(path);
        Ok(0)
    }
    pub(crate) fn rename(&mut self, from: &str, to: &str) -> FsResult<u64> {
        let resolved_from = self.resolve_path(from, false)?;
        let resolved_to = self.resolve_path(to, false)?;
        let from = resolved_from.as_str();
        let to = resolved_to.as_str();
        let source = self.node_at(from)?;
        if from == to {
            return Ok(0);
        }
        if from == "/" || to == "/" {
            return Err(Error(16));
        }
        let directory = source.borrow().is_dir();
        if directory && resolution::under(to, from) {
            return Err(Error(22));
        }
        self.require_parent(to)?;
        match self.node_at(to) {
            Ok(target) => {
                let target_dir = target.borrow().is_dir();
                if directory && !target_dir {
                    return Err(Error(20));
                }
                if !directory && target_dir {
                    return Err(Error(21));
                }
                if target_dir && self.directory_entries(to)?.len() > 2 {
                    return Err(Error(39));
                }
            }
            Err(Error(2)) => {}
            Err(error) => return Err(error),
        }
        let mut moved = Vec::new();
        self.move_tree(from, to, &mut moved)?;
        self.vfs.remove(from);
        self.vfs.remove(to);
        for (path, node, data) in moved {
            let mut entry = node.borrow_mut();
            entry.data = data;
            entry.changed = None;
            drop(entry);
            self.vfs.overlay.insert(path, node);
        }
        for descriptor in self.vfs.fds.values() {
            let mut file = descriptor.file.borrow_mut();
            if resolution::under(&file.path, from) {
                file.path = format!("{to}{}", &file.path[from.len()..]);
            }
        }
        Ok(0)
    }
    fn move_tree(
        &self,
        from: &str,
        to: &str,
        moved: &mut Vec<(String, NodeRef, Data)>,
    ) -> FsResult<()> {
        let node = self.node_at(from)?;
        let directory = node.borrow().is_dir();
        let data = if directory {
            Data::Directory
        } else {
            match &node.borrow().data {
                Data::Bytes(_) | Data::Symlink(_, _) => node.borrow().data.clone(),
                _ => Data::Bytes(Rc::new(node.borrow().copy_bytes()?)),
            }
        };
        moved.push((to.into(), node, data));
        if directory {
            for entry in self.directory_entries(from)?.into_iter().skip(2) {
                let name = String::from_utf8(entry.name).map_err(|_| Error(22))?;
                self.move_tree(&format!("{from}/{name}"), &format!("{to}/{name}"), moved)?;
            }
        }
        Ok(())
    }
}
