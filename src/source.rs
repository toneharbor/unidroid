use crate::Result;
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Clock {
    Realtime,
    Monotonic,
}
impl std::fmt::Display for Clock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Realtime => "realtime",
            Self::Monotonic => "monotonic",
        })
    }
}
#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) struct Time {
    pub clock: Clock,
    pub sec: i64,
    pub nsec: i64,
}

pub(crate) trait Recorder {
    fn record_time(&mut self, time: Time) -> Result<()>;
    fn record_random(&mut self, bytes: &[u8]) -> Result<()>;
}

pub(crate) trait Replay {
    fn time(&mut self, clock: Clock) -> Result<Time>;
    fn random(&mut self, length: usize) -> Result<Vec<u8>>;
}

#[derive(Default)]
pub(crate) enum Mode {
    #[default]
    Host,
    Recording(Box<dyn Recorder>),
    Replaying(Box<dyn Replay>),
}

#[derive(Default)]
pub(crate) struct Source {
    pub mode: Mode,
    random: Option<File>,
}
impl Source {
    pub fn time(&mut self, clock: Clock) -> Result<Time> {
        if let Mode::Replaying(replay) = &mut self.mode {
            return replay.time(clock);
        }
        let mut time = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        unsafe {
            libc::clock_gettime(
                match clock {
                    Clock::Realtime => libc::CLOCK_REALTIME,
                    Clock::Monotonic => libc::CLOCK_MONOTONIC,
                },
                &mut time,
            );
        }
        let time = Time {
            clock,
            sec: time.tv_sec,
            nsec: time.tv_nsec,
        };
        if let Mode::Recording(recorder) = &mut self.mode {
            recorder.record_time(time)?;
        }
        Ok(time)
    }
    pub fn random(&mut self, buffer: &mut [u8]) -> Result<usize> {
        if let Mode::Replaying(replay) = &mut self.mode {
            let bytes = replay.random(buffer.len())?;
            buffer[..bytes.len()].copy_from_slice(&bytes);
            return Ok(bytes.len());
        }
        if self.random.is_none() {
            self.random = Some(File::open("/dev/urandom")?);
        }
        let count = self.random.as_mut().unwrap().read(buffer)?;
        if let Mode::Recording(recorder) = &mut self.mode {
            recorder.record_random(&buffer[..count])?;
        }
        Ok(count)
    }
}
