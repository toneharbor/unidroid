use super::{TestRoot, call, errno};
use std::os::unix::fs::symlink;
use unidroid::{Emulator, Result};

#[test]
fn mkdir_and_exclusive_create_reject_final_symlinks() -> Result<()> {
    let root = TestRoot::new("exclusive-symlinks")?;
    std::fs::write(root.0.join("file"), b"host")?;
    std::fs::create_dir(root.0.join("directory"))?;
    let links = [
        ("dangling", "missing"),
        ("link", "file"),
        ("dirlink", "directory"),
    ];
    for (name, target) in links {
        symlink(target, root.0.join(name))?;
    }
    for dir in ["lib64", "lib"] {
        for function in ["mkdirat", "openat"] {
            let mut emu = Emulator::builder().rootfs(&root.0).build()?;
            let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
            let buffer = emu.malloc(128)?;
            for overlay in [false, true] {
                for (name, target) in links {
                    let mut path = emu.new_cstring(&format!("/{name}"))?;
                    if overlay {
                        let moved = emu.new_cstring(&format!("/moved-{name}"))?;
                        assert_eq!(call(&mut emu, libc, "rename", &[path, moved])?, 0);
                        path = moved;
                    }
                    let args = if function == "mkdirat" {
                        vec![(-100i64) as u64, path, 0o700]
                    } else {
                        vec![(-100i64) as u64, path, 0xc2, 0o600]
                    };
                    let result = call(&mut emu, libc, function, &args)?;
                    assert_eq!(
                        result as u32,
                        u32::MAX,
                        "{dir}: {function} {name}, overlay={overlay}"
                    );
                    errno(&mut emu, libc, result, 17)?;
                    assert_eq!(
                        call(&mut emu, libc, "readlink", &[path, buffer, 128])?,
                        target.len() as u64
                    );
                    assert_eq!(emu.read_bytes(buffer, target.len())?, target.as_bytes());
                    let missing = emu.new_cstring("/missing")?;
                    let result = call(&mut emu, libc, "access", &[missing, 0])?;
                    errno(&mut emu, libc, result, 2)?;
                    assert_eq!(emu.file("/file")?, b"host");
                }
            }
        }
    }
    Ok(())
}

#[test]
fn link_limits_and_rootfs_boundary_apply_to_every_lookup() -> Result<()> {
    let root = TestRoot::new("symlink-boundary")?;
    let guest_root = root.0.join("root");
    std::fs::create_dir(&guest_root)?;
    std::fs::write(root.0.join("secret"), b"outside")?;
    std::fs::write(guest_root.join("file"), b"inside")?;
    symlink("../secret", guest_root.join("escape"))?;
    symlink(
        root.0.join("secret").canonicalize()?,
        guest_root.join("absolute-escape"),
    )?;
    symlink("..", guest_root.join("shadow"))?;
    symlink("loop-b", guest_root.join("loop-a"))?;
    symlink("loop-a", guest_root.join("loop-b"))?;
    for index in (0..41).rev() {
        let target = if index == 40 {
            "file".into()
        } else {
            format!("chain-{}", index + 1)
        };
        symlink(target, guest_root.join(format!("chain-{index}")))?;
    }
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().rootfs(&guest_root).build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let buffer = emu.malloc(128)?;
        for name in ["/chain-0", "/loop-a"] {
            let path = emu.new_cstring(name)?;
            for (function, args) in [
                ("open", vec![path, 0]),
                ("stat", vec![path, buffer]),
                ("access", vec![path, 0]),
            ] {
                let result = call(&mut emu, libc, function, &args)?;
                errno(&mut emu, libc, result, 40)?;
            }
            assert_eq!(
                emu.file(name).unwrap_err().to_string(),
                format!("{name}: ELOOP")
            );
        }
        let path = emu.new_cstring("/chain-1")?;
        let fd = call(&mut emu, libc, "open", &[path, 0])?;
        assert_eq!(call(&mut emu, libc, "read", &[fd, buffer, 128])?, 6);
        assert_eq!(emu.read_bytes(buffer, 6)?, b"inside");
        // A final loop link can still be read or removed without following it.
        let path = emu.new_cstring("/loop-a")?;
        assert_eq!(call(&mut emu, libc, "readlink", &[path, buffer, 128])?, 6);
        assert_eq!(call(&mut emu, libc, "unlink", &[path])?, 0);
        emu.remove_file("/shadow")?;
        emu.add_file("/shadow/overlay", b"guest")?;
        assert_eq!(emu.file("/shadow/overlay")?, b"guest");
        for name in ["/escape", "/absolute-escape", "/shadow/secret"] {
            let path = emu.new_cstring(name)?;
            let result = call(&mut emu, libc, "open", &[path, 0])?;
            errno(&mut emu, libc, result, 2)?;
        }
    }
    Ok(())
}

#[test]
fn link_operations_act_on_the_link_and_renames_keep_its_target_text() -> Result<()> {
    let root = TestRoot::new("symlink-operations")?;
    std::fs::write(root.0.join("file"), b"host")?;
    std::fs::create_dir(root.0.join("directory"))?;
    std::fs::create_dir(root.0.join("tree"))?;
    for (target, name) in [
        ("file", "link"),
        ("/file", "absolute"),
        ("missing", "dangling"),
        ("directory", "dirlink"),
        ("../file", "tree/child"),
        ("missing", "unlink-dangling"),
        ("directory", "unlink-dirlink"),
        ("directory", "replace-link"),
    ] {
        symlink(target, root.0.join(name))?;
    }
    for (dir, nr) in [("lib64", 61), ("lib", 217)] {
        let mut emu = Emulator::builder()
            .rootfs(&root.0)
            .fixed_clock(123)
            .build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let buffer = emu.malloc(1024)?;
        let cwd = (-100i64) as u64;
        for name in ["/unlink-dangling", "/unlink-dirlink"] {
            let path = emu.new_cstring(name)?;
            assert_eq!(call(&mut emu, libc, "unlinkat", &[cwd, path, 0])?, 0);
        }
        emu.add_file("/replacement", b"replacement")?;
        let replacement = emu.new_cstring("/replacement")?;
        let link = emu.new_cstring("/replace-link")?;
        assert_eq!(
            call(&mut emu, libc, "renameat", &[cwd, replacement, cwd, link])?,
            0
        );
        assert_eq!(emu.file("/replace-link")?, b"replacement");
        for (name, target) in [
            ("link", "file"),
            ("absolute", "/file"),
            ("dangling", "missing"),
            ("dirlink", "directory"),
        ] {
            let path = emu.new_cstring(&format!("/{name}"))?;
            assert_eq!(
                call(&mut emu, libc, "fstatat", &[cwd, path, buffer, 0x100])?,
                0
            );
            assert_eq!(emu.read_bytes(buffer + 16, 4)?, 0o120777u32.to_le_bytes());
            assert_eq!(
                emu.read_bytes(buffer + 48, 8)?,
                (target.len() as u64).to_le_bytes()
            );
            assert_eq!(emu.read_bytes(buffer + 72, 4)?, 123u32.to_le_bytes());
            let moved = emu.new_cstring(&format!("/moved-{name}"))?;
            if dir == "lib64" && name == "absolute" {
                assert_eq!(
                    call(&mut emu, libc, "syscall", &[276, cwd, path, cwd, moved, 0])?,
                    0
                );
            } else {
                assert_eq!(call(&mut emu, libc, "rename", &[path, moved])?, 0);
            }
            let result = call(&mut emu, libc, "readlink", &[path, buffer, 128])?;
            errno(&mut emu, libc, result, 2)?;
            assert_eq!(
                call(&mut emu, libc, "readlink", &[moved, buffer, 128])?,
                target.len() as u64
            );
            assert_eq!(emu.read_bytes(buffer, target.len())?, target.as_bytes());
            assert_eq!(
                call(&mut emu, libc, "fstatat", &[cwd, moved, buffer, 0x100])?,
                0
            );
            assert_eq!(emu.read_bytes(buffer + 16, 4)?, 0o120777u32.to_le_bytes());
        }
        let tree = emu.new_cstring("/tree")?;
        let moved = emu.new_cstring("/moved-tree")?;
        assert_eq!(call(&mut emu, libc, "rename", &[tree, moved])?, 0);
        emu.add_file("/file", b"overlay")?;
        assert_eq!(emu.file("/moved-absolute")?, b"overlay");
        assert_eq!(emu.file("/moved-link")?, b"overlay");
        assert_eq!(emu.file("/moved-tree/child")?, b"overlay");
        let rootpath = emu.new_cstring("/")?;
        let fd = call(&mut emu, libc, "open", &[rootpath, 0])?;
        let length = call(&mut emu, libc, "syscall", &[nr, fd, buffer, 1024])?;
        let entries = emu.read_bytes(buffer, length as usize)?;
        let mut offset = 0;
        let mut links = Vec::new();
        while offset < entries.len() {
            let length = u16::from_le_bytes(entries[offset + 16..offset + 18].try_into()?) as usize;
            if entries[offset + 18] == 10 {
                links.push(String::from_utf8(
                    entries[offset + 19..offset + length]
                        .iter()
                        .copied()
                        .take_while(|b| *b != 0)
                        .collect(),
                )?);
            }
            offset += length;
        }
        assert_eq!(
            links,
            [
                "moved-absolute",
                "moved-dangling",
                "moved-dirlink",
                "moved-link"
            ]
        );
        let saved = emu.checkpoint()?;
        for name in [
            "/moved-absolute",
            "/moved-dangling",
            "/moved-dirlink",
            "/moved-link",
        ] {
            let path = emu.new_cstring(name)?;
            assert_eq!(call(&mut emu, libc, "unlink", &[path])?, 0);
        }
        assert_eq!(emu.file("/file")?, b"overlay");
        let directory = emu.new_cstring("/directory")?;
        assert_eq!(call(&mut emu, libc, "access", &[directory, 0])?, 0);
        let mut copy = Emulator::builder().build()?;
        copy.rewind(&saved)?;
        let path = copy.new_cstring("/moved-absolute")?;
        assert_eq!(call(&mut copy, libc, "readlink", &[path, buffer, 128])?, 5);
        assert_eq!(copy.read_bytes(buffer, 5)?, b"/file");
        assert_eq!(copy.file("/moved-absolute")?, b"overlay");
        assert!(copy.file("/absolute").is_err());
    }
    Ok(())
}

#[test]
fn symlinks_resolve_targets_through_the_guest_layers() -> Result<()> {
    let root = TestRoot::new("symlink-resolution")?;
    std::fs::write(root.0.join("file"), b"host")?;
    std::fs::create_dir(root.0.join("directory"))?;
    symlink("file", root.0.join("relative"))?;
    symlink("/file", root.0.join("absolute"))?;
    symlink("../file", root.0.join("directory/up"))?;
    symlink("directory", root.0.join("dirlink"))?;
    symlink("/proc/self/cmdline", root.0.join("generated"))?;
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().rootfs(&root.0).build()?;
        let libc = emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let buffer = emu.malloc(128)?;
        emu.add_file("/file", b"new")?;
        for name in [
            "/relative",
            "/absolute",
            "/directory/up",
            "/dirlink/up",
            "/generated",
        ] {
            let path = emu.new_cstring(name)?;
            let fd = call(&mut emu, libc, "open", &[path, 0])?;
            assert_ne!(fd as u32, u32::MAX, "{dir}: {name}");
            let expected = if name == "/generated" {
                b"unidroid\0".as_slice()
            } else {
                b"new"
            };
            assert_eq!(
                call(&mut emu, libc, "read", &[fd, buffer, 128])?,
                expected.len() as u64
            );
            assert_eq!(emu.read_bytes(buffer, expected.len())?, expected);
        }
        let path = emu.new_cstring("/dirlink/up")?;
        assert_eq!(
            call(
                &mut emu,
                libc,
                "readlinkat",
                &[(-100i64) as u64, path, buffer, 128]
            )?,
            7
        );
        assert_eq!(emu.read_bytes(buffer, 7)?, b"../file");
        emu.remove_file("/file")?;
        let path = emu.new_cstring("/relative")?;
        let result = call(&mut emu, libc, "access", &[path, 0])?;
        errno(&mut emu, libc, result, 2)?;
        // Creating through a dangling link writes its guest target.
        let fd = call(&mut emu, libc, "open", &[path, 0x42, 0o600])?;
        assert_ne!(fd as u32, u32::MAX);
        assert_eq!(emu.file("/file")?, b"");
        assert_eq!(std::fs::read(root.0.join("file"))?, b"host");
        let path = emu.new_cstring("/dirlink/created")?;
        assert_eq!(call(&mut emu, libc, "mkdir", &[path, 0o700])?, 0);
        let actual = emu.new_cstring("/directory/created")?;
        assert_eq!(call(&mut emu, libc, "access", &[actual, 0])?, 0);
    }
    Ok(())
}
