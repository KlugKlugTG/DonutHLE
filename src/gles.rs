use std::collections::HashMap;

use crate::{Framebuffer, Rgba8, VirtualScreen};

const COLOR_BUFFER_BIT: u32 = 0x4000;
const DEPTH_BUFFER_BIT: u32 = 0x0100;
const POINTS: u32 = 0x0000;
const LINES: u32 = 0x0001;
const LINE_LOOP: u32 = 0x0002;
const LINE_STRIP: u32 = 0x0003;
const TRIANGLE_STRIP: u32 = 0x0005;
const TRIANGLE_FAN: u32 = 0x0006;
const MODELVIEW: u32 = 0x1700;
const PROJECTION: u32 = 0x1701;
const TEXTURE: u32 = 0x1702;
const TEXTURE_2D: u32 = 0x0DE1;
const BLEND: u32 = 0x0BE2;
const DEPTH_TEST: u32 = 0x0B71;
const SCISSOR_TEST: u32 = 0x0C11;
const SRC_COLOR: u32 = 0x0300;
const ONE_MINUS_SRC_COLOR: u32 = 0x0301;
const SRC_ALPHA: u32 = 0x0302;
const ONE_MINUS_SRC_ALPHA: u32 = 0x0303;
const DST_ALPHA: u32 = 0x0304;
const ONE_MINUS_DST_ALPHA: u32 = 0x0305;
const DST_COLOR: u32 = 0x0306;
const ONE_MINUS_DST_COLOR: u32 = 0x0307;
const ZERO: u32 = 0;
const ONE: u32 = 1;

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
    LineLoop,
    LineStrip,
    Triangles,
    TriangleStrip,
    TriangleFan,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GlesCommand {
    Clear(Rgba8),
    ClearColor(Rgba8),
    ClearMask(u32),
    Enable(u32),
    Disable(u32),
    BlendFunc {
        src: u32,
        dst: u32,
    },
    BindTexture {
        target: u32,
        texture: u32,
    },
    TexImage2D {
        width: u32,
        height: u32,
    },
    DrawArrays {
        mode: u32,
        first: i32,
        count: i32,
    },
    DrawElements {
        mode: u32,
        count: i32,
        element_type: u32,
    },
    Viewport {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    Scissor {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    MatrixMode(u32),
    LoadIdentity,
    LoadMatrix,
    MultMatrix,
    PushMatrix,
    PopMatrix,
    Draw {
        primitive: Primitive,
        vertices: Vec<Vertex>,
    },
}

#[derive(Debug, Clone)]
struct TextureImage {
    width: u32,
    height: u32,
    pixels: Vec<Rgba8>,
}

#[derive(Debug, Clone)]
struct Pointer {
    size: usize,
    stride: usize,
    values: Vec<f32>,
}

#[derive(Debug, Clone, Copy)]
struct ProjectedVertex {
    x: i32,
    y: i32,
    z: f32,
}

#[derive(Debug)]
pub struct GlesContext {
    framebuffer: Framebuffer,
    depth_buffer: Vec<f32>,
    clear_color: Rgba8,
    current_color: Rgba8,
    viewport: (i32, i32, u32, u32),
    scissor: Option<(i32, i32, u32, u32)>,
    commands: Vec<GlesCommand>,
    enabled: HashMap<u32, bool>,
    blend_src: u32,
    blend_dst: u32,
    bound_texture: u32,
    next_texture: u32,
    textures: HashMap<u32, TextureImage>,
    vertex_pointer: Option<Pointer>,
    color_pointer: Option<Pointer>,
    texcoord_pointer: Option<Pointer>,
    matrix_mode: u32,
    projection_matrix: [f32; 16],
    modelview_matrix: [f32; 16],
    texture_matrix: [f32; 16],
    matrix_stacks: HashMap<u32, Vec<[f32; 16]>>,
    next_frame_pixels: usize,
}

impl Default for GlesContext {
    fn default() -> Self {
        Self::new(VirtualScreen::default())
    }
}

impl GlesContext {
    pub fn new(screen: VirtualScreen) -> Self {
        let clear_color = Rgba8 {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        };
        let pixel_count = screen.width.saturating_mul(screen.height) as usize;
        Self {
            framebuffer: Framebuffer::new(screen, clear_color),
            depth_buffer: vec![1.0; pixel_count],
            clear_color,
            current_color: Rgba8 {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
            viewport: (0, 0, screen.width, screen.height),
            scissor: None,
            commands: Vec::new(),
            enabled: HashMap::new(),
            blend_src: ONE,
            blend_dst: ZERO,
            bound_texture: 0,
            next_texture: 1,
            textures: HashMap::new(),
            vertex_pointer: None,
            color_pointer: None,
            texcoord_pointer: None,
            matrix_mode: MODELVIEW,
            projection_matrix: identity(),
            modelview_matrix: identity(),
            texture_matrix: identity(),
            matrix_stacks: HashMap::new(),
            next_frame_pixels: 0,
        }
    }

    pub fn set_clear_color(&mut self, color: Rgba8) {
        self.clear_color = color;
    }

    pub fn clear(&mut self) {
        self.clear_mask(COLOR_BUFFER_BIT | DEPTH_BUFFER_BIT);
        self.commands.push(GlesCommand::Clear(self.clear_color));
    }

    pub fn clear_color(&mut self, color: Rgba8) {
        self.clear_color = color;
        self.commands.push(GlesCommand::ClearColor(color));
    }

    pub fn clear_mask(&mut self, mask: u32) {
        if mask & COLOR_BUFFER_BIT != 0 {
            self.clear_color_buffer();
        }
        if mask & DEPTH_BUFFER_BIT != 0 {
            self.depth_buffer.fill(1.0);
        }
        self.commands.push(GlesCommand::ClearMask(mask));
    }

    pub fn enable(&mut self, capability: u32) {
        self.enabled.insert(capability, true);
        self.commands.push(GlesCommand::Enable(capability));
    }

    pub fn disable(&mut self, capability: u32) {
        self.enabled.insert(capability, false);
        self.commands.push(GlesCommand::Disable(capability));
    }

    pub fn blend_func(&mut self, src: u32, dst: u32) {
        self.blend_src = src;
        self.blend_dst = dst;
        self.commands.push(GlesCommand::BlendFunc { src, dst });
    }

    pub fn bind_texture(&mut self, target: u32, texture: u32) {
        self.bound_texture = texture;
        self.commands
            .push(GlesCommand::BindTexture { target, texture });
    }

    pub fn gen_texture(&mut self) -> u32 {
        let texture = self.next_texture;
        self.next_texture = self.next_texture.saturating_add(1);
        self.textures.insert(texture, white_texture());
        texture
    }

    pub fn delete_texture(&mut self, texture: u32) {
        self.textures.remove(&texture);
        if self.bound_texture == texture {
            self.bound_texture = 0;
        }
    }

    pub fn tex_image_2d(&mut self, width: u32, height: u32, pixels: &[Rgba8]) {
        if self.bound_texture == 0 {
            return;
        }
        let count = width.saturating_mul(height) as usize;
        let mut data = vec![
            Rgba8 {
                r: 255,
                g: 255,
                b: 255,
                a: 255
            };
            count
        ];
        for (destination, source) in data.iter_mut().zip(pixels.iter().copied()) {
            *destination = source;
        }
        self.textures.insert(
            self.bound_texture,
            TextureImage {
                width,
                height,
                pixels: data,
            },
        );
        self.commands
            .push(GlesCommand::TexImage2D { width, height });
    }

    pub fn upload_texture(&mut self, width: u32, height: u32, pixels: &[Rgba8]) -> u32 {
        let texture = self.gen_texture();
        self.bind_texture(TEXTURE_2D, texture);
        self.tex_image_2d(width, height, pixels);
        texture
    }

    pub fn draw_textured_quad_pixels(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        texture: u32,
        color: Rgba8,
    ) {
        self.draw_textured_region_pixels(x, y, width, height, texture, 0.0, 0.0, 1.0, 1.0, color);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_textured_region_pixels(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        texture: u32,
        u0: f32,
        v0: f32,
        u1: f32,
        v1: f32,
        color: Rgba8,
    ) {
        let previous = self.bound_texture;
        self.bind_texture(TEXTURE_2D, texture);
        self.enable(TEXTURE_2D);
        let (_, _, viewport_width, viewport_height) = self.viewport;
        if viewport_width != 0 && viewport_height != 0 {
            let left = (x / viewport_width as f32) * 2.0 - 1.0;
            let right = ((x + width) / viewport_width as f32) * 2.0 - 1.0;
            let top = 1.0 - (y / viewport_height as f32) * 2.0;
            let bottom = 1.0 - ((y + height) / viewport_height as f32) * 2.0;
            self.draw(
                Primitive::TriangleStrip,
                &[
                    Vertex {
                        x: left,
                        y: top,
                        z: 0.0,
                        u: u0,
                        v: v0,
                        color,
                    },
                    Vertex {
                        x: right,
                        y: top,
                        z: 0.0,
                        u: u1,
                        v: v0,
                        color,
                    },
                    Vertex {
                        x: right,
                        y: bottom,
                        z: 0.0,
                        u: u1,
                        v: v1,
                        color,
                    },
                    Vertex {
                        x: left,
                        y: bottom,
                        z: 0.0,
                        u: u0,
                        v: v1,
                        color,
                    },
                ],
            );
        }
        self.bind_texture(TEXTURE_2D, previous);
    }

    pub fn set_current_color(&mut self, color: Rgba8) {
        self.current_color = color;
    }

    pub fn set_vertex_pointer(&mut self, size: usize, stride: usize, values: Vec<f32>) {
        self.vertex_pointer = Some(Pointer {
            size: size.clamp(2, 4),
            stride: stride_in_values(size, stride),
            values,
        });
    }

    pub fn set_color_pointer(&mut self, size: usize, stride: usize, values: Vec<f32>) {
        self.color_pointer = Some(Pointer {
            size: size.clamp(3, 4),
            stride: stride_in_values(size, stride),
            values,
        });
    }

    pub fn set_texcoord_pointer(&mut self, size: usize, stride: usize, values: Vec<f32>) {
        self.texcoord_pointer = Some(Pointer {
            size: size.clamp(2, 4),
            stride: stride_in_values(size, stride),
            values,
        });
    }

    pub fn draw_arrays(&mut self, mode: u32, first: i32, count: i32) {
        self.commands
            .push(GlesCommand::DrawArrays { mode, first, count });
        let Some(pointer) = self.vertex_pointer.clone() else {
            return;
        };
        if first < 0 || count <= 0 || pointer.values.is_empty() {
            return;
        }
        let mut vertices = Vec::with_capacity(count as usize);
        for index in first as usize..(first as usize).saturating_add(count as usize) {
            let offset = index.saturating_mul(pointer.stride);
            if offset.saturating_add(pointer.size) > pointer.values.len() {
                break;
            }
            vertices.push(Vertex {
                x: pointer.values[offset],
                y: pointer.values[offset + 1],
                z: pointer.values.get(offset + 2).copied().unwrap_or(0.0),
                u: self.pointer_value(&self.texcoord_pointer, index, 0),
                v: self.pointer_value(&self.texcoord_pointer, index, 1),
                color: self.pointer_color(index),
            });
        }
        self.draw(Self::primitive(mode), &vertices);
    }

    pub fn draw_elements(&mut self, mode: u32, count: i32, element_type: u32) {
        self.commands.push(GlesCommand::DrawElements {
            mode,
            count,
            element_type,
        });
    }

    pub fn draw_elements_indexed(
        &mut self,
        mode: u32,
        count: i32,
        element_type: u32,
        indices: &[u32],
    ) {
        self.commands.push(GlesCommand::DrawElements {
            mode,
            count,
            element_type,
        });
        let Some(pointer) = self.vertex_pointer.clone() else {
            return;
        };
        if count <= 0 || indices.is_empty() {
            return;
        }
        let mut vertices = Vec::with_capacity(count as usize);
        for index in indices.iter().copied().take(count as usize) {
            let offset = (index as usize).saturating_mul(pointer.stride);
            if offset.saturating_add(pointer.size) > pointer.values.len() {
                continue;
            }
            vertices.push(Vertex {
                x: pointer.values[offset],
                y: pointer.values[offset + 1],
                z: pointer.values.get(offset + 2).copied().unwrap_or(0.0),
                u: self.pointer_value(&self.texcoord_pointer, index as usize, 0),
                v: self.pointer_value(&self.texcoord_pointer, index as usize, 1),
                color: self.pointer_color(index as usize),
            });
        }
        self.draw(Self::primitive(mode), &vertices);
    }

    pub fn matrix_mode(&mut self, mode: u32) {
        if matches!(mode, MODELVIEW | PROJECTION | TEXTURE) {
            self.matrix_mode = mode;
            self.commands.push(GlesCommand::MatrixMode(mode));
        }
    }

    pub fn load_identity(&mut self) {
        *self.current_matrix_mut() = identity();
        self.commands.push(GlesCommand::LoadIdentity);
    }

    pub fn load_matrix(&mut self, matrix: [f32; 16]) {
        *self.current_matrix_mut() = matrix;
        self.commands.push(GlesCommand::LoadMatrix);
    }

    pub fn mult_matrix(&mut self, matrix: [f32; 16]) {
        let current = *self.current_matrix();
        *self.current_matrix_mut() = multiply(&current, &matrix);
        self.commands.push(GlesCommand::MultMatrix);
    }

    pub fn push_matrix(&mut self) {
        let matrix = *self.current_matrix();
        self.matrix_stacks
            .entry(self.matrix_mode)
            .or_default()
            .push(matrix);
        self.commands.push(GlesCommand::PushMatrix);
    }

    pub fn pop_matrix(&mut self) {
        if let Some(matrix) = self
            .matrix_stacks
            .get_mut(&self.matrix_mode)
            .and_then(Vec::pop)
        {
            *self.current_matrix_mut() = matrix;
        }
        self.commands.push(GlesCommand::PopMatrix);
    }

    pub fn translate(&mut self, x: f32, y: f32, z: f32) {
        self.mult_matrix(translation(x, y, z));
    }

    pub fn scale(&mut self, x: f32, y: f32, z: f32) {
        self.mult_matrix(scaling(x, y, z));
    }

    pub fn rotate(&mut self, angle: f32, x: f32, y: f32, z: f32) {
        self.mult_matrix(rotation(angle, x, y, z));
    }

    pub fn ortho(&mut self, left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) {
        self.mult_matrix(orthographic(left, right, bottom, top, near, far));
    }

    pub fn scissor(&mut self, x: i32, y: i32, width: u32, height: u32) {
        self.scissor = Some((x, y, width, height));
        self.commands.push(GlesCommand::Scissor {
            x,
            y,
            width,
            height,
        });
    }

    pub fn rendered_pixels(&self) -> usize {
        self.next_frame_pixels
    }

    pub fn reset_frame_stats(&mut self) {
        self.next_frame_pixels = 0;
    }

    pub fn flush(&mut self) {
        self.commands.push(GlesCommand::DrawArrays {
            mode: TRIANGLE_STRIP,
            first: 0,
            count: 0,
        });
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

    pub fn draw_quad_pixels(&mut self, x: f32, y: f32, width: f32, height: f32, color: Rgba8) {
        let (_, _, viewport_width, viewport_height) = self.viewport;
        if viewport_width == 0 || viewport_height == 0 {
            return;
        }
        let left = (x / viewport_width as f32) * 2.0 - 1.0;
        let right = ((x + width) / viewport_width as f32) * 2.0 - 1.0;
        let top = 1.0 - (y / viewport_height as f32) * 2.0;
        let bottom = 1.0 - ((y + height) / viewport_height as f32) * 2.0;
        self.draw(
            Primitive::TriangleStrip,
            &[
                Vertex {
                    x: left,
                    y: top,
                    z: 0.0,
                    u: 0.0,
                    v: 0.0,
                    color,
                },
                Vertex {
                    x: right,
                    y: top,
                    z: 0.0,
                    u: 1.0,
                    v: 0.0,
                    color,
                },
                Vertex {
                    x: right,
                    y: bottom,
                    z: 0.0,
                    u: 1.0,
                    v: 1.0,
                    color,
                },
                Vertex {
                    x: left,
                    y: bottom,
                    z: 0.0,
                    u: 0.0,
                    v: 1.0,
                    color,
                },
            ],
        );
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

    fn clear_color_buffer(&mut self) {
        if self.enabled.get(&SCISSOR_TEST).copied().unwrap_or(false) {
            if let Some((x, y, width, height)) = self.scissor {
                for row in y.max(0) as u32..y.max(0) as u32 + height {
                    for column in x.max(0) as u32..x.max(0) as u32 + width {
                        self.framebuffer.set_pixel(column, row, self.clear_color);
                    }
                }
                return;
            }
        }
        self.framebuffer.clear(self.clear_color);
    }

    fn primitive(mode: u32) -> Primitive {
        match mode {
            POINTS => Primitive::Points,
            LINES => Primitive::Lines,
            LINE_LOOP => Primitive::LineLoop,
            LINE_STRIP => Primitive::LineStrip,
            TRIANGLE_STRIP => Primitive::TriangleStrip,
            TRIANGLE_FAN => Primitive::TriangleFan,
            _ => Primitive::Triangles,
        }
    }

    fn current_matrix(&self) -> &[f32; 16] {
        match self.matrix_mode {
            PROJECTION => &self.projection_matrix,
            TEXTURE => &self.texture_matrix,
            _ => &self.modelview_matrix,
        }
    }

    fn current_matrix_mut(&mut self) -> &mut [f32; 16] {
        match self.matrix_mode {
            PROJECTION => &mut self.projection_matrix,
            TEXTURE => &mut self.texture_matrix,
            _ => &mut self.modelview_matrix,
        }
    }

    fn pointer_value(&self, pointer: &Option<Pointer>, index: usize, component: usize) -> f32 {
        let Some(pointer) = pointer else { return 0.0 };
        let offset = index
            .saturating_mul(pointer.stride)
            .saturating_add(component);
        pointer.values.get(offset).copied().unwrap_or(0.0)
    }

    fn pointer_color(&self, index: usize) -> Rgba8 {
        let Some(pointer) = self.color_pointer.as_ref() else {
            return self.current_color;
        };
        let r = self.pointer_value(&self.color_pointer, index, 0);
        let g = self.pointer_value(&self.color_pointer, index, 1);
        let b = self.pointer_value(&self.color_pointer, index, 2);
        let a = if pointer.size >= 4 {
            self.pointer_value(&self.color_pointer, index, 3)
        } else {
            1.0
        };
        Rgba8 {
            r: (r.clamp(0.0, 1.0) * 255.0) as u8,
            g: (g.clamp(0.0, 1.0) * 255.0) as u8,
            b: (b.clamp(0.0, 1.0) * 255.0) as u8,
            a: (a.clamp(0.0, 1.0) * 255.0) as u8,
        }
    }

    fn draw(&mut self, primitive: Primitive, vertices: &[Vertex]) {
        let command = GlesCommand::Draw {
            primitive,
            vertices: vertices.to_vec(),
        };
        self.rasterize(&command);
        self.commands.push(command);
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
            Primitive::LineLoop => {
                for pair in vertices.windows(2) {
                    self.line(pair[0], pair[1]);
                }
                if vertices.len() > 1 {
                    self.line(*vertices.last().unwrap(), vertices[0]);
                }
            }
            Primitive::LineStrip => {
                for pair in vertices.windows(2) {
                    self.line(pair[0], pair[1]);
                }
            }
            Primitive::Triangles => {
                for tri in vertices.chunks_exact(3) {
                    self.triangle(tri[0], tri[1], tri[2]);
                }
            }
            Primitive::TriangleStrip => {
                for (index, tri) in vertices.windows(3).enumerate() {
                    if index % 2 == 0 {
                        self.triangle(tri[0], tri[1], tri[2]);
                    } else {
                        self.triangle(tri[1], tri[0], tri[2]);
                    }
                }
            }
            Primitive::TriangleFan => {
                if let Some(first) = vertices.first().copied() {
                    for pair in vertices[1..].windows(2) {
                        self.triangle(first, pair[0], pair[1]);
                    }
                }
            }
        }
    }

    fn project(&self, vertex: Vertex) -> ProjectedVertex {
        let model = multiply(&self.projection_matrix, &self.modelview_matrix);
        let clip = transform(&model, vertex.x, vertex.y, vertex.z, 1.0);
        let w = if clip[3].abs() < 0.000001 {
            1.0
        } else {
            clip[3]
        };
        let ndc_x = clip[0] / w;
        let ndc_y = clip[1] / w;
        let ndc_z = clip[2] / w;
        let (viewport_x, viewport_y, width, height) = self.viewport;
        ProjectedVertex {
            x: viewport_x + ((ndc_x + 1.0) * 0.5 * width as f32) as i32,
            y: viewport_y + ((1.0 - ndc_y) * 0.5 * height as f32) as i32,
            z: (ndc_z + 1.0) * 0.5,
        }
    }

    fn plot(&mut self, vertex: &Vertex) {
        let projected = self.project(*vertex);
        self.write_fragment(
            projected.x,
            projected.y,
            projected.z,
            vertex.color,
            vertex.u,
            vertex.v,
        );
    }

    fn line(&mut self, a: Vertex, b: Vertex) {
        let first = self.project(a);
        let last = self.project(b);
        let steps = (last.x - first.x)
            .abs()
            .max((last.y - first.y).abs())
            .max(1);
        for step in 0..=steps {
            let t = step as f32 / steps as f32;
            let color = interpolate_color(a.color, b.color, t);
            let u = a.u + (b.u - a.u) * t;
            let v = a.v + (b.v - a.v) * t;
            self.write_fragment(
                first.x + ((last.x - first.x) as f32 * t) as i32,
                first.y + ((last.y - first.y) as f32 * t) as i32,
                first.z + (last.z - first.z) * t,
                color,
                u,
                v,
            );
        }
    }

    fn triangle(&mut self, a: Vertex, b: Vertex, c: Vertex) {
        let pa = self.project(a);
        let pb = self.project(b);
        let pc = self.project(c);
        let min_x = pa.x.min(pb.x).min(pc.x).max(0);
        let max_x =
            pa.x.max(pb.x)
                .max(pc.x)
                .min(self.framebuffer.width().saturating_sub(1) as i32);
        let min_y = pa.y.min(pb.y).min(pc.y).max(0);
        let max_y =
            pa.y.max(pb.y)
                .max(pc.y)
                .min(self.framebuffer.height().saturating_sub(1) as i32);
        if min_x > max_x || min_y > max_y {
            return;
        }
        let area = edge(pa.x, pa.y, pb.x, pb.y, pc.x, pc.y) as f32;
        if area == 0.0 {
            return;
        }
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let wa = edge(pb.x, pb.y, pc.x, pc.y, x, y) as f32 / area;
                let wb = edge(pc.x, pc.y, pa.x, pa.y, x, y) as f32 / area;
                let wc = edge(pa.x, pa.y, pb.x, pb.y, x, y) as f32 / area;
                if (wa >= 0.0 && wb >= 0.0 && wc >= 0.0) || (wa <= 0.0 && wb <= 0.0 && wc <= 0.0) {
                    let color = barycentric_color(a.color, b.color, c.color, wa, wb, wc);
                    let u = a.u * wa + b.u * wb + c.u * wc;
                    let v = a.v * wa + b.v * wb + c.v * wc;
                    let z = pa.z * wa + pb.z * wb + pc.z * wc;
                    self.write_fragment(x, y, z, color, u, v);
                }
            }
        }
    }

    fn write_fragment(&mut self, x: i32, y: i32, depth: f32, source: Rgba8, u: f32, v: f32) {
        if x < 0
            || y < 0
            || x >= self.framebuffer.width() as i32
            || y >= self.framebuffer.height() as i32
        {
            return;
        }
        if self.enabled.get(&SCISSOR_TEST).copied().unwrap_or(false) {
            let Some((left, top, width, height)) = self.scissor else {
                return;
            };
            if x < left
                || y < top
                || x >= left.saturating_add(width as i32)
                || y >= top.saturating_add(height as i32)
            {
                return;
            }
        }
        let index = y as usize * self.framebuffer.width() as usize + x as usize;
        if self.enabled.get(&DEPTH_TEST).copied().unwrap_or(false) {
            if depth >= self.depth_buffer[index] {
                return;
            }
            self.depth_buffer[index] = depth;
        }
        let textured = self.enabled.get(&TEXTURE_2D).copied().unwrap_or(false);
        let source = if textured {
            modulate(source, self.sample_texture(u, v))
        } else {
            source
        };
        let destination = self.framebuffer.pixel(x as u32, y as u32).unwrap_or(Rgba8 {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        });
        let color = if self.enabled.get(&BLEND).copied().unwrap_or(false) {
            blend_colors(source, destination, self.blend_src, self.blend_dst)
        } else {
            source
        };
        if self.framebuffer.set_pixel(x as u32, y as u32, color) {
            self.next_frame_pixels = self.next_frame_pixels.saturating_add(1);
        }
    }

    fn sample_texture(&self, u: f32, v: f32) -> Rgba8 {
        let Some(texture) = self.textures.get(&self.bound_texture) else {
            return Rgba8 {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            };
        };
        if texture.width == 0 || texture.height == 0 || texture.pixels.is_empty() {
            return Rgba8 {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            };
        }
        let u = u.rem_euclid(1.0);
        let v = v.rem_euclid(1.0);
        let x = (u * texture.width as f32).floor() as u32;
        let y = (v * texture.height as f32).floor() as u32;
        texture.pixels
            [(y.min(texture.height - 1) * texture.width + x.min(texture.width - 1)) as usize]
    }
}

fn stride_in_values(size: usize, stride: usize) -> usize {
    if stride == 0 {
        size
    } else if stride & 3 == 0 {
        (stride / 4).max(size)
    } else {
        stride.max(size)
    }
}

fn white_texture() -> TextureImage {
    TextureImage {
        width: 1,
        height: 1,
        pixels: vec![Rgba8 {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        }],
    }
}

fn identity() -> [f32; 16] {
    [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

fn multiply(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut result = [0.0; 16];
    for column in 0..4 {
        for row in 0..4 {
            result[column * 4 + row] = (0..4)
                .map(|index| a[index * 4 + row] * b[column * 4 + index])
                .sum();
        }
    }
    result
}

fn transform(matrix: &[f32; 16], x: f32, y: f32, z: f32, w: f32) -> [f32; 4] {
    [
        matrix[0] * x + matrix[4] * y + matrix[8] * z + matrix[12] * w,
        matrix[1] * x + matrix[5] * y + matrix[9] * z + matrix[13] * w,
        matrix[2] * x + matrix[6] * y + matrix[10] * z + matrix[14] * w,
        matrix[3] * x + matrix[7] * y + matrix[11] * z + matrix[15] * w,
    ]
}

fn translation(x: f32, y: f32, z: f32) -> [f32; 16] {
    let mut matrix = identity();
    matrix[12] = x;
    matrix[13] = y;
    matrix[14] = z;
    matrix
}

fn scaling(x: f32, y: f32, z: f32) -> [f32; 16] {
    let mut matrix = identity();
    matrix[0] = x;
    matrix[5] = y;
    matrix[10] = z;
    matrix
}

fn rotation(angle: f32, x: f32, y: f32, z: f32) -> [f32; 16] {
    let length = (x * x + y * y + z * z).sqrt();
    if length == 0.0 {
        return identity();
    }
    let (x, y, z) = (x / length, y / length, z / length);
    let radians = angle.to_radians();
    let (sin, cos) = radians.sin_cos();
    let one = 1.0 - cos;
    [
        x * x * one + cos,
        y * x * one + z * sin,
        x * z * one - y * sin,
        0.0,
        x * y * one - z * sin,
        y * y * one + cos,
        y * z * one + x * sin,
        0.0,
        x * z * one + y * sin,
        y * z * one - x * sin,
        z * z * one + cos,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

fn orthographic(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> [f32; 16] {
    let mut matrix = identity();
    if right != left {
        matrix[0] = 2.0 / (right - left);
        matrix[12] = -(right + left) / (right - left);
    }
    if top != bottom {
        matrix[5] = 2.0 / (top - bottom);
        matrix[13] = -(top + bottom) / (top - bottom);
    }
    if far != near {
        matrix[10] = -2.0 / (far - near);
        matrix[14] = -(far + near) / (far - near);
    }
    matrix
}

fn interpolate_color(a: Rgba8, b: Rgba8, t: f32) -> Rgba8 {
    Rgba8 {
        r: (a.r as f32 + (b.r as f32 - a.r as f32) * t).clamp(0.0, 255.0) as u8,
        g: (a.g as f32 + (b.g as f32 - a.g as f32) * t).clamp(0.0, 255.0) as u8,
        b: (a.b as f32 + (b.b as f32 - a.b as f32) * t).clamp(0.0, 255.0) as u8,
        a: (a.a as f32 + (b.a as f32 - a.a as f32) * t).clamp(0.0, 255.0) as u8,
    }
}

fn barycentric_color(a: Rgba8, b: Rgba8, c: Rgba8, wa: f32, wb: f32, wc: f32) -> Rgba8 {
    Rgba8 {
        r: (a.r as f32 * wa + b.r as f32 * wb + c.r as f32 * wc).clamp(0.0, 255.0) as u8,
        g: (a.g as f32 * wa + b.g as f32 * wb + c.g as f32 * wc).clamp(0.0, 255.0) as u8,
        b: (a.b as f32 * wa + b.b as f32 * wb + c.b as f32 * wc).clamp(0.0, 255.0) as u8,
        a: (a.a as f32 * wa + b.a as f32 * wb + c.a as f32 * wc).clamp(0.0, 255.0) as u8,
    }
}

fn modulate(a: Rgba8, b: Rgba8) -> Rgba8 {
    Rgba8 {
        r: ((a.r as u16 * b.r as u16) / 255) as u8,
        g: ((a.g as u16 * b.g as u16) / 255) as u8,
        b: ((a.b as u16 * b.b as u16) / 255) as u8,
        a: ((a.a as u16 * b.a as u16) / 255) as u8,
    }
}

fn blend_colors(
    source: Rgba8,
    destination: Rgba8,
    source_factor: u32,
    destination_factor: u32,
) -> Rgba8 {
    let source_alpha = source.a as f32 / 255.0;
    let destination_alpha = destination.a as f32 / 255.0;
    let source_factor = blend_factor(
        source_factor,
        source,
        destination,
        source_alpha,
        destination_alpha,
    );
    let destination_factor = blend_factor(
        destination_factor,
        source,
        destination,
        source_alpha,
        destination_alpha,
    );
    Rgba8 {
        r: (source.r as f32 * source_factor + destination.r as f32 * destination_factor)
            .clamp(0.0, 255.0) as u8,
        g: (source.g as f32 * source_factor + destination.g as f32 * destination_factor)
            .clamp(0.0, 255.0) as u8,
        b: (source.b as f32 * source_factor + destination.b as f32 * destination_factor)
            .clamp(0.0, 255.0) as u8,
        a: (source.a as f32 * source_factor + destination.a as f32 * destination_factor)
            .clamp(0.0, 255.0) as u8,
    }
}

fn blend_factor(
    factor: u32,
    source: Rgba8,
    destination: Rgba8,
    source_alpha: f32,
    destination_alpha: f32,
) -> f32 {
    match factor {
        ZERO => 0.0,
        ONE => 1.0,
        SRC_COLOR => source.r as f32 / 255.0,
        ONE_MINUS_SRC_COLOR => 1.0 - source.r as f32 / 255.0,
        SRC_ALPHA => source_alpha,
        ONE_MINUS_SRC_ALPHA => 1.0 - source_alpha,
        DST_COLOR => destination.r as f32 / 255.0,
        ONE_MINUS_DST_COLOR => 1.0 - destination.r as f32 / 255.0,
        DST_ALPHA => destination_alpha,
        ONE_MINUS_DST_ALPHA => 1.0 - destination_alpha,
        _ => 1.0,
    }
}

fn edge(ax: i32, ay: i32, bx: i32, by: i32, cx: i32, cy: i32) -> i32 {
    (cx - ax) * (by - ay) - (cy - ay) * (bx - ax)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orthographic_matrix_maps_pixel_coordinates() {
        let mut gles = GlesContext::new(VirtualScreen {
            width: 10,
            height: 10,
        });
        gles.matrix_mode(PROJECTION);
        gles.load_identity();
        gles.ortho(0.0, 10.0, 0.0, 10.0, -1.0, 1.0);
        gles.matrix_mode(MODELVIEW);
        gles.load_identity();
        gles.draw(
            Primitive::TriangleStrip,
            &[
                Vertex {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    u: 0.0,
                    v: 0.0,
                    color: Rgba8 {
                        r: 255,
                        g: 0,
                        b: 0,
                        a: 255,
                    },
                },
                Vertex {
                    x: 2.0,
                    y: 0.0,
                    z: 0.0,
                    u: 1.0,
                    v: 0.0,
                    color: Rgba8 {
                        r: 255,
                        g: 0,
                        b: 0,
                        a: 255,
                    },
                },
                Vertex {
                    x: 2.0,
                    y: 2.0,
                    z: 0.0,
                    u: 1.0,
                    v: 1.0,
                    color: Rgba8 {
                        r: 255,
                        g: 0,
                        b: 0,
                        a: 255,
                    },
                },
                Vertex {
                    x: 0.0,
                    y: 2.0,
                    z: 0.0,
                    u: 0.0,
                    v: 1.0,
                    color: Rgba8 {
                        r: 255,
                        g: 0,
                        b: 0,
                        a: 255,
                    },
                },
            ],
        );
        assert_eq!(gles.framebuffer().pixel(1, 8).unwrap().r, 255);
    }

    #[test]
    fn blend_and_scissor_are_applied() {
        let mut gles = GlesContext::new(VirtualScreen {
            width: 4,
            height: 4,
        });
        gles.clear_color(Rgba8 {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        });
        gles.clear_mask(COLOR_BUFFER_BIT);
        gles.scissor(0, 0, 2, 2);
        gles.enable(SCISSOR_TEST);
        gles.clear_color(Rgba8 {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        });
        gles.clear_mask(COLOR_BUFFER_BIT);
        assert_eq!(gles.framebuffer().pixel(1, 1).unwrap().r, 255);
        assert_eq!(gles.framebuffer().pixel(3, 3).unwrap().r, 0);
    }
}
