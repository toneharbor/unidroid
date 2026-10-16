use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

/// A tiny ELF with one constructor, four AArch64 relocations, code, data, and BSS.
/// It is built here so the loader tests do not depend on a compiler or binary fixture.
pub struct TestLibrary(pub PathBuf);
impl TestLibrary {
    pub fn new(relocate_code: bool) -> Self {
        fn word(bytes: &mut [u8], offset: usize, value: u64) {
            bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        fn instruction(bytes: &mut [u8], offset: usize, value: u32) {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        let mut bytes = vec![0; 0x2000];
        bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        bytes[16..18].copy_from_slice(&3u16.to_le_bytes());
        bytes[18..20].copy_from_slice(&183u16.to_le_bytes());
        instruction(&mut bytes, 20, 1);
        word(&mut bytes, 32, 64);
        bytes[52..54].copy_from_slice(&64u16.to_le_bytes());
        bytes[54..56].copy_from_slice(&56u16.to_le_bytes());
        bytes[56..58].copy_from_slice(&3u16.to_le_bytes());
        for (i, (kind, flags, offset, filesz, memsz)) in [
            (1, 5, 0, 0x1000, 0x1000),
            (1, 6, 0x1000, 0x1000, 0x3000),
            (2, 6, 0x1000, 0x100, 0x100),
        ]
        .into_iter()
        .enumerate()
        {
            let p = 64 + i * 56;
            instruction(&mut bytes, p, kind);
            instruction(&mut bytes, p + 4, flags);
            for (pos, val) in [
                (8, offset),
                (16, offset),
                (24, offset),
                (32, filesz),
                (40, memsz),
                (48, 4096),
            ] {
                word(&mut bytes, p + pos, val);
            }
        }
        instruction(&mut bytes, 0x200, 0xd2800540); // mov x0,#42
        instruction(&mut bytes, 0x204, 0xd65f03c0); // ret
        instruction(&mut bytes, 0x220, 0x10000001 | ((0x1600 - 0x220) >> 2) << 5); // adr x1,data
        instruction(&mut bytes, 0x224, 0xd2800540);
        instruction(&mut bytes, 0x228, 0xf9000020); // str x0,[x1]
        instruction(&mut bytes, 0x22c, 0xd65f03c0);
        instruction(&mut bytes, 0x240, 0xd2801c48); // mov x8,#226 (mprotect)
        instruction(&mut bytes, 0x244, 0xd4000001); // svc #0
        instruction(&mut bytes, 0x248, 0xd65f03c0);
        let names = b"\0answer\0data\0protect\0weak\0";
        bytes[0x1100..0x1100 + names.len()].copy_from_slice(names);
        for (i, (name, value, info)) in [(1, 0x200, 0x12), (8, 0x1600, 0x11), (13, 0x240, 0x12)]
            .into_iter()
            .enumerate()
        {
            let p = 0x1200 + (i + 1) * 24;
            instruction(&mut bytes, p, name);
            bytes[p + 4] = info;
            bytes[p + 6] = 1;
            word(&mut bytes, p + 8, value);
            word(&mut bytes, p + 16, 8);
        }
        // SysV hash: one bucket and five symbols.
        instruction(&mut bytes, 0x1300, 1);
        instruction(&mut bytes, 0x1304, 5);
        instruction(&mut bytes, 0x1308, 1);
        instruction(&mut bytes, 0x1310, 2);
        instruction(&mut bytes, 0x1314, 3);
        let p = 0x1200 + 4 * 24;
        instruction(&mut bytes, p, names.len() as u32 - 5);
        bytes[p + 4] = 0x20;
        word(&mut bytes, p + 16, 8);
        let mut relocs = vec![
            (0x1500, 1027, 0x220),
            (0x1608, (1u64 << 32) | 257, 4),
            (0x1610, (1u64 << 32) | 1025, 0),
            (0x1618, (1u64 << 32) | 1026, 0),
            (0x1620, (4u64 << 32) | 1025, 0),
        ];
        if relocate_code {
            relocs.push((0x300, 1027, 0x1600));
        }
        for (i, (offset, info, addend)) in relocs.iter().enumerate() {
            let p = 0x1400 + i * 24;
            word(&mut bytes, p, *offset);
            word(&mut bytes, p + 8, *info);
            word(&mut bytes, p + 16, *addend);
        }
        for (i, (tag, value)) in [
            (4, 0x1300),
            (5, 0x1100),
            (6, 0x1200),
            (10, names.len() as u64),
            (11, 24),
            (7, 0x1400),
            (8, relocs.len() as u64 * 24),
            (9, 24),
            (25, 0x1500),
            (27, 8),
        ]
        .into_iter()
        .enumerate()
        {
            word(&mut bytes, 0x1000 + i * 16, tag);
            word(&mut bytes, 0x1008 + i * 16, value);
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
