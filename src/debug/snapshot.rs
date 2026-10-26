use crate::{Emulator, Mapping, ModuleInfo, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Read, Write},
    path::Path,
};

mod diff;
pub use diff::{Change, Diff};

pub struct Snapshot {
    pub thread: u32,
    pub registers: Vec<(String, u64)>,
    pub modules: Vec<ModuleInfo>,
    pub memory: Vec<(Mapping, Vec<u8>)>,
    pub heap: Vec<crate::Allocation>,
}

#[derive(Serialize, Deserialize)]
struct Header {
    snapshot: u32,
    version: String,
    thread: u32,
    registers: Vec<(String, u64)>,
    modules: Vec<ModuleInfo>,
    maps: Vec<Mapping>,
    #[serde(default)]
    heap: Vec<crate::Allocation>,
}

impl Emulator {
    pub fn snapshot(&self) -> Result<Snapshot> {
        Ok(Snapshot {
            thread: self.threads.tid(),
            registers: self
                .registers()?
                .into_iter()
                .map(|(name, value)| (name.into(), value))
                .collect(),
            modules: self.modules(),
            memory: self.read_mappings(self.maps()?).collect::<Result<_>>()?,
            heap: if self.trace.heap {
                self.known_allocations()
            } else {
                Vec::new()
            },
        })
    }
}

impl Snapshot {
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let mut file = BufWriter::new(File::create(path)?);
        serde_json::to_writer(
            &mut file,
            &Header {
                snapshot: 1,
                version: env!("CARGO_PKG_VERSION").into(),
                thread: self.thread,
                registers: self.registers.clone(),
                modules: self.modules.clone(),
                maps: self
                    .memory
                    .iter()
                    .map(|(mapping, _)| mapping.clone())
                    .collect(),
                heap: self.heap.clone(),
            },
        )?;
        file.write_all(b"\n")?;
        for (_, bytes) in &self.memory {
            file.write_all(bytes)?;
        }
        file.flush()?;
        Ok(())
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let mut file = BufReader::new(File::open(path)?);
        let mut line = String::new();
        file.read_line(&mut line)?;
        let header: Header = serde_json::from_str(&line)?;
        if header.snapshot != 1 {
            return Err(format!("unsupported snapshot format {}", header.snapshot).into());
        }
        let mut memory = Vec::new();
        for mapping in header.maps {
            let mut bytes = vec![0; (mapping.end - mapping.start) as usize];
            file.read_exact(&mut bytes)?;
            memory.push((mapping, bytes));
        }
        Ok(Self {
            thread: header.thread,
            registers: header.registers,
            modules: header.modules,
            memory,
            heap: header.heap,
        })
    }
}
