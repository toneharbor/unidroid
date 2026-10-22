use crate::{
    BRK_START, Emulator, Event, HOST_MALLOC_START, Result, Trace, debug, jni, source, threads,
};
use std::{collections::HashMap, path::PathBuf};

pub struct EmulatorBuilder {
    rootfs: PathBuf,
    process_name: String,
    properties: HashMap<String, String>,
    trace: Trace,
    on_event: Option<debug::events::EventHandler>,
    cache_size: u32,
    fixed_clock: Option<i64>,
    random_seed: Option<u64>,
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
    /// Start realtime at these Unix seconds; all clock reads advance by one microsecond.
    pub fn fixed_clock(mut self, unix_seconds: i64) -> Self {
        self.fixed_clock = Some(unix_seconds);
        self
    }
    /// Use a reproducible SplitMix64 byte stream for guest random reads.
    pub fn random_seed(mut self, seed: u64) -> Self {
        self.random_seed = Some(seed);
        self
    }
    pub fn build(self) -> Result<Emulator> {
        Emulator::build(self)
    }
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
            fixed_clock: None,
            random_seed: None,
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
            source: source::Source::new(builder.fixed_clock, builder.random_seed),
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
}
