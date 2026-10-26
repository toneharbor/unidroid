use super::Apk;
use crate::{Emulator, HostFunction, Result};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Default)]
pub(crate) struct Assets {
    manager: u64,
    open: HashMap<u64, Asset>,
    directories: HashMap<u64, Directory>,
}
#[derive(Clone)]
struct Directory {
    names: Vec<String>,
    position: usize,
}
#[derive(Clone)]
struct Asset {
    length: u64,
    position: u64,
}

fn entry(path: &str) -> String {
    format!("assets/{path}")
}

impl Apk {
    fn asset(&self, path: &str) -> Result<Option<Vec<u8>>> {
        let archive = &self.archives.base;
        let entry = entry(path);
        if entry.ends_with('/') || !archive.contains(&entry) {
            return Ok(None);
        }
        archive.read(&entry).map(Some)
    }
    pub(crate) fn asset_names(&self, path: &str, files_only: bool) -> Result<Vec<String>> {
        let archive = &self.archives.base;
        let prefix = format!("{}/", entry(path).trim_end_matches('/'));
        let zip = archive.zip.borrow();
        let mut names = BTreeSet::new();
        for name in zip.file_names() {
            let name = name?;
            let Some(suffix) = name.strip_prefix(&prefix) else {
                continue;
            };
            let (name, directory) = suffix
                .split_once('/')
                .map_or((suffix, false), |(name, _)| (name, true));
            if !name.is_empty() && !(files_only && directory) {
                names.insert(name.to_owned());
            }
        }
        Ok(names.into_iter().collect())
    }
}

impl Emulator {
    pub(crate) fn setup_assets(&mut self) -> Result<()> {
        if self.apk().is_some() {
            for name in [
                "AAssetManager_fromJava",
                "AAssetManager_open",
                "AAsset_read",
                "AAsset_getLength",
                "AAsset_getLength64",
                "AAsset_getRemainingLength",
                "AAsset_getRemainingLength64",
                "AAsset_getBuffer",
                "AAsset_close",
                "AAsset_seek",
                "AAsset_seek64",
                "AAssetManager_openDir",
                "AAssetDir_getNextFileName",
                "AAssetDir_rewind",
                "AAssetDir_close",
            ] {
                let address = self.stub(HostFunction::Asset(name))?;
                self.overrides.insert(name.into(), address);
            }
        }
        Ok(())
    }

    pub(crate) fn asset_function(&mut self, name: &str) -> Result<u64> {
        self.asset_call(name)
            .map_err(|error| self.jni_message(format!("{name}: {error}")).into())
    }

    fn asset_call(&mut self, name: &str) -> Result<u64> {
        let handle = self.reg(0)?;
        match name {
            "AAssetManager_fromJava" => {
                if self.assets.manager == 0 {
                    self.assets.manager = self.malloc(1)?;
                }
                Ok(self.assets.manager)
            }
            "AAssetManager_open" => {
                let path = self.cstring(self.reg(1)?)?;
                let Some(bytes) = self.apk().unwrap().asset(&path)? else {
                    return Ok(0);
                };
                let buffer = self.malloc(bytes.len())?;
                self.write_bytes(buffer, &bytes)?;
                self.assets.open.insert(
                    buffer,
                    Asset {
                        length: bytes.len() as u64,
                        position: 0,
                    },
                );
                Ok(buffer)
            }
            "AAssetManager_openDir" => {
                let path = self.cstring(self.reg(1)?)?;
                let names = self.apk().unwrap().asset_names(&path, true)?;
                let buffer = self.malloc(names.iter().map(String::len).max().unwrap_or(0) + 1)?;
                self.assets
                    .directories
                    .insert(buffer, Directory { names, position: 0 });
                Ok(buffer)
            }
            "AAssetDir_getNextFileName" | "AAssetDir_rewind" | "AAssetDir_close" => {
                let directory = self
                    .assets
                    .directories
                    .get_mut(&handle)
                    .ok_or_else(|| format!("invalid asset directory 0x{handle:x}"))?;
                match name {
                    "AAssetDir_getNextFileName" => {
                        let Some(name) = directory.names.get(directory.position).cloned() else {
                            return Ok(0);
                        };
                        directory.position += 1;
                        self.write_bytes(handle, name.as_bytes())?;
                        self.write_bytes(handle + name.len() as u64, &[0])?;
                        Ok(handle)
                    }
                    "AAssetDir_rewind" => {
                        directory.position = 0;
                        Ok(0)
                    }
                    "AAssetDir_close" => {
                        self.assets.directories.remove(&handle);
                        self.free(handle);
                        Ok(0)
                    }
                    _ => unreachable!(),
                }
            }
            _ => {
                let asset = self
                    .assets
                    .open
                    .get(&handle)
                    .ok_or_else(|| format!("invalid asset 0x{handle:x}"))?;
                match name {
                    "AAsset_getLength" | "AAsset_getLength64" => Ok(asset.length),
                    "AAsset_getRemainingLength" | "AAsset_getRemainingLength64" => {
                        Ok(asset.length - asset.position)
                    }
                    "AAsset_getBuffer" => Ok(handle),
                    "AAsset_read" => {
                        let count = self.reg(2)?.min(asset.length - asset.position);
                        if count != 0 {
                            let bytes = self.read_bytes(handle + asset.position, count as usize)?;
                            self.write_bytes(self.reg(1)?, &bytes)?;
                            self.assets.open.get_mut(&handle).unwrap().position += count;
                        }
                        Ok(count)
                    }
                    "AAsset_seek" | "AAsset_seek64" => {
                        let arm32 = self.pointer_size() == 4;
                        let wide = name == "AAsset_seek64";
                        let offset = if wide {
                            self.argument(1, b'J')? as i64
                        } else if arm32 {
                            self.reg(1)? as u32 as i32 as i64
                        } else {
                            self.reg(1)? as i64
                        };
                        let whence = self.argument(if wide && arm32 { 4 } else { 2 }, b'I')? as u32;
                        let base = match whence {
                            0 => 0,
                            1 => asset.position as i64,
                            2 => asset.length as i64,
                            _ => return Ok(u64::MAX),
                        };
                        let Some(position) = base
                            .checked_add(offset)
                            .filter(|&position| position >= 0 && position as u64 <= asset.length)
                        else {
                            return Ok(u64::MAX);
                        };
                        self.assets.open.get_mut(&handle).unwrap().position = position as u64;
                        Ok(position as u64)
                    }
                    "AAsset_close" => {
                        self.assets.open.remove(&handle);
                        self.free(handle);
                        Ok(0)
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
}
