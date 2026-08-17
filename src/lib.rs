//! DonutHLE core: a small, explicit foundation for Android 1.6 (Donut) HLE work.

pub mod apk;
pub mod audio;
pub mod compat;
pub mod dalvik;
pub mod framework;
pub mod gles;
pub mod gles_native;
pub mod input;
pub mod manifest;
pub mod resources;
pub mod runtime;
pub mod vm;

pub const API_LEVEL: u32 = 4;
pub const RELEASE: &str = "Donut";

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

#[no_mangle]
pub extern "C" fn donuthle_core_info() -> *const std::os::raw::c_char {
    c"Rust DonutHLE core: APK parsing, AXML, resources, Dalvik VM, framework, GLES, and audio subsystems".as_ptr()
}

/// # Safety
///
/// `path` must be a valid, NUL-terminated C string for the lifetime of this call.
#[no_mangle]
pub unsafe extern "C" fn donuthle_launch_report(
    path: *const std::os::raw::c_char,
) -> *mut std::os::raw::c_char {
    let result = if path.is_null() {
        Err(anyhow::anyhow!("APK path is null"))
    } else {
        let path = unsafe { std::ffi::CStr::from_ptr(path) };
        match path.to_str() {
            Ok(path) => runtime::Runtime::default().launch(path).map(|report| {
                format!(
                    "{}\nLauncher: {}\n{}",
                    report.message, report.launcher_activity, report.dex
                )
            }),
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
