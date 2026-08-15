//! XY pad — two-parameter 2D control surface (iced_audio `XYPad` parity).
//!
//! Click-and-drag positions the dot in pad-relative space. Y axis is flipped
//! so up = increase, matching iced_audio. Bound to two [`ParamHandle`]s.

use crate::param::ParamHandle;
use crate::theme::*;
use dioxus::prelude::*;

/// Plain-value variant — emits normalized `(x, y)` via callback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct XYValue {
    pub x: f32,
    pub y: f32,
}

#[component]
pub fn XYPad(
    x_handle: ParamHandle,
    y_handle: ParamHandle,
    #[props(default = 120)] size: u32,
    #[props(default)] x_label: Option<String>,
    #[props(default)] y_label: Option<String>,
    #[props(default)] color: Option<String>,
    #[props(default)] disabled: bool,
) -> Element {
    let mut dragging = use_signal(|| false);
    let mut bump = use_signal(|| 0u32);
    let _ = *bump.read();

    let s = size as f64;
    let xn = x_handle.normalized().clamp(0.0, 1.0) as f64;
    let yn = y_handle.normalized().clamp(0.0, 1.0) as f64;
    let px = (xn * s) as u32;
    let py = ((1.0 - yn) * s) as u32;

    let accent = color.as_deref().unwrap_or(ACCENT);
    let opacity = if disabled { "0.5" } else { "1.0" };
    let cursor = if disabled { "not-allowed" } else { "crosshair" };

    let update = {
        let xh = x_handle.clone();
        let yh = y_handle.clone();
        move |evt: MouseEvent| {
            let p = evt.element_coordinates();
            let nx = (p.x / s).clamp(0.0, 1.0) as f32;
            let ny = (1.0 - p.y / s).clamp(0.0, 1.0) as f32;
            xh.set_normalized(nx);
            yh.set_normalized(ny);
        }
    };

    rsx! {
        div {
            style: format!(
                "display:inline-flex; flex-direction:column; align-items:center; gap:4px; \
                 opacity:{opacity};"
            ),

            div {
                style: format!(
                    "position:relative; border-radius:4px; border:1px solid {BORDER}; \
                     background:rgba(34,34,64,0.3); overflow:hidden; cursor:{cursor}; \
                     width:{size}px; height:{size}px; user-select:none;"
                ),
                onmousedown: {
                    let xh = x_handle.clone();
                    let yh = y_handle.clone();
                    let update = update.clone();
                    move |evt: MouseEvent| {
                        if disabled { return; }
                        xh.begin_edit();
                        yh.begin_edit();
                        dragging.set(true);
                        update(evt);
                        bump += 1;
                    }
                },
                onmousemove: {
                    let update = update.clone();
                    move |evt: MouseEvent| {
                        if *dragging.read() {
                            update(evt);
                            bump += 1;
                        }
                    }
                },
                onmouseup: {
                    let xh = x_handle.clone();
                    let yh = y_handle.clone();
                    move |_| {
                        if *dragging.read() {
                            xh.end_edit();
                            yh.end_edit();
                            dragging.set(false);
                        }
                    }
                },
                onmouseleave: {
                    let xh = x_handle.clone();
                    let yh = y_handle.clone();
                    move |_| {
                        if *dragging.read() {
                            xh.end_edit();
                            yh.end_edit();
                            dragging.set(false);
                        }
                    }
                },

                div {
                    style: format!(
                        "position:absolute; left:{px}px; top:0; width:1px; height:100%; \
                         background:rgba(136,136,136,0.3); pointer-events:none;"
                    ),
                }
                div {
                    style: format!(
                        "position:absolute; left:0; top:{py}px; width:100%; height:1px; \
                         background:rgba(136,136,136,0.3); pointer-events:none;"
                    ),
                }
                div {
                    style: format!(
                        "position:absolute; left:{px}px; top:{py}px; \
                         width:12px; height:12px; border-radius:6px; \
                         background:{accent}; border:2px solid {SURFACE}; \
                         transform:translate(-50%,-50%); pointer-events:none;"
                    ),
                }
            }

            div {
                style: format!(
                    "display:flex; justify-content:space-between; width:{size}px; \
                     font-size:10px; color:{TEXT_DIM};"
                ),
                if let Some(x_label) = &x_label { span { "X: {x_label}" } }
                if let Some(y_label) = &y_label { span { "Y: {y_label}" } }
            }
        }
    }
}
