use super::*;
use crate::Policy;
use serde_json::json;

#[test]
fn failed_policy_collections_release_all_created_handles() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let kept = emu.new_string("keep me");
        let nested = json!({"list":"java/util/ArrayList","items":["nested"]});
        let bad = json!({"array":"B","base64":"!"});
        for value in [
            json!({"list":"java/util/ArrayList","items":[nested, bad]}),
            json!({"map":"java/util/HashMap","entries":[["first",nested],["second",bad]]}),
            json!({"map":"java/util/HashMap","entries":[[bad,"unused"]]}),
            json!({"list":"bad/List","items":[nested]}),
            json!({"map":"bad/Map","entries":[["key",nested]]}),
            json!({"map":"java/util/TreeMap","entries":[["key",nested],[{"object":"test/Key"},null]]}),
        ] {
            let before = emu.jni.objects.len();
            let mut policy = Policy::parse(&json!({"rules":[{"value":value}]}).to_string())?;
            let mut fallback = ();
            let mut handler = policy.handler(&mut fallback);
            assert!(
                handler
                    .member(
                        &mut emu,
                        &JniCall {
                            op: JniOp::Call,
                            is_static: false,
                            type_code: b'L',
                            class: "test/Policy",
                            name: "value",
                            signature: "()Ljava/lang/Object;",
                            receiver: Handle(0),
                            args: &[],
                        }
                    )
                    .is_none()
            );
            assert!(
                handler
                    .finish(Ok(()))
                    .unwrap_err()
                    .to_string()
                    .contains("policy rule 0")
            );
            assert_eq!(emu.jni.objects.len(), before, "leaked handles for {value}");
            assert_eq!(emu.string(kept)?, "keep me");
        }
    }
    Ok(())
}
