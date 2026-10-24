#[path = "support/apk.rs"]
mod support;
use support::{
    TestApk,
    manifest::{self, package},
    signatures::{der, insert_block, pkcs7, rotation, signers, signers_with_attributes},
};
use unidroid::{Emulator, Result};
use zip::CompressionMethod::Stored;

#[test]
fn rotated_v3_keeps_current_signers_history_and_legacy_signatures() -> Result<()> {
    let oldest = der(0x30, b"oldest");
    let previous = der(0x30, b"previous");
    let current = der(0x30, b"current");
    let v1 = der(0x30, b"v1 certificate");
    let history = vec![oldest.clone(), previous, current.clone()];
    for with_v1 in [false, true] {
        let xml = manifest::xml(true, &[package()]);
        let block = pkcs7(std::slice::from_ref(&v1));
        let mut entries = vec![("AndroidManifest.xml", xml.as_slice(), Stored, 1)];
        if with_v1 {
            entries.push(("META-INF/CERT.RSA", &block, Stored, 1));
        }
        let file = TestApk::new(&entries)?;
        insert_block(
            &file.path,
            &[(
                0xf05368c0,
                signers_with_attributes(true, &[vec![current.clone()]], &rotation(&history), true),
            )],
        )?;
        let emu = Emulator::builder().apk(&file.path).build()?;
        let apk = emu.apk().unwrap();
        assert_eq!(apk.signers, std::slice::from_ref(&current));
        assert_eq!(apk.signing_history, history);
        assert_eq!(
            apk.signatures,
            [if with_v1 { v1.clone() } else { oldest.clone() }]
        );
    }
    Ok(())
}

#[test]
fn unrotated_signers_follow_v3_v2_v1_and_unsigned_rules() -> Result<()> {
    let xml = manifest::xml(true, &[package()]);
    let certs = vec![der(0x30, b"first"), der(0x30, b"second")];
    for scheme in [0, 1, 2, 3] {
        let file = TestApk::manifest(&xml)?;
        let expected = match scheme {
            1 => {
                file.write(&[
                    ("AndroidManifest.xml", &xml, Stored, 1),
                    ("META-INF/CERT.RSA", &pkcs7(&certs), Stored, 1),
                ])?;
                certs.clone()
            }
            2 => {
                insert_block(
                    &file.path,
                    &[(
                        0x7109871a,
                        signers(false, &[vec![certs[0].clone()], vec![certs[1].clone()]]),
                    )],
                )?;
                certs.clone()
            }
            3 => {
                insert_block(
                    &file.path,
                    &[
                        (0x7109871a, signers(false, &[vec![certs[1].clone()]])),
                        (0xf05368c0, signers(true, std::slice::from_ref(&certs))),
                    ],
                )?;
                vec![certs[0].clone()]
            }
            _ => vec![],
        };
        let emu = Emulator::builder().apk(&file.path).build()?;
        let apk = emu.apk().unwrap();
        assert_eq!(apk.signers, expected);
        assert_eq!(apk.signatures, expected);
        assert!(apk.signing_history.is_empty());
    }
    Ok(())
}
