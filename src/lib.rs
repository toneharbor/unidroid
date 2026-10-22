mod arch;
mod debug;
mod dl;
mod elf;
mod jni;
mod loader;
mod memory;
mod policy;
pub use policy::{Policy, PolicyHandler};
mod probe;
pub use probe::{Probe, ProbedCall};
mod run;
pub mod serve;
mod source;
mod syscalls;
mod threads;
pub use debug::{
    Crash, Frame,
    control::{Resume, Stop, StopReason},
    coverage::BlockCount,
    crypto::CryptoConstant,
    events::{Event, MemoryAccess, Trace},
    maps::Mapping,
    modules::ModuleInfo,
    search::SEARCH_LIMIT,
    snapshot::{Change, Diff, Snapshot},
    watch::{Watch, WatchAction, WatchHit},
};
pub use jni::{Handle, JniCall, JniHandler, JniValue, PrimitiveType};

use arch::Architecture;
use elf::Image;
use std::{collections::HashMap, path::PathBuf, sync::Arc};
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
const HOST_MALLOC_START: u64 = 0x40000000;
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
    app: bool,
    path: PathBuf,
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
    trace: Trace,
    on_event: Option<debug::events::EventHandler>,
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
    pub fn trace(mut self, trace: Trace) -> Self {
        self.trace = trace;
        self
    }
    pub fn on_event(mut self, f: impl FnMut(&Event) + 'static) -> Self {
        self.on_event = Some(Box::new(f));
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
    memory_access: Option<MemoryFault>,
    slice_remaining: u64,
    thread: u32,
    depth: usize,
    import: Option<u64>,
    stop: Option<StopReason>,
    watch: Option<(WatchHit, WatchAction)>,
    step_started: bool,
    skip_breakpoint: Option<(u32, u64)>,
    finish: Option<debug::control::FinishTarget>,
}
struct MemoryFault {
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
    trace: Trace,
    events: std::rc::Rc<std::cell::RefCell<debug::events::Events>>,
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
    files: HashMap<u64, syscalls::GuestFile>,
    source: source::Source,
    next_fd: u64,
    jni: jni::JniState,
    threads: threads::Threads,
    imports: debug::imports::Imports,
    code: debug::code::Code,
    calls: Option<debug::calls::Calls>,
    coverage: debug::coverage::Coverage,
    breakpoints: HashMap<u64, unicorn_engine::UcHookId>,
    watchpoints: debug::watch::Watchpoints,
}
impl Emulator {
    pub fn builder() -> EmulatorBuilder {
        EmulatorBuilder {
            rootfs: "rootfs".into(),
            process_name: "unidroid".into(),
            properties: HashMap::from([("ro.build.version.sdk".into(), "23".into())]),
            trace: Trace::default(),
            on_event: None,
            cache_size: 8 * 1024 * 1024,
        }
    }
    fn build(builder: EmulatorBuilder) -> Result<Self> {
        for spec in builder
            .trace
            .code
            .iter()
            .chain(&builder.trace.calls)
            .chain(&builder.trace.coverage)
        {
            debug::code::Spec::parse(spec)?;
        }
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
            trace: builder.trace,
            events: std::rc::Rc::new(std::cell::RefCell::new(debug::events::Events {
                handler: builder.on_event,
            })),
            next_module: 0x10000000,
            heap: HOST_MALLOC_START,
            heap_end: HOST_MALLOC_START,
            brk: BRK_START,
            free_blocks: Vec::new(),
            allocations: HashMap::new(),
            heap_high_water: HOST_MALLOC_START,
            process_name: builder.process_name.clone(),
            dlerror_buf: 0,
            dlerror_pending: false,
            dlerror_capacity: 256,
            files: HashMap::new(),
            source: source::Source::default(),
            next_fd: 3,
            jni: jni::JniState::default(),
            threads: threads::Threads::default(),
            imports: debug::imports::Imports::default(),
            code: debug::code::Code::default(),
            calls: None,
            coverage: debug::coverage::Coverage::default(),
            breakpoints: HashMap::new(),
            watchpoints: debug::watch::Watchpoints::default(),
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
            uc.get_data_mut().memory_access = Some(MemoryFault {
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
        self.trace_imports(self.trace.imports)?;
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
}

#[cfg(test)]
#[path = "../tests/support/elf.rs"]
mod test_elf;

#[cfg(test)]
mod tests {
    use super::*;

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
}
