use crate::Result;
use memmap2::{Mmap, MmapOptions};
use std::{
    cell::RefCell,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    rc::Rc,
};
use zip::ZipArchive;

#[derive(Debug)]
pub(crate) struct Archive {
    pub path: PathBuf,
    pub(super) file: File,
    pub(super) zip: RefCell<ZipArchive<File>>,
}

impl Archive {
    pub(super) fn open(path: &Path) -> Result<Self> {
        let open = || -> Result<Self> {
            let path = path.canonicalize()?;
            let file = File::open(&path)?;
            #[cfg(test)]
            super::tests::parsed(&path);
            let zip = ZipArchive::new(file.try_clone()?)?;
            Ok(Self {
                path,
                file,
                zip: RefCell::new(zip),
            })
        };
        open().map_err(|error| format!("APK {}: {error}", path.display()).into())
    }

    pub(super) fn contains(&self, name: &str) -> bool {
        self.zip.borrow().index_for_name(name).is_some()
    }

    pub(super) fn read(&self, name: &str) -> Result<Vec<u8>> {
        let read = || -> Result<Vec<u8>> {
            let mut bytes = Vec::new();
            self.zip
                .borrow_mut()
                .by_name(name)?
                .read_to_end(&mut bytes)?;
            Ok(bytes)
        };
        read().map_err(|error| format!("APK {} entry {name}: {error}", self.path.display()).into())
    }

    pub(super) fn image(&self, name: &str) -> Result<Mmap> {
        let mut archive = self.zip.borrow_mut();
        let mut entry = archive.by_name(name)?;
        if entry.compression() == zip::CompressionMethod::Stored
            && let Some(offset) = entry.data_start()
            && offset.is_multiple_of(4096)
        {
            let file_len = self.file.metadata()?.len();
            if entry.compressed_size() != entry.size()
                || offset
                    .checked_add(entry.size())
                    .is_none_or(|end| end > file_len)
            {
                return Err(
                    "stored entry size does not match its compressed size or exceeds the APK file"
                        .into(),
                );
            }
            let size = usize::try_from(entry.size())?;
            // APK files must stay unchanged while cached images are in use.
            return Ok(unsafe {
                MmapOptions::new()
                    .offset(offset)
                    .len(size)
                    .map(&self.file)?
            });
        }
        // Anonymous mappings are page aligned; only this entry is read/inflated.
        let mut mapped = MmapOptions::new()
            .len(usize::try_from(entry.size())?)
            .map_anon()?;
        entry.read_exact(&mut mapped)?;
        Ok(mapped.make_read_only()?)
    }
}

/// Archives opened at build, shared across metadata copies and checkpoints.
#[derive(Debug)]
pub(super) struct Archives {
    pub(super) base: Rc<Archive>,
    splits: [Option<Rc<Archive>>; 2],
}
impl Archives {
    pub(super) fn new(base: Rc<Archive>) -> Result<Self> {
        let open = |name| -> Result<Option<Rc<Archive>>> {
            let path = base.path.with_file_name(name);
            if path.exists() {
                Ok(Some(Rc::new(Archive::open(&path)?)))
            } else {
                Ok(None)
            }
        };
        let splits = [
            open("config.arm64_v8a.apk")?,
            open("config.armeabi_v7a.apk")?,
        ];
        Ok(Self { base, splits })
    }

    pub(super) fn libraries(
        &self,
        arch: crate::Architecture,
    ) -> impl Iterator<Item = Rc<Archive>> + '_ {
        let index = match arch {
            crate::Architecture::Arm64 => 0,
            crate::Architecture::Arm32 => 1,
        };
        std::iter::once(&self.base)
            .chain(self.splits[index].iter())
            .cloned()
    }
}
