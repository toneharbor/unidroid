use base64::{Engine, prelude::BASE64_STANDARD};
use serde_json::{Value, json};

#[path = "support/elf.rs"]
mod elf;
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

const CLASS: &str = "com/anjuke/mobile/sign/SignUtil";
const METHOD: &str = "getSign0(Ljava/lang/String;Ljava/lang/String;Ljava/util/Map;Ljava/lang/String;I)Ljava/lang/String;";
const SIGN: &str = "1000ed2f7e5ecd00bb4a6454e1d487b7dc7500040002000acc";

struct Client {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}
impl Client {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_unidroid"))
            .args(["serve", "--rootfs", "rootfs"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            input,
            output,
        }
    }
    fn send(&mut self, value: Value) {
        writeln!(self.input, "{value}").unwrap();
        self.input.flush().unwrap();
    }
    fn receive(&mut self) -> Value {
        let mut line = String::new();
        assert_ne!(
            self.output.read_line(&mut line).unwrap(),
            0,
            "server closed stdout"
        );
        serde_json::from_str(&line).unwrap()
    }
    fn request(&mut self, value: Value) -> Value {
        self.send(value);
        let reply = self.receive();
        assert!(reply.get("ok").is_some(), "{reply}");
        reply["ok"].clone()
    }
    fn finish(self) {
        drop(self.input);
        let output = self.child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[path = "serve/debug.rs"]
mod debug;
#[path = "serve/errors.rs"]
mod errors;

#[test]
fn eof_during_a_question_fails_the_process() {
    let mut client = Client::start();
    let module =
        client.request(json!({"cmd":"load", "path":"tests/fixtures/arm64-v8a/libjnidispatch.so"}));
    client.send(json!({"cmd":"onload", "module":module}));
    assert!(client.receive().get("jni").is_some());
    drop(client.input);
    let output = client.child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "error: stdin closed while a JNI question was waiting for an answer\n"
    );
}

#[test]
fn values_memory_and_c_calls_round_trip() {
    let mut client = Client::start();
    client.request(json!({"cmd":"alias", "name":"unused", "target":"strlen"}));
    let module = client.request(json!({"cmd":"load", "path":"rootfs/lib64/libc.so"}));
    let string = client.request(json!({"cmd":"new_cstring", "value":"hello"}));
    let strlen = client.request(json!({"cmd":"find_symbol", "module":module, "name":"strlen"}));
    assert_eq!(
        client.request(json!({"cmd":"call_function", "address":strlen, "args":[string]})),
        "5"
    );
    assert_eq!(
        client.request(json!({"cmd":"read", "address":string, "size":6})),
        "aGVsbG8A"
    );
    client.request(json!({"cmd":"free", "address":string}));
    let memory = client.request(json!({"cmd":"malloc", "size":4}));
    client.request(json!({"cmd":"write", "address":memory, "base64":"AQIDBA=="}));
    assert_eq!(
        client.request(json!({"cmd":"read", "address":memory, "size":4})),
        "AQIDBA=="
    );
    client.request(json!({"cmd":"free", "address":memory}));
    for kind in ["Z", "B", "C", "S", "I", "J", "F", "D"] {
        let array =
            client.request(json!({"cmd":"new_array", "type":kind, "base64":"AQIDBAUGBwg="}));
        assert_eq!(
            client.request(json!({"cmd":"array", "handle":array})),
            "AQIDBAUGBwg="
        );
        client.request(json!({"cmd":"release", "handle":array}));
    }
    let text = client.request(json!({"cmd":"new_string", "value":"a\u{0}é😀"}));
    let array = client
        .request(json!({"cmd":"new_object_array", "class":"java/lang/Object", "items":[text,0]}));
    let items = client.request(json!({"cmd":"object_array", "handle":array}));
    assert_eq!(items[1], 0);
    assert_eq!(
        client.request(json!({"cmd":"string", "handle":items[0]})),
        "a\u{0}é😀"
    );
    client.request(json!({"cmd":"release", "handle":text}));
    client.request(json!({"cmd":"release", "handle":items[0]}));
    client.request(json!({"cmd":"release", "handle":array}));
    let class = client.request(json!({"cmd":"class", "name":"java/lang/Object"}));
    assert_eq!(
        client.request(json!({"cmd":"class", "name":"java/lang/Object"})),
        class
    );
    client.send(json!({"cmd":"new_array", "type":"Q", "base64":""}));
    assert_eq!(client.receive(), json!({"error":"unknown array type: Q"}));
    client
        .send(json!({"cmd":"call_static", "class":CLASS, "method":METHOD, "args":[1,2,3,4,"10"]}));
    assert_eq!(
        client.receive(),
        json!({"error":format!("argument 4 of {METHOD} must be a number")})
    );
    for command in [
        json!({"cmd":"call_static", "class":CLASS, "method":METHOD, "args":[]}),
        json!({"cmd":"call", "class":CLASS, "receiver":0, "method":METHOD, "args":[]}),
    ] {
        client.send(command);
        assert_eq!(
            client.receive(),
            json!({"error":format!("{METHOD} takes 5 arguments, got 0")})
        );
    }
    client.finish();
}

#[test]
fn c_calls_forward_methods_fields_constructors_and_assignability() {
    // Export a JNI method that returns its JNIEnv pointer, using the existing tiny ELF builder.
    let library = elf::TestLibrary::new(false);
    let mut bytes = std::fs::read(&library.0).unwrap();
    let name = b"Java_test_Bridge_env\0";
    bytes[0x1800..0x1800 + name.len()].copy_from_slice(name);
    bytes[0x1218..0x121c].copy_from_slice(&0x700u32.to_le_bytes());
    bytes[0x1038..0x1040].copy_from_slice(&0x720u64.to_le_bytes());
    bytes[0x200..0x204].copy_from_slice(&0xd65f03c0u32.to_le_bytes()); // ret (x0 is JNIEnv)
    std::fs::write(&library.0, bytes).unwrap();
    let mut client = Client::start();
    client.request(json!({"cmd":"load", "path":library.0}));
    let env = client
        .request(json!({"cmd":"call_static", "class":"test/Bridge", "method":"env()J", "args":[]}));
    let table = read_pointer(&mut client, env.as_str().unwrap().parse().unwrap());
    let class = client.request(json!({"cmd":"class", "name":"test/Bridge"}));
    assert_eq!(
        client.request(
            json!({"cmd":"superclass", "class":"test/Bridge", "superclass":"test/Parent"})
        ),
        Value::Null
    );
    let parent = client.request(json!({"cmd":"class", "name":"test/Parent"}));
    let get_superclass = read_pointer(&mut client, table + 10 * 8);
    assert_eq!(client.request(json!({"cmd":"call_function", "address":get_superclass.to_string(), "args":[env, class.to_string()]})), parent.to_string());
    let object = client.request(json!({"cmd":"new_object", "class":"test/Bridge"}));
    let args = client.request(json!({"cmd":"malloc", "size":8}));

    for (offset, signature, value, bits) in [
        (
            0,
            "Ljava/lang/Object;",
            object.clone(),
            object.as_u64().unwrap(),
        ),
        (1, "Z", json!(true), 1),
        (2, "B", json!(-12), (-12i64) as u64),
        (3, "C", json!(55296), 55296),
        (4, "S", json!(-32000), (-32000i64) as u64),
        (5, "I", json!(-42), (-42i64) as u64),
        (
            6,
            "J",
            json!("-9007199254740993"),
            (-9007199254740993i64) as u64,
        ),
        (7, "F", json!(1.5), 1.5f32.to_bits() as u64),
        (8, "D", json!(-2.25), (-2.25f64).to_bits()),
    ] {
        let kind = &signature[..1];
        for (is_static, id_slot, call_slot, get_slot) in
            [(false, 33, 36, 95), (true, 113, 116, 145)]
        {
            let receiver = if is_static { &class } else { &object };
            let method_signature = format!("({signature}){signature}");
            let id = member_id(
                &mut client,
                table,
                &env,
                &class,
                id_slot,
                "echo",
                &method_signature,
            );
            client.request(json!({"cmd":"write", "address":args, "base64":BASE64_STANDARD.encode(bits.to_le_bytes())}));
            let function = read_pointer(&mut client, table + (call_slot + offset * 3) * 8);
            client.send(json!({"cmd":"call_function", "address":function.to_string(), "args":[env, receiver.to_string(), id, args]}));
            let question = client.receive();
            assert_eq!(
                question,
                json!({"jni":{
                    "op":"call", "static":is_static, "type":kind, "class":"test/Bridge",
                    "name":"echo", "signature":method_signature, "receiver":receiver, "args":[value]
                }})
            );
            client.send(json!({"cmd":"answer", "value":value}));
            assert_eq!(client.receive(), json!({"ok":bits.to_string()}));

            let id = member_id(
                &mut client,
                table,
                &env,
                &class,
                if is_static { 144 } else { 94 },
                "field",
                signature,
            );
            let function = read_pointer(&mut client, table + (get_slot + offset) * 8);
            client.send(json!({"cmd":"call_function", "address":function.to_string(), "args":[env, receiver.to_string(), id]}));
            assert_eq!(
                client.receive(),
                json!({"jni":{
                    "op":"get", "static":is_static, "type":kind, "class":"test/Bridge",
                    "name":"field", "signature":signature, "receiver":receiver, "args":[]
                }})
            );
            client.send(json!({"cmd":"answer", "value":value}));
            assert_eq!(client.receive(), json!({"ok":bits.to_string()}));
        }
    }
    for (is_static, id_slot, set_slot) in [(false, 94, 109), (true, 144, 159)] {
        let receiver = if is_static { &class } else { &object };
        let id = member_id(&mut client, table, &env, &class, id_slot, "field", "I");
        let function = read_pointer(&mut client, table + set_slot * 8);
        client.send(json!({"cmd":"call_function", "address":function.to_string(), "args":[env, receiver.to_string(), id, "42"]}));
        assert_eq!(
            client.receive(),
            json!({"jni":{
                "op":"set", "static":is_static, "type":"V", "class":"test/Bridge",
                "name":"field", "signature":"I", "receiver":receiver, "args":[42]
            }})
        );
        client.send(json!({"cmd":"answer"}));
        assert_eq!(client.receive(), json!({"ok":"0"}));
    }
    let id = member_id(&mut client, table, &env, &class, 33, "<init>", "()V");
    let function = read_pointer(&mut client, table + 30 * 8);
    client.send(json!({"cmd":"call_function", "address":function.to_string(), "args":[env, class.to_string(), id, args]}));
    assert_eq!(
        client.receive(),
        json!({"jni":{
            "op":"new", "static":false, "type":"L", "class":"test/Bridge",
            "name":"<init>", "signature":"()V", "receiver":class, "args":[]
        }})
    );
    client.send(json!({"cmd":"answer", "value":object}));
    assert_eq!(client.receive(), json!({"ok":object.to_string()}));
    let other = client.request(json!({"cmd":"class", "name":"test/Other"}));
    let function = read_pointer(&mut client, table + 11 * 8);
    client.send(json!({"cmd":"call_function", "address":function.to_string(), "args":[env, class.to_string(), other.to_string()]}));
    assert_eq!(
        client.receive(),
        json!({"jni":{"op":"is_assignable", "from":"test/Bridge", "to":"test/Other"}})
    );
    client.send(json!({"cmd":"answer", "value":true}));
    assert_eq!(client.receive(), json!({"ok":"1"}));
    client.finish();
}

fn read_pointer(client: &mut Client, address: u64) -> u64 {
    let value = client.request(json!({"cmd":"read", "address":address.to_string(), "size":8}));
    u64::from_le_bytes(
        BASE64_STANDARD
            .decode(value.as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap(),
    )
}

fn member_id(
    client: &mut Client,
    table: u64,
    env: &Value,
    class: &Value,
    slot: u64,
    name: &str,
    signature: &str,
) -> Value {
    let name = client.request(json!({"cmd":"new_cstring", "value":name}));
    let signature = client.request(json!({"cmd":"new_cstring", "value":signature}));
    let function = read_pointer(client, table + slot * 8);
    client.request(json!({"cmd":"call_function", "address":function.to_string(), "args":[env, class.to_string(), name, signature]}))
}

#[test]
fn signutil_over_the_pipe() {
    let mut client = Client::start();
    let module =
        client.request(json!({"cmd":"load", "path":"tests/fixtures/armeabi-v7a/libsignutil.so"}));
    let a = client.request(json!({"cmd":"new_string", "value":"a"}));
    let b = client.request(json!({"cmd":"new_string", "value":"b"}));
    let bytes_a = client.request(json!({"cmd":"new_array", "type":"B", "base64":"Yg=="}));
    let bytes_b = client.request(json!({"cmd":"new_array", "type":"B", "base64":"Yg=="}));
    let keys = client
        .request(json!({"cmd":"new_object_array", "class":"java/lang/Object", "items":[a,b]}));
    let set = client.request(json!({"cmd":"new_object", "class":"java/util/Set"}));
    assert_eq!(
        client.request(json!({"cmd":"onload", "module":module})),
        json!(0x10004)
    );
    let aa = client.request(json!({"cmd":"new_string", "value":"aa"}));
    let bb = client.request(json!({"cmd":"new_string", "value":"bb"}));
    let map = client.request(json!({"cmd":"new_object", "class":"java/util/HashMap"}));
    let cc = client.request(json!({"cmd":"new_string", "value":"cc"}));
    client.send(
        json!({"cmd":"call_static", "class":CLASS, "method":METHOD, "args":[aa,bb,map,cc,10]}),
    );
    let mut suspended = None;
    let mut nested = false;
    let result = loop {
        let mut message = client.receive();
        if let Some(result) = message.get("ok") {
            if let Some(outer) = suspended.take() {
                assert_eq!(
                    client.request(json!({"cmd":"string", "handle":result})),
                    SIGN
                );
                message = outer;
            } else {
                break result.clone();
            }
        }
        if !nested {
            assert!(message.get("jni").is_some(), "{message}");
            suspended = Some(message);
            nested = true;
            let receiver = client.request(json!({"cmd":"class", "name":CLASS}));
            client.send(json!({"cmd":"call", "class":CLASS, "receiver":receiver, "method":METHOD, "args":[aa,bb,map,cc,10]}));
            continue;
        }
        let q = &message["jni"];
        assert_eq!(q["op"], "call", "{message}");
        assert_eq!(q["static"], false);
        let value = match (
            q["class"].as_str().unwrap(),
            q["name"].as_str().unwrap(),
            q["signature"].as_str().unwrap(),
        ) {
            ("java/util/HashMap", "size", "()I") => {
                assert_eq!(q["type"], "I");
                json!(2)
            }
            ("java/util/HashMap", "keySet", "()Ljava/util/Set;") => set.clone(),
            ("java/util/Set", "toArray", "()[Ljava/lang/Object;") => keys.clone(),
            ("java/util/HashMap", "get", "(Ljava/lang/Object;)Ljava/lang/Object;") => {
                let key = client.request(json!({"cmd":"string", "handle":q["args"][0]}));
                match key.as_str().unwrap() {
                    "a" => bytes_a.clone(),
                    "b" => bytes_b.clone(),
                    _ => json!(0),
                }
            }
            _ => panic!("{message}"),
        };
        client.send(json!({"cmd":"answer", "value":value}));
    };
    assert_eq!(
        client.request(json!({"cmd":"string", "handle":result})),
        SIGN
    );
    client.finish();
}
