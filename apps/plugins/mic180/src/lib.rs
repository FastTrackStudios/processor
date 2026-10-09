//! FTS Mic 180 — the stereo variant of FTS Mic.
//!
//! One capsule pair in (left = front capsule, right = rear); two modelled
//! mics out: mic 1 facing forward on the left, mic 2 facing back on the
//! right, balanced by Mic Pan and crossfed by Stereo Width (see
//! [`mic_dsp::StereoMic`]). The backward mic has its own measured models
//! (`<rate>[-LX]-180/`), the forward one is the ordinary model.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam_channel::{Receiver, Sender, bounded};
use mic_dsp::{MicChain, Settings, StereoMic, StereoSettings, latency};
use nice_plug::prelude::*;

#[path = "../../mic/src/common.rs"]
mod common;
use common::{FILTERS, MICS, PATTERNS, SOURCES, choice, float, index, load_model, load_model_variant};

const PLUGIN_NAME: &str = "FTS Mic 180";

#[derive(Params)]
pub struct Mic180Params {
    #[id = "type1"]
    pub type1: IntParam,
    #[id = "pattern1"]
    pub pattern1: IntParam,
    #[id = "filter1"]
    pub filter1: IntParam,
    #[id = "axis1"]
    pub axis1: FloatParam,
    #[id = "type2"]
    pub type2: IntParam,
    #[id = "pattern2"]
    pub pattern2: IntParam,
    #[id = "filter2"]
    pub filter2: IntParam,
    #[id = "axis2"]
    pub axis2: FloatParam,
    /// Mic 2 follows mic 1's type, pattern, filter and axis.
    #[id = "link"]
    pub link: BoolParam,
    #[id = "pan"]
    pub pan: FloatParam,
    #[id = "width"]
    pub width: FloatParam,
    #[id = "proximity"]
    pub proximity: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
    #[id = "phase"]
    pub phase: BoolParam,
    #[id = "rear_trim"]
    pub rear_trim: FloatParam,
    #[id = "swap"]
    pub swap: BoolParam,
    #[id = "source"]
    pub source: IntParam,
}

impl Default for Mic180Params {
    fn default() -> Self {
        Self {
            type1: choice("Mic1 Type", 0, &MICS),
            pattern1: choice("Mic1 Pattern", 4, &PATTERNS),
            filter1: choice("Mic1 Filter", 0, &FILTERS),
            axis1: float("Mic1 Axis", 0.0, 0.0, 180.0, "°", 1),
            type2: choice("Mic2 Type", 0, &MICS),
            pattern2: choice("Mic2 Pattern", 4, &PATTERNS),
            filter2: choice("Mic2 Filter", 0, &FILTERS),
            axis2: float("Mic2 Axis", 0.0, 0.0, 180.0, "°", 1),
            link: BoolParam::new("Mic Link", true),
            pan: float("Mic Pan", 0.0, -100.0, 100.0, "%", 1),
            width: float("Stereo Width", 100.0, 0.0, 200.0, "%", 1),
            proximity: float("Proximity", 0.0, -100.0, 100.0, "%", 1),
            output: float("Output", 0.0, -12.0, 12.0, " dB", 1),
            phase: BoolParam::new("Phase Invert", false),
            rear_trim: float("Rear Trim", 0.0, -6.0, 6.0, " dB", 2),
            swap: BoolParam::new("Swap Capsules", false),
            source: choice("Source Mic", 0, &SOURCES),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Want {
    mic1: usize,
    mic2: usize,
    source: usize,
    sample_rate: u32,
}

fn loader(requests: &Receiver<Want>, ready: &Sender<(Want, Box<StereoMic>)>, trash: &Receiver<Box<StereoMic>>, alive: &AtomicBool) {
    while alive.load(Ordering::Relaxed) {
        while trash.try_recv().is_ok() {}
        let Ok(mut want) = requests.recv_timeout(std::time::Duration::from_millis(50)) else { continue };
        while let Ok(newer) = requests.try_recv() {
            want = newer;
        }
        let (Some(fwd), Some(back)) = (
            load_model(want.mic1, want.source, want.sample_rate),
            load_model_variant(want.mic2, want.source, want.sample_rate, "-180"),
        ) else {
            continue;
        };
        let stereo = Box::new(StereoMic::new(MicChain::new(fwd), MicChain::new(back)));
        if ready.send((want, stereo)).is_err() {
            break;
        }
    }
}

pub struct FtsMic180 {
    params: Arc<Mic180Params>,
    engine: Option<Box<StereoMic>>,
    have: Option<Want>,
    asked: Option<Want>,
    sample_rate: u32,
    requests: Sender<Want>,
    ready: Receiver<(Want, Box<StereoMic>)>,
    trash: Sender<Box<StereoMic>>,
    alive: Arc<AtomicBool>,
}

impl Default for FtsMic180 {
    fn default() -> Self {
        let (req_tx, req_rx) = bounded(16);
        let (ready_tx, ready_rx) = bounded(2);
        let (trash_tx, trash_rx) = bounded(4);
        let alive = Arc::new(AtomicBool::new(true));
        let flag = alive.clone();
        let _ = std::thread::Builder::new().name("fts-mic180-loader".into()).spawn(move || loader(&req_rx, &ready_tx, &trash_rx, &flag));
        Self {
            params: Arc::new(Mic180Params::default()),
            engine: None,
            have: None,
            asked: None,
            sample_rate: 48_000,
            requests: req_tx,
            ready: ready_rx,
            trash: trash_tx,
            alive,
        }
    }
}

impl Drop for FtsMic180 {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
    }
}

impl FtsMic180 {
    fn linked(&self) -> bool {
        self.params.link.value()
    }

    fn want(&self) -> Want {
        let p = &self.params;
        Want {
            mic1: index(&p.type1),
            mic2: if self.linked() { index(&p.type1) } else { index(&p.type2) },
            source: index(&p.source),
            sample_rate: self.sample_rate,
        }
    }

    fn settings(&self, second: bool) -> Settings {
        let p = &self.params;
        let use2 = second && !self.linked();
        let (pattern, filter, axis) = if use2 { (&p.pattern2, &p.filter2, &p.axis2) } else { (&p.pattern1, &p.filter1, &p.axis1) };
        Settings {
            pattern: index(pattern),
            axis_deg: f64::from(axis.value()),
            low_cut: index(filter),
            proximity: f64::from(p.proximity.value()),
            output_db: f64::from(p.output.value()),
            phase_invert: p.phase.value(),
            rear_trim_db: f64::from(p.rear_trim.value()),
            swap: p.swap.value(),
        }
    }

    fn sync(&mut self) {
        let want = self.want();
        if self.asked != Some(want) && self.requests.try_send(want).is_ok() {
            self.asked = Some(want);
        }
        if let Ok((got, loaded)) = self.ready.try_recv() {
            if let Some(old) = self.engine.replace(loaded) {
                let _ = self.trash.try_send(old);
            }
            self.have = Some(got);
        }
        let (s1, s2) = (self.settings(false), self.settings(true));
        let stereo = StereoSettings {
            pan: f64::from(self.params.pan.value()) / 100.0,
            width: f64::from(self.params.width.value()) / 100.0,
        };
        if let Some(e) = self.engine.as_mut() {
            if e.forward.settings() != s1 {
                e.forward.apply(s1, false);
            }
            if e.backward.settings() != s2 {
                e.backward.apply(s2, false);
            }
            e.set(stereo);
        }
    }
}

impl Plugin for FtsMic180 {
    const NAME: &'static str = PLUGIN_NAME;
    const VENDOR: &'static str = "FastTrackStudio";
    const URL: &'static str = "https://fasttrackstudio.com";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    type Editor = ();
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn activate(&mut self, _layout: &AudioIOLayout, buffer_config: &BufferConfig, context: &mut impl ActivateContext<Self>) -> bool {
        self.sample_rate = u32::try_from(dsp_core::f32_to_index(buffer_config.sample_rate.round())).unwrap_or(48_000);
        context.set_latency_samples(u32::try_from(latency(f64::from(self.sample_rate))).unwrap_or(24));
        self.asked = None;
        true
    }

    fn reset(&mut self) {
        if let Some(e) = self.engine.as_mut() {
            e.forward.reset();
            e.backward.reset();
        }
    }

    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &mut impl ProcessContext<Self>) -> ProcessStatus {
        self.sync();
        let ready = self.have.is_some_and(|h| h.sample_rate == self.sample_rate);
        for mut frame in buffer.iter_samples() {
            let mut it = frame.iter_mut();
            let (Some(l), Some(r)) = (it.next(), it.next()) else { continue };
            let (yl, yr) = match (ready, self.engine.as_mut()) {
                (true, Some(e)) => e.process(f64::from(*l), f64::from(*r)),
                _ => (0.0, 0.0),
            };
            *l = dsp_core::num::f64_to_f32(yl);
            *r = dsp_core::num::f64_to_f32(yr);
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for FtsMic180 {
    const CLAP_ID: &'static str = "com.fasttrackstudio.mic180";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Dual-capsule microphone modelling, stereo (forward + backward mic)");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect, ClapFeature::Stereo];
}

impl Vst3Plugin for FtsMic180 {
    const VST3_CLASS_ID: [u8; 16] = *b"FtsMic180Modelr1";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Tools];
}

nice_export_clap!(FtsMic180);
nice_export_vst3!(FtsMic180);
