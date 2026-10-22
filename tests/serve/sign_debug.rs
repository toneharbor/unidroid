use super::*;

#[test]
fn signutil_break_registers_backtrace_finish_continue() {
    let mut client = Client::start();
    let module =
        client.request(json!({"cmd":"load", "path":"tests/fixtures/armeabi-v7a/libsignutil.so"}));
    let a = client.request(json!({"cmd":"new_string", "value":"a"}));
    let b = client.request(json!({"cmd":"new_string", "value":"b"}));
    let bytes = client.request(json!({"cmd":"new_array", "type":"B", "base64":"Yg=="}));
    let keys = client
        .request(json!({"cmd":"new_object_array", "class":"java/lang/Object", "items":[a,b]}));
    let set = client.request(json!({"cmd":"new_object", "class":"java/util/Set"}));
    assert_eq!(
        client.request(json!({"cmd":"onload", "module":module})),
        65540
    );
    let aa = client.request(json!({"cmd":"new_string", "value":"aa"}));
    let bb = client.request(json!({"cmd":"new_string", "value":"bb"}));
    let map = client.request(json!({"cmd":"new_object", "class":"java/util/HashMap"}));
    let cc = client.request(json!({"cmd":"new_string", "value":"cc"}));
    let symbol = "Java_com_anjuke_mobile_sign_SignUtil_getSign0";
    let address = client.request(json!({"cmd":"break", "at":symbol}));
    client.send(
        json!({"cmd":"call_static", "class":CLASS, "method":METHOD, "args":[aa,bb,map,cc,10]}),
    );
    let stop = client.receive();
    assert_eq!(stop["break"]["reason"], "breakpoint");
    assert_eq!(stop["break"]["address"], address);
    assert_eq!(stop["break"]["thread"], 1234);
    assert!(stop["break"]["location"].as_str().unwrap().contains(symbol));
    let registers = client.request(json!({"cmd":"registers"}));
    assert_eq!(registers, stop["break"]["registers"]);
    let frames = client.request(json!({"cmd":"backtrace"}));
    assert_eq!(frames[0]["address"], address);
    assert!(frames[0]["symbol"].as_str().unwrap().contains(symbol));
    client.send(json!({"cmd":"finish"}));
    loop {
        let message = client.receive();
        if let Some(stop) = message.get("break") {
            assert_eq!(stop["reason"], "finish");
            assert_eq!(stop["address"], registers["lr"]);
            assert_eq!(stop["registers"]["sp"], registers["sp"]);
            let result: u64 = stop["registers"]["r0"].as_str().unwrap().parse().unwrap();
            assert_eq!(
                client.request(json!({"cmd":"string", "handle":result})),
                SIGN
            );
            client.send(json!({"cmd":"continue"}));
            assert_eq!(client.receive(), json!({"ok":result}));
            assert_eq!(
                client.request(json!({"cmd":"string", "handle":result})),
                SIGN
            );
            break;
        }
        let q = &message["jni"];
        let value = match q["name"].as_str().unwrap() {
            "size" => json!(2),
            "keySet" => set.clone(),
            "toArray" => keys.clone(),
            "get" => {
                let key = client.request(json!({"cmd":"string", "handle":q["args"][0]}));
                assert!(key == "a" || key == "b");
                bytes.clone()
            }
            _ => panic!("{message}"),
        };
        client.send(json!({"cmd":"answer", "value":value}));
    }
    client.finish();
}
