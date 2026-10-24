use unidroid::{Emulator, Result};

fn u32_le(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

fn u64_le(words: &[u64]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

fn signatures() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        (
            "MD5 T table",
            u32_le(&[0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee]),
        ),
        (
            "MD5/SHA-1 initial state",
            u32_le(&[0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476]),
        ),
        (
            "SHA-1 K",
            u32_le(&[0x5a827999, 0x6ed9eba1, 0x8f1bbcdc, 0xca62c1d6]),
        ),
        (
            "SHA-256 initial state",
            u32_le(&[0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a]),
        ),
        (
            "SHA-256 K table",
            u32_le(&[0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5]),
        ),
        (
            "SHA-512 initial state",
            u64_le(&[0x6a09e667f3bcc908, 0xbb67ae8584caa73b]),
        ),
        (
            "SHA-512 K table",
            u64_le(&[0x428a2f98d728ae22, 0x7137449123ef65cd]),
        ),
        (
            "SM3 initial state",
            u32_le(&[0x7380166f, 0x4914b2b9, 0x172442d7, 0xda8a0600]),
        ),
        (
            "AES S-box",
            [
                0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7,
                0xab, 0x76,
            ]
            .to_vec(),
        ),
        (
            "AES inverse S-box",
            [
                0x52, 0x09, 0x6a, 0xd5, 0x30, 0x36, 0xa5, 0x38, 0xbf, 0x40, 0xa3, 0x9e, 0x81, 0xf3,
                0xd7, 0xfb,
            ]
            .to_vec(),
        ),
        (
            "AES T-table",
            u32_le(&[0xc66363a5, 0xf87c7c84, 0xee777799, 0xf67b7b8d]),
        ),
        (
            "SM4 S-box",
            [
                0xd6, 0x90, 0xe9, 0xfe, 0xcc, 0xe1, 0x3d, 0xb7, 0x16, 0xb6, 0x14, 0xc2, 0x28, 0xfb,
                0x2c, 0x05,
            ]
            .to_vec(),
        ),
        (
            "CRC32 table",
            u32_le(&[0x00000000, 0x77073096, 0xee0e612c, 0x990951ba]),
        ),
        (
            "Blowfish P-array",
            u32_le(&[0x243f6a88, 0x85a308d3, 0x13198a2e, 0x03707344]),
        ),
        ("ChaCha/Salsa constant", b"expand 32-byte k".to_vec()),
        (
            "Base64 alphabet",
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".to_vec(),
        ),
    ]
}

#[test]
fn finds_each_crypto_signature_in_both_architectures() -> Result<()> {
    let signatures = signatures();
    for dir in ["lib64", "lib"] {
        let mut emu = Emulator::builder().build()?;
        emu.load_library(format!("rootfs/{dir}/libc.so"))?;
        let size = signatures.len() * 0x100 + 64;
        let allocation = emu.malloc(size)?;
        emu.write_bytes(allocation, &vec![0; size])?;
        assert!(emu
            .crypto_constants()?
            .iter()
            .all(|constant| !(allocation..allocation + size as u64).contains(&constant.address)));
        for (i, (_, pattern)) in signatures.iter().enumerate() {
            emu.write_bytes(allocation + i as u64 * 0x100, pattern)?;
        }

        let hits: Vec<_> = emu
            .crypto_constants()?
            .into_iter()
            .filter(|constant| (allocation..allocation + size as u64).contains(&constant.address))
            .collect();
        assert_eq!(hits.len(), signatures.len());
        for (i, (name, _)) in signatures.iter().enumerate() {
            let hit = &hits[i];
            assert_eq!(hit.name, *name);
            assert_eq!(hit.address, allocation + i as u64 * 0x100);
            assert_eq!(hit.location, format!("0x{:x} [host malloc]", hit.address));
        }
        assert!(
            hits.windows(2)
                .all(|pair| pair[0].address < pair[1].address)
        );
    }
    Ok(())
}

#[path = "support/elf.rs"]
mod elf;

#[test]
fn reports_no_constants_in_a_library_without_signatures() -> Result<()> {
    let library = elf::TestLibrary::new(true, false);
    let mut emu = Emulator::builder().build()?;
    emu.load_library(&library.0)?;
    assert!(emu.crypto_constants()?.is_empty());
    Ok(())
}
