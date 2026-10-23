#[derive(Clone, Copy)]
pub(in crate::jni) enum Charset {
    Utf8,
    Ascii,
    Latin1,
    Utf16Le,
    Utf16Be,
}

fn decode_utf8(bytes: &[u8]) -> Vec<u16> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let first = bytes[i];
        if first < 128 {
            result.push(first as u16);
            i += 1;
            continue;
        }
        let length = match first {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => {
                result.push(0xfffd);
                i += 1;
                continue;
            }
        };
        let mut consumed = 1;
        while consumed < length && i + consumed < bytes.len() {
            let next = bytes[i + consumed];
            if next & 0xc0 != 0x80
                || (consumed == 1
                    && ((first == 0xe0 && next < 0xa0)
                        || (first == 0xf0 && next < 0x90)
                        || (first == 0xf4 && next > 0x8f)))
            {
                break;
            }
            consumed += 1;
        }
        if consumed != length {
            result.push(0xfffd);
            i += consumed;
            continue;
        }
        let mut code = (first & (0x7f >> length)) as u32;
        for &b in &bytes[i + 1..i + length] {
            code = (code << 6) | (b & 63) as u32;
        }
        // Java treats a complete UTF-8 encoding of a surrogate as one malformed sequence.
        result.extend(
            char::from_u32(code)
                .unwrap_or('\u{fffd}')
                .encode_utf16(&mut [0; 2])
                .iter()
                .copied(),
        );
        i += length;
    }
    result
}
impl Charset {
    pub(super) fn parse(name: &str) -> Option<Self> {
        Some(
            match name.to_ascii_lowercase().replace(['-', '_'], "").as_str() {
                "utf8" => Self::Utf8,
                "usascii" => Self::Ascii,
                "iso88591" => Self::Latin1,
                "utf16le" => Self::Utf16Le,
                "utf16be" => Self::Utf16Be,
                _ => return None,
            },
        )
    }
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Ascii => "US-ASCII",
            Self::Latin1 => "ISO-8859-1",
            Self::Utf16Le => "UTF-16LE",
            Self::Utf16Be => "UTF-16BE",
        }
    }
    pub(super) fn decode(self, bytes: &[u8]) -> Vec<u16> {
        match self {
            Self::Utf8 => decode_utf8(bytes),
            Self::Ascii => bytes
                .iter()
                .map(|&b| if b < 128 { b as u16 } else { 0xfffd })
                .collect(),
            Self::Latin1 => bytes.iter().map(|&b| b as u16).collect(),
            Self::Utf16Le | Self::Utf16Be => {
                let (pairs, rest) = bytes.as_chunks::<2>();
                let mut raw = pairs.iter().map(|&b| {
                    if matches!(self, Self::Utf16Le) {
                        u16::from_le_bytes(b)
                    } else {
                        u16::from_be_bytes(b)
                    }
                });
                let mut result = Vec::new();
                let mut incomplete_pair = false;
                while let Some(unit) = raw.next() {
                    if (0xd800..=0xdbff).contains(&unit) {
                        match raw.next() {
                            Some(low) if (0xdc00..=0xdfff).contains(&low) => {
                                result.extend([unit, low])
                            }
                            Some(_) => result.push(0xfffd),
                            None => {
                                result.push(0xfffd);
                                incomplete_pair = true;
                            }
                        }
                    } else {
                        result.push(if (0xdc00..=0xdfff).contains(&unit) {
                            0xfffd
                        } else {
                            unit
                        });
                    }
                }
                if !rest.is_empty() && !incomplete_pair {
                    result.push(0xfffd);
                }
                result
            }
        }
    }
    pub(super) fn encode(self, units: &[u16]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for c in char::decode_utf16(units.iter().copied()) {
            match self {
                Self::Utf8 => match c {
                    Ok(c) => bytes.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
                    Err(_) => bytes.push(b'?'),
                },
                Self::Ascii | Self::Latin1 => {
                    let max = if matches!(self, Self::Ascii) {
                        127
                    } else {
                        255
                    };
                    bytes.push(
                        c.ok()
                            .filter(|&c| c as u32 <= max)
                            .map_or(b'?', |c| c as u8),
                    );
                }
                Self::Utf16Le | Self::Utf16Be => {
                    for &unit in c.unwrap_or('\u{fffd}').encode_utf16(&mut [0; 2]).iter() {
                        bytes.extend(if matches!(self, Self::Utf16Le) {
                            unit.to_le_bytes()
                        } else {
                            unit.to_be_bytes()
                        });
                    }
                }
            }
        }
        bytes
    }
}
