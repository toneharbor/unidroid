use super::archive::Archive;
use crate::{Architecture, Emulator, Image, Module, Result};
use std::{path::PathBuf, rc::Rc, sync::Arc};

pub(crate) enum Library {
    Host(PathBuf),
    Apk {
        archive: Rc<Archive>,
        entry: String,
        guest: String,
    },
}
impl Library {
    pub(crate) fn path(&self) -> PathBuf {
        match self {
            Self::Host(path) => path.clone(),
            Self::Apk { archive, entry, .. } => {
                format!("{}!/{entry}", archive.path.display()).into()
            }
        }
    }
    pub(crate) fn guest_path(&self) -> String {
        match self {
            Self::Host(path) => path.to_string_lossy().into_owned(),
            Self::Apk { guest, .. } => guest.clone(),
        }
    }
    pub(crate) fn image(&self) -> Result<Arc<Image>> {
        match self {
            Self::Host(path) => Image::open(path),
            Self::Apk { archive, entry, .. } => {
                Image::cached(&archive.path, Some(entry), || archive.image(entry)).map_err(
                    |error| format!("APK {} entry {entry}: {error}", archive.path.display()).into(),
                )
            }
        }
    }
}

impl Emulator {
    pub fn load_apk_library(&mut self, name: &str) -> Result<Module> {
        let module = self.load_apk_library_without_constructors(name)?;
        self.run_constructors(module, &mut ())?;
        Ok(module)
    }
    pub fn load_apk_library_without_constructors(&mut self, name: &str) -> Result<Module> {
        let apk = self
            .apk()
            .ok_or_else(|| format!("cannot load APK library {name}: no APK configured"))?;
        let library = self.find_library(name)?.ok_or_else(|| {
            format!(
                "library {name} not found in APK {}, loaded library folders or rootfs",
                apk.path.display()
            )
        })?;
        self.load_library_internal(library)
    }

    /// One search order for named loads, dependencies and dlopen.
    pub(crate) fn find_library(&self, name: &str) -> Result<Option<Library>> {
        if let Some(apk) = self.apk() {
            let arches: &[Architecture] = match self.arch {
                Some(Architecture::Arm64) => &[Architecture::Arm64],
                Some(Architecture::Arm32) => &[Architecture::Arm32],
                None => &[Architecture::Arm64, Architecture::Arm32],
            };
            for &arch in arches {
                let (abis, split): (&[&str], &str) = match arch {
                    Architecture::Arm64 => (&["arm64-v8a"], "config.arm64_v8a.apk"),
                    Architecture::Arm32 => (&["armeabi-v7a", "armeabi"], "config.armeabi_v7a.apk"),
                };
                let split = apk.path.with_file_name(split);
                for path in [&apk.path, &split] {
                    let archive = apk
                        .archives
                        .get(path, path == &split)
                        .map_err(|error| format!("loading {name}: {error}"))?;
                    let Some(archive) = archive else {
                        continue;
                    };
                    for abi in abis {
                        let entry = format!("lib/{abi}/{name}");
                        if archive.contains(&entry) {
                            return Ok(Some(Library::Apk {
                                archive,
                                entry,
                                guest: format!("{}/{name}", apk.native_library_dir(arch)),
                            }));
                        }
                    }
                }
            }
        }
        let dir = self.arch.unwrap_or(Architecture::Arm64).library_dir();
        Ok(self
            .library_dirs
            .iter()
            .map(|directory| directory.join(name))
            .chain(std::iter::once(self.rootfs.join(dir).join(name)))
            .find(|path| path.is_file())
            .map(Library::Host))
    }
}
