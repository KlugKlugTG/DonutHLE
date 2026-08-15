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
    pub prototypes: Vec<Prototype>,
    pub fields: Vec<FieldId>,
    pub methods: Vec<MethodId>,
    pub classes: Vec<ClassDef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodId {
    pub class_name: String,
    pub name: String,
    pub prototype: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prototype {
    pub shorty: String,
    pub parameters: Vec<String>,
    pub return_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldId {
    pub class_name: String,
    pub type_name: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassDef {
    pub name: String,
    pub access_flags: u32,
    pub super_class: Option<String>,
    pub direct_methods: Vec<EncodedMethod>,
    pub virtual_methods: Vec<EncodedMethod>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedMethod {
    pub method_index: u32,
    pub access_flags: u32,
    pub code: Option<CodeItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeItem {
    pub registers_size: u16,
    pub ins_size: u16,
    pub outs_size: u16,
    pub instructions: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registers {
    values: Vec<i32>,
}

impl Registers {
    pub fn new(size: usize) -> Self {
        Self {
            values: vec![0; size],
        }
    }
    pub fn get(&self, index: usize) -> Result<i32> {
        self.values
            .get(index)
            .copied()
            .context("register index outside frame")
    }
    pub fn set(&mut self, index: usize, value: i32) -> Result<()> {
        *self
            .values
            .get_mut(index)
            .context("register index outside frame")? = value;
        Ok(())
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionResult {
    ReturnVoid,
    Return(i32),
    Continue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterpreterError {
    pub pc: usize,
    pub opcode: u8,
    pub message: String,
}

impl std::fmt::Display for InterpreterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "DEX interpreter error at pc {} opcode 0x{:02x}: {}",
            self.pc, self.opcode, self.message
        )
    }
}
impl std::error::Error for InterpreterError {}

impl DexHeader {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < DEX_HEADER_SIZE || &bytes[0..4] != b"dex\n" || &bytes[4..8] != b"035\0" {
            bail!("unsupported or truncated DEX header; expected Dalvik DEX 035")
        }
        let u32_at =
            |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let header_size = u32_at(0x70);
        if header_size != DEX_HEADER_SIZE as u32 {
            bail!("unexpected DEX header size: {header_size}");
        }
        let file_size = u32_at(0x20);
        if file_size < DEX_HEADER_SIZE as u32 || file_size as usize > bytes.len() {
            bail!("invalid DEX file size: {file_size} for {} bytes", bytes.len());
        }
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

struct Reader<'a> {
    bytes: &'a [u8],
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
    fn u16(&self, offset: usize) -> Result<u16> {
        Ok(u16::from_le_bytes(
            self.bytes
                .get(offset..offset + 2)
                .context("DEX truncated u16")?
                .try_into()
                .unwrap(),
        ))
    }
    fn u32(&self, offset: usize) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.bytes
                .get(offset..offset + 4)
                .context("DEX truncated u32")?
                .try_into()
                .unwrap(),
        ))
    }
    fn leb128(&self, cursor: &mut usize) -> Result<u32> {
        read_uleb128(self.bytes, cursor)
    }
}

impl DexFile {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let reader = Reader::new(bytes);
        let header = DexHeader::parse(bytes)?;
        header.validate_file_size(bytes.len())?;
        let string_ids_size = reader.u32(0x38)? as usize;
        let string_ids_off = reader.u32(0x3c)? as usize;
        let type_ids_size = reader.u32(0x40)? as usize;
        let type_ids_off = reader.u32(0x44)? as usize;
        let proto_ids_size = reader.u32(0x48)? as usize;
        let proto_ids_off = reader.u32(0x4c)? as usize;
        let method_ids_size = reader.u32(0x58)? as usize;
        let method_ids_off = reader.u32(0x5c)? as usize;
        let class_defs_size = reader.u32(0x60)? as usize;
        let class_defs_off = reader.u32(0x64)? as usize;
        let strings = parse_strings(&reader, string_ids_size, string_ids_off)?;
        let types = parse_types(&reader, type_ids_size, type_ids_off, &strings)?;
        let protos = parse_protos(&reader, proto_ids_size, proto_ids_off, &strings, &types)?;
        let fields_size = reader.u32(0x50)? as usize;
        let fields_off = reader.u32(0x54)? as usize;
        let methods = parse_methods(
            &reader,
            method_ids_size,
            method_ids_off,
            &strings,
            &types,
            &protos,
        )?;
        let fields = parse_fields(&reader, fields_size, fields_off, &strings, &types)?;
        let classes = parse_classes(
            &reader,
            class_defs_size,
            class_defs_off,
            &strings,
            &types,
            &methods,
        )?;
        Ok(Self {
            header,
            strings,
            types,
            prototypes: protos,
            fields,
            methods,
            classes,
        })
    }

    pub fn find_class(&self, name: &str) -> Option<&ClassDef> {
        self.classes.iter().find(|class| class.name == name)
    }
    pub fn find_method(&self, class_name: &str, name: &str) -> Option<&MethodId> {
        self.methods
            .iter()
            .find(|method| method.class_name == class_name && method.name == name)
    }
    pub fn method_id(&self, index: usize) -> Option<&MethodId> {
        self.methods.get(index)
    }

    pub fn field_id(&self, index: usize) -> Option<&FieldId> {
        self.fields.get(index)
    }

    pub fn prototype(&self, index: usize) -> Option<&Prototype> {
        self.prototypes.get(index)
    }

    pub fn method_code(&self, class_name: &str, method_name: &str) -> Option<&CodeItem> {
        let class = self.find_class(class_name)?;
        class
            .direct_methods
            .iter()
            .chain(class.virtual_methods.iter())
            .find(|method| {
                self.methods
                    .get(method.method_index as usize)
                    .is_some_and(|id| id.name == method_name)
            })
            .and_then(|method| method.code.as_ref())
    }
}

fn parse_strings(reader: &Reader<'_>, count: usize, offset: usize) -> Result<Vec<String>> {
    let mut values = Vec::with_capacity(count);
    for index in 0..count {
        let string_offset = reader.u32(offset + index * 4)? as usize;
        values.push(read_mutf8(reader.bytes, string_offset)?);
    }
    Ok(values)
}
fn parse_types(
    reader: &Reader<'_>,
    count: usize,
    offset: usize,
    strings: &[String],
) -> Result<Vec<String>> {
    (0..count)
        .map(|index| {
            Ok(strings
                .get(reader.u32(offset + index * 4)? as usize)
                .context("DEX type string index outside pool")?
                .clone())
        })
        .collect()
}
fn parse_protos(
    reader: &Reader<'_>,
    count: usize,
    offset: usize,
    strings: &[String],
    types: &[String],
) -> Result<Vec<Prototype>> {
    let mut values = Vec::with_capacity(count);
    for index in 0..count {
        let shorty = strings
            .get(reader.u32(offset + index * 12)? as usize)
            .context("DEX shorty index outside string pool")?;
        let return_type = types
            .get(reader.u32(offset + index * 12 + 4)? as usize)
            .context("DEX return type outside type list")?;
        let parameters_off = reader.u32(offset + index * 12 + 8)? as usize;
        let parameters = if parameters_off == 0 {
            Vec::new()
        } else {
            let size = reader.u32(parameters_off)? as usize;
            (0..size)
                .map(|i| {
                    types
                        .get(reader.u16(parameters_off + 4 + i * 2)? as usize)
                        .cloned()
                        .context("DEX parameter type outside type list")
                })
                .collect::<Result<Vec<_>>>()?
        };
        values.push(Prototype {
            shorty: shorty.clone(),
            parameters,
            return_type: return_type.clone(),
        });
    }
    Ok(values)
}
fn parse_fields(
    reader: &Reader<'_>,
    count: usize,
    offset: usize,
    strings: &[String],
    types: &[String],
) -> Result<Vec<FieldId>> {
    (0..count)
        .map(|index| {
            let base = offset + index * 8;
            let class_name = types
                .get(reader.u16(base)? as usize)
                .context("DEX field class outside type list")?
                .clone();
            let type_name = types
                .get(reader.u16(base + 2)? as usize)
                .context("DEX field type outside type list")?
                .clone();
            let name = strings
                .get(reader.u32(base + 4)? as usize)
                .context("DEX field name outside string pool")?
                .clone();
            Ok(FieldId {
                class_name,
                type_name,
                name,
            })
        })
        .collect()
}
fn parse_methods(
    reader: &Reader<'_>,
    count: usize,
    offset: usize,
    strings: &[String],
    types: &[String],
    protos: &[Prototype],
) -> Result<Vec<MethodId>> {
    (0..count)
        .map(|index| {
            let base = offset + index * 8;
            let class_name = types
                .get(reader.u16(base)? as usize)
                .context("DEX method class outside type list")?
                .clone();
            let proto = protos
                .get(reader.u16(base + 2)? as usize)
                .context("DEX method prototype outside list")?;
            let prototype = format!(
                "{}({})->{}",
                proto.shorty,
                proto.parameters.join(","),
                proto.return_type
            );
            let name = strings
                .get(reader.u32(base + 4)? as usize)
                .context("DEX method name outside string pool")?
                .clone();
            Ok(MethodId {
                class_name,
                name,
                prototype,
            })
        })
        .collect()
}
fn parse_classes(
    reader: &Reader<'_>,
    count: usize,
    offset: usize,
    strings: &[String],
    types: &[String],
    methods: &[MethodId],
) -> Result<Vec<ClassDef>> {
    let mut classes = Vec::with_capacity(count);
    for index in 0..count {
        let base = offset + index * 32;
        let class_name = types
            .get(reader.u32(base)? as usize)
            .context("DEX class outside type list")?
            .clone();
        let access_flags = reader.u32(base + 4)?;
        let super_class = match reader.u32(base + 8)? {
            NO_INDEX => None,
            value => Some(
                types
                    .get(value as usize)
                    .context("DEX superclass outside type list")?
                    .clone(),
            ),
        };
        let class_data_off = reader.u32(base + 24)? as usize;
        let (direct_methods, virtual_methods) = if class_data_off == 0 {
            (Vec::new(), Vec::new())
        } else {
            parse_class_data(reader, class_data_off, methods)?
        };
        let _ = strings;
        classes.push(ClassDef {
            name: class_name,
            access_flags,
            super_class,
            direct_methods,
            virtual_methods,
        });
    }
    Ok(classes)
}
fn parse_class_data(
    reader: &Reader<'_>,
    offset: usize,
    methods: &[MethodId],
) -> Result<(Vec<EncodedMethod>, Vec<EncodedMethod>)> {
    let mut cursor = offset;
    let static_fields = reader.leb128(&mut cursor)?;
    let instance_fields = reader.leb128(&mut cursor)?;
    let direct_count = reader.leb128(&mut cursor)?;
    let virtual_count = reader.leb128(&mut cursor)?;
    for _ in 0..(static_fields + instance_fields) {
        let _ = reader.leb128(&mut cursor)?;
        let _ = reader.leb128(&mut cursor)?;
    }
    let direct_methods = parse_encoded_methods(reader, &mut cursor, direct_count, methods)?;
    let virtual_methods = parse_encoded_methods(reader, &mut cursor, virtual_count, methods)?;
    Ok((direct_methods, virtual_methods))
}
fn parse_encoded_methods(
    reader: &Reader<'_>,
    cursor: &mut usize,
    count: u32,
    methods: &[MethodId],
) -> Result<Vec<EncodedMethod>> {
    let mut result = Vec::with_capacity(count as usize);
    let mut method_index = 0u32;
    for _ in 0..count {
        method_index += reader.leb128(cursor)?;
        let access_flags = reader.leb128(cursor)?;
        let code_off = reader.leb128(cursor)? as usize;
        let code = if code_off == 0 {
            None
        } else {
            Some(parse_code_item(reader, code_off)?)
        };
        if method_index as usize >= methods.len() {
            bail!("DEX method index outside method list");
        }
        result.push(EncodedMethod {
            method_index,
            access_flags,
            code,
        });
    }
    Ok(result)
}
fn parse_code_item(reader: &Reader<'_>, offset: usize) -> Result<CodeItem> {
    let registers_size = reader.u16(offset)?;
    let ins_size = reader.u16(offset + 2)?;
    let outs_size = reader.u16(offset + 4)?;
    let tries_size = reader.u16(offset + 6)?;
    let debug_off = reader.u32(offset + 8)? as usize;
    let insns_size = reader.u32(offset + 12)? as usize;
    let instructions = (0..insns_size)
        .map(|i| reader.u16(offset + 16 + i * 2))
        .collect::<Result<Vec<_>>>()?;
    let _ = (tries_size, debug_off);
    Ok(CodeItem {
        registers_size,
        ins_size,
        outs_size,
        instructions,
    })
}

pub fn execute(
    code: &CodeItem,
    registers: &mut Registers,
    max_steps: usize,
) -> std::result::Result<ExecutionResult, InterpreterError> {
    let mut pc = 0usize;
    let mut result = ExecutionResult::Continue;
    for _ in 0..max_steps {
        if pc >= code.instructions.len() {
            return Err(error(pc, 0, "program counter outside code"));
        }
        let instruction = code.instructions[pc];
        let opcode = (instruction & 0xff) as u8;
        match opcode {
            0x00 => pc += 1,
            0x0e => return Ok(ExecutionResult::ReturnVoid),
            0x0f => {
                return Ok(ExecutionResult::Return(
                    registers
                        .get((instruction >> 8) as usize)
                        .map_err(|e| error(pc, opcode, e.to_string()))?,
                ))
            }
            0x12 => {
                let register = ((instruction >> 8) & 0x0f) as usize;
                let literal = ((instruction >> 12) & 0x0f) as i8;
                registers
                    .set(register, literal as i32)
                    .map_err(|e| error(pc, opcode, e.to_string()))?;
                pc += 1;
            }
            0x13 => {
                let register = (instruction >> 8) as usize;
                let literal = *code
                    .instructions
                    .get(pc + 1)
                    .ok_or_else(|| error(pc, opcode, "const/16 literal missing"))?
                    as i16;
                registers
                    .set(register, literal as i32)
                    .map_err(|e| error(pc, opcode, e.to_string()))?;
                pc += 2;
            }
            0x14 => {
                let register = (instruction >> 8) as usize;
                let literal = u32::from(
                    code.instructions
                        .get(pc + 1)
                        .copied()
                        .ok_or_else(|| error(pc, opcode, "const literal missing"))?,
                ) | (u32::from(
                    *code
                        .instructions
                        .get(pc + 2)
                        .ok_or_else(|| error(pc, opcode, "const literal truncated"))?,
                ) << 16);
                registers
                    .set(register, literal as i32)
                    .map_err(|e| error(pc, opcode, e.to_string()))?;
                pc += 3;
            }
            0x01 => {
                let dest = (instruction >> 8) as usize;
                let source = ((instruction >> 12) & 0x0f) as usize;
                registers
                    .set(
                        dest,
                        registers
                            .get(source)
                            .map_err(|e| error(pc, opcode, e.to_string()))?,
                    )
                    .map_err(|e| error(pc, opcode, e.to_string()))?;
                pc += 1;
            }
            0x0a => {
                result = ExecutionResult::Return(
                    registers
                        .get((instruction >> 8) as usize)
                        .map_err(|e| error(pc, opcode, e.to_string()))?,
                );
                return Ok(result);
            }
            0x90 => {
                let dest = ((instruction >> 8) & 0x0f) as usize;
                let left = ((instruction >> 12) & 0x0f) as usize;
                let right = (code
                    .instructions
                    .get(pc + 1)
                    .copied()
                    .ok_or_else(|| error(pc, opcode, "add-int register missing"))?
                    & 0xff) as usize;
                registers
                    .set(
                        dest,
                        registers
                            .get(left)
                            .map_err(|e| error(pc, opcode, e.to_string()))?
                            + registers
                                .get(right)
                                .map_err(|e| error(pc, opcode, e.to_string()))?,
                    )
                    .map_err(|e| error(pc, opcode, e.to_string()))?;
                pc += 2;
            }
            _ => {
                return Err(error(
                    pc,
                    opcode,
                    "opcode is not implemented by the API-4 interpreter",
                ))
            }
        }
    }
    Ok(result)
}
fn error(pc: usize, opcode: u8, message: impl Into<String>) -> InterpreterError {
    InterpreterError {
        pc,
        opcode,
        message: message.into(),
    }
}
fn read_uleb128(bytes: &[u8], cursor: &mut usize) -> Result<u32> {
    let mut result = 0u32;
    let mut shift = 0;
    for _ in 0..5 {
        let value = *bytes.get(*cursor).context("DEX uleb128 truncated")?;
        *cursor += 1;
        result |= u32::from(value & 0x7f) << shift;
        if value & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
    }
    bail!("DEX uleb128 is too long")
}
fn read_mutf8(bytes: &[u8], offset: usize) -> Result<String> {
    let mut cursor = offset;
    let _utf16_length = read_uleb128(bytes, &mut cursor)?;
    let mut output = Vec::new();
    loop {
        let value = *bytes.get(cursor).context("DEX string data truncated")?;
        cursor += 1;
        if value == 0 {
            break;
        }
        if value == 0xc0 && bytes.get(cursor).copied() == Some(0x80) {
            output.push(0);
            cursor += 1;
        } else {
            output.push(value);
        }
    }
    Ok(String::from_utf8_lossy(&output).into_owned())
}
