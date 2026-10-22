use base64::{Engine, prelude::BASE64_STANDARD};
use serde_json::{Value, json};
use unidroid::{
    Emulator,
    serve::{self, FieldsOptions, Options},
};

#[path = "fields/cli.rs"]
mod cli;
#[path = "fields/digest.rs"]
mod digest;
#[path = "fields/errors.rs"]
mod errors;
#[path = "fields/structured.rs"]
mod structured;
#[path = "fields/variants.rs"]
mod variants;

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "unidroid-fields-{}-{}",
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

fn libc(arm64: bool) -> &'static str {
    if arm64 {
        "rootfs/lib64/libc.so"
    } else {
        "rootfs/lib/libc.so"
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

fn analyze(spec: Value, workers: usize) -> Value {
    serve::fields(
        &options(),
        &FieldsOptions { workers },
        Emulator::builder,
        spec,
    )
    .unwrap()
}

#[test]
fn empty_outputs_zero_length_fields_and_utf8_byte_offsets_are_supported() {
    let report = analyze(
        json!({
            "commands":[{"cmd":"new_string", "value":""}, {"cmd":"string", "handle":{"$ref":0}}],
            "fields":[{"name":"empty", "command":0, "offset":0, "length":0}]
        }),
        1,
    );
    assert_eq!(
        report,
        json!({
            "output":{"length":0, "base64":""},
            "fields":[{"name":"empty", "runs":1, "changed":[]}],
            "by_output":[], "unaffected":[]
        })
    );
    let report = analyze(
        json!({
            "commands":[{"cmd":"new_string", "value":"é"}, {"cmd":"string", "handle":{"$ref":0}}],
            "fields":[{"name":"accent", "command":0, "offset":1, "length":1, "values":["qA=="]}]
        }),
        1,
    );
    assert_eq!(report["output"], json!({"length":2, "base64":"w6k="}));
    assert_eq!(
        report["fields"],
        json!([{"name":"accent", "runs":1, "changed":[[1,2]]}])
    );
}

#[test]
fn output_decoding_uses_command_names_resolved_from_refs_and_setup() {
    for setup in [false, true] {
        let commands = if setup {
            json!([
                {"cmd":"new_string", "value":"abc"},
                {"cmd":{"$setup":1}, "handle":{"$ref":0}}
            ])
        } else {
            json!([
                {"cmd":"new_string", "value":"abc"},
                {"cmd":"new_string", "value":"string"},
                {"cmd":"string", "handle":{"$ref":1}},
                {"cmd":{"$ref":2}, "handle":{"$ref":0}}
            ])
        };
        let report = analyze(
            json!({
                "setup":[{"cmd":"new_string", "value":"string"}, {"cmd":"string", "handle":{"$ref":0}}],
                "commands":commands,
                "fields":[{"name":"text", "command":0, "offset":0, "length":1}]
            }),
            2,
        );
        assert_eq!(report["output"], json!({"length":3, "base64":"YWJj"}));
        assert_eq!(
            report["fields"],
            json!([{"name":"text", "runs":1, "changed":[[0,1]]}])
        );
    }
}

#[test]
fn non_string_results_use_compact_json_text() {
    let report = analyze(
        json!({
            "commands":[{"cmd":"new_string", "value":"a"}, {"cmd":"release", "handle":{"$ref":0}}],
            "fields":[{"name":"text", "command":0, "offset":0, "length":1}]
        }),
        1,
    );
    assert_eq!(
        report,
        json!({
            "output":{"length":4, "base64":"bnVsbA=="},
            "fields":[{"name":"text", "runs":1, "changed":[]}],
            "by_output":[], "unaffected":[[0,4]]
        })
    );
}

#[test]
fn string_fields_report_only_their_bytes_in_input_order() {
    let spec = json!({
        "commands":[
            {"cmd":"new_string", "value":"abcdef"},
            {"cmd":"string", "handle":{"$ref":0}}
        ],
        "fields":[
            {"name":"right", "command":0, "offset":3, "length":2},
            {"name":"left", "command":0, "offset":0, "length":2},
            {"name":"overlap", "command":0, "offset":1, "length":3}
        ]
    });
    let expected = json!({
        "output":{"length":6, "base64":"YWJjZGVm"},
        "fields":[
            {"name":"right", "runs":1, "changed":[[3,5]]},
            {"name":"left", "runs":1, "changed":[[0,2]]},
            {"name":"overlap", "runs":1, "changed":[[1,4]]}
        ],
        "by_output":[
            {"start":0, "end":1, "fields":["left"]},
            {"start":1, "end":2, "fields":["left", "overlap"]},
            {"start":2, "end":3, "fields":["overlap"]},
            {"start":3, "end":4, "fields":["right", "overlap"]},
            {"start":4, "end":5, "fields":["right"]}
        ],
        "unaffected":[[5,6]]
    });
    for workers in [1, 3] {
        assert_eq!(analyze(spec.clone(), workers), expected);
    }
}
