//! Bridge between the Dalvik VM and the native-code machine.
//!
//! `NativeBridge` owns the ARM machine for one APK: it stages `lib/<abi>/*.so`
//! bytes from the APK, loads them on demand (`System.loadLibrary`), installs
//! the JNI environment, and marshals `WrapperJinterface.native*` calls from
//! Dalvik into ARM entry points with `(env, jclass, args...)` and JNI handles
//! for Java arrays/strings.

use std::collections::HashMap;

use crate::arm::Machine;
use crate::elfload::{load_module, LoadedModule};
use crate::host::BasicHost;
use crate::jni;
use crate::vm::Value as DalvikValue;

/// Java-array metadata the native side can reach through JNI handles.
pub struct NativeBridge {
    pub machine: Machine,
    pub host: BasicHost,
    env: u32,
    /// Array handle -> (storage, length, element_size).
    java_arrays: HashMap<u32, (u32, u32, u32)>,
    java_strings: HashMap<u32, String>,
    class_handles: HashMap<String, u32>,
    /// Raw library bytes staged from the APK, keyed by `lib/<name>.so`.
    pending: HashMap<String, Vec<u8>>,
    loaded: Vec<LoadedModule>,
}

/// Strips the `lib/armeabi/` prefix from an APK entry name.
fn module_name_for(entry: &str) -> &str {
    entry.rsplit('/').next().unwrap_or(entry)
}

impl NativeBridge {
    /// Creates the machine, maps the runtime stack, and installs the JNI
    /// environment.
    pub fn new() -> Self {
        let mut machine = Machine::new(crate::arm::CpuConfig::default());
        machine
            .memory
            .map_anon(0x7F00_0000, 0x10_0000)
            .expect("stack maps");
        machine.cpu.r[13] = 0x7F0F_F000;
        for name in crate::host::HOST_FUNCTIONS {
            machine.linker.register_host(name);
        }
        for name in crate::host::GL_FUNCTIONS {
            machine.linker.register_host(name);
        }
        let env = jni::install(&mut machine);
        let mut host = BasicHost::new();
        host.gles = Some(crate::gles1_on_gl2::Gles1OnGl2::new(crate::VirtualScreen {
            width: 480,
            height: 320,
        }));
        Self {
            machine,
            host,
            env,
            java_arrays: HashMap::new(),
            java_strings: HashMap::new(),
            class_handles: HashMap::new(),
            pending: HashMap::new(),
            loaded: Vec::new(),
        }
    }

    /// Runs the wrapper's engine bring-up: nativePreInit(geometry, w, h)
    /// followed by nativeInit(time, upTime, pixel, gyro). Mirrors
    /// WrapperJinterface initialization with the game's Java-side arrays.
    pub fn boot_engine(&mut self, package: &str) -> String {
        const WRAPPER: &str = "com.com2us.wrapper.WrapperJinterface";

        if std::env::var_os("DONUTHLE_TRACE").is_some() {
            eprintln!(
                "boot_engine: env={:#x} mem[env]={:#x} vtable[6]={:#x}",
                self.env,
                self.machine.memory.read_u32(self.env).unwrap_or(0),
                self.machine
                    .memory
                    .read_u32(self.machine.memory.read_u32(self.env).unwrap_or(0) + 6 * 4)
                    .unwrap_or(0),
            );
        }
        let geometry = self.java_array(3, 4, &[0; 12]);
        match self.call_native(
            WRAPPER,
            "nativePreInit",
            &[
                DalvikValue::Object(geometry),
                DalvikValue::Int(320),
                DalvikValue::Int(480),
            ],
        ) {
            Ok(_) => {}
            Err(error) => return format!("nativePreInit stopped: {error}"),
        }
        let geometry_back = self.java_array_bytes(geometry).unwrap_or_default();
        self.host.log.push(format!(
            "nativePreInit geometry: {:?}",
            geometry_back
                .chunks(4)
                .map(|word| i32::from_le_bytes([word[0], word[1], word[2], word[3]]))
                .collect::<Vec<_>>()
        ));
        let system_time = self.java_array(4, 8, &[0; 32]);
        let up_time = self.java_array(64, 4, &[0; 256]);
        let pixel = self.java_array(320 * 480, 4, &[0; 320 * 480 * 4]);
        let gyro = self.java_array(3, 4, &[0; 12]);
        match self.call_native(
            WRAPPER,
            "nativeInit",
            &[
                DalvikValue::Object(system_time),
                DalvikValue::Object(up_time),
                DalvikValue::Object(pixel),
                DalvikValue::Object(gyro),
            ],
        ) {
            Ok(_) => {}
            Err(error) => return format!("nativeInit stopped: {error}"),
        }
        let _ = package;
        "engine bring-up complete".to_owned()
    }

    /// Renders one frame: nativeRender(timerIndex) then reports pixels.
    pub fn render_frame(&mut self) -> Result<usize, String> {
        if let Some(renderer) = self.host.gles.as_mut() {
            renderer.reset_frame_state();
            renderer.begin_frame();
            renderer.viewport(0, 0, 480, 320);
        }
        self.call_native(
            "com.com2us.wrapper.WrapperJinterface",
            "nativeRender",
            &[DalvikValue::Int(0)],
        )?;
        let pixels = self
            .host
            .gles
            .as_ref()
            .map(|renderer| renderer.rendered_pixels())
            .unwrap_or(0);
        Ok(pixels)
    }

    /// GLES framebuffer of the native render pipeline.
    pub fn framebuffer(&self) -> &crate::Framebuffer {
        self.host
            .gles
            .as_ref()
            .map(|renderer| renderer.framebuffer())
            .expect("native GLES context")
    }

    /// Number of libraries loaded into the machine this session.
    pub fn loaded_library_count(&self) -> usize {
        self.loaded.len()
    }

    /// Recent host log lines (library loading, JNI gaps).
    pub fn host_log(&self) -> Vec<String> {
        self.host.log.clone()
    }

    /// Stages a library's raw bytes from the APK for later `loadLibrary`.
    pub fn stage_library(&mut self, entry_name: &str, bytes: Vec<u8>) {
        self.pending.insert(entry_name.to_owned(), bytes);
    }

    /// Loads `lib/<name>.so` (already staged) if not loaded yet.
    pub fn load_library(&mut self, library: &str) {
        let entry = if library.ends_with(".so") {
            library.to_owned()
        } else {
            format!("lib{library}.so")
        };
        if self
            .loaded
            .iter()
            .any(|module| module.name == module_name_for(&entry))
        {
            return;
        }
        // APK entries are keyed as lib/<abi>/<name>.so; match by file name.
        let staged_entry = self
            .pending
            .keys()
            .find(|key| key.rsplit('/').next() == Some(entry.as_str()))
            .cloned();
        let Some(bytes) = staged_entry.and_then(|key| self.pending.get(&key).cloned()) else {
            self.host.log.push(format!(
                "System.loadLibrary({library}): {entry} is not in the APK"
            ));
            return;
        };
        let module_name = module_name_for(&entry).to_owned();
        match load_module(
            &mut self.machine.memory,
            &mut self.machine.linker,
            &bytes,
            &module_name,
        ) {
            Ok(module) => {
                self.host.log.push(format!(
                    "loaded {module_name}: {} relocations, {} unresolved imports",
                    module.applied_relocations,
                    module.unresolved.len()
                ));
                // Static initializers (DT_INIT + init_array) before any code
                // from the library runs.
                if let Some(init) = module.init {
                    if let Err(error) = self.machine.call_function(&mut self.host, init, &[0, 0, 0])
                    {
                        self.host
                            .log
                            .push(format!("{module_name} DT_INIT stopped: {error}"));
                    }
                }
                for (index, hook) in module.init_array.clone().into_iter().enumerate() {
                    if let Err(error) = self.machine.call_function(&mut self.host, hook, &[0, 0, 0])
                    {
                        self.host.log.push(format!(
                            "{module_name} init_array[{index}] stopped: {error}"
                        ));
                    }
                }
                self.loaded.push(module);
            }
            Err(error) => self
                .host
                .log
                .push(format!("loading {module_name} failed: {error}")),
        }
    }

    /// Creates a Java-side primitive array and remembers its geometry.
    pub fn java_array(&mut self, length: u32, element_size: u32, data: &[u8]) -> u32 {
        let handle = self
            .host
            .new_jni_array(&mut self.machine, length, element_size);
        if let Some(storage) = self.host.jni_array_data(handle) {
            self.machine.memory.write_bytes(storage, data).ok();
            self.java_arrays
                .insert(handle, (storage, length, element_size));
        }
        handle
    }

    /// Reads back a Java-side array's element data.
    pub fn java_array_bytes(&self, handle: u32) -> Option<Vec<u8>> {
        let (storage, length, element) = self.java_arrays.get(&handle)?;
        self.machine
            .memory
            .read_bytes(storage + 4, (*length as usize) * (*element as usize))
            .ok()
    }

    /// Creates a Java-side string.
    pub fn java_string(&mut self, text: &str) -> u32 {
        let handle = self.host.jni_new_string(&mut self.machine, text);
        self.java_strings.insert(handle, text.to_owned());
        handle
    }

    /// Runs a JNI native entry point. `name` is the raw Java method name
    /// (e.g. `nativeRender`); `class_name` is the declaring class in dotted
    /// form. Dalvik object values are JNI handles; `Int/Long/Float` pass
    /// through in registers; wide values are passed as their low half (the
    /// JNI marshalling of the caller already split them in the real ABI and
    /// the wrapper only uses 32-bit arguments here).
    pub fn call_native(
        &mut self,
        class_name: &str,
        name: &str,
        args: &[DalvikValue],
    ) -> Result<DalvikValue, String> {
        // Sentinel from the VM's System.loadLibrary interception.
        if name == "loadLibrary" {
            if let Some(DalvikValue::String(library)) = args.first().cloned() {
                self.load_library(&library);
                return Ok(DalvikValue::Void);
            }
        }
        let symbol = format!("Java_{}_{}", class_name.replace('.', "_"), name);
        let class_arg = match self.class_handles.get(class_name).copied() {
            Some(handle) => handle,
            None => {
                let handle = self.host.jni_class(&mut self.machine, class_name);
                self.class_handles.insert(class_name.to_owned(), handle);
                handle
            }
        };
        let mut arm_args = vec![self.env, class_arg];
        for value in args {
            arm_args.push(match value {
                DalvikValue::Int(value) => *value as u32,
                DalvikValue::Long(value) => (*value & 0xFFFF_FFFF) as u32,
                DalvikValue::Float(value) => value.to_bits(),
                DalvikValue::Double(value) => (*value as f32).to_bits(),
                DalvikValue::Object(handle) => *handle,
                DalvikValue::String(text) => {
                    let text = text.clone();
                    self.host.jni_new_string(&mut self.machine, &text)
                }
                DalvikValue::Void | DalvikValue::Null => 0,
            });
        }
        // AAPCS: arguments beyond r0-r3 are passed on the caller's stack.
        // Reserve room and write the extras so the callee's [sp] reads are
        // valid; call_function snapshots/restores r13 afterwards.
        let extra = arm_args.len().saturating_sub(4);
        if extra > 0 {
            let stack = self.machine.cpu.r[13] - (extra as u32) * 4;
            for (index, value) in arm_args[4..].iter().enumerate() {
                self.machine
                    .memory
                    .write_u32(stack + index as u32 * 4, *value)
                    .map_err(|error| format!("stack argument write failed: {error}"))?;
            }
            self.machine.cpu.r[13] = stack;
        }
        let address = self
            .machine
            .linker
            .resolve(&symbol)
            .ok_or_else(|| format!("native symbol {symbol} is not bound"))?;
        let result = self
            .machine
            .call_function(&mut self.host, address, &arm_args)?;
        Ok(DalvikValue::Int(result as i32))
    }
}

impl Default for NativeBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl BasicHost {
    pub fn jni_new_string(&mut self, machine: &mut Machine, text: &str) -> u32 {
        self.jni_string(machine, text)
    }
}
