//! Render raw little-endian f64 audio (mono, fed to both channels) through
//! the Bluebox engine; the measurement tools compare it with the reference.
//!
//! ```sh
//! cargo run --release -p saturate-dsp --example bluebox_render -- \
//!     in.f64 out.f64 [pentode=50] [triode=50] [sat_in=0] [saturation=50] [sat_freq=low|flat|high]
//!     [alt=0] [air_in=0] [air=50] [cal=dark|norm|bright] [density=0] [input=0] [output=50] [mix=100]
//! ```

use std::process::ExitCode;

use saturate_dsp::bluebox::{Bluebox, BlueboxParams, Cal, SatFreq};

fn run(args: &[String]) -> Result<(), String> {
    let [_, input, output, settings @ ..] = args else {
        return Err("usage: bluebox_render in.f64 out.f64 [name=value]...".into());
    };
    let mut p = BlueboxParams::default();
    for kv in settings {
        let (k, v) = kv.split_once('=').ok_or_else(|| format!("{kv}: expected name=value"))?;
        let num = || v.parse::<f64>().map_err(|e| format!("{kv}: {e}"));
        match k {
            "pentode" => p.pentode = num()?,
            "triode" => p.triode = num()?,
            "sat_in" => p.saturation_in = num()? != 0.0,
            "saturation" => p.saturation = num()?,
            "sat_freq" => p.saturation_freq = match v { "low" => SatFreq::Low, "high" => SatFreq::High, _ => SatFreq::Flat },
            "alt" => p.alt_tube = num()? != 0.0,
            "air_in" => p.air_in = num()? != 0.0,
            "air" => p.air = num()?,
            "cal" => p.cal = match v { "dark" => Cal::Dark, "bright" => Cal::Bright, _ => Cal::Norm },
            "density" => p.density = num()?,
            "input" => p.input_db = num()?,
            "output" => p.output = num()?,
            "mix" => p.mix = num()?,
            _ => return Err(format!("unknown control {k}")),
        }
    }
    let mut fx = Bluebox::new(48_000.0);
    fx.set(p);
    let raw = std::fs::read(input).map_err(|e| format!("{input}: {e}"))?;
    let mut out = Vec::with_capacity(raw.len());
    for c in raw.chunks_exact(8) {
        let s = f64::from_le_bytes(c.try_into().unwrap_or([0; 8]));
        out.extend_from_slice(&fx.process(s, s).0.to_le_bytes());
    }
    std::fs::write(output, out).map_err(|e| format!("{output}: {e}"))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("bluebox_render: {e}");
            ExitCode::FAILURE
        }
    }
}
