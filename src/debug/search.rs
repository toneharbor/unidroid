use crate::{Emulator, Result};

pub const SEARCH_LIMIT: usize = 1000;

impl Emulator {
    pub fn search(&self, pattern: &[u8]) -> Result<Vec<u64>> {
        search_runs(
            pattern,
            self.maps()?.into_iter().map(|mapping| {
                Ok((
                    mapping.start,
                    self.read_bytes(mapping.start, (mapping.end - mapping.start) as usize)?,
                ))
            }),
        )
    }
}

/// Search address-ordered runs, preserving partial matches only across adjacent runs.
pub(crate) fn search_runs<B: AsRef<[u8]>>(
    pattern: &[u8],
    runs: impl IntoIterator<Item = Result<(u64, B)>>,
) -> Result<Vec<u64>> {
    if pattern.is_empty() {
        return Err("empty search pattern".into());
    }
    // Prefix lengths let overlapping matches continue without rereading memory.
    let mut prefixes = vec![0; pattern.len()];
    let mut matched = 0;
    for i in 1..pattern.len() {
        while matched > 0 && pattern[i] != pattern[matched] {
            matched = prefixes[matched - 1];
        }
        if pattern[i] == pattern[matched] {
            matched += 1;
        }
        prefixes[i] = matched;
    }
    let mut addresses = Vec::new();
    let mut end = None;
    matched = 0;
    for run in runs {
        let (start, bytes) = run?;
        let bytes = bytes.as_ref();
        if end != Some(start) {
            matched = 0;
        }
        for (offset, &byte) in bytes.iter().enumerate() {
            while matched > 0 && byte != pattern[matched] {
                matched = prefixes[matched - 1];
            }
            if byte == pattern[matched] {
                matched += 1;
            }
            if matched == pattern.len() {
                addresses.push(start + offset as u64 + 1 - pattern.len() as u64);
                if addresses.len() == SEARCH_LIMIT {
                    return Ok(addresses);
                }
                matched = prefixes[matched - 1];
            }
        }
        end = Some(start + bytes.len() as u64);
    }
    Ok(addresses)
}
