use base64::{Engine, prelude::BASE64_STANDARD};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};
use unidroid::Result;

struct Client {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    count: usize,
}
impl Client {
    fn start(policy: &std::path::Path, record: &std::path::Path) -> Result<Self> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_unidroid"))
            .args([
                "serve",
                "--fixed-clock",
                "1700000000",
                "--random-seed",
                "42",
                "--policy",
            ])
            .arg(policy)
            .arg("--record")
            .arg(record)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        Ok(Self {
            input: child.stdin.take().unwrap(),
            output: BufReader::new(child.stdout.take().unwrap()),
            child,
            count: 0,
        })
    }
    fn send(&mut self, value: Value) {
        writeln!(self.input, "{value}").unwrap();
        self.input.flush().unwrap();
        self.count += 1;
    }
    fn receive(&mut self) -> Value {
        let mut line = String::new();
        assert_ne!(self.output.read_line(&mut line).unwrap(), 0);
        serde_json::from_str(&line).unwrap()
    }
    fn request(&mut self, value: Value) -> Value {
        self.send(value);
        let reply = self.receive();
        assert!(reply.get("ok").is_some(), "{reply}");
        reply["ok"].clone()
    }
    fn error(&mut self, command: Value, expected: &str) {
        self.send(command);
        assert_eq!(self.receive(), json!({"error":expected}));
    }
    fn pointer(&mut self, address: u64, wide: bool) -> u64 {
        let value = self.request(
            json!({"cmd":"read", "address":address.to_string(), "size":if wide {8} else {4}}),
        );
        let bytes = BASE64_STANDARD.decode(value.as_str().unwrap()).unwrap();
        let mut word = [0; 8];
        word[..bytes.len()].copy_from_slice(&bytes);
        u64::from_le_bytes(word)
    }
    fn sources(&mut self, wide: bool, out: &Value) -> Vec<Value> {
        [(if wide {113} else {263}, vec![json!("0"), out.clone()], if wide {16} else {8}),
         (if wide {278} else {384}, vec![out.clone(), json!("17"), json!("0")], 17)]
        .into_iter().map(|(nr, args, size)| {
            self.request(json!({"cmd":"set_register", "name":if wide {"x8"} else {"r7"}, "value":nr.to_string()}));
            self.request(json!({"cmd":"call_function", "address":"131328", "args":args}));
            self.request(json!({"cmd":"read", "address":out, "size":size}))
        }).collect()
    }
    fn finish(self) -> Result<usize> {
        drop(self.input);
        let output = self.child.wait_with_output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(self.count)
    }
}

#[test]
fn serve_checkpoints_restore_policy_positions_and_replay_in_order() -> Result<()> {
    for wide in [true, false] {
        let abi = if wide { "arm64-v8a" } else { "armeabi-v7a" };
        let path = std::env::temp_dir().join(format!(
            "unidroid-checkpoint-{}-{abi}.json",
            std::process::id()
        ));
        let recording = path.with_extension("jsonl");
        std::fs::write(
            &path,
            r#"{"rules":[{"op":"is_assignable","values":[true,false,true]}]}"#,
        )?;
        let mut c = Client::start(&path, &recording)?;
        let module = c.request(
            json!({"cmd":"load", "path":format!("tests/fixtures/{abi}/libjnidispatch.so")}),
        );
        assert_eq!(c.request(json!({"cmd":"checkpoint"})), 1);
        let entry = c.request(json!({"cmd":"find_symbol", "module":module, "name":"Java_com_sun_jna_Native_getNativeVersion"}));
        let entry: u64 = entry.as_str().unwrap().parse()?;
        c.request(json!({"cmd":"break", "at":format!("0x{entry:x}")}));
        c.send(json!({"cmd":"call_static", "class":"com/sun/jna/Native", "method":"getNativeVersion()I", "args":[]}));
        let stop = c.receive();
        let env: u64 = stop["break"]["registers"][if wide { "x0" } else { "r0" }]
            .as_str()
            .unwrap()
            .parse()?;
        let between = "checkpoint and rewind work only between calls";
        c.error(json!({"cmd":"checkpoint"}), between);
        c.error(json!({"cmd":"rewind", "id":1}), between);
        c.send(json!({"cmd":"return", "value":"0"}));
        assert_eq!(c.receive(), json!({"ok":0}));
        c.request(json!({"cmd":"unbreak", "at":format!("0x{entry:x}")}));
        let table = c.pointer(env, wide);
        let function = c.pointer(table + 11 * if wide { 8 } else { 4 }, wide);
        let from = c.request(json!({"cmd":"class", "name":"test/A"}));
        let to = c.request(json!({"cmd":"class", "name":"test/B"}));
        let call = json!({"cmd":"call_function", "address":function.to_string(), "args":[env.to_string(), from.to_string(), to.to_string()]});
        assert_eq!(c.request(call.clone()), "1");
        let words: [u32; 2] = if wide {
            [0xd4000001, 0xd65f03c0]
        } else {
            [0xef000000, 0xe12fff1e]
        };
        c.request(json!({"cmd":"write", "address":"131328", "base64":BASE64_STANDARD.encode(words.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<_>>())}));
        let out = c.request(json!({"cmd":"malloc", "size":64}));
        assert_eq!(c.request(json!({"cmd":"checkpoint"})), 2);
        let later = c.request(json!({"cmd":"new_string", "value":"later"}));
        assert_eq!(c.request(call.clone()), "0");
        assert_eq!(c.request(call.clone()), "1");
        let sources = c.sources(wide, &out);
        assert_eq!(c.request(json!({"cmd":"rewind", "id":2})), Value::Null);
        c.send(json!({"cmd":"string", "handle":later}));
        assert!(
            c.receive()["error"]
                .as_str()
                .unwrap()
                .contains("invalid handle")
        );
        assert_eq!(
            c.request(json!({"cmd":"new_string", "value":"again"})),
            later
        );
        assert_eq!(c.request(call.clone()), "0");
        assert_eq!(c.request(call.clone()), "1");
        assert_eq!(c.sources(wide, &out), sources);
        c.send(call);
        assert_eq!(c.receive()["jni"]["op"], "is_assignable");
        c.error(json!({"cmd":"checkpoint"}), between);
        c.error(json!({"cmd":"rewind", "id":2}), between);
        c.send(json!({"cmd":"answer", "value":false}));
        assert_eq!(c.receive(), json!({"ok":"0"}));
        assert_eq!(
            c.request(json!({"cmd":"drop_checkpoint", "id":2})),
            Value::Null
        );
        for command in ["rewind", "drop_checkpoint"] {
            c.error(json!({"cmd":command, "id":2}), "no checkpoint 2");
            c.error(json!({"cmd":command, "id":99}), "no checkpoint 99");
        }
        assert_eq!(c.request(json!({"cmd":"checkpoint"})), 3);
        c.request(json!({"cmd":"drop_checkpoint", "id":1}));
        c.request(json!({"cmd":"drop_checkpoint", "id":3}));
        let count = c.finish()?;
        std::fs::remove_file(path)?;
        let replay = Command::new(env!("CARGO_BIN_EXE_unidroid"))
            .arg("replay")
            .arg(&recording)
            .output()?;
        assert!(
            replay.status.success(),
            "{}{}",
            String::from_utf8_lossy(&replay.stdout),
            String::from_utf8_lossy(&replay.stderr)
        );
        assert_eq!(
            String::from_utf8(replay.stdout)?,
            format!("replayed {count} commands, all responses match\n")
        );
        std::fs::remove_file(recording)?;
    }
    Ok(())
}
