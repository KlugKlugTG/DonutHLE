use crate::{Framebuffer, Rgba8, VirtualScreen};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub u: f32,
    pub v: f32,
    pub color: Rgba8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Primitive {
    Points,
    Lines,
    Triangles,
    TriangleStrip,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GlesCommand {
    Clear(Rgba8),
    Viewport {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    Draw {
        primitive: Primitive,
        vertices: Vec<Vertex>,
    },
}

#[derive(Debug)]
pub struct GlesContext {
    framebuffer: Framebuffer,
    clear_color: Rgba8,
    viewport: (i32, i32, u32, u32),
    commands: Vec<GlesCommand>,
}

impl GlesContext {
    pub fn new(screen: VirtualScreen) -> Self {
        let clear_color = Rgba8 {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        };
        Self {
            framebuffer: Framebuffer::new(screen, clear_color),
            clear_color,
            viewport: (0, 0, screen.width, screen.height),
            commands: Vec::new(),
        }
    }

    pub fn set_clear_color(&mut self, color: Rgba8) {
        self.clear_color = color;
    }

    pub fn clear(&mut self) {
        self.framebuffer.clear(self.clear_color);
        self.commands.push(GlesCommand::Clear(self.clear_color));
    }

    pub fn viewport(&mut self, x: i32, y: i32, width: u32, height: u32) {
        self.viewport = (x, y, width, height);
        self.commands.push(GlesCommand::Viewport {
            x,
            y,
            width,
            height,
        });
    }

    pub fn draw(&mut self, primitive: Primitive, vertices: &[Vertex]) {
        let command = GlesCommand::Draw {
            primitive,
            vertices: vertices.to_vec(),
        };
        self.rasterize(&command);
        self.commands.push(command);
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, color: Rgba8) -> bool {
        self.framebuffer.set_pixel(x, y, color)
    }

    pub fn framebuffer(&self) -> &Framebuffer {
        &self.framebuffer
    }

    pub fn command_count(&self) -> usize {
        self.commands.len()
    }

    pub fn commands(&self) -> &[GlesCommand] {
        &self.commands
    }

    fn rasterize(&mut self, command: &GlesCommand) {
        let GlesCommand::Draw {
            primitive,
            vertices,
        } = command
        else {
            return;
        };
        match primitive {
            Primitive::Points => {
                for vertex in vertices {
                    self.plot(vertex);
                }
            }
            Primitive::Lines => {
                for pair in vertices.chunks_exact(2) {
                    self.line(pair[0], pair[1]);
                }
            }
            Primitive::Triangles => {
                for tri in vertices.chunks_exact(3) {
                    self.triangle(tri[0], tri[1], tri[2]);
                }
            }
            Primitive::TriangleStrip => {
                for tri in vertices.windows(3) {
                    self.triangle(tri[0], tri[1], tri[2]);
                }
            }
        }
    }

    fn project(&self, vertex: Vertex) -> (i32, i32) {
        let (_, _, width, height) = self.viewport;
        let x = ((vertex.x + 1.0) * 0.5 * width as f32) as i32;
        let y = ((1.0 - vertex.y) * 0.5 * height as f32) as i32;
        (x, y)
    }

    fn plot(&mut self, vertex: &Vertex) {
        let (x, y) = self.project(*vertex);
        if x >= 0 && y >= 0 {
            self.framebuffer.set_pixel(x as u32, y as u32, vertex.color);
        }
    }

    fn line(&mut self, a: Vertex, b: Vertex) {
        let (x0, y0) = self.project(a);
        let (x1, y1) = self.project(b);
        let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
        for step in 0..=steps {
            let t = step as f32 / steps as f32;
            let vertex = Vertex {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                u: 0.0,
                v: 0.0,
                color: blend(a.color, b.color, t),
            };
            let x = x0 + ((x1 - x0) as f32 * t) as i32;
            let y = y0 + ((y1 - y0) as f32 * t) as i32;
            if x >= 0 && y >= 0 {
                self.framebuffer.set_pixel(x as u32, y as u32, vertex.color);
            }
        }
    }

    fn triangle(&mut self, a: Vertex, b: Vertex, c: Vertex) {
        let (ax, ay) = self.project(a);
        let (bx, by) = self.project(b);
        let (cx, cy) = self.project(c);
        let min_x = ax.min(bx).min(cx).max(0);
        let max_x = ax.max(bx).max(cx);
        let min_y = ay.min(by).min(cy).max(0);
        let max_y = ay.max(by).max(cy);
        let area = edge(ax, ay, bx, by, cx, cy) as f32;
        if area == 0.0 {
            return;
        }
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let w0 = edge(bx, by, cx, cy, x, y) as f32 / area;
                let w1 = edge(cx, cy, ax, ay, x, y) as f32 / area;
                let w2 = edge(ax, ay, bx, by, x, y) as f32 / area;
                if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                    self.framebuffer.set_pixel(
                        x as u32,
                        y as u32,
                        barycentric(a.color, b.color, c.color, w0, w1, w2),
                    );
                }
            }
        }
    }
}

fn edge(ax: i32, ay: i32, bx: i32, by: i32, cx: i32, cy: i32) -> i32 {
    (cx - ax) * (by - ay) - (cy - ay) * (bx - ax)
}

fn blend(a: Rgba8, b: Rgba8, t: f32) -> Rgba8 {
    Rgba8 {
        r: (a.r as f32 + (b.r as f32 - a.r as f32) * t) as u8,
        g: (a.g as f32 + (b.g as f32 - a.g as f32) * t) as u8,
        b: (a.b as f32 + (b.b as f32 - a.b as f32) * t) as u8,
        a: (a.a as f32 + (b.a as f32 - a.a as f32) * t) as u8,
    }
}

fn barycentric(a: Rgba8, b: Rgba8, c: Rgba8, wa: f32, wb: f32, wc: f32) -> Rgba8 {
    Rgba8 {
        r: (a.r as f32 * wa + b.r as f32 * wb + c.r as f32 * wc) as u8,
        g: (a.g as f32 * wa + b.g as f32 * wb + c.g as f32 * wc) as u8,
        b: (a.b as f32 * wa + b.b as f32 * wb + c.b as f32 * wc) as u8,
        a: (a.a as f32 * wa + b.a as f32 * wb + c.a as f32 * wc) as u8,
    }
}
