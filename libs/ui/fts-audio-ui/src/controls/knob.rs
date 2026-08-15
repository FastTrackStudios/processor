//! Rotary knob — SVG arc with vertical-drag interaction and modulation overlay.
//!
//! Provider-agnostic port of `audio-gui::controls::knob::Knob`. Binds to a
//! [`ParamHandle`] instead of a `nih_plug::ParamPtr`. Requires a
//! [`crate::drag::DragProvider`] ancestor for drag capture.

use crate::color::css_color_to_hex;
use crate::drag::{begin_drag, DragState};
use crate::param::ParamHandle;
use crate::theme::*;
use dioxus::prelude::*;
use dioxus_elements::input_data::MouseButton;
use std::f64::consts::PI;

/// Display size.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum KnobSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl KnobSize {
    pub fn diameter(self) -> u32 {
        match self {
            Self::Small => 44,
            Self::Medium => 56,
            Self::Large => 72,
        }
    }
}

const START_ANGLE: f64 = 135.0;
const SWEEP: f64 = 270.0;
/// Pixels of vertical drag per full 0→1 sweep.
const SENSITIVITY: f64 = 150.0;
/// Per-notch step the mouse wheel takes (normalized value units). Hold
/// Shift on the wheel for finer steps.
const WHEEL_STEP: f64 = 0.02;
const WHEEL_STEP_FINE: f64 = 0.005;

fn angle_for_value(v: f64) -> f64 {
    START_ANGLE + v.clamp(0.0, 1.0) * SWEEP
}

fn arc_point(cx: f64, cy: f64, r: f64, angle_deg: f64) -> (f64, f64) {
    let rad = angle_deg * PI / 180.0;
    (cx + r * rad.cos(), cy + r * rad.sin())
}

fn svg_arc(cx: f64, cy: f64, r: f64, start_deg: f64, end_deg: f64) -> String {
    let (x1, y1) = arc_point(cx, cy, r, start_deg);
    let (x2, y2) = arc_point(cx, cy, r, end_deg);
    let large = if (end_deg - start_deg).abs() > 180.0 {
        1
    } else {
        0
    };
    format!("M {x1:.1} {y1:.1} A {r:.1} {r:.1} 0 {large} 1 {x2:.1} {y2:.1}")
}

/// A rotary knob bound to a [`ParamHandle`].
///
/// ## Interaction
///
/// - **Vertical drag** (mouse-down + drag up/down): coarse adjustment.
/// - **Shift + drag**: fine (4× sensitivity).
/// - **Ctrl/Cmd + drag**: ultra-fine (16× sensitivity).
/// - **Mouse wheel**: stepped adjustment. Shift = finer steps.
/// - **Double-click**: type a value into a text input.
/// - **Alt + click**: reset to default value (via [`ParamHandle::with_default`]).
///
/// ## Visualization
///
/// - Default: value arc sweeps from the start of the track toward the cursor.
/// - Bipolar (mark via [`ParamHandle::with_bipolar`]): value arc sweeps from
///   the centre detent (12 o'clock) outward in either direction. Better for
///   gain / pan / cents-style params.
#[component]
pub fn Knob(
    /// The parameter this knob drives.
    handle: ParamHandle,
    #[props(default)] size: KnobSize,
    /// Override the parameter's name when rendering the label.
    #[props(default)]
    label: Option<String>,
    /// Accent color override (e.g. `"#F97316"`).
    #[props(default)]
    color: Option<String>,
    /// Modulation range minimum (0.0–1.0). Drawn as an overlay arc.
    #[props(default)]
    mod_min: Option<f64>,
    #[props(default)] mod_max: Option<f64>,
    #[props(default)] disabled: bool,
) -> Element {
    let mut drag: Signal<DragState> = use_context();
    let mut editing = use_signal(|| false);
    let mut hovered = use_signal(|| false);

    // Re-render while a drag is active so the value display tracks the cursor.
    let _ = drag.read().move_count;

    let normalized = handle.normalized() as f64;
    let display_value = handle.display_value();
    let param_name = label.unwrap_or_else(|| handle.name());
    let is_editing = *editing.read();
    let is_hovered = *hovered.read();
    let bipolar = handle.is_bipolar();

    let d = size.diameter();
    let df = d as f64;
    let cx = df / 2.0;
    let cy = df / 2.0;
    let r = df / 2.0 - 4.0;
    let val = normalized.clamp(0.0, 1.0);

    let track_path = svg_arc(cx, cy, r, START_ANGLE, START_ANGLE + SWEEP);
    let end_angle = angle_for_value(val);
    // For bipolar params draw the value arc from the centre detent outward;
    // for unipolar from the start of the track.
    let value_path = if bipolar {
        let centre_angle = START_ANGLE + SWEEP / 2.0;
        if (val - 0.5).abs() < 0.001 {
            String::new()
        } else if val > 0.5 {
            svg_arc(cx, cy, r, centre_angle, end_angle)
        } else {
            svg_arc(cx, cy, r, end_angle, centre_angle)
        }
    } else if val > 0.001 {
        svg_arc(cx, cy, r, START_ANGLE, end_angle)
    } else {
        String::new()
    };

    let mod_path = match (mod_min, mod_max) {
        (Some(lo), Some(hi)) => {
            let lo_angle = angle_for_value(lo.clamp(0.0, 1.0));
            let hi_angle = angle_for_value(hi.clamp(0.0, 1.0));
            svg_arc(cx, cy, r - 2.0, lo_angle, hi_angle)
        }
        _ => String::new(),
    };

    let (tx, ty) = arc_point(cx, cy, r - 6.0, end_angle);
    let (tx2, ty2) = arc_point(cx, cy, r + 1.0, end_angle);

    // Resolve the active architect-ui theme tokens into concrete `#rrggbb`
    // strings — blitz doesn't substitute `var(--…)` inside SVG
    // presentation attributes, and `style="stroke:…"` is dropped on
    // path elements, so we have to do the lookup in Rust. Falls back
    // to a sensible dark-mode hex when the theme isn't in scope (e.g.
    // unit tests that don't mount a ThemeProvider).
    let theme = try_consume_context::<architect_ui::prelude::ThemeContext>();
    let resolve = |key: &str, fallback: &str| -> String {
        theme
            .as_ref()
            .and_then(|ctx| {
                let state = ctx.state.read();
                let style = state.styles.active(state.mode);
                style.get(key).map(str::to_string)
            })
            .and_then(|raw| css_color_to_hex(&raw))
            .unwrap_or_else(|| fallback.to_string())
    };
    let accent = color
        .as_deref()
        .map(str::to_string)
        .unwrap_or_else(|| resolve("primary", "#c8a86e"));
    let track_color = resolve("border", "#2a2a30");
    let pointer_color = resolve("foreground", "#d4d4d8");
    let detent_color = resolve("muted-foreground", "#737380");
    let mod_color = resolve("accent", "#8b5cf6");
    let bg_fill = resolve("card", "#0c0c0f");
    let bg_stroke = resolve("border", "#2a2a30");
    let opacity = if disabled { "0.5" } else { "1.0" };
    let cursor = if disabled { "not-allowed" } else { "pointer" };
    // Hover/drag visuals: gentle scale + a glow on the value arc.
    let active = !disabled && (is_hovered || drag.read().active);
    let scale = if active { "1.04" } else { "1.0" };
    let glow_color = color.clone().unwrap_or_else(|| "#c8a86e".to_string());
    let value_extra_css = if active {
        format!(" filter: drop-shadow(0 0 4px {glow_color});")
    } else {
        String::new()
    };
    // Centre-detent tick (only drawn for bipolar so users see the 0 mark).
    let detent_centre_angle = START_ANGLE + SWEEP / 2.0;
    let (det_x1, det_y1) = arc_point(cx, cy, r - 6.0, detent_centre_angle);
    let (det_x2, det_y2) = arc_point(cx, cy, r + 1.0, detent_centre_angle);
    // Inner cap radius — gives the knob a tactile "puck" inside the arc.
    let cap_r = (r - 7.0).max(2.0);

    rsx! {
        div {
            style: format!(
                "display:inline-flex; flex-direction:column; align-items:center; gap:6px; \
                 opacity:{opacity}; cursor:{cursor}; position:relative; \
                 transform:scale({scale}); transition:transform 90ms ease-out; \
                 padding:2px 0; \
                 color:var(--foreground);"
            ),
            title: format!("{param_name} — {display_value}\nDrag · Shift=fine · Ctrl=ultra-fine · Wheel · Dbl-click=type · Alt-click=reset"),
            onmouseenter: move |_| { hovered.set(true); },
            onmouseleave: move |_| { hovered.set(false); },

            svg {
                width: "{d}",
                height: "{d}",
                view_box: "0 0 {df} {df}",

                // Background disc — anchors the knob visually and
                // gives the value arc something to sit on.
                circle {
                    cx: "{cx:.1}",
                    cy: "{cy:.1}",
                    r: "{cap_r:.1}",
                    fill: "{bg_fill}",
                    stroke: "{bg_stroke}",
                    stroke_width: "0.75",
                    opacity: "0.95",
                }

                // Track (full-arc background).
                path {
                    d: "{track_path}",
                    fill: "none",
                    stroke: "{track_color}",
                    stroke_width: "4",
                    stroke_linecap: "round",
                }

                // Bipolar 0 detent tick (sits behind the value arc).
                if bipolar {
                    line {
                        x1: "{det_x1:.1}",
                        y1: "{det_y1:.1}",
                        x2: "{det_x2:.1}",
                        y2: "{det_y2:.1}",
                        stroke: "{detent_color}",
                        stroke_width: "1.5",
                        opacity: "0.7",
                    }
                }

                // Value arc — the only colourful element by default.
                if !value_path.is_empty() {
                    path {
                        d: "{value_path}",
                        fill: "none",
                        stroke: "{accent}",
                        stroke_width: "4.5",
                        stroke_linecap: "round",
                        style: "{value_extra_css}",
                    }
                }

                if !mod_path.is_empty() {
                    path {
                        d: "{mod_path}",
                        fill: "none",
                        stroke: "{mod_color}",
                        stroke_width: "2.25",
                        stroke_linecap: "round",
                        opacity: "0.65",
                    }
                }

                // Pointer line from cap edge through the arc.
                line {
                    x1: "{tx:.1}",
                    y1: "{ty:.1}",
                    x2: "{tx2:.1}",
                    y2: "{ty2:.1}",
                    stroke: "{pointer_color}",
                    stroke_width: "2.25",
                    stroke_linecap: "round",
                }
            }

            if !disabled {
                div {
                    style: "position:absolute; inset:0; cursor:ns-resize; user-select:none;",
                    onmousedown: {
                        let handle = handle.clone();
                        move |evt: MouseEvent| {
                            // Alt-click → reset to default value (DAW convention).
                            if evt.modifiers().alt() {
                                evt.prevent_default();
                                handle.reset_to_default();
                                return;
                            }
                            // Right-click → text-edit prompt for typing a value.
                            if evt.trigger_button() == Some(MouseButton::Secondary) {
                                evt.prevent_default();
                                editing.set(true);
                                return;
                            }
                            begin_drag(
                                &mut drag,
                                handle.clone(),
                                evt.client_coordinates().y,
                                SENSITIVITY,
                            );
                        }
                    },
                    ondoubleclick: move |_| { editing.set(true); },
                    onwheel: {
                        let handle = handle.clone();
                        move |evt: WheelEvent| {
                            evt.prevent_default();
                            let delta_y = evt.delta().strip_units().y;
                            // Wheel-up (negative delta_y on most platforms) increases.
                            let direction = if delta_y < 0.0 { 1.0 } else { -1.0 };
                            let mods = evt.modifiers();
                            let step = if mods.ctrl() || mods.meta() {
                                WHEEL_STEP_FINE * 0.25
                            } else if mods.shift() {
                                WHEEL_STEP_FINE
                            } else {
                                WHEEL_STEP
                            };
                            let cur = handle.normalized() as f64;
                            let next = (cur + direction * step).clamp(0.0, 1.0) as f32;
                            handle.begin_edit();
                            handle.set_normalized(next);
                            handle.end_edit();
                        }
                    },
                }
            }

            if is_editing {
                input {
                    r#type: "text",
                    style: format!(
                        "font-size:11px; color:var(--foreground); \
                         background:var(--card, var(--background)); \
                         border:1px solid var(--ring, var(--primary)); \
                         border-radius:4px; min-width:52px; width:60px; \
                         text-align:center; padding:2px 4px; outline:none; \
                         font-variant-numeric:tabular-nums; \
                         font-family:var(--font-sans, ui-monospace);"
                    ),
                    value: "{display_value}",
                    onkeydown: move |evt: KeyboardEvent| {
                        if evt.key() == Key::Enter || evt.key() == Key::Escape {
                            editing.set(false);
                        }
                    },
                    onchange: {
                        let handle = handle.clone();
                        move |evt: FormEvent| {
                            let text = evt.value();
                            if let Some(n) = handle.string_to_normalized(&text) {
                                handle.begin_edit();
                                handle.set_normalized(n);
                                handle.end_edit();
                            }
                            editing.set(false);
                        }
                    },
                    onfocusout: move |_| { editing.set(false); },
                }
            } else {
                span {
                    style: format!(
                        "font-size:11px; color:var(--foreground); \
                         font-variant-numeric:tabular-nums; \
                         font-weight:600; \
                         min-width:52px; text-align:center; cursor:text; \
                         letter-spacing:-0.01em;"
                    ),
                    ondoubleclick: move |_| {
                        if !disabled { editing.set(true); }
                    },
                    "{display_value}"
                }
            }

            span {
                style: format!(
                    "font-size:10px; color:var(--muted-foreground); \
                     font-weight:500; letter-spacing:0.05em; \
                     text-transform:uppercase; \
                     min-width:52px; text-align:center;"
                ),
                "{param_name}"
            }
        }
    }
}

/// Knob displaying a raw normalized value, not bound to a parameter system.
/// Useful for visualizations or custom edit handling.
#[component]
pub fn RawKnob(
    #[props(default = 0.5)] value: f64,
    #[props(default)] size: KnobSize,
    #[props(default)] label: Option<String>,
    #[props(default)] display_value: Option<String>,
    #[props(default)] color: Option<String>,
    #[props(default)] mod_min: Option<f64>,
    #[props(default)] mod_max: Option<f64>,
    #[props(default)] on_change: Option<Callback<f64>>,
    #[props(default)] disabled: bool,
) -> Element {
    let d = size.diameter();
    let df = d as f64;
    let cx = df / 2.0;
    let cy = df / 2.0;
    let r = df / 2.0 - 4.0;
    let val = value.clamp(0.0, 1.0);

    let track_path = svg_arc(cx, cy, r, START_ANGLE, START_ANGLE + SWEEP);
    let end_angle = angle_for_value(val);
    let value_path = if val > 0.001 {
        svg_arc(cx, cy, r, START_ANGLE, end_angle)
    } else {
        String::new()
    };
    let mod_path = match (mod_min, mod_max) {
        (Some(lo), Some(hi)) => {
            let lo_angle = angle_for_value(lo.clamp(0.0, 1.0));
            let hi_angle = angle_for_value(hi.clamp(0.0, 1.0));
            svg_arc(cx, cy, r - 2.0, lo_angle, hi_angle)
        }
        _ => String::new(),
    };
    let (tx, ty) = arc_point(cx, cy, r - 6.0, end_angle);
    let (tx2, ty2) = arc_point(cx, cy, r + 1.0, end_angle);

    let accent = color.as_deref().unwrap_or(ACCENT);
    let opacity = if disabled { "0.5" } else { "1.0" };
    let cursor = if disabled { "not-allowed" } else { "pointer" };

    rsx! {
        div {
            style: format!(
                "display:inline-flex; flex-direction:column; align-items:center; gap:4px; \
                 opacity:{opacity}; cursor:{cursor};"
            ),

            svg {
                width: "{d}", height: "{d}", view_box: "0 0 {df} {df}",
                path { d: "{track_path}", fill: "none", stroke: "{BORDER}", stroke_width: "3.5", stroke_linecap: "round" }
                if !value_path.is_empty() {
                    path { d: "{value_path}", fill: "none", stroke: "{accent}", stroke_width: "4", stroke_linecap: "round" }
                }
                if !mod_path.is_empty() {
                    path { d: "{mod_path}", fill: "none", stroke: "{SIGNAL_MOD}", stroke_width: "2", stroke_linecap: "round", opacity: "0.6" }
                }
                line { x1: "{tx:.1}", y1: "{ty:.1}", x2: "{tx2:.1}", y2: "{ty2:.1}", stroke: "{TEXT}", stroke_width: "2", stroke_linecap: "round" }
            }

            if !disabled {
                input {
                    r#type: "range",
                    style: "position:absolute; inset:0; opacity:0; cursor:pointer;",
                    min: "0", max: "1", step: "0.005",
                    value: "{val}",
                    oninput: move |evt: FormEvent| {
                        if let Ok(v) = evt.value().parse::<f64>() {
                            if let Some(cb) = &on_change { cb.call(v.clamp(0.0, 1.0)); }
                        }
                    },
                }
            }

            if let Some(display) = &display_value {
                span {
                    style: format!("font-size:10px; color:{TEXT_DIM}; font-variant-numeric:tabular-nums;"),
                    "{display}"
                }
            }
            if let Some(label) = &label {
                span {
                    style: format!("font-size:10px; color:{TEXT_DIM}; font-weight:500;"),
                    "{label}"
                }
            }
        }
    }
}
