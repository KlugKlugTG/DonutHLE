use anyhow::{bail, Context, Result};

const DEX_HEADER_SIZE: usize = 112;
const NO_INDEX: u32 = 0xffff_ffff;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexHeader {
    pub version: String,
    pub file_size: u32,
    pub header_size: u32,
    pub endian_tag: u32,
}

impl DexHeader {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < DEX_HEADER_SIZE || &bytes[0..4] != b"dex\n" || &bytes[4..8] != b"035\0" {
            bail!("unsupported or truncated DEX header; expected Dalvik DEX 035")
        }
        let u32_at = |offset: usize| {
            u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("DEX header bounds checked"))
        };
        let header_size = u32_at(0x70);
        if header_size != DEX_HEADER_SIZE as u32 {
            bail!("unexpected DEX header size: {header_size}; APK may use a non-DEX or transformed classes.dex")
        }
        let file_size = u32_at(0x20);
        if file_size < DEX_HEADER_SIZE as u32 || file_size as usize > bytes.len() {
            bail!("invalid DEX file size: {file_size} for {} bytes", bytes.len())
        }
        let endian_tag = u32_at(0x28);
        if endian_tag != 0x1234_5678 {
            bail!("unsupported DEX endian tag: {endian_tag:#x}")
        }
        Ok(Self { version: "035".to_owned(), file_size, header_size, endian_tag })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexMethod {
    pub class_name: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexFile {
    pub header: DexHeader,
    pub strings: Vec<String>,
    pub types: Vec<String>,
    pub methods: Vec<DexMethod>,
}

impl DexFile {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let header = DexHeader::parse(bytes)?;
        let strings = vec![];
        let types = vec![];
        let methods = vec![];
        Ok(Self { header, strings, types, methods })
    }
}
