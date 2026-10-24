pub fn der(tag: u8, value: &[u8]) -> Vec<u8> {
    let mut bytes = vec![tag];
    if value.len() < 128 {
        bytes.push(value.len() as u8);
    } else {
        let length = (value.len() as u32).to_be_bytes();
        let start = length.iter().position(|n| *n != 0).unwrap();
        bytes.push(0x80 | (4 - start) as u8);
        bytes.extend(&length[start..]);
    }
    bytes.extend(value);
    bytes
}

pub fn pkcs7(certificates: &[Vec<u8>]) -> Vec<u8> {
    let mut signed = der(2, &[1]);
    signed.extend(der(0x31, &[]));
    signed.extend(der(
        0x30,
        &der(6, &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 1, 7, 1]),
    ));
    signed.extend(der(0xa0, &certificates.concat()));
    signed.extend(der(0x31, &[]));
    let mut content = der(6, &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 1, 7, 2]);
    content.extend(der(0xa0, &der(0x30, &signed)));
    der(0x30, &content)
}

fn prefixed(value: &[u8]) -> Vec<u8> {
    let mut bytes = (value.len() as u32).to_le_bytes().to_vec();
    bytes.extend(value);
    bytes
}

pub fn signers(v3: bool, certificates: &[Vec<Vec<u8>>]) -> Vec<u8> {
    signers_with_attributes(v3, certificates, &[], false)
}

pub fn signers_with_attributes(
    v3: bool,
    certificates: &[Vec<Vec<u8>>],
    attributes: &[u8],
    trailing: bool,
) -> Vec<u8> {
    let tail: &[u8] = if trailing {
        b"ignored trailing bytes"
    } else {
        &[]
    };
    let mut signers = Vec::new();
    for chain in certificates {
        let mut signed = prefixed(&[]); // digests
        signed.extend(prefixed(
            &chain
                .iter()
                .flat_map(|cert| prefixed(cert))
                .collect::<Vec<_>>(),
        ));
        if v3 {
            signed.extend([28u32, 35].iter().flat_map(|n| n.to_le_bytes()));
        }
        signed.extend(prefixed(attributes));
        signed.extend(tail);
        let mut signer = prefixed(&signed);
        if v3 {
            signer.extend([28u32, 35].iter().flat_map(|n| n.to_le_bytes()));
        }
        signer.extend(prefixed(&[])); // signatures
        signer.extend(prefixed(&[])); // public key
        signer.extend(tail);
        signers.extend(prefixed(&signer));
    }
    let mut block = prefixed(&signers);
    block.extend(tail);
    block
}

pub fn rotation(certificates: &[Vec<u8>]) -> Vec<u8> {
    let mut proof = 1u32.to_le_bytes().to_vec();
    for cert in certificates {
        let mut signed = prefixed(cert);
        signed.extend(0x0103u32.to_le_bytes());
        let mut node = prefixed(&signed);
        node.extend(0u32.to_le_bytes());
        node.extend(0x0103u32.to_le_bytes());
        node.extend(prefixed(b"signature"));
        proof.extend(prefixed(&node));
    }
    let mut attribute = 0x3ba06f8cu32.to_le_bytes().to_vec();
    attribute.extend(proof);
    prefixed(&attribute)
}

pub fn insert_block(path: &std::path::Path, pairs: &[(u32, Vec<u8>)]) -> super::Result<()> {
    let mut bytes = std::fs::read(path)?;
    let eocd = bytes.len() - 22;
    let directory = u32::from_le_bytes(bytes[eocd + 16..eocd + 20].try_into()?) as usize;
    let mut entries = Vec::new();
    for (id, value) in pairs {
        entries.extend((value.len() as u64 + 4).to_le_bytes());
        entries.extend(id.to_le_bytes());
        entries.extend(value);
    }
    let size = entries.len() as u64 + 24;
    let mut block = size.to_le_bytes().to_vec();
    block.extend(entries);
    block.extend(size.to_le_bytes());
    block.extend(b"APK Sig Block 42");
    bytes[eocd + 16..eocd + 20].copy_from_slice(&((directory + block.len()) as u32).to_le_bytes());
    bytes.splice(directory..directory, block);
    std::fs::write(path, bytes)?;
    Ok(())
}
