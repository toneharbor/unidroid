use serde_json::{Value, json};
use unidroid::{
    Emulator,
    serve::{self, BatchOptions, Options},
};

#[path = "batch/acceptance.rs"]
mod acceptance;
#[path = "batch/cli.rs"]
mod cli;
#[path = "batch/diagnostics.rs"]
mod diagnostics;
#[path = "batch/errors.rs"]
mod errors;
#[path = "batch/mappings.rs"]
mod mappings;
#[path = "batch/policy.rs"]
mod policy;
#[path = "batch/retries.rs"]
mod retries;
#[path = "batch/setup.rs"]
mod setup;

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "unidroid-batch-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn file(&self, name: &str, content: &str) -> std::path::PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn options() -> Options {
    serde_json::from_value(json!({
        "rootfs":"rootfs", "process_name":"unidroid", "prop":[], "alias":[],
        "superclass":[], "fixed_clock":1700000000, "random_seed":42,
        "translation_cache_mb":8
    }))
    .unwrap()
}

fn lines(values: &[Value]) -> String {
    values.iter().map(|value| format!("{value}\n")).collect()
}

#[test]
fn fresh_jobs_use_references_and_keep_input_order() {
    let jobs = lines(&[
        json!({"id":{"input":1}, "commands":[
            {"cmd":"new_string", "value":"hello"},
            {"cmd":"string", "handle":{"$ref":0}}
        ]}),
        json!({"commands":[{"cmd":"new_string", "value":"next"}]}),
    ]);
    let report = serve::batch(
        &options(),
        &BatchOptions {
            workers: 3,
            ..Default::default()
        },
        Emulator::builder,
        jobs.as_bytes(),
    )
    .unwrap();
    assert_eq!(report["summary"]["jobs"], 2);
    assert_eq!(report["summary"]["ok"], 2);
    assert_eq!(report["summary"]["failed"], 0);
    assert_eq!(report["jobs"][0]["id"], json!({"input":1}));
    assert_eq!(report["jobs"][0]["results"][1], "hello");
    assert_eq!(report["jobs"][1]["id"], 2);
    assert_eq!(
        report["jobs"][0]["results"][0],
        report["jobs"][1]["results"][0]
    );
    assert_eq!(report["jobs"][0]["attempts"], 1);
}
