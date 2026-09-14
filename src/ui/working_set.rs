//! The Working Set: the user's own grid of cards from any project,
//! sessions and files alike, each where they put it. Cards sit on a
//! grid of fixed units; the number of columns comes from the window,
//! and a card past the right edge is reached by scrolling.

use std::path::Path;

use egui::{Pos2, RichText, Sense, Ui, UiBuilder, vec2};

use super::cards::{actions, file_name, is_running, kicker_text, kind_label, session_card};
use super::document::{self, Body};
use super::{DrawCtx, UiState, theme};
use crate::core::grid::{MIN_HEIGHT, MIN_WIDTH};
use crate::core::{
    AppAction, AppCore, CardState, GridRect, PinTarget, PinnedItem, SessionKind, SessionRecord,
    SetId,
};

/// Arrange mode: while on, cards are moved and resized instead of
/// used, and the one being dragged follows the pointer in whole units.
#[derive(Debug, Default)]
pub struct Arrange {
    pub on: bool,
    pub drag: Option<Drag>,
}

/// One drag in progress: what is dragged, where it started, and where
/// it would land if released now.
#[derive(Debug, Clone)]
pub struct Drag {
    pub target: PinTarget,
    pub from: GridRect,
    /// The corner handle resizes; anywhere else moves.
    pub resize: bool,
    pub start: Pos2,
    pub preview: GridRect,
}

/// The corner handle's side, in points.
const HANDLE: f32 = 16.0;

/// Where a drag that started at `from` lands after the pointer moved
/// by `delta` points: the same rectangle shifted (or grown) by the
/// nearest whole number of units, never past the left or top edge and
/// never below the minimum size.
#[must_use]
pub fn dragged_rect(from: GridRect, resize: bool, delta: egui::Vec2) -> GridRect {
    #[allow(clippy::cast_possible_truncation)]
    let (dx, dy) = (
        (delta.x / UNIT).round() as i64,
        (delta.y / UNIT).round() as i64,
    );
    let shift = |base: u32, by: i64, min: u32| -> u32 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let v = (i64::from(base) + by).max(i64::from(min)) as u32;
        v
    };
    if resize {
        GridRect {
            w: shift(from.w, dx, MIN_WIDTH),
            h: shift(from.h, dy, MIN_HEIGHT),
            ..from
        }
    } else {
        GridRect {
            x: shift(from.x, dx, 0),
            y: shift(from.y, dy, 0),
            ..from
        }
    }
}

/// One grid unit in points, gap included: seven units make a board
/// card's 230 px.
pub const UNIT: f32 = 34.0;
/// The gap between neighbouring cards, taken off each card's right and
/// bottom edge.
const GAP_PX: f32 = 8.0;

/// How many whole units fit in `width` points.
#[must_use]
pub fn columns(width: f32) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let n = ((width + GAP_PX) / UNIT).floor() as u32;
    n.max(crate::core::grid::MIN_WIDTH)
}

/// The points rectangle of a grid rectangle placed at `origin`.
#[must_use]
pub fn cell_rect(origin: egui::Pos2, rect: GridRect) -> egui::Rect {
    #[allow(clippy::cast_precision_loss)]
    let (x, y, w, h) = (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32);
    egui::Rect::from_min_size(
        origin + vec2(x * UNIT, y * UNIT),
        vec2(w * UNIT - GAP_PX, h * UNIT - GAP_PX),
    )
}

pub fn show(cx: &mut DrawCtx<'_>, ui: &mut Ui, set: SetId) {
    let p = theme::palette(ui);
    ui.spacing_mut().item_spacing = egui::vec2(10.0, 6.0);
    let Some((name, mut items)) = cx
        .core
        .working_set(set)
        .map(|s| (s.name.clone(), s.items.clone()))
    else {
        ui.label("This working set no longer exists.");
        return;
    };
    header(cx, ui, set, &name, items.is_empty());
    ui.label(theme::meta_text(
        ui,
        match items.len() {
            0 => "Nothing here yet.".to_owned(),
            1 => "1 card".to_owned(),
            n => format!("{n} cards"),
        },
    ));
    if items.is_empty() {
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "Add a session from its card's menu or its header's Working sets button, and a file from the file tree's menu.",
            )
            .color(p.n700),
        );
        return;
    }
    ui.add_space(8.0);
    let arranging = cx.state.arrange.on;
    // The card being dragged is drawn where it would land, so it snaps
    // along under the pointer; the record itself moves on release.
    if let Some(drag) = &cx.state.arrange.drag
        && let Some(item) = items.iter_mut().find(|i| i.target == drag.target)
    {
        item.rect = drag.preview;
    }
    let columns = columns(ui.available_width());
    let width_units = items
        .iter()
        .map(|i| i.rect.x + i.rect.w)
        .fold(columns, u32::max);
    // Room below the last card while arranging, to drag one down into.
    let extra = if arranging { 8 } else { 0 };
    let height_units = items.iter().map(|i| i.rect.y + i.rect.h).max().unwrap_or(0) + extra;
    egui::ScrollArea::both()
        .id_salt("working-set")
        .auto_shrink(false)
        .show(ui, |ui| {
            #[allow(clippy::cast_precision_loss)]
            let total = vec2(
                width_units as f32 * UNIT - GAP_PX,
                height_units as f32 * UNIT - GAP_PX,
            );
            let (rect, _) = ui.allocate_exact_size(total, Sense::hover());
            let origin = rect.min;
            if arranging {
                let cells: Vec<egui::Rect> =
                    items.iter().map(|i| cell_rect(origin, i.rect)).collect();
                grid_dots(ui, rect, width_units, height_units, &cells);
            }
            for item in &items {
                let cell = cell_rect(origin, item.rect);
                ui.scope_builder(UiBuilder::new().max_rect(cell), |ui| {
                    ui.set_clip_rect(cell.intersect(ui.clip_rect()));
                    if arranging {
                        ui.disable();
                    }
                    card(cx, ui, set, item);
                });
                if arranging {
                    arrange_handles(cx, ui, set, item, cell);
                }
            }
        });
}

/// A dot at every unit corner not under a card, so the grid a card
/// snaps to is visible (the dimmed cards would let dots show through).
fn grid_dots(ui: &Ui, rect: egui::Rect, width_units: u32, height_units: u32, cells: &[egui::Rect]) {
    let p = theme::palette(ui);
    let painter = ui.painter();
    for y in 0..=height_units {
        for x in 0..=width_units {
            #[allow(clippy::cast_precision_loss)]
            let at = rect.min + vec2(x as f32 * UNIT, y as f32 * UNIT)
                - vec2(GAP_PX / 2.0, GAP_PX / 2.0);
            if cells.iter().any(|c| c.contains(at)) {
                continue;
            }
            painter.circle_filled(at, 1.0, p.n300);
        }
    }
}

/// The move and resize senses over one card while arranging: the whole
/// cell moves it, the bottom-right corner resizes it. The outline shows
/// the drop: cyan where it fits, magenta where it would land on
/// another card and be refused.
fn arrange_handles(
    cx: &mut DrawCtx<'_>,
    ui: &mut Ui,
    set: SetId,
    item: &PinnedItem,
    cell: egui::Rect,
) {
    let p = theme::palette(ui);
    let id = ui.id().with(("arrange", &item.target));
    let handle = egui::Rect::from_min_size(cell.max - vec2(HANDLE, HANDLE), vec2(HANDLE, HANDLE));
    let body = ui
        .interact(cell, id.with("move"), Sense::drag())
        .on_hover_cursor(egui::CursorIcon::Grab);
    let corner = ui
        .interact(handle, id.with("resize"), Sense::drag())
        .on_hover_cursor(egui::CursorIcon::ResizeNwSe);
    let dragging = cx
        .state
        .arrange
        .drag
        .as_ref()
        .is_some_and(|d| d.target == item.target);
    for (response, resize) in [(&corner, true), (&body, false)] {
        if response.drag_started()
            && let Some(start) = response.interact_pointer_pos()
        {
            cx.state.arrange.drag = Some(Drag {
                target: item.target.clone(),
                from: item.rect,
                resize,
                start,
                preview: item.rect,
            });
        }
        if response.dragged()
            && let Some(at) = response.interact_pointer_pos()
            && let Some(drag) = cx.state.arrange.drag.as_mut()
            && drag.target == item.target
        {
            drag.preview = dragged_rect(drag.from, drag.resize, at - drag.start);
        }
        if response.drag_stopped()
            && let Some(drag) = cx.state.arrange.drag.take()
        {
            cx.dispatch(AppAction::PlacePin {
                set,
                target: drag.target,
                rect: drag.preview,
            });
        }
    }
    let fits = cx
        .core
        .working_set(set)
        .is_some_and(|s| crate::core::grid::fits(&s.items, &item.target, item.rect));
    let color = if !fits {
        p.accent_2
    } else if dragging || body.hovered() || corner.hovered() {
        p.accent
    } else {
        p.n300
    };
    let painter = ui.painter();
    painter.rect_stroke(
        cell,
        egui::CornerRadius::same(2),
        egui::Stroke::new(1.0, color),
        egui::StrokeKind::Inside,
    );
    painter.rect_filled(handle, egui::CornerRadius::same(2), color);
}

fn card(cx: &mut DrawCtx<'_>, ui: &mut Ui, set: SetId, item: &PinnedItem) {
    match &item.target {
        PinTarget::Session(id) => {
            let Some(record) = cx.core.session(*id) else {
                return;
            };
            match record.kind {
                // Commands and services keep their controls and output.
                SessionKind::Command | SessionKind::Service => session_card(cx, ui, record),
                SessionKind::Agent(_) | SessionKind::Shell => set_card(cx, ui, record),
            }
        }
        PinTarget::File(pid, rel) => file_card(cx, ui, set, *pid, rel),
    }
}

/// How a file card shows its file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileMode {
    /// The source as it is, instead of Markdown rendered.
    pub raw: bool,
    /// Raw text wraps at the card; off, it scrolls sideways.
    pub wrap: bool,
}

impl Default for FileMode {
    fn default() -> Self {
        Self {
            raw: false,
            wrap: true,
        }
    }
}

/// A tooltip is a screenful at most; the rest is a click away in the
/// raw message dialog.
const HOVER_CHARS: usize = 1500;

fn capped(text: &str) -> String {
    let mut out: String = text.chars().take(HOVER_CHARS).collect();
    if out.len() < text.len() {
        out.push_str("\n…");
    }
    out
}

/// Height kept under the body for the send line and the action row.
const FOOTER: f32 = 62.0;

/// An agent or shell on the working set: state and project, name, the
/// last prompt on one line, as much of the last answer (a shell: the
/// pane's tail) as fits, a one-line send box, and the usual actions.
/// Hovering the prompt or the answer shows more; clicking the answer
/// opens it unformatted.
/// What an agent or shell card says, gathered before drawing.
struct SetCardText {
    kicker: String,
    meta: String,
    prompt: Option<String>,
    answer: Option<String>,
    reason: Option<String>,
    agent: bool,
    state: CardState,
    running: bool,
}

fn set_card_text(cx: &DrawCtx<'_>, record: &SessionRecord) -> SetCardText {
    let state = cx.core.card_state(record.id);
    let running = is_running(cx.core, record.id);
    let agent = matches!(record.kind, SessionKind::Agent(_));
    let project = cx
        .core
        .workspace(record.project)
        .map(|w| w.project.name.clone())
        .unwrap_or_default();
    let conversation = cx.state.conversations.get(&record.id).map(|(_, c)| c);
    let last = conversation.and_then(|c| c.turns.last());
    let prompt = last.map(|t| t.user.clone()).filter(|u| !u.is_empty());
    let answer = if agent {
        last.map(|t| {
            if t.final_text.is_empty() {
                t.activity
                    .last()
                    .map(|a| a.line.clone())
                    .unwrap_or_default()
            } else {
                t.final_text.clone()
            }
        })
    } else {
        cx.state
            .snapshots
            .get(&record.id)
            .map(|s| pane_tail(s, 40))
            .or_else(|| cx.state.captions.get(&record.id).cloned())
    }
    .filter(|a| !a.trim().is_empty());
    let reason = (state == CardState::WaitingOnYou)
        .then(|| record.activity_reason.clone())
        .flatten();
    let mut parts = vec![kind_label(record.kind).to_owned()];
    parts.extend(conversation.and_then(|c| c.model.clone()));
    parts.push(file_name(&record.cwd));
    SetCardText {
        kicker: format!(
            "{} · {project}",
            kicker_text(cx.core, record, &state, running)
        ),
        meta: parts.join(" · "),
        prompt,
        answer,
        reason,
        agent,
        state,
        running,
    }
}

fn set_card(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    let p = theme::palette(ui);
    let SetCardText {
        kicker,
        meta,
        prompt,
        answer,
        reason,
        agent,
        state,
        running,
    } = set_card_text(cx, record);
    let mut open = false;
    let mut show_raw = None;
    theme::surface(ui)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
            ui.style_mut().interaction.selectable_labels = false;
            ui.horizontal(|ui| {
                theme::kicker(ui, &kicker, p.state_text(&state));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    theme::status_dot(ui, &state, 8.0);
                });
            });
            let title = ui
                .add(
                    egui::Label::new(RichText::new(&record.name).text_style(theme::card_title()))
                        .truncate()
                        .sense(Sense::click()),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            open = title.clicked();
            title.context_menu(|ui| {
                if set_menu(cx, ui, &PinTarget::Session(record.id)) {
                    ui.close();
                }
            });
            ui.add(egui::Label::new(RichText::new(&meta).small().color(p.n600)).truncate());
            if let Some(prompt) = &prompt {
                let line = prompt.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("You: {line}"))
                            .text_style(theme::meta())
                            .color(p.accent_text),
                    )
                    .truncate(),
                )
                .on_hover_text(capped(prompt));
            }
            // The body takes what is left above the footer and is cut
            // there; the card's clip rectangle does the cutting.
            let body_height = (ui.available_height() - FOOTER).max(20.0);
            let body_rect =
                egui::Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), body_height));
            // A child that allocates nothing in the card: text taller
            // than the body is cut, not allowed to push the footer down.
            let mut body = ui.new_child(UiBuilder::new().max_rect(body_rect));
            body.set_clip_rect(body_rect.intersect(ui.clip_rect()));
            {
                let ui = &mut body;
                if let Some(reason) = &reason {
                    ui.label(
                        RichText::new(reason)
                            .text_style(theme::meta())
                            .color(p.accent_2_text),
                    );
                }
                if let Some(answer) = &answer {
                    let text = if agent {
                        RichText::new(answer)
                            .text_style(theme::excerpt())
                            .color(p.n800)
                    } else {
                        RichText::new(answer).monospace().color(p.n800)
                    };
                    ui.add(egui::Label::new(text).wrap());
                }
            }
            ui.advance_cursor_after_rect(body_rect);
            if let Some(answer) = &answer {
                let hover = ui
                    .interact(body_rect, ui.id().with(("body", record.id)), Sense::click())
                    .on_hover_text(capped(answer))
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if hover.clicked() {
                    show_raw = Some(answer.clone());
                }
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                actions(cx, ui, record, running);
                send_line(cx, ui, record, running);
            });
        });
    if let Some(text) = show_raw {
        cx.state.raw_message = Some(text);
    }
    if open {
        cx.dispatch(AppAction::ShowSession(record.id));
    }
}

/// The last `lines` lines of a pane with something on them.
fn pane_tail(snapshot: &str, lines: usize) -> String {
    let kept: Vec<&str> = snapshot
        .lines()
        .rev()
        .skip_while(|l| l.trim().is_empty())
        .take(lines)
        .collect();
    kept.into_iter().rev().collect::<Vec<_>>().join("\n")
}

/// One line to type into the session without opening it: Enter sends
/// it as a line to the pane. Off while the session is not running.
fn send_line(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord, running: bool) {
    let p = theme::palette(ui);
    let field_id = ui.id().with(("quick-send", record.id));
    let draft = cx.state.input_drafts.entry(record.id).or_default();
    let hint = if running {
        "Send a line…"
    } else {
        "Not running"
    };
    // The label is invisible but names the field for tests and screen
    // readers, as the session view's message box does.
    let label = ui.add(egui::Label::new(RichText::new("Line to send").size(0.1)));
    let response = ui
        .add_enabled(
            running,
            egui::TextEdit::singleline(draft)
                .id(field_id)
                .hint_text(hint)
                .font(egui::TextStyle::Body)
                .background_color(p.bg)
                .margin(egui::Margin::symmetric(8, 5))
                .desired_width(f32::INFINITY),
        )
        .labelled_by(label.id);
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.is_none());
    if response.lost_focus() && enter && !draft.trim().is_empty() {
        let text = draft.clone();
        cx.dispatch(AppAction::SendInput {
            id: record.id,
            text,
        });
        ui.memory_mut(|m| m.request_focus(field_id));
    }
}

/// A file on the working set: its name and project, the file itself
/// scrolling inside the card (Markdown rendered, or raw; raw text
/// wrapped or scrolling sideways), and Open in app and Take off.
fn file_card(
    cx: &mut DrawCtx<'_>,
    ui: &mut Ui,
    set: SetId,
    pid: crate::core::ProjectId,
    rel: &Path,
) {
    let p = theme::palette(ui);
    let Some(project) = cx.core.workspace(pid).map(|w| &w.project) else {
        return;
    };
    let path = project.root.join(rel);
    let target = PinTarget::File(pid, rel.to_path_buf());
    let project_name = project.name.clone();
    let mut open = false;
    let mut mode = cx.state.file_modes.get(&path).copied().unwrap_or_default();
    let text_file = {
        let slot = cx.state.previews.entry(path.clone()).or_insert(None);
        document::ensure_in(slot, &path);
        slot.as_ref()
            .is_some_and(|pr| matches!(pr.body, Body::Text(_) | Body::Markdown(_)))
    };
    let is_markdown = cx
        .state
        .previews
        .get(&path)
        .and_then(Option::as_ref)
        .is_some_and(|pr| matches!(pr.body, Body::Markdown(_)));
    theme::surface(ui)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
            ui.horizontal(|ui| {
                theme::kicker(ui, &format!("File · {project_name}"), p.n600);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.spacing_mut().button_padding = vec2(0.0, 2.0);
                    if text_file && (mode.raw || !is_markdown) {
                        let wrap = if mode.wrap { "Sideways" } else { "Wrap" };
                        if theme::ghost_muted(ui, wrap)
                            .on_hover_text("Wrap long lines, or scroll sideways for them")
                            .clicked()
                        {
                            mode.wrap = !mode.wrap;
                        }
                    }
                    if is_markdown {
                        let raw = if mode.raw { "Rendered" } else { "Raw" };
                        if theme::ghost_muted(ui, raw)
                            .on_hover_text("Markdown rendered, or the source as it is")
                            .clicked()
                        {
                            mode.raw = !mode.raw;
                        }
                    }
                });
            });
            let title = ui
                .add(
                    egui::Label::new(
                        RichText::new(file_name(&path)).text_style(theme::card_title()),
                    )
                    .truncate()
                    .sense(Sense::click()),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            open = title.clicked();
            title.context_menu(|ui| {
                if set_menu(cx, ui, &target) {
                    ui.close();
                }
            });
            let body_height = (ui.available_height() - 34.0).max(20.0);
            let body_rect =
                egui::Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), body_height));
            let mut body = ui.new_child(UiBuilder::new().max_rect(body_rect));
            body.set_clip_rect(body_rect.intersect(ui.clip_rect()));
            file_body(cx.state, &mut body, &path, mode);
            ui.advance_cursor_after_rect(body_rect);
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    ui.spacing_mut().button_padding = vec2(0.0, 4.0);
                    if theme::ghost(ui, "Open in app").clicked() {
                        cx.dispatch(AppAction::OpenDocument(path.clone()));
                    }
                    if theme::ghost_muted(ui, "Take off")
                        .on_hover_text("Take it off the working set")
                        .clicked()
                    {
                        cx.dispatch(AppAction::RemoveFromWorkingSet {
                            set,
                            target: target.clone(),
                        });
                    }
                });
            });
        });
    cx.state.file_modes.insert(path.clone(), mode);
    if open {
        cx.dispatch(AppAction::ShowDocument(pid, path));
    }
}

/// The file inside its card: a scroll area, wrapped or sideways, with
/// the preview rendered or its source.
fn file_body(state: &mut UiState, ui: &mut Ui, path: &Path, mode: FileMode) {
    let UiState {
        previews, markdown, ..
    } = state;
    let Some(preview) = previews.get(path).and_then(Option::as_ref) else {
        return;
    };
    let raw = match &preview.body {
        Body::Markdown(text) if mode.raw => Some(text),
        Body::Text(text) => Some(text),
        _ => None,
    };
    let width = ui.available_width();
    let scroll = if raw.is_some() && !mode.wrap {
        egui::ScrollArea::both()
    } else {
        egui::ScrollArea::vertical()
    };
    scroll
        .id_salt(("file-card", path))
        .auto_shrink(false)
        .show(ui, |ui| {
            if let Some(text) = raw {
                let label = egui::Label::new(RichText::new(text).monospace());
                if mode.wrap {
                    ui.set_max_width(width);
                    ui.add(label.wrap());
                } else {
                    ui.add(label.wrap_mode(egui::TextWrapMode::Extend));
                }
            } else {
                ui.set_max_width(width);
                document::draw_body(preview, markdown, ui);
            }
        });
}

/// The working-set menu for `target`: one line per set, checked where
/// the set holds it (a click toggles), and a line for a new set with
/// it. Returns the actions and whether a line was chosen.
#[must_use]
pub fn set_menu_actions(
    core: &AppCore,
    ui: &mut Ui,
    target: &PinTarget,
    columns: u32,
) -> (Vec<AppAction>, bool) {
    let p = theme::palette(ui);
    let holding = core.sets_holding(target);
    let mut actions = Vec::new();
    ui.label(theme::meta_text(ui, "Working sets").color(p.n600));
    for set in core.working_sets() {
        let on = holding.contains(&set.id);
        let label = if on {
            format!("✓ {}", set.name)
        } else {
            format!("   {}", set.name)
        };
        if ui.button(label).clicked() {
            actions.push(if on {
                AppAction::RemoveFromWorkingSet {
                    set: set.id,
                    target: target.clone(),
                }
            } else {
                AppAction::AddToWorkingSet {
                    set: set.id,
                    target: target.clone(),
                    columns,
                }
            });
        }
    }
    if ui.button("New working set with this").clicked() {
        actions.push(AppAction::NewWorkingSet {
            name: None,
            clone_of: None,
            with: Some(target.clone()),
            columns,
        });
    }
    let chosen = !actions.is_empty();
    (actions, chosen)
}

/// [`set_menu_actions`] dispatched through `cx`. Returns whether a line
/// was chosen, so the menu can close.
pub fn set_menu(cx: &mut DrawCtx<'_>, ui: &mut Ui, target: &PinTarget) -> bool {
    let columns = cx.state.working_set_columns;
    let (actions, chosen) = set_menu_actions(cx.core, ui, target, columns);
    for action in actions {
        cx.dispatch(action);
    }
    chosen
}

/// The set's name (or its editor), with Arrange, Rename, Clone, and
/// Delete on the right.
fn header(cx: &mut DrawCtx<'_>, ui: &mut Ui, set: SetId, name: &str, empty: bool) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if !empty {
                let arranging = cx.state.arrange.on;
                let button = if arranging {
                    theme::primary(ui, "Done")
                } else {
                    theme::secondary(ui, "Arrange")
                };
                if button
                    .on_hover_text("Drag cards to move them; drag a corner to resize")
                    .clicked()
                {
                    cx.state.arrange.on = !arranging;
                    cx.state.arrange.drag = None;
                }
            }
            if theme::ghost_muted(ui, "Delete").clicked() {
                cx.state.delete_set = Some(set);
            }
            if theme::ghost(ui, "Clone")
                .on_hover_text("A new working set with the same cards")
                .clicked()
            {
                cx.dispatch(AppAction::NewWorkingSet {
                    name: None,
                    clone_of: Some(set),
                    with: None,
                    columns: cx.state.working_set_columns,
                });
            }
            let editing = cx
                .state
                .set_rename
                .as_ref()
                .is_some_and(|(id, _)| *id == set);
            if !editing && theme::ghost(ui, "Rename").clicked() {
                cx.state.set_rename = Some((set, name.to_owned()));
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                if editing {
                    name_editor(cx, ui, set);
                } else {
                    ui.add(
                        egui::Label::new(RichText::new(name).text_style(theme::h1())).truncate(),
                    );
                }
            });
        });
    });
}

/// The name field while a rename is under way: Enter commits, Escape
/// cancels.
fn name_editor(cx: &mut DrawCtx<'_>, ui: &mut Ui, set: SetId) {
    let mut done = None;
    if let Some((_, draft)) = cx.state.set_rename.as_mut() {
        let label = ui.label("Working set name").id;
        let response = ui
            .add(egui::TextEdit::singleline(draft).desired_width(280.0))
            .labelled_by(label);
        response.request_focus();
        let (enter, escape) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if enter {
            done = Some(Some(draft.trim().to_owned()));
        } else if escape {
            done = Some(None);
        }
    }
    match done {
        Some(Some(name)) => {
            cx.state.set_rename = None;
            if !name.is_empty() {
                cx.dispatch(AppAction::RenameWorkingSet { set, name });
            }
        }
        Some(None) => cx.state.set_rename = None,
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_snaps_to_whole_units_and_respects_the_edges_and_minimums() {
        let from = GridRect {
            x: 2,
            y: 1,
            w: 10,
            h: 8,
        };
        // Less than half a unit is nothing; more rounds to the unit.
        assert_eq!(dragged_rect(from, false, vec2(10.0, 0.0)), from);
        assert_eq!(
            dragged_rect(from, false, vec2(UNIT * 2.6, -UNIT * 1.4)),
            GridRect { x: 5, y: 0, ..from }
        );
        // Never past the top-left corner.
        assert_eq!(
            dragged_rect(from, false, vec2(-UNIT * 9.0, -UNIT * 9.0)),
            GridRect { x: 0, y: 0, ..from }
        );
        // Resizing keeps the corner and never goes under the minimums.
        assert_eq!(
            dragged_rect(from, true, vec2(UNIT * 2.0, UNIT)),
            GridRect {
                w: 12,
                h: 9,
                ..from
            }
        );
        assert_eq!(
            dragged_rect(from, true, vec2(-UNIT * 20.0, -UNIT * 20.0)),
            GridRect {
                w: MIN_WIDTH,
                h: MIN_HEIGHT,
                ..from
            }
        );
    }

    #[test]
    fn the_pane_tail_keeps_the_last_lines_with_text() {
        assert_eq!(pane_tail("a\nb\nc\n\n\n", 2), "b\nc");
        assert_eq!(pane_tail("", 3), "");
        assert_eq!(capped("short"), "short");
        let long = "x".repeat(HOVER_CHARS + 5);
        assert!(capped(&long).ends_with('…'));
    }

    #[test]
    fn seven_units_are_a_board_card_and_columns_come_from_the_width() {
        let seven = cell_rect(
            egui::Pos2::ZERO,
            GridRect {
                x: 0,
                y: 0,
                w: 7,
                h: 5,
            },
        );
        assert!((seven.width() - 230.0).abs() < 0.01);
        assert_eq!(columns(1000.0), 29);
        assert_eq!(columns(10.0), crate::core::grid::MIN_WIDTH);
    }
}
