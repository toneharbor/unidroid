use super::wire::{Address, Method};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub(super) enum Command {
    Break {
        at: String,
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
        code: Vec<String>,
        #[serde(default)]
        calls: Vec<String>,
    },
    Load {
        path: String,
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
