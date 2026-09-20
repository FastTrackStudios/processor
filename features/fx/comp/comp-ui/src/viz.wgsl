// The compressor, as light.
//
// The input is drawn WHITE-GREY on purpose. Every other block in the rack has
// a hue that says which effect it is; the compressor's input is not an
// effect's contribution, it is the signal itself, and giving it a colour would
// make it look like one more coloured lane. The gain reduction is the red
// hanging off the ceiling — the part that was taken away.
//
// Layout of `u` mirrors `CompUniforms` in viz.rs. Both traces are fixed-length
// because a uniform buffer is; `meter.z` carries how many samples are real.

const TRACE_LEN: u32 = 128u;
const TAU: f32 = 6.28318530718;

struct Comp {
    // width, height, seconds, on
    frame: vec4<f32>,
    // threshold_db, ratio, knee_db, range_db
    curve: vec4<f32>,
    // in_db, gr_db, trace_len, unused
    meter: vec4<f32>,
    // the lane's colour; w unused
    color: vec4<f32>,
    // input trace, four samples per row
    input: array<vec4<f32>, 32>,
    // gain-reduction trace, four samples per row
    gr: array<vec4<f32>, 32>,
};

@group(0) @binding(0) var<uniform> u: Comp;

fn lane_at(rows: ptr<function, array<vec4<f32>, 32>>, i: u32) -> f32 {
    let row = (*rows)[i / 4u];
    let k = i % 4u;
    if (k == 0u) { return row.x; }
    if (k == 1u) { return row.y; }
    if (k == 2u) { return row.z; }
    return row.w;
}

// One trace, sampled across the panel with its neighbours blended, so it reads
// as a body rather than as a bar chart.
fn input_at(x: f32) -> f32 {
    let count = u32(max(u.meter.z, 0.0));
    if (count < 2u) { return 0.0; }
    let pos = clamp(x, 0.0, 1.0) * f32(count - 1u);
    let i = u32(floor(pos));
    let j = min(i + 1u, count - 1u);
    var rows = u.input;
    return mix(lane_at(&rows, i), lane_at(&rows, j), fract(pos));
}

fn gr_at(x: f32) -> f32 {
    let count = u32(max(u.meter.z, 0.0));
    if (count < 2u) { return 0.0; }
    let pos = clamp(x, 0.0, 1.0) * f32(count - 1u);
    let i = u32(floor(pos));
    let j = min(i + 1u, count - 1u);
    var rows = u.gr;
    return mix(lane_at(&rows, i), lane_at(&rows, j), fract(pos));
}

// The soft-knee transfer function — the SAME maths as `compress_transfer` in
// comp_graph_svg.rs. Two copies is a real risk, so they are written to match
// line for line: below `threshold − knee/2` output equals input, above
// `threshold + knee/2` it follows the ratio slope, and inside the knee it is
// the quadratic that joins them.
fn transfer(input_db: f32) -> f32 {
    let threshold = u.curve.x;
    let ratio = max(u.curve.y, 1.0);
    let knee = u.curve.z;
    let slope = 1.0 - 1.0 / ratio;
    let half_knee = knee * 0.5;
    if (knee > 0.001 && abs(input_db - threshold) < half_knee) {
        let x = input_db - threshold + half_knee;
        return input_db - slope * x * x / (2.0 * knee);
    }
    if (input_db <= threshold) { return input_db; }
    return input_db - slope * (input_db - threshold);
}

// dB (0 at the top … −range at the bottom) → 0..=1 down the panel.
fn db_to_v(db: f32) -> f32 {
    return clamp(-db / max(u.curve.w, 1.0), 0.0, 1.0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let px = uv * vec2<f32>(u.frame.x, u.frame.y);
    let t = u.frame.z;
    let lit = u.frame.w > 0.5;

    var tint = u.color.rgb;
    if (!lit) { tint = vec3<f32>(0.30, 0.30, 0.34); }
    let hot = mix(tint, vec3<f32>(1.0), 0.5);
    // The one hue in the picture, and it is the loss.
    var cut = vec3<f32>(0.94, 0.35, 0.35);
    if (!lit) { cut = vec3<f32>(0.34, 0.28, 0.30); }

    var rgb = vec3<f32>(0.0);
    var alpha = 0.0;

    // ── The input, rising from the floor ────────────────────────────────
    let lvl = input_at(uv.x);
    let top = 1.0 - lvl;
    let below = smoothstep(0.0, 0.010, uv.y - top);
    let body = below * (0.07 + 0.30 * lvl);
    // The lit edge, tight to the waveform.
    let edge = exp(-abs(uv.y - top) * 150.0) * (0.35 + 0.65 * lvl);
    // Energy spilling past its own edge — what makes a loud passage look
    // loud rather than merely tall.
    let spill = exp(-max(top - uv.y, 0.0) * (30.0 - 14.0 * lvl)) * 0.26 * lvl;
    rgb += tint * (body + edge * 1.35 + spill);
    alpha += body * 0.55 + edge * 0.8 + spill * 0.6;

    // ── The gain reduction, hanging from the ceiling ────────────────────
    //
    // Drawn downward from the top because that is the direction the level is
    // being pushed. It is the compressor's actual output — the one thing the
    // knobs cannot show.
    let red = gr_at(uv.x);
    if (red > 0.001) {
        let inside = 1.0 - smoothstep(red - 0.004, red + 0.010, uv.y);
        let gr_body = inside * (0.10 + 0.40 * red);
        let gr_edge = exp(-abs(uv.y - red) * 150.0) * (0.4 + 0.6 * red);
        rgb += cut * (gr_body + gr_edge * 1.3);
        alpha += gr_body * 0.55 + gr_edge * 0.8;
    }

    // ── The transfer curve ──────────────────────────────────────────────
    //
    // Evaluated per pixel rather than sampled into a path: the knee is where
    // the interesting part is, and a polyline is exactly where a sampled
    // curve loses it.
    let range = max(u.curve.w, 1.0);
    let in_db = (uv.x - 1.0) * range;
    let out_v = db_to_v(transfer(in_db));
    let dist = abs(uv.y - out_v) * u.frame.y;
    let line = exp(-(dist * dist) / 3.0);
    let bloom = exp(-dist / 9.0) * 0.34;
    rgb += hot * (line * 0.95 + bloom);
    alpha += line * 0.9 + bloom * 0.45;

    // Above the knee the curve is bending the signal; hazing that region
    // says WHERE the compressor is working without another line to read.
    let knee_top = db_to_v(transfer(u.curve.x));
    if (uv.y < knee_top) {
        let work = (knee_top - uv.y) / max(knee_top, 1e-3);
        rgb += cut * work * 0.05;
        alpha += work * 0.05;
    }

    // ── The ball ────────────────────────────────────────────────────────
    //
    // Where the signal is sitting on its own curve, right now. The whole
    // picture is context for this one point.
    if (lit) {
        let bx = clamp(u.meter.x / range + 1.0, 0.0, 1.0);
        let by = db_to_v(transfer(u.meter.x));
        let d = length((vec2<f32>(bx, by) - uv) * vec2<f32>(u.frame.x, u.frame.y));
        // Breathing with the gain reduction, so the ball is doing more when
        // the compressor is.
        let work = clamp(u.meter.y / 12.0, 0.0, 1.0);
        let r = 3.0 + 2.0 * work * (1.0 + 0.12 * sin(t * TAU * 0.8));
        let core = exp(-(d * d) / (r * r * 0.6));
        let halo = exp(-d / (r * 4.0)) * 0.5;
        rgb += mix(hot, cut, work) * (core + halo);
        alpha += core * 0.95 + halo * 0.4;
    }

    // Tone-map rather than clamp: the ball sitting on the curve is two bright
    // things overlapping, and clipping loses the colour in both exactly there.
    // Driven before the tone map. Reinhard on raw contributions crushes
    // everything below 1.0, and 1.0 is brighter than anything this panel
    // produces — so without the drive the whole picture sits in the bottom
    // quarter of the curve and reads as switched off.
    rgb = rgb * 2.2;
    rgb = rgb / (rgb + vec3<f32>(1.0));
    alpha = clamp(alpha, 0.0, 0.94);
    return vec4<f32>(rgb * alpha, alpha);
}
