//! DonutHLE core: a small, explicit foundation for Android 1.6 HLE work.

pub mod apk;
pub mod dalvik;
pub mod manifest;
pub mod runtime;

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
