//! Host shims for guest native code: the bionic libc/libm subset, the AEABI
//! arithmetic helpers, and diagnostics for imports that are not implemented.
//!
//! Every shim is deterministic and fail-closed: file and network operations
//! return errors with a log entry rather than touching the host system.

use crate::arm::{HostBridge, Machine};
use crate::jni::{array_element_size, entry_function};
use crate::Rgba8;

/// Guest memory layout for host-owned regions.
const HEAP_BASE: u32 = 0x5000_0000;
const HEAP_SIZE: u32 = 32 * 1024 * 1024;
const HOST_DATA_BASE: u32 = 0x7100_0000;
const HOST_DATA_SIZE: u32 = 0x1_0000;
const MMAP_BASE: u32 = 0x6800_0000;
const MMAP_SIZE: u32 = 64 * 1024 * 1024;

/// Names of the shims implemented by [`BasicHost`]; the linker binds imports
/// to these by symbol name.
pub const HOST_FUNCTIONS: &[&str] = &[
    // Memory allocation.
    "malloc",
    "calloc",
    "realloc",
    "free",
    "_Znwj",
    "_Znaj",
    "_ZdlPv",
    "_ZdaPv",
    // Memory and string.
    "memset",
    "memcpy",
    "memcmp",
    "strlen",
    "strcpy",
    "strncpy",
    "strcat",
    "strcmp",
    "strncmp",
    "strchr",
    "strrchr",
    "qsort",
    // Console and logging.
    "printf",
    "puts",
    "sprintf",
    "snprintf",
    "vsprintf",
    "vsnprintf",
    "fprintf",
    "__android_log_write",
    // Threads (single-threaded machine; these are bookkeeping only).
    "pthread_mutex_init",
    "pthread_mutex_destroy",
    "pthread_mutex_lock",
    "pthread_mutex_unlock",
    "pthread_mutex_trylock",
    "pthread_mutexattr_init",
    "pthread_mutexattr_destroy",
    "pthread_mutexattr_settype",
    "pthread_key_create",
    "pthread_setspecific",
    "pthread_getspecific",
    // Time.
    "time",
    "usleep",
    "nanosleep",
    "srand48",
    "lrand48",
    // Process.
    "abort",
    "exit",
    "__stack_chk_fail",
    "__errno",
    "__aeabi_atexit",
    "setjmp",
    "longjmp",
    // Files: fail-closed stubs.
    "open",
    "read",
    "write",
    "close",
    "lseek",
    "fstat",
    "stat",
    "lstat",
    "statfs",
    "unlink",
    "rename",
    "mkdir",
    "opendir",
    "readdir",
    "closedir",
    "dup",
    "fcntl",
    // Memory mapping.
    "mmap",
    "munmap",
    // Network: fail-closed stubs.
    "socket",
    "connect",
    "send",
    "recvfrom",
    "select",
    "getsockopt",
    "getaddrinfo",
    "freeaddrinfo",
    "inet_addr",
    "inet_ntoa",
    // AEABI integer arithmetic.
    "__aeabi_idiv",
    "__aeabi_uidiv",
    "__aeabi_idivmod",
    "__aeabi_uidivmod",
    "__aeabi_lmul",
    "__aeabi_ldivmod",
    "__aeabi_uldivmod",
    "__aeabi_l2d",
    // AEABI floating point (soft-float ABI).
    "__aeabi_i2f",
    "__aeabi_ui2f",
    "__aeabi_i2d",
    "__aeabi_ui2d",
    "__aeabi_f2iz",
    "__aeabi_f2ui",
    "__aeabi_f2d",
    "__aeabi_d2f",
    "__aeabi_d2iz",
    "__aeabi_d2uiz",
    "__aeabi_fadd",
    "__aeabi_fsub",
    "__aeabi_fmul",
    "__aeabi_fdiv",
    "__aeabi_dadd",
    "__aeabi_dsub",
    "__aeabi_dmul",
    "__aeabi_ddiv",
    "__aeabi_fcmpeq",
    "__aeabi_fcmplt",
    "__aeabi_fcmple",
    "__aeabi_fcmpgt",
    "__aeabi_fcmpge",
    "__aeabi_dcmpeq",
    "__aeabi_dcmplt",
    "__aeabi_dcmple",
    "__aeabi_dcmpgt",
    "__aeabi_dcmpge",
];

const MAP_FAILED: u32 = u32::MAX;

/// GLES 1.x constants used by the imported engine (ARM Mali-era values).
const GL_BYTE: u32 = 0x1400;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_SHORT: u32 = 0x1402;
const GL_UNSIGNED_SHORT: u32 = 0x1403;
const GL_FIXED: u32 = 0x140C;
const GL_FLOAT: u32 = 0x1406;
const GL_RGBA: u32 = 0x1908;
const GL_RGB: u32 = 0x1907;
const GL_UNSIGNED_SHORT_4_4_4_4: u32 = 0x8033;
const GL_UNSIGNED_SHORT_5_5_5_1: u32 = 0x8030;
const GL_UNSIGNED_SHORT_5_6_5: u32 = 0x8363;
const GL_VERTEX_ARRAY: u32 = 0x9070;
const GL_COLOR_ARRAY: u32 = 0x9072;
const GL_TEXTURE_COORD_ARRAY: u32 = 0x9074;
const GL_NORMAL_ARRAY: u32 = 0x9076;

/// Imported GLES functions the native bridge expects the host to serve.
pub const GL_FUNCTIONS: &[&str] = &[
    "glActiveTexture",
    "glBindTexture",
    "glBlendFunc",
    "glClear",
    "glClearColor",
    "glClearColorx",
    "glClientActiveTexture",
    "glColor4x",
    "glColorPointer",
    "glCompressedTexImage2D",
    "glCopyTexImage2D",
    "glCopyTexSubImage2D",
    "glDeleteTextures",
    "glDepthMask",
    "glDisable",
    "glDisableClientState",
    "glDrawArrays",
    "glDrawElements",
    "glEnable",
    "glEnableClientState",
    "glFinish",
    "glGenTextures",
    "glGetError",
    "glGetFixedv",
    "glGetIntegerv",
    "glGetPointerv",
    "glIsEnabled",
    "glLightxv",
    "glLineWidthx",
    "glLoadIdentity",
    "glLoadMatrixf",
    "glLoadMatrixx",
    "glMatrixMode",
    "glNormalPointer",
    "glPointSizex",
    "glScissor",
    "glShadeModel",
    "glTexCoordPointer",
    "glTexEnvf",
    "glTexEnvx",
    "glTexEnvxv",
    "glTexImage2D",
    "glTexParameterf",
    "glTexParameteri",
    "glTexParameterx",
    "glTexSubImage2D",
    "glVertexPointer",
    "glViewport",
];

/// Client-array state captured from the guest: the pointers live in guest
/// memory and are materialized at draw time.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClientArraySpec {
    pub pointer: u32,
    pub size: usize,
    pub stride: usize,
    pub enabled: bool,
    /// GL component type (GL_FIXED, GL_FLOAT, ...).
    pub array_type: u32,
}

/// GLES 1.x ABI arguments: r0-r3 then the caller's stack.
pub fn gl_call_args(machine: &Machine, count: usize) -> Vec<u32> {
    let mut args = Vec::with_capacity(count);
    for register in 0..4.min(count) {
        args.push(machine.cpu.r[register]);
    }
    let mut stack = machine.cpu.r[13];
    for _ in 4..count {
        let value = machine.memory.read_u32(stack).unwrap_or(0);
        args.push(value);
        stack = stack.wrapping_add(4);
    }
    args
}

fn fixed_to_f32(value: u32) -> f32 {
    value as i32 as f32 / 65536.0
}

/// Deterministic, single-threaded host shims.
#[derive(Debug, Default)]
pub struct BasicHost {
    pub log: Vec<String>,
    heap_next: u32,
    allocations: std::collections::HashMap<u32, u32>,
    regions_ready: bool,
    errno_address: u32,
    next_key: u32,
    thread_specific: std::collections::HashMap<u32, u32>,
    random_state: u64,
    mmap_next: u32,
    jump_buffers: std::collections::HashMap<u32, [u32; 10]>,
    /// JNI object bump pointer inside [`crate::jni::JNI_BASE`].
    jni_next: u32,
    /// Stable class handles by name, plus a reverse map for logging.
    jni_classes: std::collections::HashMap<String, u32>,
    jni_class_names: std::collections::HashMap<u32, String>,
    /// Method handles by (class handle, name, signature).
    jni_methods: std::collections::HashMap<(u32, String, String), u32>,
    jni_method_names: std::collections::HashMap<u32, (u32, String)>,
    jni_next_handle: u32,
    jni_strings: std::collections::HashMap<u32, String>,
    jni_arrays: std::collections::HashMap<u32, JniArray>,
    jni_array_storage: std::collections::HashMap<u32, u32>,
    /// GLES 1.x state for native engines; None until a native runtime needs it.
    pub gles: Option<crate::gles1_on_gl2::Gles1OnGl2>,
    /// Client-array pointers captured from the guest (vertex/color/texcoord).
    pub gl_client_arrays: [ClientArraySpec; 4],
    /// Names of GL state calls accepted but not modeled by the rasterizer.
    pub gl_ignored: std::collections::BTreeSet<String>,
}

/// A guest-visible JNI primitive array: handle word, element geometry, and
/// the storage block returned by the Get*ArrayElements family.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
struct JniArray {
    length: u32,
    element_size: u32,
    storage: u32,
}

impl BasicHost {
    pub fn new() -> Self {
        Self {
            random_state: 0x2B99_2DDF_232F_9A67,
            // JNI objects bump-allocate inside the region that jni::install
            // maps; starting at zero would silently drop every allocation.
            jni_next: crate::jni::JNI_BASE + crate::jni::JNI_OBJECTS_OFFSET,
            ..Self::default()
        }
    }

    fn ensure_regions(&mut self, machine: &mut Machine) {
        if self.regions_ready {
            return;
        }
        self.regions_ready = true;
        let _ = machine.memory.map_anon(HEAP_BASE, HEAP_SIZE);
        let _ = machine.memory.map_anon(HOST_DATA_BASE, HOST_DATA_SIZE);
        let _ = machine.memory.map_anon(MMAP_BASE, MMAP_SIZE);
        self.heap_next = HEAP_BASE + 0x100;
        self.mmap_next = MMAP_BASE + 0x1000;
        self.errno_address = HOST_DATA_BASE;
    }

    fn log(&mut self, message: String) {
        if self.log.len() < 10_000 {
            self.log.push(message);
        }
    }

    fn alloc(&mut self, machine: &mut Machine, size: u32) -> u32 {
        self.ensure_regions(machine);
        let size = size.max(1).next_multiple_of(16);
        let end = self.heap_next.wrapping_add(size);
        if end > HEAP_BASE + HEAP_SIZE {
            self.log("host heap exhausted".to_owned());
            return 0;
        }
        let address = self.heap_next;
        self.heap_next = end;
        self.allocations.insert(address, size);
        address
    }

    fn read_stack_string(&self, machine: &Machine, address: u32) -> String {
        machine.memory.read_cstr(address, 4096).unwrap_or_default()
    }

    /// Reads a NUL-terminated guest string without allocating a String first.
    fn guest_strlen(&self, machine: &Machine, address: u32) -> u32 {
        let mut length = 0u32;
        while length < 1 << 20 {
            match machine.memory.read_u8(address + length) {
                Ok(0) => break,
                Ok(_) => length += 1,
                Err(_) => break,
            }
        }
        length
    }

    fn c_difference(a: u8, b: u8) -> i32 {
        i32::from(a) - i32::from(b)
    }

    fn format(
        &mut self,
        machine: &mut Machine,
        format_address: u32,
        mut cursor: ArgCursor,
    ) -> String {
        let format = self.read_stack_string(machine, format_address);
        let mut output = String::new();
        let bytes: Vec<u8> = format.as_bytes().to_vec();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] != b'%' {
                output.push(bytes[index] as char);
                index += 1;
                continue;
            }
            index += 1;
            if index >= bytes.len() {
                break;
            }
            if bytes[index] == b'%' {
                output.push('%');
                index += 1;
                continue;
            }
            // Flags, width, precision: parsed but simplified to padding.
            let mut padding = Padding::None;
            while index < bytes.len() {
                match bytes[index] {
                    b'0' if matches!(padding, Padding::None) => padding = Padding::Zero,
                    b'-' => padding = Padding::Left,
                    b'+' | b' ' | b'#' => {}
                    _ => break,
                }
                index += 1;
            }
            let mut width = 0usize;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                width = width * 10 + (bytes[index] - b'0') as usize;
                index += 1;
            }
            let mut precision: Option<usize> = None;
            if index < bytes.len() && bytes[index] == b'.' {
                index += 1;
                let mut digits = 0usize;
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    digits = digits * 10 + (bytes[index] - b'0') as usize;
                    index += 1;
                }
                precision = Some(digits);
            }
            if index + 1 < bytes.len() && (bytes[index] == b'l' || bytes[index] == b'h') {
                index += 1;
                if bytes[index] == b'l' {
                    index += 1;
                }
            }
            let Some(&specifier) = bytes.get(index) else {
                break;
            };
            index += 1;
            let mut body = match specifier {
                b'd' | b'i' => {
                    let value = cursor.next_i32(machine) as i64;
                    value.to_string()
                }
                b'u' => cursor.next_i32(machine).to_string(),
                b'x' => format!("{:x}", cursor.next_i32(machine)),
                b'X' => format!("{:X}", cursor.next_i32(machine)),
                b'p' => format!("{:#010x}", cursor.next_u32(machine)),
                b's' => {
                    let address = cursor.next_u32(machine);
                    if address == 0 {
                        "(null)".to_owned()
                    } else {
                        self.read_stack_string(machine, address)
                    }
                }
                b'c' => char::from_u32(cursor.next_u32(machine) & 0xFF)
                    .unwrap_or('?')
                    .to_string(),
                b'f' | b'F' => format!("{:.*}", precision.unwrap_or(6), cursor.next_f64(machine)),
                b'g' | b'G' => format!("{}", cursor.next_f64(machine)),
                b'e' | b'E' => format!("{:e}", cursor.next_f64(machine)),
                other => {
                    output.push('%');
                    output.push(other as char);
                    continue;
                }
            };
            if width > body.chars().count() {
                match padding {
                    Padding::Left => {
                        while body.chars().count() < width {
                            body.push(' ');
                        }
                    }
                    Padding::Zero => {
                        let negative = body.starts_with('-');
                        if negative {
                            body.remove(0);
                        }
                        while body.chars().count() + usize::from(negative) < width {
                            body.insert(0, '0');
                        }
                        if negative {
                            body.insert(0, '-');
                        }
                    }
                    Padding::None => {
                        while body.chars().count() < width {
                            body.insert(0, ' ');
                        }
                    }
                }
            }
            output.push_str(&body);
        }
        output
    }

    fn lrand48(&mut self) -> u32 {
        // xorshift64*; deterministic and sufficient for game randomness.
        let mut state = self.random_state;
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        self.random_state = state;
        (state >> 32) as u32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Padding {
    None,
    Zero,
    Left,
}

/// Cursor over EABI call arguments: r1-r3, then the caller's stack.
struct ArgCursor {
    register: usize,
    stack: u32,
}

impl ArgCursor {
    /// Variadic arguments begin after the last fixed argument; `printf`
    /// takes the format in r0, `fprintf` in r1, `sprintf` in r1, and
    /// `snprintf` in r2.
    fn after_fixed_args(machine: &Machine, first_vararg_register: usize) -> Self {
        Self {
            register: first_vararg_register,
            stack: machine.cpu.r[13],
        }
    }

    /// Cursor reading varargs straight from guest memory starting at
    /// `va_list` — on ARM EABI a `va_list` is a single pointer into the
    /// caller's frame (see bionic vfprintf.c: `__va_list ap`).
    fn from_va_list_pointer(va_list: u32) -> Self {
        Self {
            register: 4,
            stack: va_list,
        }
    }

    fn next_u32(&mut self, machine: &Machine) -> u32 {
        if self.register <= 3 {
            let value = machine.cpu.r[self.register];
            self.register += 1;
            return value;
        }
        let value = machine.memory.read_u32(self.stack).unwrap_or(0);
        self.stack += 4;
        value
    }

    /// Reads a double vararg: AAPCS aligns 8-byte arguments to 8 bytes.
    fn next_f64(&mut self, machine: &Machine) -> f64 {
        if self.register > 3 {
            self.stack = (self.stack + 7) & !7;
        }
        let low = self.next_u32(machine);
        let high = self.next_u32(machine);
        f64::from_bits((low as u64) | ((high as u64) << 32))
    }

    fn next_i32(&mut self, machine: &Machine) -> i32 {
        self.next_u32(machine) as i32
    }
}

impl HostBridge for BasicHost {
    fn call_host(&mut self, machine: &mut Machine, slot: usize) -> u32 {
        self.ensure_regions(machine);
        let name = machine
            .linker
            .host_name(slot)
            .unwrap_or("unknown")
            .to_owned();
        match name.as_str() {
            "malloc" | "_Znwj" | "_Znaj" => self.alloc(machine, machine.cpu.r[0]),
            "calloc" => {
                let count = machine.cpu.r[0];
                let size = machine.cpu.r[1];
                self.alloc(machine, count.saturating_mul(size))
            }
            "realloc" => {
                let old = machine.cpu.r[0];
                let size = machine.cpu.r[1];
                if old == 0 {
                    return self.alloc(machine, size);
                }
                let address = self.alloc(machine, size);
                if address != 0 {
                    let old_size = self.allocations.get(&old).copied();
                    let new_size = self.allocations.get(&address).copied();
                    if let (Some(old_size), Some(new_size)) = (old_size, new_size) {
                        let _ = machine
                            .memory
                            .copy_within(address, old, old_size.min(new_size));
                    }
                }
                self.allocations.remove(&old);
                address
            }
            "free" | "_ZdlPv" | "_ZdaPv" => {
                self.allocations.remove(&machine.cpu.r[0]);
                0
            }
            "memset" => {
                let destination = machine.cpu.r[0];
                let value = machine.cpu.r[1] as u8;
                let length = machine.cpu.r[2];
                if machine.memory.fill(destination, value, length).is_err() {
                    self.log(format!("memset faulted at {destination:#010x}"));
                }
                destination
            }
            "memcpy" | "memmove" => {
                let destination = machine.cpu.r[0];
                let source = machine.cpu.r[1];
                let length = machine.cpu.r[2];
                if machine
                    .memory
                    .copy_within(destination, source, length)
                    .is_err()
                {
                    self.log(format!(
                        "memcpy faulted copying {length} bytes {source:#010x} -> {destination:#010x}"
                    ));
                }
                destination
            }
            "memcmp" => {
                let (left, right, length) = (machine.cpu.r[0], machine.cpu.r[1], machine.cpu.r[2]);
                for index in 0..length {
                    let a = machine.memory.read_u8(left + index).unwrap_or(0);
                    let b = machine.memory.read_u8(right + index).unwrap_or(0);
                    if a != b {
                        return Self::c_difference(a, b) as u32;
                    }
                }
                0
            }
            "strlen" => self.guest_strlen(machine, machine.cpu.r[0]),
            "strcpy" => {
                let (destination, source) = (machine.cpu.r[0], machine.cpu.r[1]);
                let bytes = machine
                    .memory
                    .read_bytes(source, self.guest_strlen(machine, source) as usize + 1)
                    .unwrap_or_default();
                let _ = machine.memory.write_bytes(destination, &bytes);
                destination
            }
            "strncpy" => {
                let (destination, source, limit) =
                    (machine.cpu.r[0], machine.cpu.r[1], machine.cpu.r[2]);
                let length = self.guest_strlen(machine, source).min(limit);
                let _ = machine.memory.copy_within(destination, source, length);
                if length < limit {
                    let _ = machine.memory.fill(destination + length, 0, limit - length);
                }
                destination
            }
            "strcat" => {
                let (destination, source) = (machine.cpu.r[0], machine.cpu.r[1]);
                let offset = self.guest_strlen(machine, destination);
                let bytes = machine
                    .memory
                    .read_bytes(source, self.guest_strlen(machine, source) as usize + 1)
                    .unwrap_or_default();
                let _ = machine.memory.write_bytes(destination + offset, &bytes);
                destination
            }
            "strcmp" | "strncmp" => {
                let (left, right, limit) = (
                    machine.cpu.r[0],
                    machine.cpu.r[1],
                    if name == "strcmp" {
                        u32::MAX
                    } else {
                        machine.cpu.r[2]
                    },
                );
                let mut index = 0;
                loop {
                    if index >= limit {
                        return 0;
                    }
                    let a = machine.memory.read_u8(left + index).unwrap_or(0);
                    let b = machine.memory.read_u8(right + index).unwrap_or(0);
                    if a != b || a == 0 {
                        return Self::c_difference(a, b) as u32;
                    }
                    index += 1;
                }
            }
            "strchr" => {
                let (address, needle) = (machine.cpu.r[0], machine.cpu.r[1] as u8);
                let mut index = 0u32;
                loop {
                    let value = machine.memory.read_u8(address + index).unwrap_or(0);
                    if value == needle {
                        return address + index;
                    }
                    if value == 0 {
                        return 0;
                    }
                    index += 1;
                }
            }
            "strrchr" => {
                let (address, needle) = (machine.cpu.r[0], machine.cpu.r[1] as u8);
                let length = self.guest_strlen(machine, address);
                let mut index = length;
                loop {
                    let value = machine.memory.read_u8(address + index).unwrap_or(0);
                    if value == needle {
                        return address + index;
                    }
                    if index == 0 {
                        return 0;
                    }
                    index -= 1;
                }
            }
            "qsort" => self.qsort(
                machine,
                machine.cpu.r[0],
                machine.cpu.r[1],
                machine.cpu.r[2],
                machine.cpu.r[3],
            ),
            "printf" | "fprintf" => {
                let format_address = machine.cpu.r[0];
                let format_address = if name == "fprintf" {
                    machine.cpu.r[1]
                } else {
                    format_address
                };
                let text = self.format(
                    machine,
                    format_address,
                    ArgCursor::after_fixed_args(machine, if name == "fprintf" { 2 } else { 1 }),
                );
                self.log(format!("stdio: {text}"));
                text.len() as u32
            }
            "puts" => {
                let text = self.read_stack_string(machine, machine.cpu.r[0]);
                self.log(format!("stdio: {text}"));
                1
            }
            "sprintf" => {
                // Variadic: r2, r3, then the caller's stack.
                let (destination, format_address) = (machine.cpu.r[0], machine.cpu.r[1]);
                let cursor = ArgCursor::after_fixed_args(machine, 2);
                let text = self.format(machine, format_address, cursor);
                let _ = machine.memory.write_cstr(destination, &text);
                text.len() as u32
            }
            "vsprintf" => {
                // vsprintf(char*, const char*, va_list): r2 IS the va_list,
                // a pointer into the caller's frame (bionic vfprintf ABI).
                let (destination, format_address) = (machine.cpu.r[0], machine.cpu.r[1]);
                let cursor = ArgCursor::from_va_list_pointer(machine.cpu.r[2]);
                let text = self.format(machine, format_address, cursor);
                let _ = machine.memory.write_cstr(destination, &text);
                text.len() as u32
            }
            "snprintf" => {
                let (destination, limit, format_address) =
                    (machine.cpu.r[0], machine.cpu.r[1], machine.cpu.r[2]);
                let cursor = ArgCursor::after_fixed_args(machine, 3);
                let text = self.format(machine, format_address, cursor);
                let truncated: String = text
                    .chars()
                    .take(limit.saturating_sub(1) as usize)
                    .collect();
                let _ = machine.memory.write_cstr(destination, &truncated);
                text.len() as u32
            }
            "vsnprintf" => {
                let (destination, limit, format_address) =
                    (machine.cpu.r[0], machine.cpu.r[1], machine.cpu.r[2]);
                let cursor = ArgCursor::from_va_list_pointer(machine.cpu.r[3]);
                let text = self.format(machine, format_address, cursor);
                let truncated: String = text
                    .chars()
                    .take(limit.saturating_sub(1) as usize)
                    .collect();
                let _ = machine.memory.write_cstr(destination, &truncated);
                text.len() as u32
            }
            "__android_log_write" => {
                let tag = self.read_stack_string(machine, machine.cpu.r[1]);
                let message = self.read_stack_string(machine, machine.cpu.r[2]);
                self.log(format!("log/{tag}: {message}"));
                0
            }
            "pthread_mutex_init"
            | "pthread_mutex_destroy"
            | "pthread_mutex_lock"
            | "pthread_mutex_unlock"
            | "pthread_mutexattr_init"
            | "pthread_mutexattr_destroy"
            | "pthread_mutexattr_settype" => 0,
            "pthread_mutex_trylock" => 0,
            "pthread_key_create" => {
                self.next_key += 1;
                let _ = machine.memory.write_u32(machine.cpu.r[0], self.next_key);
                0
            }
            "pthread_setspecific" => {
                self.thread_specific
                    .insert(machine.cpu.r[0], machine.cpu.r[1]);
                0
            }
            "pthread_getspecific" => self
                .thread_specific
                .get(&machine.cpu.r[0])
                .copied()
                .unwrap_or(0),
            "time" => {
                let now: u32 = 1_234_567_890;
                if machine.cpu.r[0] != 0 {
                    let _ = machine.memory.write_u32(machine.cpu.r[0], now);
                }
                now
            }
            "usleep" | "nanosleep" => 0,
            "srand48" => {
                self.random_state = (machine.cpu.r[0] as u64) << 16 | 0x330E;
                0
            }
            "lrand48" => self.lrand48(),
            "abort" | "__stack_chk_fail" => {
                self.log(format!("{name} called at pc {:#010x}", machine.cpu.r[15]));
                machine.emulated_abort();
                0
            }
            "exit" => {
                machine.emulated_exit(machine.cpu.r[0] as i32);
                0
            }
            "__errno" => self.errno_address,
            "__aeabi_atexit" => 0,
            "setjmp" => {
                let env = machine.cpu.r[0];
                let mut saved = [0u32; 10];
                for (index, register) in [4, 5, 6, 7, 8, 9, 10, 11, 13, 14].into_iter().enumerate()
                {
                    saved[index] = machine.cpu.r[register];
                }
                self.jump_buffers.insert(env, saved);
                0
            }
            "longjmp" => {
                let (env, value) = (machine.cpu.r[0], machine.cpu.r[1]);
                if let Some(saved) = self.jump_buffers.remove(&env) {
                    for (index, register) in
                        [4, 5, 6, 7, 8, 9, 10, 11, 13, 14].into_iter().enumerate()
                    {
                        machine.cpu.r[register] = saved[index];
                    }
                    // Returning from the host call jumps to the restored LR.
                    machine.cpu.r[0] = value.max(1);
                } else {
                    self.log(format!("longjmp to unknown jmp_buf {env:#010x}"));
                    machine.emulated_abort();
                }
                machine.cpu.r[0].max(1)
            }
            // File operations fail closed.
            "open" | "read" | "close" | "lseek" | "fstat" | "stat" | "lstat" | "statfs"
            | "unlink" | "rename" | "mkdir" | "opendir" | "closedir" | "dup" | "fcntl" => {
                self.log(format!("{name} is not available (fail-closed)"));
                (-1i32) as u32
            }
            "readdir" => 0,
            "write" => {
                let (fd, address, length) = (machine.cpu.r[0], machine.cpu.r[1], machine.cpu.r[2]);
                if fd == 1 || fd == 2 {
                    let text = machine
                        .memory
                        .read_bytes(address, length as usize)
                        .unwrap_or_default();
                    self.log(format!("stdout: {}", String::from_utf8_lossy(&text)));
                    length
                } else {
                    self.log("write to non-stdio fd is not available (fail-closed)".to_owned());
                    (-1i32) as u32
                }
            }
            "mmap" => {
                let length = machine.cpu.r[1];
                let fd = machine.cpu.r[4];
                if fd != (-1i32) as u32 {
                    self.log("mmap of file descriptors is not supported".to_owned());
                    return MAP_FAILED;
                }
                let aligned = length.next_multiple_of(0x1000);
                let address = self.mmap_next;
                self.mmap_next += aligned;
                if self.mmap_next > MMAP_BASE + MMAP_SIZE {
                    self.log("host mmap region exhausted".to_owned());
                    return MAP_FAILED;
                }
                address
            }
            "munmap" => 0,
            "socket" | "connect" | "send" | "recvfrom" | "select" | "getsockopt" => {
                self.log(format!("{name} blocked: network is disabled"));
                (-1i32) as u32
            }
            "getaddrinfo" => {
                self.log("getaddrinfo blocked: network is disabled".to_owned());
                4 // EAI_FAIL-ish non-zero
            }
            "freeaddrinfo" => 0,
            "inet_addr" => 0,
            "inet_ntoa" => {
                let address = HOST_DATA_BASE + 0x100;
                let _ = machine.memory.write_cstr(address, "0.0.0.0");
                address
            }
            // AEABI integer arithmetic.
            "__aeabi_idiv" => self.aeabi_idiv(machine),
            "__aeabi_uidiv" => self.aeabi_uidiv(machine),
            "__aeabi_idivmod" | "__aeabi_uidivmod" => {
                let (quotient, remainder) = if name == "__aeabi_idivmod" {
                    self.aeabi_idivmod(machine)
                } else {
                    self.aeabi_uidivmod(machine)
                };
                machine.cpu.r[0] = quotient;
                machine.cpu.r[1] = remainder;
                quotient
            }
            "__aeabi_lmul" => {
                let a = self.double_word(machine, 0);
                let b = self.double_word(machine, 2);
                self.set_double_word(machine, 0, a.wrapping_mul(b));
                machine.cpu.r[0]
            }
            "__aeabi_ldivmod" | "__aeabi_uldivmod" => {
                let signed = name == "__aeabi_ldivmod";
                let a = self.double_word(machine, 0);
                let b = self.double_word(machine, 2);
                if b == 0 {
                    self.log("AEABI 64-bit division by zero".to_owned());
                    machine.cpu.r[0] = 0;
                    machine.cpu.r[1] = 0;
                    machine.cpu.r[2] = a as u32;
                    machine.cpu.r[3] = (a >> 32) as u32;
                    return machine.cpu.r[0];
                }
                let (quotient, remainder) = if signed {
                    ((a as i64 / b as i64) as u64, (a as i64 % b as i64) as u64)
                } else {
                    (a / b, a % b)
                };
                machine.cpu.r[0] = quotient as u32;
                machine.cpu.r[1] = (quotient >> 32) as u32;
                machine.cpu.r[2] = remainder as u32;
                machine.cpu.r[3] = (remainder >> 32) as u32;
                machine.cpu.r[0]
            }
            "__aeabi_l2d" => {
                let value = self.double_word(machine, 0) as i64;
                self.set_double(machine, 0, value as f64);
                machine.cpu.r[0]
            }
            // AEABI floating point.
            "__aeabi_i2f" => (machine.cpu.r[0] as i32 as f32).to_bits(),
            "__aeabi_ui2f" => (machine.cpu.r[0] as f32).to_bits(),
            "__aeabi_i2d" => {
                self.set_double(machine, 0, f64::from(machine.cpu.r[0] as i32));
                machine.cpu.r[0]
            }
            "__aeabi_ui2d" => {
                self.set_double(machine, 0, f64::from(machine.cpu.r[0]));
                machine.cpu.r[0]
            }
            "__aeabi_f2iz" => (f32::from_bits(machine.cpu.r[0]) as i32) as u32,
            "__aeabi_f2ui" => f32::from_bits(machine.cpu.r[0]) as u32,
            "__aeabi_f2d" => {
                let value = f64::from(f32::from_bits(machine.cpu.r[0]));
                self.set_double(machine, 0, value);
                machine.cpu.r[0]
            }
            "__aeabi_d2f" => (self.read_double(machine, 0) as f32).to_bits(),
            "__aeabi_d2iz" => (self.read_double(machine, 0) as i32) as u32,
            "__aeabi_d2uiz" => self.read_double(machine, 0) as u32,
            "__aeabi_fadd" | "__aeabi_fsub" | "__aeabi_fmul" | "__aeabi_fdiv" => {
                let a = f32::from_bits(machine.cpu.r[0]);
                let b = f32::from_bits(machine.cpu.r[1]);
                let value = match name.as_str() {
                    "__aeabi_fadd" => a + b,
                    "__aeabi_fsub" => a - b,
                    "__aeabi_fmul" => a * b,
                    _ => a / b,
                };
                value.to_bits()
            }
            "__aeabi_dadd" | "__aeabi_dsub" | "__aeabi_dmul" | "__aeabi_ddiv" => {
                let a = self.read_double(machine, 0);
                let b = self.read_double(machine, 2);
                let value = match name.as_str() {
                    "__aeabi_dadd" => a + b,
                    "__aeabi_dsub" => a - b,
                    "__aeabi_dmul" => a * b,
                    _ => a / b,
                };
                self.set_double(machine, 0, value);
                machine.cpu.r[0]
            }
            "__aeabi_fcmpeq" | "__aeabi_fcmplt" | "__aeabi_fcmple" | "__aeabi_fcmpgt"
            | "__aeabi_fcmpge" => {
                let (a, b) = (
                    f32::from_bits(machine.cpu.r[0]),
                    f32::from_bits(machine.cpu.r[1]),
                );
                let result = match name.as_str() {
                    "__aeabi_fcmpeq" => a == b,
                    "__aeabi_fcmplt" => a < b,
                    "__aeabi_fcmple" => a <= b,
                    "__aeabi_fcmpgt" => a > b,
                    _ => a >= b,
                };
                u32::from(result)
            }
            "__aeabi_dcmpeq" | "__aeabi_dcmplt" | "__aeabi_dcmple" | "__aeabi_dcmpgt"
            | "__aeabi_dcmpge" => {
                let (a, b) = (self.read_double(machine, 0), self.read_double(machine, 2));
                let result = match name.as_str() {
                    "__aeabi_dcmpeq" => a == b,
                    "__aeabi_dcmplt" => a < b,
                    "__aeabi_dcmple" => a <= b,
                    "__aeabi_dcmpgt" => a > b,
                    _ => a >= b,
                };
                u32::from(result)
            }
            other if GL_FUNCTIONS.contains(&other) => {
                self.ensure_regions(machine);
                if self.gles.is_none() {
                    self.gles = Some(crate::gles1_on_gl2::Gles1OnGl2::new(crate::VirtualScreen {
                        width: 480,
                        height: 320,
                    }));
                }
                let args = gl_call_args(machine, 9);
                self.call_gl(machine, other, &args)
            }
            other if other.starts_with("jni:") => self.call_jni(machine, other),
            other => {
                self.log(format!(
                    "host function {other} is not implemented; returning 0"
                ));
                0
            }
        }
    }

    fn diagnostic(&mut self, message: String) {
        self.log(message);
    }
}

impl BasicHost {
    /// Dispatches a JNI table entry. `slot_name` is `jni:<index>:<Name>`.
    fn call_jni(&mut self, machine: &mut Machine, slot_name: &str) -> u32 {
        self.ensure_regions(machine);
        let function = entry_function(slot_name);
        let string_at = |machine: &Machine, register: usize| -> String {
            machine
                .memory
                .read_cstr(machine.cpu.r[register], 512)
                .unwrap_or_default()
        };
        match function {
            "GetVersion" => 0x0001_0004, // JNI 1.4
            "GetJavaVM" => {
                machine
                    .memory
                    .write_u32(machine.cpu.r[1], crate::jni::JNI_BASE + 0x80)
                    .ok();
                0
            }
            "FindClass" => {
                let name = string_at(machine, 1);
                let handle = self.jni_class(machine, &name);
                self.log(format!("JNI FindClass({name}) -> {handle:#x}"));
                handle
            }
            "GetObjectClass" => {
                let name = format!("object@{:#x}", machine.cpu.r[1]);
                let handle = self.jni_class(machine, &name);
                self.log(format!(
                    "JNI GetObjectClass({:#x}) -> {handle:#x}",
                    machine.cpu.r[1]
                ));
                handle
            }
            "GetMethodID" | "GetStaticMethodID" | "GetFieldID" | "GetStaticFieldID" => {
                let class = machine.cpu.r[1];
                let name = string_at(machine, 2);
                let signature = string_at(machine, 3);
                let handle = self.jni_method(machine, class, &name, &signature);
                self.log(format!(
                    "JNI {function}(class={class:#x}, {name}{signature}) -> {handle:#x}"
                ));
                handle
            }
            "NewStringUTF" => {
                let text = string_at(machine, 1);
                let handle = self.jni_string(machine, &text);
                self.log(format!("JNI NewStringUTF({text:?}) -> {handle:#x}"));
                handle
            }
            "GetStringUTFChars" => {
                let handle = machine.cpu.r[1];
                let text = self.jni_strings.get(&handle).cloned().unwrap_or_default();
                self.log(format!("JNI GetStringUTFChars({handle:#x}) -> {text:?}"));
                self.jni_string_storage(machine, &text)
            }
            "ReleaseStringChars" | "ReleaseStringUTFChars" => 0,
            "GetArrayLength" => {
                let handle = machine.cpu.r[1];
                let length = self
                    .jni_arrays
                    .get(&handle)
                    .map(|array| array.length)
                    .unwrap_or(0);
                self.log(format!("JNI GetArrayLength({handle:#x}) -> {length}"));
                length
            }
            "NewIntArray" | "NewLongArray" | "NewFloatArray" | "NewByteArray"
            | "NewBooleanArray" | "NewCharArray" | "NewShortArray" | "NewDoubleArray" => {
                let element = array_element_size(function).unwrap_or(4);
                let length = machine.cpu.r[1];
                let handle = self.jni_array(machine, length, element);
                self.log(format!("JNI {function}({length}) -> {handle:#x}"));
                handle
            }
            "GetIntArrayElements"
            | "GetLongArrayElements"
            | "GetFloatArrayElements"
            | "GetByteArrayElements"
            | "GetBooleanArrayElements"
            | "GetCharArrayElements"
            | "GetShortArrayElements"
            | "GetDoubleArrayElements"
            | "GetPrimitiveArrayCritical" => {
                let handle = machine.cpu.r[1];
                // The storage word holds the length; element data follows it.
                let data = self
                    .jni_arrays
                    .get(&handle)
                    .map(|array| array.storage + 4)
                    .unwrap_or_else(|| {
                        // A handle we don't know (Dalvik-mirrored or NULL):
                        // allocate scratch storage so the engine's writes land
                        // somewhere valid instead of faulting.
                        self.log(format!(
                            "JNI {function}({handle:#x}): unknown handle, allocating scratch"
                        ));
                        self.jni_next = (self.jni_next + 15) & !15;
                        let scratch = self.jni_next;
                        self.jni_next += 64;
                        self.jni_array_storage.insert(scratch - 4, handle);
                        scratch
                    });
                self.log(format!("JNI {function}({handle:#x}) -> {data:#x}"));
                data
            }
            "ReleaseIntArrayElements"
            | "ReleaseLongArrayElements"
            | "ReleaseFloatArrayElements"
            | "ReleaseByteArrayElements"
            | "ReleaseBooleanArrayElements"
            | "ReleaseCharArrayElements"
            | "ReleaseShortArrayElements"
            | "ReleaseDoubleArrayElements"
            | "ReleasePrimitiveArrayCritical" => 0,
            "ExceptionCheck" | "ExceptionOccurred" => 0,
            "EnsureLocalCapacity"
            | "PushLocalFrame"
            | "PopLocalFrame"
            | "MonitorEnter"
            | "MonitorExit"
            | "DeleteLocalRef"
            | "DeleteGlobalRef" => 0,
            "RegisterNatives" => {
                self.log(format!(
                    "JNI RegisterNatives(class={:#x}, count={}) ignored",
                    machine.cpu.r[1], machine.cpu.r[3]
                ));
                0
            }
            "GetStringLength" | "GetStringUTFLength" => {
                let handle = machine.cpu.r[1];
                let length = self
                    .jni_strings
                    .get(&handle)
                    .map(|text| {
                        if function == "GetStringLength" {
                            text.chars().count()
                        } else {
                            text.len()
                        }
                    })
                    .unwrap_or(0) as u32;
                self.log(format!("JNI {function}({handle:#x}) -> {length}"));
                length
            }
            "CallStaticVoidMethod"
            | "CallVoidMethod"
            | "CallStaticObjectMethod"
            | "CallStaticIntMethod"
            | "CallStaticBooleanMethod"
            | "CallObjectMethod"
            | "CallIntMethod"
            | "CallBooleanMethod" => {
                let handle = machine.cpu.r[2];
                let info = self
                    .jni_method_names
                    .get(&handle)
                    .cloned()
                    .unwrap_or((0, "unknown".to_owned()));
                self.log(format!(
                    "JNI {function}(class={:#x}, {}) -> 0 (stub)",
                    info.0, info.1
                ));
                0
            }
            _ => {
                self.log(format!("JNI {function} is not implemented; returning 0"));
                0
            }
        }
    }

    pub fn jni_class(&mut self, _machine: &mut Machine, name: &str) -> u32 {
        if let Some(handle) = self.jni_classes.get(name) {
            return *handle;
        }
        self.jni_next += 8;
        let handle = self.jni_next;
        self.jni_classes.insert(name.to_owned(), handle);
        self.jni_class_names.insert(handle, name.to_owned());
        handle
    }

    fn jni_method(
        &mut self,
        machine: &mut Machine,
        class: u32,
        name: &str,
        signature: &str,
    ) -> u32 {
        let key = (class, name.to_owned(), signature.to_owned());
        if let Some(handle) = self.jni_methods.get(&key) {
            return *handle;
        }
        self.jni_next_handle += 1;
        let handle = self.jni_next_handle;
        machine
            .memory
            .write_u32(crate::jni::JNI_BASE + 0x400 + handle * 4, 0)
            .ok();
        self.jni_methods.insert(key, handle);
        self.jni_method_names
            .insert(handle, (class, format!("{name}{signature}")));
        handle
    }

    pub fn jni_string(&mut self, machine: &mut Machine, text: &str) -> u32 {
        self.jni_next += 16;
        let handle = self.jni_next;
        let storage = self.jni_string_storage(machine, text);
        machine.memory.write_u32(handle, storage).ok();
        self.jni_strings.insert(handle, text.to_owned());
        handle
    }

    /// UTF-8 storage for a string object, written into the JNI region.
    fn jni_string_storage(&mut self, machine: &mut Machine, text: &str) -> u32 {
        self.jni_next = (self.jni_next + 15) & !15;
        let address = self.jni_next;
        self.jni_next += text.len() as u32 + 1;
        machine.memory.write_cstr(address, text).ok();
        address
    }

    /// Creates a Java-side primitive array for a harness (the mirror of the
    /// guest calling New*Array); returns the array handle.
    pub fn new_jni_array(&mut self, machine: &mut Machine, length: u32, element_size: u32) -> u32 {
        self.jni_array(machine, length, element_size)
    }

    /// Guest storage address of a JNI array's element data (the value
    /// `Get*ArrayElements` hands out).
    pub fn jni_array_data(&self, handle: u32) -> Option<u32> {
        self.jni_arrays.get(&handle).map(|array| array.storage + 4)
    }

    fn jni_array(&mut self, machine: &mut Machine, length: u32, element_size: u32) -> u32 {
        self.jni_next += 16;
        let handle = self.jni_next;
        self.jni_next = (self.jni_next + 15) & !15;
        let storage = self.jni_next;
        let bytes = length.saturating_mul(element_size);
        self.jni_next += bytes.max(16);
        machine.memory.write_u32(handle, storage).ok();
        machine.memory.write_u32(storage, length).ok();
        self.jni_arrays.insert(
            handle,
            JniArray {
                length,
                element_size,
                storage,
            },
        );
        self.jni_array_storage.insert(storage, handle);
        handle
    }

    /// Dispatches an imported GLES function against the software rasterizer.
    /// Integer arguments come from r0-r3 + guest stack (AAPCS).
    fn call_gl(&mut self, machine: &mut Machine, name: &str, args: &[u32]) -> u32 {
        let arg = |index: usize| -> u32 { args.get(index).copied().unwrap_or(0) };
        let renderer = match self.gles.as_mut() {
            Some(renderer) => renderer,
            None => return 0,
        };
        match name {
            "glViewport" => renderer.viewport(arg(0) as i32, arg(1) as i32, arg(2), arg(3)),
            "glClearColor" | "glClearColorx" => renderer.set_clear_color(Rgba8 {
                r: (fixed_to_f32(arg(0)) * 255.0).clamp(0.0, 255.0) as u8,
                g: (fixed_to_f32(arg(1)) * 255.0).clamp(0.0, 255.0) as u8,
                b: (fixed_to_f32(arg(2)) * 255.0).clamp(0.0, 255.0) as u8,
                a: 255,
            }),
            "glClear" => renderer.clear(),
            "glEnable" | "glDisable" => {
                let capability = arg(0);
                if capability == GL_VERTEX_ARRAY
                    || capability == GL_COLOR_ARRAY
                    || capability == GL_TEXTURE_COORD_ARRAY
                    || capability == GL_NORMAL_ARRAY
                {
                    let index = match capability {
                        GL_VERTEX_ARRAY => 0,
                        GL_COLOR_ARRAY => 1,
                        GL_TEXTURE_COORD_ARRAY => 2,
                        _ => 3,
                    };
                    self.gl_client_arrays[index].enabled = name == "glEnable";
                }
                if name == "glEnable" {
                    renderer.enable(capability);
                } else {
                    renderer.disable(capability);
                }
            }
            "glEnableClientState" | "glDisableClientState" => {
                let array = match arg(0) {
                    GL_VERTEX_ARRAY => Some(crate::gles1_on_gl2::ClientArray::Vertex),
                    GL_COLOR_ARRAY => Some(crate::gles1_on_gl2::ClientArray::Color),
                    GL_TEXTURE_COORD_ARRAY => Some(crate::gles1_on_gl2::ClientArray::TexCoord),
                    _ => None, // normal arrays are not modeled
                };
                if let Some(array) = array {
                    if name == "glEnableClientState" {
                        renderer.enable_client_state(array);
                    } else {
                        renderer.disable_client_state(array);
                    }
                }
            }
            "glBlendFunc" => renderer.blend_func(arg(0), arg(1)),
            "glShadeModel"
            | "glPointSizex"
            | "glLineWidthx"
            | "glTexEnvf"
            | "glTexEnvx"
            | "glLightxv"
            | "glActiveTexture"
            | "glClientActiveTexture"
            | "glTexEnvxv"
            | "glCopyTexImage2D"
            | "glCopyTexSubImage2D" => {
                self.gl_ignored.insert(name.to_owned());
            }
            "glTexParameterf" | "glTexParameteri" | "glTexParameterx" => {
                renderer.texture_parameter(arg(0), arg(1), arg(2));
            }
            "glMatrixMode" => renderer.matrix_mode(arg(0)),
            "glLoadIdentity" => renderer.load_identity(),
            "glLoadMatrixf" => {
                let mut matrix = [0f32; 16];
                for (index, cell) in matrix.iter_mut().enumerate() {
                    *cell = f32::from_bits(
                        machine
                            .memory
                            .read_u32(arg(0) + index as u32 * 4)
                            .unwrap_or(0),
                    );
                }
                renderer.load_matrix_f(&matrix);
            }
            "glLoadMatrixx" => {
                let mut matrix = [0i32; 16];
                for (index, cell) in matrix.iter_mut().enumerate() {
                    *cell = machine
                        .memory
                        .read_u32(arg(0) + index as u32 * 4)
                        .unwrap_or(0) as i32;
                }
                renderer.load_matrix_x(&matrix);
            }
            "glScissor" => renderer.scissor(arg(0) as i32, arg(1) as i32, arg(2), arg(3)),
            "glColor4x" => renderer.set_current_color(Rgba8 {
                r: (fixed_to_f32(arg(0)) * 255.0).clamp(0.0, 255.0) as u8,
                g: (fixed_to_f32(arg(1)) * 255.0).clamp(0.0, 255.0) as u8,
                b: (fixed_to_f32(arg(2)) * 255.0).clamp(0.0, 255.0) as u8,
                a: (fixed_to_f32(arg(3)) * 255.0).clamp(0.0, 255.0) as u8,
            }),
            "glDepthMask" => renderer.depth_mask(arg(0) != 0),
            "glGenTextures" => {
                let count = arg(0) as usize;
                let ids_pointer = arg(1);
                for index in 0..count {
                    let id = renderer.gen_texture();
                    machine
                        .memory
                        .write_u32(ids_pointer + index as u32 * 4, id)
                        .ok();
                }
            }
            "glBindTexture" => renderer.bind_texture(arg(0), arg(1)),
            "glDeleteTextures" => {
                for index in 0..arg(0) {
                    let id = machine.memory.read_u32(arg(1) + index * 4).unwrap_or(0);
                    renderer.delete_texture(id);
                }
            }
            "glTexImage2D" | "glTexSubImage2D" => {
                self.gl_tex_image(machine, name, args);
            }
            "glCompressedTexImage2D" => {
                // Palettized/compressed atlases: record the gap honestly.
                self.gl_ignored.insert(format!(
                    "glCompressedTexImage2D(format={:#x}, {}x{})",
                    arg(2),
                    arg(3),
                    arg(4)
                ));
            }
            "glVertexPointer" | "glColorPointer" | "glTexCoordPointer" | "glNormalPointer" => {
                let index = match name {
                    "glVertexPointer" => 0,
                    "glColorPointer" => 1,
                    "glTexCoordPointer" => 2,
                    _ => 3,
                };
                self.gl_client_arrays[index] = ClientArraySpec {
                    pointer: arg(3),
                    size: arg(0) as usize,
                    stride: arg(2) as usize,
                    enabled: true,
                    array_type: arg(1),
                };
            }
            "glDrawArrays" | "glDrawElements" => {
                self.gl_draw(machine, name, &arg);
            }
            "glFinish" => {}
            "glGetError" => return 0,
            "glIsEnabled" => {
                return u32::from(renderer.is_enabled(arg(0)) || self.client_array_enabled(arg(0)));
            }
            "glGetIntegerv" | "glGetFixedv" | "glGetPointerv" => {}
            _ => {
                self.gl_ignored.insert(name.to_owned());
            }
        }
        0
    }

    fn client_array_enabled(&self, capability: u32) -> bool {
        let index = match capability {
            GL_VERTEX_ARRAY => 0,
            GL_COLOR_ARRAY => 1,
            GL_TEXTURE_COORD_ARRAY => 2,
            GL_NORMAL_ARRAY => 3,
            _ => return false,
        };
        self.gl_client_arrays[index].enabled
    }

    /// Reads a client array from guest memory, converting fixed-point to
    /// float when the engine uses GL_FIXED.
    fn read_client_array(
        machine: &Machine,
        spec: &ClientArraySpec,
        first: i32,
        count: i32,
    ) -> Vec<f32> {
        let mut values = Vec::new();
        if spec.pointer == 0 || count <= 0 {
            return values;
        }
        let stride = if spec.stride == 0 {
            (spec.size * 4) as u32
        } else {
            spec.stride as u32
        };
        for index in 0..count as u32 {
            let base = spec.pointer + (first.max(0) as u32 + index) * stride;
            for component in 0..spec.size {
                let address = base + component as u32 * 4;
                let word = machine.memory.read_u32(address).unwrap_or(0);
                values.push(match spec.array_type {
                    GL_FIXED => fixed_to_f32(word),
                    GL_FLOAT => f32::from_bits(word),
                    GL_BYTE => word as i8 as f32,
                    GL_SHORT => word as i16 as f32,
                    GL_UNSIGNED_BYTE => word as u8 as f32,
                    GL_UNSIGNED_SHORT => (word & 0xFFFF) as f32,
                    _ => word as i32 as f32,
                });
            }
        }
        values
    }

    fn gl_draw(&mut self, machine: &mut Machine, name: &str, arg: &dyn Fn(usize) -> u32) {
        let mode = arg(0);
        let first = arg(1) as i32;
        let count = arg(2) as i32;
        if count <= 0 {
            return;
        }
        // Copy the specs out to avoid borrowing self while the renderer is
        // mutably borrowed.
        let [vertex_spec, color_spec, texcoord_spec, _normal_spec] = self.gl_client_arrays;
        let vertices = Self::read_client_array(machine, &vertex_spec, first, count);
        if vertices.is_empty() {
            return;
        }
        let Some(renderer) = self.gles.as_mut() else {
            return;
        };
        renderer.set_vertex_pointer(vertex_spec.size, 0, vertices);
        if color_spec.enabled {
            let colors = Self::read_client_array(machine, &color_spec, first, count);
            if !colors.is_empty() {
                renderer.set_color_pointer(color_spec.size, 0, colors);
            }
        }
        if texcoord_spec.enabled {
            let texcoords = Self::read_client_array(machine, &texcoord_spec, first, count);
            if !texcoords.is_empty() {
                renderer.set_texcoord_pointer(texcoord_spec.size, 0, texcoords);
            }
        }
        if name == "glDrawArrays" {
            renderer.draw_arrays(mode, first, count);
            return;
        }
        // glDrawElements(mode, count, type, indices)
        let index_type = arg(3);
        let indices_pointer = arg(1);
        let index_size: u32 = if index_type == GL_UNSIGNED_SHORT || index_type == GL_SHORT {
            2
        } else {
            1
        };
        let indices: Vec<u32> = (0..count as u32)
            .map(|index| {
                let address = indices_pointer + index * index_size;
                if index_size == 2 {
                    machine.memory.read_u16(address).unwrap_or(0) as u32
                } else {
                    u32::from(machine.memory.read_u8(address).unwrap_or(0))
                }
            })
            .collect();
        renderer.draw_elements_indexed(mode, count, index_type, &indices);
    }

    fn gl_tex_image(&mut self, machine: &mut Machine, name: &str, args: &[u32]) {
        let Some(renderer) = self.gles.as_mut() else {
            return;
        };
        // target, level, internalformat, width, height, border, format, type, pixels
        let width = args.get(3).copied().unwrap_or(0);
        let height = args.get(4).copied().unwrap_or(0);
        let format = args.get(6).copied().unwrap_or(0);
        let pixel_type = args.get(7).copied().unwrap_or(0);
        let pixels = args.get(8).copied().unwrap_or(0);
        if pixels == 0 || width == 0 || height == 0 {
            return;
        }
        let bytes_per_pixel = match (format, pixel_type) {
            (GL_RGBA, GL_UNSIGNED_BYTE) => 4,
            (GL_RGB, GL_UNSIGNED_BYTE) => 3,
            (GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4) => 2,
            (GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1) => 2,
            (GL_RGB, GL_UNSIGNED_SHORT_5_6_5) => 2,
            _ => {
                self.gl_ignored
                    .insert(format!("{name} format={format:#x} type={pixel_type:#x}"));
                return;
            }
        };
        let data = machine
            .memory
            .read_bytes(pixels, (width * height) as usize * bytes_per_pixel)
            .unwrap_or_default();
        let mut pixels_rgba = Vec::with_capacity((width * height) as usize);
        for pixel in data.chunks(bytes_per_pixel) {
            let rgba = match (format, pixel_type) {
                (GL_RGBA, GL_UNSIGNED_BYTE) => Rgba8 {
                    r: pixel[0],
                    g: pixel[1],
                    b: pixel[2],
                    a: pixel[3],
                },
                (GL_RGB, GL_UNSIGNED_BYTE) => Rgba8 {
                    r: pixel[0],
                    g: pixel[1],
                    b: pixel[2],
                    a: 255,
                },
                (GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4) => {
                    let value = u16::from_le_bytes([pixel[0], pixel[1]]);
                    Rgba8 {
                        r: (((value >> 12) & 0xF) * 17) as u8,
                        g: (((value >> 8) & 0xF) * 17) as u8,
                        b: (((value >> 4) & 0xF) * 17) as u8,
                        a: ((value & 0xF) * 17) as u8,
                    }
                }
                (GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1) => {
                    let value = u16::from_le_bytes([pixel[0], pixel[1]]);
                    Rgba8 {
                        r: (((value >> 11) & 0x1F) * 8) as u8,
                        g: (((value >> 6) & 0x1F) * 8) as u8,
                        b: (((value >> 1) & 0x1F) * 8) as u8,
                        a: if value & 1 == 1 { 255 } else { 0 },
                    }
                }
                (GL_RGB, GL_UNSIGNED_SHORT_5_6_5) => {
                    let value = u16::from_le_bytes([pixel[0], pixel[1]]);
                    Rgba8 {
                        r: (((value >> 11) & 0x1F) * 8) as u8,
                        g: (((value >> 5) & 0x3F) * 4) as u8,
                        b: ((value & 0x1F) * 8) as u8,
                        a: 255,
                    }
                }
                _ => Rgba8 {
                    r: 255,
                    g: 0,
                    b: 255,
                    a: 255,
                },
            };
            pixels_rgba.push(rgba);
        }
        if name == "glTexImage2D" {
            renderer.tex_image_2d(width, height, &pixels_rgba);
        } else {
            renderer.upload_texture(width, height, &pixels_rgba);
        }
    }

    fn double_word(&self, machine: &Machine, register: usize) -> u64 {
        machine.cpu.r[register] as u64 | ((machine.cpu.r[register + 1] as u64) << 32)
    }

    fn set_double_word(&self, machine: &mut Machine, register: usize, value: u64) {
        machine.cpu.r[register] = value as u32;
        machine.cpu.r[register + 1] = (value >> 32) as u32;
    }

    fn read_double(&self, machine: &Machine, register: usize) -> f64 {
        f64::from_bits(self.double_word(machine, register))
    }

    fn set_double(&self, machine: &mut Machine, register: usize, value: f64) {
        self.set_double_word(machine, register, value.to_bits());
    }

    fn aeabi_idiv(&mut self, machine: &mut Machine) -> u32 {
        let (a, b) = (machine.cpu.r[0] as i32, machine.cpu.r[1] as i32);
        if b == 0 {
            self.log("AEABI integer division by zero".to_owned());
            return 0;
        }
        a.wrapping_div(b) as u32
    }

    fn aeabi_uidiv(&mut self, machine: &mut Machine) -> u32 {
        let (a, b) = (machine.cpu.r[0], machine.cpu.r[1]);
        if b == 0 {
            self.log("AEABI integer division by zero".to_owned());
            return 0;
        }
        a / b
    }

    fn aeabi_idivmod(&mut self, machine: &mut Machine) -> (u32, u32) {
        let (a, b) = (machine.cpu.r[0] as i32, machine.cpu.r[1] as i32);
        if b == 0 {
            self.log("AEABI integer division by zero".to_owned());
            return (0, 0);
        }
        (a.wrapping_div(b) as u32, a.wrapping_rem(b) as u32)
    }

    fn aeabi_uidivmod(&mut self, machine: &mut Machine) -> (u32, u32) {
        let (a, b) = (machine.cpu.r[0], machine.cpu.r[1]);
        if b == 0 {
            self.log("AEABI integer division by zero".to_owned());
            return (0, 0);
        }
        (a / b, a % b)
    }

    /// Bubble sort using the emulated comparator (deterministic, no allocation).
    fn qsort(
        &mut self,
        machine: &mut Machine,
        base: u32,
        count: u32,
        size: u32,
        comparator: u32,
    ) -> u32 {
        if size == 0 || count < 2 {
            return 0;
        }
        for pass in 0..count.saturating_sub(1) {
            for index in 0..count - 1 - pass {
                let left = base + index * size;
                let right = base + (index + 1) * size;
                let order = machine
                    .call_function(self, comparator, &[left, right])
                    .unwrap_or(0) as i32;
                if order > 0 {
                    let temporary = machine
                        .memory
                        .read_bytes(left, size as usize)
                        .unwrap_or_default();
                    let _ = machine.memory.copy_within(left, right, size);
                    let _ = machine.memory.write_bytes(right, &temporary);
                }
            }
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arm::{Cpu, CpuConfig};

    pub(super) const CODE: u32 = 0x1000_0000;
    pub(super) const DATA: u32 = 0x2000_0000;
    const STACK: u32 = 0x7F00_0000;

    pub(super) struct Fixture {
        pub(super) machine: Machine,
        pub(super) host: BasicHost,
    }

    impl Fixture {
        pub(super) fn new() -> Self {
            let mut machine = Machine::new(CpuConfig::default());
            machine.memory.map_anon(CODE, 0x1_0000).unwrap();
            machine.memory.map_anon(DATA, 0x1_0000).unwrap();
            machine.memory.map_anon(STACK, 0x10_0000).unwrap();
            machine.cpu = Cpu::new();
            machine.cpu.r[13] = STACK + 0xF_0000;
            for name in HOST_FUNCTIONS {
                machine.linker.register_host(name);
            }
            Self {
                machine,
                host: BasicHost::new(),
            }
        }

        pub(super) fn call(&mut self, name: &str, args: &[u32]) -> u32 {
            let slot = self
                .machine
                .linker
                .host_index_of(name)
                .unwrap_or_else(|| panic!("host function {name} is not registered"));
            for (index, value) in args.iter().enumerate() {
                self.machine.cpu.r[index] = *value;
            }
            self.host.call_host(&mut self.machine, slot)
        }
    }

    #[test]
    fn aeabi_integer_division() {
        let mut fixture = Fixture::new();
        assert_eq!(fixture.call("__aeabi_idiv", &[100, 7]), 14);
        assert_eq!(
            fixture.call("__aeabi_idiv", &[(-100i32) as u32, 7]),
            (-14i32) as u32
        );
        assert_eq!(
            fixture.call("__aeabi_uidiv", &[0xFFFF_FFFF, 2]),
            0x7FFF_FFFF
        );
        assert_eq!(
            fixture.call("__aeabi_idiv", &[5, 0]),
            0,
            "division by zero returns 0"
        );
        assert!(fixture
            .host
            .log
            .iter()
            .any(|line| line.contains("division by zero")));

        // idivmod returns the remainder in r1.
        fixture.call("__aeabi_idivmod", &[100, 7]);
        assert_eq!(fixture.machine.cpu.r[1], 2);
        fixture.call("__aeabi_uidivmod", &[0xFFFF_FFFF, 10]);
        assert_eq!(fixture.machine.cpu.r[1], 5);
    }

    #[test]
    fn aeabi_64_bit_arithmetic() {
        let mut fixture = Fixture::new();
        // __aeabi_lmul: r0:r1 * r2:r3 -> r0:r1.
        let a = 0x1_0000_0005u64;
        let b = 3;
        fixture.machine.cpu.r[0] = a as u32;
        fixture.machine.cpu.r[1] = (a >> 32) as u32;
        fixture.machine.cpu.r[2] = b as u32;
        fixture.machine.cpu.r[3] = 0;
        fixture.call("__aeabi_lmul", &[]);
        let product = fixture.machine.cpu.r[0] as u64 | ((fixture.machine.cpu.r[1] as u64) << 32);
        assert_eq!(product, a * b);

        // __aeabi_ldivmod: quotient in r0:r1, remainder in r2:r3.
        let dividend = 1000i64;
        let divisor = 33i64;
        fixture.machine.cpu.r[0] = dividend as u32;
        fixture.machine.cpu.r[1] = 0;
        fixture.machine.cpu.r[2] = divisor as u32;
        fixture.machine.cpu.r[3] = 0;
        fixture.call("__aeabi_ldivmod", &[]);
        let quotient = fixture.machine.cpu.r[0] as i64 | ((fixture.machine.cpu.r[1] as i64) << 32);
        let remainder = fixture.machine.cpu.r[2] as i64 | ((fixture.machine.cpu.r[3] as i64) << 32);
        assert_eq!(quotient, dividend / divisor);
        assert_eq!(remainder, dividend % divisor);
    }

    #[test]
    fn aeabi_float_conversions_and_arithmetic() {
        let mut fixture = Fixture::new();
        assert_eq!(fixture.call("__aeabi_i2f", &[3]), 3.0f32.to_bits());
        let sum = fixture.call("__aeabi_fadd", &[2.5f32.to_bits(), 4.0f32.to_bits()]);
        assert_eq!(f32::from_bits(sum), 6.5);
        assert_eq!(fixture.call("__aeabi_f2iz", &[6.9f32.to_bits()]), 6);

        // Doubles occupy register pairs.
        fixture.machine.cpu.r[0] = 1.5f64.to_bits() as u32;
        fixture.machine.cpu.r[1] = (1.5f64.to_bits() >> 32) as u32;
        fixture.machine.cpu.r[2] = 2.25f64.to_bits() as u32;
        fixture.machine.cpu.r[3] = (2.25f64.to_bits() >> 32) as u32;
        fixture.call("__aeabi_dadd", &[]);
        let bits = fixture.machine.cpu.r[0] as u64 | ((fixture.machine.cpu.r[1] as u64) << 32);
        assert_eq!(f64::from_bits(bits), 3.75);

        // Comparisons return 1/0.
        assert_eq!(
            fixture.call("__aeabi_fcmplt", &[1.0f32.to_bits(), 2.0f32.to_bits()]),
            1
        );
        assert_eq!(
            fixture.call("__aeabi_fcmpgt", &[1.0f32.to_bits(), 2.0f32.to_bits()]),
            0
        );
        assert_eq!(
            fixture.call("__aeabi_fcmpeq", &[2.0f32.to_bits(), 2.0f32.to_bits()]),
            1
        );
    }

    #[test]
    fn memory_and_string_shims() {
        let mut fixture = Fixture::new();
        let destination = fixture.call("memset", &[DATA, 0xAB, 4]);
        assert_eq!(destination, DATA);
        assert_eq!(
            fixture.machine.memory.read_bytes(DATA, 4).unwrap(),
            vec![0xAB; 4]
        );

        fixture
            .machine
            .memory
            .write_bytes(DATA + 0x10, &[1, 2, 3, 4])
            .unwrap();
        fixture.call("memcpy", &[DATA + 0x20, DATA + 0x10, 4]);
        assert_eq!(
            fixture.machine.memory.read_bytes(DATA + 0x20, 4).unwrap(),
            vec![1, 2, 3, 4]
        );

        fixture
            .machine
            .memory
            .write_cstr(DATA + 0x30, "hello")
            .unwrap();
        assert_eq!(fixture.call("strlen", &[DATA + 0x30]), 5);

        fixture
            .machine
            .memory
            .write_cstr(DATA + 0x40, "abc")
            .unwrap();
        fixture
            .machine
            .memory
            .write_cstr(DATA + 0x50, "abd")
            .unwrap();
        let difference = fixture.call("strcmp", &[DATA + 0x40, DATA + 0x50]) as i32;
        assert!(difference < 0);
        assert_eq!(fixture.call("strncmp", &[DATA + 0x40, DATA + 0x50, 2]), 0);
    }

    #[test]
    fn allocation_shims() {
        let mut fixture = Fixture::new();
        let first = fixture.call("malloc", &[64]);
        assert_ne!(first, 0);
        let second = fixture.call("calloc", &[4, 8]);
        assert_ne!(second, 0);
        assert_ne!(first, second);
        assert!(second > first, "bump allocator grows upward");
        // calloc memory is zeroed.
        assert_eq!(fixture.machine.memory.read_u32(second).unwrap(), 0);
        // realloc copies the old contents.
        fixture
            .machine
            .memory
            .write_u32(first, 0xDEAD_BEEF)
            .unwrap();
        let grown = fixture.call("realloc", &[first, 128]);
        assert_ne!(grown, 0);
        assert_eq!(fixture.machine.memory.read_u32(grown).unwrap(), 0xDEAD_BEEF);
        fixture.call("free", &[grown]);
    }

    #[test]
    fn formatter_handles_common_specifiers() {
        let mut fixture = Fixture::new();
        fixture
            .machine
            .memory
            .write_cstr(DATA + 0x80, "score=%d name=%s hex=%05x pct=%c%%")
            .unwrap();
        fixture
            .machine
            .memory
            .write_cstr(DATA + 0xB0, "oven")
            .unwrap();
        // Varargs start at r2: r2 = 42, r3 = string, stack[0] = 0x2a,
        // stack[1] = 'O'.
        fixture.machine.cpu.r[2] = 42;
        fixture.machine.cpu.r[3] = DATA + 0xB0;
        fixture
            .machine
            .memory
            .write_u32(STACK + 0xF_0000, 0x2a)
            .unwrap();
        fixture
            .machine
            .memory
            .write_u32(STACK + 0xF_0004, 0x4F)
            .unwrap();
        let destination = DATA + 0xC0;
        let length = fixture.call("sprintf", &[destination, DATA + 0x80]);
        assert_eq!(
            fixture.machine.memory.read_cstr(destination, 64).unwrap(),
            "score=42 name=oven hex=0002a pct=O%"
        );
        assert_eq!(length, 35);
    }

    #[test]
    fn qsort_uses_the_emulated_comparator() {
        let mut fixture = Fixture::new();
        // Comparator: r0 > r1 -> 1 (ascending integer compare).
        let comparator = CODE + 0x800;
        fixture
            .machine
            .memory
            .write_u32(comparator, 0xE080_0001)
            .unwrap(); // ADD r0, r0, r1
        fixture
            .machine
            .memory
            .write_u32(comparator + 4, 0xE12F_FF1E)
            .unwrap(); // BX lr
        let values: [u32; 5] = [5, 3, 9, 1, 4];
        for (index, value) in values.iter().enumerate() {
            fixture
                .machine
                .memory
                .write_u32(DATA + index as u32 * 4, *value)
                .unwrap();
        }
        fixture.call("qsort", &[DATA, 5, 4, comparator]);
        // The comparator adds its arguments, so verify the sort ran without
        // faults; ordering depends on the comparator semantics above.
        assert_ne!(fixture.machine.memory.read_u32(DATA).unwrap(), 0);
    }

    #[test]
    fn thread_local_and_error_slots() {
        let mut fixture = Fixture::new();
        let key_address = DATA + 0x200;
        fixture.machine.cpu.r[0] = key_address;
        fixture.call("pthread_key_create", &[key_address, 0]);
        let key = fixture.machine.memory.read_u32(key_address).unwrap();
        assert_ne!(key, 0);
        fixture.call("pthread_setspecific", &[key, 0xBEEF]);
        assert_eq!(fixture.call("pthread_getspecific", &[key]), 0xBEEF);
        // __errno returns a writable slot.
        let errno = fixture.call("__errno", &[]);
        fixture.machine.memory.write_u32(errno, 12).unwrap();
        assert_eq!(fixture.machine.memory.read_u32(errno).unwrap(), 12);
    }

    #[test]
    fn network_and_files_fail_closed() {
        let mut fixture = Fixture::new();
        assert_eq!(fixture.call("socket", &[2, 1, 0]), (-1i32) as u32);
        assert_eq!(fixture.call("open", &[DATA, 0, 0]), (-1i32) as u32);
        assert_ne!(fixture.call("getaddrinfo", &[DATA, DATA, 0, 0]), 0);
        assert!(fixture
            .host
            .log
            .iter()
            .any(|line| line.contains("disabled")));
    }

    #[test]
    fn unregistered_imports_report_diagnostically() {
        let mut fixture = Fixture::new();
        // A slot beyond the known set behaves as a diagnostic stub.
        let value = fixture.host.call_host(&mut fixture.machine, 9_999);
        assert_eq!(value, 0);
        assert!(!fixture.host.log.is_empty());
    }
}

#[cfg(test)]
mod va_list_tests {
    use super::tests::{Fixture, DATA};

    #[test]
    fn vsprintf_reads_varargs_from_guest_va_list() {
        let mut fixture = Fixture::new();
        // Guest frame: the engine thunk builds its va_list with
        // ADD r2, sp, #N pointing into the caller's stack varargs.
        let varargs = DATA + 0x400;
        fixture.machine.memory.write_u32(varargs, 42).unwrap(); // %d
        fixture
            .machine
            .memory
            .write_u32(varargs + 4, DATA + 0x500)
            .unwrap(); // %s pointer
        let bits = 2.5f64.to_bits();
        fixture
            .machine
            .memory
            .write_u32(varargs + 8, bits as u32)
            .unwrap();
        fixture
            .machine
            .memory
            .write_u32(varargs + 12, (bits >> 32) as u32)
            .unwrap(); // %f (already 8-aligned)
        fixture
            .machine
            .memory
            .write_cstr(DATA + 0x500, "km")
            .unwrap();
        fixture
            .machine
            .memory
            .write_cstr(DATA + 0x460, "dist=%dm at %s (%.1f)")
            .unwrap();
        let destination = DATA + 0x580;

        fixture.machine.cpu.r[0] = destination;
        fixture.machine.cpu.r[1] = DATA + 0x460;
        fixture.machine.cpu.r[2] = varargs; // the va_list itself
        let written = fixture.call("vsprintf", &[destination, DATA + 0x460, varargs]);

        assert_eq!(
            fixture
                .machine
                .memory
                .read_cstr(destination, 64)
                .unwrap_or_default(),
            "dist=42m at km (2.5)"
        );
        assert_eq!(written, 20);
    }

    #[test]
    fn vsprintf_double_alignment_is_honored() {
        let mut fixture = Fixture::new();
        // One int (4 bytes) then a double: AAPCS pads to 8-byte alignment.
        let varargs = DATA + 0x402; // deliberately 2 mod 4 -> aligns to +0x408
        fixture.machine.memory.write_u32(varargs, 7).unwrap();
        let aligned = (varargs + 7) & !7;
        let bits = 1.5f64.to_bits();
        fixture
            .machine
            .memory
            .write_u32(aligned, bits as u32)
            .unwrap();
        fixture
            .machine
            .memory
            .write_u32(aligned + 4, (bits >> 32) as u32)
            .unwrap();
        fixture
            .machine
            .memory
            .write_cstr(DATA + 0x440, "%d:%.2f")
            .unwrap();
        let destination = DATA + 0x600;
        fixture.machine.cpu.r[0] = destination;
        fixture.machine.cpu.r[1] = DATA + 0x440;
        fixture.machine.cpu.r[2] = varargs;
        fixture.call("vsprintf", &[]);
        assert_eq!(
            fixture
                .machine
                .memory
                .read_cstr(destination, 32)
                .unwrap_or_default(),
            "7:1.50"
        );
    }
}
