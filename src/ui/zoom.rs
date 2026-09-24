//! Zoom per display. egui keeps one zoom factor for the whole context
//! and takes a new one only at the main window's pass, so a pop-out
//! would inherit the dashboard's zoom. Instead the main window's zoom
//! is set each frame from the display it is on, and a pop-out's pass
//! runs with the factor swapped to its own display's and back: egui
//! reads the factor at the start of every viewport's pass, so the
//! swap is all a pop-out needs. egui's own zoom keys are turned off
//! and handled here per window, so Cmd+= and Cmd+- in a window change
//! the zoom of the display that window is on.
//!
//! Rects egui reports are in a window's own zoomed points; screen
//! frames are in native points. A rect is scaled by the window's factor
//! before it is compared with a screen or saved.

use egui::{Context, Key, Modifiers, Rect};

use super::DrawCtx;
use crate::core::AppAction;

/// The name used where the system lists no displays.
const NO_DISPLAY: &str = "main";
const STEP: u32 = 10;
const MIN: u32 = 50;
const MAX: u32 = 300;

/// The display a window is on: the one whose frame holds the window's
/// centre (`outer`, in native points), else the first, else
/// [`NO_DISPLAY`]. A window across two displays follows its centre.
#[must_use]
pub fn monitor_of(outer: Option<Rect>) -> String {
    let screens = promptbox::adapters::screens::screens();
    outer
        .and_then(|r| screens.iter().find(|s| s.frame.contains(r.center())))
        .or_else(|| screens.first())
        .map_or_else(|| NO_DISPLAY.to_owned(), |s| s.name.clone())
}

/// egui's zoom factor for a percent.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn factor(percent: u32) -> f32 {
    percent as f32 / 100.0
}

/// Cmd+= (or Cmd++) and Cmd+- in this window's input: the new percent.
fn keys(ctx: &Context, current: u32) -> Option<u32> {
    let zoom_in = ctx.input_mut(|i| {
        i.consume_key(Modifiers::COMMAND, Key::Equals)
            || i.consume_key(Modifiers::COMMAND, Key::Plus)
    });
    let zoom_out = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Minus));
    match (zoom_in, zoom_out) {
        (true, _) => Some((current + STEP).min(MAX)),
        (_, true) => Some(current.saturating_sub(STEP).max(MIN)),
        _ => None,
    }
}

/// The main window: egui's zoom keys are replaced by these, and the
/// window's zoom follows its display. Called once per frame, first.
pub fn main_window(cx: &mut DrawCtx<'_>, ctx: &Context) {
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
    // Read outside `input`: the context is one lock, taken once.
    let current = ctx.zoom_factor();
    let outer = ctx.input(|i| i.viewport().outer_rect.map(|r| r * current));
    let monitor = monitor_of(outer);
    let mut percent = cx.core.monitor_zoom(&monitor);
    if let Some(new) = keys(ctx, percent) {
        cx.dispatch(AppAction::SetMonitorZoom(monitor, new));
        percent = new;
    }
    if (ctx.zoom_factor() - factor(percent)).abs() > f32::EPSILON {
        ctx.set_zoom_factor(factor(percent));
    }
}

/// Puts a pop-out's zoom in place for its pass; the value returned is
/// given back to [`restore`] once the pass is over.
#[must_use]
pub fn install(ctx: &Context, percent: u32) -> f32 {
    let main = ctx.options(|o| o.zoom_factor);
    ctx.options_mut(|o| o.zoom_factor = factor(percent));
    main
}

/// The main window's zoom again, after a pop-out's pass.
pub fn restore(ctx: &Context, main: f32) {
    ctx.options_mut(|o| o.zoom_factor = main);
}

/// The zoom keys inside a pop-out, for its display.
pub fn window_keys(cx: &mut DrawCtx<'_>, ctx: &Context, monitor: &str, percent: u32) {
    if let Some(new) = keys(ctx, percent) {
        cx.dispatch(AppAction::SetMonitorZoom(monitor.to_owned(), new));
    }
}
