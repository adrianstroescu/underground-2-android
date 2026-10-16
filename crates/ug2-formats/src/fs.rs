//! Access to an installed game directory. Android file systems are case-sensitive
//! while the game's own references are not, so lookups ignore case.

use std::path::{Path, PathBuf};

use crate::{bail, compression, Result};

#[derive(Clone, Debug)]
pub struct GameDir {
    root: PathBuf,
}

impl GameDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// True if this looks like a UG2 install.
    pub fn is_valid(&self) -> bool {
        self.resolve("TRACKS/L4RA.BUN").is_some() && self.resolve("CARS").is_some()
    }

    /// Resolves a `/`-separated relative path, matching each component case-insensitively.
    pub fn resolve(&self, rel: &str) -> Option<PathBuf> {
        let mut cur = self.root.clone();
        for part in rel.split(['/', '\\']).filter(|p| !p.is_empty()) {
            let direct = cur.join(part);
            if direct.exists() {
                cur = direct;
                continue;
            }
            let found = std::fs::read_dir(&cur)
                .ok()?
                .filter_map(|e| e.ok())
                .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part))?;
            cur = found.path();
        }
        Some(cur)
    }

    pub fn read(&self, rel: &str) -> Result<Vec<u8>> {
        match self.resolve(rel) {
            Some(p) => Ok(std::fs::read(p)?),
            None => bail!("missing game file {rel} under {}", self.root.display()),
        }
    }

    /// Reads a file, decompressing it if the whole file is one compressed block (.lzc).
    /// Reads `rel`, preferring the 2004 original `rel.bak` when a mod replaced the file
    /// (the owner's install has modded FrontB, GlobalB and language files).
    pub fn read_original(&self, rel: &str) -> Result<Vec<u8>> {
        self.read(&format!("{rel}.bak")).or_else(|_| self.read(rel))
    }

    pub fn read_unpacked(&self, rel: &str) -> Result<Vec<u8>> {
        let data = self.read(rel)?;
        if compression::is_compressed(&data) {
            Ok(compression::decompress(&data)?.into_owned())
        } else {
            Ok(data)
        }
    }

    /// Sub-directory names of `rel` (upper-cased).
    pub fn list_dirs(&self, rel: &str) -> Vec<String> {
        let Some(dir) = self.resolve(rel) else { return Vec::new() };
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                    .map(|e| e.file_name().to_string_lossy().to_ascii_uppercase())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
}
