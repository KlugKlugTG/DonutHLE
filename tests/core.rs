use donuthle::{
    dalvik::DexHeader, manifest::AppManifest, native, ANDROID_X_MAX_API_LEVEL,
    ANDROID_X_MIN_API_LEVEL, API_LEVEL, RELEASE,
};

#[test]
fn targets_android_1x_and_2x() {
    assert_eq!(ANDROID_X_MIN_API_LEVEL, 1);
    assert_eq!(ANDROID_X_MAX_API_LEVEL, 8);
    assert_eq!(API_LEVEL, ANDROID_X_MAX_API_LEVEL);
    assert_eq!(RELEASE, "Android 1.x-2.x");
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

#[test]
fn detects_native_library_entry_names() {
    assert!(native::is_native_library_entry("lib/armeabi/libgame.so"));
    assert!(native::is_native_library_entry("lib/x86/libgame.so"));
    assert!(!native::is_native_library_entry("assets/libgame.so"));
    assert!(!native::is_native_library_entry("lib/armeabi/data.bin"));
}

#[test]
fn native_report_keeps_parse_failures_visible() {
    let report = native::NativeLibraryReport::failed("lib/armeabi/libgame.so", "missing ELF magic");
    assert_eq!(
        report.status_line(),
        "lib/armeabi/libgame.so: unreadable (missing ELF magic)"
    );
    assert!(native::status_summary(std::slice::from_ref(&report)).is_some());
    assert_eq!(native::status_summary(&[]), None);
}

#[test]
fn instruction_limit_reports_hot_pcs() {
    use donuthle::dalvik::{ClassDef, CodeItem, DexFile, EncodedMethod, MethodId, Prototype};
    use donuthle::framework::Framework;
    use donuthle::vm::{Vm, VmConfig};

    // One method that loops forever: `goto +0` (opcode 0x28, offset 0).
    let dex = DexFile {
        header: donuthle::dalvik::DexHeader {
            version: "035".to_owned(),
            file_size: 0,
            header_size: 112,
            endian_tag: 0x1234_5678,
        },
        strings: vec!["LF;".to_owned(), "a".to_owned(), "()V".to_owned()],
        types: vec!["LF;".to_owned()],
        prototypes: vec![Prototype {
            shorty: "V".to_owned(),
            parameters: vec![],
            return_type: "V".to_owned(),
        }],
        fields: vec![],
        methods: vec![MethodId {
            class_name: "LF;".to_owned(),
            name: "a".to_owned(),
            prototype: "()V".to_owned(),
        }],
        classes: vec![ClassDef {
            name: "LF;".to_owned(),
            access_flags: 0x1,
            super_class: None,
            direct_methods: vec![],
            virtual_methods: vec![EncodedMethod {
                method_index: 0,
                access_flags: 0x1,
                code: Some(CodeItem {
                    registers_size: 1,
                    ins_size: 0,
                    outs_size: 0,
                    instructions: vec![0x0028],
                }),
            }],
        }],
    };

    std::env::set_var("DONUTHLE_STRICT_BUDGET", "1");
    let mut vm = Vm::new(
        &dex,
        Framework::default(),
        VmConfig {
            max_steps: 5_000,
            ..VmConfig::default()
        },
    );
    let error = vm.run_method(0, vec![]).unwrap_err();
    assert!(
        error.message.contains("hot pcs"),
        "unexpected message: {}",
        error.message
    );
    assert!(
        error.message.contains("pc=0 (0x28, 100%)"),
        "unexpected message: {}",
        error.message
    );
}
