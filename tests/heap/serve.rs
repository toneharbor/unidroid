use super::*;
use serde_json::{Value, json};

fn commands(emu: &mut Emulator, commands: &[Value]) -> Result<Vec<Value>> {
    let lines = commands
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let mut output = Vec::new();
    unidroid::serve::serve(emu, None, std::io::Cursor::new(lines), &mut output)?;
    String::from_utf8(output)?
        .lines()
        .map(|line| Ok(serde_json::from_str(line)?))
        .collect()
}

#[test]
fn serve_heap_reset_object_depth_and_trace_defaults_use_decimal_strings() -> Result<()> {
    for arm64 in [true, false] {
        let mut emu = emulator(arm64)?;
        emu.set_trace(Trace::default())?;
        let malloc = emu.resolve("malloc")?;
        let replies = commands(
            &mut emu,
            &[
                json!({"cmd":"trace", "heap":true}),
                json!({"cmd":"call_function", "address":malloc.to_string(), "args":["32"]}),
                json!({"cmd":"heap"}),
            ],
        )?;
        assert_eq!(replies[0], json!({"ok": null}));
        let address: u64 = replies[1]["ok"].as_str().unwrap().parse()?;
        let records = replies[2]["ok"].as_array().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["address"], address.to_string());
        assert_eq!(records[0]["size"], "32");
        assert_eq!(records[0]["thread"], 1234);
        assert_eq!(records[0]["function"], "malloc");
        assert_eq!(records[0]["backtrace"][0]["address"], "131072");
        assert_eq!(records[0]["backtrace"][0]["symbol"], "host return");
        let host = emu.new_cstring("serve string")?;
        write_pointer(&mut emu, address, host, arm64)?;
        let replies = commands(
            &mut emu,
            &[
                json!({"cmd":"object", "at":format!("0x{:x}", address + 1)}),
                json!({"cmd":"object", "at":format!("0x{address:x}"), "depth":0}),
                json!({"cmd":"object", "at":format!("0x{host:x}")}),
                json!({"cmd":"heap", "reset":true}),
                json!({"cmd":"heap"}),
                json!({"cmd":"trace"}),
                json!({"cmd":"call_function", "address":malloc.to_string(), "args":["64"]}),
                json!({"cmd":"heap"}),
                json!({"cmd":"object", "at":"0xdeadbeef"}),
            ],
        )?;
        assert_eq!(replies[0]["ok"]["address"], address.to_string());
        assert_eq!(replies[0]["ok"]["fields"][0]["offset"], "0");
        assert_eq!(replies[0]["ok"]["fields"][0]["value"], host.to_string());
        assert_eq!(replies[0]["ok"]["fields"][0]["string"], "serve string");
        assert_eq!(
            replies[0]["ok"]["fields"][0]["target"]["object"]["address"],
            host.to_string()
        );
        assert!(replies[1]["ok"]["fields"][0]["target"]["object"].is_null());
        let refs = replies[2]["ok"]["referrers"].as_array().unwrap();
        let address_text = address.to_string();
        let reference = refs.iter().find(|r| r["address"] == address_text).unwrap();
        assert_eq!(reference["block"], address.to_string());
        assert_eq!(reference["object"]["address"], address.to_string());
        assert_eq!(replies[3]["ok"], Value::Array(records.clone()));
        assert_eq!(replies[4]["ok"], json!([]));
        assert!(!emu.trace().heap);
        assert_eq!(replies[7]["ok"], json!([]));
        assert_eq!(
            replies[8]["error"],
            "0xdeadbeef is not inside a known allocation"
        );
    }
    Ok(())
}
