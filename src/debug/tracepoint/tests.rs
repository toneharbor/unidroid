use super::*;
use crate::Architecture;

#[test]
fn updating_one_tracepoint_keeps_other_hooks_installed() -> Result<()> {
    for arch in [Architecture::Arm32, Architecture::Arm64] {
        let mut emu = Emulator::builder().build()?;
        emu.setup_cpu(arch)?;
        let point = Tracepoint {
            values: vec!["0x1".into()],
            log: false,
        };
        emu.add_tracepoint(0x20100, &point)?;
        let first = emu.tracepoints.points[&0x20100].1;
        for address in [0x20104, 0x20108, 0x20104] {
            emu.add_tracepoint(address, &point)?;
            assert_eq!(emu.tracepoints.points[&0x20100].1, first);
        }
        emu.remove_tracepoint(0x20104)?;
        assert_eq!(emu.tracepoints.points[&0x20100].1, first);
        emu.remove_tracepoint(0x20108)?;
        assert_eq!(emu.tracepoints.points.len(), 1);
        assert_eq!(emu.tracepoints.points[&0x20100].1, first);
    }
    Ok(())
}
