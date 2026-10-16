mod elf;
mod jni;
mod syscalls;
pub use jni::{Handle, JniCall, JniHandler, JniValue, PrimitiveType};

use elf::Image;
use std::{
    collections::HashMap,
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};
use unicorn_engine::{
    RegisterARM64 as Reg, Unicorn,
    unicorn_const::{Arch, ControlType, Mode, Prot, uc_ctl, uc_error},
};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const PAGE: u64 = 4096;
const TRAMPOLINE: u64 = 0x10000;
const RETURN: u64 = TRAMPOLINE + PAGE;
const STACK: u64 = 0x70000000;
const STACK_SIZE: u64 = 0x200000;
const INITIAL_SP: u64 = STACK + STACK_SIZE - 0x10000;
const BRK_START: u64 = 0x60000000;
pub(crate) fn page_up(n: u64) -> u64 {
    (n + PAGE - 1) & !(PAGE - 1)
}
fn page_down(n: u64) -> u64 {
    n & !(PAGE - 1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Module(pub usize);
struct Loaded {
    name: String,
    image: Arc<Image>,
    base: u64,
    dependencies: Vec<Module>,
    relocated: bool,
    initialized: bool,
    dl_references: usize,
}
#[derive(Clone)]
struct Region {
    start: u64,
    end: u64,
    shared: bool,
}
#[derive(Clone)]
enum HostFunction {
    Jni(usize),
    Vm(usize),
    Property,
    Dl(&'static str),
    Unresolved { symbol: String, module: String },
}

pub struct EmulatorBuilder {
    rootfs: PathBuf,
    process_name: String,
    properties: HashMap<String, String>,
    verbose: bool,
    cache_size: u32,
}
impl EmulatorBuilder {
    pub fn rootfs(mut self, path: impl Into<PathBuf>) -> Self {
        self.rootfs = path.into();
        self
    }
    pub fn process_name(mut self, name: impl Into<String>) -> Self {
        self.process_name = name.into();
        self
    }
    pub fn system_property(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.properties.insert(key.into(), value.into());
        self
    }
    pub fn verbose(mut self, value: bool) -> Self {
        self.verbose = value;
        self
    }
    pub fn translation_cache_size(mut self, bytes: u32) -> Self {
        self.cache_size = bytes;
        self
    }
    pub fn build(self) -> Result<Emulator> {
        Emulator::build(self)
    }
}

pub struct Emulator {
    // Unicorn must be destroyed before the Arc-backed host pointers it uses.
    uc: Unicorn<'static, Option<u32>>,
    modules: Vec<Loaded>,
    regions: Vec<Region>,
    host: Vec<HostFunction>,
    overrides: HashMap<String, u64>,
    rootfs: PathBuf,
    properties: HashMap<String, String>,
    verbose: bool,
    next_module: u64,
    heap: u64,
    heap_end: u64,
    next_mmap: u64,
    brk: u64,
    files: HashMap<u64, File>,
    next_fd: u64,
    dlerror: u64,
    jni: jni::JniState,
}
impl Emulator {
    pub fn builder() -> EmulatorBuilder {
        EmulatorBuilder {
            rootfs: "rootfs".into(),
            process_name: "minidbg".into(),
            properties: HashMap::from([("ro.build.version.sdk".into(), "23".into())]),
            verbose: false,
            cache_size: 1024 * 1024,
        }
    }
    fn build(builder: EmulatorBuilder) -> Result<Self> {
        let mut uc = Unicorn::new_with_data(Arch::ARM64, Mode::LITTLE_ENDIAN, None)?;
        // Set before the first CPU access: this allocates Unicorn's TCG cache.
        let status = unsafe {
            uc_ctl(
                uc.get_handle(),
                ControlType(ControlType::TCG_BUFFER_SIZE.0 | ControlType::IO_WRITE.0 | (1 << 26)),
                builder.cache_size,
            )
        };
        if status != uc_error::OK {
            return Err(status.into());
        }
        uc.reg_write(Reg::CPACR_EL1, 3 << 20)?;
        uc.add_intr_hook(|uc, interrupt| {
            *uc.get_data_mut() = Some(interrupt);
            let _ = uc.emu_stop();
        })?;
        uc.mem_map(TRAMPOLINE, PAGE, Prot::READ | Prot::EXEC)?;
        uc.mem_map(STACK, STACK_SIZE, Prot::READ | Prot::WRITE)?;
        let mut emu = Self {
            uc,
            modules: Vec::new(),
            regions: Vec::new(),
            host: Vec::new(),
            overrides: HashMap::new(),
            rootfs: builder.rootfs,
            properties: builder.properties,
            verbose: builder.verbose,
            next_module: 0x10000000,
            heap: 0x40000000,
            heap_end: 0x40000000,
            next_mmap: 0x50000000,
            brk: BRK_START,
            files: HashMap::new(),
            next_fd: 3,
            dlerror: 0,
            jni: jni::JniState::default(),
        };
        emu.setup_tls(&builder.process_name)?;
        let property = emu.stub(HostFunction::Property)?;
        emu.overrides
            .insert("__system_property_get".into(), property);
        for name in [
            "dlopen",
            "dlsym",
            "dlclose",
            "dlerror",
            "dladdr",
            "dl_iterate_phdr",
        ] {
            let address = emu.stub(HostFunction::Dl(name))?;
            emu.overrides.insert(name.into(), address);
        }
        emu.setup_jni()?;
        Ok(emu)
    }
    fn setup_tls(&mut self, process: &str) -> Result<()> {
        let tls = INITIAL_SP + 0x1000;
        let thread = tls + 0x400;
        let kab = tls + 0x800;
        let argv = tls + 0x900;
        let envp = tls + 0xa00;
        let auxv = tls + 0xb00;
        let name = tls + 0xc00;
        self.write_bytes(name, process.as_bytes())?;
        self.write_bytes(name + process.len() as u64, &[0])?;
        self.write_u64(argv, name)?;
        self.write_u64(kab, 1)?;
        self.write_u64(kab + 8, argv)?;
        self.write_u64(kab + 16, envp)?;
        self.write_u64(kab + 24, auxv)?;
        let mut cursor = name + page_up(process.len() as u64 + 1);
        for (i, value) in [
            "ANDROID_DATA=/data",
            "ANDROID_ROOT=/system",
            "PATH=/sbin:/vendor/bin:/system/sbin:/system/bin:/system/xbin",
            "NO_ADDR_COMPAT_LAYOUT_FIXUP=1",
        ]
        .iter()
        .enumerate()
        {
            self.write_u64(envp + i as u64 * 8, cursor)?;
            self.write_bytes(cursor, value.as_bytes())?;
            cursor += value.len() as u64 + 1;
        }
        self.write_u64(auxv, 25)?;
        self.write_u64(auxv + 8, tls + 0xe00)?;
        self.write_u64(auxv + 16, 6)?;
        self.write_u64(auxv + 24, PAGE)?;
        self.write_u64(tls, tls)?;
        self.write_u64(tls + 8, thread)?;
        self.write_u64(tls + 24, kab)?;
        self.write_bytes(thread + 16, &1234u32.to_le_bytes())?;
        self.uc.reg_write(Reg::TPIDR_EL0, tls)?;
        self.uc.reg_write(Reg::SP, INITIAL_SP)?;
        Ok(())
    }
    fn stub(&mut self, function: HostFunction) -> Result<u64> {
        let address = TRAMPOLINE + self.host.len() as u64 * 8;
        if address + 8 > RETURN {
            return Err(uc_error::NOMEM.into());
        }
        self.write_bytes(address, &[1, 0, 0, 0xd4, 0xc0, 3, 0x5f, 0xd6])?;
        self.host.push(function);
        Ok(address)
    }
    pub fn load_library(&mut self, path: impl AsRef<Path>) -> Result<Module> {
        let module = self.load_image(path.as_ref())?;
        for i in 0..self.modules.len() {
            self.relocate(Module(i))?;
        }
        self.initialize(module)?;
        Ok(module)
    }
    fn load_image(&mut self, path: &Path) -> Result<Module> {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if let Some(i) = self.modules.iter().position(|m| m.name == name) {
            return Ok(Module(i));
        }
        let image = Image::open(path)?;
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
                self.uc.mem_map(start, end - start, permissions)?;
                let offset = segment.p_offset as usize;
                self.write_bytes(
                    base + segment.p_vaddr,
                    &image.bytes[offset..offset + segment.p_filesz as usize],
                )?;
            } else {
                let file_offset = page_down(segment.p_offset) as usize;
                // The Arc keeps this read-only mmap alive until after uc_close.
                unsafe {
                    self.uc.mem_map_ptr(
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
                shared: !private,
            });
        }
        let id = Module(self.modules.len());
        self.modules.push(Loaded {
            name: name.clone(),
            image: image.clone(),
            base,
            dependencies: Vec::new(),
            relocated: false,
            initialized: false,
            dl_references: 0,
        });
        for needed in &image.needed {
            let path = self.rootfs.join("lib64").join(needed);
            if path.is_file() {
                let dep = self.load_image(&path)?;
                self.modules[id.0].dependencies.push(dep);
            } else {
                eprintln!(
                    "warning: {needed} needed by {name} not found in rootfs; its symbols stay unresolved"
                );
            }
        }
        Ok(id)
    }
    fn local_symbol(&self, module: Module, name: &str) -> Option<u64> {
        let m = &self.modules[module.0];
        let symbol = &m.image.symbols[*m.image.exports.get(name)?];
        Some(
            self.overrides
                .get(name)
                .copied()
                .unwrap_or(if symbol.absolute {
                    symbol.value
                } else {
                    m.base + symbol.value
                }),
        )
    }
    fn search_symbol(&self, module: Module, name: &str, visited: &mut Vec<Module>) -> Option<u64> {
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
        self.search_symbol(module, name, &mut Vec::new())
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound).into())
    }
    fn relocate(&mut self, module: Module) -> Result<()> {
        if self.modules[module.0].relocated {
            return Ok(());
        }
        let image = self.modules[module.0].image.clone();
        let base = self.modules[module.0].base;
        let mut unresolved = HashMap::new();
        for relocation in &image.relocations {
            let kind = relocation.info as u32;
            if kind == 0 {
                continue;
            }
            let value = if kind == 1027 {
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
                } else if let Some(value) =
                    (0..self.modules.len()).find_map(|i| self.local_symbol(Module(i), &symbol.name))
                {
                    value
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
            .wrapping_add_signed(relocation.addend);
            match kind {
                257 | 1025..=1027 => self.write_u64(base + relocation.offset, value)?,
                _ => return Err(uc_error::ARG.into()),
            }
        }
        self.modules[module.0].relocated = true;
        Ok(())
    }
    fn initialize(&mut self, module: Module) -> Result<()> {
        if self.modules[module.0].initialized {
            return Ok(());
        }
        self.modules[module.0].initialized = true;
        for dep in self.modules[module.0].dependencies.clone() {
            self.initialize(dep)?;
        }
        let image = self.modules[module.0].image.clone();
        let base = self.modules[module.0].base;
        if image.init != 0 {
            self.call_function(base + image.init, &[])?;
        }
        for i in 0..image.init_count {
            let address = self.read_u64(base + image.init_array + i * 8)?;
            if address != 0 && address != u64::MAX {
                self.call_function(address, &[])?;
            }
        }
        Ok(())
    }
    pub fn malloc(&mut self, size: usize) -> Result<u64> {
        let address = self.heap;
        self.heap += (size.max(1) as u64 + 15) & !15;
        if self.heap > self.heap_end {
            let end = page_up(self.heap);
            self.uc
                .mem_map(self.heap_end, end - self.heap_end, Prot::READ | Prot::WRITE)?;
            self.heap_end = end;
        }
        Ok(address)
    }
    pub fn new_cstring(&mut self, value: &str) -> Result<u64> {
        let ptr = self.malloc(value.len() + 1)?;
        self.write_bytes(ptr, value.as_bytes())?;
        self.write_bytes(ptr + value.len() as u64, &[0])?;
        Ok(ptr)
    }
    pub fn read_bytes(&self, address: u64, size: usize) -> Result<Vec<u8>> {
        Ok(self.uc.mem_read_as_vec(address, size)?)
    }
    pub fn write_bytes(&mut self, address: u64, bytes: &[u8]) -> Result<()> {
        // Unicorn's host write API bypasses guest permissions. Never let it
        // write through a read-only host mmap; guest mprotect privatizes first.
        if self
            .regions
            .iter()
            .any(|r| r.shared && address < r.end && address + bytes.len() as u64 > r.start)
        {
            return Err(uc_error::WRITE_PROT.into());
        }
        self.uc.mem_write(address, bytes)?;
        if !bytes.is_empty() {
            self.uc
                .ctl_remove_cache(address, address + bytes.len() as u64)?;
        }
        Ok(())
    }
    fn read_u64(&self, address: u64) -> Result<u64> {
        Ok(u64::from_le_bytes(
            self.read_bytes(address, 8)?.try_into().unwrap(),
        ))
    }
    fn write_u64(&mut self, address: u64, value: u64) -> Result<()> {
        self.write_bytes(address, &value.to_le_bytes())
    }
    fn cstring(&self, address: u64) -> Result<String> {
        let mut value = Vec::new();
        let mut p = address;
        loop {
            let byte = self.read_bytes(p, 1)?[0];
            if byte == 0 {
                break;
            }
            value.push(byte);
            p += 1;
        }
        Ok(String::from_utf8_lossy(&value).into_owned())
    }
    fn reg(&self, index: usize) -> Result<u64> {
        Ok(self.uc.reg_read(Reg::X0 as i32 + index as i32)?)
    }
    fn location(&self, address: u64) -> String {
        self.modules
            .iter()
            .find(|m| address >= m.base && address < m.base + m.image.size)
            .map_or_else(
                || format!("0x{address:x}"),
                |m| format!("{}+0x{:x}", m.name, address - m.base),
            )
    }
    pub fn call_function(&mut self, function: u64, args: &[u64]) -> Result<u64> {
        let sp = self.uc.reg_read(Reg::SP)?;
        let call_sp = (sp - args.len().saturating_sub(8) as u64 * 8) & !15;
        for (i, value) in args.iter().enumerate() {
            if i < 8 {
                self.uc.reg_write(Reg::X0 as i32 + i as i32, *value)?;
            } else {
                self.write_u64(call_sp + (i - 8) as u64 * 8, *value)?;
            }
        }
        self.uc.reg_write(Reg::SP, call_sp)?;
        self.uc.reg_write(Reg::LR, RETURN)?;
        let result = self.run(function, &mut ());
        self.uc.reg_write(Reg::SP, sp)?;
        result
    }
    fn run(&mut self, mut pc: u64, handler: &mut dyn JniHandler) -> Result<u64> {
        loop {
            *self.uc.get_data_mut() = None;
            if let Err(error) = self.uc.emu_start(pc, RETURN, 0, 0) {
                return Err(self.emulation_error(error));
            }
            pc = self.uc.reg_read(Reg::PC)?;
            let Some(interrupt) = *self.uc.get_data() else {
                return self.reg(0);
            };
            if interrupt != 2 {
                return Err(self.emulation_error(uc_error::EXCEPTION));
            }
            let result = if (TRAMPOLINE..RETURN).contains(&pc.saturating_sub(4)) {
                let function = self.host[((pc - 4 - TRAMPOLINE) / 8) as usize].clone();
                self.host_function(function, handler)?
            } else {
                let args = [
                    self.reg(0)?,
                    self.reg(1)?,
                    self.reg(2)?,
                    self.reg(3)?,
                    self.reg(4)?,
                    self.reg(5)?,
                ];
                self.syscall(self.reg(8)?, args, pc - 4)?
            };
            self.uc.reg_write(Reg::X0, result)?;
        }
    }
    fn emulation_error(&self, error: uc_error) -> Box<dyn std::error::Error> {
        let pc = self.uc.reg_read(Reg::PC).unwrap_or(0);
        let lr = self.uc.reg_read(Reg::LR).unwrap_or(0);
        format!(
            "emulation stopped: {error} at {} (pc=0x{pc:x}, lr=0x{lr:x})",
            self.location(pc)
        )
        .into()
    }
    fn host_function(
        &mut self,
        function: HostFunction,
        handler: &mut dyn JniHandler,
    ) -> Result<u64> {
        match function {
            HostFunction::Jni(index) => self.jni_function(index, handler),
            HostFunction::Vm(index) => self.vm_function(index),
            HostFunction::Property => {
                let key = self.cstring(self.reg(0)?)?;
                let value = self.properties.get(&key).cloned().unwrap_or_default();
                let out = self.reg(1)?;
                self.write_bytes(out, value.as_bytes())?;
                self.write_bytes(out + value.len() as u64, &[0])?;
                Ok(value.len() as u64)
            }
            HostFunction::Unresolved { symbol, module } => Err(format!(
                "call to unresolved symbol {symbol} imported by {module} from {}",
                self.location(self.uc.reg_read(Reg::LR)?)
            )
            .into()),
            HostFunction::Dl(name) => self.dl_function(name),
        }
    }
    fn nested_call(&mut self, function: u64, args: &[u64]) -> Result<u64> {
        let context = self.uc.context_init()?;
        let value = self.call_function(function, args);
        self.uc.context_restore(&context)?;
        value
    }
    fn dl_function(&mut self, name: &str) -> Result<u64> {
        let a = self.reg(0)?;
        let b = self.reg(1)?;
        match name {
            "dlopen" => {
                if a == 0 {
                    self.modules[0].dl_references += 1;
                    return Ok(1);
                }
                let path = self.cstring(a)?;
                let name = Path::new(&path).file_name().unwrap().to_string_lossy();
                if let Some(i) = self.modules.iter().position(|m| m.name == name) {
                    self.modules[i].dl_references += 1;
                    return Ok(i as u64 + 1);
                }
                let path = self.rootfs.join("lib64").join(name.as_ref());
                if !path.is_file() {
                    self.dlerror = self.new_cstring(&name)?;
                    return Ok(0);
                }
                let context = self.uc.context_init()?;
                let result = self.load_library(path);
                self.uc.context_restore(&context)?;
                let module = result?;
                self.modules[module.0].dl_references += 1;
                Ok(module.0 as u64 + 1)
            }
            "dlsym" => {
                let symbol = self.cstring(b)?;
                let value = if a == 0 {
                    (0..self.modules.len()).find_map(|i| self.local_symbol(Module(i), &symbol))
                } else if a == u64::MAX {
                    let caller = self.uc.reg_read(Reg::LR)?;
                    let start = self
                        .modules
                        .iter()
                        .position(|m| caller >= m.base && caller < m.base + m.image.size)
                        .map_or(0, |i| i + 1);
                    (start..self.modules.len()).find_map(|i| self.local_symbol(Module(i), &symbol))
                } else if a <= self.modules.len() as u64
                    && self.modules[(a - 1) as usize].dl_references != 0
                {
                    self.search_symbol(Module((a - 1) as usize), &symbol, &mut Vec::new())
                } else {
                    None
                };
                if value.is_none() {
                    self.dlerror = self.new_cstring(&symbol)?;
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
                    self.dlerror = self.new_cstring(&format!("0x{a:x}"))?;
                    Ok(u64::MAX)
                }
            }
            "dlerror" => Ok(std::mem::take(&mut self.dlerror)),
            "dladdr" => {
                let Some(i) = self
                    .modules
                    .iter()
                    .position(|m| a >= m.base && a < m.base + m.image.size)
                else {
                    return Ok(0);
                };
                let name = self.modules[i].name.clone();
                let path = self.new_cstring(&name)?;
                self.write_u64(b, path)?;
                self.write_u64(b + 8, self.modules[i].base)?;
                let symbol = self.modules[i]
                    .image
                    .symbols
                    .iter()
                    .filter(|s| s.defined && s.value <= a - self.modules[i].base)
                    .max_by_key(|s| s.value)
                    .map(|s| (s.name.clone(), s.value));
                let (name, address) = if let Some((name, value)) = symbol {
                    (self.new_cstring(&name)?, self.modules[i].base + value)
                } else {
                    (0, 0)
                };
                self.write_u64(b + 16, name)?;
                self.write_u64(b + 24, address)?;
                Ok(1)
            }
            "dl_iterate_phdr" => {
                let info = self.malloc(32)?;
                for i in 0..self.modules.len() {
                    let name = self.modules[i].name.clone();
                    let name = self.new_cstring(&name)?;
                    self.write_u64(info, self.modules[i].base)?;
                    self.write_u64(info + 8, name)?;
                    let m = &self.modules[i];
                    let first = &m.image.segments[0];
                    self.write_u64(
                        info + 16,
                        m.base + first.p_vaddr - first.p_offset + m.image.phoff,
                    )?;
                    self.write_u64(info + 24, self.modules[i].image.phnum as u64)?;
                    let result = self.nested_call(a, &[info, 32, b])?;
                    if result != 0 {
                        return Ok(result);
                    }
                }
                Ok(0)
            }
            _ => unreachable!(),
        }
    }
    fn protect(&mut self, start: u64, size: u64, permissions: Prot) -> Result<()> {
        let end = start + page_up(size);
        let mut regions = Vec::new();
        for region in std::mem::take(&mut self.regions) {
            let lo = start.max(region.start);
            let hi = end.min(region.end);
            if lo < hi && region.shared && permissions.0 & Prot::WRITE.0 != 0 {
                let bytes = self.read_bytes(lo, (hi - lo) as usize)?;
                self.uc.mem_unmap(lo, hi - lo)?;
                self.uc.mem_map(lo, hi - lo, permissions)?;
                self.write_bytes(lo, &bytes)?;
                if region.start < lo {
                    regions.push(Region {
                        end: lo,
                        ..region.clone()
                    });
                }
                regions.push(Region {
                    start: lo,
                    end: hi,
                    shared: false,
                });
                if hi < region.end {
                    regions.push(Region {
                        start: hi,
                        ..region
                    });
                }
            } else {
                regions.push(region);
            }
        }
        self.regions = regions;
        self.uc.mem_protect(start, end - start, permissions)?;
        Ok(())
    }
}
