use crate::Result;

const ANDROID: &str = "http://schemas.android.com/apk/res/android";
const NONE: u32 = u32::MAX;

fn bytes(data: &[u8], start: usize, size: usize) -> Result<&[u8]> {
    data.get(start..)
        .and_then(|data| data.get(..size))
        .ok_or_else(|| "truncated binary XML".into())
}
fn u16_at(data: &[u8], offset: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(bytes(data, offset, 2)?.try_into()?))
}
fn u32_at(data: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(bytes(data, offset, 4)?.try_into()?))
}
fn length(data: &[u8], cursor: &mut usize, utf8: bool) -> Result<usize> {
    let width = if utf8 { 1 } else { 2 };
    let read = |at| -> Result<usize> {
        Ok(if utf8 {
            bytes(data, at, 1)?[0] as usize
        } else {
            u16_at(data, at)? as usize
        })
    };
    let first = read(*cursor)?;
    *cursor += width;
    let flag = if utf8 { 0x80 } else { 0x8000 };
    if first & flag == 0 {
        return Ok(first);
    }
    let second = read(*cursor)?;
    *cursor += width;
    Ok(((first & !flag) << (width * 8)) | second)
}

#[derive(Default)]
struct StringPool<'a> {
    offsets: &'a [u8],
    data: &'a [u8],
    utf8: bool,
}
impl<'a> StringPool<'a> {
    fn new(data: &'a [u8], header: usize) -> Result<Self> {
        if header < 28 {
            return Err("short string pool header".into());
        }
        let count = u32_at(data, 8)? as usize;
        let start = u32_at(data, 20)? as usize;
        let offsets = bytes(
            data,
            header,
            count.checked_mul(4).ok_or("string count overflow")?,
        )?;
        Ok(Self {
            offsets,
            data: bytes(
                data,
                start,
                data.len()
                    .checked_sub(start)
                    .ok_or("bad string pool bounds")?,
            )?,
            utf8: u32_at(data, 16)? & 0x100 != 0,
        })
    }
    fn string(&self, index: u32) -> Result<String> {
        let mut cursor = u32_at(self.offsets, index as usize * 4)? as usize;
        let units = length(self.data, &mut cursor, self.utf8)?;
        if self.utf8 {
            let size = length(self.data, &mut cursor, true)?;
            Ok(String::from_utf8_lossy(bytes(self.data, cursor, size)?).into_owned())
        } else {
            let size = units.checked_mul(2).ok_or("UTF-16 length overflow")?;
            let units: Vec<_> = bytes(self.data, cursor, size)?
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| u16::from_le_bytes(*b))
                .collect();
            Ok(String::from_utf16_lossy(&units))
        }
    }
}

fn integer(kind: u8, value: u32, pool: &StringPool<'_>) -> Result<u32> {
    match kind {
        0x10 | 0x11 => Ok(value),
        3 => {
            let text = pool.string(value)?;
            Ok(if let Some(hex) = text.strip_prefix("0x") {
                u32::from_str_radix(hex, 16)?
            } else {
                text.parse()?
            })
        }
        _ => Err(format!("expected integer version code, got type {kind}").into()),
    }
}

pub(super) fn parse(xml: &[u8]) -> Result<(String, Option<String>, i64)> {
    let header = u16_at(xml, 2)? as usize;
    let size = u32_at(xml, 4)? as usize;
    if u16_at(xml, 0)? != 3 || header < 8 || size < header {
        return Err("malformed binary XML header".into());
    }
    let xml = bytes(xml, 0, size)?;
    let mut pool = StringPool::default();
    let mut resources = &[][..];
    let mut cursor = header;
    while cursor < xml.len() {
        let kind = u16_at(xml, cursor)?;
        let header = u16_at(xml, cursor + 2)? as usize;
        let size = u32_at(xml, cursor + 4)? as usize;
        if header < 8 || size < header {
            return Err("malformed binary XML chunk".into());
        }
        let chunk = bytes(xml, cursor, size)?;
        match kind {
            1 => pool = StringPool::new(chunk, header)?,
            0x180 => resources = &chunk[header..],
            0x102 => {
                if header < 16 {
                    return Err("short element header".into());
                }
                if pool.string(u32_at(chunk, header + 4)?)? != "manifest" {
                    return Err("expected root manifest element".into());
                }
                return attributes(chunk, header, &pool, resources);
            }
            _ => (),
        }
        cursor += size;
    }
    Err("no manifest element".into())
}

fn attributes(
    chunk: &[u8],
    header: usize,
    pool: &StringPool<'_>,
    resources: &[u8],
) -> Result<(String, Option<String>, i64)> {
    let start = u16_at(chunk, header + 8)? as usize;
    let stride = u16_at(chunk, header + 10)? as usize;
    let count = u16_at(chunk, header + 12)? as usize;
    if start < 20 || stride < 20 {
        return Err("malformed manifest attributes".into());
    }
    let mut package = None;
    let mut version_name = None;
    let mut code = 0;
    let mut major = 0;
    for attr in bytes(chunk, header + start, count * stride)?.chunks_exact(stride) {
        let namespace = u32_at(attr, 0)?;
        let index = u32_at(attr, 4)?;
        let id = resources
            .as_chunks::<4>()
            .0
            .get(index as usize)
            .map_or(0, |id| u32::from_le_bytes(*id));
        let field = if id != 0 {
            match id {
                0x0101021b => 2,
                0x01010576 => 3,
                0x0101021c => 4,
                _ => 0,
            }
        } else if namespace == NONE {
            if pool.string(index)? == "package" {
                1
            } else {
                0
            }
        } else if pool.string(namespace)? == ANDROID {
            match pool.string(index)?.as_str() {
                "versionCode" => 2,
                "versionCodeMajor" => 3,
                "versionName" => 4,
                _ => 0,
            }
        } else {
            0
        };
        if field == 0 {
            continue;
        }
        let kind = attr[15];
        let value = u32_at(attr, 16)?;
        match field {
            1 if kind == 3 => package = Some(pool.string(value)?),
            1 => return Err("package must be a string".into()),
            2 => code = integer(kind, value, pool)?,
            3 => major = integer(kind, value, pool)?,
            4 => {
                version_name = match kind {
                    3 => Some(pool.string(value)?),
                    0x10 | 0x11 => Some(value.to_string()),
                    1 => None,
                    _ => return Err(format!("unsupported versionName type {kind}").into()),
                }
            }
            _ => unreachable!(),
        }
    }
    Ok((
        package.ok_or("manifest has no package")?,
        version_name,
        ((u64::from(major) << 32) | u64::from(code)) as i64,
    ))
}
