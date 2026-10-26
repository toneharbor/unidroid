use crate::apk::libraries::Library;
use crate::{
    Emulator, HostFunction, Image, JniHandler, Loaded, Module, Region, Result, SharedMapping,
    page_down, page_up,
};
use std::{collections::HashMap, path::Path};
use unicorn_engine::unicorn_const::Prot;

impl Emulator {
    pub fn symbol_alias(&mut self, name: impl Into<String>, target: impl Into<String>) {
        self.symbol_aliases.insert(name.into(), target.into());
    }
    pub fn load_library(&mut self, path: impl AsRef<Path>) -> Result<Module> {
        let module = self.load_library_without_constructors(path)?;
        self.run_constructors(module, &mut ())?;
        Ok(module)
    }
    /// Map and relocate a library and its dependencies without running their constructors.
    pub fn load_library_without_constructors(&mut self, path: impl AsRef<Path>) -> Result<Module> {
        let path = path.as_ref();
        if let Some(directory) = path.parent()
            && !self.library_dirs.iter().any(|loaded| loaded == directory)
        {
            self.library_dirs.push(directory.to_owned());
        }
        self.load_library_internal(Library::Host(path.to_owned()))
    }
    pub(super) fn load_library_internal(&mut self, library: Library) -> Result<Module> {
        let module = self.load_image(library);
        self.rebuild_module_table();
        let module = module?;
        for i in 0..self.modules.len() {
            self.relocate(Module(i), None)?;
        }
        if self.trace.imports {
            self.refresh_imports();
        }
        if !self.trace.code.is_empty() {
            self.trace_code(&self.trace.code.clone())?;
        }
        if !self.trace.calls.is_empty() {
            self.trace_calls(&self.trace.calls.clone())?;
        }
        if !self.trace.coverage.is_empty() {
            self.trace_coverage(&self.trace.coverage.clone())?;
        }
        self.trace_heap(self.trace.heap)?;
        Ok(module)
    }
    fn load_image(&mut self, library: Library) -> Result<Module> {
        let path = library.path();
        let name = path
            .file_name()
            .ok_or_else(|| format!("cannot load {}: not a file path", path.display()))?
            .to_string_lossy()
            .into_owned();
        let image = library.image()?;
        if self.uc.is_none() {
            self.setup_cpu(image.arch)?;
        }
        if image.arch != self.architecture() {
            return Err(format!(
                "cannot load {name}: it is {} but this emulator runs {}",
                image.arch.name(),
                self.architecture().name()
            )
            .into());
        }
        let app = matches!(library, Library::Apk { .. })
            || path
                .parent()
                .is_some_and(|parent| self.library_dirs.iter().any(|dir| dir == parent));
        if let Some(i) = self.modules.iter().position(|m| m.name == name) {
            self.modules[i].app |= app;
            return Ok(Module(i));
        }
        let base = self.next_module;
        self.next_module += (image.size + 0xffff) & !0xffff;
        for segment in &image.segments {
            let start = base + page_down(segment.p_vaddr);
            let end = base + page_up(segment.p_vaddr + segment.p_memsz);
            let permissions = Prot(
                ((segment.p_flags & 4) >> 2) | (segment.p_flags & 2) | ((segment.p_flags & 1) << 2),
            );
            let private = segment.p_flags & 2 != 0
                || image.relocations.iter().any(|r| {
                    r.offset >= page_down(segment.p_vaddr)
                        && r.offset < page_up(segment.p_vaddr + segment.p_memsz)
                });
            if private {
                self.cpu_mut()?.mem_map(start, end - start, permissions)?;
                let offset = segment.p_offset as usize;
                self.write_bytes(
                    base + segment.p_vaddr,
                    &image.bytes[offset..offset + segment.p_filesz as usize],
                )?;
            } else {
                let file_offset = page_down(segment.p_offset) as usize;
                // The Arc keeps this read-only mmap alive until after uc_close.
                unsafe {
                    self.cpu_mut()?.mem_map_ptr(
                        start,
                        end - start,
                        permissions,
                        image.bytes.as_ptr().add(file_offset).cast_mut().cast(),
                    )?;
                }
            }
            self.regions.push(Region {
                start,
                end,
                shared: (!private).then(|| SharedMapping {
                    image: image.clone(),
                    start,
                    file_offset: page_down(segment.p_offset) as usize,
                }),
            });
        }
        let id = Module(self.modules.len());
        let guest_path = library.guest_path();
        let path_ptr = self.new_cstring(&guest_path)?;
        let p = self.pointer_size();
        let info_ptr = self.malloc((4 * p) as usize)?;
        let first = &image.segments[0];
        self.write_pointer(info_ptr, base)?;
        self.write_pointer(info_ptr + p, path_ptr)?;
        self.write_pointer(
            info_ptr + 2 * p,
            base + first.p_vaddr - first.p_offset + image.phoff,
        )?;
        self.write_bytes(info_ptr + 3 * p, &image.phnum.to_le_bytes())?;
        self.modules.push(Loaded {
            app,
            path,
            guest_path: matches!(library, Library::Apk { .. }).then_some(guest_path),
            name: name.clone(),
            image: image.clone(),
            base,
            dependencies: Vec::new(),
            relocated: false,
            initialized: false,
            path_ptr,
            info_ptr,
            dl_references: 0,
        });
        for needed in &image.needed {
            if let Some(library) = self.find_library(needed) {
                let dep = self.load_image(library)?;
                self.modules[id.0].dependencies.push(dep);
            } else {
                let symbols = if self.apk().is_some() && needed == "libandroid.so" {
                    "its asset functions are built in; the rest stay unresolved"
                } else {
                    "its symbols stay unresolved"
                };
                self.events.borrow_mut().emit(crate::Event::Warning { message: format!(
                    "{needed} needed by {name} not found in the APK, next to the loaded libraries or in rootfs; {symbols}"
                ) });
            }
        }
        Ok(id)
    }
    pub(super) fn local_symbol(&self, module: Module, name: &str) -> Option<u64> {
        if let Some(value) = self.overrides.get(name) {
            return Some(*value);
        }
        self.exported_symbol(module, name)
    }
    pub(super) fn exported_symbol(&self, module: Module, name: &str) -> Option<u64> {
        let m = &self.modules[module.0];
        let symbol = &m.image.symbols[*m.image.exports.get(name)?];
        Some(if symbol.absolute {
            symbol.value
        } else {
            m.base + symbol.value
        })
    }
    pub(super) fn search_symbol(
        &self,
        module: Module,
        name: &str,
        visited: &mut Vec<Module>,
    ) -> Option<u64> {
        if visited.contains(&module) {
            return None;
        }
        visited.push(module);
        self.local_symbol(module, name).or_else(|| {
            self.modules[module.0]
                .dependencies
                .iter()
                .find_map(|dep| self.search_symbol(*dep, name, visited))
        })
    }
    pub fn find_symbol(&self, module: Module, name: &str) -> Result<u64> {
        if module.0 >= self.modules.len() {
            return Err(format!("no module {} is loaded", module.0).into());
        }
        self.search_symbol(module, name, &mut Vec::new())
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound).into())
    }
    pub(super) fn relocate(&mut self, module: Module, hook: Option<&str>) -> Result<()> {
        if self.modules[module.0].relocated && hook.is_none() {
            return Ok(());
        }
        let image = self.modules[module.0].image.clone();
        let base = self.modules[module.0].base;
        let name = self.modules[module.0].name.clone();
        let mut unresolved = HashMap::new();
        for relocation in &image.relocations {
            let kind = relocation.info as u32;
            if kind == 0 {
                continue;
            }
            let Some((relative, implicit)) = image.arch.relocation(kind) else {
                return Err(format!(
                    "unsupported relocation type {kind} in {} at offset 0x{:x}",
                    self.modules[module.0].name, relocation.offset
                )
                .into());
            };
            if let Some(name) = hook
                && (relative || image.symbols[(relocation.info >> 32) as usize].name != name)
            {
                continue;
            }
            let context = |error: Box<dyn std::error::Error>| -> Box<dyn std::error::Error> {
                format!(
                    "relocation in {name} at offset 0x{:x}: {error}",
                    relocation.offset
                )
                .into()
            };
            let addend = if implicit {
                implicit_addend(&image, relocation.offset).map_err(context)?
            } else {
                relocation.addend
            };
            let value = if relative {
                base
            } else {
                let symbol = &image.symbols[(relocation.info >> 32) as usize];
                if let Some(value) = self.overrides.get(&symbol.name) {
                    *value
                } else if symbol.defined {
                    if symbol.absolute {
                        symbol.value
                    } else {
                        base + symbol.value
                    }
                } else if let Some(value) = (0..self.modules.len())
                    .find_map(|i| self.local_symbol(Module(i), &symbol.name))
                    .or_else(|| {
                        let target = self.symbol_aliases.get(&symbol.name)?;
                        (0..self.modules.len()).find_map(|i| self.local_symbol(Module(i), target))
                    })
                {
                    value
                } else if symbol.binding == 2 {
                    0
                } else if let Some(value) = unresolved.get(&symbol.name) {
                    *value
                } else {
                    let value = self.stub(HostFunction::Unresolved {
                        symbol: symbol.name.clone(),
                        module: self.modules[module.0].name.clone(),
                    })?;
                    unresolved.insert(symbol.name.clone(), value);
                    value
                }
            }
            .wrapping_add_signed(addend);
            self.write_pointer(base + relocation.offset, value)
                .map_err(context)?;
        }
        self.modules[module.0].relocated = true;
        Ok(())
    }
    /// Run pending dependency constructors depth first, then this module's constructors.
    pub fn run_constructors(&mut self, module: Module, handler: &mut dyn JniHandler) -> Result<()> {
        if module.0 >= self.modules.len() {
            return Err(format!("no module {} is loaded", module.0).into());
        }
        if self.modules[module.0].initialized {
            return Ok(());
        }
        self.modules[module.0].initialized = true;
        for dep in self.modules[module.0].dependencies.clone() {
            self.run_constructors(dep, handler)?;
        }
        let image = self.modules[module.0].image.clone();
        let base = self.modules[module.0].base;
        if image.init != 0 {
            self.call_function_kind(handler, base + image.init, &[], b'V')?;
        }
        for i in 0..image.init_count {
            let address = self.read_pointer(base + image.init_array + i * self.pointer_size())?;
            if address != 0 && address != self.architecture().pointer_mask() {
                self.call_function_kind(handler, address, &[], b'V')?;
            }
        }
        Ok(())
    }
    pub fn location(&self, address: u64) -> String {
        self.module_table().location(address)
    }
}

fn implicit_addend(image: &Image, address: u64) -> Result<i64> {
    let segment = image
        .segments
        .iter()
        .find(|segment| address >= segment.p_vaddr && address < segment.p_vaddr + segment.p_memsz)
        .ok_or("target is outside every PT_LOAD segment")?;
    let offset = address - segment.p_vaddr;
    let size = segment
        .p_filesz
        .saturating_sub(offset)
        .min(image.arch.pointer_size()) as usize;
    let mut bytes = [0; 8];
    if size != 0 {
        let start = (segment.p_offset + offset) as usize;
        bytes[..size].copy_from_slice(&image.bytes[start..start + size]);
    }
    Ok(u64::from_le_bytes(bytes) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_elf;

    #[test]
    fn symbol_alias_binds_missing_imports_but_preserves_exports() -> Result<()> {
        for binding in [0x10, 0x20] {
            for exported in [false, true] {
                let importer = test_elf::TestLibrary::new(true, false);
                let mut bytes = std::fs::read(&importer.0)?;
                // The synthetic ELF's fourth symbol is an undefined import named "weak".
                bytes[0x1264] = binding;
                std::fs::write(&importer.0, bytes)?;
                let mut emu = Emulator::builder().build()?;
                emu.symbol_alias("weak", "answer");
                let exporter = test_elf::TestLibrary::new(true, false);
                let expected = if exported {
                    let mut bytes = std::fs::read(&exporter.0)?;
                    // Export that name at the distinct "protect" function's address.
                    bytes[0x1266] = 1;
                    bytes[0x1268..0x1270].copy_from_slice(&0x240u64.to_le_bytes());
                    std::fs::write(&exporter.0, bytes)?;
                    let module = emu.load_library(&exporter.0)?;
                    Some(emu.find_symbol(module, "weak")?)
                } else {
                    None
                };
                let module = emu.load_library(&importer.0)?;
                let target = emu.find_symbol(module, "answer")?;
                let data = emu.find_symbol(module, "data")?;
                let bound = u64::from_le_bytes(emu.read_bytes(data + 32, 8)?.try_into().unwrap());
                assert_eq!(bound, expected.unwrap_or(target));
                if !exported {
                    assert_eq!(emu.call_function(bound, &[])?, 42);
                    assert!(emu.find_symbol(module, "weak").is_err());
                    let name = emu.new_cstring("weak")?;
                    let lookup = emu.find_symbol(module, "dlsym")?;
                    assert_eq!(emu.call_function(lookup, &[0, name])?, 0);
                }
            }
        }
        Ok(())
    }
}
