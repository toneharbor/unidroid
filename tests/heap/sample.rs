use super::*;

#[path = "../support/jna.rs"]
mod jna;

#[test]
fn real_jni_calls_return_the_same_bytes_with_heap_tracking_enabled() -> Result<()> {
    for abi in ["arm64-v8a", "armeabi-v7a"] {
        let path = std::path::Path::new("tests/fixtures").join(abi);
        let mut plain = Emulator::builder().build()?;
        let expected = jna::run(&mut plain, &path)?;
        let mut tracked = Emulator::builder()
            .trace(Trace {
                heap: true,
                ..Trace::default()
            })
            .build()?;
        let actual = jna::run(&mut tracked, &path)?;
        assert_eq!(
            (
                actual.sdk,
                actual.malloc_nonzero,
                actual.version,
                actual.checksum,
                actual.size
            ),
            (
                expected.sdk,
                expected.malloc_nonzero,
                expected.version,
                expected.checksum,
                expected.size
            )
        );
        assert!(!tracked.heap().is_empty());
        assert!(
            tracked
                .heap()
                .iter()
                .any(|a| a.function == "malloc" && a.size == 32)
        );
        assert!(plain.heap().is_empty());
    }
    Ok(())
}
