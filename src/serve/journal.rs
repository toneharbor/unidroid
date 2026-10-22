use super::Options;
use crate::{
    Policy, Result,
    source::{Clock, Time},
};
use base64::{Engine, prelude::BASE64_STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::VecDeque,
    io::{BufRead, Write},
    rc::Rc,
};

pub(super) type Shared = Rc<RefCell<Journal>>;

#[derive(Serialize, Deserialize)]
struct Header<O, P> {
    pub record: u32,
    pub version: String,
    pub options: O,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<P>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Entry {
    In(String),
    Out(Value),
    Time(Time),
    Random(String),
}
impl Entry {
    fn kind(&self) -> &'static str {
        match self {
            Self::In(_) => "input",
            Self::Out(_) => "response",
            Self::Time(_) => "time",
            Self::Random(_) => "random",
        }
    }
}

enum Mode {
    Record(Box<dyn Write>),
    Replay {
        entries: VecDeque<(usize, Entry)>,
        end: usize,
    },
}
pub(super) struct Journal {
    mode: Mode,
    failure: Option<String>,
    pub commands: usize,
}
impl Journal {
    pub fn recording(
        mut file: impl Write + 'static,
        options: &Options,
        policy: Option<&Policy>,
    ) -> Result<Shared> {
        serde_json::to_writer(
            &mut file,
            &Header {
                record: 1,
                version: env!("CARGO_PKG_VERSION").into(),
                options,
                policy,
            },
        )?;
        file.write_all(b"\n")?;
        file.flush()?;
        Ok(Rc::new(RefCell::new(Self {
            mode: Mode::Record(Box::new(file)),
            failure: None,
            commands: 0,
        })))
    }
    pub fn replaying(mut file: impl BufRead) -> Result<(Options, Option<Policy>, Shared)> {
        let mut line = String::new();
        file.read_line(&mut line)?;
        let header: Header<Options, Policy> =
            serde_json::from_str(&line).map_err(|e| format!("line 1: {e}"))?;
        if header.record != 1 {
            return Err(format!("line 1: unsupported recording format {}", header.record).into());
        }
        let mut entries = VecDeque::new();
        let mut end = 2;
        for (i, line) in file.lines().enumerate() {
            let number = i + 2;
            let entry = serde_json::from_str(&line?).map_err(|e| format!("line {number}: {e}"))?;
            entries.push_back((number, entry));
            end = number + 1;
        }
        Ok((
            header.options,
            header.policy,
            Rc::new(RefCell::new(Self {
                mode: Mode::Replay { entries, end },
                failure: None,
                commands: 0,
            })),
        ))
    }
    fn fail<T>(&mut self, line: usize, message: impl std::fmt::Display) -> Result<T> {
        let message = self
            .failure
            .get_or_insert_with(|| format!("line {line}: {message}"));
        Err(message.clone().into())
    }
    fn check(&self) -> Result<()> {
        if let Some(error) = &self.failure {
            return Err(error.clone().into());
        }
        Ok(())
    }
    fn append(&mut self, entry: Entry) -> Result<()> {
        self.check()?;
        let Mode::Record(file) = &mut self.mode else {
            unreachable!()
        };
        serde_json::to_writer(&mut *file, &entry)?;
        file.write_all(b"\n")?;
        file.flush()?;
        Ok(())
    }
    fn take(&mut self, expected: &str) -> Result<(usize, Entry)> {
        self.check()?;
        let Mode::Replay { entries, end } = &mut self.mode else {
            unreachable!()
        };
        if let Some(entry) = entries.pop_front() {
            return Ok(entry);
        }
        let line = *end;
        self.fail(
            line,
            format!("recording ran out of values: expected {expected}"),
        )
    }
    pub fn replay_input(&mut self) -> Result<Option<String>> {
        self.check()?;
        let Mode::Replay { entries, .. } = &mut self.mode else {
            unreachable!()
        };
        let Some((line, entry)) = entries.pop_front() else {
            return Ok(None);
        };
        match entry {
            Entry::In(line) => {
                self.commands += 1;
                Ok(Some(line))
            }
            Entry::Out(value) => self.fail(line, format!("missing response: expected {value}")),
            entry => self.fail(
                line,
                format!("unused {} value at end of replay", entry.kind()),
            ),
        }
    }
    pub fn is_replay(&self) -> bool {
        matches!(self.mode, Mode::Replay { .. })
    }
    pub fn input(&mut self, line: &str) -> Result<()> {
        self.commands += 1;
        self.append(Entry::In(line.to_owned()))
    }
    pub fn output(&mut self, value: Value) -> Result<()> {
        if !self.is_replay() {
            return self.append(Entry::Out(value));
        }
        self.check()?;
        if let Mode::Replay { entries, end } = &self.mode
            && entries.is_empty()
        {
            return self.fail(*end, format!("extra response: got {value}"));
        }
        let (line, entry) = self.take("response")?;
        match entry {
            Entry::Out(expected) if expected == value => Ok(()),
            Entry::Out(expected) => self.fail(
                line,
                format!("response differs: expected {expected}, got {value}"),
            ),
            entry => self.fail(
                line,
                format!("extra response: got {value}, expected {}", entry.kind()),
            ),
        }
    }
    pub fn time(&mut self, clock: Clock) -> Result<Time> {
        let (line, entry) = self.take("time")?;
        match entry {
            Entry::Time(time) if time.clock == clock => Ok(time),
            Entry::Time(time) => self.fail(
                line,
                format!("time clock differs: expected {}, got {clock}", time.clock),
            ),
            entry => self.fail(
                line,
                format!(
                    "guest asked for time, next recorded value is {}",
                    entry.kind()
                ),
            ),
        }
    }
    pub fn random(&mut self, length: usize) -> Result<Vec<u8>> {
        let (line, entry) = self.take("random")?;
        match entry {
            Entry::Random(encoded) => {
                let bytes = BASE64_STANDARD
                    .decode(encoded)
                    .map_err(|e| format!("line {line}: {e}"))?;
                if bytes.len() > length {
                    return self.fail(
                        line,
                        format!(
                            "random length differs: expected {}, got {length}",
                            bytes.len()
                        ),
                    );
                }
                Ok(bytes)
            }
            entry => self.fail(
                line,
                format!(
                    "guest asked for random, next recorded value is {}",
                    entry.kind()
                ),
            ),
        }
    }
}

impl crate::source::Recorder for Shared {
    fn record_time(&mut self, time: Time) -> Result<()> {
        self.borrow_mut().append(Entry::Time(time))
    }
    fn record_random(&mut self, bytes: &[u8]) -> Result<()> {
        self.borrow_mut()
            .append(Entry::Random(BASE64_STANDARD.encode(bytes)))
    }
}

impl crate::source::Replay for Shared {
    fn time(&mut self, clock: Clock) -> Result<Time> {
        self.borrow_mut().time(clock)
    }
    fn random(&mut self, length: usize) -> Result<Vec<u8>> {
        self.borrow_mut().random(length)
    }
}
