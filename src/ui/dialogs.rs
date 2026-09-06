//! Small modal dialogs: add a project, start a session. Each is a draft
//! struct in `UiState` that exists only while its window is open.

use std::path::PathBuf;

use egui::{Context, Ui};

use super::{DrawCtx, GAP};
use crate::core::{AgentKind, AppAction, Launch, Project, ProjectId, SessionKind};

#[derive(Debug, Default, Clone)]
pub struct AddProjectDraft {
    pub name: String,
    pub root: String,
}

#[derive(Debug, Clone)]
pub struct NewSessionDraft {
    pub project: ProjectId,
    pub name: String,
    pub kind: SessionKind,
    pub cwd: String,
    pub command: String,
}

impl NewSessionDraft {
    #[must_use]
    pub fn new(project: &Project) -> Self {
        Self {
            project: project.id,
            name: String::new(),
            kind: SessionKind::Shell,
            cwd: project.root.display().to_string(),
            command: String::new(),
        }
    }

    /// The action this draft becomes. Agents get `Launch::Shell` too: the
    /// core composes their argv from the kind, so the UI never guesses it.
    #[must_use]
    pub fn into_action(self, shell: String) -> AppAction {
        let launch = match self.kind {
            SessionKind::Shell | SessionKind::Agent(_) => Launch::Shell,
            SessionKind::Command | SessionKind::Service => Launch::Command {
                command: self.command.trim().to_string(),
                shell,
            },
        };
        AppAction::NewSession {
            project: self.project,
            name: self.name.trim().to_string(),
            kind: self.kind,
            cwd: PathBuf::from(self.cwd.trim()),
            launch,
        }
    }
}

pub fn show(cx: &mut DrawCtx<'_>, ctx: &Context) {
    add_project(cx, ctx);
    new_session(cx, ctx);
}

/// A single-line text field that tests (and screen readers) find by its
/// label.
fn field(ui: &mut Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        let id = ui.label(label).id;
        ui.add(egui::TextEdit::singleline(value).desired_width(320.0))
            .labelled_by(id);
    });
}

fn add_project(cx: &mut DrawCtx<'_>, ctx: &Context) {
    let Some(mut draft) = cx.state.add_project.take() else {
        return;
    };
    let mut keep = true;
    egui::Window::new("Add a project")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
            field(ui, "Name", &mut draft.name);
            field(ui, "Root path", &mut draft.root);
            ui.horizontal(|ui| {
                let ready = !draft.name.trim().is_empty() && !draft.root.trim().is_empty();
                if ui.add_enabled(ready, egui::Button::new("Add")).clicked() {
                    cx.dispatch(AppAction::AddProject {
                        name: draft.name.trim().to_string(),
                        root: PathBuf::from(draft.root.trim()),
                    });
                    keep = false;
                }
                if ui.button("Cancel").clicked() {
                    keep = false;
                }
            });
        });
    if keep {
        cx.state.add_project = Some(draft);
    }
}

fn new_session(cx: &mut DrawCtx<'_>, ctx: &Context) {
    let Some(mut draft) = cx.state.new_session.take() else {
        return;
    };
    let mut keep = true;
    egui::Window::new("Create a session")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
            field(ui, "Name", &mut draft.name);
            kind_radios(ui, &mut draft.kind);
            field(ui, "Directory", &mut draft.cwd);
            let needs_command = matches!(draft.kind, SessionKind::Command | SessionKind::Service);
            if needs_command {
                field(ui, "Command line", &mut draft.command);
            }
            ui.horizontal(|ui| {
                let ready = !draft.name.trim().is_empty()
                    && !draft.cwd.trim().is_empty()
                    && (!needs_command || !draft.command.trim().is_empty());
                if ui.add_enabled(ready, egui::Button::new("Create")).clicked() {
                    let action = draft.clone().into_action(super::session::login_shell());
                    cx.dispatch(action);
                    keep = false;
                }
                if ui.button("Cancel").clicked() {
                    keep = false;
                }
            });
        });
    if keep {
        cx.state.new_session = Some(draft);
    }
}

fn kind_radios(ui: &mut Ui, kind: &mut SessionKind) {
    ui.horizontal(|ui| {
        ui.label("Kind");
        ui.radio_value(kind, SessionKind::Shell, "Shell");
        ui.radio_value(
            kind,
            SessionKind::Agent(AgentKind::ClaudeCode),
            "Claude Code",
        );
        ui.radio_value(kind, SessionKind::Agent(AgentKind::Codex), "Codex");
        ui.radio_value(kind, SessionKind::Command, "Command");
        ui.radio_value(kind, SessionKind::Service, "Service");
    });
}
