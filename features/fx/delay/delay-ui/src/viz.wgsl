// The delay's taps, as light.
//
// The vector pass draws the same taps as geometry and is a perfectly good
// picture; what a shader adds is the part that is about energy rather than
// position — the bloom as the playhead crosses a repeat, the haze the tail
// leaves behind it, the way a loud tap spills past its own edge. Those are
// per-pixel effects, and drawing them as geometry means one shape per glow
// and a scene that grows with the tail.
//
// Layout of `u` mirrors `DelayUniforms` in viz.rs. `taps` is fixed-length
// because a uniform buffer is; `frame.w` carries how many are real.
//
// The beat grid is NOT here. A 1 px ruled line is the one thing the vector
// pass does better, and the grid is what the taps are read against.

const MAX_TAPS: u32 = 32u;
const TAU: f32 = 6.28318530718;

struct Delay {
    // width, height, seconds, tap_count
    frame: vec4<f32>,
    // window_s, mix, on, beat_s
    params: vec4<f32>,
    // the lane's colour; w unused
    color: vec4<f32>,
    // at_s, level, pan, unused
    taps: array<vec4<f32>, MAX_TAPS>,
};

@group(0) @binding(0) var<uniform> u: Delay;

// The playhead's position across the window, 0..=1, or −1 when the lane is
// bypassed. A bypassed delay keeps its shape and loses its motion: "off" and
// "not configured" must not look the same.
fn playhead() -> f32 {
    let window = max(u.params.x, 1e-3);
    let beat = max(u.params.w, 1e-3);
    if (u.params.z < 0.5) { return -1.0; }
    let cycle = max(beat * ceil(window / beat), beat);
    return fract(u.frame.z / cycle);
}

// How recently the playhead passed `at` (seconds), 0..=1, dying within a beat
// so two taps a beat apart never glow at once and the eye follows a single
// moving highlight.
fn recency(at: f32, head: f32) -> f32 {
    if (head < 0.0) { return 0.0; }
    let window = max(u.params.x, 1e-3);
    let beat = max(u.params.w, 1e-3);
    let cycle = max(beat * ceil(window / beat), beat);
    let pos = at / window;
    let since = fract(head - pos + 1.0);
    let over = beat / cycle;
    if (since >= over) { return 0.0; }
    let k = 1.0 - since / over;
    return k * k * k;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let px = uv * vec2<f32>(u.frame.x, u.frame.y);
    let t = u.frame.z;
    let lit = u.params.z > 0.5;

    let window = max(u.params.x, 1e-3);
    let mid = u.frame.y * 0.5;
    let head = playhead();

    // Bypassed keeps the picture and drops the colour, so the shape still
    // reads as a configured delay that happens to be off.
    var tint = u.color.rgb;
    if (!lit) { tint = vec3<f32>(0.32, 0.32, 0.36); }
    let hot = mix(tint, vec3<f32>(1.0), 0.55);

    var rgb = vec3<f32>(0.0);
    var alpha = 0.0;

    // ── Time running out ────────────────────────────────────────────────
    //
    // A ground that fades to the right. The panel is a window onto a tail,
    // and the far end of it is where the repeats have given up.
    let ground = (1.0 - uv.x) * 0.10;
    rgb += tint * ground;
    alpha += ground * 0.55;

    // ── The dry hit ─────────────────────────────────────────────────────
    //
    // At zero, full height: everything to the right of it is a repeat OF
    // this, which is the one relationship the picture has to establish.
    let dry_w = max(u.frame.x * 0.0018, 1.2);
    let dry = exp(-(px.x * px.x) / (dry_w * dry_w * 4.0))
        * (1.0 - 0.35 * abs(uv.y - 0.5) * 2.0);
    rgb += hot * dry * 0.9;
    alpha += dry * 0.8;

    // ── The taps ────────────────────────────────────────────────────────
    let count = u32(max(u.frame.w, 0.0));
    for (var i = 0u; i < MAX_TAPS; i = i + 1u) {
        if (i >= count) { break; }
        let tap = u.taps[i];
        let at = tap.x;
        let level = clamp(tap.y, 0.0, 1.0);
        let pan = clamp(tap.z, -1.0, 1.0);
        if (at > window) { continue; }

        let hit = recency(at, head);
        let x = at / window * u.frame.x;
        // Pan walks the tap off the centre line, so a ping-pong delay reads
        // as movement across the field rather than as a row of sticks.
        let y = mid - pan * u.frame.y * 0.16;
        let reach = level * u.frame.y * 0.42 * (1.0 + 0.22 * hit);

        let dx = px.x - x;
        let dy = px.y - y;

        // The stem: bright along its own column, cut off at the tap's reach.
        //
        // Width is in PIXELS and must not be a constant: a falloff tuned on a
        // 600 px panel is a hairline on a 2560 px one, which is how these
        // ended up all but invisible on the rig's own screen. Scaled off the
        // panel, with a floor so a narrow lane still draws something.
        let stem_w = max(u.frame.x * 0.0022, 1.4) * (1.0 + 0.5 * hit);
        let within = 1.0 - smoothstep(reach * 0.88, reach * 1.04, abs(dy));
        let stem = exp(-(dx * dx) / (stem_w * stem_w)) * within;
        rgb += tint * stem * (0.55 + 0.85 * level + 0.7 * hit);
        alpha += stem * (0.55 + 0.45 * level + 0.35 * hit);

        // The head: a bright point whose size is its level.
        let r = max(u.frame.x * 0.0035, 2.2) + 2.5 * level + 3.0 * hit;
        let d = length(vec2<f32>(dx, dy));
        let head_core = exp(-(d * d) / (r * r * 0.8));
        rgb += hot * head_core * (0.5 + 0.5 * level + 0.5 * hit);
        alpha += head_core * (0.5 + 0.4 * level + 0.4 * hit);

        // The bloom, only while lit by the sweep. This is the part that is
        // worth a GPU: one soft falloff per tap, per pixel, for free.
        if (hit > 0.01) {
            let br = 4.0 + 18.0 * hit * (0.4 + level);
            let bloom = exp(-d / br) * hit;
            rgb += tint * bloom * 0.55;
            alpha += bloom * 0.30;
        }
    }

    // ── The envelope ────────────────────────────────────────────────────
    //
    // The shape the tail decays along, as a haze rather than an outline: it
    // is context for the taps, and an edge would compete with them.
    if (count > 1u) {
        // Nearest-tap level at this x, so the haze follows the actual taps
        // rather than an idealised exponential the delay may not be doing.
        var env = 0.0;
        for (var i = 0u; i < MAX_TAPS; i = i + 1u) {
            if (i >= count) { break; }
            let tap = u.taps[i];
            let tx = tap.x / window;
            let w = exp(-abs(uv.x - tx) * 12.0);
            env = max(env, tap.y * w);
        }
        let span = env * 0.42;
        let inside = 1.0 - smoothstep(span * 0.7, span * 1.15, abs(uv.y - 0.5));
        let haze = inside * env * 0.40 * (1.0 - uv.x * 0.55);
        rgb += mix(tint, hot, 0.5) * haze;
        alpha += haze * 0.55;
    }

    // ── The sweep ───────────────────────────────────────────────────────
    //
    // A thin bright edge with a trail behind it. Not decoration: the taps are
    // static geometry saying *where* the repeats land, and the sweep is what
    // turns that into a rhythm readable at a glance.
    if (head >= 0.0) {
        let hx = head * u.frame.x;
        let behind = max(hx - px.x, 0.0);
        let trail = exp(-behind / (u.frame.x * 0.08)) * 0.16;
        let edge = exp(-abs(px.x - hx) / max(u.frame.x * 0.0012, 1.0)) * 0.55;
        rgb += tint * trail + hot * edge;
        alpha += trail * 0.7 + edge * 0.6;
        // A faint shimmer riding the edge, so the sweep has a leading face.
        let shimmer = edge * 0.25 * (0.5 + 0.5 * sin(px.y * 0.35 + t * 6.0));
        rgb += hot * shimmer;
        alpha += shimmer * 0.4;
    }

    // Tone-map rather than clamp: two overlapping blooms otherwise clip to
    // white and lose the colour in both, exactly where it matters most.
    // Driven before the tone map. Reinhard on raw contributions crushes
    // everything below 1.0, and 1.0 is brighter than anything this panel
    // produces — so without the drive the whole picture sits in the bottom
    // quarter of the curve and reads as switched off.
    rgb = rgb * 2.2;
    rgb = rgb / (rgb + vec3<f32>(1.0));
    alpha = clamp(alpha, 0.0, 0.92);
    // Premultiplied, because that is what the compositor expects.
    return vec4<f32>(rgb * alpha, alpha);
}
