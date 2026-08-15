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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexFile {
    pub header: DexHeader,
    pub strings: Vec<String>,
    pub types: Vec<String>,
    pub prototypes: Vec<String>,
    pub fields: Vec<FieldId>,
    pub methods: Vec<MethodId>,
    pub classes: Vec<ClassDef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldId {
    pub class_name: String,
    pub name: String,
    pub descriptor: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodId {
    pub class_name: String,
    pub name: String,
    pub descriptor: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassDef {
    pub class_name: String,
    pub superclass: Option<String>,
    pub source_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeItem {
    pub registers_size: u16,
    pub ins_size: u16,
    pub outs_size: u16,
    pub tries_size: u16,
    pub instructions: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionResult {
    pub registers: Vec<i32>,
    pub return_value: Option<i32>,
    pub steps: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterpreterError {
    pub pc: usize,
    pub opcode: u8,
    pub message: String,
}

impl std::fmt::Display for InterpreterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DEX interpreter error at pc {} opcode 0x{:02x}: {}", self.pc, self.opcode, self.message)
    }
}
impl std::error::Error for InterpreterError {}

impl DexHeader {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < DEX_HEADER_SIZE || &bytes[0..4] != b"dex\n" || &bytes[4..8] != b"035\0" {
            bail!("unsupported or truncated DEX header; expected Dalvik DEX 035")
        }
        let u32_at = |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let header_size = u32_at(0x70);
        if header_size != DEX_HEADER_SIZE as u32 {
            bail!("unexpected DEX header size: {header_size}")
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
    pub fn validate_file_size(&self, actual_len: usize) -> Result<()> {
        if self.file_size as usize != actual_len { bail!("DEX file_size {} does not match actual size {}", self.file_size, actual_len); }
        Ok(())
    }
    pub fn summary(&self) -> String { format!("DEX {} / {} bytes", self.version, self.file_size) }
}

struct Reader<'a> { bytes: &'a [u8] }
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self { Self { bytes } }
    fn u16(&self, offset: usize) -> Result<u16> { Ok(u16::from_le_bytes(self.bytes.get(offset..offset + 2).context("DEX u16 outside file")?.try_into().unwrap())) }
    fn u32(&self, offset: usize) -> Result<u32> { Ok(u32::from_le_bytes(self.bytes.get(offset..offset + 4).context("DEX u32 outside file")?.try_into().unwrap())) }
    fn bytes(&self, offset: usize, count: usize) -> Result<&'a [u8]> { self.bytes.get(offset..offset + count).context("DEX slice outside file") }
}

impl DexFile {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let header = DexHeader::parse(bytes)?;
        let reader = Reader::new(bytes);
        let string_ids_size = reader.u32(0x38)? as usize;
        let string_ids_off = reader.u32(0x3c)? as usize;
        let type_ids_size = reader.u32(0x40)? as usize;
        let type_ids_off = reader.u32(0x44)? as usize;
        let proto_ids_size = reader.u32(0x48)? as usize;
        let proto_ids_off = reader.u32(0x4c)? as usize;
        let field_ids_size = reader.u32(0x50)? as usize;
        let field_ids_off = reader.u32(0x54)? as usize;
        let method_ids_size = reader.u32(0x58)? as usize;
        let method_ids_off = reader.u32(0x5c)? as usize;
        let class_defs_size = reader.u32(0x60)? as usize;
        let class_defs_off = reader.u32(0x64)? as usize;
        let strings = (0..string_ids_size).map(|index| read_string(bytes, reader.u32(string_ids_off + index * 4)? as usize)).collect::<Result<Vec<_>>>()?;
        let types = (0..type_ids_size).map(|index| { let string_index = reader.u32(type_ids_off + index * 4)? as usize; strings.get(string_index).cloned().context("DEX type string outside string list") }).collect::<Result<Vec<_>>>()?;
        let prototypes = (0..proto_ids_size).map(|index| { let return_type = reader.u32(proto_ids_off + index * 12 + 8)? as usize; types.get(return_type).cloned().context("DEX prototype return type outside type list") }).collect::<Result<Vec<_>>>()?;
        let fields = (0..field_ids_size).map(|index| { let base = field_ids_off + index * 8; let class_name = types.get(reader.u16(base)? as usize).context("DEX field class outside type list")?.clone(); let descriptor = types.get(reader.u16(base + 2)? as usize).context("DEX field type outside type list")?.clone(); let name = strings.get(reader.u32(base + 4)? as usize).context("DEX field name outside string list")?.clone(); Ok(FieldId { class_name, name, descriptor }) }).collect::<Result<Vec<_>>>()?;
        let methods = (0..method_ids_size).map(|index| { let base = method_ids_off + index * 8; let class_name = types.get(reader.u16(base)? as usize).context("DEX method class outside type list")?.clone(); let proto_index = reader.u16(base + 2)? as usize; let name = strings.get(reader.u32(base + 4)? as usize).context("DEX method name outside string list")?.clone(); let descriptor = prototypes.get(proto_index).cloned().unwrap_or_default(); Ok(MethodId { class_name, name, descriptor }) }).collect::<Result<Vec<_>>>()?;
        let classes = (0..class_defs_size).map(|index| { let base = class_defs_off + index * 32; let class_name = types.get(reader.u32(base)? as usize).context("DEX class outside type list")?.clone(); let superclass_index = reader.u32(base + 8)?; let source_index = reader.u32(base + 16)?; Ok(ClassDef { class_name, superclass: if superclass_index == NO_INDEX { None } else { Some(types.get(superclass_index as usize).cloned().context("DEX superclass outside type list")?) }, source_file: if source_index == NO_INDEX { None } else { Some(strings.get(source_index as usize).cloned().context("DEX source outside string list")?) } }) }).collect::<Result<Vec<_>>>()?;
        Ok(Self { header, strings, types, prototypes, fields, methods, classes })
    }
    pub fn method_code(&self, _class_name: &str, _method_name: &str) -> Option<&CodeItem> { None }
}

fn read_string(bytes: &[u8], offset: usize) -> Result<String> {
    let mut cursor = offset;
    let mut value = Vec::new();
    while cursor < bytes.len() { let byte = bytes[cursor]; cursor += 1; if byte == 0 { break; } value.push(byte); }
    String::from_utf8(value).context("DEX string is not UTF-8")
}

pub fn decode_uleb128(bytes: &[u8], mut offset: usize) -> Result<(u32, usize)> { let mut value = 0u32; let mut shift = 0; loop { let byte = *bytes.get(offset).context("ULEB128 outside file")?; offset += 1; value |= u32::from(byte & 0x7f) << shift; if byte & 0x80 == 0 { return Ok((value, offset)); } shift += 7; if shift >= 32 { bail!("ULEB128 is too long"); } } }

pub fn parse_dex(bytes: &[u8]) -> Result<DexFile> { DexFile::parse(bytes) }
