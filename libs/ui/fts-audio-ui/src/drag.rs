//! Global drag capture for parameter-edit gestures.
//!
//! Blitz dispatches mouse events to whichever element is under the cursor (no
//! pointer capture). [`DragProvider`] wraps the editor root and routes
//! `mousemove`/`mouseup` to whichever widget started a drag. Widgets call
//! [`begin_drag`] / [`begin_drag_axis`] from their own `onmousedown`.

use crate::param::ParamHandle;
use dioxus::prelude::*;

/// Drag axis. Vertical drag = up increases value (knobs, vertical sliders).
/// Horizontal drag = right increases value (horizontal sliders).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DragAxis {
    #[default]
    Vertical,
    Horizontal,
}

/// In-flight drag state shared between the active widget and [`DragProvider`].
#[derive(Clone, Default)]
pub struct DragState {
    pub active: bool,
    pub handle: Option<ParamHandle>,
    pub axis: DragAxis,
    pub start_value: f32,
    pub start_x: f64,
    pub start_y: f64,
    /// Pixels of drag per full 0→1 sweep.
    pub sensitivity: f64,
    /// Bumped on every mousemove so subscriber widgets re-render.
    pub move_count: u64,
}

/// Begin a vertical drag (up = increase). Convenience for knobs.
pub fn begin_drag(
    drag: &mut Signal<DragState>,
    handle: ParamHandle,
    start_y: f64,
    sensitivity: f64,
) {
    begin_drag_axis(drag, handle, DragAxis::Vertical, 0.0, start_y, sensitivity);
}

/// Begin a drag along the given axis.
pub fn begin_drag_axis(
    drag: &mut Signal<DragState>,
    handle: ParamHandle,
    axis: DragAxis,
    start_x: f64,
    start_y: f64,
    sensitivity: f64,
) {
    let start_value = handle.normalized();
    handle.begin_edit();
    drag.set(DragState {
        active: true,
        handle: Some(handle),
        axis,
        start_value,
        start_x,
        start_y,
        sensitivity,
        move_count: 0,
    });
}

/// Wrap your editor root in this. Captures `mousemove`/`mouseup` and feeds
/// them to whichever widget started a drag.
#[component]
pub fn DragProvider(children: Element) -> Element {
    let mut drag = use_signal(DragState::default);

    use_context_provider(|| drag);

    rsx! {
        div {
            style: "width:100vw; height:100vh;",

            onmousemove: move |evt: MouseEvent| {
                let state = drag.read().clone();
                if state.active {
                    if let Some(handle) = &state.handle {
                        let pos = evt.client_coordinates();
                        // Modifier-key sensitivity multiplier — increases the
                        // pixels-per-unit so motion feels finer:
                        //  * Shift: 4× (fine)
                        //  * Ctrl/Meta: 16× (ultra-fine)
                        let mods = evt.modifiers();
                        let mult = if mods.ctrl() || mods.meta() {
                            16.0
                        } else if mods.shift() {
                            4.0
                        } else {
                            1.0
                        };
                        let sens = state.sensitivity * mult;
                        let delta = match state.axis {
                            DragAxis::Vertical => (state.start_y - pos.y) / sens,
                            DragAxis::Horizontal => (pos.x - state.start_x) / sens,
                        };
                        let new_val = (state.start_value as f64 + delta).clamp(0.0, 1.0) as f32;
                        handle.set_normalized(new_val);
                        let mut s = state.clone();
                        s.move_count += 1;
                        drag.set(s);
                    }
                }
            },

            onmouseup: move |_| {
                let state = drag.read().clone();
                if state.active {
                    if let Some(handle) = &state.handle {
                        handle.end_edit();
                    }
                    drag.set(DragState::default());
                }
            },

            {children}
        }
    }
}
