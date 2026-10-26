use super::*;
use serde_json::json;

#[test]
fn nested_collection_policy_values_create_fresh_builtin_objects() -> Result<()> {
    for lib in ["rootfs/lib/libc.so", "rootfs/lib64/libc.so"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(lib)?;
        let mut policy = Policy::parse(&json!({"rules":[{"value":{
            "map":"java/util/LinkedHashMap", "entries":[["k",{"list":"java/util/LinkedList","items":["v",null,{"array":"B","base64":"YQ=="}]}]]
        }}]}).to_string())?;
        for _ in 0..2 {
            let mut fallback = ();
            let mut handler = policy.handler(&mut fallback);
            let answer = handler
                .member(&mut emu, &call("()Ljava/lang/Object;"))
                .map(JniValue::object);
            handler.finish(Ok(()))?;
            let entries = emu.java_map(answer.unwrap())?;
            assert_eq!(emu.string(entries[0].0)?, "k");
            let items = emu.java_list(entries[0].1)?;
            assert_eq!(emu.string(items[0])?, "v");
            assert_eq!(items[1], Handle(0));
            assert_eq!(emu.byte_array(items[2])?, b"a");
            for handle in [
                answer.unwrap(),
                entries[0].0,
                entries[0].1,
                items[0],
                items[2],
            ] {
                emu.release(handle);
            }
        }
        let mut policy =
            Policy::parse(r#"{"rules":[{"value":{"map":"java/util/Map","entries":[]}}]}"#)?;
        let mut fallback = ();
        let mut handler = policy.handler(&mut fallback);
        assert!(
            handler
                .member(&mut emu, &call("()Ljava/lang/Object;"))
                .is_none()
        );
        assert!(
            handler
                .finish(Ok(()))
                .unwrap_err()
                .to_string()
                .contains("policy rule 0")
        );
    }
    Ok(())
}
