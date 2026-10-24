use crate::Result;
use memmap2::{Mmap, MmapOptions};
use std::{
    cell::RefCell,
    collections::HashMap,
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

/// Shared across metadata copies and checkpoints. Missing splits may appear later.
#[derive(Debug)]
pub(super) struct Archives(RefCell<HashMap<PathBuf, Rc<Archive>>>);
impl Archives {
    pub(super) fn new(base: Rc<Archive>) -> Self {
        Self(RefCell::new(HashMap::from([(base.path.clone(), base)])))
    }

    pub(super) fn get(&self, path: &Path, optional: bool) -> Result<Option<Rc<Archive>>> {
        let mut archives = self.0.borrow_mut();
        if let Some(archive) = archives.get(path) {
            return Ok(Some(archive.clone()));
        }
        if optional && !path.is_file() {
            return Ok(None);
        }
        let archive = Rc::new(Archive::open(path)?);
        archives.insert(path.to_owned(), archive.clone());
        Ok(Some(archive))
    }
}
