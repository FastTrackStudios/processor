//! A painted panel in a browser: the widget itself, on a canvas.
//!
//! Natively a painted panel is a Blitz *custom widget*: the host calls
//! `can_create_surfaces` once (where the widget takes a GPU device and builds
//! its [`ShaderSurface`](crate::shader::ShaderSurface)), then `paint` each
//! frame, and composites the returned [`anyrender::Scene`] into its own pass.
//!
//! A DOM renderer cannot be handed a scene at all, so the browser gets a
//! surface of its own: a `<canvas>` with vello on WebGPU behind it, driven
//! through the same two calls. The panel is the widget — the same struct the
//! plugin mounts, running the same code — so the browser gets the shader
//! layers too, not a vector imitation of them.
//!
//! That is the whole point of taking `anyrender`'s painter rather than a
//! renderer of our own: `VelloScenePainter` is both a [`PaintScene`] *and* a
//! [`RenderContext`], so `renderer_specific_context()` hands the widget a
//! real `wgpu` device and `try_register_custom_resource` takes the texture
//! its shader drew into. Nothing about the widget knows which host it is in.
//!
//! The clock matches the native one (the effects' `use_repaint_clock`): a
//! visualiser animates without anything in the DOM changing, so the canvas
//! repaints on a timer rather than on a re-render.

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::Arc;

use anyrender::{PaintScene, RenderContext, Scene, WindowRenderer};
use anyrender_vello::VelloWindowRenderer;
use dioxus::prelude::*;
use kurbo::Affine;
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle,
    WebCanvasWindowHandle, WindowHandle,
};
use wasm_bindgen::{JsCast, JsValue};

/// Repaints a second — the native clock's rate.
const FPS: u32 = 40;

thread_local! {
    /// Canvas ids are unique per document, and a panel needs one to find its
    /// own canvas again from the timer.
    static NEXT_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// What a painted panel does, on any host: the two calls Blitz makes.
///
/// Implemented by the effects' widgets themselves (`CompWidget`,
/// `DelayWidget`, …), so the browser runs the plugin's own painting code.
pub trait CanvasPanel: 'static {
    /// Take a GPU device, if the host has one — where a widget builds its
    /// shader surface. Called once, when the renderer becomes active.
    fn can_create_surfaces(&mut self, _ctx: &mut dyn RenderContext) {}

    /// Record this frame. `width`/`height` are the canvas's backing store
    /// (device pixels), `scale` the device pixel ratio.
    fn paint(&mut self, ctx: &mut dyn RenderContext, width: u32, height: u32, scale: f64) -> Scene;
}

/// A [`CanvasPanel`] as a component prop: shared, and compared by identity.
#[derive(Clone)]
pub struct Panel(Rc<RefCell<dyn CanvasPanel>>);

impl Panel {
    #[must_use]
    pub fn new(panel: impl CanvasPanel) -> Self {
        Self(Rc::new(RefCell::new(panel)))
    }
}

impl PartialEq for Panel {
    /// Same panel, not "same contents": a widget is a thing with state, and
    /// two of them are never interchangeable.
    fn eq(&self, other: &Self) -> bool {
        std::ptr::addr_eq(Rc::as_ptr(&self.0), Rc::as_ptr(&other.0))
    }
}

/// The canvas as something wgpu can make a surface on.
struct CanvasWindow(web_sys::HtmlCanvasElement);

impl HasWindowHandle for CanvasWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let value: &JsValue = self.0.as_ref();
        let obj: NonNull<c_void> = NonNull::from(value).cast();
        let handle = WebCanvasWindowHandle::new(obj);
        // SAFETY: the handle borrows this canvas's `JsValue`, and `self`
        // outlives the borrow — the renderer holds this window in an `Arc`
        // for as long as it has a surface on it.
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::WebCanvas(handle)) })
    }
}

impl HasDisplayHandle for CanvasWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::web())
    }
}

/// The renderer for one canvas, kept between frames: building a device per
/// frame would cost far more than drawing.
struct Surface {
    renderer: VelloWindowRenderer,
    /// Held for the renderer's surface (see [`CanvasWindow::window_handle`]).
    _window: Arc<CanvasWindow>,
    /// The backing-store size the surface was made for.
    size: (u32, u32),
    /// The renderer is active and has been offered a device.
    ready: bool,
}

/// A painted panel on a canvas: the widget draws `FPS` times a second, with
/// a real GPU device behind it.
#[component]
pub fn SceneCanvas(
    /// The widget doing the painting.
    panel: Panel,
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
        format!("fts-scene-canvas-{n}")
    });
    let surface: Rc<RefCell<Option<Surface>>> = use_hook(|| Rc::new(RefCell::new(None)));

    use_future({
        let (id, surface, panel) = (id.clone(), surface.clone(), panel.clone());
        move || {
            let (id, surface, panel) = (id.clone(), surface.clone(), panel.clone());
            async move {
                loop {
                    gloo_timers::future::TimeoutFuture::new(1000 / FPS).await;
                    draw(&id, &surface, &panel);
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
            //
            // `pointer-events: none` because this is a picture, not a
            // control: a positioned canvas paints above the gesture layer
            // that sits with it (the EQ's svg, the compressor's), and
            // without this it would swallow every press meant for that.
            style: "display: block; width: 100%; height: 100%; pointer-events: none;",
        }
    }
}

/// One frame: size the surface, then let the widget paint into it.
fn draw(id: &str, surface: &Rc<RefCell<Option<Surface>>>, panel: &Panel) {
    let Some(canvas) = canvas_by_id(id) else {
        return;
    };
    let (css_w, css_h) = (
        f64::from(canvas.client_width()),
        f64::from(canvas.client_height()),
    );
    if css_w < 1.0 || css_h < 1.0 {
        return;
    }
    let dpr = web_sys::window()
        .map_or(1.0, |w| w.device_pixel_ratio())
        .max(1.0);
    #[expect(
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation,
        reason = "a pixel count"
    )]
    let (w, h) = ((css_w * dpr) as u32, (css_h * dpr) as u32);

    let mut slot = surface.borrow_mut();
    if slot.is_none() {
        canvas.set_width(w);
        canvas.set_height(h);
        let window = Arc::new(CanvasWindow(canvas.clone()));
        // Transparent, not the renderer's default white: these panels are
        // layers over the page's own ground, and several paint no ground of
        // their own.
        let mut renderer = VelloWindowRenderer::with_options(anyrender_vello::VelloRendererOptions {
            base_color: peniko::Color::TRANSPARENT,
            ..Default::default()
        });
        // Async on wasm: `complete_resume` below finishes it, on some later
        // frame. Nothing is drawn until it does.
        renderer.resume(window.clone(), w, h, || {});
        *slot = Some(Surface {
            renderer,
            _window: window,
            size: (w, h),
            ready: false,
        });
    }
    let Some(surface) = slot.as_mut() else { return };

    if !surface.ready {
        if !surface.renderer.complete_resume() {
            return;
        }
        surface.ready = true;
        // Where a widget takes the device and builds its shader surface —
        // the same call, in the same order, Blitz makes natively. The
        // renderer is itself a `RenderContext`, and carries the device.
        panel
            .0
            .borrow_mut()
            .can_create_surfaces(&mut surface.renderer);
    }

    if surface.size != (w, h) {
        canvas.set_width(w);
        canvas.set_height(h);
        surface.renderer.set_size(w, h);
        surface.size = (w, h);
    }

    let panel = panel.0.clone();
    surface.renderer.render(|painter| {
        let scene = panel.borrow_mut().paint(painter, w, h, dpr);
        painter.append_scene(scene, Affine::IDENTITY);
    });
}

fn canvas_by_id(id: &str) -> Option<web_sys::HtmlCanvasElement> {
    web_sys::window()?
        .document()?
        .get_element_by_id(id)?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .ok()
}
