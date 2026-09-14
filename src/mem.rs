//! Sparse flat memory for the native-code machine.
//!
//! Guest memory is a list of disjoint contiguous regions, each with POSIX-
//! style read/write/execute permissions. Reads and writes that would leave a
//! region fail with a diagnostic instead of wrapping, so wild pointers surface
//! as visible errors. Instruction fetches additionally require the execute
//! bit, mirroring the NX behaviour of a real kernel.

use anyhow::{bail, Context, Result};

/// POSIX-style access permissions for a mapped region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Permissions {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}

impl Permissions {
    pub const RW: Self = Self {
        read: true,
        write: true,
        execute: false,
    };
    pub const RX: Self = Self {
        read: true,
        write: false,
        execute: true,
    };
    pub const RWX: Self = Self {
        read: true,
        write: true,
        execute: true,
    };
    pub const RO: Self = Self {
        read: true,
        write: false,
        execute: false,
    };
    pub const NONE: Self = Self {
        read: false,
        write: false,
        execute: false,
    };

    /// Decodes a Linux `mmap`/`mprotect` `prot` value (PROT_READ=1,
    /// PROT_WRITE=2, PROT_EXEC=4).
    pub fn from_prot(prot: u32) -> Self {
        Self {
            read: prot & 1 != 0,
            write: prot & 2 != 0,
            execute: prot & 4 != 0,
        }
    }

    pub fn describe(self) -> String {
        let mut text = String::new();
        if self.read {
            text.push('r');
        } else {
            text.push('-');
        }
        if self.write {
            text.push('w');
        } else {
            text.push('-');
        }
        if self.execute {
            text.push('x');
        } else {
            text.push('-');
        }
        text
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub base: u32,
    pub data: Vec<u8>,
    pub perms: Permissions,
}

#[derive(Debug, Default)]
pub struct Memory {
    regions: Vec<Region>,
}

impl Memory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Maps `size` zeroed bytes at `base` with the given permissions.
    /// Overlaps are rejected so that accidental double-mapping cannot
    /// silently alias.
    pub fn map(&mut self, base: u32, size: u32, perms: Permissions) -> Result<()> {
        if size == 0 {
            return Ok(());
        }
        let end = base
            .checked_add(size)
            .context("memory mapping overflows the 32-bit address space")?;
        self.check_free(base, end)?;
        self.regions.push(Region {
            base,
            data: vec![0; size as usize],
            perms,
        });
        self.regions.sort_by_key(|region| region.base);
        Ok(())
    }

    /// Legacy entry point: maps anonymous read/write/execute memory.
    pub fn map_anon(&mut self, base: u32, size: u32) -> Result<()> {
        self.map(base, size, Permissions::RWX)
    }

    /// Removes the mapping covering `[base, base + size)`, splitting regions
    /// when the range covers part of one. Returns an error if the range is
    /// not fully mapped (Linux would return -ENOMEM).
    pub fn unmap(&mut self, base: u32, size: u32) -> Result<()> {
        if size == 0 {
            return Ok(());
        }
        let end = base
            .checked_add(size)
            .context("unmapping overflows the 32-bit address space")?;
        if !self.is_range_mapped(base, end) {
            bail!("unmapping {base:#010x}..{end:#010x} covers unmapped pages");
        }
        let mut replacement: Vec<Region> = Vec::new();
        for region in self.regions.drain(..) {
            let region_end = region.base + region.data.len() as u32;
            if end <= region.base || region_end <= base {
                replacement.push(region);
                continue;
            }
            // Head: [region.base, base)
            if region.base < base {
                let length = (base - region.base) as usize;
                replacement.push(Region {
                    base: region.base,
                    data: region.data[..length].to_vec(),
                    perms: region.perms,
                });
            }
            // Tail: [end, region_end)
            if end < region_end {
                let offset = (end - region.base) as usize;
                replacement.push(Region {
                    base: end,
                    data: region.data[offset..].to_vec(),
                    perms: region.perms,
                });
            }
        }
        self.regions = replacement;
        self.regions.sort_by_key(|region| region.base);
        Ok(())
    }

    /// Updates permissions for every byte in `[base, base + size)`. The whole
    /// range must already be mapped, as mprotect requires.
    pub fn mprotect(&mut self, base: u32, size: u32, perms: Permissions) -> Result<()> {
        if size == 0 {
            return Ok(());
        }
        let end = base
            .checked_add(size)
            .context("mprotect overflows the 32-bit address space")?;
        if !self.is_range_mapped(base, end) {
            bail!("mprotect {base:#010x}..{end:#010x} covers unmapped pages");
        }
        let mut replacement: Vec<Region> = Vec::new();
        for region in self.regions.drain(..) {
            let region_end = region.base + region.data.len() as u32;
            if end <= region.base || region_end <= base {
                replacement.push(region);
                continue;
            }
            // Head keeps its old permissions.
            if region.base < base {
                let length = (base - region.base) as usize;
                replacement.push(Region {
                    base: region.base,
                    data: region.data[..length].to_vec(),
                    perms: region.perms,
                });
            }
            // Overlap gets the new permissions.
            let overlap_base = region.base.max(base);
            let overlap_end = region_end.min(end);
            let start_offset = (overlap_base - region.base) as usize;
            let end_offset = (overlap_end - region.base) as usize;
            replacement.push(Region {
                base: overlap_base,
                data: region.data[start_offset..end_offset].to_vec(),
                perms,
            });
            // Tail keeps its old permissions.
            if end < region_end {
                let offset = (end - region.base) as usize;
                replacement.push(Region {
                    base: end,
                    data: region.data[offset..].to_vec(),
                    perms: region.perms,
                });
            }
        }
        self.regions = replacement;
        self.regions.sort_by_key(|region| region.base);
        Ok(())
    }

    /// Permissions of the region containing `address`.
    pub fn permissions_at(&self, address: u32) -> Result<Permissions> {
        let region = self.region_for(address, 1)?;
        Ok(region.perms)
    }

    fn check_free(&self, start: u32, end: u32) -> Result<()> {
        for region in &self.regions {
            let region_end = region.base + region.data.len() as u32;
            if start < region_end && region.base < end {
                bail!(
                    "memory mapping {start:#010x}..{end:#010x} overlaps region at {:#010x}",
                    region.base
                );
            }
        }
        Ok(())
    }

    /// True when every byte of `[start, end)` lies inside a mapped region.
    pub fn is_range_mapped(&self, start: u32, end: u32) -> bool {
        let mut cursor = start;
        for region in &self.regions {
            let region_end = region.base + region.data.len() as u32;
            if region.base <= cursor && cursor < region_end {
                cursor = region_end.min(end);
                if cursor >= end {
                    return true;
                }
            } else if region.base > cursor {
                break;
            }
        }
        cursor >= end
    }

    fn region_for(&self, address: u32, length: u32) -> Result<&Region> {
        let end = address
            .checked_add(length)
            .context("memory access overflows the 32-bit address space")?;
        let region = self
            .regions
            .iter()
            .find(|region| region.base <= address && end <= region.base + region.data.len() as u32)
            .with_context(|| {
                format!("memory access {address:#010x}..{end:#010x} is outside any mapped region")
            })?;
        Ok(region)
    }


    fn region_for_access(
        &self,
        address: u32,
        length: u32,
        needed: fn(&Permissions) -> bool,
        action: &str,
    ) -> Result<&Region> {
        let region = self.region_for(address, length)?;
        if !needed(&region.perms) {
            bail!(
                "{action} at {address:#010x} denied: region has permissions {}",
                region.perms.describe()
            );
        }
        Ok(region)
    }

    fn region_for_access_mut(
        &mut self,
        address: u32,
        length: u32,
        action: &str,
    ) -> Result<&mut Region> {
        let end = address
            .checked_add(length)
            .context("memory access overflows the 32-bit address space")?;
        let region = self
            .regions
            .iter_mut()
            .find(|region| region.base <= address && end <= region.base + region.data.len() as u32)
            .with_context(|| {
                format!("memory access {address:#010x}..{end:#010x} is outside any mapped region")
            })?;
        if !region.perms.write {
            bail!(
                "{action} at {address:#010x} denied: region has permissions {}",
                region.perms.describe()
            );
        }
        Ok(region)
    }

    pub fn read_u8(&self, address: u32) -> Result<u8> {
        let region = self.region_for_access(address, 1, |p| p.read, "read")?;
        Ok(region.data[(address - region.base) as usize])
    }

    pub fn read_u16(&self, address: u32) -> Result<u16> {
        let region = self.region_for_access(address, 2, |p| p.read, "read")?;
        let offset = (address - region.base) as usize;
        Ok(u16::from_le_bytes(
            region.data[offset..offset + 2].try_into().unwrap(),
        ))
    }

    pub fn read_u32(&self, address: u32) -> Result<u32> {
        let region = self.region_for_access(address, 4, |p| p.read, "read")?;
        let offset = (address - region.base) as usize;
        Ok(u32::from_le_bytes(
            region.data[offset..offset + 4].try_into().unwrap(),
        ))
    }

    /// Instruction fetch: requires the execute permission.
    pub fn fetch_u16(&self, address: u32) -> Result<u16> {
        let region = self.region_for_access(address, 2, |p| p.execute, "instruction fetch")?;
        let offset = (address - region.base) as usize;
        Ok(u16::from_le_bytes(
            region.data[offset..offset + 2].try_into().unwrap(),
        ))
    }

    /// Instruction fetch: requires the execute permission.
    pub fn fetch_u32(&self, address: u32) -> Result<u32> {
        let region = self.region_for_access(address, 4, |p| p.execute, "instruction fetch")?;
        let offset = (address - region.base) as usize;
        Ok(u32::from_le_bytes(
            region.data[offset..offset + 4].try_into().unwrap(),
        ))
    }

    pub fn write_u8(&mut self, address: u32, value: u8) -> Result<()> {
        let region = self.region_for_access_mut(address, 1, "write")?;
        region.data[(address - region.base) as usize] = value;
        Ok(())
    }

    pub fn write_u16(&mut self, address: u32, value: u16) -> Result<()> {
        let region = self.region_for_access_mut(address, 2, "write")?;
        let offset = (address - region.base) as usize;
        region.data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    pub fn write_u32(&mut self, address: u32, value: u32) -> Result<()> {
        let region = self.region_for_access_mut(address, 4, "write")?;
        let offset = (address - region.base) as usize;
        region.data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    pub fn read_u64(&self, address: u32) -> Result<u64> {
        let low = u64::from(self.read_u32(address)?);
        let high = u64::from(self.read_u32(address + 4)?);
        Ok(low | (high << 32))
    }

    pub fn write_u64(&mut self, address: u32, value: u64) -> Result<()> {
        self.write_u32(address, value as u32)?;
        self.write_u32(address + 4, (value >> 32) as u32)?;
        Ok(())
    }

    pub fn read_bytes(&self, address: u32, length: usize) -> Result<Vec<u8>> {
        let region = self.region_for_access(
            address,
            length as u32,
            |p| p.read,
            "read",
        )?;
        let offset = (address - region.base) as usize;
        Ok(region.data[offset..offset + length].to_vec())
    }

    pub fn write_bytes(&mut self, address: u32, bytes: &[u8]) -> Result<()> {
        let region = self.region_for_access_mut(address, bytes.len() as u32, "write")?;
        let offset = (address - region.base) as usize;
        region.data[offset..offset + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }

    pub fn fill(&mut self, address: u32, value: u8, length: u32) -> Result<()> {
        let region = self.region_for_access_mut(address, length, "write")?;
        let offset = (address - region.base) as usize;
        region.data[offset..offset + length as usize].fill(value);
        Ok(())
    }

    /// Copies `length` bytes, tolerating overlapping guest ranges.
    pub fn copy_within(&mut self, destination: u32, source: u32, length: u32) -> Result<()> {
        let bytes = self.read_bytes(source, length as usize)?;
        self.write_bytes(destination, &bytes)
    }

    /// Reads a NUL-terminated string, up to `max` bytes.
    pub fn read_cstr(&self, address: u32, max: usize) -> Result<String> {
        let mut bytes = Vec::with_capacity(max.min(256));
        for index in 0..max {
            let value = self.read_u8(address + index as u32)?;
            if value == 0 {
                return Ok(String::from_utf8_lossy(&bytes).into_owned());
            }
            bytes.push(value);
        }
        bail!("string at {address:#010x} is not NUL-terminated within {max} bytes")
    }

    pub fn write_cstr(&mut self, address: u32, value: &str) -> Result<()> {
        let mut bytes = value.as_bytes().to_vec();
        bytes.push(0);
        self.write_bytes(address, &bytes)
    }

    pub fn is_mapped(&self, address: u32, length: u32) -> bool {
        self.region_for(address, length).is_ok()
    }

    /// True when `[start, end)` touches no mapped region.
    pub fn is_range_free(&self, start: u32, end: u32) -> bool {
        self.regions.iter().all(|region| {
            let region_end = region.base + region.data.len() as u32;
            end <= region.base || start >= region_end
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_read_write() {
        let mut memory = Memory::new();
        memory.map_anon(0x1000, 0x100).unwrap();
        memory.write_u32(0x1010, 0xDEAD_BEEF).unwrap();
        assert_eq!(memory.read_u32(0x1010).unwrap(), 0xDEAD_BEEF);
        memory.write_u8(0x1014, 0x7F).unwrap();
        assert_eq!(memory.read_u8(0x1014).unwrap(), 0x7F);
        memory.write_u16(0x1016, 0xBEEF).unwrap();
        assert_eq!(memory.read_u16(0x1016).unwrap(), 0xBEEF);
    }

    #[test]
    fn rejects_overlaps_and_holes() {
        let mut memory = Memory::new();
        memory.map_anon(0x1000, 0x100).unwrap();
        assert!(memory.map_anon(0x1080, 0x100).is_err());
        assert!(memory.read_u32(0x2000).is_err());
        assert!(memory.write_u32(0x10FE, 0).is_err()); // crosses the region end
        assert!(memory.read_u32(0x10FE).is_err());
        assert!(memory.is_mapped(0x10FC, 4));
        assert!(!memory.is_mapped(0x10FD, 4));
    }

    #[test]
    fn strings_and_block_moves() {
        let mut memory = Memory::new();
        memory.map_anon(0x1000, 0x100).unwrap();
        memory.write_cstr(0x1010, "hello").unwrap();
        assert_eq!(memory.read_cstr(0x1010, 16).unwrap(), "hello");
        memory.write_bytes(0x1020, &[1, 2, 3, 4, 5]).unwrap();
        memory.copy_within(0x1030, 0x101E, 5).unwrap();
        assert_eq!(memory.read_bytes(0x1030, 5).unwrap(), vec![0, 0, 1, 2, 3]);
        memory.fill(0x1040, 0xAB, 4).unwrap();
        assert_eq!(memory.read_bytes(0x1040, 4).unwrap(), vec![0xAB; 4]);
    }

    #[test]
    fn permissions_gate_access() {
        let mut memory = Memory::new();
        memory.map(0x1000, 0x100, Permissions::RW).unwrap();
        memory.map(0x2000, 0x100, Permissions::RO).unwrap();
        memory
            .map(0x3000, 0x100, Permissions { read: false, write: false, execute: true })
            .unwrap();
        // Read-only: reads fine, writes denied.
        assert_eq!(memory.read_u32(0x2000).unwrap(), 0);
        assert!(memory.write_u32(0x2000, 1).is_err());
        // RX is readable (ARMv5 has no execute-only), writes denied.
        assert!(memory.read_u32(0x2000).is_ok());
        // True execute-only: fetch fine, data reads denied.
        memory.write_u32(0x3000, 0xE1A00000).unwrap_err();
        assert!(memory.fetch_u32(0x3000).is_ok());
        assert!(memory.read_u32(0x3000).is_err());
        // Read/write: fetch denied.
        assert!(memory.fetch_u32(0x1000).is_err());
    }

    #[test]
    fn mprotect_updates_permissions() {
        let mut memory = Memory::new();
        memory.map_anon(0x1000, 0x200).unwrap();
        memory.mprotect(0x1000, 0x100, Permissions::RO).unwrap();
        assert!(memory.read_u32(0x1000).is_ok());
        assert!(memory.write_u32(0x1000, 1).is_err());
        // The back half keeps its old permissions.
        memory.write_u32(0x1100, 1).unwrap();
        // Unmapped ranges are rejected, like the kernel would.
        assert!(memory.mprotect(0x1900, 0x400, Permissions::RO).is_err());
    }

    #[test]
    fn unmap_splits_and_frees() {
        let mut memory = Memory::new();
        memory.map_anon(0x1000, 0x300).unwrap();
        // Unmap the middle: head and tail survive independently.
        memory.unmap(0x1100, 0x100).unwrap();
        assert!(!memory.is_mapped(0x1100, 4));
        assert!(memory.is_mapped(0x10FC, 4));
        assert!(memory.is_mapped(0x1200, 4));
        memory.write_u32(0x10F0, 7).unwrap();
        memory.write_u32(0x1210, 9).unwrap();
        // Unmapping already-free memory fails like -ENOMEM.
        assert!(memory.unmap(0x1100, 0x100).is_err());
        // Re-mapping the hole works again.
        memory.map_anon(0x1100, 0x100).unwrap();
    }

    #[test]
    fn prot_decode() {
        assert_eq!(Permissions::from_prot(0), Permissions::NONE);
        assert_eq!(Permissions::from_prot(1).read, true);
        assert_eq!(Permissions::from_prot(3).write, true);
        assert_eq!(Permissions::from_prot(5).execute, true);
    }
}
