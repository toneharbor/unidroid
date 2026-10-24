pub const ANDROID: &str = "http://schemas.android.com/apk/res/android";

pub enum Value<'a> {
    String(&'a str),
    Int(u32),
    Hex(u32),
    Reference(u32),
}

pub struct Attribute<'a> {
    pub namespace: Option<&'a str>,
    pub name: &'a str,
    pub id: u32,
    pub value: Value<'a>,
}

pub fn package() -> Attribute<'static> {
    Attribute {
        namespace: None,
        name: "package",
        id: 0,
        value: Value::String("com.example.apk"),
    }
}

pub fn android<'a>(name: &'a str, id: u32, value: Value<'a>) -> Attribute<'a> {
    Attribute {
        namespace: Some(ANDROID),
        name,
        id,
        value,
    }
}

fn chunk(kind: u16, header: u16, body: Vec<u8>) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(kind.to_le_bytes());
    bytes.extend(header.to_le_bytes());
    bytes.extend(((body.len() + 8) as u32).to_le_bytes());
    bytes.extend(body);
    bytes
}

fn length8(bytes: &mut Vec<u8>, length: usize) {
    if length > 127 {
        bytes.push(0x80 | (length >> 8) as u8);
    }
    bytes.push(length as u8);
}

fn length16(bytes: &mut Vec<u8>, length: usize) {
    if length > 32767 {
        bytes.extend((0x8000 | (length >> 16) as u16).to_le_bytes());
    }
    bytes.extend((length as u16).to_le_bytes());
}

pub fn xml(utf8: bool, attributes: &[Attribute<'_>]) -> Vec<u8> {
    let mut strings = vec!["manifest"];
    let mut ids = vec![0];
    let mut attrs = Vec::new();
    for attr in attributes {
        let namespace = match attr.namespace {
            Some(ns) => {
                strings.push(ns);
                ids.push(0);
                (strings.len() - 1) as u32
            }
            None => u32::MAX,
        };
        strings.push(attr.name);
        ids.push(attr.id);
        let name = (strings.len() - 1) as u32;
        let (kind, data) = match attr.value {
            Value::String(text) => {
                strings.push(text);
                ids.push(0);
                (3, (strings.len() - 1) as u32)
            }
            Value::Int(n) => (0x10, n),
            Value::Hex(n) => (0x11, n),
            Value::Reference(n) => (1, n),
        };
        for n in [namespace, name, u32::MAX] {
            attrs.extend(n.to_le_bytes());
        }
        attrs.extend(8u16.to_le_bytes());
        attrs.extend([0, kind]);
        attrs.extend(data.to_le_bytes());
    }
    let mut offsets = Vec::new();
    let mut data = Vec::new();
    for string in &strings {
        offsets.extend((data.len() as u32).to_le_bytes());
        let utf16: Vec<_> = string.encode_utf16().collect();
        if utf8 {
            length8(&mut data, utf16.len());
            length8(&mut data, string.len());
            data.extend(string.as_bytes());
            data.push(0);
        } else {
            length16(&mut data, utf16.len());
            for unit in utf16 {
                data.extend(unit.to_le_bytes());
            }
            data.extend([0, 0]);
        }
    }
    while !data.len().is_multiple_of(4) {
        data.push(0);
    }
    let mut pool = Vec::new();
    for n in [
        strings.len() as u32,
        0,
        if utf8 { 0x100 } else { 0 },
        28 + strings.len() as u32 * 4,
        0,
    ] {
        pool.extend(n.to_le_bytes());
    }
    pool.extend(offsets);
    pool.extend(data);
    let mut body = chunk(1, 28, pool);
    body.extend(chunk(
        0x180,
        8,
        ids.iter().flat_map(|n| n.to_le_bytes()).collect(),
    ));
    let mut start = Vec::new();
    for n in [1u32, u32::MAX, u32::MAX, 0] {
        start.extend(n.to_le_bytes());
    }
    for n in [20, 20, attributes.len() as u16, 0, 0, 0] {
        start.extend(n.to_le_bytes());
    }
    start.extend(attrs);
    body.extend(chunk(0x102, 16, start));
    let end = [1u32, u32::MAX, u32::MAX, 0]
        .iter()
        .flat_map(|n| n.to_le_bytes())
        .collect();
    body.extend(chunk(0x103, 16, end));
    chunk(3, 8, body)
}
