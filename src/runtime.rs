use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::{
    apk,
    dalvik::{DexFile, DexHeader},
    manifest::AppManifest,
    resources::ResourceTable,
    VirtualScreen, API_LEVEL, RELEASE,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub screen: VirtualScreen,
    pub api_level: u32,
    pub release: &'static str,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            screen: VirtualScreen::default(),
            api_level: API_LEVEL,
            release: RELEASE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchReport {
    pub package: String,
    pub dex: String,
    pub launcher_activity: String,
    pub message: String,
}

#[derive(Default)]
pub struct Runtime {
    pub config: RuntimeConfig,
}

impl Runtime {
    pub fn validate_apk(&self, path: impl AsRef<Path>) -> Result<LaunchReport> {
        let path = path.as_ref();
        let info = apk::inspect(path)?;
        if !info.has_manifest {
            bail!("APK has no AndroidManifest.xml");
        }
        if !info.has_dex {
            bail!("APK has no classes.dex");
        }
        let file = File::open(path).with_context(|| format!("open APK {}", path.display()))?;
        let mut archive = zip::ZipArchive::new(file)?;
        let manifest_bytes = {
            let mut entry = archive.by_name("AndroidManifest.xml")?;
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            bytes
        };
        let dex_bytes = {
            let mut entry = archive.by_name("classes.dex")?;
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            bytes
        };
        let manifest = AppManifest::parse_axml(&manifest_bytes)?;
        let dex = DexFile::parse(&dex_bytes)?;
        let resource_status = if info.entries.iter().any(|entry| entry == "resources.arsc") {
            let mut entry = archive.by_name("resources.arsc")?;
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            ResourceTable::parse(&bytes)
                .map(|table| format!("{} resources", table.values.len()))
                .unwrap_or_else(|_| "present but not decoded".to_owned())
        } else {
            "not present".to_owned()
        };
        let launcher = manifest
            .launcher_activity
            .clone()
            .unwrap_or_else(|| "none".to_owned());
        Ok(LaunchReport {
            package: manifest.package,
            launcher_activity: launcher.clone(),
            dex: format!(
                "{} bytes / {} classes / {} methods",
                dex.header.file_size,
                dex.classes.len(),
                dex.methods.len()
            ),
            message: format!(
                "manifest decoded; launcher: {launcher}; resources: {resource_status}"
            ),
        })
    }

    pub fn launch(&self, path: impl AsRef<Path>) -> Result<()> {
        let report = self.validate_apk(path)?;
        bail!("APK parsed successfully but execution needs Android framework bindings and a host render loop: {}", report.message)
    }

    pub fn parse_dex(&self, bytes: &[u8]) -> Result<DexHeader> {
        DexHeader::parse(bytes)
    }

    pub fn parse_manifest(&self, bytes: &[u8]) -> Result<AppManifest> {
        AppManifest::parse_axml(bytes)
    }
}
