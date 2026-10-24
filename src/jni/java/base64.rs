use super::*;
use crate::PrimitiveType;
use ::base64::{
    Engine, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};

pub(super) fn call(
    emu: &mut Emulator,
    call: &JniCall<'_>,
    function: &str,
) -> Result<Option<JniValue>> {
    if call.class != "android/util/Base64" {
        return Ok(None);
    }
    if !matches!(
        (call.name, call.signature),
        ("encodeToString", "([BI)Ljava/lang/String;")
            | ("encode", "([BI)[B")
            | ("decode", "(Ljava/lang/String;I)[B" | "([BI)[B")
    ) {
        return Ok(None);
    }
    let flags = call.args[1].int();
    let alphabet = if flags & 8 != 0 {
        &alphabet::URL_SAFE
    } else {
        &alphabet::STANDARD
    };
    let engine = GeneralPurpose::new(
        alphabet,
        GeneralPurposeConfig::new()
            .with_encode_padding(flags & 1 == 0)
            .with_decode_padding_mode(DecodePaddingMode::Indifferent)
            .with_decode_allow_trailing_bits(true),
    );
    let handle = call.args[0].object();
    let input = if call.signature.starts_with("(Ljava/lang/String;") {
        charset::Charset::Utf8.encode(&units(emu, handle)?)
    } else {
        array(emu, handle, function)?
    };
    let output = if call.name == "decode" {
        // Android's SKIP table covers every non-alphabet byte, not just whitespace.
        let clean: Vec<_> = input
            .into_iter()
            .filter(|b| *b == b'=' || alphabet.as_str().as_bytes().contains(b))
            .collect();
        let padding = clean.iter().position(|&b| b == b'=');
        if padding.is_some() && clean.len() % 4 != 0 {
            return Err(exception(
                "java/lang/IllegalArgumentException",
                Some("bad base-64".into()),
            ));
        }
        engine.decode(clean).map_err(|_| {
            exception(
                "java/lang/IllegalArgumentException",
                Some("bad base-64".into()),
            )
        })?
    } else {
        let encoded = engine.encode(input);
        if flags & 2 != 0 {
            encoded.into_bytes()
        } else {
            let mut bytes = Vec::new();
            for line in encoded.as_bytes().chunks(76) {
                bytes.extend(line);
                if flags & 4 != 0 {
                    bytes.push(b'\r');
                }
                bytes.push(b'\n');
            }
            bytes
        }
    };
    Ok(Some(if call.name == "encodeToString" {
        string(emu, output.iter().map(|&b| b as u16).collect::<Vec<_>>())
    } else {
        emu.insert_object(Object::Array(PrimitiveType::Byte, output))
            .into()
    }))
}
