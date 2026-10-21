use crate::Crash;
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Response {
    Ok(Value),
    Jni(Value),
    #[serde(untagged)]
    Error {
        error: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        crash: Option<Value>,
    },
}

pub(super) fn registers_value(registers: &[(&str, u64)]) -> Value {
    Value::Object(
        registers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), Value::String(value.to_string())))
            .collect(),
    )
}

pub(super) fn crash_value(crash: &Crash) -> Value {
    json!({
        "message": crash.message,
        "thread": crash.thread,
        "registers": registers_value(&crash.registers),
        "backtrace": crash.backtrace.iter().map(|frame| json!({
            "address": frame.address.to_string(), "location": frame.location, "symbol": frame.symbol,
        })).collect::<Vec<_>>(),
    })
}
