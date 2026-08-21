use std::collections::HashMap;
use std::io::{Read, Seek};
use std::path::Path;

use anyhow::{Context, Result};
use jpeg_decoder::Decoder as JpegDecoder;
use png::ColorType;

use crate::Rgba8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<Rgba8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtlasRegion {
    pub page: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Default)]
pub struct AssetStore {
    files: HashMap<String, Vec<u8>>,
    images: HashMap<String, AssetImage>,
    atlases: HashMap<String, HashMap<String, AtlasRegion>>,
}

impl AssetStore {
    pub fn from_archive<R: Read + Seek>(archive: &mut zip::ZipArchive<R>) -> Result<Self> {
        let mut store = Self::default();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            if entry.is_dir() {
                continue;
            }
            let name = normalize(entry.name());
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            store.files.insert(name, bytes);
        }
        store.index_images();
        store.index_atlases();
        Ok(store)
    }

    pub fn image(&mut self, path: &str) -> Result<Option<AssetImage>> {
        let path = normalize(path);
        if let Some(image) = self.images.get(&path) {
            return Ok(Some(image.clone()));
        }
        let Some(bytes) = self.files.get(&path).cloned() else {
            return Ok(None);
        };
        let image = decode_image(&bytes, &path)?;
        self.images.insert(path, image.clone());
        Ok(Some(image))
    }

    pub fn resolve(&self, path: &str) -> Option<&[u8]> {
        self.files.get(&normalize(path)).map(Vec::as_slice)
    }

    pub fn atlas_region(&self, atlas_path: &str, region_name: &str) -> Option<AtlasRegion> {
        self.atlases
            .get(&normalize(atlas_path))
            .and_then(|regions| regions.get(region_name))
            .cloned()
    }

    fn index_images(&mut self) {
        let paths: Vec<String> = self.files.keys().cloned().collect();
        for path in paths {
            if !is_image_path(&path) {
                continue;
            }
            if let Ok(image) = decode_image(self.files.get(&path).expect("image path exists"), &path) {
                self.images.insert(path, image);
            }
        }
    }

    fn index_atlases(&mut self) {
        let atlas_paths: Vec<String> = self
            .files
            .keys()
            .filter(|path| path.ends_with(".atlas") || path.ends_with("/pack"))
            .cloned()
            .collect();
        for atlas_path in atlas_paths {
            let Some(bytes) = self.files.get(&atlas_path) else {
                continue;
            };
            let text = String::from_utf8_lossy(bytes);
            let mut lines = text.lines();
            let mut page = String::new();
            let mut regions = HashMap::new();
            while let Some(line) = lines.next() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if line.ends_with(".png") || line.ends_with(".jpg") || line.ends_with(".jpeg") {
                    page = join_parent(&atlas_path, line);
                    for _ in 0..4 {
                        let _ = lines.next();
                    }
                    continue;
                }
                let Some(name) = line.strip_suffix(":") else {
                    continue;
                };
                let mut x = None;
                let mut y = None;
                let mut width = None;
                let mut height = None;
                for _ in 0..8 {
                    let Some(value) = lines.next() else {
                        break;
                    };
                    let mut parts = value.split(':');
                    let key = parts.next().unwrap_or_default().trim();
                    let data = parts.next().unwrap_or_default().trim();
                    match key {
                        "xy" => {
                            let values: Vec<_> = data.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                            if values.len() == 2 { x = Some(values[0]); y = Some(values[1]); }
                        }
                        "size" => {
                            let values: Vec<_> = data.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                            if values.len() == 2 { width = Some(values[0]); height = Some(values[1]); }
                        }
                        "orig" | "offset" | "index" | "rotate" | "split" | "pad" => {}
                        _ => break,
                    }
                }
                if let (Some(x), Some(y), Some(width), Some(height)) = (x, y, width, height) {
                    regions.insert(name.to_owned(), AtlasRegion { page: page.clone(), x, y, width, height });
                }
            }
            self.atlases.insert(normalize(&atlas_path), regions);
        }
    }
}

fn normalize(path: &str) -> String {
    path.trim_start_matches('/')
        .strip_prefix("assets/")
        .unwrap_or(path.trim_start_matches('/'))
        .replace('\\', "/")
}

fn join_parent(path: &str, child: &str) -> String {
    path.rsplit_once('/').map_or_else(
        || child.to_owned(),
        |(parent, _)| format!("{parent}/{child}"),
    )
}

fn is_image_path(path: &str) -> bool {
    path.ends_with(".png") || path.ends_with(".jpg") || path.ends_with(".jpeg")
}

fn decode_image(bytes: &[u8], path: &str) -> Result<AssetImage> {
    if path.ends_with(".png") {
        decode_png(bytes)
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        decode_jpeg(bytes)
    } else {
        anyhow::bail!("unsupported image format: {path}")
    }
}

fn decode_png(bytes: &[u8]) -> Result<AssetImage> {
    let decoder = png::Decoder::new(bytes);
    let mut reader = decoder.read_info()?;
    let mut output = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut output)?;
    let data = &output[..info.buffer_size()];
    let pixels = match info.color_type {
        ColorType::Rgba => data.chunks_exact(4).map(|chunk| Rgba8 { r: chunk[0], g: chunk[1], b: chunk[2], a: chunk[3] }).collect(),
        ColorType::Rgb => data.chunks_exact(3).map(|chunk| Rgba8 { r: chunk[0], g: chunk[1], b: chunk[2], a: 255 }).collect(),
        ColorType::GrayscaleAlpha => data.chunks_exact(2).map(|chunk| Rgba8 { r: chunk[0], g: chunk[0], b: chunk[0], a: chunk[1] }).collect(),
        ColorType::Grayscale => data.iter().map(|value| Rgba8 { r: *value, g: *value, b: *value, a: 255 }).collect(),
        ColorType::Indexed => {
            let palette = reader.info().palette.as_ref().context("indexed PNG has no palette")?;
            let transparency = reader.info().trns.as_ref();
            data.iter().map(|index| {
                let offset = *index as usize * 3;
                Rgba8 { r: palette[offset], g: palette[offset + 1], b: palette[offset + 2], a: transparency.and_then(|values| values.get(*index as usize)).copied().unwrap_or(255) }
            }).collect()
        }
    };
    Ok(AssetImage { width: info.width.into(), height: info.height.into(), pixels })
}

fn decode_jpeg(bytes: &[u8]) -> Result<AssetImage> {
    let mut decoder = JpegDecoder::new(bytes);
    let pixels = decoder.decode()?;
    let info = decoder.info().context("JPEG has no image info")?;
    let pixels = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => pixels.chunks_exact(3).map(|chunk| Rgba8 { r: chunk[0], g: chunk[1], b: chunk[2], a: 255 }).collect(),
        jpeg_decoder::PixelFormat::L8 => pixels.iter().map(|value| Rgba8 { r: *value, g: *value, b: *value, a: 255 }).collect(),
        format => anyhow::bail!("unsupported JPEG pixel format: {format:?}"),
    };
    Ok(AssetImage { width: info.width.into(), height: info.height.into(), pixels })
}
