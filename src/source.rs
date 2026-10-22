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

struct SplitMix64 {
    state: u64,
    bytes: [u8; 8],
    position: usize,
}
impl SplitMix64 {
    fn fill(&mut self, buffer: &mut [u8]) {
        for byte in buffer {
            if self.position == 8 {
                self.state = self.state.wrapping_add(0x9e3779b97f4a7c15);
                let mut value = self.state;
                value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
                value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
                self.bytes = (value ^ (value >> 31)).to_le_bytes();
                self.position = 0;
            }
            *byte = self.bytes[self.position];
            self.position += 1;
        }
    }
}

#[derive(Default)]
pub(crate) struct Source {
    pub mode: Mode,
    random: Option<File>,
    pub fixed_clock: Option<i64>,
    elapsed_ns: u64,
    seeded_random: Option<SplitMix64>,
}
impl Source {
    pub fn new(fixed_clock: Option<i64>, random_seed: Option<u64>) -> Self {
        Self {
            fixed_clock,
            seeded_random: random_seed.map(|state| SplitMix64 {
                state,
                bytes: [0; 8],
                position: 8,
            }),
            ..Self::default()
        }
    }
    pub fn time(&mut self, clock: Clock) -> Result<Time> {
        if let Mode::Replaying(replay) = &mut self.mode {
            return replay.time(clock);
        }
        let time = if let Some(seconds) = self.fixed_clock {
            let time = Time {
                clock,
                sec: match clock {
                    Clock::Realtime => seconds,
                    Clock::Monotonic => 1,
                } + (self.elapsed_ns / 1_000_000_000) as i64,
                nsec: (self.elapsed_ns % 1_000_000_000) as i64,
            };
            self.elapsed_ns += 1_000;
            time
        } else {
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
            Time {
                clock,
                sec: time.tv_sec,
                nsec: time.tv_nsec,
            }
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
        let count = if let Some(random) = &mut self.seeded_random {
            random.fill(buffer);
            buffer.len()
        } else {
            if self.random.is_none() {
                self.random = Some(File::open("/dev/urandom")?);
            }
            self.random.as_mut().unwrap().read(buffer)?
        };
        if let Mode::Recording(recorder) = &mut self.mode {
            recorder.record_random(&buffer[..count])?;
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_ticks_carry_into_seconds() -> Result<()> {
        let mut source = Source::new(Some(-1), None);
        let first = source.time(Clock::Realtime)?;
        assert_eq!((first.sec, first.nsec), (-1, 0));
        for _ in 1..1_000_000 {
            source.time(Clock::Monotonic)?;
        }
        let realtime = source.time(Clock::Realtime)?;
        assert_eq!((realtime.sec, realtime.nsec), (0, 0));
        let monotonic = source.time(Clock::Monotonic)?;
        assert_eq!((monotonic.sec, monotonic.nsec), (2, 1_000));
        Ok(())
    }

    #[test]
    fn replay_precedes_fixed_sources_without_consuming_them() -> Result<()> {
        struct Saved;
        impl Replay for Saved {
            fn time(&mut self, clock: Clock) -> Result<Time> {
                Ok(Time {
                    clock,
                    sec: 999,
                    nsec: 123,
                })
            }
            fn random(&mut self, _: usize) -> Result<Vec<u8>> {
                Ok(vec![1, 2, 3])
            }
        }
        let mut source = Source::new(Some(-1), Some(0));
        source.mode = Mode::Replaying(Box::new(Saved));
        for clock in [Clock::Realtime, Clock::Monotonic] {
            let time = source.time(clock)?;
            assert_eq!((time.sec, time.nsec), (999, 123));
        }
        let mut bytes = [0xa5; 8];
        assert_eq!(source.random(&mut bytes)?, 3);
        assert_eq!(bytes, [1, 2, 3, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5]);
        source.mode = Mode::Host;
        let time = source.time(Clock::Realtime)?;
        assert_eq!((time.sec, time.nsec), (-1, 0));
        assert_eq!(source.random(&mut bytes)?, 8);
        assert_eq!(bytes, [0xaf, 0xcd, 0x1d, 0x7b, 0x39, 0xa8, 0x20, 0xe2]);
        Ok(())
    }
}
