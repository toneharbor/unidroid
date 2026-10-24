#[path = "support/apk.rs"]
mod support;
use support::{
    TestApk,
    manifest::{self, Value, android, package},
};
use unidroid::{Emulator, Result};

#[test]
fn signing_blocks_prefer_v1_then_v3_then_v2_and_keep_leaf_bytes() -> Result<()> {
    use support::signatures::{der, insert_block, pkcs7, signers};
    use zip::CompressionMethod::Stored;
    let xml = manifest::xml(true, &[package()]);
    let leaf = der(0x30, b"leaf v2");
    let other = der(0x30, b"other v2");
    let v3_leaf = der(0x30, b"leaf v3");
    let chain = der(0x30, b"chain");
    let v2 = (
        0x7109871a,
        signers(
            false,
            &[vec![leaf.clone(), chain.clone()], vec![other.clone()]],
        ),
    );
    let v3 = (
        0xf05368c0,
        signers(true, &[vec![v3_leaf.clone(), chain.clone()]]),
    );
    for (pairs, expected) in [
        (vec![v2.clone()], vec![leaf, other]),
        (vec![v3.clone()], vec![v3_leaf.clone()]),
        (vec![v2.clone(), v3.clone()], vec![v3_leaf]),
    ] {
        let file = TestApk::manifest(&xml)?;
        insert_block(&file.path, &pairs)?;
        let emu = Emulator::builder().apk(&file.path).build()?;
        assert_eq!(emu.apk().unwrap().signatures, expected);
    }
    let v1 = pkcs7(std::slice::from_ref(&chain));
    let file = TestApk::new(&[
        ("AndroidManifest.xml", &xml, Stored, 1),
        ("META-INF/CERT.DSA", &v1, Stored, 1),
    ])?;
    insert_block(&file.path, &[v2, v3])?;
    assert_eq!(
        Emulator::builder()
            .apk(&file.path)
            .build()?
            .apk()
            .unwrap()
            .signatures,
        [chain]
    );
    Ok(())
}

#[test]
fn malformed_signature_blocks_name_the_apk() -> Result<()> {
    use support::signatures::insert_block;
    use zip::CompressionMethod::Stored;
    let xml = manifest::xml(true, &[package()]);
    for v1 in [true, false] {
        let file = TestApk::manifest(&xml)?;
        if v1 {
            file.write(&[
                ("AndroidManifest.xml", &xml, Stored, 1),
                ("META-INF/CERT.RSA", b"bad DER", Stored, 1),
            ])?;
        } else {
            insert_block(&file.path, &[(0x7109871a, vec![255; 8])])?;
        }
        let error = Emulator::builder()
            .apk(&file.path)
            .build()
            .err()
            .expect("malformed signature rejected")
            .to_string();
        assert!(
            error.contains(file.path.to_str().unwrap()) && error.contains("signature block"),
            "{error}"
        );
    }
    Ok(())
}

#[test]
fn manifest_metadata_uses_both_string_pools_and_long_version_code() -> Result<()> {
    for utf8 in [true, false] {
        let xml = manifest::xml(
            utf8,
            &[
                package(),
                android("renamedCode", 0x0101021b, Value::Int(0xffff_fffe)),
                android("versionCodeMajor", 0, Value::Hex(2)),
                android("versionName", 0x0101021c, Value::String("版本 😀 2.0")),
            ],
        );
        let file = TestApk::manifest(&xml)?;
        let emu = Emulator::builder().apk(&file.path).build()?;
        let apk = emu.apk().unwrap();
        assert_eq!(apk.path, file.path.canonicalize()?);
        assert_eq!(apk.package, "com.example.apk");
        assert_eq!(apk.version_name.as_deref(), Some("版本 😀 2.0"));
        assert_eq!(apk.version_code, 12_884_901_886);
        assert!(apk.signatures.is_empty());
    }
    Ok(())
}

#[test]
fn manifest_defaults_references_and_attribute_identity() -> Result<()> {
    use manifest::Attribute;
    for utf8 in [true, false] {
        for attrs in [
            vec![package()],
            vec![
                package(),
                android("versionName", 0, Value::Reference(0x7f010001)),
                android("versionCode", 0x01010000, Value::Int(99)),
                Attribute {
                    namespace: Some("other"),
                    name: "versionCodeMajor",
                    id: 0,
                    value: Value::Int(1),
                },
            ],
        ] {
            let file = TestApk::manifest(&manifest::xml(utf8, &attrs))?;
            let emu = Emulator::builder().apk(&file.path).build()?;
            assert_eq!(emu.apk().unwrap().version_name, None);
            assert_eq!(emu.apk().unwrap().version_code, 0);
        }
    }
    Ok(())
}

#[test]
fn invalid_apks_name_the_path_and_problem() -> Result<()> {
    let file = TestApk::manifest(&manifest::xml(true, &[]))?;
    for (bytes, problem) in [
        (manifest::xml(true, &[]), "no package"),
        (b"not binary XML".to_vec(), "binary XML"),
    ] {
        file.write(&[(
            "AndroidManifest.xml",
            &bytes,
            zip::CompressionMethod::Stored,
            1,
        )])?;
        let error = Emulator::builder()
            .apk(&file.path)
            .build()
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains(file.path.to_str().unwrap()), "{error}");
        assert!(error.contains(problem), "{error}");
    }
    file.write(&[])?;
    let error = Emulator::builder()
        .apk(&file.path)
        .build()
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains(file.path.to_str().unwrap()) && error.contains("AndroidManifest.xml"),
        "{error}"
    );
    std::fs::write(&file.path, b"not a zip")?;
    let error = Emulator::builder()
        .apk(&file.path)
        .build()
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains(file.path.to_str().unwrap()) && error.contains("archive"),
        "{error}"
    );
    Ok(())
}

#[test]
fn v1_certificates_keep_bytes_and_central_directory_order() -> Result<()> {
    use support::signatures::{der, pkcs7};
    use zip::CompressionMethod::Stored;
    let xml = manifest::xml(true, &[package()]);
    let certificates = [
        der(0x30, b"first"),
        der(0x30, &[0xa5; 200]),
        der(0x30, b"third"),
    ];
    let first = pkcs7(&certificates[..2]);
    let second = pkcs7(&certificates[2..]);
    for count in [1, 2] {
        let mut entries = vec![
            ("AndroidManifest.xml", xml.as_slice(), Stored, 1),
            ("META-INF/Z.rSa", first.as_slice(), Stored, 1),
        ];
        if count == 2 {
            entries.push(("meta-inf/A.Ec", second.as_slice(), Stored, 1));
        }
        entries.push(("META-INF/nested/IGNORED.RSA", b"not a block", Stored, 1));
        let file = TestApk::new(&entries)?;
        let emu = Emulator::builder().apk(&file.path).build()?;
        assert_eq!(
            emu.apk().unwrap().signatures,
            certificates[..if count == 1 { 2 } else { 3 }]
        );
    }
    Ok(())
}
