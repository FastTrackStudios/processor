//! What both FTS Mic plugins share: the mic list, the parameter helpers
//! and where the models live. Included by path from each plugin crate.

use std::path::PathBuf;
use std::sync::Arc;

use mic_dsp::MicModel;
use nice_plug::prelude::*;

/// The modelled mics, in the reference's order.
pub const MICS: [&str; 40] = [
    "LD-47K", "LD-49K", "LD-67", "LD-67 NOS", "LD-87 Vintage", "LD-87 Modern", "LD-87 TK", "LD-103", "LD-12",
    "LD-251", "LD-800", "LD-37A", "LD-37P", "LD-BV1", "LD-414 Brass", "LD-414 Nylon", "LD-414 US", "LD-414 T2",
    "LD-563", "LD-017T", "SD-451", "SD-416", "RB-4038", "RB-77DX Satin", "RB-77DX Umber", "RB-121", "RB-160",
    "DN-57", "DN-7", "DN-20", "DN-409N", "DN-409U", "DN-421N", "DN-421S", "DN-421B", "DN-12A", "DN-12E",
    "Sphere Linear", "Sphere Diffuse", "Sphere Direct",
];
pub const PATTERNS: [&str; 9] =
    ["Omni", "Sub-Omni", "Wide-Card", "Sub-Card", "Cardioid", "Super-Card", "Hyper-Card", "Sub-8", "Figure-8"];
pub const FILTERS: [&str; 4] = ["Off", "Low", "Med", "High"];
pub const SOURCES: [&str; 2] = ["L22/DLX", "LX"];

type ToText = Arc<dyn Fn(i32) -> String + Send + Sync>;
type FromText = Arc<dyn Fn(&str) -> Option<i32> + Send + Sync>;

fn named(names: &'static [&'static str]) -> (ToText, FromText) {
    (
        Arc::new(move |v| usize::try_from(v).ok().and_then(|i| names.get(i)).map_or_else(String::new, |s| (*s).to_string())),
        Arc::new(move |s| names.iter().position(|n| n.eq_ignore_ascii_case(s.trim())).and_then(|i| i32::try_from(i).ok())),
    )
}

pub fn choice(name: &str, default: i32, names: &'static [&'static str]) -> IntParam {
    let (to_s, from_s) = named(names);
    let max = i32::try_from(names.len()).unwrap_or(1).saturating_sub(1);
    IntParam::new(name, default, IntRange::Linear { min: 0, max }).with_value_to_string(to_s).with_string_to_value(from_s)
}

pub fn float(name: &str, default: f32, min: f32, max: f32, unit: &'static str, digits: usize) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min, max })
        .with_unit(unit)
        .with_value_to_string(formatters::v2s_f32_rounded(digits))
        .with_string_to_value(Arc::new(|s: &str| s.trim().trim_end_matches(|c: char| c.is_alphabetic() || c == '%' || c == '°').trim().parse().ok()))
}

pub fn models_dir() -> PathBuf {
    std::env::var_os("FTS_MIC_MODELS").map_or_else(
        || {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
                .unwrap_or_default()
                .join("fts/mic-models")
        },
        PathBuf::from,
    )
}

/// `variant` is "" for the forward models, "-180" for the 180's backward ones.
pub fn load_model_variant(mic: usize, source: usize, sample_rate: u32, variant: &str) -> Option<MicModel> {
    let name = MICS.get(mic)?.replace(' ', "_");
    let dir = if source == 1 { format!("{sample_rate}-LX{variant}") } else { format!("{sample_rate}{variant}") };
    let bytes = std::fs::read(models_dir().join(dir).join(format!("{name}.micm"))).ok()?;
    MicModel::from_bytes(&bytes).ok()
}


pub fn load_model(mic: usize, source: usize, sample_rate: u32) -> Option<MicModel> {
    load_model_variant(mic, source, sample_rate, "")
}

pub fn index(p: &IntParam) -> usize {
    usize::try_from(p.value()).unwrap_or(0)
}
