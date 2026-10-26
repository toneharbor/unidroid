use crate::{Emulator, Event, HostFunction, Result, TRAMPOLINE};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};
use unicorn_engine::UcHookId;

#[derive(Default)]
pub(crate) struct Imports {
    hook: Option<UcHookId>,
    targets: Rc<RefCell<Targets>>,
}
#[derive(Default)]
struct Targets {
    exports: HashMap<u64, Target>,
    suspended: HashSet<(usize, u32, u64)>,
    blocks: HashMap<u32, Blocks>,
}
#[derive(Default)]
struct Blocks {
    previous: Option<u64>,
    current: Option<u64>,
}
struct Target {
    function: String,
    module: String,
    owner: Option<usize>,
}

// Bit i marks a C string in argument register i.
fn strings(function: &str) -> u8 {
    match function {
        "strcmp" | "strncmp" | "strcpy" | "strncpy" | "strcat" | "strstr" | "fopen" => 0b0011,
        "strlen"
        | "strchr"
        | "strrchr"
        | "strdup"
        | "open"
        | "access"
        | "stat"
        | "lstat"
        | "getenv"
        | "atoi"
        | "atol"
        | "strtol"
        | "strtoul"
        | "dlopen"
        | "__system_property_get"
        | "printf" => 0b0001,
        "dlsym" | "sprintf" => 0b0010,
        "snprintf" => 0b0100,
        "__android_log_write" | "__android_log_print" => 0b0110,
        _ => 0,
    }
}

impl Emulator {
    pub(crate) fn refresh_imports(&mut self) {
        let mut targets = self.imports.targets.borrow_mut();
        targets.exports.clear();
        for (i, m) in self.modules.iter().enumerate() {
            for symbol in &m.image.symbols {
                if symbol.defined && symbol.function && !symbol.name.is_empty() {
                    let address = if symbol.absolute {
                        symbol.value
                    } else {
                        m.base + symbol.value
                    } & !1;
                    targets.exports.entry(address).or_insert_with(|| Target {
                        function: symbol.name.clone(),
                        module: m.name.clone(),
                        owner: Some(i),
                    });
                }
            }
        }
        for (i, host) in self.host.iter().enumerate() {
            let (function, module) = match host {
                HostFunction::Asset(name) => ((*name).to_owned(), "libandroid.so".to_owned()),
                HostFunction::Dl(name) => ((*name).to_owned(), "libdl.so".to_owned()),
                HostFunction::Property => ("__system_property_get".into(), "libc.so".into()),
                HostFunction::Import { name, .. } => (name.clone(), "<hook>".into()),
                HostFunction::Unresolved { symbol, .. } => (symbol.clone(), "<unresolved>".into()),
                HostFunction::Jni(_) | HostFunction::Vm(_) => continue,
            };
            targets.exports.insert(
                TRAMPOLINE + i as u64 * 8,
                Target {
                    function,
                    module,
                    owner: None,
                },
            );
        }
    }

    pub(crate) fn suspend_traces(&mut self, pc: u64) -> Result<()> {
        // Instruction events are still pending; imports have already been emitted.
        self.discard_unexecuted_code(pc);
        self.suspend_calls(pc);
        let data = self.cpu()?.get_data();
        let mut targets = self.imports.targets.borrow_mut();
        if let Some(blocks) = targets.blocks.get_mut(&data.thread)
            && blocks.current == Some(pc)
        {
            // Block hooks run before execution; the scheduler may stop this block.
            blocks.current = blocks.previous;
        }
        if data.import == Some(pc) {
            targets.suspended.insert((data.depth, data.thread, pc));
        }
        Ok(())
    }

    pub(crate) fn discard_suspended_import(&mut self, address: u64) {
        self.imports.targets.borrow_mut().suspended.remove(&(
            self.threads.depth,
            self.threads.tid(),
            address,
        ));
    }

    pub(crate) fn trace_imports(&mut self, enabled: bool) -> Result<()> {
        if self.uc.is_none() {
            return Ok(());
        }
        if enabled && self.imports.hook.is_none() {
            self.refresh_imports();
            let targets = self.imports.targets.clone();
            let modules = self.module_table.clone();
            let arch = self.architecture();
            let events = self.events.clone();
            let hook = self
                .cpu_mut()?
                .add_block_hook(1, 0, move |uc, address, _| {
                    uc.get_data_mut().import = None;
                    let thread = uc.get_data().thread;
                    let mut targets = targets.borrow_mut();
                    let blocks = targets.blocks.entry(thread).or_default();
                    let previous = blocks.current.replace(address);
                    blocks.previous = previous;
                    // A later scheduling hook may have stopped this block before execution.
                    if targets
                        .suspended
                        .remove(&(uc.get_data().depth, thread, address))
                    {
                        uc.get_data_mut().import = Some(address);
                        return;
                    }
                    let Some(target) = targets.exports.get(&address) else {
                        return;
                    };
                    let Some(previous) = previous else { return };
                    let modules = modules.borrow();
                    let from_app = |address| {
                        modules
                            .at(address)
                            .filter(|(owner, module)| module.app && target.owner != Some(*owner))
                    };
                    if from_app(previous).is_none() {
                        return;
                    }
                    let lr = uc.reg_read(arch.lr()).expect("link register") & !1;
                    let Some((_, app)) = from_app(lr) else {
                        return;
                    };
                    let args =
                        super::arguments::register_arguments(uc, arch, strings(&target.function));
                    let from = app.symbolize(lr);
                    uc.get_data_mut().import = Some(address);
                    events.borrow_mut().emit(Event::Call {
                        function: target.function.clone(),
                        module: target.module.clone(),
                        args,
                        from,
                    });
                })?;
            self.imports.hook = Some(hook);
            self.cpu_mut()?.ctl_flush_tb()?;
        } else if !enabled && let Some(hook) = self.imports.hook.take() {
            super::hooks::remove_hooks(self.cpu_mut()?, [hook])?;
            *self.imports.targets.borrow_mut() = Targets::default();
        }
        Ok(())
    }
}
