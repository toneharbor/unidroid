use crate::{Allocation, Crash, Frame, HeapObject, Referrer, Stop, StopReason, Target};
use serde::{Serialize, Serializer, ser::SerializeMap};
use serde_json::{Value, json};

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Response {
    Ok(Value),
    #[serde(rename = "ok")]
    Registers(Registers),
    Jni(Value),
    Import(Value),
    Break(Break),
    #[serde(untagged)]
    Error {
        error: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        crash: Option<CrashResponse>,
    },
}

pub(super) struct Registers(Vec<(&'static str, u64)>);
impl Serialize for Registers {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (name, value) in &self.0 {
            map.serialize_entry(name, &value.to_string())?;
        }
        map.end()
    }
}

pub(super) fn registers_value(registers: &[(&'static str, u64)]) -> Registers {
    Registers(registers.to_vec())
}

pub(super) fn frames_value(frames: &[Frame]) -> Value {
    json!(frames.iter().map(|frame| json!({
        "address": frame.address.to_string(), "location": frame.location, "symbol": frame.symbol,
    })).collect::<Vec<_>>())
}

#[derive(Serialize)]
pub(super) struct CrashResponse {
    message: String,
    thread: u32,
    registers: Registers,
    backtrace: Value,
    stack: Vec<[String; 2]>,
    jni: Vec<String>,
}

pub(super) fn crash_value(crash: &Crash) -> CrashResponse {
    CrashResponse {
        message: crash.message.clone(),
        thread: crash.thread,
        registers: registers_value(&crash.registers),
        backtrace: frames_value(&crash.backtrace),
        stack: crash
            .stack
            .iter()
            .map(|(address, value)| [address.to_string(), value.to_string()])
            .collect(),
        jni: crash.jni.clone(),
    }
}

pub(super) fn allocation_value(allocation: &Allocation) -> Value {
    json!({
        "address": allocation.address.to_string(),
        "size": allocation.size.to_string(),
        "live": allocation.live,
        "function": allocation.function,
        "thread": allocation.thread,
        "backtrace": frames_value(&allocation.backtrace),
        "freed": allocation.freed.as_ref().map(|frames| frames_value(frames)),
    })
}

pub(super) fn object_value(object: &HeapObject) -> Value {
    json!({
        "address": object.address.to_string(),
        "size": object.size.to_string(),
        "allocation": allocation_value(&object.allocation),
        "fields": object.fields.iter().map(|field| json!({
            "offset": field.offset.to_string(),
            "value": field.value.to_string(),
            "target": field.target.as_ref().map(target_value),
            "string": field.string,
        })).collect::<Vec<_>>(),
        "referrers": object.referrers.iter().map(referrer_value).collect::<Vec<_>>(),
    })
}

fn target_value(target: &Target) -> Value {
    match target {
        Target::Block {
            address,
            offset,
            object,
        } => json!({
            "kind": "block",
            "address": address.to_string(),
            "offset": offset.to_string(),
            "object": object.as_deref().map(object_value),
        }),
        Target::Location { location } => json!({ "kind": "location", "location": location }),
    }
}

fn referrer_value(referrer: &Referrer) -> Value {
    json!({
        "address": referrer.address.to_string(),
        "location": referrer.location,
        "block": referrer.block.map(|address| address.to_string()),
        "object": referrer.object.as_deref().map(object_value),
    })
}

#[derive(Serialize)]
pub(super) struct Break {
    reason: &'static str,
    address: String,
    location: String,
    thread: u32,
    registers: Registers,
    #[serde(skip_serializing_if = "Option::is_none")]
    watch: Option<Value>,
}

pub(super) fn break_value(stop: &Stop, emu: &crate::Emulator, registers: Registers) -> Break {
    Break {
        reason: match stop.reason {
            StopReason::Breakpoint => "breakpoint",
            StopReason::Watchpoint => "watchpoint",
            StopReason::Step => "step",
            StopReason::Finish => "finish",
        },
        address: stop.address.to_string(),
        location: emu.symbolize(stop.address),
        thread: stop.thread,
        registers,
        watch: stop.watch.map(|watch| {
            json!({
                "pc": watch.pc.to_string(), "location": emu.symbolize(watch.pc),
                "address": watch.address.to_string(), "size": watch.size,
                "write": watch.write, "value": watch.value.to_string(),
            })
        }),
    }
}
