use super::{
    Options,
    commands::{self, BatchSession},
    diagnostics,
};
use crate::{Checkpoint, Emulator, EmulatorBuilder, Policy, Result, Trace};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Deserialize)]
struct Job {
    commands: Vec<Value>,
    policy: Option<PathBuf>,
    coverage: Option<Vec<String>>,
    #[serde(default)]
    diff: bool,
}

pub(in crate::serve) struct Worker<'a, F> {
    options: &'a Options,
    builder: &'a F,
    setup: &'a str,
    emu: Emulator,
    session: BatchSession,
    checkpoint: Option<Checkpoint>,
    policy: Option<Policy>,
    policies: HashMap<PathBuf, std::result::Result<Policy, String>>,
    setup_results: Vec<Value>,
    trace: Trace,
}

impl<'a, F: Fn() -> EmulatorBuilder> Worker<'a, F> {
    pub(super) fn new(options: &'a Options, builder: &'a F, setup: &'a str) -> Result<Self> {
        let mut emu = options.build(builder())?;
        let mut policies = HashMap::new();
        let mut session = commands::session(load_policy(&mut policies, options.policy.as_deref())?);
        let setup_results = run_setup(&mut session, &mut emu, setup)?;
        let checkpoint = if emu.modules().is_empty() {
            None
        } else {
            Some(emu.checkpoint()?)
        };
        let policy = session.policy.clone();
        let trace = emu.trace().clone();
        Ok(Self {
            options,
            builder,
            setup,
            emu,
            session,
            checkpoint,
            policy,
            policies,
            setup_results,
            trace,
        })
    }

    fn prepare(&mut self) -> Result<()> {
        if let Some(checkpoint) = &self.checkpoint {
            self.emu.rewind(checkpoint)?;
            self.session.policy = self.policy.clone();
        } else {
            self.emu = self.options.build((self.builder)())?;
            self.session = commands::session(load_policy(
                &mut self.policies,
                self.options.policy.as_deref(),
            )?);
            self.setup_results = run_setup(&mut self.session, &mut self.emu, self.setup)?;
        }
        self.emu.reset_coverage();
        Ok(())
    }

    pub(in crate::serve) fn resolve(&self, command: &Value, results: &[Value]) -> Result<Value> {
        commands::resolve(command, results, &self.setup_results)
    }

    pub(in crate::serve) fn run(&mut self, index: usize, line: &str) -> Value {
        let start = Instant::now();
        let mut report = json!({"id":index + 1, "ok":false, "results":[]});
        let mut before = None;
        let mut coverage = false;
        let result = (|| -> Result<()> {
            let job: Value = serde_json::from_str(line)?;
            if let Some(id) = job.get("id") {
                report["id"] = id.clone();
            }
            let job: Job = serde_json::from_value(job)?;
            self.prepare()?;
            if job.diff {
                before = Some(diagnostics::snapshot(&self.emu)?);
            }
            coverage = job.coverage.is_some();
            if let Some(specs) = job.coverage {
                let mut trace = self.emu.trace().clone();
                trace.coverage.extend(specs);
                self.emu.set_trace(trace)?;
            }
            if job.policy.is_some() {
                self.session.policy = load_policy(&mut self.policies, job.policy.as_deref())?;
            }
            let results = report["results"].as_array_mut().unwrap();
            for command in &job.commands {
                let response = commands::run(
                    &mut self.session,
                    &mut self.emu,
                    command,
                    results,
                    &self.setup_results,
                )?;
                if let Some(error) = response.get("error") {
                    report["error"] = error.clone();
                    if let Some(crash) = response.get("crash") {
                        report["crash"] = crash.clone();
                    }
                    return Ok(());
                }
                results.push(response["ok"].clone());
            }
            report["ok"] = json!(true);
            Ok(())
        })();
        if let Err(error) = result {
            fail(
                &mut report,
                if error.is::<serde_json::Error>() {
                    format!("bad job: {error}").into()
                } else {
                    error
                },
            );
        }
        let diagnostics = (|| -> Result<()> {
            if coverage {
                report["coverage"] = commands::run(
                    &mut self.session,
                    &mut self.emu,
                    &json!({"cmd":"coverage"}),
                    &[],
                    &[],
                )?["ok"]
                    .clone();
            }
            if let Some(before) = before {
                report["diff"] =
                    diagnostics::diff_value(before.diff(&diagnostics::snapshot(&self.emu)?));
            }
            Ok(())
        })();
        if let Err(error) = diagnostics {
            fail(&mut report, error);
        }
        if let Err(error) = self.emu.set_trace(self.trace.clone()) {
            fail(&mut report, error);
        }
        report["ms"] = json!(start.elapsed().as_millis());
        report
    }
}

fn fail(report: &mut Value, error: Box<dyn std::error::Error>) {
    report["ok"] = json!(false);
    if report.get("error").is_none() {
        report["error"] = json!(error.to_string());
    }
}

fn load_policy(
    cache: &mut HashMap<PathBuf, std::result::Result<Policy, String>>,
    path: Option<&Path>,
) -> Result<Option<Policy>> {
    path.map(|path| {
        cache
            .entry(path.to_owned())
            .or_insert_with(|| Policy::load(path).map_err(|error| error.to_string()))
            .clone()
            .map_err(Into::into)
    })
    .transpose()
}

fn run_setup(session: &mut BatchSession, emu: &mut Emulator, setup: &str) -> Result<Vec<Value>> {
    let mut results = Vec::new();
    for (index, line) in setup.lines().enumerate() {
        let result = (|| -> Result<Value> {
            let command = serde_json::from_str(line)?;
            let response = commands::run(session, emu, &command, &results, &[])?;
            if let Some(error) = response.get("error") {
                return Err(error.as_str().unwrap().into());
            }
            Ok(response["ok"].clone())
        })();
        results.push(result.map_err(|error| {
            let message = if error.is::<serde_json::Error>() {
                format!("bad command: {error}")
            } else {
                error.to_string()
            };
            format!("setup command {}: {message}", index + 1)
        })?);
    }
    Ok(results)
}
