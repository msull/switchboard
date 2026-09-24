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
//!
//! Pointer events are converted to points as they arrive, between
//! frames, with whatever factor is installed then. So the frame ends
//! with the factor of the window under the pointer installed, and the
//! main window's is put back, with its raw input rescaled to match,
//! just before its next pass ([`before_main_pass`]).

use egui::{Context, Key, Modifiers, RawInput, Rect};

use super::{DrawCtx, UiState};
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

/// The zoom keys inside a pop-out, for its display. A pop-out with the
/// pointer over it asks for its factor to stay installed after the
/// frame, so the pointer events it gets next are scaled for it.
pub fn window_keys(cx: &mut DrawCtx<'_>, ctx: &Context, monitor: &str, percent: u32) {
    if let Some(new) = keys(ctx, percent) {
        cx.dispatch(AppAction::SetMonitorZoom(monitor.to_owned(), new));
    }
    if ctx.input(|i| i.pointer.has_pointer()) {
        cx.state.zoom_under_pointer = Some(percent);
    }
}

/// The end of the frame: the factor left installed is that of the
/// window under the pointer, and the main window's is remembered.
pub fn end_frame(cx: &mut DrawCtx<'_>, ctx: &Context) {
    let main = ctx.zoom_factor();
    cx.state.main_zoom = Some(main);
    if let Some(percent) = cx.state.zoom_under_pointer.take() {
        ctx.options_mut(|o| o.zoom_factor = factor(percent));
    }
}

/// Just before the main window's pass: its own factor is put back, and
/// the rects `egui_winit` already converted with the other factor are
/// brought to the main window's points.
pub fn before_main_pass(state: &UiState, ctx: &Context, raw: &mut RawInput) {
    let Some(main) = state.main_zoom else {
        return;
    };
    let installed = ctx.zoom_factor();
    if (installed - main).abs() <= f32::EPSILON {
        return;
    }
    let ratio = installed / main;
    if let Some(r) = raw.screen_rect.as_mut() {
        *r = *r * ratio;
    }
    for v in raw.viewports.values_mut() {
        if let Some(r) = v.inner_rect.as_mut() {
            *r = *r * ratio;
        }
        if let Some(r) = v.outer_rect.as_mut() {
            *r = *r * ratio;
        }
        if let Some(s) = v.monitor_size.as_mut() {
            *s *= ratio;
        }
    }
    ctx.options_mut(|o| o.zoom_factor = main);
}
