use crate::{Architecture, Emulator, Result};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

pub(crate) enum GuestFile {
    Host { file: File, path: String },
    Random { path: String },
}
pub(crate) enum FileState {
    Host { path: String, offset: u64 },
    Random { path: String },
}
impl FileState {
    pub fn reopen(&self, emu: &Emulator) -> Result<GuestFile> {
        match self {
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
    pub fn checkpoint(&mut self) -> Result<FileState> {
        Ok(match self {
            Self::Host { file, path } => FileState::Host {
                path: path.clone(),
                offset: file.stream_position()?,
            },
            Self::Random { path } => FileState::Random { path: path.clone() },
        })
    }
    pub fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        match self {
            Self::Host { file, .. } => file.seek(from),
            Self::Random { .. } => Ok(0),
        }
    }
    pub fn stream_position(&mut self) -> std::io::Result<u64> {
        match self {
            Self::Host { file, .. } => file.stream_position(),
            Self::Random { .. } => Ok(0),
        }
    }
    pub fn stat(
        &self,
        arch: Architecture,
        fixed_clock: Option<i64>,
    ) -> std::io::Result<([u8; 128], usize)> {
        let (metadata, path) = match self {
            Self::Host { file, path } => (file.metadata()?, path),
            Self::Random { path } => (std::fs::metadata("/dev/urandom")?, path),
        };
        Ok(arch.stat(&metadata, path, fixed_clock))
    }
}
impl Emulator {
    pub(super) fn read_file(&mut self, fd: u64, buffer: &mut [u8]) -> Result<usize> {
        match self.files.get_mut(&fd).expect("open file") {
            GuestFile::Host { file, .. } => Ok(file.read(buffer)?),
            GuestFile::Random { .. } => self.source.random(buffer),
        }
    }
}
