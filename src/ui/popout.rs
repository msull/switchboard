//! A session in a window of its own: the same page the main window
//! draws, with the side panel, in a native window (an egui viewport)
//! that the main window's frame renders. The page of a session is in
//! one place only, so while a session has a window the main window's
//! page for it is a note and a button that raises the window.

use std::time::{Duration, Instant};

use egui::{Context, Key, Modifiers, Ui, ViewportBuilder, ViewportCommand, ViewportId};

/// How long after launch the main window is held at its saved frame.
/// macOS moves a new window onto the Dock's display, and clamps it to
/// that screen, some time after the window exists.
const HOLD_AT_LAUNCH: Duration = Duration::from_millis(1500);

use super::{DrawCtx, session, side_panel, theme, zoom};
use crate::core::{AppAction, Popout, RecordId, SessionKind, WindowFrame};

/// Size of a new window before it is moved or resized.
const DEFAULT_SIZE: egui::Vec2 = egui::vec2(1100.0, 780.0);
/// A window's frame is written to settings once it has held still this
/// long, so a drag does not save on every frame.
const SETTLE: Duration = Duration::from_millis(800);

/// The viewport that shows this session.
#[must_use]
pub fn viewport_id(id: RecordId) -> ViewportId {
    ViewportId::from_hash_of(("popout", id))
}

/// Every session window, after the main window's panels.
pub fn show_all(cx: &mut DrawCtx<'_>, ctx: &Context) {
    let popouts: Vec<Popout> = cx.core.settings().popouts.clone();
    // Raise the windows the core asked for, then draw them all.
    for id in std::mem::take(&mut cx.state.focus_windows) {
        if popouts.iter().any(|p| p.session == id) {
            ctx.send_viewport_cmd_to(viewport_id(id), ViewportCommand::Focus);
        }
    }
    cx.state
        .popout_opened
        .retain(|id, _| popouts.iter().any(|p| p.session == *id));
    for popout in popouts {
        window(cx, ctx, &popout);
    }
}

fn window(cx: &mut DrawCtx<'_>, ctx: &Context, popout: &Popout) {
    let id = popout.session;
    let Some(record) = cx.core.session(id).cloned() else {
        return;
    };
    // The window's display sets its zoom, found from where the window
    // was last seen, or where it was saved. Frames are in native points;
    // the builder takes the window's own zoomed points.
    let last_seen = cx
        .state
        .popout_frames
        .get(&id)
        .map(|(f, _)| f.clone())
        .or_else(|| popout.frame.clone())
        .map(|f| rect_of(&f));
    let monitor = zoom::monitor_of(last_seen);
    let percent = cx.core.monitor_zoom(&monitor);
    let factor = zoom::factor(percent);
    let mut builder = ViewportBuilder::default()
        .with_title(format!("{} · Switchboard", record.name))
        .with_inner_size(DEFAULT_SIZE);
    // The opening geometry is fixed when the window is first shown and
    // given unchanged after, so the builder never moves the window.
    let opened = *cx.state.popout_opened.entry(id).or_insert_with(|| {
        popout
            .frame
            .as_ref()
            .filter(|f| zoom::monitor_attached(&f.monitor))
            .map(|f| {
                let r = rect_of(f);
                (r.min / factor, r.size() / factor)
            })
    });
    if let Some((pos, size)) = opened {
        builder = builder.with_position(pos).with_inner_size(size);
    }
    let main_zoom = zoom::install(ctx, percent);
    ctx.show_viewport_immediate(viewport_id(id), builder, |ctx, _class| {
        let (close_requested, frame) = ctx.input(|i| {
            let v = i.viewport();
            (v.close_requested(), v.inner_rect.zip(v.outer_rect))
        });
        // The window's own close button, or Cmd+W in it.
        if close_requested || ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::W)) {
            cx.dispatch(AppAction::ClosePopout(id));
        }
        if let Some((inner, outer)) = frame {
            let now = frame_of(inner * factor, outer * factor, monitor.clone());
            let seen = cx
                .state
                .popout_frames
                .entry(id)
                .or_insert((now.clone(), Instant::now()));
            if settled(seen, now.clone()) && popout.frame.as_ref() != Some(&now) {
                cx.dispatch(AppAction::PopoutMoved(id, now));
            }
        }
        zoom::window_keys(cx, ctx, &monitor, percent);
        shortcuts(cx, ctx, &record.id, record.kind);
        cx.state.in_popout = Some(id);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::palette_of(ctx).bg))
            .show(ctx, |ui| body(cx, ui, &record));
        cx.state.in_popout = None;
        super::switcher::toasts(cx, ctx, Some(id));
    });
    zoom::restore(ctx, main_zoom);
}

fn rect_of(f: &WindowFrame) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(points(f.x), points(f.y)),
        egui::vec2(points(f.w), points(f.h)),
    )
}

/// The session page with the side panel beside it, as the main window
/// lays them out.
fn body(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &crate::core::SessionRecord) {
    if cx.core.settings().files_open {
        let message = matches!(record.kind, SessionKind::Agent(_)).then_some(record.id);
        side_panel(
            cx,
            ui,
            record.project,
            true,
            message,
            Some(record.id),
            Some(record.id),
        );
    }
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(theme::palette(ui).bg)
                .inner_margin(egui::Margin {
                    left: 24,
                    right: 24,
                    top: 20,
                    bottom: 16,
                }),
        )
        .show(ui, |ui| session::show(cx, ui, record.id));
}

/// A frame from a window's inner and outer rects in native points: the
/// position is the outer one, the size the inner one, which is what a
/// window is opened with again.
fn frame_of(inner: egui::Rect, outer: egui::Rect, monitor: String) -> WindowFrame {
    WindowFrame {
        x: whole(outer.min.x),
        y: whole(outer.min.y),
        w: whole(inner.width()),
        h: whole(inner.height()),
        monitor,
    }
}

/// Whether a window has held still at `now` for a moment: `seen` is the
/// frame last seen and since when, so a drag is not saved on every
/// frame.
fn settled(seen: &mut (WindowFrame, Instant), now: WindowFrame) -> bool {
    if seen.0 != now {
        *seen = (now, Instant::now());
        return false;
    }
    seen.1.elapsed() >= SETTLE
}

/// The main window's frame, saved like a pop-out's once it settles, so
/// the next launch opens it there.
pub fn remember_main_window(cx: &mut DrawCtx<'_>, ctx: &Context, monitor: String) {
    let factor = ctx.zoom_factor();
    let rects = ctx.input(|i| {
        let v = i.viewport();
        v.inner_rect.zip(v.outer_rect)
    });
    let Some((inner, outer)) = rects else {
        return;
    };
    let now = frame_of(inner * factor, outer * factor, monitor);
    let first = *cx.state.first_frame.get_or_insert_with(Instant::now);
    if first.elapsed() < HOLD_AT_LAUNCH {
        hold_at_launch(cx, ctx, &now, factor);
        return;
    }
    let seen = cx
        .state
        .main_frame
        .get_or_insert_with(|| (now.clone(), Instant::now()));
    if settled(seen, now.clone()) && cx.core.settings().main_window.as_ref() != Some(&now) {
        cx.dispatch(AppAction::MainWindowMoved(now));
    }
}

/// The saved frame is asked for again while the window is elsewhere,
/// in the window's own zoomed points, which is what viewport commands
/// take.
fn hold_at_launch(cx: &DrawCtx<'_>, ctx: &Context, now: &WindowFrame, factor: f32) {
    let Some(saved) = cx.core.settings().main_window.as_ref() else {
        return;
    };
    if !zoom::monitor_attached(&saved.monitor) || saved == now {
        return;
    }
    let r = rect_of(saved);
    ctx.send_viewport_cmd(ViewportCommand::OuterPosition(r.min / factor));
    ctx.send_viewport_cmd(ViewportCommand::InnerSize(r.size() / factor));
    ctx.request_repaint_after(Duration::from_millis(100));
}

/// Screen points are whole numbers far below what `f32` counts exactly.
#[allow(clippy::cast_precision_loss)]
fn points(v: i32) -> f32 {
    v as f32
}

#[allow(clippy::cast_possible_truncation)]
fn whole(v: f32) -> i32 {
    v.round() as i32
}

/// The session shortcuts that make sense with the window focused:
/// Cmd+. interrupts, Cmd+T shows the raw pane, Cmd+B, Cmd+R, and
/// Cmd+N pick a side tab as in the main window.
fn shortcuts(cx: &mut DrawCtx<'_>, ctx: &Context, id: &RecordId, kind: SessionKind) {
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Period)) {
        cx.dispatch(AppAction::Interrupt(*id));
    }
    if matches!(kind, SessionKind::Agent(_))
        && ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::T))
    {
        cx.state.terminal_open = !cx.state.terminal_open;
    }
    super::side_tab_keys(cx, ctx, true);
}
