use crate::{Emulator, Result};
use std::fmt::Write;

impl Emulator {
    pub(super) fn proc_maps(&self) -> Result<Vec<u8>> {
        let mut text = String::new();
        for mapping in self.maps()? {
            let (dev, label) = if self
                .modules
                .iter()
                .any(|module| module.name == mapping.name)
            {
                let dir = if mapping.name.ends_with(".so") {
                    self.architecture().library_dir()
                } else {
                    "bin"
                };
                ("b3:19", format!("/system/{dir}/{}", mapping.name))
            } else if mapping.name == "[stack]" {
                ("00:00", "[stack]".into())
            } else {
                ("00:00", "anonymous".into())
            };
            writeln!(
                text,
                "{:08x}-{:08x} {}p 00000000 {dev} 0          {label}",
                mapping.start, mapping.end, mapping.perms
            )?;
        }
        Ok(text.into_bytes())
    }
}
