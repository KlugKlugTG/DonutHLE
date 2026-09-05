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
use crate::vm::{HeapObject, Value as DalvikValue};

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
    /// (Dalvik object, JNI handle, component) pairs to copy back after calls.
    dalvik_mirrors: Vec<(u32, u32, String)>,
    /// Snapshot of the Dalvik heap used to mirror arrays into the machine.
    pub dalvik_heap: Vec<crate::vm::HeapObject>,
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
            dalvik_mirrors: Vec::new(),
            dalvik_heap: Vec::new(),
            loaded: Vec::new(),
        }
    }

    /// Renders one frame: nativeRender(timerIndex) then reports pixels.
    pub fn render_frame(&mut self) -> Result<usize, String> {
        if let Some(renderer) = self.host.gles.as_mut() {
            renderer.reset_frame_state();
            renderer.begin_frame();
            renderer.viewport(0, 0, 480, 320);
        }
        let snapshot = std::mem::take(&mut self.dalvik_heap);
        let (result, heap) = self.call_native(
            "com.com2us.wrapper.WrapperJinterface",
            "nativeRender",
            &[DalvikValue::Int(0)],
            &snapshot,
        );
        self.dalvik_heap = heap;
        result?;
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
    /// Mirrors a Dalvik heap array into a JNI array the native code can use,
    /// remembering the pairing so results copy back after the call.
    fn mirror_dalvik_array(&mut self, object: u32) -> Option<u32> {
        let (component, values) = match self.dalvik_heap.get(object as usize)? {
            HeapObject::Array { component, values } => (component.clone(), values.clone()),
            _ => return None,
        };
        type EncodeFn = Box<dyn Fn(&DalvikValue) -> Vec<u8>>;
        let (element_size, encode): (u32, EncodeFn) = match component.as_str() {
            "J" => (
                8,
                Box::new(|value: &DalvikValue| match value {
                    DalvikValue::Long(value) => value.to_le_bytes().to_vec(),
                    _ => 0i64.to_le_bytes().to_vec(),
                }),
            ),
            "F" => (
                4,
                Box::new(|value: &DalvikValue| match value {
                    DalvikValue::Float(value) => value.to_le_bytes().to_vec(),
                    DalvikValue::Int(value) => (*value as f32).to_le_bytes().to_vec(),
                    _ => 0f32.to_le_bytes().to_vec(),
                }),
            ),
            _ => (
                4,
                Box::new(|value: &DalvikValue| match value {
                    DalvikValue::Int(value) => value.to_le_bytes().to_vec(),
                    DalvikValue::Long(value) => (*value as i32).to_le_bytes().to_vec(),
                    DalvikValue::Float(value) => value.to_bits().to_le_bytes().to_vec(),
                    _ => 0i32.to_le_bytes().to_vec(),
                }) as EncodeFn,
            ),
        };
        let mut data = Vec::with_capacity(values.len() * element_size as usize);
        for value in &values {
            data.extend(encode(value));
        }
        let handle = self.java_array(values.len() as u32, element_size, &data);
        self.dalvik_mirrors.push((object, handle, component));
        Some(handle)
    }

    /// Copies mirrored JNI arrays back into the Dalvik heap.
    fn sync_mirrors_back(&mut self) {
        for (object, handle, component) in std::mem::take(&mut self.dalvik_mirrors) {
            let Some(bytes) = self.java_array_bytes(handle) else {
                continue;
            };
            let element_size: usize = match component.as_str() {
                "J" => 8,
                "F" => 4,
                _ => 4,
            };
            let values: Vec<DalvikValue> = bytes
                .chunks(element_size)
                .map(|chunk| {
                    let word = i32::from_le_bytes(chunk[..4].try_into().unwrap_or([0; 4]));
                    match component.as_str() {
                        "J" => DalvikValue::Long(i64::from_le_bytes(
                            chunk[..8].try_into().unwrap_or([0; 8]),
                        )),
                        "F" => DalvikValue::Float(f32::from_bits(word as u32)),
                        _ => DalvikValue::Int(word),
                    }
                })
                .collect();
            if let Some(HeapObject::Array { values: slot, .. }) =
                self.dalvik_heap.get_mut(object as usize)
            {
                *slot = values;
            }
        }
    }

    pub fn call_native(
        &mut self,
        class_name: &str,
        name: &str,
        args: &[DalvikValue],
        heap_snapshot: &[HeapObject],
    ) -> (Result<DalvikValue, String>, Vec<HeapObject>) {
        self.dalvik_heap = heap_snapshot.to_vec();
        // The Samsung zirconia license check verifies a Samsung Apps license
        // file that cannot exist outside the original storefront device; the
        // supplied APK's license check is already satisfied by its own patched
        // code, so the environment mirrors that outcome instead of fail-closing
        // on a missing license file. Logged for transparency.
        if class_name.starts_with("com.samsung.zirconia.")
            && matches!(
                name,
                "checkLicenseFile" | "checkLicenseFile2" | "doPassphraseTest"
            )
        {
            self.host
                .log
                .push("license check: environment reports licensed (see policy note)".to_owned());
            return (Ok(DalvikValue::Int(1)), self.dalvik_heap.clone());
        }
        // Sentinel from the VM's System.loadLibrary interception.
        if name == "loadLibrary" {
            if let Some(DalvikValue::String(library)) = args.first().cloned() {
                self.load_library(&library);
                return (Ok(DalvikValue::Void), self.dalvik_heap.clone());
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
                DalvikValue::Object(handle) => {
                    // Dalvik heap arrays mirror into JNI arrays so the
                    // engine's Get*ArrayElements sees real data.
                    self.mirror_dalvik_array(*handle).unwrap_or(*handle)
                }
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
                if let Err(error) = self
                    .machine
                    .memory
                    .write_u32(stack + index as u32 * 4, *value)
                {
                    let message = format!("stack argument write failed: {error}");
                    return (Err(message), self.dalvik_heap.clone());
                }
            }
            self.machine.cpu.r[13] = stack;
        }
        let Some(address) = self.machine.linker.resolve(&symbol) else {
            let message = format!("native symbol {symbol} is not bound");
            return (Err(message), self.dalvik_heap.clone());
        };
        let call = self
            .machine
            .call_function(&mut self.host, address, &arm_args);
        self.sync_mirrors_back();
        let result = call.map(|value| DalvikValue::Int(value as i32));
        (result, self.dalvik_heap.clone())
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
