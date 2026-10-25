use super::*;
use std::io::Cursor;
use unidroid::serve;

fn commands(emu: &mut Emulator, input: &[Value]) -> Result<Vec<Value>> {
    let lines = input.iter().map(|v| format!("{v}\n")).collect::<String>();
    let mut output = Vec::new();
    serve::serve(emu, None, Cursor::new(lines), &mut output)?;
    String::from_utf8(output)?
        .lines()
        .map(|line| Ok(serde_json::from_str(line)?))
        .collect()
}

#[test]
fn serve_tracepoints_counts_reset_and_conditional_breaks_use_the_wire_format() -> Result<()> {
    for arm64 in [true, false] {
        let words = if arm64 {
            [0x91000400, 0xd65f03c0]
        } else {
            [0xe2800001, 0xe12fff1e]
        };
        let (mut emu, events) = emulator(arm64, &words)?;
        let reg = if arm64 { "x0" } else { "r0" };
        let output = commands(
            &mut emu,
            &[
                json!({"cmd":"tracepoint","at":"0x20101","values":[reg,"[0x0]:1"]}),
                json!({"cmd":"tracepoint","at":"0x20104","values":[reg],"log":false}),
                json!({"cmd":"call_function","address":CODE.to_string(),"args":["7"]}),
                json!({"cmd":"tracepoint_hits","at":"0x20100","reset":true}),
                json!({"cmd":"tracepoint_hits","at":"0x20100"}),
                json!({"cmd":"tracepoint_hits","at":"0x20104"}),
                json!({"cmd":"call_function","address":CODE.to_string(),"args":["2"]}),
                json!({"cmd":"tracepoint_hits","at":"0x20101"}),
                json!({"cmd":"untracepoint","at":"0x20101"}),
                json!({"cmd":"untracepoint","at":"0x20104"}),
                json!({"cmd":"tracepoint_hits","at":"0x20100"}),
                json!({"cmd":"break","at":"0x20101","if":format!("{reg}==0x7")}),
                json!({"cmd":"call_function","address":CODE.to_string(),"args":["7"]}),
                json!({"cmd":"continue"}),
                json!({"cmd":"call_function","address":CODE.to_string(),"args":["2"]}),
                json!({"cmd":"break","at":"0x20100"}),
                json!({"cmd":"call_function","address":CODE.to_string(),"args":["2"]}),
                json!({"cmd":"continue"}),
                json!({"cmd":"unbreak","at":"0x20100"}),
            ],
        )?;
        assert_eq!(
            &output[..12],
            &[
                json!({"ok":"131328"}),
                json!({"ok":"131332"}),
                json!({"ok":"8"}),
                json!({"ok":[{"value":"7","count":1}]}),
                json!({"ok":[]}),
                json!({"ok":[{"value":"8","count":1}]}),
                json!({"ok":"3"}),
                json!({"ok":[{"value":"2","count":1}]}),
                json!({"ok":null}),
                json!({"ok":null}),
                json!({"error":"no tracepoint at 0x20100"}),
                json!({"ok":"131328"}),
            ]
        );
        assert_eq!(output.len(), 19);
        assert_eq!(output[12]["break"]["reason"], "breakpoint");
        assert_eq!(output[12]["break"]["address"], "131328");
        assert_eq!(output[12]["break"]["registers"][reg], "7");
        assert_eq!(output[13], json!({"ok":"8"}));
        assert_eq!(output[14], json!({"ok":"3"}));
        assert_eq!(output[15], json!({"ok":"131328"}));
        assert_eq!(output[16]["break"]["registers"][reg], "2");
        assert_eq!(output[17], json!({"ok":"3"}));
        assert_eq!(output[18], json!({"ok":null}));
        let events = events.borrow();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].0["values"],
            json!([[reg, "0x7"], ["[0x0]:1", null]])
        );
        assert_eq!(
            events[1].0["values"],
            json!([[reg, "0x2"], ["[0x0]:1", null]])
        );
    }
    Ok(())
}

#[test]
fn cli_json_tracepoint_events_go_only_to_stderr() -> Result<()> {
    use base64::{Engine, prelude::BASE64_STANDARD};
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    for arm64 in [true, false] {
        let reg = if arm64 { "x0" } else { "r0" };
        let ret: u32 = if arm64 { 0xd65f03c0 } else { 0xe12fff1e };
        let mut child = Command::new(env!("CARGO_BIN_EXE_unidroid"))
            .args(["serve", "--rootfs", "rootfs", "--log-format", "json"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut input = child.stdin.take().unwrap();
        for command in [
            json!({"cmd":"load","path":if arm64 { "rootfs/lib64/libc.so" } else { "rootfs/lib/libc.so" }}),
            json!({"cmd":"write","address":CODE.to_string(),"base64":BASE64_STANDARD.encode(ret.to_le_bytes())}),
            json!({"cmd":"tracepoint","at":"0x20100","values":[reg]}),
            json!({"cmd":"call_function","address":CODE.to_string(),"args":["42"]}),
        ] {
            writeln!(input, "{command}")?;
        }
        drop(input);
        let output = child.wait_with_output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let replies: Vec<Value> = String::from_utf8(output.stdout)?
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        assert_eq!(
            replies,
            [
                json!({"ok":0}),
                json!({"ok":null}),
                json!({"ok":"131328"}),
                json!({"ok":"42"})
            ]
        );
        let events: Vec<Value> = String::from_utf8(output.stderr)?
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        assert_eq!(
            events,
            [
                json!({"event":"tracepoint","at":"0x20100","address":"0x20100","thread":1234,"values":[[reg,"0x2a"]]})
            ]
        );
    }
    Ok(())
}

#[test]
fn batch_refuses_tracepoint_changes_but_allows_count_queries() -> Result<()> {
    let options = serde_json::from_value(json!({
        "rootfs":"rootfs", "apk":null, "process_name":null, "prop":[], "alias":[], "file":[],
        "superclass":[], "translation_cache_mb":8, "no_threads":false
    }))?;
    let batch = serve::BatchOptions {
        workers: 1,
        ..Default::default()
    };
    let input = [
        json!({"commands":[{"cmd":"tracepoint","at":"0x20100","values":["0x0"]}]}),
        json!({"commands":[{"cmd":"untracepoint","at":"0x20100"}]}),
        json!({"commands":[{"cmd":"tracepoint_hits","at":"0x20100"}]}),
    ]
    .iter()
    .map(|v| format!("{v}\n"))
    .collect::<String>();
    let output = serve::batch(&options, &batch, Emulator::builder, Cursor::new(input))?;
    assert_eq!(
        output["jobs"][0]["error"],
        "tracepoint is not available in batch"
    );
    assert_eq!(
        output["jobs"][1]["error"],
        "untracepoint is not available in batch"
    );
    assert_eq!(output["jobs"][2]["error"], "no tracepoint at 0x20100");
    let path = std::env::temp_dir().join(format!(
        "unidroid-tracepoint-batch-{}.jsonl",
        std::process::id()
    ));
    for name in ["tracepoint", "untracepoint"] {
        std::fs::write(
            &path,
            format!("{}\n", json!({"cmd":name,"at":"0x20100","values":["0x0"]})),
        )?;
        let settings = serve::BatchOptions {
            setup: Some(path.clone()),
            workers: 1,
            ..Default::default()
        };
        let result = serve::batch(&options, &settings, Emulator::builder, Cursor::new(""));
        std::fs::remove_file(&path)?;
        assert_eq!(
            result.unwrap_err().to_string(),
            format!("setup command 1: {name} is not available in batch")
        );
    }
    Ok(())
}
