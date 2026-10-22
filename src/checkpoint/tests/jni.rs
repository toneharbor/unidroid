use crate::{Architecture, Emulator, Handle, Result};
use std::{cell::Cell, rc::Rc};

fn jni(emu: &mut Emulator, index: u64, args: &[u64]) -> Result<u64> {
    let slot = emu
        .host
        .iter()
        .position(|host| matches!(host, crate::HostFunction::Jni(n) if *n as u64 == index))
        .unwrap();
    let function = crate::TRAMPOLINE + slot as u64 * 8;
    let mut arguments = vec![0];
    arguments.extend_from_slice(args);
    emu.call_function(function, &arguments)
}

#[test]
fn checkpoints_copy_deep_object_array_chains_without_host_recursion() -> Result<()> {
    const CHILD: &str = "UNIDROID_DEEP_CHECKPOINT_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe()?)
            .args(["--exact", "checkpoint::tests::jni::checkpoints_copy_deep_object_array_chains_without_host_recursion", "--nocapture"])
            .env(CHILD, "1")
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return Ok(());
    }
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(arch)?;
        let mut root = emu.new_string("leaf");
        for _ in 0..20_000 {
            let parent = emu.new_object_array("java/lang/Object", &[root])?;
            emu.release(root);
            root = parent;
        }
        let saved = emu.checkpoint()?;
        let mut copy = Emulator::builder().build()?;
        copy.rewind(&saved)?;
        for current in [&mut emu, &mut copy] {
            let mut item = root;
            for _ in 0..20_000 {
                item = current.object_array(item)?[0];
            }
            assert_eq!(current.string(item)?, "leaf");
        }
    }
    Ok(())
}

#[test]
fn rewind_deep_copies_jni_graphs_and_shares_only_user_payloads() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(arch)?;
        let bytes = emu.new_byte_array(&[1, 2, 3]);
        let alias = Handle(jni(&mut emu, 21, &[bytes.0])?); // NewGlobalRef
        let array = emu.new_object_array("java/lang/Object", &[bytes, alias, Handle(0)])?;
        // A cyclic object graph also has to preserve identity.
        jni(&mut emu, 174, &[array.0, 2, array.0])?;
        let payload = Rc::new(Cell::new(7));
        let user = emu.new_object("example/User", payload.clone());
        let class = emu.class("example/User");
        let patch = emu.malloc(3)?;
        emu.write_bytes(patch, &[9, 8, 7])?;
        let checkpoint = emu.checkpoint()?;
        let string = emu.new_string("later");
        jni(&mut emu, 208, &[alias.0, 0, 3, patch])?; // SetByteArrayRegion
        assert_eq!(emu.byte_array(bytes)?, [9, 8, 7]);
        emu.release(class);
        payload.set(42);
        for _ in 0..2 {
            emu.rewind(&checkpoint)?;
            assert!(emu.string(string).is_err());
            assert_eq!(emu.new_string("again"), string);
            assert_eq!(emu.class("example/User"), class);
            assert_eq!(emu.byte_array(alias)?, [1, 2, 3]);
            let items = emu.object_array(array)?;
            assert_eq!(jni(&mut emu, 24, &[items[2].0, array.0])?, 1);
            jni(&mut emu, 208, &[items[0].0, 0, 3, patch])?;
            assert_eq!(emu.byte_array(items[1])?, [9, 8, 7]);
            assert_eq!(emu.byte_array(bytes)?, [9, 8, 7]);
            assert_eq!(emu.user_value::<Rc<Cell<i32>>>(user).unwrap().get(), 42);
        }
        let mut copy = Emulator::builder().build()?;
        copy.rewind(&checkpoint)?;
        assert_eq!(copy.byte_array(bytes)?, [1, 2, 3]);
        jni(&mut copy, 208, &[bytes.0, 0, 3, patch])?;
        emu.rewind(&checkpoint)?;
        assert_eq!(emu.byte_array(bytes)?, [1, 2, 3]);
        assert_eq!(copy.byte_array(alias)?, [9, 8, 7]);
        // Break the cycles in each live state before dropping them.
        jni(&mut emu, 174, &[array.0, 2, 0])?;
        jni(&mut copy, 174, &[array.0, 2, 0])?;
    }
    Ok(())
}
