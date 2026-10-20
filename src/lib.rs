mod arch;
mod elf;
mod jni;
pub mod serve;
mod syscalls;
mod threads;
pub use jni::{Handle, JniCall, JniHandler, JniValue, PrimitiveType};

use arch::Architecture;
use elf::Image;
use std::{
    collections::HashMap,
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};
use unicorn_engine::{
    Unicorn,
    unicorn_const::{ControlType, HookType, MemType, Prot, uc_ctl, uc_error},
};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const NO_LIBRARY: &str = "no library loaded yet: the first load_library picks arm64 or arm32";
const PAGE: u64 = 4096;
const TRAMPOLINE: u64 = 0x10000;
const TRAMPOLINE_SIZE: u64 = 0x10000;
const TRAMPOLINE_STUBS: usize = 8192;
const RETURN: u64 = TRAMPOLINE + TRAMPOLINE_SIZE;
pub(crate) const MMAP_START: u64 = 0x50000000;
pub(crate) const MMAP_END: u64 = 0x60000000;
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
    path_ptr: u64,
    info_ptr: u64,
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

#[derive(Default)]
struct CpuData {
    interrupt: Option<u32>,
    memory_access: Option<MemoryAccess>,
}
struct MemoryAccess {
    kind: MemType,
    address: u64,
    size: usize,
}

pub struct Emulator {
    // Unicorn must be destroyed before the Arc-backed host pointers it uses.
    uc: Option<Unicorn<'static, CpuData>>,
    cache_size: u32,
    arch: Option<Architecture>,
    modules: Vec<Loaded>,
    regions: Vec<Region>,
    host: Vec<HostFunction>,
    overrides: HashMap<String, u64>,
    symbol_aliases: HashMap<String, String>,
    rootfs: PathBuf,
    library_dirs: Vec<PathBuf>,
    properties: HashMap<String, String>,
    verbose: bool,
    next_module: u64,
    heap: u64,
    heap_end: u64,
    brk: u64,
    free_blocks: Vec<(u64, u64)>,
    allocations: HashMap<u64, u64>,
    heap_high_water: u64,
    process_name: String,
    dlerror_buf: u64,
    dlerror_pending: bool,
    dlerror_capacity: usize,
    files: HashMap<u64, File>,
    next_fd: u64,
    jni: jni::JniState,
    threads: threads::Threads,
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
        Ok(Self {
            uc: None,
            arch: None,
            cache_size: builder.cache_size,
            modules: Vec::new(),
            regions: Vec::new(),
            host: Vec::new(),
            overrides: HashMap::new(),
            symbol_aliases: HashMap::new(),
            rootfs: builder.rootfs,
            library_dirs: Vec::new(),
            properties: builder.properties,
            verbose: builder.verbose,
            next_module: 0x10000000,
            heap: 0x40000000,
            heap_end: 0x40000000,
            brk: BRK_START,
            free_blocks: Vec::new(),
            allocations: HashMap::new(),
            heap_high_water: 0x40000000,
            process_name: builder.process_name.clone(),
            dlerror_buf: 0,
            dlerror_pending: false,
            dlerror_capacity: 256,
            files: HashMap::new(),
            next_fd: 3,
            jni: jni::JniState::default(),
            threads: threads::Threads::default(),
        })
    }
    fn setup_cpu(&mut self, arch: Architecture) -> Result<()> {
        let (cpu_arch, mode) = arch.cpu();
        let mut uc = Unicorn::new_with_data(cpu_arch, mode, CpuData::default())?;
        // Set before the first CPU access: this allocates Unicorn's TCG cache.
        let status = unsafe {
            uc_ctl(
                uc.get_handle(),
                ControlType(ControlType::TCG_BUFFER_SIZE.0 | ControlType::IO_WRITE.0 | (1 << 26)),
                self.cache_size,
            )
        };
        if status != uc_error::OK {
            return Err(status.into());
        }
        arch.enable_fp(&mut uc)?;
        uc.add_intr_hook(|uc, interrupt| {
            uc.get_data_mut().interrupt = Some(interrupt);
            let _ = uc.emu_stop();
        })?;
        uc.add_mem_hook(HookType::MEM_INVALID, 1, 0, |uc, kind, address, size, _| {
            uc.get_data_mut().memory_access = Some(MemoryAccess {
                kind,
                address,
                size,
            });
            false
        })?;
        uc.mem_map(TRAMPOLINE, TRAMPOLINE_SIZE, Prot::READ | Prot::EXEC)?;
        uc.mem_map(RETURN, PAGE, Prot::READ | Prot::EXEC)?;
        uc.mem_map(STACK, STACK_SIZE, Prot::READ | Prot::WRITE)?;
        if arch == Architecture::Arm32 {
            uc.mem_map(0xffff0000, PAGE, Prot::READ | Prot::EXEC)?;
            // Linux's cmpxchg64 occupies two slots (0xf60..0xf9f). Preserve r1
            // as well as the scratch callee-saved registers used for the values.
            for (address, words) in [
                (
                    0xffff0f60,
                    &[
                        0xe92d00f2u32, // push {r1,r4-r7}
                        0xe1c040d0,    // ldrd r4,r5,[r0]
                        0xe1c160d0,    // ldrd r6,r7,[r1]
                        0xe1b20f9f,    // retry: ldrexd r0,r1,[r2]
                        0xe0303004,    // eors r3,r0,r4
                        0x00313005,    // eorseq r3,r1,r5
                        0x01a23f96,    // strexdeq r3,r6,r7,[r2]
                        0x03330001,    // teqeq r3,#1
                        0x0afffff9,    // beq retry
                        0xe2730000,    // rsbs r0,r3,#0 (success => C set)
                        0xe8bd00f2,    // pop {r1,r4-r7}
                        0xe12fff1e,    // bx lr
                    ][..],
                ),
                (
                    0xffff0fc0,
                    &[
                        0xe1923f9f, // retry: ldrex r3,[r2]
                        0xe0533000, // subs r3,r3,r0
                        0x01823f91, // strexeq r3,r1,[r2]
                        0x03330001, // teqeq r3,#1
                        0x0afffffa, // beq retry
                        0xe2730000, // rsbs r0,r3,#0 (success => C set)
                        0xe12fff1e, // bx lr
                    ][..],
                ),
            ] {
                for (i, word) in words.iter().enumerate() {
                    uc.mem_write(address + i as u64 * 4, &word.to_le_bytes())?;
                }
            }
            // All guest accesses run on one CPU, so the memory barrier just returns.
            uc.mem_write(0xffff0fa0, &0xe12fff1eu32.to_le_bytes())?; // bx lr
            uc.mem_write(0xffff0fe0, &0xee1d0f70u32.to_le_bytes())?; // mrc p15,0,r0,c13,c0,3
            uc.mem_write(0xffff0fe4, &0xe12fff1eu32.to_le_bytes())?; // bx lr
            uc.mem_write(0xffff0ffc, &5u32.to_le_bytes())?;
        }
        self.uc = Some(uc);
        self.arch = Some(arch);
        self.setup_tls(&self.process_name.clone())?;
        let property = self.stub(HostFunction::Property)?;
        self.overrides
            .insert("__system_property_get".into(), property);
        for name in [
            "dlopen",
            "dlsym",
            "dlclose",
            "dlerror",
            "dladdr",
            "dl_iterate_phdr",
        ] {
            let address = self.stub(HostFunction::Dl(name))?;
            self.overrides.insert(name.into(), address);
        }
        self.setup_jni()?;
        self.dlerror_buf = self.malloc(self.dlerror_capacity)?;
        Ok(())
    }
    fn cpu(&self) -> Result<&Unicorn<'static, CpuData>> {
        self.uc.as_ref().ok_or_else(|| NO_LIBRARY.into())
    }
    fn cpu_mut(&mut self) -> Result<&mut Unicorn<'static, CpuData>> {
        self.uc.as_mut().ok_or_else(|| NO_LIBRARY.into())
    }
    fn setup_tls(&mut self, process: &str) -> Result<()> {
        let p = self.pointer_size();
        let tls = INITIAL_SP + 0x1000;
        let thread = tls + 0x400;
        let kab = tls + 0x800;
        let argv = tls + 0x900;
        let envp = tls + 0xa00;
        let auxv = tls + 0xb00;
        let name = tls + 0xc00;
        self.write_bytes(name, process.as_bytes())?;
        self.write_bytes(name + process.len() as u64, &[0])?;
        self.write_pointer(argv, name)?;
        self.write_pointer(kab, 1)?;
        self.write_pointer(kab + p, argv)?;
        self.write_pointer(kab + 2 * p, envp)?;
        self.write_pointer(kab + 3 * p, auxv)?;
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
            self.write_pointer(envp + i as u64 * p, cursor)?;
            self.write_bytes(cursor, value.as_bytes())?;
            cursor += value.len() as u64 + 1;
        }
        self.write_pointer(auxv, 25)?;
        self.write_pointer(auxv + p, tls + 0xe00)?;
        self.write_pointer(auxv + 2 * p, 6)?;
        self.write_pointer(auxv + 3 * p, PAGE)?;
        self.write_pointer(tls, tls)?;
        self.write_pointer(tls + p, thread)?;
        self.write_pointer(tls + 3 * p, kab)?;
        self.write_bytes(thread + 2 * p, &threads::MAIN_TID.to_le_bytes())?;
        let reg = self.architecture().tls();
        self.cpu_mut()?.reg_write(reg, tls)?;
        self.set_sp(INITIAL_SP)?;
        Ok(())
    }
    fn stub(&mut self, function: HostFunction) -> Result<u64> {
        if self.host.len() >= TRAMPOLINE_STUBS {
            return Err(
                "trampoline area full: more than 8192 host functions and unresolved symbols".into(),
            );
        }
        let address = TRAMPOLINE + self.host.len() as u64 * 8;
        self.write_bytes(address, &self.architecture().stub())?;
        self.host.push(function);
        Ok(address)
    }
    pub fn symbol_alias(&mut self, name: impl Into<String>, target: impl Into<String>) {
        self.symbol_aliases.insert(name.into(), target.into());
    }
    pub fn load_library(&mut self, path: impl AsRef<Path>) -> Result<Module> {
        let path = path.as_ref();
        if let Some(directory) = path.parent()
            && !self.library_dirs.iter().any(|loaded| loaded == directory)
        {
            self.library_dirs.push(directory.to_owned());
        }
        self.load_library_internal(path)
    }
    fn library_file(&self, name: &str) -> Option<PathBuf> {
        self.library_dirs
            .iter()
            .map(|directory| directory.join(name))
            .chain(std::iter::once(
                self.rootfs
                    .join(self.architecture().library_dir())
                    .join(name),
            ))
            .find(|path| path.is_file())
    }
    fn load_library_internal(&mut self, path: &Path) -> Result<Module> {
        let module = self.load_image(path)?;
        for i in 0..self.modules.len() {
            self.relocate(Module(i))?;
        }
        self.initialize(module)?;
        Ok(module)
    }
    fn load_image(&mut self, path: &Path) -> Result<Module> {
        let name = path
            .file_name()
            .ok_or_else(|| format!("cannot load {}: not a file path", path.display()))?
            .to_string_lossy()
            .into_owned();
        let image = Image::open(path)?;
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
        if let Some(i) = self.modules.iter().position(|m| m.name == name) {
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
                shared: !private,
            });
        }
        let id = Module(self.modules.len());
        let path_ptr = self.new_cstring(&path.to_string_lossy())?;
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
            if let Some(path) = self.library_file(needed) {
                let dep = self.load_image(&path)?;
                self.modules[id.0].dependencies.push(dep);
            } else {
                eprintln!(
                    "warning: {needed} needed by {name} not found next to the loaded libraries or in rootfs; its symbols stay unresolved"
                );
            }
        }
        Ok(id)
    }
    fn local_symbol(&self, module: Module, name: &str) -> Option<u64> {
        if let Some(value) = self.overrides.get(name) {
            return Some(*value);
        }
        let m = &self.modules[module.0];
        let symbol = &m.image.symbols[*m.image.exports.get(name)?];
        Some(if symbol.absolute {
            symbol.value
        } else {
            m.base + symbol.value
        })
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
        if module.0 >= self.modules.len() {
            return Err(format!("no module {} is loaded", module.0).into());
        }
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
            let Some((relative, implicit)) = image.arch.relocation(kind) else {
                return Err(format!(
                    "unsupported relocation type {kind} in {} at offset 0x{:x}",
                    self.modules[module.0].name, relocation.offset
                )
                .into());
            };
            let addend = if implicit {
                self.read_pointer(base + relocation.offset)? as i64
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
            self.write_pointer(base + relocation.offset, value)?;
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
            let address = self.read_pointer(base + image.init_array + i * self.pointer_size())?;
            if address != 0 && address != self.architecture().pointer_mask() {
                self.call_function(address, &[])?;
            }
        }
        Ok(())
    }
    pub fn malloc(&mut self, size: usize) -> Result<u64> {
        self.cpu()?;
        let size = (size.max(1) as u64 + 15) & !15;
        let address = if let Some(index) = self
            .free_blocks
            .iter()
            .position(|(_, block)| *block >= size)
        {
            let (address, block) = self.free_blocks.remove(index);
            if block > size {
                self.free_blocks.push((address + size, block - size));
            }
            address
        } else {
            let address = self.heap;
            self.heap = self.heap.checked_add(size).ok_or(uc_error::NOMEM)?;
            if self.heap > self.heap_end {
                let end = page_up(self.heap);
                let start = self.heap_end;
                self.cpu_mut()?
                    .mem_map(start, end - start, Prot::READ | Prot::WRITE)?;
                self.heap_end = end;
            }
            self.heap_high_water = self.heap_high_water.max(self.heap);
            address
        };
        self.allocations.insert(address, size);
        Ok(address)
    }
    pub fn free(&mut self, ptr: u64) {
        let Some(size) = self.allocations.remove(&ptr) else {
            return;
        };
        self.free_blocks.push((ptr, size));
        self.free_blocks
            .sort_unstable_by_key(|(address, _)| *address);
        let mut merged = Vec::with_capacity(self.free_blocks.len());
        for (address, size) in self.free_blocks.drain(..) {
            if let Some((last, length)) = merged.last_mut()
                && *last + *length == address
            {
                *length += size;
                continue;
            }
            merged.push((address, size));
        }
        self.free_blocks = merged;
    }
    pub fn heap_high_water_mark(&self) -> u64 {
        self.heap_high_water
    }
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

    pub fn new_cstring(&mut self, value: &str) -> Result<u64> {
        let ptr = self.malloc(value.len() + 1)?;
        self.write_bytes(ptr, value.as_bytes())?;
        self.write_bytes(ptr + value.len() as u64, &[0])?;
        Ok(ptr)
    }
    pub fn read_bytes(&self, address: u64, size: usize) -> Result<Vec<u8>> {
        Ok(self.cpu()?.mem_read_as_vec(address, size)?)
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
        self.cpu_mut()?.mem_write(address, bytes)?;
        if !bytes.is_empty() {
            self.cpu_mut()?
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
    pub fn location(&self, address: u64) -> String {
        self.modules
            .iter()
            .find(|m| address >= m.base && address < m.base + m.image.size)
            .map_or_else(
                || format!("0x{address:x}"),
                |m| format!("{}+0x{:x}", m.name, address - m.base),
            )
    }
    pub fn call_function(&mut self, function: u64, args: &[u64]) -> Result<u64> {
        self.invoke(
            function,
            args.iter().map(|&bits| (b'L', bits)),
            b'L',
            &mut (),
        )
    }
    pub(crate) fn invoke(
        &mut self,
        function: u64,
        args: impl IntoIterator<Item = (u8, u64)>,
        kind: u8,
        handler: &mut dyn JniHandler,
    ) -> Result<u64> {
        let context = if self.threads.depth > 0 {
            Some(self.cpu()?.context_init()?)
        } else {
            None
        };
        let result = (|| {
            let sp = self.place_arguments(args)?;
            let result = self
                .run(function, handler)
                .and_then(|_| self.result_bits(kind));
            self.set_sp(sp)?;
            result
        })();
        if let Some(context) = context {
            self.cpu_mut()?.context_restore(&context)?;
        }
        result
    }
    fn run(&mut self, pc: u64, handler: &mut dyn JniHandler) -> Result<u64> {
        self.threads.depth += 1;
        let result = self.run_loop(pc, handler);
        self.threads.depth -= 1;
        if self.threads.depth == 0 && result.is_err() {
            self.restore_main_thread()?;
        }
        result
    }
    fn run_loop(&mut self, mut pc: u64, handler: &mut dyn JniHandler) -> Result<u64> {
        let outermost = self.threads.depth == 1;
        loop {
            let count = if outermost && self.threads.has_runnable() {
                threads::SLICE_INSTRUCTIONS
            } else {
                0
            };
            *self.cpu_mut()?.get_data_mut() = CpuData::default();
            if let Err(error) = self.cpu_mut()?.emu_start(pc, RETURN, 0, count) {
                return Err(self.emulation_error(error));
            }
            let (restart, location) = self.restart_pc()?;
            pc = restart;
            let Some(interrupt) = self.cpu()?.get_data().interrupt else {
                if pc & !1 == RETURN {
                    if !outermost || self.threads.tid() == threads::MAIN_TID {
                        return self.reg(0);
                    }
                    return Err(format!(
                        "guest thread {} returned to RETURN at {}",
                        self.threads.tid(),
                        self.location(pc)
                    )
                    .into());
                }
                pc = self.switch_thread(pc)?;
                continue;
            };
            if interrupt != 2 {
                return Err(self.emulation_error(uc_error::EXCEPTION));
            }
            if (TRAMPOLINE..RETURN).contains(&location) {
                let function = self.host[((location - TRAMPOLINE) / 8) as usize].clone();
                let result = self.host_function(function, handler);
                // A nested callback can exit the current child. Unwind its host
                // calls before switching, even when the nested call returned an error.
                if outermost && matches!(self.threads.action, threads::Action::Exit) {
                    pc = self.switch_thread(pc)?;
                    continue;
                }
                self.write_host_result(result?)?;
            } else {
                let args = [
                    self.reg(0)?,
                    self.reg(1)?,
                    self.reg(2)?,
                    self.reg(3)?,
                    self.reg(4)?,
                    self.reg(5)?,
                ];
                let result = self.syscall(
                    self.reg(self.architecture().syscall_register())?,
                    args,
                    location,
                )?;
                self.write_reg(0, result)?;
            }
            if outermost && !matches!(self.threads.action, threads::Action::Continue) {
                pc = self.switch_thread(pc)?;
            }
        }
    }
    fn emulation_error(&self, error: uc_error) -> Box<dyn std::error::Error> {
        let pc = self.pc().unwrap_or(0);
        let lr = self.lr().unwrap_or(0);
        let mut message = format!(
            "emulation stopped: {error} at {} (pc=0x{pc:x}, lr=0x{lr:x})",
            self.location(pc)
        );
        if matches!(
            error,
            uc_error::READ_UNMAPPED
                | uc_error::WRITE_UNMAPPED
                | uc_error::FETCH_UNMAPPED
                | uc_error::READ_PROT
                | uc_error::WRITE_PROT
                | uc_error::FETCH_PROT
        ) && let Some(fault) = self
            .uc
            .as_ref()
            .and_then(|uc| uc.get_data().memory_access.as_ref())
        {
            let access = match fault.kind {
                MemType::READ_UNMAPPED | MemType::READ_PROT => "read",
                MemType::WRITE_UNMAPPED | MemType::WRITE_PROT => "write",
                _ => "instruction fetch",
            };
            message.push_str(&format!(
                ": {}-byte {access} at 0x{:x}",
                fault.size, fault.address
            ));
            if fault.address < TRAMPOLINE {
                message.push_str(", likely a null pointer");
            }
        }
        message.into()
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
                self.location(self.lr()?)
            )
            .into()),
            HostFunction::Dl(name) => self.dl_function(name),
        }
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
                let Some(name) = Path::new(&path).file_name().map(|n| n.to_string_lossy()) else {
                    self.set_dlerror(&format!("dlopen failed: library \"{path}\" not found"))?;
                    return Ok(0);
                };
                if let Some(i) = self.modules.iter().position(|m| m.name == name) {
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
                let result = self.load_library_internal(&path);
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
                    let result = self.call_function(
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
    fn protect(&mut self, start: u64, size: u64, permissions: Prot) -> Result<()> {
        let end = start + page_up(size);
        let mut regions = Vec::new();
        for region in std::mem::take(&mut self.regions) {
            let lo = start.max(region.start);
            let hi = end.min(region.end);
            if lo < hi && region.shared && permissions.0 & Prot::WRITE.0 != 0 {
                let bytes = self.read_bytes(lo, (hi - lo) as usize)?;
                self.cpu_mut()?.mem_unmap(lo, hi - lo)?;
                self.cpu_mut()?.mem_map(lo, hi - lo, permissions)?;
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
        self.cpu_mut()?
            .mem_protect(start, end - start, permissions)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/support/elf.rs"]
mod test_elf;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_faults_name_the_address_and_only_hint_at_low_null_pointers() -> Result<()> {
        for arch in [Architecture::Arm32, Architecture::Arm64] {
            let mut emu = Emulator::builder().build()?;
            emu.setup_cpu(arch)?;
            let code = RETURN + 0x100;
            let (words, size) = match arch {
                Architecture::Arm32 => ([0xe5900000u32, 0xe12fff1e], 4), // ldr r0,[r0]; bx lr
                Architecture::Arm64 => ([0xf9400000, 0xd65f03c0], 8),    // ldr x0,[x0]; ret
            };
            emu.write_bytes(
                code,
                &words
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
            )?;
            for (address, hint) in [(0x18, ", likely a null pointer"), (0x30000000, "")] {
                assert_eq!(
                    emu.call_function(code, &[address]).unwrap_err().to_string(),
                    format!(
                        "emulation stopped: Invalid memory read (UC_ERR_READ_UNMAPPED) at 0x20100 (pc=0x20100, lr=0x20000): {size}-byte read at 0x{address:x}{hint}"
                    )
                );
            }
            let value = emu.malloc(8)?;
            emu.write_u64(value, 42)?;
            assert_eq!(emu.call_function(code, &[value])?, 42);
        }
        Ok(())
    }

    #[test]
    fn memory_fault_hooks_cover_writes_fetches_and_protection_errors() -> Result<()> {
        for arch in [Architecture::Arm32, Architecture::Arm64] {
            let mut emu = Emulator::builder().build()?;
            emu.setup_cpu(arch)?;
            let code = RETURN + 0x100;
            let address = 0x30000000;
            let (read, write, trap, size) = match arch {
                Architecture::Arm32 => (0xe5900000u32, 0xe5800000u32, 0xe1200070u32, 4),
                Architecture::Arm64 => (0xf9400000, 0xf9000000, 0xd4200000, 8),
            };
            emu.write_bytes(code, &write.to_le_bytes())?;
            assert_eq!(
                emu.call_function(code, &[address]).unwrap_err().to_string(),
                format!(
                    "emulation stopped: Invalid memory write (UC_ERR_WRITE_UNMAPPED) at 0x20100 (pc=0x20100, lr=0x20000): {size}-byte write at 0x30000000"
                )
            );
            assert_eq!(
                emu.call_function(address, &[]).unwrap_err().to_string(),
                "emulation stopped: Invalid memory fetch (UC_ERR_FETCH_UNMAPPED) at 0x30000000 (pc=0x30000000, lr=0x20000): 4-byte instruction fetch at 0x30000000"
            );
            emu.cpu_mut()?.mem_map(address, PAGE, Prot::READ)?;
            assert_eq!(
                emu.call_function(code, &[address]).unwrap_err().to_string(),
                format!(
                    "emulation stopped: Write to write-protected memory (UC_ERR_WRITE_PROT) at 0x20100 (pc=0x20100, lr=0x20000): {size}-byte write at 0x30000000"
                )
            );
            assert_eq!(
                emu.call_function(address, &[]).unwrap_err().to_string(),
                "emulation stopped: Fetch from non-executable memory (UC_ERR_FETCH_PROT) at 0x30000000 (pc=0x30000000, lr=0x20000): 4-byte instruction fetch at 0x30000000"
            );
            emu.cpu_mut()?.mem_protect(address, PAGE, Prot::WRITE)?;
            emu.write_bytes(code, &read.to_le_bytes())?;
            assert_eq!(
                emu.call_function(code, &[address]).unwrap_err().to_string(),
                format!(
                    "emulation stopped: Read from non-readable memory (UC_ERR_READ_PROT) at 0x20100 (pc=0x20100, lr=0x20000): {size}-byte read at 0x30000000"
                )
            );
            emu.write_bytes(code, &trap.to_le_bytes())?;
            assert_eq!(
                emu.call_function(code, &[]).unwrap_err().to_string(),
                "emulation stopped: Unhandled CPU exception (UC_ERR_EXCEPTION) at 0x20100 (pc=0x20100, lr=0x20000)"
            );
        }
        Ok(())
    }

    #[test]
    fn rust_jni_handler_can_call_guest_code_without_clobbering_the_outer_call() -> Result<()> {
        struct Handler {
            strlen: u64,
            text: u64,
            fields: usize,
        }
        impl JniHandler for Handler {
            fn get_static_object_field(
                &mut self,
                emu: &mut Emulator,
                call: &JniCall<'_>,
            ) -> Option<Handle> {
                if call.name != "TYPE" || call.signature != "Ljava/lang/Class;" {
                    return None;
                }
                let name = match call.class {
                    "java/lang/Void" => "void",
                    "java/lang/Boolean" => "boolean",
                    "java/lang/Byte" => "byte",
                    "java/lang/Character" => "char",
                    "java/lang/Short" => "short",
                    "java/lang/Integer" => "int",
                    "java/lang/Long" => "long",
                    "java/lang/Float" => "float",
                    "java/lang/Double" => "double",
                    _ => return None,
                };
                assert_eq!(emu.call_function(self.strlen, &[self.text]).unwrap(), 6);
                self.fields += 1;
                Some(emu.class(name))
            }
            fn new_object(&mut self, emu: &mut Emulator, call: &JniCall<'_>) -> Option<Handle> {
                if (call.class, call.name, call.signature)
                    != ("java/lang/String", "<init>", "([B)V")
                {
                    return None;
                }
                Some(emu.new_string(&String::from_utf8_lossy(
                    &emu.byte_array(call.args[0].object()).ok()?,
                )))
            }
            fn call_static_object_method(
                &mut self,
                emu: &mut Emulator,
                call: &JniCall<'_>,
            ) -> Option<Handle> {
                if (call.class, call.name, call.signature)
                    == (
                        "java/lang/System",
                        "getProperty",
                        "(Ljava/lang/String;)Ljava/lang/String;",
                    )
                    && emu.string(call.args[0].object()).ok()? == "file.encoding"
                {
                    Some(emu.new_string("UTF-8"))
                } else {
                    None
                }
            }
        }
        for arch in ["arm64-v8a", "armeabi-v7a"] {
            let mut emu = Emulator::builder().build()?;
            let module = emu.load_library(format!("tests/fixtures/{arch}/libjnidispatch.so"))?;
            let mut handler = Handler {
                strlen: emu.find_symbol(module, "strlen")?,
                text: emu.new_cstring("nested")?,
                fields: 0,
            };
            assert_eq!(
                emu.call_jni_onload(module, &mut handler)?,
                0x10004,
                "{arch}"
            );
            assert_eq!(handler.fields, 9, "{arch}");
        }
        Ok(())
    }

    #[test]
    fn arm32_kuser_barrier_tls_and_version_work_without_threads() -> Result<()> {
        let mut emu = Emulator::builder().build()?;
        emu.load_library("rootfs/lib/libc.so")?;
        assert_eq!(emu.call_function(0xffff0fa0, &[42])?, 42);
        let tls = emu.cpu()?.reg_read(emu.architecture().tls())?;
        assert_eq!(emu.call_function(0xffff0fe0, &[])?, tls);
        assert_eq!(emu.read_bytes(0xffff0ffc, 4)?, 5u32.to_le_bytes());
        let region = emu
            .cpu()?
            .mem_regions()?
            .into_iter()
            .find(|region| region.begin == 0xffff0000)
            .unwrap();
        assert_eq!(region.end, 0xffff0fff);
        assert_eq!(region.perms, (Prot::READ | Prot::EXEC).0);
        Ok(())
    }

    #[test]
    fn arm32_kuser_cmpxchg_results_carry_and_registers() -> Result<()> {
        use unicorn_engine::RegisterARM;

        let mut emu = Emulator::builder().build()?;
        emu.load_library("rootfs/lib/libc.so")?;
        let old = 0x1234_5678_89ab_cdefu64;
        let new = 0xfedc_ba98_7654_3210u64;
        let old_ptr = emu.malloc(8)?;
        let new_ptr = emu.malloc(8)?;
        let target = emu.malloc(16)?;
        emu.write_u64(old_ptr, old)?;
        emu.write_u64(new_ptr, new)?;
        for (helper, width) in [(0xffff0fc0, 4), (0xffff0f60, 8)] {
            let args = if width == 4 {
                [old as u32 as u64, new as u32 as u64, target]
            } else {
                [old_ptr, new_ptr, target]
            };
            let mismatches: &[u64] = if width == 4 {
                &[0, 1]
            } else {
                &[0, 1, 1 << 32]
            };
            for &mismatch in mismatches {
                let before = old ^ mismatch;
                emu.write_bytes(target, &[0xa5; 16])?;
                emu.write_bytes(target, &before.to_le_bytes()[..width])?;
                for reg in 4..12 {
                    emu.write_reg(reg, 0x12340000 + reg as u64)?;
                }
                let sp = emu.sp()?;
                let cpsr = emu.cpu()?.reg_read(RegisterARM::CPSR)?;
                // Start with the opposite C flag, so both outcomes must set it.
                emu.cpu_mut()?.reg_write(
                    RegisterARM::CPSR,
                    (cpsr & !(1 << 29)) | if mismatch == 0 { 0 } else { 1 << 29 },
                )?;
                let result = emu.call_function(helper, &args)?;
                assert_eq!(result == 0, mismatch == 0, "helper=0x{helper:x}");
                assert_eq!(
                    emu.cpu()?.reg_read(RegisterARM::CPSR)? & (1 << 29) != 0,
                    mismatch == 0
                );
                let expected = if mismatch == 0 { new } else { before };
                assert_eq!(
                    emu.read_bytes(target, width)?,
                    expected.to_le_bytes()[..width]
                );
                assert_eq!(
                    emu.read_bytes(target + width as u64, 16 - width)?,
                    vec![0xa5; 16 - width]
                );
                assert_eq!(emu.reg(1)?, args[1]);
                assert_eq!(emu.reg(2)?, target);
                for reg in 4..12 {
                    assert_eq!(emu.reg(reg)?, 0x12340000 + reg as u64);
                }
                assert_eq!(emu.sp()?, sp);
                assert_eq!(emu.lr()?, RETURN);
                assert_eq!(emu.read_u64(old_ptr)?, old);
                assert_eq!(emu.read_u64(new_ptr)?, new);
            }
        }
        Ok(())
    }

    #[test]
    fn symbol_alias_binds_missing_imports_but_preserves_exports() -> Result<()> {
        for binding in [0x10, 0x20] {
            for exported in [false, true] {
                let importer = test_elf::TestLibrary::new(false);
                let mut bytes = std::fs::read(&importer.0)?;
                // The synthetic ELF's fourth symbol is an undefined import named "weak".
                bytes[0x1264] = binding;
                std::fs::write(&importer.0, bytes)?;
                let mut emu = Emulator::builder().build()?;
                emu.symbol_alias("weak", "answer");
                let exporter = test_elf::TestLibrary::new(false);
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
