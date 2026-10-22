use super::Session;
use crate::{Emulator, Policy, Result};
use serde_json::Value;
use std::io::{Empty, Sink};

pub(super) type BatchSession = Session<Empty, Sink>;

pub(super) fn session(policy: Option<Policy>) -> BatchSession {
    Session {
        input: std::io::empty(),
        output: std::io::sink(),
        error: None,
        fatal: false,
        journal: None,
        policy,
        checkpoints: Default::default(),
        next_checkpoint: 0,
        driver: false,
    }
}

pub(super) fn run(
    session: &mut BatchSession,
    emu: &mut Emulator,
    command: &Value,
    results: &[Value],
    setup: &[Value],
) -> Result<Value> {
    let command = resolve(command, results, setup)?;
    if let Some(name) = command.get("cmd").and_then(Value::as_str)
        && matches!(
            name,
            "break"
                | "unbreak"
                | "watch"
                | "unwatch"
                | "tracepoint"
                | "untracepoint"
                | "continue"
                | "step"
                | "finish"
                | "return"
                | "abort"
                | "answer"
                | "unhandled"
                | "checkpoint"
                | "rewind"
                | "drop_checkpoint"
        )
    {
        return Err(format!("{name} is not available in batch").into());
    }
    let command = serde_json::from_value(command)?;
    Ok(serde_json::to_value(session.dispatch(emu, Ok(command))?)?)
}

pub(super) fn resolve(value: &Value, results: &[Value], setup: &[Value]) -> Result<Value> {
    match value {
        Value::Object(object) => {
            if object.len() == 1 {
                for (name, values) in [("$ref", results), ("$setup", setup)] {
                    if let Some(index) = object.get(name) {
                        return index
                            .as_u64()
                            .and_then(|n| usize::try_from(n).ok())
                            .and_then(|n| values.get(n))
                            .cloned()
                            .ok_or_else(|| format!("bad reference {name}: {index}").into());
                    }
                }
            }
            Ok(Value::Object(
                object
                    .iter()
                    .map(|(key, value)| Ok((key.clone(), resolve(value, results, setup)?)))
                    .collect::<Result<_>>()?,
            ))
        }
        Value::Array(array) => Ok(Value::Array(
            array
                .iter()
                .map(|value| resolve(value, results, setup))
                .collect::<Result<_>>()?,
        )),
        _ => Ok(value.clone()),
    }
}
