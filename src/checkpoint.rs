use crate::{Architecture, Emulator, HostFunction, Loaded, Region, Result};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use unicorn_engine::Context;

mod memory;
use memory::Memory;

#[cfg(test)]
mod tests;

const BETWEEN_CALLS: &str = "checkpoint and rewind work only between calls";

/// A reusable, in-memory copy of guest state, captured between calls.
///
/// Read-only library images are shared. User object payloads (`Rc<dyn Any>`) are
/// shared too; all other guest memory and JNI objects are copied.
pub struct Checkpoint {
    arch: Architecture,
    context: Context,
    memory: Vec<Memory>,
    modules: Vec<Loaded>,
    next_module: u64,
    regions: Vec<Region>,
    host: Vec<HostFunction>,
    overrides: HashMap<String, u64>,
    symbol_aliases: HashMap<String, String>,
    library_dirs: Vec<PathBuf>,
    rootfs: PathBuf,
    properties: HashMap<String, String>,
    process_name: String,
    heap: u64,
    heap_end: u64,
    brk: u64,
    free_blocks: Vec<(u64, u64)>,
    allocations: HashMap<u64, u64>,
    heap_high_water: u64,
    dlerror_buf: u64,
    dlerror_pending: bool,
    dlerror_capacity: usize,
    files: HashMap<u64, crate::syscalls::FileState>,
    next_fd: u64,
    source: crate::source::State,
    jni: crate::jni::JniState,
    threads: crate::threads::Threads,
    heap_records: crate::debug::heap::SavedHeap,
}

impl Emulator {
    /// Capture guest state. Fails while a guest call or debugger stop is active.
    pub fn checkpoint(&mut self) -> Result<Checkpoint> {
        self.between_calls()?;
        self.cpu()?;
        Ok(Checkpoint {
            arch: self.architecture(),
            context: self.cpu()?.context_init()?,
            memory: Memory::capture(self)?,
            modules: self.modules.clone(),
            next_module: self.next_module,
            regions: self.regions.clone(),
            host: self.host.clone(),
            overrides: self.overrides.clone(),
            symbol_aliases: self.symbol_aliases.clone(),
            library_dirs: self.library_dirs.clone(),
            rootfs: self.rootfs.clone(),
            properties: self.properties.clone(),
            process_name: self.process_name.clone(),
            heap: self.heap,
            heap_end: self.heap_end,
            brk: self.brk,
            free_blocks: self.free_blocks.clone(),
            allocations: self.allocations.clone(),
            heap_high_water: self.heap_high_water,
            dlerror_buf: self.dlerror_buf,
            dlerror_pending: self.dlerror_pending,
            dlerror_capacity: self.dlerror_capacity,
            files: self
                .files
                .iter_mut()
                .map(|(&fd, file)| Ok((fd, file.checkpoint()?)))
                .collect::<Result<_>>()?,
            next_fd: self.next_fd,
            source: self.source.shared.borrow().state.clone(),
            jni: self.jni.clone(),
            threads: self.threads.copy(self.uc.as_mut().unwrap())?,
            heap_records: self.saved_heap(),
        })
    }

    /// Restore a checkpoint, including into a fresh emulator of its architecture.
    /// Debugger settings, events, coverage counts and recording/replay stay local.
    pub fn rewind(&mut self, checkpoint: &Checkpoint) -> Result<()> {
        self.between_calls()?;
        let created_cpu = self.uc.is_none();
        if let Some(arch) = self.arch
            && arch != checkpoint.arch
        {
            return Err(format!(
                "cannot rewind {} checkpoint into {} emulator",
                checkpoint.arch.name(),
                arch.name()
            )
            .into());
        }
        self.source.shared.borrow_mut().state = checkpoint.source.clone();
        if created_cpu {
            self.create_cpu(checkpoint.arch)?;
        } else {
            self.source
                .configure_counter(self.uc.as_mut().unwrap(), checkpoint.arch)?;
        }
        let modules_changed = self.modules.len() != checkpoint.modules.len()
            || self.modules.iter().zip(&checkpoint.modules).any(|(a, b)| {
                a.base != b.base
                    || a.path != b.path
                    || a.app != b.app
                    || !Arc::ptr_eq(&a.image, &b.image)
            });
        Memory::restore(self, &checkpoint.memory)?;
        self.modules = checkpoint.modules.clone();
        self.next_module = checkpoint.next_module;
        self.regions = checkpoint.regions.clone();
        self.host = checkpoint.host.clone();
        self.overrides = checkpoint.overrides.clone();
        self.symbol_aliases = checkpoint.symbol_aliases.clone();
        self.library_dirs = checkpoint.library_dirs.clone();
        self.rootfs = checkpoint.rootfs.clone();
        self.properties = checkpoint.properties.clone();
        self.process_name = checkpoint.process_name.clone();
        self.heap = checkpoint.heap;
        self.heap_end = checkpoint.heap_end;
        self.brk = checkpoint.brk;
        self.free_blocks = checkpoint.free_blocks.clone();
        self.allocations = checkpoint.allocations.clone();
        self.heap_high_water = checkpoint.heap_high_water;
        self.dlerror_buf = checkpoint.dlerror_buf;
        self.dlerror_pending = checkpoint.dlerror_pending;
        self.dlerror_capacity = checkpoint.dlerror_capacity;
        self.files = checkpoint
            .files
            .iter()
            .map(|(&fd, file)| Ok((fd, file.reopen(self)?)))
            .collect::<Result<_>>()?;
        self.next_fd = checkpoint.next_fd;
        self.jni.rewind(&checkpoint.jni);
        let mut threads = checkpoint.threads.copy(self.cpu_mut()?)?;
        threads.slice_hook_installed = self.threads.slice_hook_installed;
        self.threads = threads;
        self.restore_heap_records(&checkpoint.heap_records);
        if self.threads.has_saved() {
            self.install_thread_slice_hook()?;
        }
        self.cpu_mut()?.context_restore(&checkpoint.context)?;
        *self.cpu_mut()?.get_data_mut() = crate::CpuData::default();
        if self.trace.imports {
            self.refresh_imports();
        }
        if modules_changed || created_cpu {
            self.set_trace(self.trace.clone())?;
        }
        Ok(())
    }

    fn between_calls(&self) -> Result<()> {
        if self.threads.depth != 0 {
            return Err(BETWEEN_CALLS.into());
        }
        Ok(())
    }
}
