use super::{Options, Session, journal::Journal};
use crate::{EmulatorBuilder, Result};
use std::io::{BufRead, Write};

/// Serve and flush each input, response and external value to a JSON Lines recording.
pub fn record(
    options: &Options,
    builder: EmulatorBuilder,
    file: impl Write + 'static,
    input: impl BufRead,
    output: impl Write,
) -> Result<()> {
    let mut emu = options.build(builder)?;
    let journal = Journal::recording(file, options)?;
    emu.source.mode = crate::source::Mode::Recording(Box::new(journal.clone()));
    let mut session = Session {
        input,
        output,
        error: None,
        fatal: false,
        journal: Some(journal),
    };
    session.run(&mut emu)
}

/// Replay a recording with the builder's diagnostic settings; return the input count.
pub fn replay(file: impl BufRead, builder: EmulatorBuilder) -> Result<usize> {
    let (options, journal) = Journal::replaying(file)?;
    let mut emu = options.build(builder)?;
    emu.source.mode = crate::source::Mode::Replaying(Box::new(journal.clone()));
    let mut session = Session {
        input: std::io::empty(),
        output: std::io::sink(),
        error: None,
        fatal: false,
        journal: Some(journal.clone()),
    };
    session.run(&mut emu)?;
    Ok(journal.borrow().commands)
}
