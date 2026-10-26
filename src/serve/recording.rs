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
    let policy = options.load_policy()?;
    let mut effective = options.clone();
    effective.apk = options.apk.clone().or_else(|| builder.apk.clone());
    let mut emu = options.build(builder)?;
    effective.process_name = Some(emu.process_name.clone());
    effective.no_threads = !emu.threads.enabled();
    {
        let source = emu.source.shared.borrow();
        effective.fixed_clock = source.state.fixed_clock;
        effective.random_seed = source.state.random_seed;
    }
    let journal = Journal::recording(file, &effective, policy.as_ref())?;
    emu.source.shared.borrow_mut().mode = crate::source::Mode::Recording(Box::new(journal.clone()));
    let mut session = Session::new(input, output, policy);
    session.journal = Some(journal);
    session.run(&mut emu)
}

/// Replay a recording with the builder's diagnostic settings; return the input count.
pub fn replay(file: impl BufRead, builder: EmulatorBuilder) -> Result<usize> {
    let (options, policy, journal) = Journal::replaying(file)?;
    let mut emu = options.build(builder.threads(!options.no_threads))?;
    emu.source.shared.borrow_mut().mode = crate::source::Mode::Replaying(Box::new(journal.clone()));
    let mut session = Session::new(std::io::empty(), std::io::sink(), policy);
    session.journal = Some(journal.clone());
    session.run(&mut emu)?;
    Ok(journal.borrow().commands)
}
