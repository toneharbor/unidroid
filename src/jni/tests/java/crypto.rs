use super::*;

#[test]
fn message_digests_match_java_and_reset_after_digest() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        for (algorithm, expected) in [
            ("MD5", "900150983cd24fb0d6963f7d28e17f72"),
            ("SHA-1", "a9993e364706816aba3e25717850c26c9cd0d89d"),
            (
                "SHA-224",
                "23097d223405d8228642a477bda255b32aadbce4bda0b3f7e36c9da7",
            ),
            (
                "SHA-256",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                "SHA-384",
                "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7",
            ),
            (
                "SHA-512",
                "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
            ),
        ] {
            let name = emu.new_string(algorithm);
            let digest = static_call(
                &mut emu,
                "java/security/MessageDigest",
                "getInstance(Ljava/lang/String;)Ljava/security/MessageDigest;",
                &[name.into()],
            )?
            .object();
            let bytes = emu.new_byte_array(b"xabcy");
            call(
                &mut emu,
                digest,
                "update([BII)V",
                &[bytes.into(), 1.into(), 2.into()],
            )?;
            call(
                &mut emu,
                digest,
                "update(B)V",
                &[JniValue::Byte(b'c' as i8)],
            )?;
            let output = call(&mut emu, digest, "digest()[B", &[])?.object();
            assert_eq!(
                emu.byte_array(output)?
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                expected
            );
            let bytes = emu.new_byte_array(b"abc");
            call(&mut emu, digest, "update([B)V", &[bytes.into()])?;
            call(&mut emu, digest, "reset()V", &[])?;
            let name = call(&mut emu, digest, "getAlgorithm()Ljava/lang/String;", &[])?.object();
            assert_eq!(emu.string(name)?, algorithm);
            let prefix = emu.new_byte_array(b"ab");
            call(&mut emu, digest, "update([B)V", &[prefix.into()])?;
            let checkpoint = emu.checkpoint()?;
            call(&mut emu, digest, "update([B)V", &[bytes.into()])?;
            emu.rewind(&checkpoint)?;
            let suffix = emu.new_byte_array(b"c");
            let output = call(&mut emu, digest, "digest([B)[B", &[suffix.into()])?.object();
            assert_eq!(
                emu.byte_array(output)?
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                expected
            );
            let output = call(&mut emu, digest, "digest([B)[B", &[bytes.into()])?.object();
            assert_eq!(
                emu.byte_array(output)?
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                expected
            );
        }
    }
    Ok(())
}

#[test]
fn android_base64_flags_padding_and_line_endings() -> Result<()> {
    for arch in ARCHES {
        let mut emu = emulator(arch)?;
        let input = emu.new_byte_array(&[0xfb, 0xff]);
        for (flags, expected) in [
            (0, "+/8=\n"),
            (1, "+/8\n"),
            (2, "+/8="),
            (3, "+/8"),
            (4, "+/8=\r\n"),
            (5, "+/8\r\n"),
            (6, "+/8="),
            (7, "+/8"),
            (8, "-_8=\n"),
            (9, "-_8\n"),
            (10, "-_8="),
            (11, "-_8"),
            (12, "-_8=\r\n"),
            (13, "-_8\r\n"),
            (14, "-_8="),
            (15, "-_8"),
        ] {
            let output = static_call(
                &mut emu,
                "android/util/Base64",
                "encodeToString([BI)Ljava/lang/String;",
                &[input.into(), flags.into()],
            )?
            .object();
            assert_eq!(emu.string(output)?, expected);
            let output = static_call(
                &mut emu,
                "android/util/Base64",
                "decode(Ljava/lang/String;I)[B",
                &[output.into(), flags.into()],
            )?
            .object();
            assert_eq!(emu.byte_array(output)?, [0xfb, 0xff]);
            let encoded = emu.new_byte_array(expected.as_bytes());
            let decoded = static_call(
                &mut emu,
                "android/util/Base64",
                "decode([BI)[B",
                &[encoded.into(), flags.into()],
            )?
            .object();
            assert_eq!(emu.byte_array(decoded)?, [0xfb, 0xff]);
        }
        let spaced = emu.new_string(" +\t/\r\n8= ");
        let decoded = static_call(
            &mut emu,
            "android/util/Base64",
            "decode(Ljava/lang/String;I)[B",
            &[spaced.into(), 0.into()],
        )?
        .object();
        assert_eq!(emu.byte_array(decoded)?, [0xfb, 0xff]);
        for (length, expected) in [
            (0, ""),
            (
                57,
                "YWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFh\n",
            ),
            (
                58,
                "YWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFh\nYQ==\n",
            ),
        ] {
            let input = emu.new_byte_array(&vec![b'a'; length]);
            let output = static_call(
                &mut emu,
                "android/util/Base64",
                "encode([BI)[B",
                &[input.into(), 0.into()],
            )?
            .object();
            assert_eq!(emu.byte_array(output)?, expected.as_bytes());
        }
    }
    Ok(())
}
