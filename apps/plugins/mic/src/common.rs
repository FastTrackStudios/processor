//! Where the models live (the mic list and parameter helpers are
//! `mic_ui::params`).

use std::path::PathBuf;
use mic_dsp::MicModel;

pub use mic_ui::params::{MICS, index};

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

