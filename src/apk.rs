use crate::{Emulator, Result};
use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

mod archive;
pub(crate) mod libraries;
mod manifest;
mod signatures;
#[cfg(test)]
mod tests;

/// Metadata from one installed APK. Certificates retain their original DER bytes.
#[derive(Clone, Debug)]
pub struct Apk {
    pub path: PathBuf,
    pub package: String,
    pub version_name: Option<String>,
    pub version_code: i64,
    pub signatures: Vec<Vec<u8>>,
    pub signers: Vec<Vec<u8>>,
    pub signing_history: Vec<Vec<u8>>,
    archives: Rc<archive::Archives>,
}

impl Apk {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let parse = || -> Result<Self> {
            let path = path.canonicalize()?;
            let archive = Rc::new(archive::Archive::open(&path)?);
            let xml = archive.read("AndroidManifest.xml")?;
            let (package, version_name, version_code) =
                manifest::parse(&xml).map_err(|error| format!("AndroidManifest.xml: {error}"))?;
            let signatures = signatures::read(&archive)?;
            Ok(Self {
                path,
                package,
                version_name,
                version_code,
                signatures: signatures.signatures,
                signers: signatures.signers,
                signing_history: signatures.signing_history,
                archives: Rc::new(archive::Archives::new(archive)),
            })
        };
        parse().map_err(|error| format!("APK {}: {error}", path.display()).into())
    }
}

impl Emulator {
    pub fn apk(&self) -> Option<&Apk> {
        self.apk.as_deref()
    }

    /// Change metadata. Changing path or package does not move existing guest files.
    pub fn apk_mut(&mut self) -> Option<&mut Apk> {
        self.apk.as_mut().map(Rc::make_mut)
    }
}
