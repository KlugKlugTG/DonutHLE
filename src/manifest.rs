use anyhow::{bail, Context, Result};

const RES_XML_TYPE: u16 = 0x0003;
const RES_STRING_POOL_TYPE: u16 = 0x0001;
const RES_XML_START_NAMESPACE_TYPE: u16 = 0x0100;
const RES_XML_END_NAMESPACE_TYPE: u16 = 0x0101;
const RES_XML_START_ELEMENT_TYPE: u16 = 0x0102;
const RES_XML_END_ELEMENT_TYPE: u16 = 0x0103;
const RES_XML_CDATA_TYPE: u16 = 0x0104;
const UTF8_FLAG: u32 = 0x0000_0100;
const TYPE_STRING: u8 = 0x03;
const TYPE_INT_DEC: u8 = 0x10;
const TYPE_INT_HEX: u8 = 0x11;
const TYPE_INT_BOOLEAN: u8 = 0x12;
const ANDROID_NS: &str = "http://schemas.android.com/apk/res/android";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActivityInfo {
    pub name: String,
    pub exported: Option<bool>,
    pub screen_orientation: Option<String>,
    pub has_main_action: bool,
    pub has_launcher_category: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppManifest {
    pub package: String,
    pub version_name: Option<String>,
    pub version_code: Option<u32>,
    pub min_sdk: Option<u32>,
    pub target_sdk: Option<u32>,
    pub launcher_activity: Option<String>,
    pub application_label: Option<String>,
    pub activities: Vec<ActivityInfo>,
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AxmlDocument {
    pub strings: Vec<String>,
    pub manifest: AppManifest,
}

#[derive(Debug, Clone)]
struct Attribute {
    namespace: Option<String>,
    name: String,
    value: String,
}

#[derive(Debug, Clone)]
struct Element {
    name: String,
    attributes: Vec<Attribute>,
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
            .context("AXML truncated u16")?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&self, offset: usize) -> Result<u32> {
        let bytes = self
            .bytes
            .get(offset..offset + 4)
            .context("AXML truncated u32")?;
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn slice(&self, offset: usize, length: usize) -> Result<&'a [u8]> {
        self.bytes
            .get(offset..offset + length)
            .context("AXML chunk outside document")
    }
}

impl AppManifest {
    pub fn parse_axml(bytes: &[u8]) -> Result<Self> {
        Ok(parse_document(bytes)?.manifest)
    }
}

pub fn parse_document(bytes: &[u8]) -> Result<AxmlDocument> {
    let reader = Reader::new(bytes);
    if bytes.len() < 8 || reader.u16(0)? != RES_XML_TYPE {
        bail!("not Android binary XML (AXML)");
    }
    let document_size = reader.u32(4)? as usize;
    if document_size < 8 || document_size > bytes.len() {
        bail!("AXML document size is invalid");
    }

    let mut offset = 8usize;
    let mut strings = Vec::new();
    let mut stack: Vec<Element> = Vec::new();
    let mut manifest = AppManifest::default();
    let mut current_activity: Option<usize> = None;
    let mut action_stack: Vec<Option<String>> = Vec::new();

    while offset + 8 <= document_size {
        let chunk_type = reader.u16(offset)?;
        let header_size = reader.u16(offset + 2)? as usize;
        let chunk_size = reader.u32(offset + 4)? as usize;
        if header_size < 8 || chunk_size < header_size || offset + chunk_size > document_size {
            bail!("invalid AXML chunk at offset {offset}");
        }
        match chunk_type {
            RES_STRING_POOL_TYPE => {
                strings = parse_string_pool(&reader, offset, header_size, chunk_size)?;
            }
            RES_XML_START_NAMESPACE_TYPE | RES_XML_END_NAMESPACE_TYPE => {}
            RES_XML_START_ELEMENT_TYPE => {
                let element = parse_start_element(&reader, offset, header_size, &strings)?;
                let depth = stack.len();
                if depth == 0 && element.name == "manifest" {
                    manifest.package =
                        attr_string(&element.attributes, None, "package").unwrap_or_default();
                    manifest.version_name =
                        attr_value(&element.attributes, ANDROID_NS, "versionName");
                    manifest.version_code =
                        attr_u32(&element.attributes, ANDROID_NS, "versionCode");
                } else if element.name == "uses-sdk" {
                    manifest.min_sdk = attr_u32(&element.attributes, ANDROID_NS, "minSdkVersion");
                    manifest.target_sdk =
                        attr_u32(&element.attributes, ANDROID_NS, "targetSdkVersion");
                } else if element.name == "uses-permission" {
                    if let Some(name) = attr_value(&element.attributes, ANDROID_NS, "name") {
                        manifest.permissions.push(name);
                    }
                } else if element.name == "application" {
                    manifest.application_label =
                        attr_value(&element.attributes, ANDROID_NS, "label");
                } else if element.name == "activity" || element.name == "activity-alias" {
                    let name =
                        attr_value(&element.attributes, ANDROID_NS, "name").unwrap_or_default();
                    manifest.activities.push(ActivityInfo {
                        name: qualify_class_name(&manifest.package, &name),
                        exported: attr_bool(&element.attributes, ANDROID_NS, "exported"),
                        screen_orientation: attr_value(
                            &element.attributes,
                            ANDROID_NS,
                            "screenOrientation",
                        ),
                        has_main_action: false,
                        has_launcher_category: false,
                    });
                    current_activity = Some(manifest.activities.len() - 1);
                } else if element.name == "action" {
                    action_stack.push(attr_value(&element.attributes, ANDROID_NS, "name"));
                } else if element.name == "category"
                    && attr_value(&element.attributes, ANDROID_NS, "name").as_deref()
                        == Some("android.intent.category.LAUNCHER")
                {
                    if let Some(index) = current_activity {
                        manifest.activities[index].has_launcher_category = true;
                    }
                }
                stack.push(element);
            }
            RES_XML_END_ELEMENT_TYPE => {
                if let Some(element) = stack.pop() {
                    if element.name == "action"
                        && action_stack.pop().flatten().as_deref()
                            == Some("android.intent.action.MAIN")
                    {
                        if let Some(index) = current_activity {
                            manifest.activities[index].has_main_action = true;
                        }
                    }
                    if element.name == "activity" || element.name == "activity-alias" {
                        if let Some(index) = current_activity {
                            let activity = &manifest.activities[index];
                            if activity.has_main_action
                                && activity.has_launcher_category
                                && manifest.launcher_activity.is_none()
                            {
                                manifest.launcher_activity = Some(activity.name.clone());
                            }
                        }
                        current_activity = stack
                            .iter()
                            .rposition(|item| {
                                item.name == "activity" || item.name == "activity-alias"
                            })
                            .and_then(|index| {
                                manifest
                                    .activities
                                    .iter()
                                    .position(|activity| activity.name == stack[index].name)
                            });
                    }
                } else {
                    bail!("AXML end element without matching start element");
                }
            }
            RES_XML_CDATA_TYPE => {}
            _ => {}
        }
        offset += chunk_size;
    }
    if !stack.is_empty() {
        bail!("AXML document has unclosed elements");
    }
    if manifest.package.is_empty() {
        bail!("AXML manifest element is missing");
    }
    Ok(AxmlDocument { strings, manifest })
}

fn parse_string_pool(
    reader: &Reader<'_>,
    offset: usize,
    header_size: usize,
    chunk_size: usize,
) -> Result<Vec<String>> {
    if header_size < 28 {
        bail!("AXML string pool header is truncated");
    }
    let count = reader.u32(offset + 8)? as usize;
    let flags = reader.u32(offset + 16)?;
    let strings_start = reader.u32(offset + 20)? as usize;
    let offsets_start = offset + header_size;
    let offsets_len = count.checked_mul(4).context("AXML string pool too large")?;
    reader.slice(offsets_start, offsets_len)?;
    let string_data = offset
        .checked_add(strings_start)
        .context("AXML string data offset overflow")?;
    let string_data_end = offset + chunk_size;
    let mut strings = Vec::with_capacity(count);
    for index in 0..count {
        let string_offset = reader.u32(offsets_start + index * 4)? as usize;
        let absolute = string_data
            .checked_add(string_offset)
            .context("AXML string offset overflow")?;
        let available = string_data_end
            .checked_sub(absolute)
            .context("AXML string offset outside pool")?;
        strings.push(if flags & UTF8_FLAG != 0 {
            decode_utf8_string(reader.slice(absolute, available)?)?
        } else {
            decode_utf16_string(reader.slice(absolute, available)?)?
        });
    }
    Ok(strings)
}

fn decode_utf8_string(bytes: &[u8]) -> Result<String> {
    let (_, first_len) = read_utf8_length(bytes)?;
    let (length, second_len) = read_utf8_length(&bytes[first_len..])?;
    let start = first_len + second_len;
    let value = bytes
        .get(start..start + length)
        .context("AXML UTF-8 string truncated")?;
    Ok(String::from_utf8_lossy(value).into_owned())
}

fn read_utf8_length(bytes: &[u8]) -> Result<(usize, usize)> {
    let first = *bytes.first().context("AXML string length missing")?;
    if first & 0x80 == 0 {
        Ok((first as usize, 1))
    } else {
        let second = *bytes.get(1).context("AXML string length truncated")?;
        Ok((((first & 0x7f) as usize) << 8 | second as usize, 2))
    }
}

fn decode_utf16_string(bytes: &[u8]) -> Result<String> {
    let (length, used) = read_utf16_length(bytes)?;
    let byte_len = length
        .checked_mul(2)
        .context("AXML UTF-16 string too large")?;
    let value = bytes
        .get(used..used + byte_len)
        .context("AXML UTF-16 string truncated")?;
    let units = value
        .as_chunks::<2>()
        .0
        .iter()
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    Ok(String::from_utf16_lossy(&units))
}

fn read_utf16_length(bytes: &[u8]) -> Result<(usize, usize)> {
    let first = u16::from_le_bytes([
        *bytes.first().context("AXML UTF-16 length missing")?,
        *bytes.get(1).context("AXML UTF-16 length truncated")?,
    ]);
    if first & 0x8000 == 0 {
        Ok((first as usize, 2))
    } else {
        let second = u16::from_le_bytes([
            *bytes.get(2).context("AXML UTF-16 length truncated")?,
            *bytes.get(3).context("AXML UTF-16 length truncated")?,
        ]);
        Ok((((first & 0x7fff) as usize) << 16 | second as usize, 4))
    }
}

fn parse_start_element(
    reader: &Reader<'_>,
    offset: usize,
    header_size: usize,
    strings: &[String],
) -> Result<Element> {
    if header_size < 16 {
        bail!("AXML start element header is truncated");
    }
    let attr_start = reader.u16(offset + 24)? as usize;
    let attr_size = reader.u16(offset + 26)? as usize;
    let attr_count = reader.u16(offset + 28)? as usize;
    let name = string_at(strings, reader.u32(offset + 20)?)?.to_owned();
    if attr_size < 20 {
        bail!("AXML attribute size is invalid");
    }
    let attributes_start = offset
        .checked_add(16 + attr_start)
        .context("AXML attribute offset overflow")?;
    let attributes_len = attr_count
        .checked_mul(attr_size)
        .context("AXML attribute list too large")?;
    reader.slice(attributes_start, attributes_len)?;
    let mut attributes = Vec::with_capacity(attr_count);
    for index in 0..attr_count {
        let base = attributes_start + index * attr_size;
        let namespace = match reader.u32(base)? {
            u32::MAX => None,
            index => Some(string_at(strings, index)?.to_owned()),
        };
        let attr_name = string_at(strings, reader.u32(base + 4)?)?.to_owned();
        let raw = match reader.u32(base + 8)? {
            u32::MAX => None,
            index => Some(string_at(strings, index)?.to_owned()),
        };
        let data_type = reader
            .bytes
            .get(base + 15)
            .copied()
            .context("AXML attribute type missing")?;
        let data = reader.u32(base + 16)?;
        attributes.push(Attribute {
            namespace,
            name: attr_name,
            value: decode_value(strings, raw, data_type, data)?,
        });
    }
    Ok(Element { name, attributes })
}

fn decode_value(
    strings: &[String],
    raw: Option<String>,
    data_type: u8,
    data: u32,
) -> Result<String> {
    Ok(match data_type {
        TYPE_STRING => string_at(strings, data)?.to_owned(),
        TYPE_INT_DEC => data.to_string(),
        TYPE_INT_HEX => format!("0x{data:08x}"),
        TYPE_INT_BOOLEAN => (data != 0).to_string(),
        _ => raw.unwrap_or_else(|| format!("@0x{data:08x}")),
    })
}

fn string_at(strings: &[String], index: u32) -> Result<&str> {
    strings
        .get(index as usize)
        .map(String::as_str)
        .context("AXML string index outside pool")
}

fn attr_string(attributes: &[Attribute], namespace: Option<&str>, name: &str) -> Option<String> {
    attributes
        .iter()
        .find(|attr| attr.name == name && attr.namespace.as_deref() == namespace)
        .map(|attr| attr.value.clone())
}

fn attr_value(attributes: &[Attribute], namespace: &str, name: &str) -> Option<String> {
    attr_string(attributes, Some(namespace), name)
}

fn attr_u32(attributes: &[Attribute], namespace: &str, name: &str) -> Option<u32> {
    attr_value(attributes, namespace, name).and_then(|value| {
        value
            .parse::<u32>()
            .ok()
            .or_else(|| u32::from_str_radix(value.strip_prefix("0x")?, 16).ok())
    })
}

fn attr_bool(attributes: &[Attribute], namespace: &str, name: &str) -> Option<bool> {
    attr_value(attributes, namespace, name).and_then(|value| match value.as_str() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    })
}

fn qualify_class_name(package: &str, value: &str) -> String {
    if value.starts_with('.') {
        format!("{package}{value}")
    } else if value.contains('.') || value.is_empty() {
        value.to_owned()
    } else {
        format!("{package}.{value}")
    }
}
