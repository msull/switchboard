//! The project config editor: `.switchboard/project.json` as text, with
//! the options it takes listed beside it and the parse result shown as
//! it is typed. Save is the one write Switchboard makes into a project
//! directory, and only on the user's click.

use egui::{Context, RichText, Ui};

use super::{DrawCtx, GAP, theme};
use crate::adapters::project_config::parse;
use crate::app::Services;
use crate::core::{AppAction, AppCore, ProjectId};

/// What a new file starts as.
const TEMPLATE: &str =
    "{\n  \"version\": 1,\n  \"commands\": [],\n  \"services\": [],\n  \"show\": []\n}\n";

/// The options, one line each, as the editor lists them.
const OPTIONS: &[(&str, &str)] = &[
    ("version", "1, the only version this build reads"),
    (
        "commands",
        "[{ name, command, cwd?, env? }]: one-shot commands on the Run tab; run after you approve them",
    ),
    (
        "services",
        "[{ name, command, cwd?, env?, autostart? }]: long-running; autostart needs approval",
    ),
    (
        "show",
        "[\"folder\", …]: folders the file side lists despite the root's .gitignore",
    ),
    (
        "cwd",
        "relative to the root, without ..; env lists variable names the command expects",
    ),
];

/// The editor's state while it is open.
#[derive(Debug, Clone)]
pub struct ConfigDraft {
    pub pid: ProjectId,
    pub name: String,
    pub text: String,
    /// The file could not be read; Save would replace whatever is there.
    pub read_error: Option<String>,
    /// There was no file when the editor opened.
    pub fresh: bool,
}

impl ConfigDraft {
    /// The file as it is now, or the template for a project without one.
    #[must_use]
    pub fn open(cx: &DrawCtx<'_>, pid: ProjectId) -> Option<Self> {
        Self::read(cx.core, cx.services, pid)
    }

    /// The same, from the app itself (the script dev aid).
    #[must_use]
    pub fn read(core: &AppCore, services: &Services, pid: ProjectId) -> Option<Self> {
        let workspace = core.workspace(pid)?;
        let (text, read_error, fresh) =
            match services.project_config.read_text(&workspace.project.root) {
                Ok(Some(text)) => (text, None, false),
                Ok(None) => (TEMPLATE.to_owned(), None, true),
                Err(e) => (TEMPLATE.to_owned(), Some(e), true),
            };
        Some(Self {
            pid,
            name: workspace.project.name.clone(),
            text,
            read_error,
            fresh,
        })
    }
}

pub fn show(cx: &mut DrawCtx<'_>, ctx: &Context) {
    let Some(mut draft) = cx.state.config_dialog.take() else {
        return;
    };
    let mut keep = true;
    let mut save = false;
    egui::Window::new("")
        .id(egui::Id::new("project-config"))
        .title_bar(false)
        .collapsible(false)
        .resizable(true)
        .default_width(720.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            let p = theme::palette(ui);
            ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
            ui.label(RichText::new(format!("{} · .switchboard/project.json", draft.name)).text_style(theme::brand()));
            if let Some(e) = &draft.read_error {
                ui.label(RichText::new(format!("Could not read the file: {e}. Save replaces it.")).color(p.accent_2_text));
            } else if draft.fresh {
                ui.label(theme::meta_text(ui, "No file yet; Save creates it.".to_owned()));
            }
            options(ui);
            ui.add_space(4.0);
            let label = ui.label(theme::meta_text(ui, "Config text".to_owned())).id;
            egui::ScrollArea::vertical()
                .id_salt("project-config-text")
                .max_height(360.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut draft.text)
                            .code_editor()
                            .desired_rows(14)
                            .desired_width(f32::INFINITY),
                    )
                    .labelled_by(label);
                });
            let valid = status(ui, &draft.text);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Save writes the file in the project; entries still need approval on the Run tab.")
                        .small()
                        .color(p.n600),
                );
                if theme::ghost_muted(ui, "Cancel").clicked() {
                    keep = false;
                }
                if ui.add_enabled_ui(valid, |ui| theme::primary(ui, "Save")).inner.clicked() {
                    save = true;
                    keep = false;
                }
            });
        });
    if save {
        cx.dispatch(AppAction::SaveProjectConfig {
            project: draft.pid,
            text: draft.text.clone(),
        });
    }
    if keep {
        cx.state.config_dialog = Some(draft);
    }
}

/// The options list: key in mono, meaning beside it.
fn options(ui: &mut Ui) {
    let p = theme::palette(ui);
    theme::kicker(ui, "Options", p.n600);
    egui::Grid::new("project-config-options")
        .num_columns(2)
        .spacing(egui::vec2(12.0, 2.0))
        .show(ui, |ui| {
            for (key, meaning) in OPTIONS {
                ui.label(theme::mono_text(ui, (*key).to_owned()));
                ui.add(
                    egui::Label::new(
                        RichText::new(*meaning)
                            .text_style(theme::meta())
                            .color(p.n700),
                    )
                    .wrap(),
                );
                ui.end_row();
            }
        });
}

/// What the text parses to, or why it does not. Returns whether it can
/// be saved.
fn status(ui: &mut Ui, text: &str) -> bool {
    let p = theme::palette(ui);
    match parse(text, String::new()) {
        Ok(config) => {
            let commands = config
                .entries
                .iter()
                .filter(|e| e.kind == crate::core::SessionKind::Command)
                .count();
            let services = config.entries.len() - commands;
            ui.label(theme::meta_text(
                ui,
                format!(
                    "Valid: {commands} commands · {services} services · {} shown folders",
                    config.show.len()
                ),
            ));
            for warning in &config.warnings {
                ui.label(
                    RichText::new(warning)
                        .text_style(theme::meta())
                        .color(p.accent_2_text),
                );
            }
            true
        }
        Err(e) => {
            ui.label(
                RichText::new(format!("Not valid: {e}"))
                    .text_style(theme::meta())
                    .color(p.accent_2_text),
            );
            false
        }
    }
}
