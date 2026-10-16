use crate::Result;
use goblin::elf::{
    Elf,
    program_header::{PT_LOAD, ProgramHeader},
};
use memmap2::{Mmap, MmapOptions};
use std::{
    collections::HashMap,
    fs::File,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

pub(crate) struct Image {
    pub bytes: Mmap,
    pub segments: Vec<ProgramHeader>,
    pub dynstr: u64,
    pub symbols: Vec<Symbol>,
    pub exports: HashMap<String, usize>,
    pub needed: Vec<String>,
    pub relocations: Vec<Relocation>,
    pub init: u64,
    pub init_array: u64,
    pub init_count: u64,
    pub size: u64,
    pub phoff: u64,
    pub phnum: u16,
}

pub(crate) struct Symbol {
    pub name: String,
    pub name_offset: usize,
    pub value: u64,
    pub defined: bool,
    pub absolute: bool,
    pub binding: u8,
}

impl Image {
    pub fn open(path: &Path) -> Result<Arc<Self>> {
        static IMAGES: OnceLock<Mutex<HashMap<PathBuf, Arc<Image>>>> = OnceLock::new();
        let path = path.canonicalize()?;
        let mut cache = IMAGES.get_or_init(Default::default).lock().unwrap();
        if let Some(image) = cache.get(&path) {
            return Ok(image.clone());
        }
        let file = File::open(&path)?;
        // Library files must remain unchanged for the lifetime of the process.
        let bytes = unsafe { MmapOptions::new().map(&file)? };
        let elf = Elf::parse(&bytes)?;
        if elf.header.e_machine != 183 || !elf.is_64 || !elf.little_endian {
            return Err(std::io::Error::from(std::io::ErrorKind::InvalidData).into());
        }
        let mut relocations: Vec<_> = elf
            .dynrelas
            .iter()
            .chain(elf.pltrelocs.iter())
            .map(|r| Relocation {
                offset: r.r_offset,
                info: (r.r_sym as u64) << 32 | u64::from(r.r_type),
                addend: r.r_addend.unwrap_or(0),
            })
            .collect();
        let dynamic = elf.dynamic.as_ref().unwrap();
        let tag = |tag| {
            dynamic
                .dyns
                .iter()
                .find(|d| d.d_tag == tag)
                .map_or(0, |d| d.d_val)
        };
        let packed = tag(0x60000011);
        if packed != 0 {
            let seg = elf
                .program_headers
                .iter()
                .find(|s| {
                    s.p_type == PT_LOAD && packed >= s.p_vaddr && packed < s.p_vaddr + s.p_filesz
                })
                .unwrap();
            let start = (seg.p_offset + packed - seg.p_vaddr) as usize;
            relocations.extend(android_relocations(
                &bytes[start..start + tag(0x60000012) as usize],
            )?);
        }
        let symbols: Vec<_> = elf
            .dynsyms
            .iter()
            .map(|s| Symbol {
                name: elf.dynstrtab.get_at(s.st_name).unwrap_or("").into(),
                name_offset: s.st_name,
                value: s.st_value,
                defined: s.st_shndx != 0,
                absolute: s.st_shndx == 0xfff1,
                binding: s.st_bind(),
            })
            .collect();
        let exports = symbols
            .iter()
            .enumerate()
            .filter(|(_, s)| s.defined && !s.name.is_empty())
            .map(|(i, s)| (s.name.clone(), i))
            .collect();
        let segments: Vec<_> = elf
            .program_headers
            .iter()
            .filter(|s| s.p_type == PT_LOAD)
            .cloned()
            .collect();
        let size = segments
            .iter()
            .map(|s| crate::page_up(s.p_vaddr + s.p_memsz))
            .max()
            .unwrap();
        let image = Self {
            segments,
            dynstr: tag(5),
            symbols,
            exports,
            needed: elf.libraries.iter().map(|s| (*s).to_owned()).collect(),
            relocations,
            init: tag(12),
            init_array: tag(25),
            init_count: tag(27) / 8,
            size,
            phoff: elf.header.e_phoff,
            phnum: elf.header.e_phnum,
            bytes,
        };
        let image = Arc::new(image);
        cache.insert(path, image.clone());
        Ok(image)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Relocation {
    pub offset: u64,
    pub info: u64,
    pub addend: i64,
}

pub fn android_relocations(bytes: &[u8]) -> Result<Vec<Relocation>> {
    fn sleb(bytes: &mut &[u8]) -> Result<i64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let (&byte, rest) = bytes
                .split_first()
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::UnexpectedEof))?;
            *bytes = rest;
            value |= u64::from(byte & 127) << shift;
            if byte & 128 == 0 {
                if shift < 57 && byte & 64 != 0 {
                    value |= !0 << (shift + 7);
                }
                return Ok(value as i64);
            }
        }
        Err(std::io::Error::from(std::io::ErrorKind::InvalidData).into())
    }
    if !bytes.starts_with(b"APS2") {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidData).into());
    }
    let mut bytes = &bytes[4..];
    let count = usize::try_from(sleb(&mut bytes)?)?;
    let mut offset = sleb(&mut bytes)?;
    let mut addend = 0i64;
    let mut relocs = Vec::with_capacity(count);
    while relocs.len() < count {
        let size = usize::try_from(sleb(&mut bytes)?)?;
        let flags = sleb(&mut bytes)?;
        if size == 0 || size > count - relocs.len() {
            return Err(std::io::Error::from(std::io::ErrorKind::InvalidData).into());
        }
        let delta = if flags & 2 != 0 { sleb(&mut bytes)? } else { 0 };
        let info = if flags & 1 != 0 {
            sleb(&mut bytes)? as u64
        } else {
            0
        };
        let add_delta = if flags & 12 == 12 {
            sleb(&mut bytes)?
        } else {
            0
        };
        for _ in 0..size {
            offset = offset.wrapping_add(if flags & 2 != 0 {
                delta
            } else {
                sleb(&mut bytes)?
            });
            let info = if flags & 1 != 0 {
                info
            } else {
                sleb(&mut bytes)? as u64
            };
            addend = if flags & 8 == 0 {
                0
            } else {
                addend.wrapping_add(if flags & 4 != 0 {
                    add_delta
                } else {
                    sleb(&mut bytes)?
                })
            };
            relocs.push(Relocation {
                offset: offset as u64,
                info,
                addend,
            });
        }
    }
    Ok(relocs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_relocations_accumulate_offsets_and_signed_addends() {
        // APS2: 2 entries, initial offset 0, group of 2 with grouped
        // info=1027 and offset delta=8, individual addend deltas 5,-2.
        let data = b"APS2\x02\x00\x02\x0b\x08\x83\x08\x05\x7e";
        assert_eq!(
            android_relocations(data).unwrap(),
            vec![
                Relocation {
                    offset: 8,
                    info: 1027,
                    addend: 5
                },
                Relocation {
                    offset: 16,
                    info: 1027,
                    addend: 3
                },
            ]
        );
    }
}
