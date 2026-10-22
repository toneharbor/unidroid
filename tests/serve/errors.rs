use super::*;

#[test]
fn errors_leave_the_server_ready_for_the_next_command() {
    for (abi, location) in [
        ("arm64-v8a", "libjnidispatch.so+0x8bc4"),
        ("armeabi-v7a", "libjnidispatch.so+0x8ffc"),
    ] {
        let mut client = Client::start();
        writeln!(client.input, "not-json").unwrap();
        assert_eq!(
            client.receive(),
            json!({"error":"bad message: expected ident at line 1 column 2"})
        );
        client.request(json!({"cmd":"new_string", "value":"after bad message"}));
        for command in [
            json!({"cmd":"answer", "value":0}),
            json!({"cmd":"unhandled"}),
        ] {
            client.send(command);
            assert_eq!(
                client.receive(),
                json!({"error":"no JNI question is waiting for an answer"})
            );
            client.request(json!({"cmd":"new_string", "value":"after stray answer"}));
        }
        let module = client.request(
            json!({"cmd":"load", "path":format!("tests/fixtures/{abi}/libjnidispatch.so")}),
        );
        let unhandled = format!(
            "unhandled JNI call: GetStaticObjectField java/lang/Void->TYPELjava/lang/Class; from {location}"
        );
        for (answer, error) in [
            (json!({"cmd":"unhandled"}), unhandled.as_str()),
            (
                json!({"cmd":"answer", "value":false}),
                "answer for java/lang/Void->TYPELjava/lang/Class; must be a handle",
            ),
        ] {
            client.send(json!({"cmd":"onload", "module":module}));
            let question = client.receive();
            assert_eq!(question["jni"]["op"], "get");
            assert_eq!(question["jni"]["static"], true);
            assert_eq!(question["jni"]["type"], "L");
            assert_eq!(question["jni"]["args"], json!([]));
            client.send(answer);
            let response = client.receive();
            assert_eq!(response["error"], error);
            assert_eq!(response["crash"]["thread"], 1234);
            assert!(
                response["crash"]["message"]
                    .as_str()
                    .unwrap()
                    .starts_with("unhandled JNI call:")
            );
            assert_eq!(response["crash"]["backtrace"][0]["location"], location);
            let jni = response["crash"]["jni"].as_array().unwrap();
            assert_eq!(
                jni.last().unwrap(),
                &json!(format!(
                    "JNIEnv->GetStaticObjectField java/lang/Void->TYPELjava/lang/Class; from {location}"
                ))
            );
            assert!(jni.iter().any(|line| {
                line.as_str()
                    .unwrap()
                    .starts_with("JNIEnv->FindClass(java/lang/Void) from libjnidispatch.so+")
            }));
            assert!(jni.iter().any(|line| {
                line.as_str().unwrap().starts_with(
                    "JNIEnv->GetStaticFieldID(java/lang/Void->TYPELjava/lang/Class;) from libjnidispatch.so+"
                )
            }));
            assert!(response["crash"]["backtrace"][0]["address"].is_string());
            let registers = response["crash"]["registers"].as_object().unwrap();
            assert!(registers.values().all(|value| {
                value
                    .as_str()
                    .is_some_and(|text| text.parse::<u64>().is_ok())
            }));
            assert!(registers["pc"].is_string());
            let stack = response["crash"]["stack"].as_array().unwrap();
            assert_eq!(stack.len(), 32);
            assert_eq!(stack[0][0], registers["sp"]);
            let sp = registers["sp"].as_str().unwrap().parse::<u64>().unwrap();
            let width = if abi == "arm64-v8a" { 8 } else { 4 };
            for (i, pair) in stack.iter().enumerate() {
                let pair = pair.as_array().unwrap();
                assert_eq!(pair.len(), 2);
                assert_eq!(pair[0], (sp + i as u64 * width).to_string());
                assert!(pair[1].as_str().unwrap().parse::<u64>().is_ok());
            }
            assert!(registers[if abi == "arm64-v8a" { "x0" } else { "r0" }].is_string());
            client.request(json!({"cmd":"new_string", "value":"after unanswered question"}));
        }
        client.finish();
    }
}

#[test]
fn invalid_handles_modules_and_paths_leave_the_server_ready() {
    let mut client = Client::start();
    for (command, error) in [
        (
            json!({"cmd":"array", "type":"B", "handle":999}),
            "invalid handle 999: no live object (never created, or already released)",
        ),
        (
            json!({"cmd":"find_symbol", "module":99, "name":"x"}),
            "no module 99 is loaded",
        ),
        (
            json!({"cmd":"onload", "module":99}),
            "no module 99 is loaded",
        ),
        (
            json!({"cmd":"load", "path":"."}),
            "cannot load .: not a file path",
        ),
    ] {
        client.send(command);
        assert_eq!(client.receive(), json!({"error":error}));
        let handle = client.request(json!({"cmd":"new_string", "value":"still ready"}));
        assert_eq!(
            client.request(json!({"cmd":"string", "handle":handle})),
            "still ready"
        );
    }
    let string = client.request(json!({"cmd":"new_string", "value":"abc"}));
    let array = client.request(json!({"cmd":"new_array", "type":"B", "base64":"AQID"}));
    for (command, error) in [
        (
            json!({"cmd":"array", "handle":string}),
            format!("invalid handle {string}: java/lang/String, not a primitive array"),
        ),
        (
            json!({"cmd":"string", "handle":array}),
            format!("invalid handle {array}: [B, not a string"),
        ),
        (
            json!({"cmd":"object_array", "handle":string}),
            format!("invalid handle {string}: java/lang/String, not an object array"),
        ),
        (
            json!({"cmd":"new_object_array", "class":"java/lang/Object", "items":[999]}),
            "invalid handle 999: no live object (never created, or already released)".into(),
        ),
    ] {
        client.send(command);
        assert_eq!(client.receive(), json!({"error":error}));
        assert_eq!(
            client.request(json!({"cmd":"string", "handle":string})),
            "abc"
        );
    }
    client.finish();
}
