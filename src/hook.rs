use crate::{Emulator, HostFunction, JniHandler, Module, Result};

/// An imported C call. Arguments are the raw x0–x7 or r0–r3 registers.
pub struct ImportCall<'a> {
    pub name: &'a str,
    pub args: &'a [u64],
    pub original: Option<u64>,
    pub return_address: u64,
}

pub(crate) enum HostResult {
    Return(u64),
    Jump(u64),
}

impl Emulator {
    /// Replace symbol resolution with a call to the running handler's `import` method.
    pub fn hook_import(&mut self, name: impl Into<String>) -> Result<()> {
        let name = name.into();
        if self.host.iter().any(
            |host| matches!(host, HostFunction::Import { name: hooked, .. } if hooked == &name),
        ) {
            return Ok(());
        }
        let hook = HostFunction::Import {
            original: self.overrides.get(&name).copied(),
            name: name.clone(),
        };
        if self.uc.is_none() {
            self.host.push(hook);
            return Ok(());
        }
        let address = self.stub(hook)?;
        self.overrides.insert(name.clone(), address);
        for i in 0..self.modules.len() {
            if self.modules[i].relocated {
                self.relocate(Module(i), Some(&name))?;
            }
        }
        if self.trace.imports {
            self.refresh_imports();
        }
        Ok(())
    }

    pub(super) fn install_pending_imports(&mut self, pending: Vec<HostFunction>) -> Result<()> {
        for hook in pending {
            let HostFunction::Import { name, .. } = hook else {
                unreachable!()
            };
            self.hook_import(name)?;
        }
        Ok(())
    }

    pub(super) fn import_call(
        &mut self,
        name: &str,
        original: Option<u64>,
        handler: &mut dyn JniHandler,
    ) -> Result<HostResult> {
        let original = original.or_else(|| {
            (0..self.modules.len()).find_map(|i| self.exported_symbol(Module(i), name))
        });
        let count = if self.pointer_size() == 8 { 8 } else { 4 };
        let mut args = [0; 8];
        for (i, arg) in args[..count].iter_mut().enumerate() {
            *arg = self.reg(i)?;
        }
        let return_address = self.lr()?;
        let call = ImportCall {
            name,
            args: &args[..count],
            original,
            return_address,
        };
        match handler.import(self, &call) {
            Some(value) => Ok(HostResult::Return(value)),
            None => original.map(HostResult::Jump).ok_or_else(|| {
                format!(
                    "call to unresolved symbol {name} from {}",
                    self.location(return_address)
                )
                .into()
            }),
        }
    }
}
