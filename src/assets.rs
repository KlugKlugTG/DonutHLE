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

impl AssetImage {
    pub fn decode(path: &Path, bytes: &[u8]) -> Result<Self> {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg") {
            decode_jpeg(bytes)
        } else {
            decode_png(bytes)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtlasRegionInfo {
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
    atlases: HashMap<String, HashMap<String, AtlasRegionInfo>>,
}

impl AssetStore {
    pub fn from_archive<R: Read + Seek>(archive: &mut zip::ZipArchive<R>) -> Result<Self> {
        let mut store = Self::default();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            if entry.is_dir() {
                continue;
            }
            let name = entry.name().to_owned();
            let key = name.strip_prefix("assets/").unwrap_or(&name).to_owned();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            store.files.insert(key, bytes);
        }
        store.index_atlases();
        Ok(store)
    }

    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.files.get(&normalize(path)).cloned()
    }

    pub fn image_size(&self, path: &str) -> Option<(u32, u32)> {
        self.image(path)
            .ok()
            .map(|image| (image.width, image.height))
    }

    pub fn image(&self, path: &str) -> Result<AssetImage> {
        let key = normalize(path);
        if let Some(image) = self.images.get(&key) {
            return Ok(image.clone());
        }
        let bytes = self.files.get(&key).context("asset is missing")?;
        AssetImage::decode(Path::new(&key), bytes)
    }

    pub fn atlas_region(&self, atlas_path: &str, name: &str) -> Option<AtlasRegionInfo> {
        self.atlases
            .get(&normalize(atlas_path))
            .and_then(|regions| regions.get(name).cloned())
    }

    fn index_atlases(&mut self) {
        let atlas_paths: Vec<String> = self
            .files
            .keys()
            .filter(|path| path.ends_with("/pack") || path.ends_with(".atlas"))
            .cloned()
            .collect();
        for atlas_path in atlas_paths {
            let Some(bytes) = self.files.get(&atlas_path) else {
                continue;
            };
            let text = String::from_utf8_lossy(bytes);
            let mut lines = text.lines().peekable();
            let mut page = None;
            let mut regions = HashMap::new();
            while let Some(line) = lines.next() {
                let line = line.trim();
                if line.is_empty()
                    || line.starts_with("format:")
                    || line.starts_with("filter:")
                    || line.starts_with("repeat:")
                {
                    continue;
                }
                if line.starts_with("rotate:")
                    || line.starts_with("xy:")
                    || line.starts_with("size:")
                    || line.starts_with("orig:")
                    || line.starts_with("offset:")
                    || line.starts_with("index:")
                {
                    continue;
                }
                if let Some(next) = lines.peek().map(|value| value.trim()) {
                    if next.starts_with("format:") {
                        page = Some(line.to_owned());
                        lines.next();
                        continue;
                    }
                }
                let Some(rotate) = lines.next() else { continue };
                let Some(xy) = lines.next() else { continue };
                let Some(size) = lines.next() else { continue };
                if !rotate.trim().starts_with("rotate:")
                    || !xy.trim().starts_with("xy:")
                    || !size.trim().starts_with("size:")
                {
                    continue;
                }
                let Some((x, y)) = parse_pair(xy.trim().trim_start_matches("xy:")) else {
                    continue;
                };
                let Some((width, height)) = parse_pair(size.trim().trim_start_matches("size:"))
                else {
                    continue;
                };
                let Some(page_name) = page.clone() else {
                    continue;
                };
                let page_path = join_parent(&atlas_path, &page_name);
                regions.insert(
                    line.to_owned(),
                    AtlasRegionInfo {
                        page: page_path,
                        x,
                        y,
                        width,
                        height,
                    },
                );
                for _ in 0..3 {
                    let _ = lines.next();
                }
            }
            self.atlases.insert(normalize(&atlas_path), regions);
        }
    }
}

fn normalize(path: &str) -> String {
    path.trim_start_matches("/")
        .strip_prefix("assets/")
        .unwrap_or(path.trim_start_matches("/"))
        .replace('\\', "/")
}

fn join_parent(path: &str, child: &str) -> String {
    path.rsplit_once('/').map_or_else(
        || child.to_owned(),
        |(parent, _)| format!("{parent}/{child}"),
    )
}

fn parse_pair(value: &str) -> Option<(u32, u32)> {
    let mut parts = value.trim().split(',').map(str::trim);
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

fn decode_png(bytes: &[u8]) -> Result<AssetImage> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().context("decode PNG header")?;
    let mut output = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut output)
        .context("decode PNG pixels")?;
    let pixels = match info.color_type {
        ColorType::Rgba => output[..info.buffer_size()]
            .chunks_exact(4)
            .map(|chunk| Rgba8 {
                r: chunk[0],
                g: chunk[1],
                b: chunk[2],
                a: chunk[3],
            })
            .collect(),
        ColorType::Rgb => output[..info.buffer_size()]
            .chunks_exact(3)
            .map(|chunk| Rgba8 {
                r: chunk[0],
                g: chunk[1],
                b: chunk[2],
                a: 255,
            })
            .collect(),
        ColorType::GrayscaleAlpha => output[..info.buffer_size()]
            .chunks_exact(2)
            .map(|chunk| Rgba8 {
                r: chunk[0],
                g: chunk[0],
                b: chunk[0],
                a: chunk[1],
            })
            .collect(),
        ColorType::Grayscale => output[..info.buffer_size()]
            .iter()
            .copied()
            .map(|value| Rgba8 {
                r: value,
                g: value,
                b: value,
                a: 255,
            })
            .collect(),
        ColorType::Indexed => {
            let palette = reader
                .info()
                .palette
                .as_ref()
                .context("indexed PNG has no palette")?;
            let transparency = reader.info().trns.as_ref();
            output[..info.buffer_size()]
                .iter()
                .map(|index| {
                    let offset = usize::from(*index) * 3;
                    let alpha = transparency
                        .and_then(|values| values.get(usize::from(*index)))
                        .copied()
                        .unwrap_or(255);
                    Rgba8 {
                        r: *palette.get(offset).unwrap_or(&0),
                        g: *palette.get(offset + 1).unwrap_or(&0),
                        b: *palette.get(offset + 2).unwrap_or(&0),
                        a: alpha,
                    }
                })
                .collect()
        }
    };
    Ok(AssetImage {
        width: info.width,
        height: info.height,
        pixels,
    })
}

fn decode_jpeg(bytes: &[u8]) -> Result<AssetImage> {
    let mut decoder = JpegDecoder::new(std::io::Cursor::new(bytes));
    let pixels = decoder.decode().context("decode JPEG pixels")?;
    let info = decoder.info().context("read JPEG dimensions")?;
    let pixels = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => pixels
            .chunks_exact(3)
            .map(|chunk| Rgba8 {
                r: chunk[0],
                g: chunk[1],
                b: chunk[2],
                a: 255,
            })
            .collect(),
        jpeg_decoder::PixelFormat::L8 => pixels
            .iter()
            .copied()
            .map(|value| Rgba8 {
                r: value,
                g: value,
                b: value,
                a: 255,
            })
            .collect(),
        format => anyhow::bail!("unsupported JPEG pixel format: {format:?}"),
    };
    Ok(AssetImage {
        width: info.width.into(),
        height: info.height.into(),
        pixels,
    })
}
