//! Session and document cards, plus the colors and text helpers the
//! other views share.

use std::path::Path;
use std::time::SystemTime;

use egui::{Color32, RichText, Sense, Ui, vec2};

use super::{DrawCtx, GAP};
use crate::core::{AppAction, AppCore, CardState, ProjectId, RecordId, SessionKind, SessionRecord};
use crate::ports::host::Liveness;

pub const CARD_SIZE: egui::Vec2 = vec2(260.0, 136.0);

/// Semantic colors for card states. Orange and green are fixed; the rest
/// come from the theme so dim text stays readable in light and dark.
pub fn state_color(ui: &Ui, state: &CardState) -> Color32 {
    match state {
        CardState::WaitingOnYou => Color32::from_rgb(235, 140, 0),
        CardState::Working => Color32::from_rgb(60, 170, 80),
        CardState::Idle => ui.visuals().text_color(),
        CardState::Exited(Some(code)) if *code != 0 => ui.visuals().error_fg_color,
        CardState::Exited(_) | CardState::NotRunning | CardState::NotResumable => {
            ui.visuals().weak_text_color()
        }
    }
}

/// Dot color for a project in the switcher: red if anything waits,
/// green if anything works, grey otherwise.
pub fn project_dot_color(core: &AppCore, project: ProjectId) -> Color32 {
    let states: Vec<CardState> = core
        .workspace(project)
        .map(|w| w.sessions.iter().map(|s| core.card_state(s.id)).collect())
        .unwrap_or_default();
    if states.contains(&CardState::WaitingOnYou) {
        Color32::from_rgb(220, 60, 60)
    } else if states.contains(&CardState::Working) {
        Color32::from_rgb(60, 170, 80)
    } else {
        Color32::GRAY
    }
}

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

/// One session card. The whole card opens the session; the buttons act
/// on it without opening.
pub fn session_card(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    let state = cx.core.card_state(record.id);
    let running = is_running(cx.core, record.id);
    let caption = cx
        .state
        .captions
        .get(&record.id)
        .cloned()
        .or_else(|| cx.core.host_status(record.id).and_then(|h| h.title.clone()));

    // The board lays cards out left to right; the card's own content
    // stacks top to bottom regardless.
    let response = ui
        .allocate_ui_with_layout(CARD_SIZE, egui::Layout::top_down(egui::Align::Min), |ui| {
            egui::Frame::group(ui.style())
                .inner_margin(GAP)
                .show(ui, |ui| {
                    ui.set_min_size(CARD_SIZE - vec2(2.0 * GAP, 2.0 * GAP));
                    ui.set_max_width(CARD_SIZE.x - 2.0 * GAP);
                    ui.spacing_mut().item_spacing.y = GAP / 2.0;
                    // Text on a card is not selectable; otherwise a click
                    // on the name would start a selection instead of
                    // opening the session.
                    ui.style_mut().interaction.selectable_labels = false;
                    ui.strong(&record.name);
                    ui.label(
                        RichText::new(format!(
                            "{} · {}",
                            kind_label(record.kind),
                            file_name(&record.cwd)
                        ))
                        .weak(),
                    );
                    state_line(ui, record, &state, running);
                    if let Some(caption) = caption {
                        ui.add(
                            egui::Label::new(RichText::new(last_line(&caption)).weak().small())
                                .truncate(),
                        );
                    }
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                        card_buttons(cx, ui, record.id, running);
                    });
                });
        })
        .response;
    if response.interact(Sense::click()).clicked() {
        cx.dispatch(AppAction::ShowSession(record.id));
    }
}

fn state_line(ui: &mut Ui, record: &SessionRecord, state: &CardState, running: bool) {
    ui.horizontal(|ui| {
        let mut label = RichText::new(state.label()).color(state_color(ui, state));
        if *state == CardState::NotResumable {
            label = label.strikethrough();
        }
        ui.label(label);
        let (prefix, since) = if running {
            ("running", record.last_seen)
        } else {
            ("since", record.created)
        };
        ui.label(RichText::new(format!("{prefix} {}", since_text(since))).weak());
    });
}

fn card_buttons(cx: &mut DrawCtx<'_>, ui: &mut Ui, id: RecordId, running: bool) {
    ui.horizontal(|ui| {
        if running {
            if ui.small_button("Open").clicked() {
                cx.dispatch(AppAction::ReturnToSession(id));
            }
            if ui.small_button("Kill").clicked() {
                cx.dispatch(AppAction::KillSession(id));
            }
        } else {
            if ui.small_button("Return").clicked() {
                cx.dispatch(AppAction::ReturnToSession(id));
            }
            if ui.small_button("Remove").clicked() {
                cx.dispatch(AppAction::RemoveSession(id));
            }
        }
    });
}

/// A pinned document. Opening it needs an `Effect::OpenPath`, which no
/// UI action produces yet, so the card only shows the file for now.
// TODO: dispatch an open-document action once the core exposes one.
pub fn document_card(ui: &mut Ui, path: &Path) {
    ui.allocate_ui(vec2(CARD_SIZE.x, 48.0), |ui| {
        egui::Frame::group(ui.style())
            .inner_margin(GAP)
            .show(ui, |ui| {
                ui.set_min_width(CARD_SIZE.x - 2.0 * GAP);
                ui.strong(file_name(path));
                ui.label(RichText::new(path.display().to_string()).weak().small());
            });
    });
}

fn last_line(text: &str) -> String {
    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}
