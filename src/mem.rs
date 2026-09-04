//! Sparse flat memory for the native-code machine.
//!
//! Guest memory is a list of disjoint contiguous regions. Reads and writes
//! that would leave a region fail with a diagnostic instead of wrapping, so
//! wild pointers surface as visible errors.

use anyhow::{bail, Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub base: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct Memory {
    regions: Vec<Region>,
}

impl Memory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Maps `size` zeroed bytes at `base`. Overlaps are rejected so that
    /// accidental double-mapping cannot silently alias.
    pub fn map_anon(&mut self, base: u32, size: u32) -> Result<()> {
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
        });
        self.regions.sort_by_key(|region| region.base);
        Ok(())
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

    fn region_for_mut(&mut self, address: u32, length: u32) -> Result<&mut Region> {
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
        Ok(region)
    }

    pub fn read_u8(&self, address: u32) -> Result<u8> {
        let region = self.region_for(address, 1)?;
        Ok(region.data[(address - region.base) as usize])
    }

    pub fn read_u16(&self, address: u32) -> Result<u16> {
        let region = self.region_for(address, 2)?;
        let offset = (address - region.base) as usize;
        Ok(u16::from_le_bytes(
            region.data[offset..offset + 2].try_into().unwrap(),
        ))
    }

    pub fn read_u32(&self, address: u32) -> Result<u32> {
        let region = self.region_for(address, 4)?;
        let offset = (address - region.base) as usize;
        Ok(u32::from_le_bytes(
            region.data[offset..offset + 4].try_into().unwrap(),
        ))
    }

    pub fn write_u8(&mut self, address: u32, value: u8) -> Result<()> {
        let region = self.region_for_mut(address, 1)?;
        region.data[(address - region.base) as usize] = value;
        Ok(())
    }

    pub fn write_u16(&mut self, address: u32, value: u16) -> Result<()> {
        let region = self.region_for_mut(address, 2)?;
        let offset = (address - region.base) as usize;
        region.data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    pub fn write_u32(&mut self, address: u32, value: u32) -> Result<()> {
        let region = self.region_for_mut(address, 4)?;
        let offset = (address - region.base) as usize;
        region.data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    pub fn read_bytes(&self, address: u32, length: usize) -> Result<Vec<u8>> {
        let region = self.region_for(address, length as u32)?;
        let offset = (address - region.base) as usize;
        Ok(region.data[offset..offset + length].to_vec())
    }

    pub fn write_bytes(&mut self, address: u32, bytes: &[u8]) -> Result<()> {
        let region = self.region_for_mut(address, bytes.len() as u32)?;
        let offset = (address - region.base) as usize;
        region.data[offset..offset + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }

    pub fn fill(&mut self, address: u32, value: u8, length: u32) -> Result<()> {
        let region = self.region_for_mut(address, length)?;
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
}
