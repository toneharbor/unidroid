#![allow(dead_code)]

use std::{
    fs::File,
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

#[path = "apk/manifest.rs"]
pub mod manifest;
#[path = "apk/signatures.rs"]
pub mod signatures;

pub struct TestApk {
    pub path: PathBuf,
}

impl TestApk {
    pub fn new(entries: &[(&str, &[u8], CompressionMethod, u16)]) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "target/test-apks/{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory)?;
        let apk = Self {
            path: directory.join("base.apk"),
        };
        apk.write(entries)?;
        Ok(apk)
    }

    pub fn write(&self, entries: &[(&str, &[u8], CompressionMethod, u16)]) -> Result<()> {
        let mut zip = ZipWriter::new(File::create(&self.path)?);
        for &(name, bytes, method, alignment) in entries {
            zip.start_file(
                name,
                SimpleFileOptions::default()
                    .compression_method(method)
                    .with_alignment(alignment),
            )?;
            zip.write_all(bytes)?;
        }
        zip.finish()?;
        Ok(())
    }

    pub fn assets(entries: &[(&str, &[u8], CompressionMethod)]) -> Result<Self> {
        let xml = manifest::xml(true, &[manifest::package()]);
        let names: Vec<_> = entries
            .iter()
            .map(|(name, ..)| format!("assets/{name}"))
            .collect();
        let mut files = vec![(
            "AndroidManifest.xml",
            xml.as_slice(),
            CompressionMethod::Stored,
            1,
        )];
        files.extend(
            entries
                .iter()
                .zip(&names)
                .map(|((_, bytes, method), name)| (name.as_str(), *bytes, *method, 1)),
        );
        Self::new(&files)
    }

    pub fn manifest(bytes: &[u8]) -> Result<Self> {
        Self::new(&[("AndroidManifest.xml", bytes, CompressionMethod::Stored, 1)])
    }
}

impl Drop for TestApk {
    fn drop(&mut self) {
        std::fs::remove_dir_all(self.path.parent().unwrap()).unwrap();
    }
}
