use super::*;

fn keys(emu: &mut Emulator, map: Handle) -> Result<Vec<String>> {
    let view = call(emu, map, "keySet()Ljava/util/Set;", &[])?.object();
    let array = call(emu, view, "toArray()[Ljava/lang/Object;", &[])?.object();
    emu.object_array(array)?
        .into_iter()
        .map(|h| emu.string(h))
        .collect()
}
fn put(emu: &mut Emulator, map: Handle, key: &str) -> Result<()> {
    let key = emu.new_string(key);
    call(
        emu,
        map,
        "put(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
        &[key.into(), key.into()],
    )?;
    Ok(())
}

#[test]
fn hash_map_bucket_order_matches_java_21_before_and_after_resizes() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let map = construct(&mut emu, "java/util/HashMap", "()V", &[])?;
        // Literal orders captured from Java 21 HashMap.keySet(), not computed by this test.
        for (count, expected) in [
            (
                12,
                "key04 key03 key02 key01 key08 key07 key06 key05 key09 key00 key11 key10",
            ),
            (
                13,
                "key00 key11 key10 key04 key03 key02 key01 key12 key08 key07 key06 key05 key09",
            ),
            (
                24,
                "key00 key11 key22 key10 key21 key20 key04 key15 key03 key14 key02 key13 key01 key12 key23 key08 key19 key07 key18 key06 key17 key05 key16 key09",
            ),
            (
                25,
                "key00 key22 key21 key20 key15 key14 key13 key12 key19 key18 key17 key16 key11 key10 key04 key03 key02 key24 key01 key23 key08 key07 key06 key05 key09",
            ),
            (
                26,
                "key00 key22 key21 key20 key15 key14 key13 key12 key19 key18 key17 key16 key11 key10 key04 key03 key25 key02 key24 key01 key23 key08 key07 key06 key05 key09",
            ),
        ] {
            let start = call(&mut emu, map, "size()I", &[])?.int();
            for i in start..count {
                put(&mut emu, map, &format!("key{i:02}"))?;
            }
            assert_eq!(
                keys(&mut emu, map)?,
                expected.split(' ').collect::<Vec<_>>()
            );
        }
        let map = construct(&mut emu, "java/util/HashMap", "()V", &[])?;
        for c in ('a'..='z').rev() {
            put(&mut emu, map, &c.to_string())?;
        }
        assert_eq!(
            keys(&mut emu, map)?,
            [
                "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p",
                "q", "r", "s", "t", "u", "v", "w", "x", "y", "z"
            ]
        );
        let map = construct(&mut emu, "java/util/HashMap", "(I)V", &[16.into()])?;
        for k in ["q", "a", "Q", "A"] {
            put(&mut emu, map, k)?;
        }
        assert_eq!(keys(&mut emu, map)?, ["q", "a", "Q", "A"]);
        let key = emu.new_string("a");
        call(
            &mut emu,
            map,
            "remove(Ljava/lang/Object;)Ljava/lang/Object;",
            &[key.into()],
        )?;
        put(&mut emu, map, "a")?;
        assert_eq!(keys(&mut emu, map)?, ["q", "Q", "A", "a"]);
        let map = construct(&mut emu, "java/util/HashMap", "()V", &[])?;
        for k in ["Aa", "BB", "AaAa", "BBBB", "AaBB", "BBAa"] {
            put(&mut emu, map, k)?;
        }
        assert_eq!(
            keys(&mut emu, map)?,
            ["Aa", "BB", "AaAa", "BBBB", "AaBB", "BBAa"]
        );
        let key = emu.new_string("AaAa");
        call(
            &mut emu,
            map,
            "remove(Ljava/lang/Object;)Ljava/lang/Object;",
            &[key.into()],
        )?;
        put(&mut emu, map, "AaAa")?;
        assert_eq!(
            keys(&mut emu, map)?,
            ["Aa", "BB", "BBBB", "AaBB", "BBAa", "AaAa"]
        );
    }
    Ok(())
}

#[test]
fn linked_hash_map_tree_map_and_hash_set_have_java_order() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        for (class, expected) in [
            ("java/util/LinkedHashMap", ["z", "a", "m"]),
            ("java/util/TreeMap", ["a", "m", "z"]),
        ] {
            let map = construct(&mut emu, class, "()V", &[])?;
            for key in ["z", "a", "m", "a"] {
                put(&mut emu, map, key)?;
            }
            assert_eq!(keys(&mut emu, map)?, expected);
            let entries = call(&mut emu, map, "entrySet()Ljava/util/Set;", &[])?.object();
            let array = call(&mut emu, entries, "toArray()[Ljava/lang/Object;", &[])?.object();
            for (entry, expected) in emu.object_array(array)?.into_iter().zip(expected) {
                let key = call(&mut emu, entry, "getKey()Ljava/lang/Object;", &[])?.object();
                let value = call(&mut emu, entry, "getValue()Ljava/lang/Object;", &[])?.object();
                assert_eq!(emu.string(key)?, expected);
                assert_eq!(emu.string(value)?, expected);
            }
        }
        let set = construct(&mut emu, "java/util/HashSet", "()V", &[])?;
        for key in ["q", "a", "Q", "A"] {
            let key = emu.new_string(key);
            assert_eq!(
                call(&mut emu, set, "add(Ljava/lang/Object;)Z", &[key.into()])?,
                JniValue::Boolean(true)
            );
            assert_eq!(
                call(&mut emu, set, "add(Ljava/lang/Object;)Z", &[key.into()])?,
                JniValue::Boolean(false)
            );
            assert_eq!(
                call(
                    &mut emu,
                    set,
                    "contains(Ljava/lang/Object;)Z",
                    &[key.into()]
                )?,
                JniValue::Boolean(true)
            );
        }
        let array = call(&mut emu, set, "toArray()[Ljava/lang/Object;", &[])?.object();
        assert_eq!(
            emu.object_array(array)?
                .into_iter()
                .map(|h| emu.string(h))
                .collect::<Result<Vec<_>>>()?,
            ["q", "a", "Q", "A"]
        );
        let tree = emu.new_java_map("java/util/TreeMap", &[])?;
        for n in [10, -5, 2] {
            let n = emu.new_boxed(JniValue::Long(n))?;
            call(
                &mut emu,
                tree,
                "put(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
                &[n.into(), Handle(0).into()],
            )?;
        }
        let entries = emu.java_map(tree)?;
        assert_eq!(
            entries
                .into_iter()
                .map(|(k, _)| emu.unboxed(k))
                .collect::<Result<Vec<_>>>()?,
            [JniValue::Long(-5), JniValue::Long(2), JniValue::Long(10)]
        );
    }
    Ok(())
}
