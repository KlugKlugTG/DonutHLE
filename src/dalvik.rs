use anyhow::{bail, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexHeader {
    pub version: String,
    pub file_size: u32,
    pub header_size: u32,
    pub endian_tag: u32,
}

impl DexHeader {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 112 || &bytes[0..4] != b"dex\n" || &bytes[4..8] != b"035\0" {
            bail!("unsupported or truncated DEX header; expected Dalvik DEX 035")
        }
        let u32_at =
            |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let header_size = u32_at(0x70);
        if header_size != 112 {
            bail!("unexpected DEX header size: {header_size}");
        }
        let file_size = u32_at(0x20);
        let endian_tag = u32_at(0x28);
        if endian_tag != 0x1234_5678 {
            bail!("unsupported DEX endian tag: {endian_tag:#x}");
        }
        Ok(Self {
            version: "035".to_owned(),
            file_size,
            header_size,
            endian_tag,
        })
    }

    pub fn validate_file_size(&self, actual_len: usize) -> Result<()> {
        if self.file_size as usize != actual_len {
            bail!(
                "DEX file_size {} does not match actual size {}",
                self.file_size,
                actual_len
            );
        }
        Ok(())
    }

    pub fn summary(&self) -> String {
        format!("DEX {} / {} bytes", self.version, self.file_size)
    }
}
