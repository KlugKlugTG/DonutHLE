use anyhow::{bail, Context, Result};

const RES_TABLE_TYPE: u16 = 0x0002;
const RES_STRING_POOL_TYPE: u16 = 0x0001;
const RES_TABLE_PACKAGE_TYPE: u16 = 0x0200;
const RES_TABLE_TYPE_TYPE: u16 = 0x0201;
const TYPE_NULL: u8 = 0x00;
const TYPE_REFERENCE: u8 = 0x01;
const TYPE_ATTRIBUTE: u8 = 0x02;
const TYPE_STRING: u8 = 0x03;
const TYPE_FLOAT: u8 = 0x04;
const TYPE_DIMENSION: u8 = 0x05;
const TYPE_FRACTION: u8 = 0x06;
const TYPE_INT_DEC: u8 = 0x10;
const TYPE_INT_HEX: u8 = 0x11;
const TYPE_INT_BOOLEAN: u8 = 0x12;
const UTF8_FLAG: u32 = 0x00000100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceValue {
    pub id: u32,
    pub package: String,
    pub type_name: String,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResourceTable {
    pub values: Vec<ResourceValue>,
    pub packages: Vec<String>,
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
    fn u16(&self, offset: usize) -> Result<u16> {
        let bytes = self
            .bytes
            .get(offset..offset + 2)
            .context("resource table truncated u16")?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }
    fn u32(&self, offset: usize) -> Result<u32> {
        let bytes = self
            .bytes
            .get(offset..offset + 4)
            .context("resource table truncated u32")?;
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
    }
    fn slice(&self, offset: usize, length: usize) -> Result<&'a [u8]> {
        self.bytes
            .get(offset..offset + length)
            .context("resource table chunk outside file")
    }
}

#[derive(Clone)]
struct StringPool {
    strings: Vec<String>,
}

impl StringPool {
    fn parse(
        reader: &Reader<'_>,
        offset: usize,
        header_size: usize,
        chunk_size: usize,
    ) -> Result<Self> {
        if header_size < 28 {
            bail!("resource string pool header is truncated");
        }
        let count = reader.u32(offset + 8)? as usize;
        let flags = reader.u32(offset + 16)?;
        let strings_start = reader.u32(offset + 20)? as usize;
        let offsets_start = offset + header_size;
        let offsets_len = count
            .checked_mul(4)
            .context("resource string pool too large")?;
        reader.slice(offsets_start, offsets_len)?;
        let data_start = offset
            .checked_add(strings_start)
            .context("resource string pool offset overflow")?;
        let data_end = offset
            .checked_add(chunk_size)
            .context("resource string pool size overflow")?;
        let mut strings = Vec::with_capacity(count);
        for index in 0..count {
            let relative = reader.u32(offsets_start + index * 4)? as usize;
            let start = data_start
                .checked_add(relative)
                .context("resource string offset overflow")?;
            let bytes = reader.slice(
                start,
                data_end
                    .checked_sub(start)
                    .context("resource string offset outside chunk")?,
            )?;
            strings.push(if flags & UTF8_FLAG != 0 {
                decode_utf8(bytes)?
            } else {
                decode_utf16(bytes)?
            });
        }
        Ok(Self { strings })
    }
    fn get(&self, index: u32) -> Result<&str> {
        self.strings
            .get(index as usize)
            .map(String::as_str)
            .context("resource string index outside pool")
    }
}

impl ResourceTable {
    pub fn value_by_id(&self, id: u32) -> Option<&str> {
        self.values
            .iter()
            .find(|resource| resource.id == id)
            .map(|resource| resource.value.as_str())
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let reader = Reader::new(bytes);
        if bytes.len() < 12 || reader.u16(0)? != RES_TABLE_TYPE {
            bail!("not a compiled Android resource table");
        }
        let file_size = reader.u32(4)? as usize;
        if file_size > bytes.len() || file_size < 12 {
            bail!("resource table size is invalid");
        }
        let package_count = reader.u32(8)? as usize;
        let mut offset = 12usize;
        let mut global_pool = None;
        let mut table = Self::default();
        while offset + 8 <= file_size {
            let chunk_type = reader.u16(offset)?;
            let header_size = reader.u16(offset + 2)? as usize;
            let chunk_size = reader.u32(offset + 4)? as usize;
            if header_size < 8 || chunk_size < header_size || offset + chunk_size > file_size {
                bail!("invalid resource table chunk at {offset}");
            }
            match chunk_type {
                RES_STRING_POOL_TYPE => {
                    global_pool = Some(StringPool::parse(&reader, offset, header_size, chunk_size)?)
                }
                RES_TABLE_PACKAGE_TYPE => {
                    let package = parse_package(
                        &reader,
                        offset,
                        header_size,
                        chunk_size,
                        global_pool
                            .as_ref()
                            .context("resource package before global pool")?,
                    )?;
                    table.packages.push(package.name.clone());
                    table.values.extend(package.values);
                }
                _ => {}
            }
            offset += chunk_size;
        }
        if package_count != 0 && table.packages.len() > package_count {
            bail!("resource package count is inconsistent");
        }
        Ok(table)
    }
}

struct PackageData {
    name: String,
    values: Vec<ResourceValue>,
}

fn parse_package(
    reader: &Reader<'_>,
    offset: usize,
    header_size: usize,
    chunk_size: usize,
    global_pool: &StringPool,
) -> Result<PackageData> {
    if header_size < 284 {
        bail!("resource package header is truncated");
    }
    let package_id = reader.u32(offset + 8)?;
    let name = decode_utf16_fixed(reader.slice(offset + 12, 256)?)?;
    let type_strings_offset = reader.u32(offset + 268)? as usize;
    let key_strings_offset = reader.u32(offset + 276)? as usize;
    let type_pool = if type_strings_offset != 0 {
        Some(StringPool::parse(
            reader,
            offset + type_strings_offset,
            reader.u16(offset + type_strings_offset + 2)? as usize,
            reader.u32(offset + type_strings_offset + 4)? as usize,
        )?)
    } else {
        None
    };
    let key_pool = if key_strings_offset != 0 {
        Some(StringPool::parse(
            reader,
            offset + key_strings_offset,
            reader.u16(offset + key_strings_offset + 2)? as usize,
            reader.u32(offset + key_strings_offset + 4)? as usize,
        )?)
    } else {
        None
    };
    let mut values = Vec::new();
    let mut cursor = offset + header_size;
    let end = offset
        .checked_add(chunk_size)
        .context("resource package size overflow")?;
    while cursor + 8 <= end {
        let kind = reader.u16(cursor).unwrap_or(0);
        let child_header = reader.u16(cursor + 2).unwrap_or(0) as usize;
        let child_size = reader.u32(cursor + 4).unwrap_or(0) as usize;
        if child_header < 8 || child_size < child_header || cursor + child_size > end {
            break;
        }
        if kind == RES_TABLE_TYPE_TYPE {
            parse_type_chunk(
                reader,
                cursor,
                child_header,
                child_size,
                package_id,
                &name,
                type_pool.as_ref(),
                key_pool.as_ref(),
                global_pool,
                &mut values,
            )?;
        }
        cursor += child_size;
    }
    Ok(PackageData { name, values })
}

#[allow(clippy::too_many_arguments)]
fn parse_type_chunk(
    reader: &Reader<'_>,
    offset: usize,
    header_size: usize,
    chunk_size: usize,
    package_id: u32,
    package: &str,
    type_pool: Option<&StringPool>,
    key_pool: Option<&StringPool>,
    global_pool: &StringPool,
    values: &mut Vec<ResourceValue>,
) -> Result<()> {
    if header_size < 20 {
        return Ok(());
    }
    let type_id = reader.slice(offset + 8, 1)?[0] as u32;
    let entry_count = reader.u32(offset + 12)? as usize;
    let entries_start = reader.u32(offset + 16)? as usize;
    let offsets_start = offset + header_size;
    let offsets_len = entry_count
        .checked_mul(4)
        .context("resource entry offsets too large")?;
    reader.slice(offsets_start, offsets_len)?;
    let type_name = type_pool
        .and_then(|pool| pool.get(type_id.saturating_sub(1)).ok())
        .unwrap_or("unknown")
        .to_owned();
    let entries_base = offset
        .checked_add(entries_start)
        .context("resource entries offset overflow")?;
    let end = offset
        .checked_add(chunk_size)
        .context("resource package size overflow")?;
    for index in 0..entry_count {
        let relative = reader.u32(offsets_start + index * 4)?;
        if relative == 0xffff_ffff {
            continue;
        }
        let entry = entries_base
            .checked_add(relative as usize)
            .context("resource entry offset overflow")?;
        if entry + 8 > end {
            continue;
        }
        let entry_size = reader.u16(entry)? as usize;
        let flags = reader.u16(entry + 2)?;
        let key_index = reader.u32(entry + 4)?;
        let key = key_pool
            .and_then(|pool| pool.get(key_index).ok())
            .unwrap_or("unknown")
            .to_owned();
        let id = (package_id << 24) | (type_id << 16) | index as u32;
        if flags & 0x0001 != 0 {
            let count = reader.u32(entry + entry_size)? as usize;
            let map_base = entry + entry_size + 4;
            for map_index in 0..count {
                let map = map_base + map_index * 12;
                if map + 12 > end {
                    break;
                }
                let value = format_value(reader, map + 4, global_pool)?;
                values.push(ResourceValue {
                    id,
                    package: package.to_owned(),
                    type_name: type_name.clone(),
                    name: key.clone(),
                    value,
                });
            }
        } else if entry + entry_size + 8 <= end {
            let value = format_value(reader, entry + entry_size, global_pool)?;
            values.push(ResourceValue {
                id,
                package: package.to_owned(),
                type_name: type_name.clone(),
                name: key,
                value,
            });
        }
    }
    Ok(())
}

fn format_value(reader: &Reader<'_>, offset: usize, pool: &StringPool) -> Result<String> {
    let data_type = reader.slice(offset + 3, 1)?[0];
    let data = reader.u32(offset + 4)?;
    Ok(match data_type {
        TYPE_NULL => "null".to_owned(),
        TYPE_REFERENCE => format!("@0x{data:08x}"),
        TYPE_ATTRIBUTE => format!("?0x{data:08x}"),
        TYPE_STRING => pool.get(data)?.to_owned(),
        TYPE_FLOAT => format!("{}f", f32::from_bits(data)),
        TYPE_DIMENSION | TYPE_FRACTION => format!("0x{data:08x}"),
        TYPE_INT_DEC => (data as i32).to_string(),
        TYPE_INT_HEX => format!("0x{data:08x}"),
        TYPE_INT_BOOLEAN => (data != 0).to_string(),
        _ => format!("0x{data:08x}"),
    })
}

fn decode_utf8(bytes: &[u8]) -> Result<String> {
    let (_, first) = read_utf8_length(bytes)?;
    let (length, second) = read_utf8_length(
        bytes
            .get(first..)
            .context("resource UTF-8 length truncated")?,
    )?;
    let start = first + second;
    Ok(String::from_utf8_lossy(
        bytes
            .get(start..start + length)
            .context("resource UTF-8 string truncated")?,
    )
    .into_owned())
}

fn read_utf8_length(bytes: &[u8]) -> Result<(usize, usize)> {
    let first = *bytes.first().context("resource UTF-8 length missing")?;
    if first & 0x80 == 0 {
        Ok((first as usize, 1))
    } else {
        Ok((
            ((first & 0x7f) as usize) << 8
                | *bytes.get(1).context("resource UTF-8 length truncated")? as usize,
            2,
        ))
    }
}

fn decode_utf16(bytes: &[u8]) -> Result<String> {
    let (length, used) = read_utf16_length(bytes)?;
    let size = length
        .checked_mul(2)
        .context("resource UTF-16 size overflow")?;
    let raw = bytes
        .get(used..used + size)
        .context("resource UTF-16 string truncated")?;
    let units = raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|item| u16::from_le_bytes([item[0], item[1]]))
        .collect::<Vec<_>>();
    Ok(String::from_utf16_lossy(&units))
}

fn read_utf16_length(bytes: &[u8]) -> Result<(usize, usize)> {
    let first = u16::from_le_bytes([
        *bytes.first().context("resource UTF-16 length missing")?,
        *bytes.get(1).context("resource UTF-16 length truncated")?,
    ]);
    if first & 0x8000 == 0 {
        Ok((first as usize, 2))
    } else {
        Ok((
            ((first & 0x7fff) as usize) << 16
                | u16::from_le_bytes([
                    *bytes.get(2).context("resource UTF-16 length truncated")?,
                    *bytes.get(3).context("resource UTF-16 length truncated")?,
                ]) as usize,
            4,
        ))
    }
}

fn decode_utf16_fixed(bytes: &[u8]) -> Result<String> {
    let mut units = Vec::new();
    for chunk in bytes.as_chunks::<2>().0 {
        let unit = u16::from_le_bytes([chunk[0], chunk[1]]);
        if unit == 0 {
            break;
        }
        units.push(unit);
    }
    Ok(String::from_utf16_lossy(&units))
}
