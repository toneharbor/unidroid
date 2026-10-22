use super::{Options, batch};
use crate::{EmulatorBuilder, Result};
use base64::{Engine, prelude::BASE64_STANDARD};
use clap::Args;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// Settings for differential input-field analysis.
#[derive(Args)]
pub struct FieldsOptions {
    /// Number of worker threads.
    #[arg(long, default_value_t = batch::workers())]
    pub workers: usize,
}

impl Default for FieldsOptions {
    fn default() -> Self {
        Self {
            workers: batch::workers(),
        }
    }
}

#[derive(Deserialize)]
struct Spec {
    #[serde(default)]
    setup: Vec<Value>,
    commands: Vec<Value>,
    output: Option<usize>,
    fields: Vec<Field>,
}

#[derive(Deserialize)]
struct Field {
    name: String,
    command: usize,
    offset: usize,
    length: usize,
    values: Option<Vec<String>>,
}

/// Compare one changed input field at a time using the batch runner.
/// Invalid specs, setup failures and failed or nonrepeatable base runs return Err.
/// Field-run failures remain in the returned JSON report.
pub fn fields(
    options: &Options,
    settings: &FieldsOptions,
    builder: impl Fn() -> EmulatorBuilder + Sync,
    spec: Value,
) -> Result<Value> {
    let spec: Spec = serde_json::from_value(spec)?;
    batch::check_workers(settings.workers)?;
    let output = spec.output.unwrap_or(spec.commands.len().saturating_sub(1));
    let command = spec
        .commands
        .get(output)
        .ok_or("output command out of range")?;
    let mut jobs = Vec::new();
    let mut variants = Vec::new();
    for field in &spec.fields {
        let changes = replacements(field, &spec.commands)
            .map_err(|error| format!("field {}: {error}", field.name))?;
        variants.push(
            changes
                .into_iter()
                .map(|change| {
                    change.map(|command| {
                        let mut commands = spec.commands.clone();
                        commands[field.command] = command;
                        let index = jobs.len();
                        jobs.push(json!({"commands":commands}).to_string());
                        index
                    })
                })
                .collect::<Vec<_>>(),
        );
    }
    let setup: String = spec
        .setup
        .iter()
        .map(|value| format!("{value}\n"))
        .collect();
    let base_job = json!({"commands":spec.commands}).to_string();
    let mut base = Vec::new();
    let reports = batch::run(
        options,
        &batch::BatchOptions {
            workers: settings.workers,
            ..Default::default()
        },
        &builder,
        &setup,
        jobs,
        |worker| {
            let report = worker.run(0, &base_job);
            base = output_bytes(worker, &report, output, command)
                .map_err(|error| format!("base run: {error}"))?;
            let report = worker.run(1, &base_job);
            let repeated = output_bytes(worker, &report, output, command)
                .map_err(|error| format!("base run: {error}"))?;
            if base != repeated {
                return Err(
                    "the unmodified run is not repeatable; use --fixed-clock and --random-seed"
                        .into(),
                );
            }
            Ok(())
        },
        |worker, report| {
            output_bytes(worker, &report, output, command).map_err(|error| error.to_string())
        },
    )?;
    let mut fields = Vec::new();
    let mut by_byte = vec![Vec::new(); base.len()];
    for (field, variants) in spec.fields.iter().zip(variants) {
        let mut offsets = BTreeSet::new();
        let mut length_changed = false;
        let mut errors = Vec::new();
        let runs = variants.len();
        for (index, variant) in variants.into_iter().enumerate() {
            let result = variant.and_then(|job| reports[job].clone());
            match result {
                Ok(changed) => {
                    length_changed |= base.len() != changed.len();
                    offsets.extend(
                        (0..base.len().max(changed.len()))
                            .filter(|&offset| base.get(offset) != changed.get(offset)),
                    );
                }
                Err(error) => errors.push(format!("value {index}: {error}")),
            }
        }
        for &offset in offsets.range(..base.len()) {
            by_byte[offset].push(field.name.as_str());
        }
        let mut report = json!({"name":field.name, "runs":runs, "changed":ranges(offsets)});
        if length_changed {
            report["length_changed"] = json!(true);
        }
        if !errors.is_empty() {
            report["errors"] = json!(errors);
        }
        fields.push(report);
    }
    let mut by_output = Vec::new();
    let mut unaffected = Vec::new();
    let mut start = 0;
    while start < base.len() {
        let mut end = start + 1;
        while end < base.len() && by_byte[end] == by_byte[start] {
            end += 1;
        }
        if by_byte[start].is_empty() {
            unaffected.push([start, end]);
        } else {
            by_output.push(json!({"start":start, "end":end, "fields":by_byte[start]}));
        }
        start = end;
    }
    Ok(json!({
        "output":{"length":base.len(), "base64":BASE64_STANDARD.encode(&base)},
        "fields":fields, "by_output":by_output, "unaffected":unaffected
    }))
}

// Spec errors stop before workers are built; invalid UTF-8 belongs to one run.
fn replacements(
    field: &Field,
    commands: &[Value],
) -> Result<Vec<std::result::Result<Value, String>>> {
    let command = commands.get(field.command).ok_or("command out of range")?;
    let binary = match command["cmd"].as_str() {
        Some("write" | "new_array") => true,
        Some("new_string" | "new_cstring") => false,
        _ => return Err("command has no field payload".into()),
    };
    let key = if binary { "base64" } else { "value" };
    let payload = command[key].as_str().ok_or("missing payload")?;
    let bytes = if binary {
        BASE64_STANDARD.decode(payload)?
    } else {
        payload.as_bytes().to_vec()
    };
    let end = field
        .offset
        .checked_add(field.length)
        .ok_or("range past payload")?;
    let range = bytes.get(field.offset..end).ok_or("range past payload")?;
    let values = match &field.values {
        Some(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let bytes = BASE64_STANDARD
                    .decode(value)
                    .map_err(|error| format!("value {index}: {error}"))?;
                if bytes.len() != field.length {
                    return Err(format!(
                        "value {index}: expected {} bytes, got {}",
                        field.length,
                        bytes.len()
                    )
                    .into());
                }
                Ok(bytes)
            })
            .collect::<Result<Vec<_>>>()?,
        None => vec![range.iter().map(|byte| byte ^ 1).collect()],
    };
    Ok(values
        .into_iter()
        .map(|value| {
            let mut bytes = bytes.clone();
            bytes[field.offset..end].copy_from_slice(&value);
            let payload = if binary {
                BASE64_STANDARD.encode(bytes)
            } else {
                String::from_utf8(bytes).map_err(|error| error.to_string())?
            };
            let mut command = command.clone();
            command[key] = json!(payload);
            Ok(command)
        })
        .collect())
}

fn output_bytes(
    worker: &batch::Worker<'_, impl Fn() -> EmulatorBuilder>,
    report: &Value,
    output: usize,
    command: &Value,
) -> Result<Vec<u8>> {
    if let Some(error) = report["error"].as_str() {
        return Err(error.into());
    }
    let results = report["results"].as_array().unwrap();
    let command = worker.resolve(command, &results[..output])?;
    let value = &results[output];
    Ok(match command["cmd"].as_str() {
        Some("read" | "array") => {
            BASE64_STANDARD.decode(value.as_str().ok_or("output is not base64 text")?)?
        }
        Some("string") => value
            .as_str()
            .ok_or("output is not a string")?
            .as_bytes()
            .to_vec(),
        _ => serde_json::to_vec(value)?,
    })
}

fn ranges(offsets: BTreeSet<usize>) -> Vec<[usize; 2]> {
    let mut ranges: Vec<[usize; 2]> = Vec::new();
    for offset in offsets {
        if let Some(last) = ranges.last_mut()
            && last[1] == offset
        {
            last[1] += 1;
        } else {
            ranges.push([offset, offset + 1]);
        }
    }
    ranges
}
