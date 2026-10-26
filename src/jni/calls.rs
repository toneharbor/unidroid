use super::names::mangle;
use super::value::{decode, method_types};
use super::{Handle, JniHandler, JniValue, MEMBER_ID_BASE, Member, Object};
use crate::{Emulator, HostFunction, Module, Result};
use std::{cell::RefCell, rc::Rc};

impl Emulator {
    pub(crate) fn jni_message(&self, message: impl std::fmt::Display) -> String {
        let message = message.to_string();
        let suffix = format!(" from {}", self.location(self.lr().unwrap_or(0)));
        if message.ends_with(&suffix) {
            message
        } else {
            message + &suffix
        }
    }

    pub(super) fn reference_error(
        &self,
        function: &str,
        handle: u64,
    ) -> Box<dyn std::error::Error> {
        self.jni_message(format!("invalid JNI reference 0x{handle:x} in {function}"))
            .into()
    }
    pub(super) fn require_object(
        &self,
        function: &str,
        handle: u64,
    ) -> Result<Rc<RefCell<Object>>> {
        if handle == 0 {
            return Err(self.reference_error(function, handle));
        }
        self.jni
            .objects
            .get(&Handle(handle))
            .map(|reference| reference.object.clone())
            .ok_or_else(|| self.reference_error(function, handle))
    }
    pub(super) fn require_member(&self, function: &str, id: u64) -> Result<Member> {
        id.checked_sub(MEMBER_ID_BASE)
            .and_then(|index| self.jni.members.get(index as usize))
            .cloned()
            .ok_or_else(|| self.member_error(function, id))
    }
    pub(super) fn member_error(&self, function: &str, id: u64) -> Box<dyn std::error::Error> {
        self.jni_message(format!("invalid JNI id 0x{id:x} in {function}"))
            .into()
    }
    pub(crate) fn setup_jni(&mut self) -> Result<()> {
        let p = self.pointer_size();
        let table = self.malloc(233 * p as usize)?;
        for index in 0..233 {
            let stub = self.stub(HostFunction::Jni(index))?;
            self.write_pointer(table + index as u64 * p, stub)?;
        }
        self.jni.env = self.malloc(p as usize)?;
        self.write_pointer(self.jni.env, table)?;
        let table = self.malloc(8 * p as usize)?;
        for index in 0..8 {
            let stub = self.stub(HostFunction::Vm(index))?;
            self.write_pointer(table + index as u64 * p, stub)?;
        }
        self.jni.vm = self.malloc(p as usize)?;
        self.write_pointer(self.jni.vm, table)?;
        Ok(())
    }
    pub fn registered_natives(&self) -> Vec<(String, String, u64)> {
        let mut natives: Vec<_> = self
            .jni
            .natives
            .iter()
            .map(|((class, method), &address)| (class.clone(), method.clone(), address))
            .collect();
        natives.sort_unstable_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        natives
    }
    pub fn call_jni_onload(&mut self, module: Module, handler: &mut dyn JniHandler) -> Result<u64> {
        let function = self.find_symbol(module, "JNI_OnLoad")?;
        self.invoke_jni(
            function,
            &[
                JniValue::Object(Handle(self.jni.vm)),
                JniValue::Object(Handle(0)),
            ],
            b'I',
            handler,
        )
        .map(JniValue::bits)
    }
    pub fn call_static_jni_method(
        &mut self,
        handler: &mut dyn JniHandler,
        class: &str,
        method: &str,
        args: &[JniValue],
    ) -> Result<JniValue> {
        self.call_native_method(handler, class, None, method, args)
    }
    pub fn call_jni_method(
        &mut self,
        handler: &mut dyn JniHandler,
        class: &str,
        receiver: Handle,
        method: &str,
        args: &[JniValue],
    ) -> Result<JniValue> {
        self.call_native_method(handler, class, Some(receiver), method, args)
    }
    fn call_native_method(
        &mut self,
        handler: &mut dyn JniHandler,
        class: &str,
        receiver: Option<Handle>,
        method: &str,
        args: &[JniValue],
    ) -> Result<JniValue> {
        self.cpu()?;
        let invalid = || {
            self.jni_message(format!(
                "invalid JNI signature {method} in call_native_method"
            ))
        };
        let (name, signature) = method.split_at(method.find('(').ok_or_else(invalid)?);
        let (_, result_type) = method_types(signature).ok_or_else(invalid)?;
        let chain = self.class_chain(class);
        let found = chain.iter().find_map(|&class| {
            let key = (class.to_owned(), method.to_owned());
            let function = self.jni.natives.get(&key).copied().or_else(|| {
                let short = format!("Java_{}_{}", mangle(class), mangle(name));
                let args_sig = signature[1..].split(')').next().unwrap();
                let long = format!("{short}__{}", mangle(args_sig));
                [short, long].iter().find_map(|symbol| {
                    (0..self.modules.len()).find_map(|i| self.local_symbol(Module(i), symbol))
                })
            })?;
            Some((class.to_owned(), function))
        });
        let (declaring, function) = found.ok_or_else(|| format!(
            "native method {class}.{method} not found: no RegisterNatives entry or Java_ export on {} (an inherited method needs its superclasses declared)",
            chain.join(", ")
        ))?;
        let receiver = receiver.unwrap_or_else(|| self.class(&declaring));
        let mut values = vec![JniValue::Object(Handle(self.jni.env)), receiver.into()];
        values.extend_from_slice(args);
        self.invoke_jni(function, &values, result_type, handler)
    }
    pub(super) fn invoke_jni(
        &mut self,
        function: u64,
        args: &[JniValue],
        kind: u8,
        handler: &mut dyn JniHandler,
    ) -> Result<JniValue> {
        self.jni_frame(false, kind, |emu| {
            let result = emu.invoke(
                function,
                args.iter().map(|&value| (value.kind(), value.bits())),
                kind,
                handler,
            );
            emu.exception_result()?;
            result
        })
    }
    pub(crate) fn jni_frame(
        &mut self,
        join_active: bool,
        kind: u8,
        body: impl FnOnce(&mut Self) -> Result<u64>,
    ) -> Result<JniValue> {
        // Raw C calls join an active frame; JNI invocations always open a new one.
        if join_active && self.jni.scope >= 2 {
            return body(self).map(|bits| decode(kind, bits));
        }
        let tid = self.threads.tid();
        let old_scope = self.jni.scope;
        let old_floor = self.jni.frame_floor;
        self.jni.scope = old_scope.max(1) + 1;
        self.jni.frame_floor = self.jni.scope;
        let result = body(self).map(|bits| decode(kind, bits));
        if let Ok(JniValue::Object(handle)) = result
            && let Some(reference) = self.jni.objects.get_mut(&handle)
            && reference.scope > old_scope.max(1)
            && reference.tid == tid
        {
            reference.scope = old_scope;
        }
        self.jni
            .objects
            .retain(|_, reference| reference.scope <= old_scope.max(1) || reference.tid != tid);
        self.jni.scope = old_scope;
        self.jni.frame_floor = old_floor;
        result
    }
    pub(super) fn jni_allocator(&mut self, name: &str, argument: u64) -> Result<u64> {
        let function = (0..self.modules.len())
            .find_map(|i| self.local_symbol(Module(i), name))
            .ok_or_else(|| format!("JNI buffer requires {name}: no loaded library exports it"))?;
        self.call_function(function, &[argument])
    }
    pub(super) fn jni_buffer(&mut self, bytes: &[u8]) -> Result<u64> {
        let ptr = self.jni_allocator("malloc", bytes.len() as u64)?;
        self.write_bytes(ptr, bytes)?;
        Ok(ptr)
    }
    pub(super) fn jni_error(
        &self,
        function: &str,
        class: &str,
        name: &str,
        signature: &str,
    ) -> Box<dyn std::error::Error> {
        self.jni_message(format!(
            "unhandled JNI call: {function} {class}->{name}{signature}"
        ))
        .into()
    }
    pub(super) fn unsupported_jni(&self, name: &str) -> Box<dyn std::error::Error> {
        self.jni_message(format!("unsupported JNIEnv function {name}"))
            .into()
    }
    pub(super) fn unsupported_vm(&self, name: &str) -> Box<dyn std::error::Error> {
        self.jni_message(format!("unsupported JavaVM function {name}"))
            .into()
    }
    pub(super) fn validate_ref(&self, function: &str, handle: u64, kind: u8) -> Result<()> {
        if handle == 0 && kind == 6 {
            return Ok(());
        }
        if handle == 0 {
            return Err(self.reference_error(function, handle));
        }
        let object = self.require_object(function, handle)?;
        let valid = matches!(
            (&*object.borrow(), kind),
            (_, 0 | 6)
                | (Object::Class(_), 1)
                | (Object::String(_), 2)
                | (Object::Array(_, _), 3)
                | (Object::ObjectArray(_, _), 4)
                | (Object::Array(_, _) | Object::ObjectArray(_, _), 5)
        );
        if valid {
            Ok(())
        } else {
            Err(self.reference_error(function, handle))
        }
    }
    pub(super) fn signature_error(
        &self,
        function: &str,
        signature: &str,
    ) -> Box<dyn std::error::Error> {
        self.jni_message(format!("invalid JNI signature {signature} in {function}"))
            .into()
    }
    pub(super) fn jni_arguments(
        &self,
        function: &str,
        signature: &str,
        variant: usize,
        start: usize,
    ) -> Result<Vec<JniValue>> {
        let (kinds, _) =
            method_types(signature).ok_or_else(|| self.signature_error(function, signature))?;
        let bits = self.argument_bits(&kinds, variant, start)?;
        Ok(kinds
            .into_iter()
            .zip(bits)
            .map(|(kind, bits)| {
                if kind == b'F' && variant != 2 {
                    JniValue::Float(f64::from_bits(bits) as f32)
                } else {
                    decode(kind, bits)
                }
            })
            .collect())
    }
}
