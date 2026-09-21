//! The card: one widget for every board entry (agent, shell, command,
//! service), differing only by its kicker, glyph, and actions. Also the
//! grid that lays cards out and the text helpers the views share.

use std::path::Path;
use std::time::SystemTime;

use egui::{RichText, Sense, Ui, vec2};

use super::{DrawCtx, theme};
use crate::core::{
    AppAction, AppCore, Approval, CardState, Launch, PinTarget, ProjectId, RecordId, SessionKind,
    SessionRecord,
};
use crate::ports::host::Liveness;
use crate::ports::transcript::Conversation;

/// Cards are at least this wide; the grid adds columns as room allows.
pub const MIN_CARD_WIDTH: f32 = 230.0;
/// Agent and shell cards; command and service cards are shorter.
pub const SESSION_CARD_HEIGHT: f32 = 172.0;
pub const ENTRY_CARD_HEIGHT: f32 = 184.0;
const GRID_GAP: f32 = 14.0;

#[must_use]
pub fn kind_label(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Agent(agent) => agent.label(),
        SessionKind::Command => "command",
        SessionKind::Service => "service",
        SessionKind::Shell => "shell",
    }
}

/// Last path component, or the whole path when there is none.
#[must_use]
pub fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// "3m", "2h", "5d": how long ago `then` was. The UI may read the wall
/// clock; the core may not.
#[must_use]
pub fn since_text(then: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(then)
        .map_or(0, |d| d.as_secs());
    match secs {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

/// Is the host process alive for this record?
pub fn is_running(core: &AppCore, id: RecordId) -> bool {
    matches!(
        core.host_status(id).map(|h| &h.liveness),
        Some(Liveness::Running { .. })
    )
}

/// Sort key for cards: state rank first (waiting on top), then the
/// board order the user chose.
pub fn card_key(core: &AppCore, record: &SessionRecord) -> (u8, u32) {
    (core.card_state(record.id).rank(), record.layout.order)
}

/// Lay `count` cells out in a grid whose columns are as many
/// `MIN_CARD_WIDTH` cards as fit the width, 14 px apart, every cell
/// `height` tall. `cell` draws cell `i`.
pub fn grid(ui: &mut Ui, count: usize, height: f32, mut cell: impl FnMut(&mut Ui, usize)) {
    let width = ui.available_width();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let columns = (((width + GRID_GAP) / (MIN_CARD_WIDTH + GRID_GAP)).floor() as usize).max(1);
    #[allow(clippy::cast_precision_loss)]
    let cell_width = (width - GRID_GAP * (columns as f32 - 1.0)) / columns as f32;
    let mut i = 0;
    while i < count {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GRID_GAP;
            for _ in 0..columns {
                if i >= count {
                    break;
                }
                ui.allocate_ui_with_layout(
                    vec2(cell_width, height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_size(vec2(cell_width, height));
                        ui.set_max_width(cell_width);
                        cell(ui, i);
                    },
                );
                i += 1;
            }
        });
        ui.add_space(GRID_GAP - ui.spacing().item_spacing.y);
    }
}

/// The kicker line of a card: the state, and for commands and services
/// the kind before it. The reason a session waits is the body instead.
pub(super) fn kicker_text(
    core: &AppCore,
    record: &SessionRecord,
    state: &CardState,
    running: bool,
) -> String {
    let age = if running {
        since_text(record.last_seen)
    } else {
        since_text(record.created)
    };
    let label = state.label();
    match record.kind {
        SessionKind::Command | SessionKind::Service => {
            format!("{} · {label}", kind_label(record.kind))
        }
        SessionKind::Agent(_) | SessionKind::Shell => match state {
            CardState::WaitingOnYou => {
                let _ = core;
                label
            }
            _ => format!("{label} · {age}"),
        },
    }
}

/// One card. Its title opens the session; the buttons act on it without
/// opening. `not running` cards are outlined instead of filled.
pub fn session_card(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    let p = theme::palette(ui);
    let state = cx.core.card_state(record.id);
    let running = is_running(cx.core, record.id);
    let entry = matches!(record.kind, SessionKind::Command | SessionKind::Service);
    let hollow = !running && !entry;
    let conversation = cx.state.conversations.get(&record.id).map(|(_, c)| c);
    // An agent's pane ends in its own chrome (prompt hints, token
    // counts), so its excerpt comes from the transcript instead.
    let caption = if matches!(record.kind, SessionKind::Agent(_)) {
        conversation.and_then(agent_excerpt)
    } else {
        cx.state
            .captions
            .get(&record.id)
            .cloned()
            .or_else(|| cx.core.host_status(record.id).and_then(|h| h.title.clone()))
    };
    let model = conversation.and_then(|c| c.model.clone());
    let kicker = if entry {
        format!(
            "{} · {}",
            kind_label(record.kind),
            super::runs::kicker(record, running, SystemTime::now())
        )
    } else {
        kicker_text(cx.core, record, &state, running)
    };
    let reason = (state == CardState::WaitingOnYou)
        .then(|| record.activity_reason.clone())
        .flatten();

    let mut frame = egui::Frame::new()
        .corner_radius(2)
        .inner_margin(egui::Margin::symmetric(14, 12));
    frame = if hollow {
        frame.stroke(egui::Stroke::new(1.0, p.n300))
    } else {
        frame.fill(p.surface)
    };
    let mut open = false;
    frame.show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
        // Text on a card is not selectable; otherwise a click on the
        // name would start a selection instead of opening the session.
        ui.style_mut().interaction.selectable_labels = false;
        ui.horizontal(|ui| {
            theme::kicker(ui, &kicker, p.state_text(&state));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if record.kind == SessionKind::Command {
                    ui.label(RichText::new("▶").small().color(p.n600));
                } else {
                    theme::status_dot(ui, &state, 8.0);
                }
            });
        });
        let title_color = if hollow { p.n700 } else { p.text };
        let title = ui
            .add(
                egui::Label::new(
                    RichText::new(&record.name)
                        .text_style(theme::card_title())
                        .color(title_color),
                )
                .truncate()
                .sense(Sense::click()),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        open = title.clicked();
        title.context_menu(|ui| {
            if super::working_set::set_menu(cx, ui, &PinTarget::Session(record.id)) {
                ui.close();
            }
        });
        if entry {
            super::runs::card_body(cx, ui, record);
        } else {
            card_body(ui, record, entry, model, reason, caption.as_deref());
        }
        if !running && !entry && state == CardState::NotResumable {
            ui.label(
                RichText::new("not resumable")
                    .small()
                    .color(p.accent_2_text),
            );
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            actions(cx, ui, record, running);
        });
    });
    if open {
        cx.dispatch(AppAction::ShowSession(record.id));
    }
}

/// The meta line and the body of a card: kind, model, and directory (or
/// the command for an entry), then why the session waits and the pane's
/// last line.
fn card_body(
    ui: &mut Ui,
    record: &SessionRecord,
    entry: bool,
    model: Option<String>,
    reason: Option<String>,
    caption: Option<&str>,
) {
    let p = theme::palette(ui);
    match &record.launch {
        Launch::Command { command, .. } if entry => {
            ui.add(egui::Label::new(theme::mono_text(ui, command)).truncate());
        }
        _ => {
            let mut parts = vec![kind_label(record.kind).to_owned()];
            parts.extend(model);
            if !entry {
                parts.push(file_name(&record.cwd));
            }
            ui.add(
                egui::Label::new(RichText::new(parts.join(" · ")).small().color(p.n600)).truncate(),
            );
        }
    }
    // The body: why the session waits, or else the excerpt, clamped to
    // two lines so the action row below keeps its place. Commands keep
    // theirs in mono, one line.
    let body_style = theme::excerpt();
    if let Some(reason) = reason {
        let text = clamp_lines(ui, &reason, &theme::meta(), 2);
        ui.add(
            egui::Label::new(
                RichText::new(text)
                    .text_style(theme::meta())
                    .color(p.accent_2_text),
            )
            .wrap(),
        );
        return;
    }
    let body = caption.map(last_line);
    if let Some(body) = body.filter(|b| !b.is_empty()) {
        if entry {
            ui.add(egui::Label::new(RichText::new(body).monospace().color(p.n800)).truncate());
        } else {
            let text = clamp_lines(ui, &body, &body_style, 2);
            ui.add(
                egui::Label::new(RichText::new(text).text_style(body_style).color(p.n800)).wrap(),
            );
        }
    }
}

/// `text` cut so that, wrapped at the current width in `style`, it takes
/// at most `lines` rows, with an ellipsis where it was cut.
fn clamp_lines(ui: &Ui, text: &str, style: &egui::TextStyle, lines: usize) -> String {
    let font = style.resolve(ui.style());
    let width = ui.available_width();
    let galley = ui
        .painter()
        .layout(text.to_owned(), font, egui::Color32::BLACK, width);
    if galley.rows.len() <= lines {
        return text.to_owned();
    }
    // Keep the glyphs of the first rows, less room for the ellipsis.
    let keep: usize = galley.rows.iter().take(lines).map(|r| r.glyphs.len()).sum();
    let cut: String = text.chars().take(keep.saturating_sub(2)).collect();
    format!("{}…", cut.trim_end())
}

/// The action row: ghost buttons flush left, destructive ones last in
/// neutral.
pub(super) fn actions(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord, running: bool) {
    ui.horizontal(|ui| {
        // No side padding: the text sits flush with the title above, and a
        // negative space would push the row's edge out and grow the panel.
        ui.spacing_mut().item_spacing.x = 14.0;
        ui.spacing_mut().button_padding = vec2(0.0, 4.0);
        let id = record.id;
        match record.kind {
            SessionKind::Agent(_) | SessionKind::Shell => {
                if running {
                    if theme::ghost(ui, "Open").clicked() {
                        cx.dispatch(AppAction::ReturnToSession(id));
                    }
                    if theme::ghost_muted(ui, "Kill").clicked() {
                        cx.dispatch(AppAction::KillSession(id));
                    }
                } else {
                    if theme::ghost(ui, "Return").clicked() {
                        cx.dispatch(AppAction::ReturnToSession(id));
                    }
                    if theme::ghost_muted(ui, "Remove").clicked() {
                        cx.dispatch(AppAction::RemoveSession(id));
                    }
                }
            }
            SessionKind::Command | SessionKind::Service => {
                super::runs::actions(cx, ui, record, running);
            }
        }
    });
}

/// A live defined entry would come back on the next read, so Remove is
/// for the user's own records and orphans only.
pub(super) fn removable(record: &SessionRecord) -> bool {
    matches!(
        record.approval(),
        Approval::NotApplicable | Approval::Orphaned
    )
}

/// The dashed "+ New session" cell that ends the agents grid.
pub fn new_session_cell(ui: &mut Ui) -> bool {
    let p = theme::palette(ui);
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click());
    let text = "+ New session";
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    if response.hovered() {
        ui.painter().rect_filled(rect, 2.0, p.text_alpha(0.04));
    }
    theme::dashed_rect(ui, rect.shrink(0.5), p.n400);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::new(14.0, egui::FontFamily::Proportional),
        p.accent_text,
    );
    response.clicked()
}

/// A pinned document: its name previews it; the buttons open it in the
/// default app or unpin it. `rel` is the path as stored (relative to the
/// project root), `path` the absolute one.
pub fn document_card(cx: &mut DrawCtx<'_>, ui: &mut Ui, pid: ProjectId, rel: &Path, path: &Path) {
    let p = theme::palette(ui);
    let mut open = false;
    theme::surface(ui)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
            ui.style_mut().interaction.selectable_labels = false;
            theme::kicker(ui, "Document", p.n600);
            open = ui
                .add(
                    egui::Label::new(
                        RichText::new(file_name(path)).text_style(theme::card_title()),
                    )
                    .truncate()
                    .sense(Sense::click()),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked();
            // The folder, when there is one; the name is above.
            if let Some(dir) = rel.parent().filter(|d| !d.as_os_str().is_empty()) {
                ui.add(
                    egui::Label::new(theme::mono_text(ui, dir.display().to_string())).truncate(),
                );
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.horizontal(|ui| {
                    // No side padding: the text sits flush with the title above, and a

                    // negative space would push the row's edge out and grow the panel.

                    ui.spacing_mut().item_spacing.x = 14.0;
                    ui.spacing_mut().button_padding = vec2(0.0, 4.0);
                    if theme::ghost(ui, "Open in app")
                        .on_hover_text("Open with the default app")
                        .clicked()
                    {
                        cx.dispatch(AppAction::OpenDocument(path.to_path_buf()));
                    }
                    if theme::ghost_muted(ui, "Unpin").clicked() {
                        cx.dispatch(AppAction::UnpinDocument(pid, rel.to_path_buf()));
                    }
                });
            });
        });
    if open {
        cx.dispatch(AppAction::ShowDocument(pid, path.to_path_buf()));
    }
}

/// The last line of a pane with something to read on it: a bare prompt
/// glyph is not worth a line, and the serif has no shape for it anyway.
/// What an agent card says under its title: the first line of the last
/// answer, else what the agent is doing, else the prompt it is on.
fn agent_excerpt(c: &Conversation) -> Option<String> {
    let turn = c.turns.last()?;
    let first_line = |text: &str| {
        text.lines()
            .map(|l| l.trim().trim_start_matches(['#', '-', '*', '>']).trim())
            .find(|l| !l.is_empty())
            .map(str::to_owned)
    };
    let text = first_line(&turn.final_text)
        .or_else(|| turn.activity.last().map(|a| a.line.clone()))
        .or_else(|| first_line(&turn.user).map(|u| format!("You: {u}")))?;
    let mut out: String = text.chars().take(160).collect();
    if out.len() < text.len() {
        out.push('…');
    }
    Some(out)
}

fn last_line(text: &str) -> String {
    text.lines()
        .rev()
        .find(|l| l.chars().any(char::is_alphanumeric))
        .unwrap_or("")
        .trim()
        .to_string()
}
