use super::archive::Archive;
use crate::Result;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

fn take<'a>(bytes: &mut &'a [u8], size: usize) -> Result<&'a [u8]> {
    if size > bytes.len() {
        return Err("truncated signature block".into());
    }
    let (value, rest) = bytes.split_at(size);
    *bytes = rest;
    Ok(value)
}

fn header(bytes: &mut &[u8]) -> Result<(u8, Option<usize>)> {
    let tag = take(bytes, 1)?[0];
    let first = take(bytes, 1)?[0];
    if first == 0x80 {
        if tag & 0x20 == 0 {
            return Err("indefinite primitive BER value".into());
        }
        return Ok((tag, None));
    }
    let size = if first & 0x80 == 0 {
        first as usize
    } else {
        let width = (first & 0x7f) as usize;
        if width > std::mem::size_of::<usize>() {
            return Err("bad BER length".into());
        }
        take(bytes, width)?
            .iter()
            .fold(0usize, |n, b| (n << 8) | *b as usize)
    };
    Ok((tag, Some(size)))
}

// Entering a container never walks its unused tail, including for indefinite BER.
fn enter<'a>(bytes: &mut &'a [u8], expected: u8) -> Result<&'a [u8]> {
    let (tag, size) = header(bytes)?;
    if tag != expected {
        return Err(format!("expected BER tag {expected:#x}, got {tag:#x}").into());
    }
    match size {
        Some(size) => take(bytes, size),
        None => Ok(*bytes),
    }
}

fn der<'a>(bytes: &mut &'a [u8]) -> Result<&'a [u8]> {
    let original = *bytes;
    let (_, size) = header(bytes)?;
    if let Some(size) = size {
        take(bytes, size)?;
    } else {
        let mut depth = 1;
        while depth != 0 {
            if bytes.starts_with(&[0, 0]) {
                take(bytes, 2)?;
                depth -= 1;
            } else {
                let (_, size) = header(bytes)?;
                if let Some(size) = size {
                    take(bytes, size)?;
                } else {
                    depth += 1;
                }
            }
        }
    }
    Ok(&original[..original.len() - bytes.len()])
}

fn pkcs7(mut bytes: &[u8]) -> Result<Vec<Vec<u8>>> {
    let mut content = enter(&mut bytes, 0x30)?;
    if enter(&mut content, 6)? != [0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 1, 7, 2] {
        return Err("PKCS#7 content is not SignedData".into());
    }
    let mut wrapped = enter(&mut content, 0xa0)?;
    let mut signed = enter(&mut wrapped, 0x30)?;
    der(&mut signed)?; // version
    der(&mut signed)?; // digestAlgorithms
    der(&mut signed)?; // inner ContentInfo
    let mut certificates = Vec::new();
    if signed.first() == Some(&0xa0) {
        let mut values = enter(&mut signed, 0xa0)?;
        while !values.is_empty() && !values.starts_with(&[0, 0]) {
            certificates.push(der(&mut values)?.to_vec());
        }
    }
    Ok(certificates)
}

#[derive(Default)]
pub(super) struct Signing {
    pub signatures: Vec<Vec<u8>>,
    pub signers: Vec<Vec<u8>>,
    pub signing_history: Vec<Vec<u8>>,
}

pub(super) fn read(archive: &Archive) -> Result<Signing> {
    let mut certificates = Vec::new();
    let mut v1 = false;
    let count = archive.zip.borrow().len();
    for index in 0..count {
        let name = archive
            .zip
            .borrow()
            .by_index_data(index)?
            .name()?
            .into_owned();
        let upper = name.to_ascii_uppercase();
        if let Some(file) = upper.strip_prefix("META-INF/")
            && !file.contains('/')
            && [".RSA", ".DSA", ".EC"]
                .iter()
                .any(|ext| file.ends_with(ext))
        {
            v1 = true;
            let bytes = archive.read(&name)?;
            certificates
                .extend(pkcs7(&bytes).map_err(|error| format!("signature block {name}: {error}"))?);
        }
    }
    let offset = archive.zip.borrow().central_directory_start();
    let mut signing = signing_block(archive.file.try_clone()?, offset)
        .map_err(|error| format!("APK signature block: {error}"))?
        .unwrap_or_else(|| Signing {
            signers: certificates.clone(),
            ..Signing::default()
        });
    signing.signatures = if v1 {
        certificates
    } else if let Some(oldest) = signing.signing_history.first() {
        vec![oldest.clone()]
    } else {
        signing.signers.clone()
    };
    Ok(signing)
}

fn prefixed<'a>(bytes: &mut &'a [u8]) -> Result<&'a [u8]> {
    let size = u32::from_le_bytes(take(bytes, 4)?.try_into()?) as usize;
    take(bytes, size)
}

fn signers(mut bytes: &[u8], v3: bool) -> Result<Signing> {
    let mut signers = prefixed(&mut bytes)?;
    let mut result = Signing::default();
    while !signers.is_empty() {
        let mut signer = prefixed(&mut signers)?;
        let mut signed = prefixed(&mut signer)?;
        prefixed(&mut signed)?; // digests
        let mut chain = prefixed(&mut signed)?;
        result.signers.push(prefixed(&mut chain)?.to_vec());
        if v3 {
            take(&mut signed, 8)?; // minimum and maximum SDK
            result.signing_history = history(prefixed(&mut signed)?)?;
            break;
        }
    }
    Ok(result)
}

fn history(mut attributes: &[u8]) -> Result<Vec<Vec<u8>>> {
    while !attributes.is_empty() {
        let mut attribute = prefixed(&mut attributes)?;
        let id = u32::from_le_bytes(take(&mut attribute, 4)?.try_into()?);
        if id != 0x3ba06f8c {
            continue;
        }
        take(&mut attribute, 4)?; // proof-of-rotation version
        let mut certificates = Vec::new();
        while !attribute.is_empty() {
            let mut node = prefixed(&mut attribute)?;
            let mut signed = prefixed(&mut node)?;
            certificates.push(prefixed(&mut signed)?.to_vec());
        }
        return Ok(certificates);
    }
    Ok(Vec::new())
}

fn signing_block(mut file: File, directory: u64) -> Result<Option<Signing>> {
    if directory < 24 {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(directory - 24))?;
    let mut footer = [0; 24];
    file.read_exact(&mut footer)?;
    if &footer[8..] != b"APK Sig Block 42" {
        return Ok(None);
    }
    let size = u64::from_le_bytes(footer[..8].try_into()?);
    if size < 24 || size > directory - 8 {
        return Err("invalid signing block size".into());
    }
    file.seek(SeekFrom::Start(directory - size - 8))?;
    let mut block = vec![0; usize::try_from(size - 16)?];
    file.read_exact(&mut block)?;
    let mut pairs = block.as_slice();
    take(&mut pairs, 8)?; // size header; the footer supplies the bounds we need
    let mut v2 = None;
    let mut v3 = None;
    while !pairs.is_empty() {
        let length = u64::from_le_bytes(take(&mut pairs, 8)?.try_into()?);
        let mut pair = take(&mut pairs, usize::try_from(length)?)?;
        let id = u32::from_le_bytes(take(&mut pair, 4)?.try_into()?);
        match id {
            0x7109871a => v2 = Some(pair),
            0xf05368c0 => v3 = Some(pair),
            _ => (),
        }
    }
    if let Some(bytes) = v3 {
        signers(bytes, true).map(Some)
    } else if let Some(bytes) = v2 {
        signers(bytes, false).map(Some)
    } else {
        Ok(None)
    }
}
