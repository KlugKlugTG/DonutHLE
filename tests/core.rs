use donuthle::{dalvik::DexHeader, manifest::AppManifest, API_LEVEL, RELEASE};

#[test]
fn targets_donut() {
    assert_eq!(API_LEVEL, 4);
    assert_eq!(RELEASE, "Donut");
}

#[test]
fn rejects_non_dex() {
    assert!(DexHeader::parse(b"not a dex").is_err());
}

#[test]
fn parses_dex_035_header() {
    let mut bytes = vec![0u8; 116];
    bytes[0..8].copy_from_slice(b"dex\n035\0");
    bytes[0x20..0x24].copy_from_slice(&116u32.to_le_bytes());
    bytes[0x28..0x2c].copy_from_slice(&0x1234_5678u32.to_le_bytes());
    bytes[0x70..0x74].copy_from_slice(&112u32.to_le_bytes());
    let header = DexHeader::parse(&bytes).unwrap();
    assert_eq!(header.summary(), "DEX 035 / 116 bytes");
    assert!(header.validate_file_size(bytes.len()).is_ok());
}

#[test]
fn rejects_non_axml() {
    assert!(AppManifest::parse_axml(b"not xml").is_err());
}
