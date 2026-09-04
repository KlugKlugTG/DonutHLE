//! ELF32 program loading, dynamic linking, and host-symbol bridging.
//!
//! Loads shared objects into [`Memory`] at a chosen base, applies R_ARM
//! relocations, resolves imports against already-loaded modules and the host
//! shim registry, and collects the `init`/`init_array` entry points.

use std::collections::HashMap;

use anyhow::{bail, Context, Result};

use crate::mem::Memory;

pub const HOST_BASE: u32 = 0x7000_0000;
pub const FIRST_MODULE_BASE: u32 = 0x6000_0000;

const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const DT_NULL: u32 = 0;
const DT_NEEDED: u32 = 1;
const DT_HASH: u32 = 4;
const DT_STRTAB: u32 = 5;
const DT_SYMTAB: u32 = 6;
const DT_RELA: u32 = 7;
const DT_PLTRELSZ: u32 = 2;
const DT_JMPREL: u32 = 23;
const DT_STRSZ: u32 = 10;
const DT_REL: u32 = 17;
const DT_RELSZ: u32 = 18;
const DT_RELENT: u32 = 19;
const DT_INIT: u32 = 12;
const DT_FINI: u32 = 13;
const DT_SONAME: u32 = 14;
const DT_INIT_ARRAY: u32 = 25;
const DT_FINI_ARRAY: u32 = 26;
const DT_INIT_ARRAYSZ: u32 = 27;
const DT_GNU_HASH: u32 = 0x06ff_fef5;

const R_ARM_ABS32: u8 = 2;
// R_ARM_RELATIVE is dynamic type 23 on ARM (8 is the x86-64 number).
const R_ARM_RELATIVE: u8 = 23;
const R_ARM_GLOB_DAT: u8 = 21;
const R_ARM_JUMP_SLOT: u8 = 22;
const R_ARM_COPY: u8 = 26;

const SHN_UNDEF: u16 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedModule {
    pub name: String,
    pub base: u32,
    pub soname: Option<String>,
    pub needed: Vec<String>,
    pub init: Option<u32>,
    pub init_array: Vec<u32>,
    pub applied_relocations: usize,
    pub unresolved: Vec<String>,
    pub unsupported_relocations: Vec<String>,
}

#[derive(Debug, Clone)]
struct Segment {
    vaddr: u32,
    offset: u32,
    filesz: u32,
    memsz: u32,
}

#[derive(Debug, Clone)]
struct DynamicTable {
    strtab: u32,
    symtab: u32,
    strsz: u32,
    hash: Option<u32>,
    gnu_hash: Option<u32>,
    rel: Option<u32>,
    relsz: u32,
    relent: u32,
    jmprel: Option<u32>,
    pltrelsz: u32,
    init: Option<u32>,
    init_array: Option<u32>,
    init_arraysz: u32,
    soname: Option<u32>,
    needed: Vec<u32>,
}

#[derive(Debug, Clone)]
struct GuestSymbol {
    name: String,
    value: u32,
    defined: bool,
}

/// Registry of host shims and loaded modules; resolves dynamic symbols.
#[derive(Debug, Default)]
pub struct Linker {
    host_names: Vec<String>,
    pub host_slots: HashMap<u32, usize>,
    next_host_address: u32,
    modules: Vec<(String, u32, Vec<GuestSymbol>)>,
    pub diagnostics: Vec<String>,
}

impl Linker {
    pub fn new() -> Self {
        Self {
            host_names: Vec::new(),
            host_slots: HashMap::new(),
            next_host_address: HOST_BASE,
            modules: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Registers a host shim by ELF symbol name; returns its slot index.
    pub fn register_host(&mut self, name: &str) -> usize {
        if let Some(index) = self.host_names.iter().position(|existing| existing == name) {
            return index;
        }
        self.host_names.push(name.to_owned());
        self.host_names.len() - 1
    }

    pub fn host_index_of(&self, name: &str) -> Option<usize> {
        self.host_names.iter().position(|existing| existing == name)
    }

    pub fn host_name(&self, slot: usize) -> Option<&str> {
        self.host_names.get(slot).map(String::as_str)
    }

    pub fn host_count(&self) -> usize {
        self.host_names.len()
    }

    fn assign_host_address(&mut self, slot: usize) -> u32 {
        if let Some((address, _)) = self.host_slots.iter().find(|(_, value)| **value == slot) {
            return *address;
        }
        let address = self.next_host_address;
        self.next_host_address += 4;
        self.host_slots.insert(address, slot);
        address
    }

    /// Resolves a symbol against loaded modules, then registered host shims;
    /// unknown names receive a diagnostic host slot so that calling them
    /// produces a visible report instead of a null-pointer fault.
    pub fn resolve(&mut self, name: &str) -> Option<u32> {
        for (_, module_base, symbols) in &self.modules {
            if let Some(symbol) = symbols
                .iter()
                .find(|symbol| symbol.defined && symbol.name == name)
            {
                return Some(*module_base + symbol.value);
            }
        }
        self.host_index_of(name)
            .map(|slot| self.assign_host_address(slot))
    }

    pub fn host_slot_for(&self, address: u32) -> Option<usize> {
        self.host_slots.get(&address).copied()
    }
}

/// Parses the minimum of an ELF32 header needed for loading.
pub struct ElfImage {
    pub object_type: u16,
    segments: Vec<Segment>,
    dynamic: Option<DynamicTable>,
    symbols: Vec<GuestSymbol>,
    relocations: Vec<(u32, u8, u32)>,
}

impl ElfImage {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 52 {
            bail!("file is too short for an ELF32 header");
        }
        if &bytes[0..4] != b"\x7fELF" || bytes[4] != 1 || bytes[5] != 1 {
            bail!("not a little-endian ELF32 image");
        }
        let u16_at = |offset: usize| -> u16 {
            u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
        };
        let u32_at = |offset: usize| -> u32 {
            u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
        };
        let object_type = u16_at(16);
        let phoff = u32_at(28) as usize;
        let phentsize = u16_at(42) as usize;
        let phnum = u16_at(44) as usize;
        if phentsize < 32 {
            bail!("ELF program entry size {phentsize} is too small");
        }

        let mut segments = Vec::new();
        let mut dynamic_vaddr: Option<u32> = None;
        let mut dynamic_filesz = 0;
        for index in 0..phnum {
            let base = phoff + index * phentsize;
            if base + 32 > bytes.len() {
                bail!("ELF program headers extend past the end of the file");
            }
            let p_type = u32_at(base);
            let p_offset = u32_at(base + 4);
            let p_vaddr = u32_at(base + 8);
            let p_filesz = u32_at(base + 16);
            let p_memsz = u32_at(base + 20);
            if p_type == PT_LOAD {
                segments.push(Segment {
                    vaddr: p_vaddr,
                    offset: p_offset,
                    filesz: p_filesz,
                    memsz: p_memsz,
                });
            } else if p_type == PT_DYNAMIC {
                dynamic_vaddr = Some(p_vaddr);
                dynamic_filesz = p_filesz;
            }
        }
        if segments.is_empty() {
            bail!("ELF image has no PT_LOAD segments");
        }

        let mut dynamic_words: Vec<(u32, u32)> = Vec::new();
        if let Some(dynamic_vaddr) = dynamic_vaddr {
            for word in 0..(dynamic_filesz / 8) {
                let Some(offset) = base_offset(&segments, dynamic_vaddr + word * 8) else {
                    continue;
                };
                dynamic_words.push((u32_at(offset), u32_at(offset + 4)));
            }
        }

        let mut dynamic = None;
        if !dynamic_words.is_empty() {
            let mut table = DynamicTable {
                strtab: 0,
                symtab: 0,
                strsz: 0,
                hash: None,
                gnu_hash: None,
                rel: None,
                relsz: 0,
                relent: 8,
                jmprel: None,
                pltrelsz: 0,
                init: None,
                init_array: None,
                init_arraysz: 0,
                soname: None,
                needed: Vec::new(),
            };
            for (tag, value) in dynamic_words {
                match tag {
                    DT_NULL => break,
                    DT_NEEDED => table.needed.push(value),
                    DT_HASH => table.hash = Some(value),
                    DT_STRTAB => table.strtab = value,
                    DT_SYMTAB => table.symtab = value,
                    DT_STRSZ => table.strsz = value,
                    DT_REL => table.rel = Some(value),
                    DT_RELSZ => table.relsz = value,
                    DT_RELENT => table.relent = value,
                    DT_JMPREL => table.jmprel = Some(value),
                    DT_PLTRELSZ => table.pltrelsz = value,
                    DT_INIT => table.init = Some(value),
                    DT_INIT_ARRAY => table.init_array = Some(value),
                    DT_INIT_ARRAYSZ => table.init_arraysz = value,
                    DT_SONAME => table.soname = Some(value),
                    DT_GNU_HASH => table.gnu_hash = Some(value),
                    DT_RELA => bail!("ELF image uses RELA relocations, which ARM does not use"),
                    DT_FINI | DT_FINI_ARRAY => {
                        // FINI hooks are not executed by this emulator.
                    }
                    _ => {}
                }
            }
            dynamic = Some(table);
        }

        let mut symbols = Vec::new();
        let mut relocations = Vec::new();
        if let Some(table) = &dynamic {
            let strtab_offset = base_offset(&segments, table.strtab).with_context(|| {
                format!(
                    "dynamic string table {:#x} is not inside a PT_LOAD segment",
                    table.strtab
                )
            })?;
            let symtab_offset = base_offset(&segments, table.symtab).with_context(|| {
                format!(
                    "dynamic symbol table {:#x} is not inside a PT_LOAD segment",
                    table.symtab
                )
            })?;
            let count = symbol_count(bytes, &segments, table)?;
            for index in 0..count {
                let base = symtab_offset + index * 16;
                if base + 16 > bytes.len() {
                    bail!("dynamic symbol table extends past the end of the file");
                }
                let st_name = u32_at(base);
                let st_value = u32_at(base + 4);
                let st_shndx = u16_at(base + 14);
                let name = if st_name != 0 && (st_name as usize) < table.strsz as usize {
                    let start = strtab_offset + st_name as usize;
                    let stop = bytes[start..]
                        .iter()
                        .position(|byte| *byte == 0)
                        .map(|position| start + position)
                        .unwrap_or(bytes.len());
                    String::from_utf8_lossy(&bytes[start..stop]).into_owned()
                } else {
                    String::new()
                };
                symbols.push(GuestSymbol {
                    name,
                    value: st_value,
                    defined: st_shndx != SHN_UNDEF,
                });
            }
            // .rel.dyn holds general relocations; .rel.plt (DT_JMPREL) holds
            // the JUMP_SLOT entries that back the PLT stubs. Both share the
            // same entry format on ARM.
            for (table_vaddr, size) in [(table.rel, table.relsz), (table.jmprel, table.pltrelsz)] {
                let Some(rel_vaddr) = table_vaddr else {
                    continue;
                };
                let rel_offset = base_offset(&segments, rel_vaddr).with_context(|| {
                    format!("relocation table {rel_vaddr:#x} is not inside a PT_LOAD segment")
                })?;
                let entries = (size / table.relent.max(8)) as usize;
                for index in 0..entries {
                    let base = rel_offset + index * table.relent.max(8) as usize;
                    if base + 8 > bytes.len() {
                        bail!("relocation table extends past the end of the file");
                    }
                    let r_offset = u32_at(base);
                    let r_info = u32_at(base + 4);
                    relocations.push((r_offset, (r_info & 0xFF) as u8, r_info >> 8));
                }
            }
        }

        Ok(Self {
            object_type,
            segments,
            dynamic,
            symbols,
            relocations,
        })
    }

    fn load_span(&self) -> u32 {
        self.segments
            .iter()
            .map(|segment| segment.vaddr + segment.memsz)
            .max()
            .unwrap_or(0)
    }

    fn load(&self, bytes: &[u8], memory: &mut Memory, base: u32) -> Result<()> {
        memory.map_anon(base, self.load_span().next_multiple_of(0x1000))?;
        for segment in &self.segments {
            if segment.filesz > 0 {
                let source = segment.offset as usize;
                if source + segment.filesz as usize > bytes.len() {
                    bail!("PT_LOAD segment extends past the end of the file");
                }
                memory.write_bytes(
                    base + segment.vaddr,
                    &bytes[source..source + segment.filesz as usize],
                )?;
            }
        }
        Ok(())
    }
}

fn base_offset(segments: &[Segment], vaddr: u32) -> Option<usize> {
    segments.iter().find_map(|segment| {
        if segment.vaddr <= vaddr && vaddr < segment.vaddr + segment.filesz {
            Some(segment.offset as usize + (vaddr - segment.vaddr) as usize)
        } else {
            None
        }
    })
}

fn symbol_count(bytes: &[u8], segments: &[Segment], table: &DynamicTable) -> Result<usize> {
    if let Some(hash_vaddr) = table.hash {
        let offset =
            base_offset(segments, hash_vaddr).context("DT_HASH is not inside a PT_LOAD segment")?;
        if offset + 8 <= bytes.len() {
            let nchain = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap());
            return Ok(nchain as usize);
        }
    }
    if let Some(gnu_vaddr) = table.gnu_hash {
        let offset = base_offset(segments, gnu_vaddr)
            .context("DT_GNU_HASH is not inside a PT_LOAD segment")?;
        if offset + 16 <= bytes.len() {
            let nbucket =
                u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
            let symoffset =
                u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
            let bloom_size =
                u32::from_le_bytes(bytes[offset + 8..offset + 12].try_into().unwrap()) as usize;
            let buckets = offset + 16 + bloom_size * 4;
            let mut max_index = None;
            for bucket in 0..nbucket {
                let at = buckets + bucket * 4;
                if at + 4 > bytes.len() {
                    break;
                }
                let value = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
                if value < symoffset {
                    continue;
                }
                let mut index = value;
                loop {
                    let chain = buckets + nbucket * 4 + (index - symoffset) * 4;
                    if chain + 4 > bytes.len() {
                        break;
                    }
                    let word = u32::from_le_bytes(bytes[chain..chain + 4].try_into().unwrap());
                    if word & 1 == 1 {
                        break;
                    }
                    index += 1;
                }
                max_index = Some(max_index.unwrap_or(0).max(index));
            }
            return Ok(max_index.map_or(symoffset, |index| index + 1));
        }
    }
    bail!("cannot determine dynamic symbol count (no DT_HASH or DT_GNU_HASH)")
}

/// Loads an ELF image at a free base, links it, and returns its module record.
pub fn load_module(
    memory: &mut Memory,
    linker: &mut Linker,
    bytes: &[u8],
    name: &str,
) -> Result<LoadedModule> {
    let image = ElfImage::parse(bytes)?;
    // ET_DYN images load at a chosen base; ET_EXEC images own absolute
    // addresses and map where their program headers say.
    let mut base = if image.object_type == 2 {
        0
    } else {
        FIRST_MODULE_BASE
    };
    let span = image.load_span().next_multiple_of(0x1000);
    if image.object_type != 2 {
        let span = span.max(0x1000);
        let _ = &mut base;
        while !memory.is_range_free(base, base + span) {
            base += span;
        }
    }
    image.load(bytes, memory, base)?;

    let mut module = LoadedModule {
        name: name.to_owned(),
        base,
        soname: None,
        needed: Vec::new(),
        init: None,
        init_array: Vec::new(),
        applied_relocations: 0,
        unresolved: Vec::new(),
        unsupported_relocations: Vec::new(),
    };

    if let Some(table) = &image.dynamic {
        let strtab_offset = base_offset(&image.segments, table.strtab)
            .context("dynamic string table is not inside a PT_LOAD segment")?;
        let read_name = |offset: u32| -> String {
            let start = strtab_offset + offset as usize;
            if start >= bytes.len() {
                return String::new();
            }
            let stop = bytes[start..]
                .iter()
                .position(|byte| byte == &0)
                .map(|position| start + position)
                .unwrap_or(bytes.len());
            String::from_utf8_lossy(&bytes[start..stop]).into_owned()
        };
        module.needed = table.needed.iter().map(|entry| read_name(*entry)).collect();
        if let Some(soname) = table.soname {
            module.soname = Some(read_name(soname));
        }
        for (r_offset, r_type, r_symbol) in &image.relocations {
            let target = base + r_offset;
            match *r_type {
                R_ARM_RELATIVE => {
                    if let Ok(value) = memory.read_u32(target) {
                        memory.write_u32(target, value.wrapping_add(base))?;
                        module.applied_relocations += 1;
                    }
                }
                R_ARM_ABS32 | R_ARM_GLOB_DAT | R_ARM_JUMP_SLOT => {
                    let symbol = image.symbols.get(*r_symbol as usize);
                    let mut unresolved_name: Option<String> = None;
                    let resolved = symbol
                        .filter(|symbol| !symbol.name.is_empty())
                        .map(|symbol| {
                            if symbol.defined {
                                base + symbol.value
                            } else {
                                match linker.resolve(&symbol.name) {
                                    Some(value) => value,
                                    None => {
                                        unresolved_name = Some(symbol.name.clone());
                                        // Bind to a diagnostic slot so a call to
                                        // this import produces a visible report.
                                        let slot = linker.register_host(&symbol.name);
                                        linker.assign_host_address(slot)
                                    }
                                }
                            }
                        });
                    match resolved {
                        Some(value) => {
                            memory.write_u32(target, value)?;
                            module.applied_relocations += 1;
                            if let Some(name) = unresolved_name {
                                if !module.unresolved.contains(&name) {
                                    module.unresolved.push(name);
                                }
                            }
                        }
                        // Symbol index 0 (e.g. ABS32 against a local value)
                        // needs no resolution; only named imports that failed
                        // to bind are reported.
                        None => {
                            if let Some(name) = symbol
                                .map(|symbol| symbol.name.clone())
                                .filter(|name| !name.is_empty())
                            {
                                if !module.unresolved.contains(&name) {
                                    module.unresolved.push(name);
                                }
                            }
                        }
                    }
                }
                R_ARM_COPY => {
                    let message =
                        format!("{name}: R_ARM_COPY relocation at {r_offset:#x} is not supported");
                    module.unsupported_relocations.push(message.clone());
                    linker.diagnostics.push(message);
                }
                other => {
                    let message = format!(
                        "{name}: relocation type {other} at {r_offset:#x} is not supported"
                    );
                    module.unsupported_relocations.push(message.clone());
                    linker.diagnostics.push(message);
                }
            }
        }

        // init_array entries are read after relocations so that
        // R_ARM_RELATIVE entries re-bind them to the load base.
        if let Some(init) = table.init {
            module.init = Some(base + init);
        }
        if let Some(init_array) = table.init_array {
            if table.init_arraysz > 0 {
                let address = base + init_array;
                for index in 0..table.init_arraysz / 4 {
                    if let Ok(value) = memory.read_u32(address + index * 4) {
                        module.init_array.push(value);
                    }
                }
            }
        }
    }

    let symbols: Vec<GuestSymbol> = image
        .symbols
        .iter()
        .map(|symbol| GuestSymbol {
            name: symbol.name.clone(),
            value: symbol.value,
            defined: symbol.defined,
        })
        .collect();
    linker.modules.push((name.to_owned(), base, symbols));

    Ok(module)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal ARM shared object with two PT_LOADs, a dynamic
    /// section, three symbols, relocations, and an init_array entry.
    fn build_elf() -> Vec<u8> {
        let mut bytes: Vec<u8> = Vec::new();
        let u16v = |bytes: &mut Vec<u8>, value: u16| bytes.extend_from_slice(&value.to_le_bytes());
        let u32v = |bytes: &mut Vec<u8>, value: u32| bytes.extend_from_slice(&value.to_le_bytes());

        // Layout constants (file offset == vaddr for the single LOAD segment).
        const PHOFF: usize = 0x34;
        const CODE: usize = 0x100; // my_init: mov r0, #0x2A; bx lr
        const HELPER: usize = 0x108; // mov r0, #0x2C; bx lr
        const STRTAB: usize = 0x120;
        const DYNSYM: usize = 0x150;
        const HASH: usize = 0x190;
        const REL: usize = 0x1A8;
        const DATA: usize = 0x1D0; // three relocated words
        const INIT_ARRAY: usize = 0x1DC;
        const DYNAMIC: usize = 0x1E0;
        const TOTAL: usize = 0x240;

        // ELF header.
        bytes.extend_from_slice(b"\x7fELF");
        bytes.push(1); // ELFCLASS32
        bytes.push(1); // little-endian
        bytes.push(1); // EV_CURRENT
        bytes.extend(std::iter::repeat_n(0u8, 9));
        u16v(&mut bytes, 3); // ET_DYN
        u16v(&mut bytes, 40); // EM_ARM
        u32v(&mut bytes, 1); // e_version
        u32v(&mut bytes, 0); // e_entry
        u32v(&mut bytes, PHOFF as u32); // e_phoff
        u32v(&mut bytes, 0); // e_shoff
        u32v(&mut bytes, 0x0500_0000); // e_flags: EABI5
        u16v(&mut bytes, 52); // e_ehsize
        u16v(&mut bytes, 32); // e_phentsize
        u16v(&mut bytes, 2); // e_phnum
        u16v(&mut bytes, 0); // e_shentsize
        u16v(&mut bytes, 0); // e_shnum
        u16v(&mut bytes, 0); // e_shstrndx
        assert_eq!(bytes.len(), PHOFF);

        // Program headers: one PT_LOAD covering everything, one PT_DYNAMIC.
        for segment in [
            (0u32, 0u32, TOTAL as u32, TOTAL as u32 + 0x100),
            (DYNAMIC as u32, DYNAMIC as u32, 0x60, 0x60),
        ] {
            let (offset, vaddr, filesz, memsz) = segment;
            u32v(
                &mut bytes,
                if offset == DYNAMIC as u32 {
                    PT_DYNAMIC
                } else {
                    PT_LOAD
                },
            );
            u32v(&mut bytes, offset);
            u32v(&mut bytes, vaddr);
            u32v(&mut bytes, 0); // p_physaddr
            u32v(&mut bytes, filesz);
            u32v(&mut bytes, memsz);
            u32v(&mut bytes, 7); // p_flags
            u32v(&mut bytes, 0x1000); // p_align
        }
        assert_eq!(bytes.len(), 0x74);

        // Code: my_init returns 42; helper returns 44.
        bytes.resize(CODE, 0);
        u32v(&mut bytes, 0xE3A0_002A); // mov r0, #42
        u32v(&mut bytes, 0xE12F_FF1E); // bx lr
        u32v(&mut bytes, 0xE3A0_002C); // mov r0, #44
        u32v(&mut bytes, 0xE12F_FF1E); // bx lr

        // String table.
        bytes.resize(STRTAB, 0);
        bytes.extend_from_slice(b"\0my_init\0malloc\0helper\0libtest.so\0");
        // Offsets: 1 my_init, 9 malloc, 16 helper, 23 libtest.so
        assert!(bytes.len() <= DYNSYM);

        // Dynamic symbols: null, my_init (defined), malloc (import), helper.
        bytes.resize(DYNSYM, 0);
        for (name, value, shndx) in [
            (0u32, 0u32, 0u16),
            (1, CODE as u32, 1),
            (9, 0, 0),
            (16, HELPER as u32, 1),
        ] {
            u32v(&mut bytes, name);
            u32v(&mut bytes, value);
            u32v(&mut bytes, 0); // size
            bytes.push(0x12); // GLOBAL FUNC for functions
            bytes.push(0);
            u16v(&mut bytes, shndx);
        }
        assert_eq!(bytes.len(), HASH);

        // DT_HASH: nbucket=1, nchain=4, one bucket pointing at symbol 1.
        bytes.resize(HASH, 0);
        u32v(&mut bytes, 1);
        u32v(&mut bytes, 4);
        u32v(&mut bytes, 1);
        u32v(&mut bytes, 0);
        u32v(&mut bytes, 0);

        // Relocations: RELATIVE at DATA, JUMP_SLOT(malloc) at DATA+4,
        // ABS32(helper) at DATA+8.
        bytes.resize(REL, 0);
        for (offset, info) in [
            (DATA as u32, 23u32),             // R_ARM_RELATIVE, symbol 0
            (DATA as u32 + 4, (2 << 8) | 22), // JUMP_SLOT malloc
            (DATA as u32 + 8, (3 << 8) | 2),  // ABS32 helper
            (INIT_ARRAY as u32, 23u32),       // R_ARM_RELATIVE for init_array
        ] {
            u32v(&mut bytes, offset);
            u32v(&mut bytes, info);
        }
        assert_eq!(bytes.len(), 0x1C8);

        // Data words and init_array (points at my_init; a RELATIVE entry
        // above re-binds it to the load base).
        bytes.resize(DATA, 0);
        u32v(&mut bytes, 0x1000); // RELATIVE target
        u32v(&mut bytes, 0); // malloc slot
        u32v(&mut bytes, 0); // helper
        u32v(&mut bytes, CODE as u32); // init_array[0]
        assert_eq!(bytes.len(), DYNAMIC);

        // Dynamic table.
        let dyn_entries: [(u32, u32); 12] = [
            (DT_NEEDED, 23), // libtest.so
            (DT_SONAME, 23),
            (DT_STRTAB, STRTAB as u32),
            (DT_SYMTAB, DYNSYM as u32),
            (DT_STRSZ, 0x30),
            (DT_HASH, HASH as u32),
            (DT_REL, REL as u32),
            (DT_RELSZ, 32),
            (DT_RELENT, 8),
            (DT_INIT_ARRAY, INIT_ARRAY as u32),
            (DT_INIT_ARRAYSZ, 4),
            (DT_NULL, 0),
        ];
        // The relative relocation for init_array goes after the three above.
        // Append it to the REL region by extending RELSZ is not possible
        // post-hoc, so instead patch: add one more REL entry by rewriting
        // RELSZ below (the region has room up to DYNAMIC).
        for (tag, value) in dyn_entries {
            u32v(&mut bytes, tag);
            u32v(&mut bytes, value);
        }

        bytes.resize(TOTAL, 0);
        bytes
    }

    #[test]
    fn loads_links_and_binds_host_functions() {
        let bytes = build_elf();
        let mut memory = Memory::new();
        let mut linker = Linker::new();
        linker.register_host("malloc");
        let malloc_slot = linker.host_index_of("malloc").unwrap();

        let module = load_module(&mut memory, &mut linker, &bytes, "test.so").expect("load");
        assert_eq!(module.base, FIRST_MODULE_BASE);
        assert_eq!(module.soname.as_deref(), Some("libtest.so"));
        assert_eq!(module.needed, vec!["libtest.so".to_owned()]);
        assert_eq!(module.unresolved, Vec::<String>::new());
        assert_eq!(module.applied_relocations, 4);

        // R_ARM_RELATIVE relocated the data word and the init_array entry.
        assert_eq!(
            memory.read_u32(module.base + 0x1D0).unwrap(),
            module.base + 0x1000
        );
        assert_eq!(module.init_array, vec![module.base + 0x100]);
        // JUMP_SLOT(malloc) points into the host trampoline range.
        let malloc_address = memory.read_u32(module.base + 0x1D4).unwrap();
        assert!(malloc_address >= HOST_BASE);
        assert_eq!(linker.host_slot_for(malloc_address), Some(malloc_slot));
        // ABS32(helper) resolved against the module itself.
        assert_eq!(
            memory.read_u32(module.base + 0x1D8).unwrap(),
            module.base + 0x108
        );
    }

    #[test]
    fn unresolved_imports_get_diagnostic_slots() {
        let bytes = build_elf();
        let mut memory = Memory::new();
        let mut linker = Linker::new();
        let module = load_module(&mut memory, &mut linker, &bytes, "test.so").expect("load");
        assert!(module.unresolved.contains(&"malloc".to_owned()));
        let malloc_address = memory.read_u32(module.base + 0x1D4).unwrap();
        assert!(malloc_address >= HOST_BASE, "bound to a diagnostic slot");
        assert_eq!(linker.host_slot_for(malloc_address), Some(0));
        assert_eq!(linker.host_name(0), Some("malloc"));
    }

    #[test]
    fn rejects_non_elf_images() {
        let mut memory = Memory::new();
        let mut linker = Linker::new();
        assert!(load_module(&mut memory, &mut linker, b"not an elf", "x.so").is_err());
    }
}
