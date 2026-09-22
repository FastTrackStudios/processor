//! A painted panel in a browser: the effects' own scenes, on a canvas.
//!
//! Natively a painted panel is a Blitz *custom widget*: the effect records an
//! [`anyrender::Scene`] — a command list, not pixels — and Blitz composites it
//! into its own vello pass. A DOM renderer cannot be handed a scene at all,
//! so the browser needs a surface of its own. It gets one: a `<canvas>` with
//! vello_hybrid's WebGL2 renderer behind it, replaying the very same scene
//! the effect records for the plugin.
//!
//! So the rig draws the same pictures in both places, from one painter —
//! `delay_ui::viz::paint_delay`, `comp_ui::viz::paint_comp` and the rest —
//! rather than a painted widget natively and a hand-drawn imitation here.
//!
//! The clock matches the native one (the effects' `use_repaint_clock`): a
//! visualiser animates without anything in the DOM changing, so the canvas
//! repaints on a timer rather than on a re-render.

use std::cell::RefCell;
use std::rc::Rc;

use anyrender::{PaintScene, Scene};
use dioxus::prelude::*;
use kurbo::Affine;
use wasm_bindgen::JsCast;

/// Repaints a second — the native clock's rate.
const FPS: u32 = 40;

thread_local! {
    /// Canvas ids are unique per document, and a component needs one to find
    /// its own canvas again from the timer.
    static NEXT_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// What the picture is drawn into, kept between frames: making a WebGL
/// context per frame would be far slower than drawing.
struct Surface {
    renderer: vello_hybrid::WebGlRenderer,
    /// GPU resources (images, gradients) the renderer builds as it goes.
    resources: vello_hybrid::Resources,
    /// The backing-store size the renderer was made for.
    size: (u32, u32),
}

/// What a panel is asked to draw: its size, and how long it has been up.
///
/// The time is the canvas's own clock, as the custom widget keeps its own
/// natively — a visualiser animates on it (an LFO's phase, a delay's
/// travelling taps) without anything in the DOM changing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// CSS pixels.
    pub width: f64,
    /// CSS pixels.
    pub height: f64,
    /// Seconds since this canvas first drew.
    pub seconds: f64,
    /// Device pixels per CSS pixel — a host mapping a pointer back into the
    /// picture needs it, as the custom widget's `scale` gives it natively.
    pub scale: f64,
}

/// A painted visualiser on a canvas: `paint` records the picture at the
/// canvas's own CSS size, and it is replayed to the GPU `FPS` times a second.
#[component]
pub fn SceneCanvas(
    /// Records the picture for this frame.
    paint: Callback<Frame, Scene>,
    /// Extra classes for the canvas element.
    #[props(default)]
    class: String,
) -> Element {
    let id = use_hook(|| {
        let n = NEXT_ID.with(|c| {
            let n = c.get();
            c.set(n + 1);
            n
        });
        format!("signal-scene-canvas-{n}")
    });
    let surface: Rc<RefCell<Option<Surface>>> = use_hook(|| Rc::new(RefCell::new(None)));
    let started = use_hook(js_sys_now);

    use_future({
        let (id, surface) = (id.clone(), surface.clone());
        move || {
            let (id, surface) = (id.clone(), surface.clone());
            async move {
                loop {
                    gloo_timers::future::TimeoutFuture::new(1000 / FPS).await;
                    draw(&id, &surface, paint, (js_sys_now() - started) / 1000.0);
                }
            }
        }
    });

    rsx! {
        canvas {
            id: "{id}",
            class: "{class}",
            // The element sizes itself; the backing store follows it in
            // `draw`, at the device's pixel ratio.
            style: "display: block; width: 100%; height: 100%;",
        }
    }
}

/// One frame: size the backing store, record the picture, replay it.
fn draw(
    id: &str,
    surface: &Rc<RefCell<Option<Surface>>>,
    paint: Callback<Frame, Scene>,
    seconds: f64,
) {
    let Some(canvas) = canvas_by_id(id) else { return };
    let (css_w, css_h) = (canvas.client_width() as f64, canvas.client_height() as f64);
    if css_w < 1.0 || css_h < 1.0 {
        return;
    }
    let dpr = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio()).max(1.0);
    let (w, h) = ((css_w * dpr) as u32, (css_h * dpr) as u32);

    let mut slot = surface.borrow_mut();
    // A resized canvas needs a renderer for the new backing store.
    if slot.as_ref().is_none_or(|s| s.size != (w, h)) {
        canvas.set_width(w);
        canvas.set_height(h);
        *slot = Some(Surface {
            renderer: vello_hybrid::WebGlRenderer::new(&canvas),
            resources: vello_hybrid::Resources::new(),
            size: (w, h),
        });
    }
    let Some(surface) = slot.as_mut() else { return };

    // The effect paints in CSS pixels; the canvas is in device pixels.
    let recorded = paint.call(Frame { width: css_w, height: css_h, seconds, scale: dpr });
    let mut scene = vello_hybrid::Scene::new(w as u16, h as u16);
    {
        // These scenes are paths and gradients — no images — so the upload
        // cache has nothing to keep between frames.
        let mut images = rustc_hash::FxHashMap::default();
        let image_manager = anyrender_vello_hybrid::WebGlImageManager::new(
            &mut surface.renderer,
            &mut surface.resources,
            &mut images,
        );
        let mut painter = anyrender_vello_hybrid::WebGlScenePainter::new(&mut scene, image_manager);
        painter.append_scene(recorded, Affine::scale(dpr));
    }
    let _ = surface.renderer.render(
        &scene,
        &mut surface.resources,
        &vello_hybrid::RenderSize { width: w, height: h },
    );
}

/// Milliseconds on the page's clock.
fn js_sys_now() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}

fn canvas_by_id(id: &str) -> Option<web_sys::HtmlCanvasElement> {
    web_sys::window()?
        .document()?
        .get_element_by_id(id)?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .ok()
}
