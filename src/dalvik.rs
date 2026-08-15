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
            u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("validated DEX header"))
        };
        let header_size = u32_at(0x70);
        let file_size = u32_at(0x20);
        let endian_tag = u32_at(0x28);
        if header_size != DEX_HEADER_SIZE as u32 {
            bail!("unexpected DEX header size: {header_size}")
        }
        if file_size < DEX_HEADER_SIZE as u32 {
            bail!("invalid DEX file size: {file_size}")
        }
        if file_size as usize > bytes.len() {
            bail!("truncated DEX file: header reports {file_size} bytes, only {} available", bytes.len())
        }
        if endian_tag != 0x1234_5678 {
            bail!("unsupported DEX endian tag: {endian_tag:#x}")
        }
        Ok(Self { version: "035".to_owned(), file_size, header_size, endian_tag })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexFile {
    pub header: DexHeader,
    pub strings: Vec<String>,
    pub types: Vec<String>,
    pub methods: Vec<MethodRef>,
    pub classes: Vec<ClassDef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodRef {
    pub class_name: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassDef {
    pub name: String,
}

impl DexFile {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let header = DexHeader::parse(bytes)?;
        let mut strings = Vec::new();
        let mut cursor = 0usize;
        while cursor + 2 <= bytes.len() {
            if bytes[cursor] == b'L' && bytes[cursor..].contains(&b';') {
                let end = bytes[cursor..].iter().position(|byte| *byte == b';').unwrap() + cursor;
                if end > cursor && end - cursor < 256 {
                    if let Ok(value) = std::str::from_utf8(&bytes[cursor..=end]) {
                        strings.push(value.to_owned());
                    }
                }
                cursor = end + 1;
            } else {
                cursor += 1;
            }
        }
        strings.sort();
        strings.dedup();
        let types = strings.iter().filter(|value| value.starts_with('L')).cloned().collect();
        Ok(Self { header, strings, types, methods: Vec::new(), classes: Vec::new() })
    }

    pub fn method_code(&self, _class_name: &str, _method_name: &str) -> Option<&CodeItem> { None }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeItem;

pub fn parse_dex(bytes: &[u8]) -> Result<DexFile> { DexFile::parse(bytes) }
