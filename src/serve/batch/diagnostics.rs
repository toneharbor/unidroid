use crate::{Diff, Emulator, Result, Snapshot};
use base64::{Engine, prelude::BASE64_STANDARD};
use serde_json::{Value, json};

pub(super) fn snapshot(emu: &Emulator) -> Result<Snapshot> {
    if emu.uc.is_none() {
        // Before the first load there are no CPU registers or mapped bytes.
        return Ok(Snapshot {
            thread: emu.threads.tid(),
            registers: Vec::new(),
            modules: Vec::new(),
            memory: Vec::new(),
            heap: Vec::new(),
        });
    }
    emu.snapshot()
}

pub(super) fn diff_value(diff: Diff) -> Value {
    let registers: serde_json::Map<_, _> = diff
        .registers
        .into_iter()
        .map(|(name, before, after)| (name, json!([before.to_string(), after.to_string()])))
        .collect();
    let mappings = |maps: Vec<crate::Mapping>| {
        maps.into_iter()
            .map(|map| {
                json!({
                    "start":map.start.to_string(), "end":map.end.to_string(),
                })
            })
            .collect::<Vec<_>>()
    };
    json!({
        "registers":registers,
        "mapped":mappings(diff.mapped), "unmapped":mappings(diff.unmapped),
        "changes":diff.changes.into_iter().map(|change| json!({
            "address":change.address.to_string(), "location":change.location,
            "size":change.after.len(), "after":BASE64_STANDARD.encode(change.after),
        })).collect::<Vec<_>>(),
    })
}
