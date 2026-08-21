use std::collections::BTreeSet;

use crate::dalvik::DexFile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityReport {
    pub implemented: Vec<String>,
    pub unimplemented: Vec<String>,
}

impl CompatibilityReport {
    pub fn format_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for feature in &self.implemented {
            lines.push(format!("IMPLEMENTED: {feature}"));
        }
        for feature in &self.unimplemented {
            lines.push(format!("UNIMPLEMENTED: {feature}"));
        }
        lines
    }
}

const FEATURES: &[(&str, &str, bool)] = &[
    ("android/opengl", "GLES 1.x graphics backend", true),
    ("android/graphics/Canvas", "legacy Canvas renderer", true),
    (
        "javax/microedition/khronos",
        "OpenGL ES EGL/Khronos bridge",
        true,
    ),
    ("android/media/AudioTrack", "AudioTrack PCM mixer", true),
    ("android/media/MediaPlayer", "MediaPlayer backend", false),
    ("android/media/SoundPool", "SoundPool effects backend", true),
    (
        "android/view/SurfaceView",
        "SurfaceView and framebuffer bridge",
        false,
    ),
    (
        "android/view/SurfaceHolder",
        "Surface lifecycle and buffer queue",
        false,
    ),
    (
        "android/database/sqlite",
        "SQLite compatibility layer",
        false,
    ),
    ("SharedPreferences", "SharedPreferences persistence", false),
    ("android/os/Looper", "Looper/Handler message queues", true),
    (
        "android/hardware/Sensor",
        "sensor compatibility layer",
        false,
    ),
    (
        "android/hardware/Camera",
        "camera compatibility layer",
        false,
    ),
    ("android/location", "location services", false),
    ("android/bluetooth", "Bluetooth services", false),
    ("android/webkit", "WebView compatibility layer", false),
    ("android/net/", "Android network services", false),
    (
        "android/view/MotionEvent",
        "touch and motion input bridge",
        true,
    ),
    ("dalvik/system", "Dalvik system APIs", false),
];

pub fn scan_dex(dex: &DexFile) -> CompatibilityReport {
    let mut symbols = BTreeSet::new();
    symbols.extend(dex.strings.iter().map(String::as_str));
    symbols.extend(dex.types.iter().map(String::as_str));
    symbols.extend(
        dex.methods
            .iter()
            .flat_map(|method| [method.class_name.as_str(), method.name.as_str()]),
    );
    let joined = symbols.into_iter().collect::<Vec<_>>().join("\n");
    let mut implemented = BTreeSet::new();
    let mut unimplemented = BTreeSet::new();
    for (needle, feature, is_implemented) in FEATURES {
        if joined.contains(needle) {
            if *is_implemented {
                implemented.insert((*feature).to_owned());
            } else {
                unimplemented.insert((*feature).to_owned());
            }
        }
    }
    CompatibilityReport {
        implemented: implemented.into_iter().collect(),
        unimplemented: unimplemented.into_iter().collect(),
    }
}
