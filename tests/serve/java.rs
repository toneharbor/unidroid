use super::*;

#[test]
fn java_collection_and_box_commands_round_trip() {
    for library in ["rootfs/lib/libc.so", "rootfs/lib64/libc.so"] {
        let mut client = Client::start();
        client.request(json!({"cmd":"load","path":library}));
        let key = client.request(json!({"cmd":"new_string","value":"k"}));
        for (kind, value) in [
            ("Z", json!(true)),
            ("B", json!(-128)),
            ("C", json!(55296)),
            ("S", json!(-32768)),
            ("I", json!(-42)),
            ("J", json!("-9007199254740993")),
            ("F", json!(1.5)),
            ("D", json!(-2.25)),
        ] {
            let boxed = client.request(json!({"cmd":"new_boxed","type":kind,"value":value}));
            assert_eq!(
                client.request(json!({"cmd":"unboxed","handle":boxed})),
                json!({"type":kind,"value":value})
            );
            for class in [
                "java/util/HashMap",
                "java/util/LinkedHashMap",
                "java/util/TreeMap",
            ] {
                let map = client
                    .request(json!({"cmd":"new_java_map","class":class,"entries":[[key,boxed]]}));
                let entries = client.request(json!({"cmd":"java_map","handle":map}));
                assert_eq!(
                    client.request(json!({"cmd":"string","handle":entries[0][0]})),
                    "k"
                );
                assert_eq!(
                    client.request(json!({"cmd":"unboxed","handle":entries[0][1]})),
                    json!({"type":kind,"value":value})
                );
                for list_class in ["java/util/ArrayList", "java/util/LinkedList"] {
                    let list = client.request(
                        json!({"cmd":"new_java_list","class":list_class,"items":[boxed,0]}),
                    );
                    let items = client.request(json!({"cmd":"java_list","handle":list}));
                    assert_eq!(items[1], 0);
                    assert_eq!(
                        client.request(json!({"cmd":"unboxed","handle":items[0]})),
                        json!({"type":kind,"value":value})
                    );
                }
            }
        }
        for message in [
            json!({"cmd":"new_boxed","type":"L","value":0}),
            json!({"cmd":"java_map","handle":key}),
            json!({"cmd":"new_java_list","class":"bad","items":[]}),
        ] {
            client.send(message);
            assert!(client.receive().get("error").is_some());
        }
        client.finish();
    }
}
