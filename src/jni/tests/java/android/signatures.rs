use super::*;

fn certificate(emu: &mut Emulator, bytes: &[u8]) -> Result<Handle> {
    let bytes = emu.new_byte_array(bytes);
    construct(
        emu,
        "android/content/pm/Signature",
        "([B)V",
        &[bytes.into()],
    )
}

fn certificates(emu: &mut Emulator, array: Handle) -> Result<Vec<Vec<u8>>> {
    emu.object_array(array)?
        .into_iter()
        .map(|signature| {
            let bytes = call(emu, signature, "toByteArray()[B", &[])?.object();
            emu.byte_array(bytes)
        })
        .collect()
}

#[test]
fn signatures_copy_bytes_and_use_android_hex_hash_and_equality() -> Result<()> {
    for arch in ARCHES {
        let (_file, mut emu) = apk_emulator(arch)?;
        // Literals from java.util.Arrays.hashCode(byte[]) in JShell.
        for (bytes, hash, hex) in [
            (&[][..], 1, ""),
            (&[1, 2, 255][..], 30813, "0102ff"),
            (&[128, 255, 0, 127][..], -2890561, "80ff007f"),
        ] {
            let signature = certificate(&mut emu, bytes)?;
            let other = certificate(&mut emu, bytes)?;
            assert_eq!(
                call(&mut emu, signature, "hashCode()I", &[])?,
                JniValue::Int(hash)
            );
            let value = call(
                &mut emu,
                signature,
                "toCharsString()Ljava/lang/String;",
                &[],
            )?
            .object();
            assert_eq!(emu.string(value)?, hex);
            assert_eq!(
                call(
                    &mut emu,
                    signature,
                    "equals(Ljava/lang/Object;)Z",
                    &[other.into()]
                )?,
                JniValue::Boolean(true)
            );
            let unequal = certificate(&mut emu, &[42])?;
            let opaque = emu.new_object("android/content/pm/Signature", bytes.to_vec());
            for object in [unequal, opaque, Handle(0)] {
                assert_eq!(
                    call(
                        &mut emu,
                        signature,
                        "equals(Ljava/lang/Object;)Z",
                        &[object.into()]
                    )?,
                    JniValue::Boolean(false)
                );
            }
            let array = call(&mut emu, signature, "toByteArray()[B", &[])?.object();
            if !bytes.is_empty() {
                let replacement = emu.new_cstring("x")?;
                jni_call(&mut emu, 208, &[array.0, 0, 1, replacement])?; // SetByteArrayRegion
            }
            let again = call(&mut emu, signature, "toByteArray()[B", &[])?.object();
            assert_ne!(array, again);
            assert_eq!(emu.byte_array(again)?, bytes);
            let text = text(&mut emu, signature)?;
            assert_eq!(
                text,
                format!("android.content.pm.Signature@{:x}", hash as u32)
            );
        }
        let input = emu.new_byte_array(&[1, 2]);
        let signature = construct(
            &mut emu,
            "android/content/pm/Signature",
            "([B)V",
            &[input.into()],
        )?;
        let replacement = emu.new_cstring("x")?;
        jni_call(&mut emu, 208, &[input.0, 0, 1, replacement])?;
        let output = call(&mut emu, signature, "toByteArray()[B", &[])?.object();
        assert_eq!(emu.byte_array(output)?, [1, 2]);
        let error = construct(
            &mut emu,
            "android/content/pm/Signature",
            "([B)V",
            &[Handle(0).into()],
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("java/lang/NullPointerException"), "{error}");
    }
    Ok(())
}

#[test]
fn signing_info_reads_current_signers_rotation_and_multiple_signers() -> Result<()> {
    for arch in ARCHES {
        let (_file, mut emu) = apk_emulator(arch)?;
        let info = package_info(&mut emu, 0x08000000)?;
        let signing = package_field(
            &mut emu,
            info,
            "signingInfo",
            "Landroid/content/pm/SigningInfo;",
        )?
        .object();
        for (signers, history, expected) in [
            (vec![], vec![], Some(vec![])),
            (vec![vec![1]], vec![], Some(vec![vec![1]])),
            (
                vec![vec![2]],
                vec![vec![1], vec![2]],
                Some(vec![vec![1], vec![2]]),
            ),
            (vec![vec![2], vec![3]], vec![], None),
            (vec![vec![2], vec![3]], vec![vec![1], vec![2]], None),
        ] {
            let apk = emu.apk_mut().unwrap();
            apk.signers = signers.clone();
            apk.signing_history = history.clone();
            assert_eq!(
                call(&mut emu, signing, "hasMultipleSigners()Z", &[])?,
                JniValue::Boolean(signers.len() > 1)
            );
            assert_eq!(
                call(&mut emu, signing, "hasPastSigningCertificates()Z", &[])?,
                JniValue::Boolean(!history.is_empty())
            );
            let array = call(
                &mut emu,
                signing,
                "getApkContentsSigners()[Landroid/content/pm/Signature;",
                &[],
            )?
            .object();
            assert_eq!(certificates(&mut emu, array)?, signers);
            let array = call(
                &mut emu,
                signing,
                "getSigningCertificateHistory()[Landroid/content/pm/Signature;",
                &[],
            )?
            .object();
            match expected {
                Some(expected) => assert_eq!(certificates(&mut emu, array)?, expected),
                None => assert_eq!(array, Handle(0)),
            }
        }
    }
    Ok(())
}
