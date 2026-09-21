//! One button per command and service of a project, drawn under the
//! board strip and the session header so a rerun or a start/stop is a
//! click away from anywhere in the project. Entries that are not
//! approved yet open the Run tab instead, where the definition can be
//! read before it is allowed to run.

use egui::{RichText, Ui};

use super::cards::is_running;
use super::{DrawCtx, theme};
use crate::core::{AppAction, Launch, ProjectId, SessionKind, SideTab};

/// `open_side` is set on a session view, where the side panel may be
/// closed and has to be opened for the Run tab to show.
pub fn show(cx: &mut DrawCtx<'_>, ui: &mut Ui, pid: ProjectId, open_side: bool) {
    let core = cx.core;
    let entries = core.run_entries(pid);
    if entries.is_empty() {
        return;
    }
    let p = theme::palette(ui);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.spacing_mut().button_padding = egui::vec2(8.0, 5.0);
        for record in entries {
            let state = core.card_state(record.id);
            let running = is_running(core, record.id);
            let service = record.kind == SessionKind::Service;
            let command = match &record.launch {
                Launch::Command { command, .. } => command.clone(),
                Launch::Shell | Launch::Argv(_) => String::new(),
            };
            let last = super::runs::kicker(record, running, std::time::SystemTime::now());
            // The name opens the entry's page; the glyph beside it is the
            // one thing here that starts or stops a run.
            let mut name = RichText::new(&record.name).text_style(theme::meta());
            if !record.runnable() {
                name = name.color(p.n500);
            } else if running {
                name = name.color(p.state_text(&state));
            }
            let name_button = ui
                .add(egui::Button::new(name).frame_when_inactive(false))
                .on_hover_text(format!("{command}\n{last}\nOpen: runs and output"));
            // Screen readers and tests tell the two buttons apart by
            // what they do, not by the glyph.
            ui.ctx().accesskit_node_builder(name_button.id, |node| {
                node.set_label(format!("Open {}", record.name));
            });
            let opened = name_button.clicked();
            if opened {
                if record.runnable() {
                    cx.dispatch(AppAction::ShowSession(record.id));
                } else {
                    cx.dispatch(AppAction::SetSideTab(SideTab::Run));
                    if open_side && !core.settings().files_open {
                        cx.dispatch(AppAction::SetFilesOpen(true));
                    }
                }
            }
            let (glyph, hint, action) = if running {
                ("■", "Stop", Some(AppAction::KillSession(record.id)))
            } else if !record.runnable() {
                ("▶", "Not approved yet: open it on the Run tab", None)
            } else if service {
                ("▶", "Start", Some(AppAction::RestartSession(record.id)))
            } else {
                ("▶", "Run", Some(AppAction::RestartSession(record.id)))
            };
            let glyph_text =
                RichText::new(glyph)
                    .text_style(theme::meta())
                    .color(if action.is_some() {
                        p.accent_text
                    } else {
                        p.n500
                    });
            let glyph_button = ui
                .add(egui::Button::new(glyph_text).frame_when_inactive(false))
                .on_hover_text(hint);
            let verb = if running {
                "Stop"
            } else if service {
                "Start"
            } else {
                "Run"
            };
            ui.ctx().accesskit_node_builder(glyph_button.id, |node| {
                node.set_label(format!("{verb} {}", record.name));
            });
            if glyph_button.clicked()
                && let Some(action) = action
            {
                cx.dispatch(action);
            }
            // A command ends on its own, so a running one gets a spinner
            // and its latest line right here; a service is just on.
            if running && !service {
                ui.add(egui::Spinner::new().size(ui.text_style_height(&egui::TextStyle::Body)));
                if let Some(line) = cx.state.captions.get(&record.id) {
                    ui.add(egui::Label::new(theme::mono_text(ui, line)).truncate());
                }
            }
            ui.add_space(6.0);
        }
    });
}
