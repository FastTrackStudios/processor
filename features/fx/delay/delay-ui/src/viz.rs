//! What the delay is actually doing, painted.
//!
//! This panel was knobs and a number. A delay's character is its taps — where
//! they land against the beat, how fast they give up, whether they walk across
//! the stereo field — and none of that is legible from `TIME 0.42 · FB 0.28`.
//! The picture's whole job is that a quarter-note delay is one you can *see*
//! is a quarter note, because its taps sit on the gridlines.
//!
//! # Two painters, one widget
//!
//! [`Widget::paint`] records into an `anyrender::Scene` — a command list, not
//! a wgpu call — so whatever can replay it can draw this: the GPU backend, the
//! CPU one, and a canvas in a browser. That is the **fallback**, and it is not
//! a poor relation: vello rasterises it with real gradients and blurs.
//!
//! The **shader** path is the escape hatch the widget trait describes: take
//! the device in [`Widget::can_create_surfaces`], render WGSL into a texture,
//! and hand the scene that texture's id. It is asked for per frame rather than
//! decided per build, so a renderer that cannot do it simply gets the vectors.
//!
//! # Not a signal, a cell
//!
//! Paint happens inside Blitz's traversal, which is not the Dioxus runtime;
//! reading a `Signal` there is a panic waiting for the first frame that takes
//! a different path. What this needs is a handful of numbers, and numbers can
//! live in a `RefCell` the component writes and the widget reads.

use std::cell::RefCell;
use std::rc::Rc;

use anyrender::{PaintScene, RenderContext, Scene};
use blitz_dom::node::{ComputedStyles, Widget};
use dioxus::prelude::*;
use fts_audio_ui::paint::lane;
use fts_audio_ui::shader::ShaderSurface;

use vello::kurbo::{Affine, BezPath, Circle, Line, Point, Rect, Stroke};
use vello::peniko::{Color, ColorStop, Fill, Gradient};

use lane::faded;

/// The most taps the shader carries. A uniform array is fixed-length, and a
/// tail longer than this is past the point where individual repeats can be
/// told apart anyway.
pub const MAX_TAPS: usize = 32;

/// One delay tap: when it lands, how loud, and where it sits in the field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tap {
    /// Seconds after the dry hit.
    pub at: f32,
    /// Linear level, 0..=1.
    pub level: f32,
    /// −1 hard left, +1 hard right.
    pub pan: f32,
}

/// What the delay panel draws.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DelayView {
    pub taps: Vec<Tap>,
    /// Seconds the window shows — the tail's own length, so a long delay is
    /// not drawn as a wall of taps against the left edge.
    pub window: f32,
    /// Wet/dry, for how much of the panel the taps are allowed to own.
    pub mix: f32,
    /// Engaged; a bypassed delay draws its shape unlit rather than vanishing,
    /// because "off" and "not configured" must not look the same.
    pub on: bool,
    /// Seconds per beat. The whole point of the picture: a tap that lands on a
    /// gridline is a tap in time.
    pub beat: f32,
    /// What the division is called ("1/4", "1/8."), if the block is locked to
    /// one. Carried for the DOM half of the lane; the painter draws no text.
    pub division: String,
    /// The lane's own colour, as the panel draws it. Delay is blue-led and
    /// reverb purple-led, and a painted lane that ignored that would be the
    /// one thing on screen disagreeing about which effect it is.
    pub color: [u8; 3],
    /// Seconds since the panel appeared — the animation's clock.
    ///
    /// Read from the widget's own `Instant` rather than pushed in by the
    /// component: a repaint can happen for reasons the component knows
    /// nothing about, and an animation that only advances when a prop changes
    /// is an animation that stutters.
    pub time: f32,
}

/// The numbers a widget reads, written by the component that owns it.
pub type Shared<T> = Rc<RefCell<T>>;

/// The WGSL the shader path runs.
const DELAY_SHADER: &str = include_str!("viz.wgsl");

/// The uniform block, laid out to match `Delay` in `viz.wgsl`.
///
/// Every row is a `vec4`, so there is no padding for the two languages to
/// disagree about.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DelayUniforms {
    /// `[width, height, seconds, tap_count]`.
    pub frame: [f32; 4],
    /// `[window_s, mix, on, beat_s]`.
    pub params: [f32; 4],
    /// The lane's colour; `w` unused.
    pub color: [f32; 4],
    /// `[at_s, level, pan, _]` per tap.
    pub taps: [[f32; 4]; MAX_TAPS],
}

impl Default for DelayUniforms {
    fn default() -> Self {
        Self {
            frame: [0.0; 4],
            params: [0.0; 4],
            color: [0.0; 4],
            taps: [[0.0; 4]; MAX_TAPS],
        }
    }
}

impl DelayUniforms {
    /// Fill from a view. Taps past [`MAX_TAPS`] are dropped, which is the
    /// right failure: they are the quietest ones, at the far end of the tail.
    #[must_use]
    pub fn of(view: &DelayView, w: f32, h: f32) -> Self {
        let mut u = Self::default();
        let [r, g, b] = view.color;
        u.frame = [w, h, view.time, 0.0];
        u.params = [
            view.window.max(0.05),
            view.mix,
            if view.on { 1.0 } else { 0.0 },
            view.beat.max(1e-3),
        ];
        u.color = [
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
            1.0,
        ];
        let n = view.taps.len().min(MAX_TAPS);
        for (slot, tap) in u.taps.iter_mut().zip(&view.taps[..n]) {
            *slot = [tap.at, tap.level, tap.pan, 0.0];
        }
        u.frame[3] = n as f32;
        u
    }
}

/// The delay's taps, painted.
pub struct DelayWidget {
    view: Shared<DelayView>,
    /// Built once, from whatever `can_create_surfaces` hands over. `None` on a
    /// renderer with no device to give, which is not an error: the vector
    /// painter below draws the same taps.
    gpu: Option<ShaderSurface>,
    uniforms: DelayUniforms,
    born: std::time::Instant,
}

impl DelayWidget {
    #[must_use]
    pub fn new(view: Shared<DelayView>) -> Self {
        Self {
            view,
            gpu: None,
            uniforms: DelayUniforms::default(),
            born: std::time::Instant::now(),
        }
    }
}

impl Widget for DelayWidget {
    fn can_create_surfaces(&mut self, render_ctx: &mut dyn RenderContext) {
        self.gpu = render_ctx.renderer_specific_context().and_then(|ctx| {
            ShaderSurface::with_uniform_size(
                ctx,
                DELAY_SHADER,
                std::mem::size_of::<DelayUniforms>() as u64,
            )
        });
    }

    fn paint(
        &mut self,
        render_ctx: &mut dyn RenderContext,
        _styles: &ComputedStyles,
        width: u32,
        height: u32,
        _scale: f64,
    ) -> Scene {
        let mut scene = Scene::new();
        let (w, h) = (f64::from(width), f64::from(height));
        if w < 2.0 || h < 2.0 {
            return scene;
        }
        let mut view = self.view.borrow().clone();
        view.time = self.born.elapsed().as_secs_f32();

        // Asked every frame rather than decided once: a renderer can decline
        // for reasons that change frame to frame, such as a surface that is
        // not up yet.
        if self.gpu.is_some() {
            self.uniforms = DelayUniforms::of(&view, w as f32, h as f32);
            let bytes = bytemuck::bytes_of(&self.uniforms);
            let drew = self
                .gpu
                .as_mut()
                .is_some_and(|gpu| gpu.draw_raw(render_ctx, &mut scene, width, height, bytes));
            if drew {
                // The grid is geometry on purpose: a 1 px line is the one
                // thing a shader is worse at than the vector pass, and the
                // grid is the ruler everything else is read against.
                lane::beats(
                    &mut scene,
                    w,
                    h,
                    f64::from(view.window.max(0.05)),
                    f64::from(view.beat),
                    view.on,
                );
                return scene;
            }
        }

        paint_delay(&mut scene, &view, w, h);
        scene
    }
}

// ── The fallback painter ────────────────────────────────────────────────────

/// The taps, on a time axis, with the tail they imply.
pub fn paint_delay(scene: &mut Scene, view: &DelayView, w: f64, h: f64) {
    let (signal, accent) = lane::palette(view.on, view.color);
    let mid = h * 0.5;
    let window = f64::from(view.window.max(0.05));

    // A ground that darkens to the right: time running out.
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        &Gradient::new_linear(Point::new(0.0, 0.0), Point::new(w, 0.0)).with_stops([
            ColorStop::from((0.0, faded(signal, 0.10))),
            ColorStop::from((1.0, Color::from_rgba8(0, 0, 0, 0))),
        ]),
        None,
        &Rect::new(0.0, 0.0, w, h),
    );

    lane::beats(scene, w, h, window, f64::from(view.beat), view.on);

    // The centre line — the stereo axis the taps hang off.
    scene.stroke(
        &Stroke::new(1.0),
        Affine::IDENTITY,
        faded(signal, 0.22),
        None,
        &Line::new(Point::new(0.0, mid), Point::new(w, mid)),
    );

    // The dry hit, at zero.
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        faded(signal, 0.9),
        None,
        &Rect::new(0.0, mid - h * 0.42, 2.0, mid + h * 0.42),
    );

    // The envelope the taps decay along, as a filled curve — the shape of the
    // tail, which is the thing a number cannot show.
    if view.taps.len() > 1 {
        let mut env = BezPath::new();
        env.move_to((0.0, mid));
        for tap in &view.taps {
            let x = f64::from(tap.at) / window * w;
            let y = mid - f64::from(tap.level) * h * 0.42;
            env.line_to((x, y));
        }
        env.line_to((w, mid));
        for tap in view.taps.iter().rev() {
            let x = f64::from(tap.at) / window * w;
            let y = mid + f64::from(tap.level) * h * 0.42;
            env.line_to((x, y));
        }
        env.close_path();
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &Gradient::new_linear(Point::new(0.0, 0.0), Point::new(w, 0.0)).with_stops([
                ColorStop::from((0.0, faded(accent, 0.34))),
                ColorStop::from((1.0, faded(accent, 0.02))),
            ]),
            None,
            &env,
        );
    }

    // The division's NAME is not drawn here. Text in a scene needs a font
    // handle the widget does not have, and an empty chip is worse than no
    // chip — the panel's own "1/4" selector is inches away, and what this
    // picture adds is that the taps sit on the grid, which needs no caption.

    // The playhead: a pulse crossing the window once per cycle, so the panel
    // keeps the tempo even when nothing is being played into it.
    //
    // Not decoration. The taps are static geometry — they say *where* the
    // repeats land — and the sweep is what makes that a rhythm you can read
    // at a glance rather than a row of sticks.
    let beat = f64::from(view.beat).max(1e-3);
    let cycle = beat * ((window / beat).ceil()).max(1.0);
    let head = if view.on && cycle > 0.0 {
        (f64::from(view.time) % cycle) / cycle
    } else {
        -1.0
    };

    // Each tap: an impulse whose height is its level and whose offset from the
    // centre is its pan, with a head bright enough to count at a glance. A tap
    // blooms as the sweep reaches it and falls back over the next beat.
    for tap in &view.taps {
        let x = f64::from(tap.at) / window * w;
        if x > w {
            continue;
        }
        let level = f64::from(tap.level).clamp(0.0, 1.0);

        // How recently the playhead passed this tap, 0..=1.
        let hit = if head < 0.0 {
            0.0
        } else {
            let at = f64::from(tap.at) / window;
            let since = (head - at + 1.0) % 1.0;
            // A bloom that dies within a beat, so two taps a beat apart never
            // glow at once and the eye follows one moving highlight.
            let over = beat / cycle;
            if since < over {
                (1.0 - since / over).powi(3)
            } else {
                0.0
            }
        };

        let reach = level * h * 0.42 * (1.0 + 0.22 * hit);
        let pan = f64::from(tap.pan).clamp(-1.0, 1.0);
        let y = mid - pan * h * 0.16;

        // The bloom, behind: a soft halo that only exists while lit.
        if hit > 0.01 {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                faded(accent, (0.30 * hit) as f32),
                None,
                &Circle::new(Point::new(x, y), 4.0 + 16.0 * hit * (0.4 + level)),
            );
        }

        scene.stroke(
            &Stroke::new(2.0 + 1.5 * hit),
            Affine::IDENTITY,
            faded(signal, (0.35 + 0.65 * level + 0.6 * hit) as f32),
            None,
            &Line::new(Point::new(x, y - reach), Point::new(x, y + reach)),
        );
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            faded(signal, (0.5 + 0.5 * level + 0.5 * hit) as f32),
            None,
            &Circle::new(Point::new(x, y), 1.5 + 2.0 * level + 2.5 * hit),
        );
    }

    // The sweep itself — a thin bright edge with a trail behind it.
    if head >= 0.0 {
        let hx = head * w;
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &Gradient::new_linear(Point::new(hx - w * 0.08, 0.0), Point::new(hx, 0.0)).with_stops([
                ColorStop::from((0.0, faded(signal, 0.0))),
                ColorStop::from((1.0, faded(signal, 0.16))),
            ]),
            None,
            &Rect::new((hx - w * 0.08).max(0.0), 0.0, hx.max(0.0), h),
        );
        scene.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            faded(signal, 0.5),
            None,
            &Line::new(Point::new(hx, 0.0), Point::new(hx, h)),
        );
    }
}


// ── Mounting ────────────────────────────────────────────────────────────────

/// Mark this scope dirty ~40 times a second, for as long as it lives.
///
/// Blitz repaints when the document changes, and an animation changes nothing
/// in the DOM — the movement is inside a widget's scene. So the clock has to
/// come from outside: a thread that pokes the runtime. `schedule_update` is
/// documented as safe to call from off the runtime, which is what this is.
pub fn use_repaint_clock() {
    use_hook(|| {
        let updater = dioxus_core::schedule_update();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(25));
                updater();
            }
        });
    });
}

/// A delay lane, painted by [`DelayWidget`].
///
/// A component per lane because `CustomWidgetAttr` is write-once: the widget
/// is built in a hook that runs once, and every later render pushes numbers
/// through the shared cell instead of rebuilding it. Rebuilding would hand
/// Blitz a second widget for the same node and lose the first one's state.
///
/// Taps arrive as `(milliseconds, amplitude, upper)` because that is the shape
/// a delay block reports; seconds and a pan are what the picture is drawn in.
#[component]
pub fn DelayViz(
    taps: Vec<(f32, f32, bool)>,
    win_ms: f32,
    on: bool,
    beat_ms: f32,
    division: String,
    color: [u8; 3],
) -> Element {
    use_repaint_clock();
    let view: Shared<DelayView> = use_hook(|| Rc::new(RefCell::new(DelayView::default())));
    let attr =
        use_hook(|| dioxus_native_dom::CustomWidgetAttr::new(DelayWidget::new(Rc::clone(&view))));

    *view.borrow_mut() = view_of(&taps, win_ms, on, beat_ms, division, color);

    rsx! {
        object {
            "data": attr,
            style: "position:absolute; top:0; left:0; right:0; bottom:0; \
                    width:100%; height:100%; display:block; pointer-events:none;",
        }
    }
}

/// The reported taps, as the picture draws them.
///
/// Levels are normalised to the loudest tap. The reported amplitudes start at
/// the wet mix, so a delay at 8% sits in the bottom twentieth of the lane and
/// its decay is invisible. What the picture is for is the PATTERN — where the
/// taps land and how fast they give up — and both survive normalising; the
/// absolute level is on the MIX knob two inches away.
fn view_of(
    taps: &[(f32, f32, bool)],
    win_ms: f32,
    on: bool,
    beat_ms: f32,
    division: String,
    color: [u8; 3],
) -> DelayView {
    let peak = taps.iter().map(|(_, a, _)| *a).fold(0.0f32, f32::max);
    let scale = if peak > f32::EPSILON { 1.0 / peak } else { 1.0 };
    DelayView {
        taps: taps
            .iter()
            .map(|(t, amp, up)| Tap {
                at: t / 1000.0,
                level: (amp * scale).clamp(0.0, 1.0),
                pan: if *up { -0.8 } else { 0.8 },
            })
            .collect(),
        window: win_ms / 1000.0,
        mix: 1.0,
        on,
        beat: beat_ms / 1000.0,
        division,
        color,
        // The widget keeps its own clock; this is only a starting value.
        time: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn taps(n: usize) -> Vec<Tap> {
        (0..n)
            .map(|i| Tap {
                at: 0.4 * (i as f32 + 1.0),
                level: 0.8f32.powi(i as i32 + 1),
                pan: if i % 2 == 0 { -0.7 } else { 0.7 },
            })
            .collect()
    }

    fn view() -> DelayView {
        DelayView {
            taps: taps(6),
            window: 2.0,
            mix: 0.3,
            on: true,
            beat: 0.4,
            division: "1/4".to_string(),
            color: [56, 189, 248],
            time: 0.0,
        }
    }

    /// Painting produces a scene rather than panicking, at the sizes a panel
    /// actually takes — including the degenerate ones a flex row hands out
    /// mid-layout.
    #[test]
    fn it_paints_at_every_size() {
        for (w, h) in [(2.0, 2.0), (120.0, 40.0), (1280.0, 300.0)] {
            let mut scene = Scene::new();
            paint_delay(&mut scene, &view(), w, h);
        }
    }

    /// A delay locked to a quarter note puts a tap on every beat line — the
    /// whole reason the grid is drawn. Checked as geometry rather than pixels:
    /// tap `n` sits at `n` beats, so it shares an x with gridline `n`.
    #[test]
    fn a_quarter_note_delay_lands_on_the_grid() {
        let beat = 0.4_f32;
        let taps: Vec<Tap> = (1..=4)
            .map(|n| Tap {
                at: beat * n as f32,
                level: 0.8_f32.powi(n),
                pan: 0.0,
            })
            .collect();
        let window = 8.0_f64 * f64::from(beat);
        for (i, tap) in taps.iter().enumerate() {
            let tap_x = f64::from(tap.at) / window;
            let line_x = f64::from(beat) * (i + 1) as f64 / window;
            // A fraction of the window, so the tolerance means something on
            // screen: 1e-6 of a 2560px panel is three thousandths of a pixel.
            assert!(
                (tap_x - line_x).abs() < 1e-6,
                "tap {i} at {tap_x} should sit on gridline at {line_x}"
            );
        }
    }

    /// The animation stays inside the panel and repeats: a sweep that runs off
    /// the end, or never comes back, is a sweep nobody can read a tempo from.
    #[test]
    fn the_sweep_wraps_and_stays_in_frame() {
        let mut v = view();
        for step in 0..200 {
            v.time = step as f32 * 0.05;
            let mut scene = Scene::new();
            paint_delay(&mut scene, &v, 640.0, 56.0);
        }
    }

    /// A bypassed block does not animate — a panel that is not in the signal
    /// path must not look like one that is.
    #[test]
    fn bypassed_does_not_sweep() {
        let mut v = view();
        v.on = false;
        v.time = 1.7;
        let mut scene = Scene::new();
        paint_delay(&mut scene, &v, 640.0, 56.0);
    }

    /// The reported amplitudes start at the wet mix, so the loudest tap is
    /// pulled up to the top of the lane and the pattern is readable whatever
    /// the mix knob says.
    #[test]
    fn levels_are_normalised_to_the_loudest_tap() {
        let reported = vec![(400.0, 0.08, true), (800.0, 0.04, false)];
        let v = view_of(&reported, 2000.0, true, 400.0, "1/4".into(), [56, 189, 248]);
        assert!((v.taps[0].level - 1.0).abs() < 1e-6);
        assert!((v.taps[1].level - 0.5).abs() < 1e-6);
    }

    /// Silence must not divide by its own peak.
    #[test]
    fn a_silent_delay_normalises_to_nothing() {
        let v = view_of(&[(400.0, 0.0, true)], 2000.0, true, 400.0, String::new(), [1, 2, 3]);
        assert_eq!(v.taps[0].level, 0.0);
    }

    /// The uniform block is four-float rows all the way down, so what Rust
    /// writes and what WGSL reads cannot drift apart over padding.
    #[test]
    fn the_uniform_block_is_vec4_rows() {
        assert_eq!(std::mem::size_of::<DelayUniforms>() % 16, 0);
        // frame + params + color + MAX_TAPS rows.
        assert_eq!(
            std::mem::size_of::<DelayUniforms>(),
            (3 + MAX_TAPS) * 16
        );
    }

    /// A tail longer than the array is truncated rather than overflowing, and
    /// the count the shader reads matches what was actually written.
    #[test]
    fn more_taps_than_the_array_holds_are_dropped() {
        let mut v = view();
        v.taps = taps(MAX_TAPS + 9);
        let u = DelayUniforms::of(&v, 640.0, 56.0);
        assert_eq!(u.frame[3], MAX_TAPS as f32);
    }

    /// The shader compiles, as `compose` assembles it — not on its own.
    ///
    /// Nothing else in the tree would notice if it did not: a `ShaderSurface`
    /// that fails to build is indistinguishable from a renderer that declined,
    /// so the panel would silently draw vectors forever. Validating the
    /// fragment alone is not enough either — it passes happily while the
    /// composed module collides with the prelude.
    #[test]
    fn the_shader_compiles_and_validates() {
        let source = fts_audio_ui::shader::compose(DELAY_SHADER);
        let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| {
            panic!("the delay shader does not parse: {}", e.emit_to_string(&source))
        });
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        );
        if let Err(e) = validator.validate(&module) {
            panic!("the delay shader does not validate: {e:?}");
        }
    }
}
