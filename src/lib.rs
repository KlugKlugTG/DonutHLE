//! DonutHLE core: an explicit foundation for Android 1.x HLE work.

pub mod apk;
pub mod assets;
pub mod audio;
pub mod compat;
pub mod dalvik;
#[cfg(target_os = "linux")]
pub mod desktop;
pub mod framework;
pub mod gles;
pub mod gles1_on_gl2;
pub mod gles_native;
pub mod input;
pub mod manifest;
pub mod resources;
pub mod runtime;
pub mod vm;

/// Single host graphics entry point: GLES 1.x is always adapted to the GL2-style renderer.
pub type HostGles = gles1_on_gl2::Gles1OnGl2;

pub const ANDROID_X_MIN_API_LEVEL: u32 = 1;
pub const ANDROID_X_MAX_API_LEVEL: u32 = 4;
pub const API_LEVEL: u32 = ANDROID_X_MAX_API_LEVEL;
pub const RELEASE: &str = "Android 1.x";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualScreen {
    pub width: u32,
    pub height: u32,
}

impl Default for VirtualScreen {
    fn default() -> Self {
        Self {
            width: 320,
            height: 480,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

#[derive(Debug, Clone)]
pub struct Framebuffer {
    width: u32,
    height: u32,
    pixels: Vec<Rgba8>,
}

impl Framebuffer {
    pub fn new(screen: VirtualScreen, color: Rgba8) -> Self {
        Self {
            width: screen.width,
            height: screen.height,
            pixels: vec![color; (screen.width * screen.height) as usize],
        }
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn clear(&mut self, color: Rgba8) {
        self.pixels.fill(color);
    }
    pub fn set_pixel(&mut self, x: u32, y: u32, color: Rgba8) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        self.pixels[(y * self.width + x) as usize] = color;
        true
    }
    pub fn pixel(&self, x: u32, y: u32) -> Option<Rgba8> {
        if x >= self.width || y >= self.height {
            return None;
        }
        Some(self.pixels[(y * self.width + x) as usize])
    }
    pub fn pixels(&self) -> &[Rgba8] {
        &self.pixels
    }
}

use std::sync::Mutex;

struct FrameSnapshot {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

static FRAME_SNAPSHOT: Mutex<Option<FrameSnapshot>> = Mutex::new(None);
static RUNTIME: Mutex<Option<runtime::Runtime>> = Mutex::new(None);

fn clear_framebuffer() {
    if let Ok(mut snapshot) = FRAME_SNAPSHOT.lock() {
        *snapshot = None;
    }
}

pub(crate) fn publish_framebuffer(framebuffer: &Framebuffer) {
    let mut pixels = Vec::with_capacity(framebuffer.pixels().len().saturating_mul(4));
    for pixel in framebuffer.pixels() {
        pixels.extend_from_slice(&[pixel.r, pixel.g, pixel.b, pixel.a]);
    }
    if let Ok(mut snapshot) = FRAME_SNAPSHOT.lock() {
        *snapshot = Some(FrameSnapshot {
            width: framebuffer.width(),
            height: framebuffer.height(),
            pixels,
        });
    }
}

#[no_mangle]
pub extern "C" fn donuthle_framebuffer_width() -> u32 {
    FRAME_SNAPSHOT
        .lock()
        .ok()
        .and_then(|snapshot| snapshot.as_ref().map(|frame| frame.width))
        .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn donuthle_framebuffer_height() -> u32 {
    FRAME_SNAPSHOT
        .lock()
        .ok()
        .and_then(|snapshot| snapshot.as_ref().map(|frame| frame.height))
        .unwrap_or(0)
}

/// # Safety
///
/// `output` must point to a writable buffer of at least `output_len` bytes.
#[no_mangle]
pub unsafe extern "C" fn donuthle_framebuffer_copy(output: *mut u8, output_len: usize) -> usize {
    if output.is_null() || output_len == 0 {
        return 0;
    }
    let Ok(snapshot) = FRAME_SNAPSHOT.lock() else {
        return 0;
    };
    let Some(frame) = snapshot.as_ref() else {
        return 0;
    };
    let count = frame.pixels.len().min(output_len);
    std::ptr::copy_nonoverlapping(frame.pixels.as_ptr(), output, count);
    count
}

#[no_mangle]
pub extern "C" fn donuthle_render_frame(_width: u32, _height: u32) -> u32 {
    let Ok(mut runtime) = RUNTIME.lock() else {
        return 0;
    };
    let Some(runtime) = runtime.as_mut() else {
        return 0;
    };
    let Some(session) = runtime.session.as_mut() else {
        clear_framebuffer();
        return 0;
    };
    match session.render_current_frame() {
        Ok((commands, _)) => commands as u32,
        Err(error) => {
            eprintln!("DonutHLE frame render failed: {error}");
            clear_framebuffer();
            0
        }
    }
}

#[no_mangle]
pub extern "C" fn donuthle_touch(action: i32, x: f32, y: f32) -> i32 {
    let Ok(mut runtime) = RUNTIME.lock() else {
        return 0;
    };
    let Some(runtime) = runtime.as_mut() else {
        return 0;
    };
    let Some(session) = runtime.session.as_mut() else {
        return 0;
    };
    let is_tiny_santa = session
        .vm
        .heap_object(session.listener)
        .is_some_and(|object| {
            matches!(
                object,
                crate::vm::HeapObject::Instance { class_name, .. }
                    if class_name == "Lde/nurogames/android/tinysanta/views/TinySantaView;"
            )
        });
    if is_tiny_santa && action == 0 && (30.0..=290.0).contains(&x) && (155.0..=335.0).contains(&y) {
        session.start_tiny_santa_game();
    }
    let result = session.vm.dispatch_touch(session.listener, action, x, y);
    match result {
        Ok(crate::vm::Value::Int(value)) => value,
        Ok(_) => 1,
        Err(error) => {
            eprintln!("DonutHLE touch dispatch failed: {error}");
            0
        }
    }
}

#[no_mangle]
pub extern "C" fn donuthle_core_info() -> *const std::os::raw::c_char {
    c"Rust DonutHLE core: Android 1.x APK parsing, AXML, resources, Dalvik VM, framework, GLES, and audio subsystems".as_ptr()
}

#[no_mangle]
pub extern "C" fn donuthle_game_title() -> *mut std::os::raw::c_char {
    let title = RUNTIME
        .lock()
        .ok()
        .and_then(|runtime| {
            runtime
                .as_ref()
                .and_then(|runtime| runtime.game_title.clone())
        })
        .unwrap_or_else(|| "Unknown game".to_owned());
    std::ffi::CString::new(title)
        .unwrap_or_else(|_| std::ffi::CString::new("Unknown game").unwrap())
        .into_raw()
}

/// # Safety
///
/// `path` must be a valid, NUL-terminated C string for the lifetime of this call.
#[no_mangle]
pub unsafe extern "C" fn donuthle_launch_report(
    path: *const std::os::raw::c_char,
) -> *mut std::os::raw::c_char {
    clear_framebuffer();
    if let Ok(mut runtime) = RUNTIME.lock() {
        *runtime = None;
    }
    let result = if path.is_null() {
        Err(anyhow::anyhow!("APK path is null"))
    } else {
        let path = unsafe { std::ffi::CStr::from_ptr(path) };
        match path.to_str() {
            Ok(path) => {
                let mut runtime = runtime::Runtime::default();
                match runtime.launch(path) {
                    Ok(report) => match RUNTIME.lock() {
                        Ok(mut shared) => {
                            *shared = Some(runtime);
                            Ok(format!(
                                "{}\nLauncher: {}\n{}",
                                report.message, report.launcher_activity, report.dex
                            ))
                        }
                        Err(_) => Err(anyhow::anyhow!("runtime lock is poisoned")),
                    },
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(anyhow::anyhow!("APK path is not UTF-8: {error}")),
        }
    };
    let message = match result {
        Ok(message) => message,
        Err(error) => format!("Runtime error: {error}"),
    };
    std::ffi::CString::new(message)
        .unwrap_or_else(|_| std::ffi::CString::new("Runtime returned an invalid message").unwrap())
        .into_raw()
}

/// # Safety
///
/// `value` must be a pointer returned by `donuthle_launch_report` and must not be freed twice.
#[no_mangle]
pub unsafe extern "C" fn donuthle_free_string(value: *mut std::os::raw::c_char) {
    if !value.is_null() {
        drop(std::ffi::CString::from_raw(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framebuffer_clear_and_bounds() {
        let mut fb = Framebuffer::new(
            VirtualScreen {
                width: 2,
                height: 2,
            },
            Rgba8 {
                r: 1,
                g: 2,
                b: 3,
                a: 255,
            },
        );
        assert!(fb.set_pixel(
            1,
            1,
            Rgba8 {
                r: 9,
                g: 8,
                b: 7,
                a: 255
            }
        ));
        assert!(!fb.set_pixel(
            2,
            1,
            Rgba8 {
                r: 0,
                g: 0,
                b: 0,
                a: 0
            }
        ));
        assert_eq!(fb.pixel(1, 1).unwrap().r, 9);
        fb.clear(Rgba8 {
            r: 4,
            g: 5,
            b: 6,
            a: 255,
        });
        assert_eq!(fb.pixels()[0].g, 5);
    }
}

#[cfg(test)]
mod framebuffer_tests {
    use super::*;

    #[test]
    fn published_framebuffer_is_available_to_native_bridge() {
        let framebuffer = Framebuffer::new(
            VirtualScreen {
                width: 1,
                height: 1,
            },
            Rgba8 {
                r: 1,
                g: 2,
                b: 3,
                a: 4,
            },
        );
        publish_framebuffer(&framebuffer);
        assert_eq!(donuthle_framebuffer_width(), 1);
        assert_eq!(donuthle_framebuffer_height(), 1);
        let mut output = [0_u8; 4];
        assert_eq!(
            unsafe { donuthle_framebuffer_copy(output.as_mut_ptr(), output.len()) },
            4
        );
        assert_eq!(output, [1, 2, 3, 4]);
    }
}
