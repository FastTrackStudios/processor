// The EQ's light: the spectrum as a field, and a bloom under every band node.
//
// Painted UNDER the vector graph, not instead of it. The curves, the grid and
// the node rings stay crisp geometry — they are read precisely, and a shader
// is the wrong tool for a 1 px line. What a shader is the right tool for is
// everything that should look like energy rather than like a drawing: the
// analyser's body, the haze where a band is lifting, the halo that says which
// node has focus.
//
// Layout of `u` mirrors `Glow` in eq_glow.rs. The arrays are fixed-length
// because a uniform buffer is: `bins` and `nodes` carry their own counts.

const MAX_BINS: u32 = 128u;
const MAX_NODES: u32 = 24u;
const TAU: f32 = 6.28318530718;

struct Glow {
    // width, height, seconds, bin_count
    frame: vec4<f32>,
    // node_count, db_range, spectrum_floor_db, unused
    cfg: vec4<f32>,
    // Each bin's magnitude in dB, packed four to a row.
    bins: array<vec4<f32>, 32>,
    // x, y, radius, strength — in pixels, already laid out by the graph.
    nodes: array<vec4<f32>, MAX_NODES>,
    // Each node's colour; w is unused.
    node_color: array<vec4<f32>, MAX_NODES>,
};

@group(0) @binding(0) var<uniform> u: Glow;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VsOut {
    var out: VsOut;
    let x = f32((idx << 1u) & 2u);
    let y = f32(idx & 2u);
    let uv = vec2<f32>(x, y) * 2.0;
    out.uv = vec2<f32>(uv.x, 1.0 - uv.y);
    out.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

// One bin, by index, out of the packed rows.
fn bin_at(i: u32) -> f32 {
    let row = u.bins[i / 4u];
    let lane = i % 4u;
    if (lane == 0u) { return row.x; }
    if (lane == 1u) { return row.y; }
    if (lane == 2u) { return row.z; }
    return row.w;
}

// The analyser's height at this x, 0..=1, smoothed across neighbours so the
// field reads as a body rather than as a bar chart.
fn spectrum_height(x: f32) -> f32 {
    let count = u32(max(u.frame.w, 1.0));
    if (count < 2u) { return 0.0; }
    let floor_db = u.cfg.z;
    let pos = clamp(x, 0.0, 1.0) * f32(count - 1u);
    let i = u32(floor(pos));
    let f = fract(pos);
    let j = min(i + 1u, count - 1u);
    let a = bin_at(i);
    let b = bin_at(j);
    let db = mix(a, b, f);
    return clamp((db - floor_db) / (0.0 - floor_db), 0.0, 1.0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let px = uv * vec2<f32>(u.frame.x, u.frame.y);
    let t = u.frame.z;

    var rgb = vec3<f32>(0.0);
    var alpha = 0.0;

    // ── The analyser ────────────────────────────────────────────────────
    //
    // A filled body with a lit rim, plus a soft bloom above it. The bloom is
    // what makes a loud band look loud rather than just tall: energy spilling
    // past its own edge is how light behaves, and the eye reads it before it
    // reads a height.
    let h = spectrum_height(uv.x);
    let top = 1.0 - h;
    let below = smoothstep(0.0, 0.012, uv.y - top);
    let body = below * (0.10 + 0.45 * h);

    // The rim, tight to the edge.
    let rim = exp(-abs(uv.y - top) * 190.0) * (0.35 + 0.65 * h);
    // The spill above it, wider where there is more energy.
    let spill = exp(-max(top - uv.y, 0.0) * (26.0 - 12.0 * h)) * 0.30 * h;

    // Cool at the bottom of the band, hot at the top — a spectrum that is all
    // one colour hides where the energy actually is.
    let warm = vec3<f32>(0.42, 0.78, 1.0);
    let hot = vec3<f32>(0.75, 0.93, 1.0);
    let spectrum_rgb = mix(warm, hot, h);

    rgb += spectrum_rgb * (body + rim * 1.5 + spill);
    alpha += body * 0.55 + rim * 0.85 + spill * 0.7;

    // ── The bands ───────────────────────────────────────────────────────
    //
    // A halo under each node, in the node's own colour, sized by its radius
    // and breathing very slightly so a static graph is never quite still.
    let count = u32(max(u.cfg.x, 0.0));
    for (var i = 0u; i < MAX_NODES; i = i + 1u) {
        if (i >= count) { break; }
        let n = u.nodes[i];
        let strength = n.w;
        if (strength <= 0.001) { continue; }

        let d = distance(px, n.xy);
        let r = max(n.z, 1.0);
        // Breath is per-node and out of phase, so a row of bands shimmers
        // rather than pulsing in unison like a warning light.
        let breath = 1.0 + 0.06 * sin(t * TAU * 0.35 + f32(i) * 1.7);
        let core = exp(-(d * d) / (r * r * 0.55 * breath));
        let halo = exp(-d / (r * 3.2 * breath)) * 0.45;
        let lit = (core + halo) * strength;

        rgb += u.node_color[i].rgb * lit;
        alpha += lit * 0.60;
    }

    // Tone-map rather than clamp: without this two overlapping halos clip to
    // white and the colour information in both is lost exactly where they
    // matter most.
    rgb = rgb / (rgb + vec3<f32>(1.0));
    alpha = clamp(alpha, 0.0, 0.88);

    // Premultiplied, because that is what the compositor expects of the
    // texture it is handed.
    return vec4<f32>(rgb * alpha, alpha);
}
