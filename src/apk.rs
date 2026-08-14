use std::{fs::File, path::Path};

use anyhow::{bail, Context, Result};
use zip::ZipArchive;

const MAX_ENTRIES: usize = 50_000;
const MAX_ENTRY_SIZE: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApkInfo {
    pub path: String,
    pub entries: Vec<String>,
    pub has_manifest: bool,
    pub has_dex: bool,
    pub dex_size: Option<u64>,
}

pub fn inspect(path: impl AsRef<Path>) -> Result<ApkInfo> {
    let path = path.as_ref();
    let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut archive = ZipArchive::new(file).context("not a valid APK/ZIP archive")?;
    if archive.len() > MAX_ENTRIES {
        bail!("APK contains too many entries: {}", archive.len());
    }

    let mut entries = Vec::with_capacity(archive.len());
    let mut has_manifest = false;
    let mut has_dex = false;
    let mut dex_size = None;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).context("cannot read APK entry")?;
        if entry.size() > MAX_ENTRY_SIZE {
            bail!("APK entry {} exceeds the safety limit", entry.name());
        }
        let name = entry.name().to_owned();
        has_manifest |= name == "AndroidManifest.xml";
        if name == "classes.dex" {
            has_dex = true;
            dex_size = Some(entry.size());
        }
        entries.push(name);
    }
    entries.sort();
    Ok(ApkInfo {
        path: path.display().to_string(),
        entries,
        has_manifest,
        has_dex,
        dex_size,
    })
}
