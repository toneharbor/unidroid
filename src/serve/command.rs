use super::wire::{Address, Method};
use serde::Deserialize;
use serde_json::Value;

fn default_object_depth() -> usize {
    1
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub(super) enum Command {
    Break {
        at: String,
        r#if: Option<String>,
    },
    Unbreak {
        at: String,
    },
    Watch {
        at: String,
        size: u64,
        kind: crate::Watch,
        #[serde(default)]
        log: bool,
    },
    Unwatch {
        at: String,
    },
    Tracepoint {
        at: String,
        values: Vec<String>,
        #[serde(default = "default_true")]
        log: bool,
    },
    Untracepoint {
        at: String,
    },
    TracepointHits {
        at: String,
        #[serde(default)]
        reset: bool,
    },
    Resolve {
        at: String,
    },
    Registers,
    SetRegister {
        name: String,
        value: Address,
    },
    Backtrace,
    Modules,
    Maps,
    Search {
        base64: String,
    },
    Crypto,
    Coverage {
        #[serde(default)]
        reset: bool,
    },
    Heap {
        #[serde(default)]
        reset: bool,
    },
    Object {
        at: String,
        #[serde(default = "default_object_depth")]
        depth: usize,
    },
    Checkpoint,
    Rewind {
        id: u64,
    },
    DropCheckpoint {
        id: u64,
    },
    Snapshot {
        path: String,
    },
    Continue,
    Step,
    Finish,
    Return {
        value: Address,
    },
    Abort,
    Trace {
        #[serde(default)]
        jni: bool,
        #[serde(default)]
        syscalls: bool,
        #[serde(default)]
        imports: bool,
        #[serde(default)]
        heap: bool,
        #[serde(default)]
        code: Vec<String>,
        #[serde(default)]
        calls: Vec<String>,
        #[serde(default)]
        coverage: Vec<String>,
    },
    Load {
        path: String,
        #[serde(default = "default_true")]
        constructors: bool,
    },
    LoadApkLibrary {
        name: String,
        #[serde(default = "default_true")]
        constructors: bool,
    },
    Apk,
    SetApk {
        #[serde(flatten)]
        values: ApkUpdate,
    },
    RunConstructors {
        module: usize,
    },
    HookImport {
        name: String,
    },
    Alias {
        name: String,
        target: String,
    },
    Superclass {
        class: String,
        superclass: String,
    },
    FindSymbol {
        module: usize,
        name: String,
    },
    Onload {
        module: usize,
    },
    CallStatic {
        class: String,
        method: Method,
        args: Vec<Value>,
    },
    Call {
        class: String,
        receiver: u64,
        method: Method,
        args: Vec<Value>,
    },
    CallFunction {
        address: Address,
        args: Vec<Address>,
    },
    Malloc {
        size: usize,
    },
    Free {
        address: Address,
    },
    NewCstring {
        value: String,
    },
    Read {
        address: Address,
        size: usize,
    },
    Write {
        address: Address,
        base64: String,
    },
    AddFile {
        path: String,
        base64: String,
    },
    AddHostFile {
        path: String,
        host_path: String,
    },
    RemoveFile {
        path: String,
    },
    File {
        path: String,
    },
    NewString {
        value: String,
    },
    String {
        handle: u64,
    },
    NewArray {
        r#type: String,
        base64: String,
    },
    Array {
        handle: u64,
    },
    NewObjectArray {
        class: String,
        items: Vec<u64>,
    },
    ObjectArray {
        handle: u64,
    },
    NewJavaMap {
        class: String,
        entries: Vec<[u64; 2]>,
    },
    JavaMap {
        handle: u64,
    },
    NewJavaList {
        class: String,
        items: Vec<u64>,
    },
    JavaList {
        handle: u64,
    },
    NewBoxed {
        r#type: String,
        value: Value,
    },
    Unboxed {
        handle: u64,
    },
    NewObject {
        class: String,
    },
    Class {
        name: String,
    },
    Release {
        handle: u64,
    },
    Answer {
        #[serde(default)]
        value: Value,
    },
    Unhandled,
}

use crate::{Emulator, Result};
use base64::{Engine, prelude::BASE64_STANDARD};

pub(super) fn apk_value(emu: &Emulator) -> Value {
    emu.apk().map_or(Value::Null, |apk| serde_json::json!({
        "path": apk.path, "package": apk.package,
        "version_name": apk.version_name, "version_code": apk.version_code,
        "signatures": apk.signatures.iter().map(|cert| BASE64_STANDARD.encode(cert)).collect::<Vec<_>>(),
        "signers": apk.signers.iter().map(|cert| BASE64_STANDARD.encode(cert)).collect::<Vec<_>>(),
        "signing_history": apk.signing_history.iter().map(|cert| BASE64_STANDARD.encode(cert)).collect::<Vec<_>>(),
    }))
}

#[derive(Deserialize)]
pub(super) struct ApkUpdate {
    package: Option<String>,
    #[serde(default, deserialize_with = "nullable_name")]
    version_name: Option<Option<String>>,
    version_code: Option<i64>,
    signatures: Option<Vec<String>>,
    signers: Option<Vec<String>>,
    signing_history: Option<Vec<String>>,
}

fn nullable_name<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}

impl ApkUpdate {
    pub(super) fn apply(self, emu: &mut Emulator) -> Result<()> {
        let apk = emu.apk_mut().ok_or("set_apk: no APK configured")?;
        let decode = |values: Option<Vec<String>>, field: &str| -> Result<Option<Vec<Vec<u8>>>> {
            values
                .map(|values| {
                    values
                        .iter()
                        .map(|value| BASE64_STANDARD.decode(value))
                        .collect::<std::result::Result<Vec<_>, _>>()
                })
                .transpose()
                .map_err(|error| format!("set_apk {} {field}: {error}", apk.path.display()).into())
        };
        let signatures = decode(self.signatures, "signatures")?;
        let signers = decode(self.signers, "signers")?;
        let signing_history = decode(self.signing_history, "signing_history")?;
        if let Some(package) = self.package {
            apk.package = package;
        }
        if let Some(version_name) = self.version_name {
            apk.version_name = version_name;
        }
        if let Some(version_code) = self.version_code {
            apk.version_code = version_code;
        }
        if let Some(signatures) = signatures {
            apk.signatures = signatures;
        }
        if let Some(signers) = signers {
            apk.signers = signers;
        }
        if let Some(signing_history) = signing_history {
            apk.signing_history = signing_history;
        }
        Ok(())
    }
}
