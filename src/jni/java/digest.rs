use super::*;
use crate::PrimitiveType;
use md5::Digest as _;

#[derive(Clone)]
pub(in crate::jni) struct Digest {
    algorithm: String,
    state: State,
}
macro_rules! hashes {
    ($($variant:ident($ty:ty)),* $(,)?) => {
        #[derive(Clone)]
        enum State { $($variant($ty)),* }
        impl State {
            fn update(&mut self, bytes: &[u8]) { match self { $(Self::$variant(state) => state.update(bytes)),* } }
            fn digest(&mut self) -> Vec<u8> { match self { $(Self::$variant(state) => state.finalize_reset().to_vec()),* } }
            fn reset(&mut self) { match self { $(Self::$variant(state) => *state = <$ty>::new()),* } }
        }
    };
}
hashes! { Md5(md5::Md5), Sha1(sha1::Sha1), Sha224(sha2::Sha224), Sha256(sha2::Sha256), Sha384(sha2::Sha384), Sha512(sha2::Sha512) }
pub(super) fn static_call(emu: &mut Emulator, call: &JniCall<'_>) -> Result<Option<JniValue>> {
    if (call.class, call.name, call.signature)
        != (
            "java/security/MessageDigest",
            "getInstance",
            "(Ljava/lang/String;)Ljava/security/MessageDigest;",
        )
    {
        return Ok(None);
    }
    let algorithm = String::from_utf16_lossy(&units(emu, call.args[0].object())?);
    let state = match algorithm.to_ascii_uppercase().as_str() {
        "MD5" => State::Md5(md5::Md5::new()),
        "SHA" | "SHA1" | "SHA-1" => State::Sha1(sha1::Sha1::new()),
        "SHA-224" => State::Sha224(sha2::Sha224::new()),
        "SHA-256" => State::Sha256(sha2::Sha256::new()),
        "SHA-384" => State::Sha384(sha2::Sha384::new()),
        "SHA-512" => State::Sha512(sha2::Sha512::new()),
        _ => return Ok(None),
    };
    Ok(Some(
        emu.insert_object(Object::Digest(Box::new(Digest { algorithm, state })))
            .into(),
    ))
}
pub(super) fn call(
    emu: &mut Emulator,
    call: &JniCall<'_>,
    receiver: &Rc<RefCell<Object>>,
    function: &str,
) -> Result<Option<JniValue>> {
    let a = call.args;
    let bytes = match (call.name, call.signature) {
        ("update", "([B)V" | "([BII)V") | ("digest", "([B)[B") => {
            if call.signature == "([BII)V" && a[0].object().0 == 0 {
                return Err(exception(
                    "java/lang/IllegalArgumentException",
                    Some("No input buffer given".into()),
                ));
            }
            let bytes = array(emu, a[0].object(), function)?;
            if a.len() == 3 {
                let (start, length) = (a[1].int(), a[2].int());
                if start as i64 + length as i64 > bytes.len() as i64 {
                    return Err(exception(
                        "java/lang/IllegalArgumentException",
                        Some("Input buffer too short".into()),
                    ));
                }
                if start < 0 || length < 0 {
                    return Err(exception("java/lang/ArrayIndexOutOfBoundsException", None));
                }
                Some(bytes[start as usize..start as usize + length as usize].to_vec())
            } else {
                Some(bytes)
            }
        }
        ("update", "(B)V") => Some(vec![a[0].bits() as u8]),
        _ => None,
    };
    let Object::Digest(digest) = &mut *receiver.borrow_mut() else {
        unreachable!()
    };
    let result = match (call.name, call.signature) {
        ("update", "([B)V" | "([BII)V" | "(B)V") => {
            digest.state.update(&bytes.unwrap());
            JniValue::Void
        }
        ("digest", "()[B" | "([B)[B") => {
            if let Some(bytes) = bytes {
                digest.state.update(&bytes);
            }
            emu.insert_object(Object::Array(PrimitiveType::Byte, digest.state.digest()))
                .into()
        }
        ("reset", "()V") => {
            digest.state.reset();
            JniValue::Void
        }
        ("getAlgorithm", "()Ljava/lang/String;") => {
            string(emu, digest.algorithm.encode_utf16().collect::<Vec<_>>())
        }
        _ => return Ok(None),
    };
    Ok(Some(result))
}
