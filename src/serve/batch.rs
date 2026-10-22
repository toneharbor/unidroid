use super::{Options, Session};
use crate::{EmulatorBuilder, Result};
use clap::{Args, ValueEnum};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    io::BufRead,
    path::PathBuf,
    sync::{Barrier, Mutex},
    thread,
    time::Instant,
};

mod commands;
mod diagnostics;
mod worker;

/// Settings for a batch of independent serve command sequences.
#[derive(Args)]
pub struct BatchOptions {
    /// Serve commands to run before preparing each worker.
    #[arg(long, value_name = "FILE")]
    pub setup: Option<PathBuf>,
    /// Number of worker threads.
    #[arg(long, default_value_t = workers())]
    pub workers: usize,
    /// Extra attempts for each failed job.
    #[arg(long, default_value_t = 0)]
    pub retries: usize,
    /// Include only matching jobs; the summary still counts all jobs.
    #[arg(long, value_enum)]
    pub only: Option<BatchOnly>,
}

/// Select which job results appear in the report.
#[derive(Clone, Copy, ValueEnum)]
pub enum BatchOnly {
    Ok,
    Failed,
}

pub(super) use worker::Worker;

pub(super) fn workers() -> usize {
    thread::available_parallelism().map_or(1, usize::from)
}

pub(super) fn check_workers(count: usize) -> Result<()> {
    if count == 0 {
        return Err("workers must be greater than zero".into());
    }
    Ok(())
}

impl Default for BatchOptions {
    fn default() -> Self {
        Self {
            setup: None,
            workers: workers(),
            retries: 0,
            only: None,
        }
    }
}

/// Run JSON Lines jobs in input order in the report. The factory builds each
/// emulator's builder inside its worker, including diagnostic event handlers.
/// Setup failures return an error before any job runs; job failures stay in the report.
pub fn batch(
    options: &Options,
    settings: &BatchOptions,
    builder: impl Fn() -> EmulatorBuilder + Sync,
    jobs: impl BufRead,
) -> Result<Value> {
    let start = Instant::now();
    check_workers(settings.workers)?;
    let setup = settings
        .setup
        .as_ref()
        .map(std::fs::read_to_string)
        .transpose()?
        .unwrap_or_default();
    let lines = jobs.lines().collect::<std::io::Result<Vec<_>>>()?;
    let mut jobs = run(
        options,
        settings,
        &builder,
        &setup,
        lines,
        |_| Ok(()),
        |_, report| report,
    )?;
    let total = jobs.len();
    let ok = jobs.iter().filter(|job| job["ok"] == true).count();
    if let Some(only) = settings.only {
        jobs.retain(|job| job["ok"] == matches!(only, BatchOnly::Ok));
    }
    Ok(json!({"summary":{"jobs":total, "ok":ok, "failed":total-ok,
        "ms":start.elapsed().as_millis()}, "jobs":jobs}))
}

// The preflight runs on one prepared worker before any queued jobs. Fields uses
// it to check the base twice, keeping the same pool and checkpoints for variants.
pub(super) fn run<F: Fn() -> EmulatorBuilder + Sync, T: Send>(
    options: &Options,
    settings: &BatchOptions,
    builder: &F,
    setup: &str,
    jobs: Vec<String>,
    before: impl FnOnce(&mut Worker<'_, F>) -> Result<()> + Send,
    finish: impl Fn(&Worker<'_, F>, Value) -> T + Sync,
) -> Result<Vec<T>> {
    let queue = Mutex::new(jobs.into_iter().enumerate().collect::<VecDeque<_>>());
    let ready = Barrier::new(settings.workers);
    let failure = Mutex::new(None);
    let preflight_error = Mutex::new(None);
    let mut jobs = thread::scope(|scope| {
        let mut before = Some(before);
        let handles: Vec<_> = (0..settings.workers)
            .map(|_| {
                let before = before.take();
                let finish = &finish;
                let (queue, ready, failure, preflight_error) =
                    (&queue, &ready, &failure, &preflight_error);
                scope.spawn(move || {
                    let worker = Worker::new(options, builder, setup);
                    if let Err(error) = &worker {
                        failure
                            .lock()
                            .unwrap()
                            .get_or_insert_with(|| error.to_string());
                    }
                    ready.wait();
                    if failure.lock().unwrap().is_some() {
                        return Vec::new();
                    }
                    let mut worker = worker.unwrap();
                    if let Some(before) = before
                        && let Err(error) = before(&mut worker)
                    {
                        *preflight_error.lock().unwrap() = Some(error.to_string());
                    }
                    ready.wait();
                    if preflight_error.lock().unwrap().is_some() {
                        return Vec::new();
                    }
                    let mut results = Vec::new();
                    loop {
                        let Some((index, line)) = queue.lock().unwrap().pop_front() else {
                            break;
                        };
                        let mut report = worker.run(index, &line);
                        let mut attempts = 1;
                        while report["ok"] == false && attempts <= settings.retries {
                            report = worker.run(index, &line);
                            attempts += 1;
                        }
                        report["attempts"] = json!(attempts);
                        results.push((index, finish(&worker, report)));
                    }
                    results
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    if let Some(error) = failure
        .into_inner()
        .unwrap()
        .or(preflight_error.into_inner().unwrap())
    {
        return Err(error.into());
    }
    jobs.sort_by_key(|(index, _)| *index);
    Ok(jobs.into_iter().map(|(_, job)| job).collect())
}
