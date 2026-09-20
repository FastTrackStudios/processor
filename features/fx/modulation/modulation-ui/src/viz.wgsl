// The modulation panels, on the GPU.
//
// Every engine here works by interference — copies of a signal against
// itself, displaced in time or pitch, reinforcing and cancelling. So the
// picture is interference: fields beating against each other at the engine's
// own rate and depth, which is both what the effect does and the reason it
// looks the way it does.
//
// `u.frame` is [width, height, seconds, engine]; `u.params` is
// [rate_hz, depth, mix, engaged]; `u.color` is the group's colour.

const CHORUS:  f32 = 0.0;
const FLANGER: f32 = 1.0;
const PHASER:  f32 = 2.0;
const TREMOLO: f32 = 3.0;
const VIBRATO: f32 = 4.0;
const ROTARY:  f32 = 5.0;

const TAU: f32 = 6.28318530718;

// A cheap smooth noise, for the grain that keeps a flat field from looking
// like a gradient.
fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash(i + vec2<f32>(0.0, 0.0)), hash(i + vec2<f32>(1.0, 0.0)), u.x),
        mix(hash(i + vec2<f32>(0.0, 1.0)), hash(i + vec2<f32>(1.0, 1.0)), u.x),
        u.y
    );
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let t = u.frame.z;
    let engine = u.frame.w;
    let rate = max(u.params.x, 0.01);
    let depth = clamp(u.params.y, 0.0, 1.0);
    let engaged = u.params.w;

    // The LFO every engine is driven by, so they share a heartbeat even when
    // they draw differently.
    let phase = t * rate;
    let lfo = sin(phase * TAU);

    // Distance from the centre line, which is where most of these live.
    let mid = abs(uv.y - 0.5) * 2.0;

    var field = 0.0;

    if (engine < CHORUS + 0.5) {
        // Three voices weaving apart and back together.
        //
        // Drawn as the voices themselves rather than as their interference
        // pattern. Summing three detuned sines is the honest maths and a bad
        // picture: the beat envelope is slow by construction, so it is one
        // bright lobe in the middle of the lane with two thirds of the panel
        // empty either side, and nothing about it says "three voices".
        //
        // What a chorus IS, to look at, is copies of one line pulling apart
        // and closing again. Each voice is a curve; where they converge the
        // light piles up, which is the thickening you hear.
        var v = 0.0;
        for (var i = 0; i < 3; i = i + 1) {
            let which = f32(i) - 1.0;
            // How far this voice has wandered from the dry one, breathing
            // with the LFO and travelling along the panel so the weave moves
            // rather than standing still.
            let wander = sin(uv.x * TAU * 1.6 - phase * TAU + which * 1.9);
            let spread = (0.06 + depth * 0.30) * which * wander;
            let y = 0.5 + spread;
            // A soft line, wider for the outer voices so the centre stays
            // the one that reads as the signal.
            // Thick enough to be the brightest thing on the panel. At
            // 0.012 a voice was under two pixels on a rig lane, which the
            // grain simply drowned.
            let thick = 0.055 + 0.030 * abs(which);
            let d = (uv.y - y) / thick;
            // A core with a wider halo around it, so the voices read as
            // light with body rather than as hairlines.
            v = v + exp(-d * d) + exp(-abs(d) * 0.8) * 0.45;
        }
        // Where two voices cross, the light adds — which is the whole point.
        field = clamp(v * 0.62, 0.0, 1.6);
    } else if (engine < FLANGER + 0.5) {
        // A comb whose teeth slide: cos of a frequency that sweeps.
        let sweep = (lfo * 0.5 + 0.5) * depth;
        let teeth = 6.0 + 26.0 * sweep;
        field = cos(uv.x * teeth * TAU) * (1.0 - mid * 0.5);
        // Sharpen toward notches, which is where a flanger lives.
        field = sign(field) * pow(abs(field), 0.45);
    } else if (engine < PHASER + 0.5) {
        // Four allpass notches travelling through the band.
        var v = 1.0;
        for (var i = 0; i < 4; i = i + 1) {
            let centre = fract(0.12 + 0.2 * f32(i) + (lfo * 0.5 + 0.5) * 0.3);
            let d = (uv.x - centre) / (0.035 + 0.02 * depth);
            v = v - depth * exp(-d * d);
        }
        field = (v - 0.5) * 2.0 * (1.0 - mid * 0.6);
    } else if (engine < TREMOLO + 0.5) {
        // Amplitude, pumping: the carrier's envelope is the LFO.
        //
        // The envelope floors at a fraction of full height rather than at
        // zero, so the quiet part of the cycle is still a waveform being
        // held down rather than a hole in the panel. A tremolo at full depth
        // is silent for part of its cycle, but silence in a picture is
        // indistinguishable from nothing loaded.
        let swing = sin((uv.x - phase) * TAU) * 0.5 + 0.5;
        let env = mix(1.0 - depth * 0.80, 1.0, swing);
        let carrier = sin(uv.x * 40.0 * TAU);
        field = carrier * env;
        field = field * step(mid, env);
    } else if (engine < VIBRATO + 0.5) {
        // Pitch, bending — drawn as a WAVEFORM whose wavelength stretches and
        // compresses, not as a field of bars.
        //
        // As bars it was a comb with uneven spacing, which is a flanger with
        // uneven spacing: the two engines came out looking like the same
        // picture, and they are not remotely the same effect. A single line
        // that visibly bunches and spreads says "the pitch is moving" in a
        // way a bar field cannot, because a bar field has no continuity for
        // the eye to follow along.
        let bend = depth * 1.3 * sin((uv.x * 1.6 - phase) * TAU);
        let wave = sin((uv.x * 16.0 + bend * 6.0) * TAU);
        let y = 0.5 + wave * 0.30;
        let d = abs(uv.y - y);
        // The trace, plus a wider glow so it has body at a rack lane's
        // height.
        field = exp(-(d * d) / 0.0016) + exp(-d * 9.0) * 0.40;
        // Where the cycles bunch, the line is doing more — brighten it, so
        // the modulation peak is readable without counting wavelengths.
        let bunching = abs(cos((uv.x * 1.6 - phase) * TAU)) * depth;
        field = field * (0.75 + 0.55 * bunching);
    } else {
        // A horn going round, seen from above: the orbit it travels, the
        // horn on it, and the doppler it throws into the room.
        //
        // Measured in ASPECT-CORRECTED space. `distance(uv, c)` on a lane
        // that is three times wider than it is tall squashes the orbit into
        // a sliver and puts almost all of the falloff in x — which is why
        // this drew a single blob near the bottom and nothing else.
        let aspect = max(u.frame.x, 1.0) / max(u.frame.y, 1.0);
        let p = vec2<f32>((uv.x - 0.5) * aspect, uv.y - 0.5);

        let a = phase * TAU;
        let radius = 0.42 * (0.62 + 0.38 * depth);
        // Wider than tall, because a rotor seen from the front is an ellipse
        // and the lane is wide.
        let c = vec2<f32>(cos(a) * radius * 1.9, sin(a) * radius);
        let d = distance(p, c);

        // The orbit itself, faint: the path is context for the horn on it.
        let on_path = abs(length(vec2<f32>(p.x / 1.9, p.y)) - radius);
        // Bright enough to be the picture rather than a hint of one: the
        // orbit is what says "this thing goes round", and the horn alone is
        // a dot moving in the dark.
        let path = exp(-on_path * 13.0) * 0.55;

        // The horn. Coming toward you it is bright and tight; going away it
        // is dim and smeared — which IS the doppler, drawn rather than
        // described.
        let toward = sin(a) * 0.5 + 0.5;
        let tight = mix(45.0, 170.0, toward);
        let horn = exp(-d * d * tight) * (0.85 + 1.05 * toward);

        // The wake it drags behind itself around the orbit.
        var wake = 0.0;
        for (var i = 1u; i < 5u; i = i + 1u) {
            let lag = f32(i) * 0.16;
            let ca = a - lag;
            let cp = vec2<f32>(cos(ca) * radius * 1.9, sin(ca) * radius);
            let dd = distance(p, cp);
            wake = wake + exp(-dd * dd * 70.0) * (0.55 / f32(i));
        }

        field = path + horn + wake;
    }

    // Grain, so a flat region reads as material rather than as a fill.
    //
    // Sampled on SQUARE cells. Scaling uv by the panel's pixel size gave
    // cells as wide as the panel and as tall — 35 across and 12 down on a
    // rig lane — so the grain came out as vertical streaks, and on a quiet
    // engine those streaks were the brightest thing on the panel. Square
    // cells read as material; rectangular ones read as a pattern the effect
    // is not making.
    let cell = max(u.frame.x, u.frame.y) * 0.05;
    let grain = (noise(uv * cell + t * 0.3) - 0.5) * 0.10;
    var lit = clamp(abs(field) + grain, 0.0, 1.5);

    // A bypassed engine keeps its shape and loses its light — "off" and
    // "not configured" must not look the same.
    lit = lit * mix(0.16, 1.0, engaged);

    // Colour: the group's hue, lifted toward white where the field is
    // strongest, so the bright parts read as energy rather than as a
    // different colour arriving.
    let base = u.color.rgb;
    let hot = mix(base, vec3<f32>(1.0), clamp(lit - 0.55, 0.0, 1.0) * 0.85);
    let rgb = hot * clamp(lit, 0.0, 1.0);
    let alpha = clamp(lit * 0.82, 0.0, 0.92);

    // ── The lane itself ─────────────────────────────────────────────────
    //
    // The panel is a tinted FIELD, not a black box with coloured marks on
    // it. Modulation is cyan-led and motion pink-led, and that grouping is
    // how you know which of the two rows you are reading without going to
    // the label — but it only worked where the engine happened to be
    // drawing. On a quiet engine, or in the wide empty margins a narrow
    // shape leaves, the panel was black and said nothing at all.
    //
    // Deep and desaturated: this is UNDER the field, and a ground bright
    // enough to compete with it would cost the picture its contrast. It also
    // keeps its colour into the corners rather than fading out, which is
    // exactly where the panel stops identifying itself.
    let ground_base = mix(vec3<f32>(0.012, 0.018, 0.024), base, 0.13);
    // A little brighter through the middle, where the engines live.
    let across = mix(1.20, 0.72, clamp(mid, 0.0, 1.0));
    let along = mix(1.06, 0.88, abs(uv.x - 0.5) * 2.0);
    // Bypassed keeps a ground, dimmed: an engine that is off should look
    // switched off, not unloaded.
    let ground_rgb = ground_base * across * along * mix(0.45, 1.0, engaged);
    let edge = 1.0 - smoothstep(0.80, 1.0, mid) * 0.40;
    let ground_a = 0.94 * edge;

    // Field over ground, composited rather than summed. The ground is not
    // light the panel is emitting, it is the panel.
    let out_a = alpha + ground_a * (1.0 - alpha);
    let out_rgb = rgb * alpha + ground_rgb * ground_a * (1.0 - alpha);
    return vec4<f32>(out_rgb, out_a);
}
