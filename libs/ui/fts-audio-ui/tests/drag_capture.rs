//! A drag has to survive the pointer leaving the control.
//!
//! Blitz has no pointer capture: it hit-tests every move and dispatches to
//! whatever is under the cursor. [`DragProvider`] is the answer — it wraps
//! the editor root, so a move anywhere in the window bubbles to it and
//! reaches whichever widget started the drag. These tests are that claim,
//! stated: press a knob, move the pointer somewhere else entirely, and the
//! parameter must still follow.
//!
//! ```sh
//! cargo test -p fts-audio-ui --test drag_capture
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use dioxus::prelude::*;
use dioxus_test::{by_testid, render};

use fts_audio_ui::controls::knob::{Knob, KnobSize};
use fts_audio_ui::drag::DragProvider;
use fts_audio_ui::param::ParamHandle;

/// The parameter under test, as bits so it can cross into the component.
static VALUE: AtomicU32 = AtomicU32::new(0.5_f32.to_bits());

fn value() -> f32 {
    f32::from_bits(VALUE.load(Ordering::Relaxed))
}

fn handle() -> ParamHandle {
    ParamHandle::new(
        || value(),
        || {},
        |v| VALUE.store(v.to_bits(), Ordering::Relaxed),
        || {},
        || format!("{:.3}", value()),
        || "Drive".to_string(),
        |_| None,
    )
}

#[component]
fn Sheet() -> Element {
    rsx! {
        style { "html, body {{ margin:0; padding:0; }}" }
        DragProvider {
            div {
                // The knob sits in the middle, so there is room to drag well
                // clear of it in any direction without leaving the window.
                style: "width:900px; height:700px; display:flex; \
                        align-items:center; justify-content:center;",
                Knob { handle: handle(), size: KnobSize::Large }
            }
        }
    }
}

struct Fx {
    tester: dioxus_test::DocumentTester,
}

impl Fx {
    async fn settle(&self) {
        let _ = self.tester.pump().await;
        self.tester.relayout();
    }

    /// The centre of the knob's gesture surface.
    fn dial(&self) -> (f64, f64) {
        let el = self
            .tester
            .query(by_testid("knob-Drive-dial"))
            .immediately()
            .expect("the knob did not draw its gesture surface");
        let (x, y) = el.document_origin();
        let (w, h) = el.size();
        (x + f64::from(w) / 2.0, y + f64::from(h) / 2.0)
    }
}

async fn mount() -> Fx {
    VALUE.store(0.5_f32.to_bits(), Ordering::Relaxed);
    let tester = render(Sheet).with_window_size(900, 700).build();
    let fx = Fx { tester };
    fx.settle().await;
    fx
}

/// The baseline: a drag that stays on the dial moves the parameter.
#[tokio::test]
async fn a_drag_on_the_dial_moves_the_parameter() {
    let fx = mount().await;
    let (x, y) = fx.dial();

    fx.tester.pointer_down(x, y);
    fx.settle().await;
    fx.tester.pointer_move(x, y - 20.0, true);
    fx.settle().await;
    fx.tester.pointer_up(x, y - 20.0);
    fx.settle().await;

    assert!(value() > 0.5, "a 20 px drag up left the value at {}", value());
}

/// THE regression: once a drag is in flight the pointer belongs to the knob,
/// wherever in the window it goes. Blitz hit-tests every move and dispatches
/// to whatever is under the cursor, so a move that lands on some other
/// element only reaches the drag layer because `DragProvider` is above it in
/// the tree. Anything that breaks that path makes a knob usable only while
/// the cursor is still inside its own 72 px box, which is not how a knob
/// works — you set a knob by dragging *away* from it.
#[tokio::test]
async fn a_drag_survives_the_pointer_leaving_the_control() {
    let fx = mount().await;
    let (x, y) = fx.dial();

    fx.tester.pointer_down(x, y);
    fx.settle().await;

    // Up and across, well clear of the knob but still inside the window.
    for step in 1..=8 {
        let t = f64::from(step) / 8.0;
        fx.tester.pointer_move(x + 220.0 * t, y - 120.0 * t, true);
        fx.settle().await;
    }

    // 120 px up at the knob's 150 px sweep is +0.8.
    let held = value();
    assert!(
        held > 0.95,
        "a 120 px drag up that left the knob only reached {held}",
    );

    fx.tester.pointer_up(x + 220.0, y - 120.0);
    fx.settle().await;
}

/// And it survives the pointer leaving the *window*.
///
/// A hand setting a knob runs out of screen long before it runs out of
/// range. On macOS the view that took the mouse-down keeps receiving drag
/// events wherever the cursor goes, so the events exist — the question is
/// whether anything downstream throws them away. Headless, the harness will
/// not hit-test a point outside the viewport at all, which is the same
/// failure by a different route: `#[ignore]`d as a statement of intent until
/// the host path can deliver it.
#[tokio::test]
#[ignore = "needs pointer capture: blitz hit-tests every move and drops the ones outside the viewport"]
async fn a_drag_survives_the_pointer_leaving_the_window() {
    let fx = mount().await;
    let (x, y) = fx.dial();

    fx.tester.pointer_down(x, y);
    fx.settle().await;
    for step in 1..=8 {
        fx.tester.pointer_move(x, y - f64::from(step) * 60.0, true);
        fx.settle().await;
    }

    assert!(
        value() > 0.99,
        "a drag that left the top of the window stopped at {}",
        value(),
    );
    fx.tester.pointer_up(x, y - 480.0);
    fx.settle().await;
}
