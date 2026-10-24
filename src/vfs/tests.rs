use super::*;

#[test]
fn checkpoints_share_unchanged_file_bytes_and_reopen_host_files_lazily() -> Result<()> {
    for (arch, dir) in [
        (crate::Architecture::Arm64, "lib64"),
        (crate::Architecture::Arm32, "lib"),
    ] {
        let mut emu = Emulator::builder().build()?;
        emu.add_file("/value", b"unchanged")?;
        emu.add_host_file("/linked", format!("rootfs/{dir}/libdl.so"))?;
        emu.setup_cpu(arch)?;
        let fd = emu.open_file("/value", 2, 0)?;
        let host = emu.open_file("/linked", 0, 0)?;
        let root = emu.open_file(&format!("/{dir}/libdl.so"), 0, 0)?;
        let saved = emu.checkpoint()?;
        let mut copy = Emulator::builder().build()?;
        copy.rewind(&saved)?;
        let data = |vfs: &Vfs| {
            let node = vfs.overlay["/value"].borrow();
            let Data::Bytes(bytes) = &node.data else {
                panic!("regular file")
            };
            bytes.clone()
        };
        let original = data(&emu.vfs);
        assert!(Rc::ptr_eq(&original, &data(&copy.vfs)));
        for current in [&emu.vfs, &copy.vfs] {
            for fd in [host, root] {
                let file = current.descriptor(fd)?.file.borrow();
                assert!(matches!(file.node.borrow().data, Data::Host(_)));
                assert!(file.host.is_some());
            }
        }
        copy.rename("/value", "/renamed")?;
        copy.rename("/renamed", "/value")?;
        assert!(Rc::ptr_eq(&original, &data(&copy.vfs)));
        copy.write_file_bytes(fd, b"X", None)?;
        assert!(!Rc::ptr_eq(&original, &data(&copy.vfs)));
        assert_eq!(copy.file("/value")?, b"Xnchanged");
        emu.write_file_bytes(fd, b"Y", None)?;
        assert!(!Rc::ptr_eq(&original, &data(&emu.vfs)));
        emu.rewind(&saved)?;
        assert!(Rc::ptr_eq(&original, &data(&emu.vfs)));
        assert_eq!(emu.file("/value")?, b"unchanged");
    }
    Ok(())
}
