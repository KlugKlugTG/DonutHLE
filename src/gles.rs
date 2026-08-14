use crate::{Framebuffer, Rgba8, VirtualScreen};

#[derive(Debug)]
pub struct GlesContext {
    framebuffer: Framebuffer,
    clear_color: Rgba8,
}

impl GlesContext {
    pub fn new(screen: VirtualScreen) -> Self {
        Self {
            framebuffer: Framebuffer::new(
                screen,
                Rgba8 {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
            ),
            clear_color: Rgba8 {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            },
        }
    }
    pub fn set_clear_color(&mut self, color: Rgba8) {
        self.clear_color = color;
    }
    pub fn clear(&mut self) {
        self.framebuffer.clear(self.clear_color);
    }
    pub fn set_pixel(&mut self, x: u32, y: u32, color: Rgba8) -> bool {
        self.framebuffer.set_pixel(x, y, color)
    }
    pub fn framebuffer(&self) -> &Framebuffer {
        &self.framebuffer
    }
}
