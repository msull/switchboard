//! One button per command and service of a project, drawn under the
//! board strip and the session header so a rerun or a start/stop is a
//! click away from anywhere in the project. Entries that are not
//! approved yet open the Run tab instead, where the definition can be
//! read before it is allowed to run.

use egui::{RichText, Ui};

use super::cards::{is_running, state_color};
use super::{DrawCtx, GAP};
use crate::core::{AppAction, Launch, ProjectId, SessionKind, SideTab};

/// `open_side` is set on a session view, where the side panel may be
/// closed and has to be opened for the Run tab to show.
pub fn show(cx: &mut DrawCtx<'_>, ui: &mut Ui, pid: ProjectId, open_side: bool) {
    let core = cx.core;
    let entries = core.run_entries(pid);
    if entries.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        for record in entries {
            let state = core.card_state(record.id);
            let running = is_running(core, record.id);
            let service = record.kind == SessionKind::Service;
            let glyph = if service { "•" } else { "▶" };
            let mut text = RichText::new(format!("{glyph} {}", record.name));
            let command = match &record.launch {
                Launch::Command { command, .. } => command.clone(),
                Launch::Shell | Launch::Argv(_) => String::new(),
            };
            let (hover, action) = if !record.runnable() {
                text = text.weak();
                (
                    format!("{command}\nNot approved yet: opens the Run tab"),
                    None,
                )
            } else if service && running {
                text = text.color(state_color(ui, &state));
                (
                    format!("{command}\nRunning: click to stop"),
                    Some(AppAction::KillSession(record.id)),
                )
            } else if running {
                text = text.color(state_color(ui, &state));
                (
                    format!("{command}\nRunning: click to show"),
                    Some(AppAction::ShowSession(record.id)),
                )
            } else if service {
                (
                    format!("{command}\nClick to start"),
                    Some(AppAction::RestartSession(record.id)),
                )
            } else {
                (
                    format!("{command}\nClick to run"),
                    Some(AppAction::RestartSession(record.id)),
                )
            };
            if ui.button(text).on_hover_text(hover).clicked() {
                if let Some(action) = action {
                    cx.dispatch(action);
                } else {
                    cx.dispatch(AppAction::SetSideTab(SideTab::Run));
                    if open_side && !core.settings().files_open {
                        cx.dispatch(AppAction::SetFilesOpen(true));
                    }
                }
            }
        }
    });
}
