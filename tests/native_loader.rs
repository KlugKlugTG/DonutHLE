//! End-to-end native-library loading against real APK contents.
//!
//! These tests are skipped unless `DONUTHLE_OVENBREAK_LIBS` points at a
//! directory containing `libgame.so` and `libnativeinterface.so` (for example
//! the decompiled OvenBreak APK tree). The repository ships no game files.

use std::path::Path;

use donuthle::arm::Machine;
use donuthle::elfload::load_module;
use donuthle::host::{BasicHost, HOST_FUNCTIONS};

fn ovenbreak_libraries() -> Option<std::path::PathBuf> {
    let directory = std::env::var_os("DONUTHLE_OVENBREAK_LIBS")?;
    let directory = std::path::PathBuf::from(directory);
    if directory.join("libgame.so").is_file() {
        Some(directory)
    } else {
        None
    }
}

fn load_real_libraries() -> (Machine, BasicHost, Vec<String>) {
    let directory =
        ovenbreak_libraries().expect("DONUTHLE_OVENBREAK_LIBS must point at the libraries");
    let mut machine = Machine::new(donuthle::arm::CpuConfig::default());
    machine
        .memory
        .map_anon(0x7F00_0000, 0x10_0000)
        .expect("stack maps");
    machine.cpu.r[13] = 0x7F0F_F000;
    for name in HOST_FUNCTIONS {
        machine.linker.register_host(name);
    }
    let mut loaded = Vec::new();
    for name in ["libnativeinterface.so", "libgame.so"] {
        let path = directory.join(name);
        let bytes =
            std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let module = load_module(&mut machine.memory, &mut machine.linker, &bytes, name)
            .unwrap_or_else(|error| panic!("load {name}: {error}"));
        let mut kinds: std::collections::BTreeMap<String, usize> = Default::default();
        for message in &module.unsupported_relocations {
            let kind = message
                .split("relocation type ")
                .nth(1)
                .unwrap_or("?")
                .to_owned();
            *kinds.entry(kind).or_default() += 1;
        }
        eprintln!(
            "{name}: base={:#x} relocations={} unresolved_count={} has_mutex_init_unresolved={} pthread_mutex_init_resolves={} unresolved={:?} unsupported_count={:?} jni={} init={:?}",
            module.base,
            module.applied_relocations,
            module.unresolved.len(),
            module.unresolved.contains(&"pthread_mutex_init".to_owned()),
            machine.linker.resolve("pthread_mutex_init").is_some(),
            module.unresolved.clone(),
            kinds,
            machine
                .linker
                .resolve("Java_com_com2us_wrapper_WrapperJinterface_nativeRender")
                .is_some(),
            module.init_array.len(),
        );
        loaded.push(format!("{}@{:#x}", name, module.base));
    }
    (machine, BasicHost::new(), loaded)
}

#[test]
fn links_real_ovenbreak_libraries() {
    let Some(_) = ovenbreak_libraries() else {
        eprintln!("skipped: set DONUTHLE_OVENBREAK_LIBS to run");
        return;
    };
    let (mut machine, _host, loaded) = load_real_libraries();
    assert_eq!(loaded.len(), 2);
    // Imported data symbols are backed with writable guest memory and carry
    // their expected values; the guard is readable by protected functions.
    for name in ["__stack_chk_guard", "__page_size", "__sF", "__dso_handle"] {
        let address = machine
            .linker
            .data_symbol(name)
            .unwrap_or_else(|| panic!("{name} must be backed"));
        assert!(
            address >= donuthle::elfload::DATA_BASE,
            "{name} in the data region"
        );
        // Reading must not fault.
        let _ = machine.memory.read_u32(address).unwrap();
    }
    let guard = machine.linker.data_symbol("__stack_chk_guard").unwrap();
    assert_eq!(
        machine.memory.read_u32(guard).unwrap(),
        0x5EED_C0DE,
        "guard carries its deterministic value"
    );
    // Both libraries resolved their JNI entry points.
    for name in [
        "Java_com_com2us_wrapper_WrapperJinterface_nativeInit",
        "Java_com_com2us_wrapper_WrapperJinterface_nativeRender",
        "Java_com_com2us_wrapper_WrapperUserDefined_StartGame",
    ] {
        let address = machine
            .linker
            .resolve(name)
            .unwrap_or_else(|| panic!("{name} must resolve"));
        assert!(address >= 0x6000_0000, "{name} points into a module");
    }
}

#[test]
fn runs_real_library_init_when_requested() {
    let Some(directory) = ovenbreak_libraries() else {
        eprintln!("skipped: set DONUTHLE_OVENBREAK_LIBS to run");
        return;
    };
    if std::env::var_os("DONUTHLE_RUN_NATIVE_INIT").is_none() {
        eprintln!("skipped: set DONUTHLE_RUN_NATIVE_INIT to execute init_array");
        return;
    }
    let mut machine = Machine::new(donuthle::arm::CpuConfig::default());
    machine.memory.map_anon(0x7F00_0000, 0x10_0000).unwrap();
    machine.cpu.r[13] = 0x7F0F_F000;
    for name in HOST_FUNCTIONS {
        machine.linker.register_host(name);
    }
    let mut host = BasicHost::new();
    // libgame.so's static initializers may call unimplemented imports; the
    // diagnostic log records exactly which gaps remain.
    for name in ["libnativeinterface.so", "libgame.so"] {
        let path = Path::new(&directory).join(name);
        let bytes = std::fs::read(&path).unwrap();
        let module = load_module(&mut machine.memory, &mut machine.linker, &bytes, name).unwrap();
        eprintln!(
            "running init for {name}: {} hooks",
            module.init_array.len() + usize::from(module.init.is_some())
        );
        if let Some(init) = module.init {
            match machine.call_function(&mut host, init, &[module.base, 0, 0]) {
                Ok(_) => eprintln!("  DT_INIT ok"),
                Err(error) => eprintln!("  DT_INIT stopped: {error}"),
            }
        }
        for (index, hook) in module.init_array.clone().into_iter().enumerate() {
            match machine.call_function(&mut host, hook, &[module.base, 0, 0]) {
                Ok(_) => eprintln!("  init_array[{index}] ok"),
                Err(error) => eprintln!("  init_array[{index}] stopped: {error}"),
            }
        }
        eprintln!(
            "  GOT[0x7b1e8] = {:#x} (host range starts at 0x70000000)",
            machine.memory.read_u32(module.base + 0x7B1E8).unwrap_or(0)
        );
        eprintln!("  diagnostics: {:?}", host.log);
    }
}

/// Harness mirroring the Java wrapper's boot sequence: `nativePreInit`
/// receives `int[3]` plus display width/height and writes back a geometry
/// triple; `nativeInit` receives the time/up-time/pixel/gyro arrays whose
/// element storage the engine keeps for rendering.
#[test]
fn runs_real_jni_entry_points_when_requested() {
    let Some(directory) = ovenbreak_libraries() else {
        eprintln!("skipped: set DONUTHLE_OVENBREAK_LIBS to run");
        return;
    };
    let mut machine = Machine::new(donuthle::arm::CpuConfig::default());
    machine.memory.map_anon(0x7F00_0000, 0x10_0000).unwrap();
    machine.cpu.r[13] = 0x7F0F_F000;
    for name in HOST_FUNCTIONS {
        machine.linker.register_host(name);
    }
    let mut host = BasicHost::new();

    for name in ["libnativeinterface.so", "libgame.so"] {
        let path = Path::new(&directory).join(name);
        let bytes = std::fs::read(&path).unwrap();
        let module = load_module(&mut machine.memory, &mut machine.linker, &bytes, name).unwrap();
        if let Some(init) = module.init {
            let _ = machine.call_function(&mut host, init, &[module.base, 0, 0]);
        }
        for hook in module.init_array.clone() {
            let _ = machine.call_function(&mut host, hook, &[module.base, 0, 0]);
        }
    }

    let env = donuthle::jni::install(&mut machine);
    let log_and_clear = |host: &mut BasicHost, label: &str| {
        eprintln!("--- {label} ---");
        for line in &host.log {
            eprintln!("  {line}");
        }
        host.log.clear();
    };

    // Java-side arrays, as the wrapper builds them before calling native code.
    let parameter = host.new_jni_array(&mut machine, 3, 4);
    let system_time = host.new_jni_array(&mut machine, 4, 8);
    let up_time = host.new_jni_array(&mut machine, 64, 4);
    let pixel = host.new_jni_array(&mut machine, 320 * 480, 4);
    let gyro = host.new_jni_array(&mut machine, 3, 4);

    // static nativePreInit(int[] parameter, int width, int height)
    let pre_init = machine
        .linker
        .resolve("Java_com_com2us_wrapper_WrapperJinterface_nativePreInit")
        .expect("nativePreInit resolves");
    let result = machine.call_function(&mut host, pre_init, &[env, env, parameter, 320, 480]);
    let parameter_data = host.jni_array_data(parameter).unwrap_or(0);
    if result.is_err() {
        eprintln!(
            "  registers on fault: r0={:08x} r1={:08x} r2={:08x} r3={:08x} r12={:08x}",
            machine.cpu.r[0],
            machine.cpu.r[1],
            machine.cpu.r[2],
            machine.cpu.r[3],
            machine.cpu.r[12]
        );
        eprintln!(
            "  vtable[187] = {:#x}; env word = {:#x}",
            machine.memory.read_u32(env + 0x100 + 187 * 4).unwrap_or(0),
            machine.memory.read_u32(env).unwrap_or(0)
        );
    }
    eprintln!(
        "nativePreInit -> {result:?}; geometry = {:?}",
        machine
            .memory
            .read_bytes(parameter_data, 12)
            .unwrap_or_default()
    );
    log_and_clear(&mut host, "nativePreInit log");

    // static nativeInit(long[] time, int[] upTime, int[] pixel, float[] gyro)
    let init = machine
        .linker
        .resolve("Java_com_com2us_wrapper_WrapperJinterface_nativeInit")
        .expect("nativeInit resolves");
    let result = machine.call_function(
        &mut host,
        init,
        &[env, env, system_time, up_time, pixel, gyro],
    );
    eprintln!("nativeInit -> {result:?}");
    log_and_clear(&mut host, "nativeInit log");
}
