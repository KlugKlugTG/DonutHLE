use std::path::Path;

use anyhow::{bail, Result};

use crate::{apk, dalvik::DexHeader, manifest::AppManifest, VirtualScreen, API_LEVEL, RELEASE};

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
    pub message: String,
}

#[derive(Default)]
pub struct Runtime {
    pub config: RuntimeConfig,
}

impl Runtime {
    pub fn validate_apk(&self, path: impl AsRef<Path>) -> Result<LaunchReport> {
        let info = apk::inspect(path)?;
        if !info.has_manifest {
            bail!("APK has no AndroidManifest.xml");
        }
        if !info.has_dex {
            bail!("APK has no classes.dex");
        }
        Ok(LaunchReport {
            package: "unknown until AXML decoder is implemented".to_owned(),
            dex: info
                .dex_size
                .map(|size| format!("{size} bytes"))
                .unwrap_or_else(|| "missing".to_owned()),
            message: "package structure accepted; launcher is not implemented yet".to_owned(),
        })
    }

    pub fn launch(&self, path: impl AsRef<Path>) -> Result<()> {
        let report = self.validate_apk(path)?;
        let _ = (&self.config, report);
        bail!("launch pipeline is not implemented yet: next milestone is AXML + Dalvik")
    }

    pub fn parse_dex(&self, bytes: &[u8]) -> Result<DexHeader> {
        DexHeader::parse(bytes)
    }

    pub fn parse_manifest(&self, bytes: &[u8]) -> Result<AppManifest> {
        AppManifest::parse_axml(bytes)
    }
}
