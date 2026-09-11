//! The Environment dialog, for the global layer or one project: plain
//! variables, secrets (stored on Save, never shown back), the opt-in to
//! the project's `.env` files, and a masked preview of what a new session
//! would get.

use egui::{Context, RichText, Ui};

use super::{DrawCtx, GAP, theme};
use crate::app::{Services, resolve_project_env};
use crate::core::{AppAction, AppCore, EnvVar, ProjectEnv, ProjectId, Resolved, SecretScope};

#[derive(Debug, Clone)]
pub struct RowDraft {
    pub name: String,
    /// The plain value, or the new secret value (blank keeps the stored one).
    pub value: String,
    pub secret: bool,
    /// A secret value exists in the store for this name.
    pub stored: bool,
}

#[derive(Debug, Clone)]
pub struct EnvDraft {
    pub scope: SecretScope,
    title: String,
    pub rows: Vec<RowDraft>,
    /// Secret names removed from the list; deleted from the store on Save.
    removed: Vec<String>,
    pub load_dotenv: bool,
    pub dotenv_files: String,
    preview: Resolved,
    reveal: bool,
}

impl EnvDraft {
    /// The global layer.
    #[must_use]
    pub fn global(core: &AppCore, services: &Services) -> Self {
        let scope = SecretScope::Global;
        let rows = rows(&core.settings().env, scope, services);
        let lookup = |account: &str| services.secrets.get(account).ok().flatten();
        let preview = crate::core::env::resolve(
            &core.settings().env,
            ProjectId::new(),
            &ProjectEnv::default(),
            &[],
            &[],
            &lookup,
        );
        Self {
            scope,
            title: "Environment: every project".into(),
            rows,
            removed: Vec::new(),
            load_dotenv: false,
            dotenv_files: String::new(),
            preview,
            reveal: false,
        }
    }

    /// One project's layer.
    #[must_use]
    pub fn project(core: &AppCore, services: &Services, pid: ProjectId) -> Option<Self> {
        let project = &core.workspace(pid)?.project;
        let scope = SecretScope::Project(pid);
        Some(Self {
            scope,
            title: format!("Environment: {}", project.name),
            rows: rows(&project.env.vars, scope, services),
            removed: Vec::new(),
            load_dotenv: project.env.load_dotenv,
            dotenv_files: project.env.files().join(", "),
            preview: resolve_project_env(core, services, pid),
            reveal: false,
        })
    }

    fn vars(&self) -> Vec<EnvVar> {
        self.rows
            .iter()
            .filter(|r| !r.name.trim().is_empty())
            .map(|r| EnvVar {
                name: r.name.trim().to_owned(),
                value: if r.secret {
                    String::new()
                } else {
                    r.value.clone()
                },
                secret: r.secret,
            })
            .collect()
    }
}

fn rows(vars: &[EnvVar], scope: SecretScope, services: &Services) -> Vec<RowDraft> {
    vars.iter()
        .map(|v| RowDraft {
            name: v.name.clone(),
            value: if v.secret {
                String::new()
            } else {
                v.value.clone()
            },
            secret: v.secret,
            stored: v.secret
                && services
                    .secrets
                    .get(&scope.account(&v.name))
                    .ok()
                    .flatten()
                    .is_some(),
        })
        .collect()
}

pub fn show(cx: &mut DrawCtx<'_>, ctx: &Context) {
    let Some(mut draft) = cx.state.env_dialog.take() else {
        return;
    };
    let mut keep = true;
    let mut save = false;
    egui::Window::new("")
        .id(egui::Id::new("environment"))
        .title_bar(false)
        .collapsible(false)
        .resizable(true)
        .default_width(640.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            let p = theme::palette(ui);
            ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
            ui.label(RichText::new(&draft.title).text_style(theme::brand()));
            ui.add_space(8.0);
            variables(ui, &mut draft);
            if let SecretScope::Project(_) = draft.scope {
                ui.add_space(8.0);
                ui.checkbox(
                    &mut draft.load_dotenv,
                    "Read .env files from the project (opt-in)",
                )
                .on_hover_text("Off by default: a repository can ship any .env");
                ui.horizontal(|ui| {
                    let id = ui.label("Files").id;
                    ui.add_enabled(
                        draft.load_dotenv,
                        egui::TextEdit::singleline(&mut draft.dotenv_files)
                            .hint_text(".env, .env.local")
                            .desired_width(300.0),
                    )
                    .labelled_by(id);
                });
            }
            ui.add_space(8.0);
            preview(ui, &mut draft);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Secrets go to the Keychain on Save; records keep names only.")
                        .small()
                        .color(p.n600),
                );
                if theme::ghost_muted(ui, "Cancel").clicked() {
                    keep = false;
                }
                if theme::primary(ui, "Save").clicked() {
                    save = true;
                    keep = false;
                }
            });
        });
    if save {
        commit(cx, &draft);
    }
    if keep {
        cx.state.env_dialog = Some(draft);
    }
}

fn variables(ui: &mut Ui, draft: &mut EnvDraft) {
    let mut remove = None;
    egui::Grid::new("env_rows")
        .num_columns(4)
        .spacing([GAP, GAP])
        .show(ui, |ui| {
            let p = theme::palette(ui);
            theme::kicker(ui, "Name", p.n600);
            theme::kicker(ui, "Value", p.n600);
            theme::kicker(ui, "Secret", p.n600);
            ui.end_row();
            for (i, row) in draft.rows.iter_mut().enumerate() {
                let name_id = ui.label(format!("Name {}", i + 1)).id;
                ui.add(egui::TextEdit::singleline(&mut row.name).desired_width(160.0))
                    .labelled_by(name_id);
                let hint = match (row.secret, row.stored) {
                    (true, true) => "stored; type to replace",
                    (true, false) => "value to store",
                    (false, _) => "",
                };
                let value_id = ui.label(format!("Value {}", i + 1)).id;
                ui.add(
                    egui::TextEdit::singleline(&mut row.value)
                        .password(row.secret)
                        .hint_text(hint)
                        .desired_width(220.0),
                )
                .labelled_by(value_id);
                ui.checkbox(&mut row.secret, format!("Secret {}", i + 1));
                if theme::ghost_muted(ui, "Remove").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = remove {
        let row = draft.rows.remove(i);
        if row.stored {
            draft.removed.push(row.name.trim().to_owned());
        }
    }
    if theme::ghost(ui, "Add variable").clicked() {
        draft.rows.push(RowDraft {
            name: String::new(),
            value: String::new(),
            secret: false,
            stored: false,
        });
    }
}

fn preview(ui: &mut Ui, draft: &mut EnvDraft) {
    let p = theme::palette(ui);
    ui.horizontal(|ui| {
        theme::kicker(ui, "New sessions get", p.n600);
        ui.label(theme::meta_text(
            ui,
            "(as saved; reopen after Save to refresh)",
        ));
        let label = if draft.reveal { "Hide" } else { "Reveal" };
        if theme::ghost_muted(ui, label).clicked() {
            draft.reveal = !draft.reveal;
        }
    });
    if draft.preview.vars.is_empty() {
        ui.label(theme::meta_text(ui, "nothing yet"));
    }
    for var in &draft.preview.vars {
        ui.horizontal(|ui| {
            ui.monospace(&var.name);
            match &var.value {
                None => {
                    ui.label(RichText::new("no value stored").color(p.accent_2_text));
                }
                Some(v) if var.secret && !draft.reveal => {
                    ui.monospace("••••••••");
                }
                Some(v) => {
                    ui.monospace(v);
                }
            }
            ui.label(theme::meta_text(ui, var.source.label()));
        });
    }
    if !draft.preview.example_missing.is_empty() {
        ui.label(
            RichText::new(format!(
                ".env.example also asks for: {}",
                draft.preview.example_missing.join(", ")
            ))
            .color(p.accent_2_text),
        );
    }
}

fn commit(cx: &mut DrawCtx<'_>, draft: &EnvDraft) {
    let vars = draft.vars();
    match draft.scope {
        SecretScope::Global => cx.dispatch(AppAction::SetGlobalEnv(vars)),
        SecretScope::Project(pid) => {
            let files: Vec<String> = draft
                .dotenv_files
                .split(',')
                .map(str::trim)
                .filter(|f| !f.is_empty())
                .map(str::to_owned)
                .collect();
            cx.dispatch(AppAction::SetProjectEnv(
                pid,
                ProjectEnv {
                    vars,
                    load_dotenv: draft.load_dotenv,
                    dotenv_files: files,
                },
            ));
        }
    }
    for row in &draft.rows {
        let name = row.name.trim().to_owned();
        if name.is_empty() {
            continue;
        }
        if row.secret && !row.value.is_empty() {
            cx.dispatch(AppAction::StoreSecret {
                scope: draft.scope,
                name,
                value: row.value.clone(),
            });
        } else if !row.secret && row.stored {
            cx.dispatch(AppAction::DeleteSecret {
                scope: draft.scope,
                name,
            });
        }
    }
    for name in &draft.removed {
        cx.dispatch(AppAction::DeleteSecret {
            scope: draft.scope,
            name: name.clone(),
        });
    }
}
