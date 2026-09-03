//! Native-library detection for APKs.
//!
//! Scope: detect `lib/<abi>/*.so` entries, parse the ELF32 header, and report
//! the dynamic-symbol inventory (imports, exports, JNI entry points). This
//! module does not execute native code; it exists so launch reports can state
//! honestly which libraries require native execution support.

use anyhow::{bail, Context, Result};

const ELF_MAGIC: &[u8] = b"\x7fELF";
const ELF_CLASS_32: u8 = 1;
const ELF_DATA_LSB: u8 = 1;
const SHN_UNDEF: u16 = 0;
const SHT_DYNSYM: u32 = 11;
const SHT_DYNAMIC: u32 = 6;
const DT_NULL: u32 = 0;
const DT_NEEDED: u32 = 1;
const DT_SONAME: u32 = 14;
const MAX_SYMBOLS: usize = 1 << 20;
const MAX_DYNAMIC_ENTRIES: usize = 4096;

/// A parsed ELF32 shared object from an APK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeLibrary {
    pub format: String,
    pub soname: Option<String>,
    pub needed: Vec<String>,
    pub imports: Vec<String>,
    pub export_count: usize,
    pub jni_functions: Vec<String>,
}

/// One APK native-library entry: either a parsed library or a parse failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeLibraryReport {
    pub entry: String,
    pub library: Option<NativeLibrary>,
    pub error: Option<String>,
}

impl NativeLibraryReport {
    pub fn failed(entry: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            entry: entry.into(),
            library: None,
            error: Some(error.into()),
        }
    }

    /// One-line status of this entry, without a report prefix.
    pub fn status_line(&self) -> String {
        match (&self.library, &self.error) {
            (Some(library), _) => format!("{}: {}", self.entry, library.summary()),
            (None, Some(error)) => format!("{}: unreadable ({error})", self.entry),
            (None, None) => format!("{}: unreadable", self.entry),
        }
    }
}

impl NativeLibrary {
    /// One-line description of the library, e.g. for compatibility logs.
    pub fn summary(&self) -> String {
        let mut parts = vec![self.format.clone()];
        if let Some(soname) = &self.soname {
            parts.push(format!("soname {soname}"));
        }
        if !self.needed.is_empty() {
            parts.push(format!("needs {}", self.needed.join(", ")));
        }
        parts.push(format!(
            "{} exported, {} imported",
            self.export_count,
            self.imports.len()
        ));
        if !self.jni_functions.is_empty() {
            parts.push(format!("{} JNI", self.jni_functions.len()));
        }
        parts.join(", ")
    }
}

/// Formats native-library reports as compatibility lines (without a prefix).
pub fn format_report_lines(reports: &[NativeLibraryReport]) -> Vec<String> {
    let mut lines = Vec::new();
    for report in reports {
        lines.push(report.status_line());
        if let Some(library) = &report.library {
            if !library.imports.is_empty() {
                lines.push(format!(
                    "{} imports: {}",
                    report.entry,
                    library.imports.join(", ")
                ));
            }
            if !library.jni_functions.is_empty() {
                lines.push(format!(
                    "{} JNI entry points: {}",
                    report.entry,
                    library.jni_functions.join(", ")
                ));
            }
        }
    }
    lines
}

/// Short status clause for launch messages, or `None` when the APK has no
/// native libraries.
pub fn status_summary(reports: &[NativeLibraryReport]) -> Option<String> {
    if reports.is_empty() {
        return None;
    }
    let entries = reports
        .iter()
        .map(|report| report.entry.clone())
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "native libraries present; native execution is not implemented: {entries}"
    ))
}

/// True when the APK entry name is a native library (`lib/<abi>/<name>.so`).
pub fn is_native_library_entry(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("lib/") else {
        return false;
    };
    let Some((abi, file)) = rest.split_once('/') else {
        return false;
    };
    !abi.is_empty() && file.len() > 3 && file.ends_with(".so")
}

/// Parses an ELF32 shared object and extracts its dynamic-symbol inventory.
pub fn parse(bytes: &[u8]) -> Result<NativeLibrary> {
    if bytes.len() < 52 {
        bail!("file is too short for an ELF32 header");
    }
    if &bytes[0..4] != ELF_MAGIC {
        bail!("missing ELF magic");
    }
    if bytes[4] != ELF_CLASS_32 {
        bail!("ELF class {} is not supported (only ELF32)", bytes[4]);
    }
    if bytes[5] != ELF_DATA_LSB {
        bail!("big-endian ELF is not supported");
    }
    let object_type = u16_at(bytes, 16, "e_type")?;
    let machine = u16_at(bytes, 18, "e_machine")?;
    let flags = u32_at(bytes, 36, "e_flags")?;
    let section_offset = u32_at(bytes, 32, "e_shoff")? as usize;
    let section_size = u16_at(bytes, 46, "e_shentsize")? as usize;
    let section_count = u16_at(bytes, 48, "e_shnum")? as usize;

    let sections = read_sections(bytes, section_offset, section_size, section_count)?;
    let mut library = NativeLibrary {
        format: format_summary(object_type, machine, flags),
        soname: None,
        needed: Vec::new(),
        imports: Vec::new(),
        export_count: 0,
        jni_functions: Vec::new(),
    };
    if let Some(dynamic_index) = sections
        .iter()
        .position(|section| section.sh_type == SHT_DYNAMIC)
    {
        read_dynamic(bytes, &sections, dynamic_index, &mut library)?;
    }
    if let Some(dynsym_index) = sections
        .iter()
        .position(|section| section.sh_type == SHT_DYNSYM)
    {
        read_symbols(bytes, &sections, dynsym_index, &mut library)?;
    } else {
        bail!("no dynamic symbol table (SHT_DYNSYM section is missing)");
    }
    Ok(library)
}

struct Section {
    sh_type: u32,
    offset: usize,
    size: usize,
    link: usize,
    entsize: usize,
}

fn read_sections(
    bytes: &[u8],
    offset: usize,
    entry_size: usize,
    count: usize,
) -> Result<Vec<Section>> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if entry_size < 40 {
        bail!("ELF section entry size {entry_size} is too small");
    }
    let end = offset
        .checked_add(
            entry_size
                .checked_mul(count)
                .context("section table overflows")?,
        )
        .context("section table overflows")?;
    if end > bytes.len() {
        bail!("ELF section table extends past the end of the file");
    }
    let mut sections = Vec::with_capacity(count);
    for index in 0..count {
        let base = offset + index * entry_size;
        sections.push(Section {
            sh_type: u32_at(bytes, base + 4, "sh_type")?,
            offset: u32_at(bytes, base + 16, "sh_offset")? as usize,
            size: u32_at(bytes, base + 20, "sh_size")? as usize,
            link: u32_at(bytes, base + 24, "sh_link")? as usize,
            entsize: u32_at(bytes, base + 36, "sh_entsize")? as usize,
        });
    }
    Ok(sections)
}

fn read_dynamic(
    bytes: &[u8],
    sections: &[Section],
    dynamic_index: usize,
    library: &mut NativeLibrary,
) -> Result<()> {
    let dynamic = &sections[dynamic_index];
    let strtab = sections
        .get(dynamic.link)
        .context("ELF dynamic section has an invalid string-table link")?;
    let count = (dynamic.size / 8).min(MAX_DYNAMIC_ENTRIES);
    for index in 0..count {
        let base = dynamic.offset + index * 8;
        match u32_at(bytes, base, "d_tag")? {
            DT_NULL => break,
            DT_NEEDED => {
                let value = u32_at(bytes, base + 4, "d_val")?;
                library.needed.push(read_dynstr(bytes, strtab, value)?);
            }
            DT_SONAME => {
                let value = u32_at(bytes, base + 4, "d_val")?;
                library.soname = Some(read_dynstr(bytes, strtab, value)?);
            }
            _ => {}
        }
    }
    Ok(())
}

fn read_symbols(
    bytes: &[u8],
    sections: &[Section],
    dynsym_index: usize,
    library: &mut NativeLibrary,
) -> Result<()> {
    let dynsym = &sections[dynsym_index];
    let strtab = sections
        .get(dynsym.link)
        .context("ELF dynamic symbol section has an invalid string-table link")?;
    let entry_size = dynsym.entsize.max(16);
    let count = (dynsym.size / entry_size).min(MAX_SYMBOLS);
    for index in 0..count {
        let base = dynsym.offset + index * entry_size;
        let st_name = u32_at(bytes, base, "st_name")?;
        if st_name == 0 {
            continue;
        }
        let st_value = u32_at(bytes, base + 4, "st_value")?;
        let st_shndx = u16_at(bytes, base + 14, "st_shndx")?;
        let name = read_dynstr(bytes, strtab, st_name)?;
        if st_shndx == SHN_UNDEF {
            library.imports.push(name);
        } else if st_value != 0 {
            library.export_count += 1;
            if name.starts_with("Java_") {
                library.jni_functions.push(name);
            }
        }
    }
    Ok(())
}

fn read_dynstr(bytes: &[u8], strtab: &Section, offset: u32) -> Result<String> {
    let start = strtab.offset + offset as usize;
    if strtab.offset > bytes.len() || start > bytes.len() {
        bail!("ELF string-table offset is out of bounds");
    }
    let limit = strtab.offset.saturating_add(strtab.size).min(bytes.len());
    let stop = bytes[start..limit]
        .iter()
        .position(|byte| *byte == 0)
        .map(|position| start + position)
        .unwrap_or(limit);
    Ok(String::from_utf8_lossy(&bytes[start..stop]).into_owned())
}

fn format_summary(object_type: u16, machine: u16, flags: u32) -> String {
    let kind = match object_type {
        2 => "executable".to_owned(),
        3 => "shared object".to_owned(),
        other => format!("type {other}"),
    };
    let arch = match machine {
        3 => "x86".to_owned(),
        8 => "MIPS".to_owned(),
        40 => "ARM".to_owned(),
        62 => "x86-64".to_owned(),
        183 => "AArch64".to_owned(),
        other => format!("machine {other}"),
    };
    let eabi = match flags >> 24 {
        5 => " EABI5",
        4 => " EABI4",
        _ => "",
    };
    format!("ELF32 {arch}{eabi} {kind}")
}

fn u16_at(bytes: &[u8], offset: usize, what: &str) -> Result<u16> {
    let end = offset
        .checked_add(2)
        .context("ELF field offset overflows")?;
    bytes
        .get(offset..end)
        .map(|slice| u16::from_le_bytes(slice.try_into().expect("slice is two bytes")))
        .with_context(|| format!("truncated ELF while reading {what}"))
}

fn u32_at(bytes: &[u8], offset: usize, what: &str) -> Result<u32> {
    let end = offset
        .checked_add(4)
        .context("ELF field offset overflows")?;
    bytes
        .get(offset..end)
        .map(|slice| u32::from_le_bytes(slice.try_into().expect("slice is four bytes")))
        .with_context(|| format!("truncated ELF while reading {what}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STRTAB: &[u8] = b"\0libgame.so\0libc.so\0malloc\0Java_com_test_Game_start\0game_render\0";
    // strtab offsets: 0 "", 1 libgame.so, 12 libc.so, 20 malloc,
    // 27 Java_com_test_Game_start, 51 game_render
    const STR_SONAME: u32 = 1;
    const STR_NEEDED: u32 = 12;
    const STR_IMPORT: u32 = 20;
    const STR_JNI: u32 = 27;
    const STR_EXPORT: u32 = 51;

    fn push_u16(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn symbol(st_name: u32, st_value: u32, st_shndx: u16) -> Vec<u8> {
        let mut entry = Vec::new();
        push_u32(&mut entry, st_name);
        push_u32(&mut entry, st_value);
        push_u32(&mut entry, 12);
        entry.push(18); // st_info: GLOBAL FUNC
        entry.push(0); // st_other
        push_u16(&mut entry, st_shndx);
        entry
    }

    /// Layout: [0..52) ELF header, [52..172) three 40-byte section headers,
    /// then dynsym, strtab, dynamic.
    fn build_elf() -> Vec<u8> {
        let dynsym_offset: usize = 172;
        let dynsym_size: usize = 3 * 16;
        let strtab_offset = dynsym_offset + dynsym_size;
        let strtab_size = STRTAB.len();
        let dynamic_offset = strtab_offset + strtab_size;
        let dynamic_size: usize = 5 * 8;

        let mut bytes = Vec::new();
        bytes.extend_from_slice(ELF_MAGIC);
        bytes.push(ELF_CLASS_32);
        bytes.push(ELF_DATA_LSB);
        bytes.push(1); // EI_VERSION
        bytes.extend(std::iter::repeat_n(0, 9)); // e_ident padding to 16 bytes
        push_u16(&mut bytes, 3); // e_type = ET_DYN
        push_u16(&mut bytes, 40); // e_machine = EM_ARM
        push_u32(&mut bytes, 1); // e_version
        push_u32(&mut bytes, 0); // e_entry
        push_u32(&mut bytes, 0); // e_phoff
        push_u32(&mut bytes, 52); // e_shoff
        push_u32(&mut bytes, 0x0500_0002); // e_flags = EABI5
        push_u16(&mut bytes, 52); // e_ehsize
        push_u16(&mut bytes, 0); // e_phentsize
        push_u16(&mut bytes, 0); // e_phnum
        push_u16(&mut bytes, 40); // e_shentsize
        push_u16(&mut bytes, 3); // e_shnum
        push_u16(&mut bytes, 0); // e_shstrndx

        // Section headers: [0] SHT_DYNSYM, [1] SHT_STRTAB, [2] SHT_DYNAMIC.
        let push_section = |bytes: &mut Vec<u8>,
                            sh_type: u32,
                            offset: usize,
                            size: usize,
                            link: usize,
                            entsize: usize| {
            push_u32(bytes, 0); // sh_name
            push_u32(bytes, sh_type);
            push_u32(bytes, 0); // sh_flags
            push_u32(bytes, 0); // sh_addr
            push_u32(bytes, offset as u32);
            push_u32(bytes, size as u32);
            push_u32(bytes, link as u32);
            push_u32(bytes, 0); // sh_info
            push_u32(bytes, 4); // sh_addralign
            push_u32(bytes, entsize as u32);
        };
        push_section(&mut bytes, SHT_DYNSYM, dynsym_offset, dynsym_size, 1, 16);
        push_section(&mut bytes, 3, strtab_offset, strtab_size, 0, 0);
        push_section(&mut bytes, SHT_DYNAMIC, dynamic_offset, dynamic_size, 1, 8);
        assert_eq!(bytes.len(), dynsym_offset);

        bytes.extend_from_slice(&symbol(STR_JNI, 0x1000, 4));
        bytes.extend_from_slice(&symbol(STR_IMPORT, 0, SHN_UNDEF));
        bytes.extend_from_slice(&symbol(STR_EXPORT, 0x2000, 4));
        bytes.extend_from_slice(STRTAB);

        let push_dynamic = |bytes: &mut Vec<u8>, tag: u32, value: u32| {
            push_u32(bytes, tag);
            push_u32(bytes, value);
        };
        push_dynamic(&mut bytes, DT_NEEDED, STR_NEEDED);
        push_dynamic(&mut bytes, DT_SONAME, STR_SONAME);
        push_dynamic(&mut bytes, 5, strtab_offset as u32); // DT_STRTAB
        push_dynamic(&mut bytes, 6, dynsym_offset as u32); // DT_SYMTAB
        push_dynamic(&mut bytes, DT_NULL, 0);
        bytes
    }

    #[test]
    fn parses_symbols_needed_and_soname() {
        let library = parse(&build_elf()).expect("synthetic ELF parses");
        assert_eq!(library.format, "ELF32 ARM EABI5 shared object");
        assert_eq!(library.soname.as_deref(), Some("libgame.so"));
        assert_eq!(library.needed, vec!["libc.so".to_owned()]);
        assert_eq!(library.imports, vec!["malloc".to_owned()]);
        assert_eq!(library.export_count, 2);
        assert_eq!(
            library.jni_functions,
            vec!["Java_com_test_Game_start".to_owned()]
        );
        let summary = library.summary();
        assert!(summary.contains("2 exported, 1 imported"));
        assert!(summary.contains("soname libgame.so"));
        assert!(summary.contains("1 JNI"));
    }

    #[test]
    fn rejects_non_elf() {
        assert!(parse(b"not an elf").is_err());
        assert!(parse(&[]).is_err());
    }

    #[test]
    fn rejects_elf64() {
        let mut bytes = build_elf();
        bytes[4] = 2;
        let error = parse(&bytes).expect_err("ELF64 is rejected").to_string();
        assert!(error.contains("only ELF32"), "unexpected error: {error}");
    }

    #[test]
    fn detects_native_library_entries() {
        assert!(is_native_library_entry("lib/armeabi/libgame.so"));
        assert!(is_native_library_entry("lib/armeabi-v7a/libgame.so"));
        assert!(is_native_library_entry("lib/x86/libgame.so"));
        assert!(!is_native_library_entry("libgame.so"));
        assert!(!is_native_library_entry("lib/armeabi/"));
        assert!(!is_native_library_entry("lib/armeabi/libgame.dex"));
        assert!(!is_native_library_entry("assets/game_res/lib.so"));
        assert!(!is_native_library_entry("res/lib/armeabi/libgame.so"));
    }

    #[test]
    fn report_lines_cover_status_imports_and_jni() {
        let parsed = NativeLibraryReport {
            entry: "lib/armeabi/libgame.so".to_owned(),
            library: Some(parse(&build_elf()).unwrap()),
            error: None,
        };
        let broken = NativeLibraryReport::failed("lib/armeabi/libbad.so", "not ELF");
        let lines = format_report_lines(&[parsed, broken]);
        assert_eq!(lines.len(), 4);
        assert!(lines[0].starts_with("lib/armeabi/libgame.so: ELF32"));
        assert!(lines[1].starts_with("lib/armeabi/libgame.so imports: malloc"));
        assert!(lines[2].starts_with("lib/armeabi/libgame.so JNI entry points: Java_"));
        assert_eq!(lines[3], "lib/armeabi/libbad.so: unreadable (not ELF)");

        assert_eq!(format_report_lines(&[]), Vec::<String>::new());
    }

    #[test]
    fn status_summary_lists_entries_only_when_present() {
        assert_eq!(status_summary(&[]), None);
        let report = NativeLibraryReport {
            entry: "lib/armeabi/libgame.so".to_owned(),
            library: None,
            error: None,
        };
        let summary = status_summary(std::slice::from_ref(&report)).expect("summary");
        assert!(summary.contains("native execution is not implemented"));
        assert!(summary.contains("lib/armeabi/libgame.so"));
    }
}
