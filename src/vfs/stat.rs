use super::*;
use crate::{Architecture, arch::Stat};
use std::os::unix::fs::MetadataExt;

impl Node {
    pub(super) fn stat(
        &mut self,
        path: &str,
        arch: Architecture,
        source: &mut crate::source::Source,
    ) -> Result<([u8; 128], usize)> {
        let fixed = source.shared.borrow().state.fixed_clock;
        let fields = match &self.data {
            Data::Host(host) => {
                host_stat(&std::fs::metadata(host).map_err(Error::from)?, path, fixed)
            }
            Data::Symlink(target, metadata) => {
                let mut fields = host_stat(metadata, path, fixed);
                fields.mode = 0o120777;
                fields.size = target.len() as u64;
                fields
            }
            Data::Random => host_stat(
                &std::fs::metadata("/dev/urandom").map_err(Error::from)?,
                path,
                fixed,
            ),
            data => {
                let generated = matches!(data, Data::Generated(_));
                let size = match data {
                    Data::Bytes(bytes) | Data::Generated(bytes) => bytes.len() as u64,
                    _ => 0,
                };
                let (sec, nanos) = if let Some(sec) = fixed {
                    (sec, 0)
                } else if !generated && let Some(time) = self.changed {
                    time
                } else {
                    let time = source.time(crate::source::Clock::Realtime)?;
                    let time = (time.sec, time.nsec);
                    if !generated {
                        self.changed = Some(time);
                    }
                    time
                };
                Stat {
                    dev: u64::from(generated),
                    ino: if generated { 1 } else { inode(path.as_bytes()) },
                    mode: self.mode,
                    nlink: u64::from(!generated),
                    uid: 0,
                    gid: 0,
                    rdev: 0,
                    size,
                    blksize: 4096,
                    blocks: size.div_ceil(if generated { 4096 } else { 512 }),
                    times: [sec, nanos, sec, nanos, sec, nanos],
                }
            }
        };
        Ok(arch.stat(&fields))
    }
}
fn host_stat(m: &std::fs::Metadata, path: &str, fixed_clock: Option<i64>) -> Stat {
    let mut fields = Stat {
        dev: m.dev(),
        ino: m.ino(),
        mode: m.mode(),
        nlink: m.nlink(),
        uid: m.uid(),
        gid: m.gid(),
        rdev: m.rdev(),
        size: m.size(),
        blksize: 4096,
        blocks: m.blocks(),
        times: [
            m.atime(),
            m.atime_nsec(),
            m.mtime(),
            m.mtime_nsec(),
            m.ctime(),
            m.ctime_nsec(),
        ],
    };
    if let Some(seconds) = fixed_clock {
        fields.dev = 0;
        fields.ino = inode(path.as_bytes());
        fields.uid = 0;
        fields.gid = 0;
        fields.blocks = m.size().div_ceil(512);
        fields.times = [seconds, 0, seconds, 0, seconds, 0];
    }
    fields
}
impl Emulator {
    pub(crate) fn stat_file(&mut self, fd: u64) -> Result<([u8; 128], usize)> {
        let arch = self.architecture();
        let file = self.vfs.descriptor(fd)?.file.borrow();
        file.node
            .borrow_mut()
            .stat(&file.path, arch, &mut self.source)
    }
    pub(crate) fn stat_path(&mut self, path: &str, follow: bool) -> Result<([u8; 128], usize)> {
        let path = self.resolve_path(path, follow)?;
        let arch = self.architecture();
        self.node_at(&path)?
            .borrow_mut()
            .stat(&path, arch, &mut self.source)
    }
}
