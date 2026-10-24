use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

/// A tiny ARM ELF with one constructor, relocations, code, data, and BSS.
/// Built here so loader tests need no compiler or binary fixture.
pub struct TestLibrary(pub PathBuf);
impl TestLibrary {
    pub fn new(arm64: bool, relocate_code: bool) -> Self {
        fn instruction(bytes: &mut [u8], offset: usize, value: u32) {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        let width = if arm64 { 8 } else { 4 };
        let word = |bytes: &mut [u8], offset: usize, value: u64| {
            bytes[offset..offset + width].copy_from_slice(&value.to_le_bytes()[..width]);
        };
        let mut bytes = vec![0; 0x2000];
        bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        bytes[4] = if arm64 { 2 } else { 1 };
        bytes[16..18].copy_from_slice(&3u16.to_le_bytes());
        bytes[18..20].copy_from_slice(&(if arm64 { 183u16 } else { 40 }).to_le_bytes());
        instruction(&mut bytes, 20, 1);
        let (phoff, phsize, sizes) = if arm64 { (64, 56, 52) } else { (52, 32, 40) };
        word(&mut bytes, if arm64 { 32 } else { 28 }, phoff as u64);
        for (i, value) in [phoff, phsize, 3].into_iter().enumerate() {
            bytes[sizes + i * 2..sizes + i * 2 + 2].copy_from_slice(&(value as u16).to_le_bytes());
        }
        for (i, (kind, flags, offset, filesz, memsz)) in [
            (1, 5, 0, 0x1000, 0x1000),
            (1, 6, 0x1000, 0x1000, 0x3000),
            (2, 6, 0x1000, 0x100, 0x100),
        ]
        .into_iter()
        .enumerate()
        {
            let p = phoff + i * phsize;
            instruction(&mut bytes, p, kind);
            instruction(&mut bytes, p + if arm64 { 4 } else { 24 }, flags);
            for (pos, val) in if arm64 {
                [
                    (8, offset),
                    (16, offset),
                    (24, offset),
                    (32, filesz),
                    (40, memsz),
                    (48, 4096),
                ]
            } else {
                [
                    (4, offset),
                    (8, offset),
                    (12, offset),
                    (16, filesz),
                    (20, memsz),
                    (28, 4096),
                ]
            } {
                word(&mut bytes, p + pos, val);
            }
        }
        let code: &[(usize, u32)] = if arm64 {
            &[
                (0x200, 0xd2800540),                                // mov x0,#42
                (0x204, 0xd65f03c0),                                // ret
                (0x220, 0x10000001 | ((0x1600 - 0x220) >> 2) << 5), // adr x1,data
                (0x224, 0xd2800540),
                (0x228, 0xf9000020), // str x0,[x1]
                (0x22c, 0xd65f03c0),
                (0x240, 0xd2801c48), // mov x8,#226 (mprotect)
                (0x244, 0xd4000001), // svc #0
                (0x248, 0xd65f03c0),
            ]
        } else {
            &[
                (0x200, 0xe3a0002a), // mov r0,#42
                (0x204, 0xe12fff1e), // bx lr
                (0x220, 0xe59f100c), // ldr r1,[pc,#12]
                (0x224, 0xe08f1001), // add r1,pc,r1
                (0x228, 0xe3a0002a),
                (0x22c, 0xe5810000), // str r0,[r1]
                (0x230, 0xe12fff1e),
                (0x234, 0x1600 - 0x22c),
                (0x240, 0xe3a0707d), // mov r7,#125 (mprotect)
                (0x244, 0xef000000), // svc #0
                (0x248, 0xe12fff1e),
            ]
        };
        for &(offset, value) in code {
            instruction(&mut bytes, offset, value);
        }
        let names = b"\0answer\0data\0protect\0weak\0";
        bytes[0x1100..0x1100 + names.len()].copy_from_slice(names);
        let symbol_size = if arm64 { 24 } else { 16 };
        for (i, (name, value, info, defined)) in [
            (1, 0x200, 0x12, 1),
            (8, 0x1600, 0x11, 1),
            (13, 0x240, 0x12, 1),
            (21, 0, 0x20, 0),
        ]
        .into_iter()
        .enumerate()
        {
            let p = 0x1200 + (i + 1) * symbol_size;
            instruction(&mut bytes, p, name);
            bytes[p + if arm64 { 4 } else { 12 }] = info;
            bytes[p + if arm64 { 6 } else { 14 }] = defined;
            word(&mut bytes, p + if arm64 { 8 } else { 4 }, value);
            word(&mut bytes, p + if arm64 { 16 } else { 8 }, 8);
        }
        // SysV hash: one bucket and five symbols.
        for (offset, value) in [
            (0x1300, 1),
            (0x1304, 5),
            (0x1308, 1),
            (0x1310, 2),
            (0x1314, 3),
        ] {
            instruction(&mut bytes, offset, value);
        }
        let (relative, absolute, global, jump) = if arm64 {
            (1027, 257, 1025, 1026)
        } else {
            (23, 2, 21, 22)
        };
        let info = |symbol: u64, kind: u64| (symbol << if arm64 { 32 } else { 8 }) | kind;
        let mut relocs = vec![
            (0x1500, relative, 0x220),
            (0x1608, info(1, absolute), 4),
            (0x1610, info(1, global), 0),
            (0x1618, info(1, jump), 0),
            (0x1620, info(4, global), 0),
        ];
        if relocate_code {
            relocs.push((0x300, relative, 0x1600));
        }
        let relocation_size = if arm64 { 24 } else { 8 };
        for (i, &(offset, info, addend)) in relocs.iter().enumerate() {
            let p = 0x1400 + i * relocation_size;
            word(&mut bytes, p, offset);
            word(&mut bytes, p + width, info);
            word(
                &mut bytes,
                if arm64 { p + 16 } else { offset as usize },
                addend,
            );
        }
        for (i, (tag, value)) in [
            (4, 0x1300),
            (5, 0x1100),
            (6, 0x1200),
            (10, names.len() as u64),
            (11, symbol_size as u64),
            (if arm64 { 7 } else { 17 }, 0x1400),
            (
                if arm64 { 8 } else { 18 },
                (relocs.len() * relocation_size) as u64,
            ),
            (if arm64 { 9 } else { 19 }, relocation_size as u64),
            (25, 0x1500),
            (27, width as u64),
        ]
        .into_iter()
        .enumerate()
        {
            word(&mut bytes, 0x1000 + i * width * 2, tag);
            word(&mut bytes, 0x1000 + (i * 2 + 1) * width, value);
        }
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-images");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(format!(
            "{}-{}.so",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, bytes).unwrap();
        Self(path)
    }
}
impl Drop for TestLibrary {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).unwrap();
    }
}
