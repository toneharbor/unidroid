use crate::{Crash, Frame, Stop, StopReason};
use serde::{Serialize, Serializer, ser::SerializeMap};
use serde_json::{Value, json};

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Response {
    Ok(Value),
    #[serde(rename = "ok")]
    Registers(Registers),
    Jni(Value),
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
    }
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
