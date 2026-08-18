/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::ops::{Deref, DerefMut};

use crate::gles::GlesContext;
use crate::VirtualScreen;

pub const GL_FIXED: u32 = 0x140C;
pub const GL_FLOAT: u32 = 0x1406;
pub const GL_UNSIGNED_BYTE: u32 = 0x1401;
pub const GL_UNSIGNED_SHORT: u32 = 0x1403;
pub const GL_UNSIGNED_INT: u32 = 0x1405;
pub const GL_VERTEX_ARRAY: u32 = 0x8074;
pub const GL_COLOR_ARRAY: u32 = 0x8076;
pub const GL_TEXTURE_COORD_ARRAY: u32 = 0x8078;
pub const GL_MATRIX_PALETTE_OES: u32 = 0x8840;
pub const GL_MATRIX_INDEX_ARRAY_OES: u32 = 0x8844;
pub const GL_WEIGHT_ARRAY_OES: u32 = 0x86AD;
pub const GL_MODELVIEW: u32 = 0x1700;
pub const GL_PROJECTION: u32 = 0x1701;
pub const GL_TEXTURE: u32 = 0x1702;
pub const GL_ARRAY_BUFFER: u32 = 0x8892;
pub const GL_ELEMENT_ARRAY_BUFFER: u32 = 0x8893;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientArray {
    Vertex,
    Color,
    TexCoord,
}

#[derive(Debug, Clone)]
struct ArrayPointer {
    size: usize,
    stride: usize,
    values: Vec<f32>,
    enabled: bool,
}

impl ArrayPointer {
    fn new(size: usize, stride: usize, values: Vec<f32>) -> Self {
        let size = size.clamp(1, 4);
        Self {
            size,
            stride: if stride == 0 { size } else { stride.max(size) },
            values,
            enabled: false,
        }
    }

    fn value(&self, index: usize, component: usize) -> f32 {
        let offset = index.saturating_mul(self.stride).saturating_add(component);
        self.values.get(offset).copied().unwrap_or(0.0)
    }
}

#[derive(Debug)]
pub struct Gles1OnGl2 {
    renderer: GlesContext,
    vertex: Option<ArrayPointer>,
    color: Option<ArrayPointer>,
    texcoord: Option<ArrayPointer>,
    palette_weights: Option<ArrayPointer>,
    palette_indices: Option<ArrayPointer>,
    palette_matrices: Vec<[f32; 16]>,
    current_palette_matrix: usize,
    matrix_palette_enabled: bool,
    matrix_mode: u32,
    modelview: [f32; 16],
    projection: [f32; 16],
    texture: [f32; 16],
    modelview_stack: Vec<[f32; 16]>,
    projection_stack: Vec<[f32; 16]>,
    texture_stack: Vec<[f32; 16]>,
}

impl Default for Gles1OnGl2 {
    fn default() -> Self {
        Self::new(VirtualScreen::default())
    }
}

impl Deref for Gles1OnGl2 {
    type Target = GlesContext;

    fn deref(&self) -> &Self::Target {
        &self.renderer
    }
}

impl DerefMut for Gles1OnGl2 {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.renderer
    }
}

impl Gles1OnGl2 {
    pub fn new(screen: VirtualScreen) -> Self {
        Self {
            renderer: GlesContext::new(screen),
            vertex: None,
            color: None,
            texcoord: None,
            palette_weights: None,
            palette_indices: None,
            palette_matrices: vec![identity(); 9],
            current_palette_matrix: 0,
            matrix_palette_enabled: false,
            matrix_mode: GL_MODELVIEW,
            modelview: identity(),
            projection: identity(),
            texture: identity(),
            modelview_stack: Vec::new(),
            projection_stack: Vec::new(),
            texture_stack: Vec::new(),
        }
    }

    pub fn renderer(&self) -> &GlesContext {
        &self.renderer
    }

    pub fn renderer_mut(&mut self) -> &mut GlesContext {
        &mut self.renderer
    }

    pub fn gl2_capability(capability: u32) -> Option<u32> {
        match capability {
            GL_MATRIX_PALETTE_OES | GL_MATRIX_INDEX_ARRAY_OES | GL_WEIGHT_ARRAY_OES => None,
            GL_FIXED => None,
            other => Some(other),
        }
    }

    pub fn enable_client_state(&mut self, array: ClientArray) {
        self.array_mut(array).enabled = true;
    }

    pub fn disable_client_state(&mut self, array: ClientArray) {
        self.array_mut(array).enabled = false;
    }

    pub fn enable(&mut self, capability: u32) {
        if capability == GL_MATRIX_PALETTE_OES {
            self.matrix_palette_enabled = true;
        } else {
            self.renderer
                .enable(Self::gl2_capability(capability).unwrap_or(capability));
        }
    }

    pub fn disable(&mut self, capability: u32) {
        if capability == GL_MATRIX_PALETTE_OES {
            self.matrix_palette_enabled = false;
        } else {
            self.renderer
                .disable(Self::gl2_capability(capability).unwrap_or(capability));
        }
    }

    pub fn is_enabled(&self, capability: u32) -> bool {
        if capability == GL_MATRIX_PALETTE_OES {
            self.matrix_palette_enabled
        } else {
            self.renderer
                .commands()
                .iter()
                .rev()
                .find_map(|command| match command {
                    crate::gles::GlesCommand::Enable(value) if *value == capability => Some(true),
                    crate::gles::GlesCommand::Disable(value) if *value == capability => Some(false),
                    _ => None,
                })
                .unwrap_or(false)
        }
    }

    pub fn vertex_pointer(&mut self, size: usize, type_: u32, stride: usize, values: &[u8]) {
        let size = size.clamp(2, 4);
        let values = decode_components(size, type_, values, false);
        let enabled = self
            .vertex
            .as_ref()
            .map(|pointer| pointer.enabled)
            .unwrap_or(false);
        let mut pointer = ArrayPointer::new(size, stride / element_size(type_), values);
        pointer.enabled = enabled;
        self.vertex = Some(pointer);
    }

    pub fn color_pointer(&mut self, size: usize, type_: u32, stride: usize, values: &[u8]) {
        let size = size.clamp(3, 4);
        let values = decode_components(size, type_, values, true);
        let mut pointer = ArrayPointer::new(size, stride / element_size(type_), values);
        pointer.size = size;
        pointer.enabled = self
            .color
            .as_ref()
            .map(|value| value.enabled)
            .unwrap_or(false);
        self.color = Some(pointer);
    }

    pub fn texcoord_pointer(&mut self, size: usize, type_: u32, stride: usize, values: &[u8]) {
        let size = size.clamp(2, 4);
        let values = decode_components(size, type_, values, false);
        let enabled = self
            .texcoord
            .as_ref()
            .map(|pointer| pointer.enabled)
            .unwrap_or(false);
        let mut pointer = ArrayPointer::new(size, stride / element_size(type_), values);
        pointer.enabled = enabled;
        self.texcoord = Some(pointer);
    }

    pub fn vertex_pointer_fixed(&mut self, size: usize, stride: usize, values: &[i32]) {
        let size = size.clamp(2, 4);
        let enabled = self
            .vertex
            .as_ref()
            .map(|pointer| pointer.enabled)
            .unwrap_or(false);
        let mut pointer = ArrayPointer::new(
            size,
            stride_in_values(size, stride, 4),
            values.iter().map(|value| fixed_to_float(*value)).collect(),
        );
        pointer.enabled = enabled;
        self.vertex = Some(pointer);
    }

    pub fn color_pointer_fixed(&mut self, size: usize, stride: usize, values: &[i32]) {
        let size = size.clamp(3, 4);
        let mut pointer = ArrayPointer::new(
            size,
            stride_in_values(size, stride, 4),
            values.iter().map(|value| fixed_to_float(*value)).collect(),
        );
        pointer.size = size.clamp(3, 4);
        pointer.enabled = self
            .color
            .as_ref()
            .map(|value| value.enabled)
            .unwrap_or(false);
        self.color = Some(pointer);
    }

    pub fn texcoord_pointer_fixed(&mut self, size: usize, stride: usize, values: &[i32]) {
        let size = size.clamp(2, 4);
        let enabled = self
            .texcoord
            .as_ref()
            .map(|pointer| pointer.enabled)
            .unwrap_or(false);
        let mut pointer = ArrayPointer::new(
            size,
            stride_in_values(size, stride, 4),
            values.iter().map(|value| fixed_to_float(*value)).collect(),
        );
        pointer.enabled = enabled;
        self.texcoord = Some(pointer);
    }

    pub fn weight_pointer(&mut self, size: usize, type_: u32, stride: usize, values: &[u8]) {
        self.palette_weights = Some(ArrayPointer::new(
            size,
            stride / element_size(type_),
            decode_components(size, type_, values, true),
        ));
    }

    pub fn enable_weight_array(&mut self) {
        if let Some(pointer) = self.palette_weights.as_mut() {
            pointer.enabled = true;
        }
    }

    pub fn disable_weight_array(&mut self) {
        if let Some(pointer) = self.palette_weights.as_mut() {
            pointer.enabled = false;
        }
    }

    pub fn matrix_index_pointer(&mut self, size: usize, type_: u32, stride: usize, values: &[u8]) {
        self.palette_indices = Some(ArrayPointer::new(
            size,
            stride / element_size(type_),
            decode_components(size, type_, values, false),
        ));
    }

    pub fn enable_matrix_index_array(&mut self) {
        if let Some(pointer) = self.palette_indices.as_mut() {
            pointer.enabled = true;
        }
    }

    pub fn disable_matrix_index_array(&mut self) {
        if let Some(pointer) = self.palette_indices.as_mut() {
            pointer.enabled = false;
        }
    }

    pub fn draw_arrays(&mut self, mode: u32, first: i32, count: i32) {
        if self.matrix_palette_active() {
            if let Some(vertices) = self.skinned_vertices(first, count) {
                self.draw_skinned(mode, vertices.0, vertices.1);
                return;
            }
        }
        self.upload_client_arrays(first, count);
        self.renderer.draw_arrays(mode, 0, count);
    }

    pub fn draw_elements(&mut self, mode: u32, count: i32, element_type: u32) {
        self.renderer.draw_elements(mode, count, element_type);
    }

    pub fn draw_elements_indexed(
        &mut self,
        mode: u32,
        count: i32,
        element_type: u32,
        indices: &[u32],
    ) {
        if self.matrix_palette_active() {
            if let Some(vertices) = self.skinned_indexed_vertices(count, indices) {
                self.draw_skinned(mode, vertices.0, vertices.1);
                return;
            }
        }
        self.upload_client_arrays(
            0,
            Self::max_index(indices, count).map_or(0, |value| value + 1) as i32,
        );
        self.renderer
            .draw_elements_indexed(mode, count, element_type, indices);
    }

    pub fn matrix_mode(&mut self, mode: u32) {
        if matches!(mode, GL_MODELVIEW | GL_PROJECTION | GL_TEXTURE) {
            self.matrix_mode = mode;
            self.renderer.matrix_mode(mode);
        }
    }

    pub fn load_identity(&mut self) {
        *self.current_matrix_mut() = identity();
        self.renderer.load_identity();
    }

    pub fn load_matrix_f(&mut self, matrix: &[f32; 16]) {
        *self.current_matrix_mut() = *matrix;
        self.renderer.load_matrix(*matrix);
    }

    pub fn load_matrix_x(&mut self, matrix: &[i32; 16]) {
        let matrix = matrix.map(fixed_to_float);
        self.load_matrix_f(&matrix);
    }

    pub fn mult_matrix_f(&mut self, matrix: &[f32; 16]) {
        let current = *self.current_matrix();
        *self.current_matrix_mut() = multiply(&current, matrix);
        self.renderer.mult_matrix(*matrix);
    }

    pub fn mult_matrix_x(&mut self, matrix: &[i32; 16]) {
        let matrix = matrix.map(fixed_to_float);
        self.mult_matrix_f(&matrix);
    }

    pub fn push_matrix(&mut self) {
        let matrix = *self.current_matrix();
        self.matrix_stack_mut().push(matrix);
        self.renderer.push_matrix();
    }

    pub fn pop_matrix(&mut self) {
        if let Some(matrix) = self.matrix_stack_mut().pop() {
            *self.current_matrix_mut() = matrix;
        }
        self.renderer.pop_matrix();
    }

    pub fn translate_f(&mut self, x: f32, y: f32, z: f32) {
        let matrix = translation(x, y, z);
        self.mult_matrix_f(&matrix);
    }

    pub fn scale_f(&mut self, x: f32, y: f32, z: f32) {
        let matrix = scaling(x, y, z);
        self.mult_matrix_f(&matrix);
    }

    pub fn current_palette_matrix(&mut self, index: usize) {
        self.current_palette_matrix = index.min(self.palette_matrices.len() - 1);
    }

    pub fn load_palette_from_modelview(&mut self) {
        self.palette_matrices[self.current_palette_matrix] = self.modelview;
    }

    pub fn load_palette_matrix_f(&mut self, matrix: &[f32; 16]) {
        self.palette_matrices[self.current_palette_matrix] = *matrix;
    }

    pub fn load_palette_matrix_x(&mut self, matrix: &[i32; 16]) {
        self.load_palette_matrix_f(&matrix.map(fixed_to_float));
    }

    pub fn framebuffer(&self) -> &crate::Framebuffer {
        self.renderer.framebuffer()
    }

    pub fn into_renderer(self) -> GlesContext {
        self.renderer
    }

    fn array_mut(&mut self, array: ClientArray) -> &mut ArrayPointer {
        let target = match array {
            ClientArray::Vertex => &mut self.vertex,
            ClientArray::Color => &mut self.color,
            ClientArray::TexCoord => &mut self.texcoord,
        };
        target.get_or_insert_with(|| ArrayPointer::new(2, 2, Vec::new()))
    }

    fn current_matrix(&self) -> &[f32; 16] {
        match self.matrix_mode {
            GL_PROJECTION => &self.projection,
            GL_TEXTURE => &self.texture,
            _ => &self.modelview,
        }
    }

    fn current_matrix_mut(&mut self) -> &mut [f32; 16] {
        match self.matrix_mode {
            GL_PROJECTION => &mut self.projection,
            GL_TEXTURE => &mut self.texture,
            _ => &mut self.modelview,
        }
    }

    fn matrix_stack_mut(&mut self) -> &mut Vec<[f32; 16]> {
        match self.matrix_mode {
            GL_PROJECTION => &mut self.projection_stack,
            GL_TEXTURE => &mut self.texture_stack,
            _ => &mut self.modelview_stack,
        }
    }

    fn matrix_palette_active(&self) -> bool {
        self.matrix_palette_enabled
            && self
                .palette_weights
                .as_ref()
                .is_some_and(|pointer| pointer.enabled)
            && self
                .palette_indices
                .as_ref()
                .is_some_and(|pointer| pointer.enabled)
    }

    fn upload_client_arrays(&mut self, first: i32, count: i32) {
        let Some(vertex) = self.vertex.as_ref() else {
            return;
        };
        if !vertex.enabled || first < 0 || count <= 0 {
            return;
        }
        let first = first as usize;
        let count = count as usize;
        let mut positions = Vec::with_capacity(count * vertex.size);
        let mut colors = Vec::with_capacity(count * 4);
        let mut texcoords = Vec::with_capacity(count * 2);
        for index in first..first.saturating_add(count) {
            for component in 0..vertex.size {
                positions.push(vertex.value(index, component));
            }
            if let Some(color) = self.color.as_ref().filter(|pointer| pointer.enabled) {
                for component in 0..color.size {
                    colors.push(color.value(index, component));
                }
            }
            if let Some(texcoord) = self.texcoord.as_ref().filter(|pointer| pointer.enabled) {
                texcoords.push(texcoord.value(index, 0));
                texcoords.push(texcoord.value(index, 1));
            }
        }
        self.renderer.set_vertex_pointer(vertex.size, 0, positions);
        if !colors.is_empty() {
            self.renderer.set_color_pointer(4, 0, colors);
        }
        if !texcoords.is_empty() {
            self.renderer.set_texcoord_pointer(2, 0, texcoords);
        }
    }

    fn skinned_vertices(&self, first: i32, count: i32) -> Option<(Vec<f32>, usize)> {
        if first < 0 || count <= 0 {
            return None;
        }
        let vertex = self.vertex.as_ref()?.clone();
        let weights = self.palette_weights.as_ref()?;
        let indices = self.palette_indices.as_ref()?;
        let mut output = Vec::with_capacity(count as usize * vertex.size);
        for index in first as usize..first as usize + count as usize {
            output.extend_from_slice(&self.skin_one(&vertex, weights, indices, index)?);
        }
        Some((output, vertex.size))
    }

    fn skinned_indexed_vertices(&self, count: i32, indices: &[u32]) -> Option<(Vec<f32>, usize)> {
        if count <= 0 {
            return None;
        }
        let vertex = self.vertex.as_ref()?.clone();
        let weights = self.palette_weights.as_ref()?;
        let matrix_indices = self.palette_indices.as_ref()?;
        let mut output = Vec::with_capacity(count as usize * vertex.size);
        for index in indices.iter().copied().take(count as usize) {
            output.extend_from_slice(&self.skin_one(
                &vertex,
                weights,
                matrix_indices,
                index as usize,
            )?);
        }
        Some((output, vertex.size))
    }

    fn skin_one(
        &self,
        vertex: &ArrayPointer,
        weights: &ArrayPointer,
        indices: &ArrayPointer,
        index: usize,
    ) -> Option<Vec<f32>> {
        let mut object = [0.0, 0.0, 0.0, 1.0];
        for (component, value) in object.iter_mut().enumerate().take(vertex.size.min(4)) {
            *value = vertex.value(index, component);
        }
        let units = weights.size.min(indices.size).min(4);
        if units == 0 {
            return None;
        }
        let mut result = [0.0; 4];
        for unit in 0..units {
            let weight = weights.value(index, unit);
            let palette_index = indices.value(index, unit).round() as isize;
            let palette_index =
                palette_index.clamp(0, self.palette_matrices.len() as isize - 1) as usize;
            let transformed = transform(&self.palette_matrices[palette_index], object);
            for component in 0..4 {
                result[component] += weight * transformed[component];
            }
        }
        Some(result[..vertex.size].to_vec())
    }

    fn draw_skinned(&mut self, mode: u32, vertices: Vec<f32>, size: usize) {
        let old_modelview = self.modelview;
        let count = vertices.len() / size.max(1);
        self.renderer.matrix_mode(GL_MODELVIEW);
        self.renderer.load_identity();
        self.renderer.set_vertex_pointer(size, 0, vertices);
        self.renderer.draw_arrays(mode, 0, count as i32);
        self.modelview = old_modelview;
        self.renderer.load_matrix(old_modelview);
    }

    fn max_index(indices: &[u32], count: i32) -> Option<usize> {
        indices
            .iter()
            .copied()
            .take(count.max(0) as usize)
            .max()
            .map(|value| value as usize)
    }
}

fn fixed_to_float(value: i32) -> f32 {
    value as f32 / 65536.0
}

fn element_size(type_: u32) -> usize {
    match type_ {
        GL_UNSIGNED_BYTE => 1,
        GL_UNSIGNED_SHORT => 2,
        _ => 4,
    }
}

fn stride_in_values(size: usize, stride: usize, bytes_per_component: usize) -> usize {
    if stride == 0 {
        size
    } else {
        (stride / bytes_per_component).max(size)
    }
}

fn decode_components(size: usize, type_: u32, bytes: &[u8], normalized: bool) -> Vec<f32> {
    let component_size = element_size(type_);
    let mut output = Vec::with_capacity(bytes.len() / component_size);
    for chunk in bytes.chunks_exact(component_size) {
        let value = match type_ {
            GL_FIXED => {
                fixed_to_float(i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            }
            GL_FLOAT => f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]),
            GL_UNSIGNED_BYTE => {
                let value = chunk[0] as f32;
                if normalized {
                    value / 255.0
                } else {
                    value
                }
            }
            GL_UNSIGNED_SHORT => {
                let value = u16::from_le_bytes([chunk[0], chunk[1]]) as f32;
                if normalized {
                    value / 65535.0
                } else {
                    value
                }
            }
            GL_UNSIGNED_INT => u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as f32,
            _ => 0.0,
        };
        output.push(value);
    }
    let _ = size;
    output
}

fn identity() -> [f32; 16] {
    [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

fn multiply(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut output = [0.0; 16];
    for column in 0..4 {
        for row in 0..4 {
            for index in 0..4 {
                output[column * 4 + row] += a[index * 4 + row] * b[column * 4 + index];
            }
        }
    }
    output
}

fn transform(matrix: &[f32; 16], vector: [f32; 4]) -> [f32; 4] {
    [
        matrix[0] * vector[0]
            + matrix[4] * vector[1]
            + matrix[8] * vector[2]
            + matrix[12] * vector[3],
        matrix[1] * vector[0]
            + matrix[5] * vector[1]
            + matrix[9] * vector[2]
            + matrix[13] * vector[3],
        matrix[2] * vector[0]
            + matrix[6] * vector[1]
            + matrix[10] * vector[2]
            + matrix[14] * vector[3],
        matrix[3] * vector[0]
            + matrix[7] * vector[1]
            + matrix[11] * vector[2]
            + matrix[15] * vector[3],
    ]
}

fn translation(x: f32, y: f32, z: f32) -> [f32; 16] {
    let mut output = identity();
    output[12] = x;
    output[13] = y;
    output[14] = z;
    output
}

fn scaling(x: f32, y: f32, z: f32) -> [f32; 16] {
    let mut output = identity();
    output[0] = x;
    output[5] = y;
    output[10] = z;
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_point_vertex_arrays_are_translated_before_draw() {
        let mut context = Gles1OnGl2::new(VirtualScreen {
            width: 16,
            height: 16,
        });
        context.matrix_mode(GL_PROJECTION);
        context.load_identity();
        context.matrix_mode(GL_MODELVIEW);
        context.load_identity();
        context.vertex_pointer_fixed(2, 0, &[0, 0, 65536, 0, 0, 65536]);
        context.enable_client_state(ClientArray::Vertex);
        context.draw_arrays(0x0004, 0, 3);
        assert!(context.commands().iter().any(|command| matches!(
            command,
            crate::gles::GlesCommand::Draw { vertices, .. }
                if vertices.iter().any(|vertex| vertex.x == 1.0)
        )));
    }

    #[test]
    fn gl2_does_not_forward_oes_palette_caps() {
        assert_eq!(Gles1OnGl2::gl2_capability(GL_MATRIX_PALETTE_OES), None);
        assert_eq!(Gles1OnGl2::gl2_capability(0x0BE2), Some(0x0BE2));
    }

    #[test]
    fn palette_skinning_uses_fixed_point_weights_and_indices() {
        let mut context = Gles1OnGl2::new(VirtualScreen {
            width: 16,
            height: 16,
        });
        context.vertex_pointer_fixed(3, 0, &[0, 0, 0]);
        context.enable_client_state(ClientArray::Vertex);
        context.weight_pointer(1, GL_FIXED, 0, &65536i32.to_le_bytes());
        context.matrix_index_pointer(1, GL_UNSIGNED_BYTE, 0, &[0]);
        context.palette_matrices[0][12] = 1.0;
        context.enable_client_state(ClientArray::Vertex);
        context.palette_weights.as_mut().unwrap().enabled = true;
        context.palette_indices.as_mut().unwrap().enabled = true;
        context.enable(GL_MATRIX_PALETTE_OES);
        context.draw_arrays(0x0000, 0, 1);
        assert!(context.commands().iter().any(|command| matches!(
            command,
            crate::gles::GlesCommand::Draw { vertices, .. }
                if vertices.iter().any(|vertex| vertex.x == 1.0)
        )));
    }
}
