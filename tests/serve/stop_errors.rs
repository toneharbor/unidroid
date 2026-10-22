use super::*;

#[test]
fn innermost_question_owns_answers_and_resume_commands() {
    for (abi, words) in [
        ("arm64-v8a", [0x91000400u32, 0xd65f03c0]),
        ("armeabi-v7a", [0xe2800001, 0xe12fff1e]),
    ] {
        let mut client = Client::start();
        let resumes = ["continue", "step", "finish", "return", "abort"];
        for cmd in resumes {
            client.send(json!({"cmd":cmd,"value":"42"}));
            assert_eq!(
                client.receive(),
                json!({"error":"no breakpoint stop is waiting"})
            );
        }
        let module = client.request(
            json!({"cmd":"load","path":format!("tests/fixtures/{abi}/libjnidispatch.so")}),
        );
        client.request(json!({"cmd":"write", "address":"131328", "base64":BASE64_STANDARD.encode(words.into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>())}));
        client.request(json!({"cmd":"break","at":"0x20100"}));
        let call = json!({"cmd":"call_function","address":"131328","args":["7"]});
        client.send(call.clone());
        assert_eq!(client.receive()["break"]["reason"], "breakpoint");
        for cmd in ["answer", "unhandled"] {
            client.send(json!({"cmd":cmd,"value":0}));
            assert_eq!(
                client.receive(),
                json!({"error":"breakpoint stop expects continue, step, finish, return or abort"})
            );
        }
        // JNI question nested inside a stop.
        client.send(json!({"cmd":"onload","module":module}));
        assert!(client.receive().get("jni").is_some());
        for cmd in resumes {
            client.send(json!({"cmd":cmd,"value":"42"}));
            assert_eq!(
                client.receive(),
                json!({"error":"JNI question expects answer or unhandled"})
            );
        }
        // Stop nested inside that JNI question, then another stop at the same address.
        client.send(call.clone());
        assert_eq!(client.receive()["break"]["reason"], "breakpoint");
        client.send(call);
        assert_eq!(client.receive()["break"]["reason"], "breakpoint");
        client.send(json!({"cmd":"return","value":"42"}));
        assert_eq!(client.receive(), json!({"ok":"42"}));
        client.send(json!({"cmd":"abort"}));
        let error = client.receive();
        assert_eq!(error["error"], "stopped by the debugger at 0x20100");
        assert_eq!(error["crash"]["registers"]["pc"], "131328");
        client.send(json!({"cmd":"continue"}));
        assert_eq!(
            client.receive(),
            json!({"error":"JNI question expects answer or unhandled"})
        );
        client.send(json!({"cmd":"unhandled"}));
        assert!(
            client.receive()["error"]
                .as_str()
                .unwrap()
                .starts_with("unhandled JNI call:")
        );
        client.send(json!({"cmd":"answer","value":0}));
        assert_eq!(
            client.receive(),
            json!({"error":"breakpoint stop expects continue, step, finish, return or abort"})
        );
        client.send(json!({"cmd":"continue"}));
        assert_eq!(client.receive(), json!({"ok":"8"}));
        client.finish();
    }
}

#[test]
fn eof_at_a_stop_fails_without_resuming_the_guest() {
    for dir in ["lib64", "lib"] {
        let mut client = Client::start();
        client.request(json!({"cmd":"load","path":format!("rootfs/{dir}/libc.so")}));
        client.request(json!({"cmd":"break","at":"getpid"}));
        let address = client.request(json!({"cmd":"resolve","at":"getpid"}));
        client.send(json!({"cmd":"call_function","address":address,"args":[]}));
        assert_eq!(client.receive()["break"]["reason"], "breakpoint");
        drop(client.input);
        let output = client.child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            "error: stdin closed while a breakpoint stop was waiting for a resume command\n"
        );
    }
}
