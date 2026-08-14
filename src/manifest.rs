use anyhow::{bail, Result};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppManifest {
    pub package: String,
    pub version_name: Option<String>,
    pub version_code: Option<u32>,
    pub min_sdk: Option<u32>,
    pub target_sdk: Option<u32>,
    pub launcher_activity: Option<String>,
}

impl AppManifest {
    pub fn parse_axml(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 8 || u16::from_le_bytes([bytes[0], bytes[1]]) != 0x0003 {
            bail!("not Android binary XML (AXML)");
        }
        bail!("AXML parser scaffold: manifest decoding is the next milestone")
    }
}
