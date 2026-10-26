use crate::{CpuData, arch::Architecture};
use unicorn_engine::Unicorn;

pub(super) fn register_arguments(
    cpu: &Unicorn<'_, CpuData>,
    arch: Architecture,
    strings: u8,
) -> Vec<String> {
    arguments(
        cpu,
        (0..4).map(|i| cpu.reg_read(arch.gp(i)).expect("argument register")),
        strings,
    )
}

// Bit i marks a C string in argument i.
pub(super) fn arguments(
    cpu: &Unicorn<'_, CpuData>,
    values: impl IntoIterator<Item = u64>,
    strings: u8,
) -> Vec<String> {
    values
        .into_iter()
        .enumerate()
        .map(|(i, value)| {
            if strings & (1 << i) != 0 {
                trace_string(cpu, value).unwrap_or_else(|| format!("0x{value:x}"))
            } else {
                format!("0x{value:x}")
            }
        })
        .collect()
}

fn trace_string(cpu: &Unicorn<'_, CpuData>, address: u64) -> Option<String> {
    let mut bytes = Vec::new();
    for i in 0..256 {
        let mut byte = [0];
        cpu.mem_read(address.checked_add(i)?, &mut byte).ok()?;
        if byte[0] == 0 {
            break;
        }
        bytes.push(byte[0]);
    }
    Some(format!("{:?}", String::from_utf8_lossy(&bytes)))
}
