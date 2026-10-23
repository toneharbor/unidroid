use crate::{Emulator, JniHandler, Module, Result};
use std::path::Path;

impl Emulator {
    fn set_dlerror(&mut self, message: &str) -> Result<()> {
        if self.dlerror_buf == 0 {
            self.dlerror_buf = self.malloc(self.dlerror_capacity)?;
        }
        let bytes = message.as_bytes();
        let length = bytes.len().min(self.dlerror_capacity.saturating_sub(1));
        self.write_bytes(self.dlerror_buf, &bytes[..length])?;
        self.write_bytes(self.dlerror_buf + length as u64, &[0])?;
        self.dlerror_pending = true;
        Ok(())
    }
    fn dlerror(&mut self) -> u64 {
        if self.dlerror_pending {
            self.dlerror_pending = false;
            self.dlerror_buf
        } else {
            0
        }
    }
    fn invalid_handle_error(&mut self, handle: u64) -> Result<()> {
        self.set_dlerror(&format!("invalid handle: 0x{handle:x}"))
    }

    pub(super) fn dl_function(&mut self, name: &str, handler: &mut dyn JniHandler) -> Result<u64> {
        let a = self.reg(0)?;
        let b = self.reg(1)?;
        match name {
            "dlopen" => {
                if a == 0 {
                    self.modules[0].dl_references += 1;
                    return Ok(1);
                }
                let path = self.cstring(a)?;
                let Some(name) = Path::new(&path).file_name().map(|n| n.to_string_lossy()) else {
                    self.set_dlerror(&format!("dlopen failed: library \"{path}\" not found"))?;
                    return Ok(0);
                };
                if let Some(i) = self.modules.iter().position(|m| m.name == name) {
                    self.run_constructors(Module(i), handler)?;
                    self.modules[i].dl_references += 1;
                    return Ok(i as u64 + 1);
                }
                let full = self.rootfs.join(path.trim_start_matches('/'));
                let path = if !path.contains("..") && full.is_file() {
                    Some(full)
                } else {
                    self.library_file(&name)
                };
                let Some(path) = path else {
                    self.set_dlerror(&format!("dlopen failed: library \"{name}\" not found"))?;
                    return Ok(0);
                };
                let context = self.cpu()?.context_init()?;
                let result = self.load_library_internal(&path).and_then(|module| {
                    self.run_constructors(module, handler)?;
                    Ok(module)
                });
                self.cpu_mut()?.context_restore(&context)?;
                let module = result?;
                self.modules[module.0].dl_references += 1;
                Ok(module.0 as u64 + 1)
            }
            "dlsym" => {
                let symbol = self.cstring(b)?;
                let value = if a == self.architecture().rtld_default() {
                    (0..self.modules.len()).find_map(|i| self.local_symbol(Module(i), &symbol))
                } else if a == self.architecture().rtld_next() {
                    let caller = self.lr()?;
                    let start = self
                        .modules
                        .iter()
                        .position(|m| caller >= m.base && caller < m.base + m.image.size)
                        .map_or(0, |i| i + 1);
                    (start..self.modules.len()).find_map(|i| self.local_symbol(Module(i), &symbol))
                } else if a != 0
                    && a <= self.modules.len() as u64
                    && self.modules[(a - 1) as usize].dl_references != 0
                {
                    self.search_symbol(Module((a - 1) as usize), &symbol, &mut Vec::new())
                } else {
                    None
                };
                if value.is_none() {
                    self.set_dlerror(&format!("undefined symbol: {symbol}"))?;
                }
                Ok(value.unwrap_or(0))
            }
            "dlclose" => {
                if a != 0
                    && a <= self.modules.len() as u64
                    && self.modules[(a - 1) as usize].dl_references != 0
                {
                    // The loader itself owns a lifetime reference to every module.
                    self.modules[(a - 1) as usize].dl_references -= 1;
                    Ok(0)
                } else {
                    self.invalid_handle_error(a)?;
                    Ok(u64::MAX)
                }
            }
            "dlerror" => Ok(self.dlerror()),
            "dladdr" => {
                let Some(i) = self
                    .modules
                    .iter()
                    .position(|m| a >= m.base && a < m.base + m.image.size)
                else {
                    return Ok(0);
                };
                let path = self.modules[i].path_ptr;
                let p = self.pointer_size();
                self.write_pointer(b, path)?;
                self.write_pointer(b + p, self.modules[i].base)?;
                let symbol = self.modules[i]
                    .image
                    .symbols
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.defined && s.value <= a - self.modules[i].base)
                    .max_by_key(|(_, s)| s.value)
                    .map(|(_, s)| {
                        (
                            self.modules[i].base
                                + self.modules[i].image.dynstr
                                + s.name_offset as u64,
                            self.modules[i].base + s.value,
                        )
                    });
                let (name, address) = symbol.unwrap_or((0, 0));
                self.write_pointer(b + 2 * p, name)?;
                self.write_pointer(b + 3 * p, address)?;
                Ok(1)
            }
            "dl_iterate_phdr" => {
                for i in 0..self.modules.len() {
                    let result = self.call_function_with(
                        handler,
                        a,
                        &[self.modules[i].info_ptr, 4 * self.pointer_size(), b],
                    )?;
                    if result != 0 {
                        return Ok(result);
                    }
                }
                Ok(0)
            }
            _ => unreachable!(),
        }
    }
}
