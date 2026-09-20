// The reverb's decay, as light.
//
// The vector pass draws the same decay as geometry; what a shader adds is the
// part that is about a field of energy rather than an outline — the density of
// early reflections as actual grain, the tail as a haze that thins rather than
// a curve that descends, the bloom where a reflection lands. Those are
// per-pixel, and as geometry each one costs a shape.
//
// Layout of `u` mirrors `ReverbUniforms` in viz.rs.
//
// The beat grid is NOT here, for the same reason as the delay's: a 1 px ruled
// line is the one thing the vector pass does better, and the grid is what the
// tail's length is read against.

const TAU: f32 = 6.28318530718;

struct Reverb {
    // width, height, seconds, on
    frame: vec4<f32>,
    // decay_s, density, predelay_s, mix
    params: vec4<f32>,
    // beat_s, window_s, unused, unused
    time: vec4<f32>,
    // the lane's colour; w unused
    color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Reverb;

// A cheap stable hash, for reflection placement. Deterministic in x, so the
// reflections stand still instead of crawling: where they ARE is the density,
// and a field that shimmered position would be saying something untrue.
fn hash11(p: f32) -> f32 {
    return fract(sin(p * 127.1) * 43758.5453);
}

// The tail's height at `t` seconds, 0..=1 — on the dB axis, not the linear
// one.
//
// RT60 is the definition: 60 dB down at `decay` seconds. Plotting the LINEAR
// amplitude of that is a curve which has visually vanished by a fifth of the
// way across (−14 dB is already 0.2 of full scale), so nine tenths of the
// panel draws a tail that is still audibly there as nothing at all. Every
// other meter in this rack reads in dB for the same reason.
//
// On the dB axis the same envelope is a straight fall from full height to the
// floor at exactly `decay` — so the SLOPE is the RT60, which is the number
// the picture exists to show, and two reverbs set the same still look the
// same.
fn envelope(t: f32) -> f32 {
    let predelay = max(u.params.z, 0.0);
    if (t < predelay) { return 0.0; }
    let decay = max(u.params.x, 1e-3);
    let since = t - predelay;
    return clamp(1.0 - since / decay, 0.0, 1.0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let px = uv * vec2<f32>(u.frame.x, u.frame.y);
    let t = u.frame.z;
    let lit = u.frame.w > 0.5;

    let window = max(u.time.y, 1e-3);
    let predelay = max(u.params.z, 0.0);
    let density = clamp(u.params.y, 0.0, 1.0);
    let mid = 0.5;

    var tint = u.color.rgb;
    if (!lit) { tint = vec3<f32>(0.32, 0.32, 0.36); }
    let hot = mix(tint, vec3<f32>(1.0), 0.55);

    var rgb = vec3<f32>(0.0);
    var alpha = 0.0;

    // Seconds at this column.
    let at = uv.x * window;
    let env = envelope(at);
    // Distance from the centre line as a fraction of the half-height.
    let off = abs(uv.y - mid) * 2.0;

    // ── Before anything arrives ─────────────────────────────────────────
    //
    // Pre-delay is silence, and silence should look like silence: the gap
    // before the first reflection is the single clearest thing separating a
    // room from a hall, so it is drawn as an actually empty lane.
    let gap = 1.0 - step(predelay, at);
    let ground = (1.0 - uv.x * 0.6) * 0.07 * (1.0 - gap * 0.6);
    rgb += tint * ground;
    alpha += ground * 0.5;

    // ── The tail ────────────────────────────────────────────────────────
    //
    // A body that narrows as it decays, hazed rather than outlined. The eye
    // reads the RATE it closes, which is the RT60 made visible.
    let span = env;
    let inside = 1.0 - smoothstep(span * 0.72, span * 1.1, off);
    let body = inside * env * 0.55;
    rgb += tint * body;
    alpha += body * 0.75;

    // The lit rim where the body ends — the decay curve itself, as an edge
    // made of light rather than a stroked path.
    let rim = exp(-abs(off - span) * 14.0) * env * 0.9;
    rgb += hot * rim;
    alpha += rim * 0.85;

    // ── The early reflections ───────────────────────────────────────────
    //
    // Discrete arrivals near the front, packing tighter as density rises
    // until they stop being countable and become the wash. That transition
    // IS density, and it is why this is grain rather than a number.
    let early_span = predelay + max(u.params.x, 1e-3) * 0.28;
    if (at < early_span && at >= predelay) {
        // 4 reflections at density 0, 40 at density 1.
        let n = 4.0 + 36.0 * density;
        let k = (at - predelay) / max(early_span - predelay, 1e-4);
        let cell = floor(k * n);
        let seed = hash11(cell + 1.0);
        // Each reflection sits somewhere off the centre; its own height
        // falls with the envelope like everything else.
        let y = mix(-0.8, 0.8, hash11(cell + 17.0));
        let d = length(vec2<f32>((fract(k * n) - 0.5) * (u.frame.x / n), (uv.y - mid) * 2.0 - y) * vec2<f32>(1.0, u.frame.y * 0.25));
        let r = max(u.frame.x * 0.0016, 1.6) + 4.0 * (1.0 - density) + 2.5 * seed;
        // Early reflections thin out towards the wash rather than stopping at
        // a hard line — the transition from countable to uncountable IS the
        // density, and an edge would draw a boundary the ear does not hear.
        let into_wash = 1.0 - smoothstep(0.55, 1.0, k);
        let spark = exp(-(d * d) / (r * r)) * env * (0.45 + 0.55 * seed) * into_wash;
        rgb += hot * spark * 1.35;
        alpha += spark * 0.85;
    }

    // ── The wash ────────────────────────────────────────────────────────
    //
    // Past the early field the reflections are too dense to count, so they
    // are drawn as a moving grain instead. This is the only part that moves,
    // and it moves because a tail is not a still object.
    if (lit && at >= predelay) {
        // Two drifting fields rather than a product of two pixel-rate sines.
        // The sines striped the panel vertically at any real resolution, which
        // reads as static sitting on top of the reverb rather than as the
        // reverb itself — and static does not decay, so it fought the one
        // thing the picture is saying.
        //
        // Slow, wide, and crossing at an angle: close to still where the tail
        // is quiet, alive where it is not.
        let q = vec2<f32>(uv.x * 7.0, (uv.y - mid) * 5.0);
        let a = sin(q.x * 1.7 - t * 0.55 + q.y * 0.9);
        let b = sin(q.x * 1.1 + t * 0.37 - q.y * 1.6);
        let field = (a * b) * 0.5 + 0.5;
        // Density is how BROKEN the wash is: a sparse reverb still has
        // separable reflections this far out, a dense one is smooth.
        let broken = mix(smoothstep(0.35, 0.95, field), field, density);
        let wash = broken * env * inside * 0.30 * (0.4 + 0.6 * density);
        rgb += mix(tint, hot, 0.35) * wash;
        alpha += wash * 0.55;
    }

    // ── The arrival ─────────────────────────────────────────────────────
    //
    // A bright mark where the first reflection lands. With the empty gap
    // beside it, this is the pre-delay stated as a picture.
    let arrive = exp(-abs(at - predelay) * 240.0 / window) * (1.0 - off * 0.5);
    rgb += hot * arrive * 0.7;
    alpha += arrive * 0.6;

    // Driven before the tone map. Reinhard on raw contributions crushes
    // everything below 1.0, and 1.0 is brighter than anything this panel
    // produces — so without the drive the whole picture sits in the bottom
    // quarter of the curve and reads as switched off.
    rgb = rgb * 2.2;
    rgb = rgb / (rgb + vec3<f32>(1.0));
    alpha = clamp(alpha, 0.0, 0.92);
    return vec4<f32>(rgb * alpha, alpha);
}
