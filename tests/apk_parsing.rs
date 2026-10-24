#[path = "support/apk.rs"]
mod support;
use support::{
    TestApk,
    manifest::{self, Value, android, package},
    signatures::{der, insert_block, signers_with_attributes},
};
use unidroid::{Emulator, Result};
use zip::CompressionMethod::Stored;

#[test]
fn signing_schemes_ignore_trailing_bytes_in_nested_slices() -> Result<()> {
    for (v3, id) in [(false, 0x7109871a), (true, 0xf05368c0)] {
        let certificate = der(0x30, b"leaf");
        let file = TestApk::manifest(&manifest::xml(true, &[package()]))?;
        let block = signers_with_attributes(v3, &[vec![certificate.clone()]], &[], true);
        insert_block(&file.path, &[(id, block)])?;
        let emu = Emulator::builder().apk(&file.path).build()?;
        assert_eq!(emu.apk().unwrap().signatures, [certificate]);
    }
    Ok(())
}

fn constructed(tag: u8, value: &[u8], indefinite: bool) -> Vec<u8> {
    if !indefinite {
        return der(tag, value);
    }
    let mut bytes = vec![tag, 0x80];
    bytes.extend(value);
    bytes.extend([0, 0]);
    bytes
}

#[test]
fn v1_accepts_ber_containers_and_ignores_data_after_certificates() -> Result<()> {
    for indefinite in [false, true] {
        let certificates = [der(0x30, b"first\0\0exact"), der(0x30, b"second")];
        let mut signed = der(2, &[1]);
        signed.extend(constructed(
            0x31,
            &constructed(0x30, &der(6, &[42]), indefinite),
            indefinite,
        ));
        signed.extend(constructed(0x30, &der(6, &[42]), indefinite));
        signed.extend(der(0xa0, &certificates.concat()));
        signed.extend(b"ignored, not even ASN.1");
        let mut content = der(6, &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 1, 7, 2]);
        content.extend(constructed(
            0xa0,
            &constructed(0x30, &signed, indefinite),
            indefinite,
        ));
        let mut block = constructed(0x30, &content, indefinite);
        block.extend(b"trailing ContentInfo bytes");
        let xml = manifest::xml(true, &[package()]);
        let file = TestApk::new(&[
            ("AndroidManifest.xml", &xml, Stored, 1),
            ("META-INF/CERT.RSA", &block, Stored, 1),
        ])?;
        let emu = Emulator::builder().apk(&file.path).build()?;
        assert_eq!(emu.apk().unwrap().signatures, certificates);
    }
    Ok(())
}

#[test]
fn manifest_ignores_unused_pool_strings_and_bytes_after_root_attributes() -> Result<()> {
    for utf8 in [true, false] {
        let mut xml = manifest::xml(
            utf8,
            &[
                package(),
                android("unused", 0, Value::String("unused value")),
            ],
        );
        // Index 5 is the unused attribute's string value. Its offset is invalid.
        xml[8 + 28 + 5 * 4..8 + 28 + 6 * 4].copy_from_slice(&u32::MAX.to_le_bytes());
        // No trailing element needs to be parsed after the manifest attributes.
        let end = xml.len() - 24;
        xml[end..].fill(0xff);
        xml.extend(b"bytes beyond the document size");
        let file = TestApk::manifest(&xml)?;
        let emu = Emulator::builder().apk(&file.path).build()?;
        assert_eq!(emu.apk().unwrap().package, "com.example.apk");
        assert_eq!(emu.apk().unwrap().version_code, 0);
    }
    Ok(())
}

#[test]
fn manifest_decodes_used_strings_lossily_without_count_or_terminator_checks() -> Result<()> {
    for utf8 in [true, false] {
        let mut xml = manifest::xml(
            utf8,
            &[package(), android("versionName", 0, Value::String("xy"))],
        );
        let start = u32::from_le_bytes(xml[28..32].try_into()?) as usize + 8;
        let offset = u32::from_le_bytes(xml[8 + 28 + 5 * 4..8 + 28 + 6 * 4].try_into()?) as usize;
        let value = start + offset;
        if utf8 {
            xml[value] = 17; // Incorrect UTF-16 unit count is ignored.
            xml[value + 2] = 0xff;
            xml[value + 4] = 0xff; // Terminator is not checked.
        } else {
            xml[value + 2..value + 4].copy_from_slice(&0xd800u16.to_le_bytes());
            xml[value + 6..value + 8].fill(0xff);
        }
        let file = TestApk::manifest(&xml)?;
        let emu = Emulator::builder().apk(&file.path).build()?;
        assert_eq!(emu.apk().unwrap().version_name.as_deref(), Some("�y"));
    }
    Ok(())
}
