use crate::{CpuData, Emulator, Event, HostFunction, Image, Result, TRAMPOLINE};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};
use unicorn_engine::{UcHookId, Unicorn};

#[derive(Default)]
pub(crate) struct Imports {
    hook: Option<UcHookId>,
    targets: Rc<RefCell<Targets>>,
}
#[derive(Default)]
struct Targets {
    exports: HashMap<u64, Target>,
    apps: Vec<App>,
    suspended: HashSet<(u32, u64)>,
    blocks: HashMap<u32, Blocks>,
}
#[derive(Default)]
struct Blocks {
    previous: Option<u64>,
    current: Option<u64>,
}
struct App {
    owner: usize,
    base: u64,
    name: String,
    image: Arc<Image>,
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
        targets.apps.clear();
        for (i, m) in self.modules.iter().enumerate() {
            if m.app {
                targets.apps.push(App {
                    owner: i,
                    base: m.base,
                    name: m.name.clone(),
                    image: m.image.clone(),
                });
            }
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
                HostFunction::Dl(name) => ((*name).to_owned(), "libdl.so".to_owned()),
                HostFunction::Property => ("__system_property_get".into(), "libc.so".into()),
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

    pub(crate) fn suspend_import(&mut self, pc: u64) -> Result<()> {
        let data = self.cpu()?.get_data();
        let mut targets = self.imports.targets.borrow_mut();
        if let Some(blocks) = targets.blocks.get_mut(&data.thread)
            && blocks.current == Some(pc)
        {
            // Block hooks run before execution; the scheduler may stop this block.
            blocks.current = blocks.previous;
        }
        if data.import == Some(pc) {
            targets.suspended.insert((data.thread, pc));
        }
        Ok(())
    }

    pub(crate) fn trace_imports(&mut self, enabled: bool) -> Result<()> {
        if self.uc.is_none() {
            return Ok(());
        }
        if enabled && self.imports.hook.is_none() {
            self.refresh_imports();
            let targets = self.imports.targets.clone();
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
                    if targets.suspended.remove(&(thread, address)) {
                        return;
                    }
                    let Some(target) = targets.exports.get(&address) else {
                        return;
                    };
                    let Some(previous) = previous else { return };
                    if !targets.apps.iter().any(|app| {
                        target.owner != Some(app.owner)
                            && (app.base..app.base + app.image.size).contains(&previous)
                    }) {
                        return;
                    }
                    let lr = uc.reg_read(arch.lr()).expect("link register") & !1;
                    let Some(app) = targets.apps.iter().find(|app| {
                        target.owner != Some(app.owner)
                            && (app.base..app.base + app.image.size).contains(&lr)
                    }) else {
                        return;
                    };
                    let mask = strings(&target.function);
                    let args = (0..4)
                        .map(|i| {
                            let value = uc.reg_read(arch.gp(i)).expect("argument register");
                            if mask & (1 << i) != 0 {
                                trace_string(uc, value).unwrap_or_else(|| format!("0x{value:x}"))
                            } else {
                                format!("0x{value:x}")
                            }
                        })
                        .collect();
                    let offset = lr - app.base;
                    let mut from = format!("{}+0x{offset:x}", app.name);
                    if let Some(symbol) = app.image.function(offset) {
                        from.push_str(&format!(" ({symbol})"));
                    }
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
            self.cpu_mut()?.remove_hook(hook)?;
            self.cpu_mut()?.ctl_flush_tb()?;
            *self.imports.targets.borrow_mut() = Targets::default();
        }
        Ok(())
    }
}

fn trace_string(uc: &Unicorn<'_, CpuData>, address: u64) -> Option<String> {
    let mut bytes = Vec::new();
    for i in 0..256 {
        let mut byte = [0];
        uc.mem_read(address.checked_add(i)?, &mut byte).ok()?;
        if byte[0] == 0 {
            break;
        }
        bytes.push(byte[0]);
    }
    Some(format!("{:?}", String::from_utf8_lossy(&bytes)))
}
