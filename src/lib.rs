//! DonutHLE core: a small, explicit foundation for Android 1.6 HLE work.

pub mod apk;
pub mod dalvik;
pub mod manifest;
pub mod resources;
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
