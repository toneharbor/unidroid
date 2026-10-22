use crate::{Emulator, Result};
use std::{
    fs::{File, Metadata},
    io::{Read, Seek, SeekFrom},
};

pub(crate) enum GuestFile {
    Host(File),
    Random,
}
impl GuestFile {
    pub fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        match self {
            Self::Host(file) => file.seek(from),
            Self::Random => Ok(0),
        }
    }
    pub fn stream_position(&mut self) -> std::io::Result<u64> {
        match self {
            Self::Host(file) => file.stream_position(),
            Self::Random => Ok(0),
        }
    }
    pub fn metadata(&self) -> std::io::Result<Metadata> {
        match self {
            Self::Host(file) => file.metadata(),
            Self::Random => std::fs::metadata("/dev/urandom"),
        }
    }
}
impl Emulator {
    pub(super) fn read_file(&mut self, fd: u64, buffer: &mut [u8]) -> Result<usize> {
        match self.files.get_mut(&fd).expect("open file") {
            GuestFile::Host(file) => Ok(file.read(buffer)?),
            GuestFile::Random => self.source.random(buffer),
        }
    }
}
