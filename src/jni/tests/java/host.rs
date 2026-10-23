use super::*;

#[test]
fn host_collections_preserve_identity_and_reject_wrong_kinds() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let key = emu.new_string("k");
        let value = emu.new_boxed(JniValue::Int(7))?;
        let map = emu.new_java_map("java/util/HashMap", &[(key, value)])?;
        let entries = emu.java_map(map)?;
        assert_eq!(emu.string(entries[0].0)?, "k");
        assert_eq!(emu.unboxed(entries[0].1)?, JniValue::Int(7));
        let list = emu.new_java_list("java/util/ArrayList", &[map, Handle(0), map])?;
        let items = emu.java_list(list)?;
        assert_eq!(jni_call(&mut emu, 24, &[items[0].0, items[2].0])?, 1);
        assert_eq!(items[1], Handle(0));
        call(
            &mut emu,
            map,
            "put(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
            &[key.into(), list.into()],
        )?;
        let checkpoint = emu.checkpoint()?;
        call(&mut emu, map, "clear()V", &[])?;
        emu.rewind(&checkpoint)?;
        let entries = emu.java_map(map)?;
        let restored = emu.java_list(entries[0].1)?;
        assert_eq!(jni_call(&mut emu, 24, &[restored[0].0, map.0])?, 1);
        emu.release(key);
        emu.release(value);
        let entries = emu.java_map(map)?;
        assert_eq!(emu.string(entries[0].0)?, "k");
        for bad in [JniValue::Object(map), JniValue::Void] {
            assert!(
                emu.new_boxed(bad)
                    .unwrap_err()
                    .to_string()
                    .contains("primitive")
            );
        }
        assert!(emu.unboxed(map).unwrap_err().to_string().contains("boxed"));
        assert!(emu.java_map(list).unwrap_err().to_string().contains("map"));
        assert!(emu.java_list(map).unwrap_err().to_string().contains("list"));
        assert!(
            emu.new_java_map("java/util/Map", &[])
                .unwrap_err()
                .to_string()
                .contains("HashMap")
        );
        assert!(
            emu.new_java_list("java/util/List", &[])
                .unwrap_err()
                .to_string()
                .contains("ArrayList")
        );
    }
    Ok(())
}
