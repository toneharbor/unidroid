use std::time::{Duration, Instant};
use unidroid::{Emulator, Handle, Result, Trace};

#[derive(Debug, PartialEq)]
struct Outcome {
    bytes: Vec<u8>,
    coverage: Vec<(u64, u64)>,
}

fn call(emu: &mut Emulator, key: Handle, iv: Handle, input: u8) -> Result<Outcome> {
    let data = emu.new_byte_array(&[input; 32]);
    emu.reset_coverage();
    emu.call_static_jni_method(
        &mut (),
        "org/telegram/messenger/Utilities",
        "aesCtrDecryptionByteArray([B[B[BIII)V",
        &[
            data.into(),
            key.into(),
            iv.into(),
            0.into(),
            32.into(),
            0.into(),
        ],
    )?;
    Ok(Outcome {
        bytes: emu.byte_array(data)?,
        coverage: emu
            .coverage()
            .into_iter()
            .map(|b| (b.address, b.count))
            .collect(),
    })
}

#[test]
fn repeated_jni_inputs_and_one_hundred_independent_copies_match() -> Result<()> {
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let trace = Trace {
            coverage: vec!["libtmessages.29.so".into()],
            ..Trace::default()
        };
        let mut emu = Emulator::builder()
            .fixed_clock(1_700_000_000)
            .random_seed(42)
            .trace(trace.clone())
            .on_event(|_| {})
            .build()?;
        let module = emu.load_library(format!("tests/fixtures/{abi}/libtmessages.29.so"))?;
        emu.call_jni_onload(module, &mut ())?;
        let key = emu.new_byte_array(&[0; 32]);
        let iv = emu.new_byte_array(&[0; 16]);
        let marker = emu.malloc(16)?;
        emu.write_bytes(marker, &[7; 16])?;
        let checkpoint = emu.checkpoint()?;
        let writable: u64 = emu
            .maps()?
            .iter()
            .filter(|m| m.perms.contains('w'))
            .map(|m| m.end - m.start)
            .sum();
        let mut expected = Vec::new();
        for input in 0..100 {
            emu.rewind(&checkpoint)?;
            expected.push(call(&mut emu, key, iv, input)?);
        }
        assert!(!expected[0].coverage.is_empty());
        let mut elapsed = Duration::ZERO;
        for input in 0..100 {
            let counts = serde_json::to_value(emu.coverage())?;
            let start = Instant::now();
            emu.rewind(&checkpoint)?;
            elapsed += start.elapsed();
            assert_eq!(serde_json::to_value(emu.coverage())?, counts);
            assert_eq!(emu.byte_array(iv)?, [0; 16]);
            assert_eq!(call(&mut emu, key, iv, input)?, expected[input as usize]);
        }
        let mut first_copy: Option<Emulator> = None;
        for input in 0..100 {
            let mut copy = Emulator::builder()
                .trace(trace.clone())
                .on_event(|_| {})
                .build()?;
            copy.rewind(&checkpoint)?;
            assert_eq!(copy.read_bytes(marker, 16)?, [7; 16]);
            assert_eq!(call(&mut copy, key, iv, input)?, expected[input as usize]);
            copy.write_bytes(marker, &[input; 16])?;
            assert_eq!(emu.read_bytes(marker, 16)?, [7; 16]);
            if let Some(first) = &first_copy {
                assert_eq!(first.read_bytes(marker, 16)?, [0; 16]);
            } else {
                first_copy = Some(copy);
            }
        }
        eprintln!(
            "tmessages AES-CTR {abi}: checkpoint writable bytes={writable}; mean rewind after call={:.3} ms (100 rewinds)",
            elapsed.as_secs_f64() * 10.0
        );
        drop(emu);
        drop(checkpoint);
        assert_eq!(first_copy.unwrap().byte_array(key)?, [0; 32]);
    }
    Ok(())
}
