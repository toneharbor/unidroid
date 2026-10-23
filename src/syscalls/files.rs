use crate::{Architecture, Emulator, Result, arch::Stat};
use std::{
    fs::File,
    io::{Cursor, Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) const IO_LIMIT: u64 = 33_554_431;

pub(crate) enum GuestFile {
    Memory(Cursor<Vec<u8>>),
    Host { file: File, path: String },
    Random { path: String },
}
pub(crate) enum FileState {
    Memory(Cursor<Vec<u8>>),
    Host { path: String, offset: u64 },
    Random { path: String },
}
impl FileState {
    pub fn reopen(&self, emu: &Emulator) -> Result<GuestFile> {
        match self {
            Self::Memory(data) => Ok(GuestFile::Memory(data.clone())),
            Self::Host { path, offset } => {
                let mut file = emu.open_guest(path)?;
                file.seek(SeekFrom::Start(*offset))?;
                Ok(file)
            }
            Self::Random { path } => Ok(GuestFile::Random { path: path.clone() }),
        }
    }
}

impl GuestFile {
    pub fn read_length(&mut self, requested: u64) -> std::io::Result<usize> {
        let available = match self {
            Self::Memory(data) => (data.get_ref().len() as u64).saturating_sub(data.position()),
            Self::Host { file, .. } => file
                .metadata()?
                .len()
                .saturating_sub(file.stream_position()?),
            Self::Random { .. } => IO_LIMIT,
        };
        Ok(requested.min(available) as usize)
    }
    pub fn checkpoint(&mut self) -> Result<FileState> {
        Ok(match self {
            Self::Memory(data) => FileState::Memory(data.clone()),
            Self::Host { file, path } => FileState::Host {
                path: path.clone(),
                offset: file.stream_position()?,
            },
            Self::Random { path } => FileState::Random { path: path.clone() },
        })
    }
    pub fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        match self {
            Self::Memory(data) => data.seek(from),
            Self::Host { file, .. } => file.seek(from),
            Self::Random { .. } => Ok(0),
        }
    }
    pub fn stream_position(&mut self) -> std::io::Result<u64> {
        match self {
            Self::Memory(data) => Ok(data.position()),
            Self::Host { file, .. } => file.stream_position(),
            Self::Random { .. } => Ok(0),
        }
    }
    pub fn stat(
        &self,
        arch: Architecture,
        fixed_clock: Option<i64>,
    ) -> std::io::Result<([u8; 128], usize)> {
        let fields = match self {
            Self::Memory(data) => {
                let (seconds, nanos) = if let Some(seconds) = fixed_clock {
                    (seconds, 0)
                } else {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_err(std::io::Error::other)?;
                    (now.as_secs() as i64, now.subsec_nanos() as i64)
                };
                let size = data.get_ref().len() as u64;
                Stat {
                    dev: 1,
                    ino: 1,
                    mode: 0o100000,
                    nlink: 0,
                    uid: 0,
                    gid: 0,
                    rdev: 0,
                    size,
                    blksize: 4096,
                    blocks: size.div_ceil(4096),
                    times: [seconds, nanos, seconds, nanos, seconds, nanos],
                }
            }
            Self::Host { file, path } => host_stat(&file.metadata()?, path, fixed_clock),
            Self::Random { path } => {
                host_stat(&std::fs::metadata("/dev/urandom")?, path, fixed_clock)
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
        fields.ino = path
            .as_bytes()
            .iter()
            .fold(0xcbf29ce484222325u64, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
            });
        fields.uid = 0;
        fields.gid = 0;
        fields.blocks = m.size().div_ceil(512);
        fields.times = [seconds, 0, seconds, 0, seconds, 0];
    }
    fields
}

impl Emulator {
    pub(super) fn faccessat(&self, address: u64) -> Result<u64> {
        let path = self.cstring(address)?;
        Ok(if self.open_guest(&path).is_ok() {
            0
        } else {
            (-2i64) as u64
        })
    }

    pub(super) fn read_file(&mut self, fd: u64, buffer: &mut [u8]) -> Result<usize> {
        match self.files.get_mut(&fd).expect("open file") {
            GuestFile::Memory(data) => Ok(data.read(buffer)?),
            GuestFile::Host { file, .. } => Ok(file.read(buffer)?),
            GuestFile::Random { .. } => self.source.random(buffer),
        }
    }
}
