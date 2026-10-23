use crate::{Architecture, Emulator};

impl Emulator {
    pub(super) fn uname(&mut self, address: u64) -> u64 {
        let machine = match self.architecture() {
            Architecture::Arm32 => "armv7l",
            Architecture::Arm64 => "armv8l",
        };
        let mut bytes = [0; 390];
        for (field, value) in bytes.as_chunks_mut::<65>().0.iter_mut().zip([
            "Linux",
            "localhost",
            "1.0.0-unidbg",
            "#1 SMP PREEMPT Thu Apr 19 14:36:58 CST 2018",
            machine,
            "localdomain",
        ]) {
            field[..value.len()].copy_from_slice(value.as_bytes());
        }
        if self.write_bytes(address, &bytes).is_err() {
            (-14i64) as u64
        } else {
            0
        }
    }
}
