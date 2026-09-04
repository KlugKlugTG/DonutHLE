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

/// Guest range backing imported data symbols (STT_OBJECT). Unlike host
/// function slots, this range is mapped writable guest memory so that data
/// reads and writes work.
pub const DATA_BASE: u32 = 0x7200_0000;
const DATA_REGION_SIZE: u32 = 0x0010_0000;

/// Deterministic initial values for imports whose content the guest assumes.
const STACK_CHK_GUARD: u32 = 0x5EED_C0DE;
const PAGE_SIZE: u32 = 4096;

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
    /// Symbol type is STT_OBJECT; `object_size` is its `st_size`.
    is_object: bool,
    object_size: u32,
}

/// Registry of host shims and loaded modules; resolves dynamic symbols.
#[derive(Debug, Default)]
pub struct Linker {
    host_names: Vec<String>,
    pub host_slots: HashMap<u32, usize>,
    next_host_address: u32,
    modules: Vec<(String, u32, Vec<GuestSymbol>)>,
    /// Imported data symbols (by name) backed with writable guest memory.
    data_symbols: HashMap<String, u32>,
    data_next: u32,
    data_region_mapped: bool,
    pub diagnostics: Vec<String>,
}

impl Linker {
    pub fn new() -> Self {
        Self {
            host_names: Vec::new(),
            host_slots: HashMap::new(),
            next_host_address: HOST_BASE,
            modules: Vec::new(),
            data_symbols: HashMap::new(),
            data_next: DATA_BASE,
            data_region_mapped: false,
            diagnostics: Vec::new(),
        }
    }

    /// Backs an imported data symbol with writable guest memory and returns
    /// its address. Known names receive deterministic initial values; the
    /// rest are zeroed. Returns `None` if the data region is exhausted.
    pub fn ensure_data_symbol(
        &mut self,
        name: &str,
        size: u32,
        memory: &mut Memory,
    ) -> Option<u32> {
        if let Some(address) = self.data_symbols.get(name) {
            return Some(*address);
        }
        if !self.data_region_mapped {
            memory.map_anon(DATA_BASE, DATA_REGION_SIZE).ok()?;
            self.data_region_mapped = true;
        }
        let size = size.max(4);
        let address = self.data_next;
        self.data_next = self.data_next.wrapping_add((size + 15) & !15);
        if self.data_next >= DATA_BASE + DATA_REGION_SIZE && size != 0 {
            return None; // data region exhausted
        }
        let initial = initial_bytes_for(name, size);
        memory.write_bytes(address, &initial).ok()?;
        self.data_symbols.insert(name.to_owned(), address);
        Some(address)
    }

    pub fn data_symbol(&self, name: &str) -> Option<u32> {
        self.data_symbols.get(name).copied()
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
        if let Some(address) = self.data_symbols.get(name) {
            return Some(*address);
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
                let st_size = u32_at(base + 8);
                let st_type = bytes[base + 12] & 0xF; // STT_OBJECT = 1, STT_FUNC = 2
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
                    is_object: st_type == 1,
                    object_size: st_size,
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

/// Initial content for known imported data symbols; everything else zeroes.
fn initial_bytes_for(name: &str, size: u32) -> Vec<u8> {
    let mut bytes = vec![0u8; size as usize];
    let value = match name {
        "__page_size" => Some(PAGE_SIZE),
        "__stack_chk_guard" => Some(STACK_CHK_GUARD),
        "__dso_handle" => Some(1), // any non-null handle; __aeabi_atexit ignores it
        _ => None,
    };
    if let Some(value) = value {
        if size >= 4 {
            bytes[..4].copy_from_slice(&value.to_le_bytes());
        }
    }
    bytes
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
                            } else if symbol.is_object {
                                // Data imports need readable/writable backing,
                                // not a call target: bind to guest memory.
                                linker
                                    .ensure_data_symbol(&symbol.name, symbol.object_size, memory)
                                    .unwrap_or_else(|| {
                                        unresolved_name = Some(symbol.name.clone());
                                        0
                                    })
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
            is_object: symbol.is_object,
            object_size: symbol.object_size,
        })
        .collect();
    linker.modules.push((name.to_owned(), base, symbols));

    Ok(module)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::arm::{CpuConfig, Machine};
    use crate::host::BasicHost;
    use crate::mem::Memory;

    /// Builds a minimal ARM shared object: two PT_LOADs, a dynamic section,
    /// six symbols (including two object imports), six relocations, and an
    /// init_array entry whose function reads `__stack_chk_guard`.
    fn build_elf() -> Vec<u8> {
        // Layout (file offset == vaddr for the single LOAD segment).
        const PHOFF: usize = 0x34;
        const CODE: usize = 0x080; // my_init (3 words) + helper (2 words)
        const HELPER: usize = 0x08C;
        const STRTAB: usize = 0x0A0;
        const DYNSYM: usize = 0x0E0;
        const HASH: usize = 0x140;
        const REL: usize = 0x180;
        const DATA: usize = 0x1C0; // relocated words + init_array
        const GUARD_PTR: usize = DATA + 0x0C;
        const DYNAMIC: usize = 0x1E0;
        const TOTAL: usize = 0x280;

        let strtab = b"\0my_init\0malloc\0helper\0libtest.so\0__stack_chk_guard\0__page_size\0";
        // Offsets: 1 my_init, 9 malloc, 16 helper, 23 libtest.so,
        // 34 __stack_chk_guard, 52 __page_size.

        let mut bytes: Vec<u8> = Vec::new();
        let u16v = |bytes: &mut Vec<u8>, value: u16| {
            bytes.extend_from_slice(&value.to_le_bytes());
        };
        let u32v = |bytes: &mut Vec<u8>, value: u32| {
            bytes.extend_from_slice(&value.to_le_bytes());
        };

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
        for (p_type, offset, vaddr, filesz, memsz) in [
            (PT_LOAD, 0u32, 0u32, TOTAL as u32, TOTAL as u32 + 0x100),
            (PT_DYNAMIC, DYNAMIC as u32, DYNAMIC as u32, 0x60, 0x60),
        ] {
            u32v(&mut bytes, p_type);
            u32v(&mut bytes, offset);
            u32v(&mut bytes, vaddr);
            u32v(&mut bytes, 0); // p_physaddr
            u32v(&mut bytes, filesz);
            u32v(&mut bytes, memsz);
            u32v(&mut bytes, 7); // p_flags
            u32v(&mut bytes, 0x1000); // p_align
        }

        // Code: my_init loads the guard pointer (ABS32-relocated) and then
        // the guard value itself; helper returns 44.
        bytes.resize(CODE, 0);
        let pc_relative = (GUARD_PTR - (CODE + 8)) as u32;
        u32v(&mut bytes, 0xE59F_0000 | pc_relative); // ldr r0, [pc, #off]
        u32v(&mut bytes, 0xE590_0000); // ldr r0, [r0]
        u32v(&mut bytes, 0xE12F_FF1E); // bx lr
        u32v(&mut bytes, 0xE3A0_002C); // helper: mov r0, #44
        u32v(&mut bytes, 0xE12F_FF1E); // bx lr

        // String table.
        bytes.resize(STRTAB, 0);
        bytes.extend_from_slice(strtab);

        // Dynamic symbols: null, my_init, malloc (func import), helper,
        // __stack_chk_guard (object import), __page_size (object import).
        bytes.resize(DYNSYM, 0);
        for (name, value, kind, size, shndx) in [
            (0u32, 0u32, 0u8, 0u32, 0u16),
            (1, CODE as u32, 2, 0, 1),    // my_init: GLOBAL FUNC
            (9, 0, 2, 0, 0),              // malloc: GLOBAL FUNC, undefined
            (16, HELPER as u32, 2, 0, 1), // helper: GLOBAL FUNC
            (34, 0, 1, 4, 0),             // __stack_chk_guard: OBJECT, undefined
            (52, 0, 1, 4, 0),             // __page_size: OBJECT, undefined
        ] {
            u32v(&mut bytes, name);
            u32v(&mut bytes, value);
            u32v(&mut bytes, size);
            bytes.push((0x10 | kind) << 4 >> 4); // GLOBAL, type in low nibble
            bytes.push(0);
            u16v(&mut bytes, shndx);
        }
        assert_eq!(bytes.len(), HASH);

        // DT_HASH: nbucket=1, nchain=6, one bucket, six chains.
        bytes.resize(HASH, 0);
        u32v(&mut bytes, 1);
        u32v(&mut bytes, 6);
        u32v(&mut bytes, 1);
        for _ in 0..7 {
            u32v(&mut bytes, 0);
        }

        // Relocations: RELATIVE for a data word and init_array, JUMP_SLOT for
        // malloc, ABS32 for helper, guard pointer, and page-size pointer.
        bytes.resize(REL, 0);
        for (offset, info) in [
            (DATA as u32, 23u32),               // R_ARM_RELATIVE, symbol 0
            (DATA as u32 + 4, (2 << 8) | 22),   // JUMP_SLOT malloc
            (DATA as u32 + 8, (3 << 8) | 2),    // ABS32 helper
            (GUARD_PTR as u32, (4 << 8) | 2),   // ABS32 __stack_chk_guard
            (DATA as u32 + 0x10, (5 << 8) | 2), // ABS32 __page_size
            (DATA as u32 + 0x14, 23),           // R_ARM_RELATIVE init_array
        ] {
            u32v(&mut bytes, offset);
            u32v(&mut bytes, info);
        }

        // Data words and init_array (points at my_init; RELATIVE re-binds it).
        bytes.resize(DATA, 0);
        u32v(&mut bytes, 0x1000); // RELATIVE target
        u32v(&mut bytes, 0); // malloc slot
        u32v(&mut bytes, 0); // helper
        u32v(&mut bytes, 0); // guard pointer
        u32v(&mut bytes, 0); // page-size pointer
        u32v(&mut bytes, CODE as u32); // init_array[0]
        assert_eq!(bytes.len(), DATA + 0x18);

        // Dynamic table.
        bytes.resize(DYNAMIC, 0);
        for (tag, value) in [
            (DT_NEEDED, 23u32), // libtest.so
            (DT_SONAME, 23),
            (DT_STRTAB, STRTAB as u32),
            (DT_SYMTAB, DYNSYM as u32),
            (DT_STRSZ, strtab.len() as u32),
            (DT_HASH, HASH as u32),
            (DT_REL, REL as u32),
            (DT_RELSZ, 48),
            (DT_RELENT, 8),
            (DT_INIT_ARRAY, (DATA + 0x14) as u32),
            (DT_INIT_ARRAYSZ, 4),
            (DT_NULL, 0),
        ] {
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
        let malloc_slot = linker.register_host("malloc");

        let module = load_module(&mut memory, &mut linker, &bytes, "test.so").expect("load");
        assert_eq!(module.base, FIRST_MODULE_BASE);
        assert_eq!(module.soname.as_deref(), Some("libtest.so"));
        assert_eq!(module.needed, vec!["libtest.so".to_owned()]);
        assert_eq!(module.unresolved, Vec::<String>::new());
        assert_eq!(module.applied_relocations, 6);

        // R_ARM_RELATIVE relocated the data word and the init_array entry.
        assert_eq!(
            memory.read_u32(module.base + 0x1C0).unwrap(),
            module.base + 0x1000
        );
        assert_eq!(module.init_array, vec![module.base + 0x080]);
        // JUMP_SLOT(malloc) points into the host trampoline range.
        let malloc_address = memory.read_u32(module.base + 0x1C4).unwrap();
        assert!(malloc_address >= HOST_BASE);
        assert_eq!(linker.host_slot_for(malloc_address), Some(malloc_slot));
        // ABS32(helper) resolved against the module itself.
        assert_eq!(
            memory.read_u32(module.base + 0x1C8).unwrap(),
            module.base + 0x08C
        );
    }

    #[test]
    fn object_imports_get_backing_storage() {
        let bytes = build_elf();
        let mut memory = Memory::new();
        let mut linker = Linker::new();
        let module = load_module(&mut memory, &mut linker, &bytes, "test.so").expect("load");

        // The guard pointer word resolved into the data region.
        let guard_address = memory.read_u32(module.base + 0x1CC).unwrap();
        assert!(
            guard_address >= super::DATA_BASE,
            "backed in the data region"
        );
        assert_eq!(linker.data_symbol("__stack_chk_guard"), Some(guard_address));

        // Known names carry deterministic values; reads do not fault.
        assert_eq!(memory.read_u32(guard_address).unwrap(), 0x5EED_C0DE);
        let page_address = memory.read_u32(module.base + 0x1D0).unwrap();
        assert_eq!(memory.read_u32(page_address).unwrap(), 4096);

        // Function imports still land on diagnostic slots.
        assert!(module.unresolved.contains(&"malloc".to_owned()));
        let malloc_address = memory.read_u32(module.base + 0x1C4).unwrap();
        assert!((HOST_BASE..super::DATA_BASE).contains(&malloc_address));
    }

    #[test]
    fn init_reads_stack_protector_through_backed_symbol() {
        let bytes = build_elf();
        let mut machine = Machine::new(CpuConfig::default());
        machine.memory.map_anon(0x7F00_0000, 0x10_0000).unwrap();
        machine.cpu.r[13] = 0x7F0F_F000;
        let mut host = BasicHost::new();
        let module =
            load_module(&mut machine.memory, &mut machine.linker, &bytes, "test.so").expect("load");

        // my_init dereferences the backed __stack_chk_guard pointer and
        // returns the guard value: a stack-protected function can run.
        let result = machine
            .call_function(&mut host, module.init_array[0], &[])
            .expect("init should run without faults");
        assert_eq!(result, 0x5EED_C0DE);
    }

    #[test]
    fn rejects_non_elf_images() {
        let mut memory = Memory::new();
        let mut linker = Linker::new();
        assert!(load_module(&mut memory, &mut linker, b"not an elf", "x.so").is_err());
    }
}
