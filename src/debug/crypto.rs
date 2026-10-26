use crate::{Emulator, Result};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CryptoConstant {
    pub name: &'static str,
    pub address: u64,
    pub location: String,
}

struct Signature {
    name: &'static str,
    pattern: &'static [u8],
}

const SIGNATURES: &[Signature] = &[
    Signature {
        name: "MD5 T table",
        pattern: b"\x78\xa4\x6a\xd7\x56\xb7\xc7\xe8\xdb\x70\x20\x24\xee\xce\xbd\xc1",
    },
    Signature {
        name: "MD5/SHA-1 initial state",
        pattern: b"\x01\x23\x45\x67\x89\xab\xcd\xef\xfe\xdc\xba\x98\x76\x54\x32\x10",
    },
    Signature {
        name: "SHA-1 K",
        pattern: b"\x99\x79\x82\x5a\xa1\xeb\xd9\x6e\xdc\xbc\x1b\x8f\xd6\xc1\x62\xca",
    },
    Signature {
        name: "SHA-256 initial state",
        pattern: b"\x67\xe6\x09\x6a\x85\xae\x67\xbb\x72\xf3\x6e\x3c\x3a\xf5\x4f\xa5",
    },
    Signature {
        name: "SHA-256 K table",
        pattern: b"\x98\x2f\x8a\x42\x91\x44\x37\x71\xcf\xfb\xc0\xb5\xa5\xdb\xb5\xe9",
    },
    Signature {
        name: "SHA-512 initial state",
        pattern: b"\x08\xc9\xbc\xf3\x67\xe6\x09\x6a\x3b\xa7\xca\x84\x85\xae\x67\xbb",
    },
    Signature {
        name: "SHA-512 K table",
        pattern: b"\x22\xae\x28\xd7\x98\x2f\x8a\x42\xcd\x65\xef\x23\x91\x44\x37\x71",
    },
    Signature {
        name: "SM3 initial state",
        pattern: b"\x6f\x16\x80\x73\xb9\xb2\x14\x49\xd7\x42\x24\x17\x00\x06\x8a\xda",
    },
    Signature {
        name: "AES S-box",
        pattern: b"\x63\x7c\x77\x7b\xf2\x6b\x6f\xc5\x30\x01\x67\x2b\xfe\xd7\xab\x76",
    },
    Signature {
        name: "AES inverse S-box",
        pattern: b"\x52\x09\x6a\xd5\x30\x36\xa5\x38\xbf\x40\xa3\x9e\x81\xf3\xd7\xfb",
    },
    Signature {
        name: "AES T-table",
        pattern: b"\xa5\x63\x63\xc6\x84\x7c\x7c\xf8\x99\x77\x77\xee\x8d\x7b\x7b\xf6",
    },
    Signature {
        name: "SM4 S-box",
        pattern: b"\xd6\x90\xe9\xfe\xcc\xe1\x3d\xb7\x16\xb6\x14\xc2\x28\xfb\x2c\x05",
    },
    Signature {
        name: "CRC32 table",
        pattern: b"\x00\x00\x00\x00\x96\x30\x07\x77\x2c\x61\x0e\xee\xba\x51\x09\x99",
    },
    Signature {
        name: "Blowfish P-array",
        pattern: b"\x88\x6a\x3f\x24\xd3\x08\xa3\x85\x2e\x8a\x19\x13\x44\x73\x70\x03",
    },
    Signature {
        name: "ChaCha/Salsa constant",
        pattern: b"expand 32-byte k",
    },
    Signature {
        name: "Base64 alphabet",
        pattern: b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
    },
];

impl Emulator {
    pub fn crypto_constants(&self) -> Result<Vec<CryptoConstant>> {
        let runs: Vec<_> = self.read_mappings(self.maps()?).collect::<Result<_>>()?;
        let mut constants = Vec::new();
        for signature in SIGNATURES {
            for address in super::search::search_runs(
                signature.pattern,
                runs.iter()
                    .map(|(mapping, bytes)| Ok((mapping.start, bytes))),
            )? {
                constants.push(CryptoConstant {
                    name: signature.name,
                    address,
                    location: self.memory_location(
                        address,
                        runs.iter().map(|(mapping, _)| mapping),
                        false,
                    ),
                });
            }
        }
        constants.sort_by_key(|constant| constant.address);
        Ok(constants)
    }
}
