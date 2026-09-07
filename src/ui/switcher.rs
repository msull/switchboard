//! Top bar (project switcher, switchboard button, waiting badge, add
//! project, files toggle, back) and bottom bar (notice, host error,
//! read-only tag).

use egui::{Button, Color32, RichText, Ui};

use super::cards::{project_dot_color, state_color};
use super::dialogs::AddProjectDraft;
use super::{DrawCtx, GAP};
use crate::core::{AppAction, AppCore, CardState, Project, ThemeMode, View};

/// The top bar's toggle for the file side of a session.
pub const FILES_ICON: &str = "📁";

/// Projects most recently active first: the switcher order, also used
/// for Cmd+1..9.
pub fn projects_by_recency(core: &AppCore) -> Vec<&Project> {
    let mut projects: Vec<&Project> = core.visible_workspaces().map(|w| &w.project).collect();
    projects.sort_by_key(|p| std::cmp::Reverse(p.last_active));
    projects
}

pub fn top_bar(cx: &mut DrawCtx<'_>, ui: &mut Ui, view: &View) {
    ui.spacing_mut().item_spacing.x = GAP;
    ui.horizontal(|ui| {
        // The app name doubles as the button for the view the app is
        // named after; Cmd+0 does the same.
        if ui
            .add(Button::new(RichText::new("Switchboard").heading()).frame(false))
            .on_hover_text("Every session across every project (Cmd+0)")
            .clicked()
        {
            cx.dispatch(AppAction::ShowSwitchboard);
        }
        ui.separator();

        let active = match view {
            View::Board(pid) | View::Document(pid, _) => Some(*pid),
            View::Session(id) => cx.core.session(*id).map(|s| s.project),
            View::Switchboard => None,
        };
        let projects: Vec<Project> = projects_by_recency(cx.core).into_iter().cloned().collect();
        for (n, project) in projects.iter().enumerate() {
            dot(ui, project_dot_color(cx.core, project.id));
            let button = Button::new(&project.name).selected(active == Some(project.id));
            let hover = if n < 9 {
                format!("Cmd+{}", n + 1)
            } else {
                project.root.display().to_string()
            };
            if ui.add(button).on_hover_text(hover).clicked() {
                cx.dispatch(AppAction::ShowBoard(project.id));
            }
        }

        if ui.button("Add project").clicked() {
            cx.state.add_project = Some(AddProjectDraft::default());
        }
        if ui
            .button("Go to")
            .on_hover_text("Find a project or session (Cmd+K)")
            .clicked()
        {
            cx.state.palette = Some(super::palette::PaletteDraft::default());
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let settings = cx.core.settings().clone();
            settings_menu(cx, ui, &settings);
            if settings.exclusive {
                ui.label(RichText::new("exclusive").weak())
                    .on_hover_text("Other projects are hidden (Settings)");
            }
            if *view != View::Switchboard && ui.button("Back").clicked() {
                cx.dispatch(AppAction::Back);
            }
            // The glyph comes from egui's emoji font; it reads as a
            // folder in light and dark.
            if matches!(view, View::Session(_))
                && ui
                    .add(
                        Button::new(RichText::new(FILES_ICON).size(18.0))
                            .selected(cx.state.files_open),
                    )
                    .on_hover_text("Show the project's files beside the session (Cmd+B)")
                    .clicked()
            {
                cx.state.files_open = !cx.state.files_open;
            }
            let waiting = cx.core.waiting_count();
            if waiting > 0 {
                ui.label(
                    RichText::new(format!("{waiting} waiting"))
                        .color(state_color(ui, &CardState::WaitingOnYou))
                        .strong(),
                );
            }
        });
    });
}

/// The Settings menu: theme, editor command, environment, exclusive.
fn settings_menu(cx: &mut DrawCtx<'_>, ui: &mut Ui, settings: &crate::core::Settings) {
    ui.menu_button("Settings", |ui| {
        ui.label(RichText::new("Theme").weak());
        for mode in ThemeMode::ALL {
            if ui.radio(mode == settings.theme, mode.label()).clicked() {
                cx.dispatch(AppAction::SetTheme(mode));
                ui.close();
            }
        }
        ui.separator();
        ui.label(RichText::new("Editor command").weak());
        let draft = cx
            .state
            .editor_draft
            .get_or_insert_with(|| settings.editor.clone());
        let field = ui.add(
            egui::TextEdit::singleline(draft)
                .hint_text("code, zed, cursor (blank: system editor)")
                .desired_width(200.0),
        );
        if field.lost_focus() {
            let editor = draft.clone();
            cx.state.editor_draft = None;
            if editor.trim() != settings.editor {
                cx.dispatch(AppAction::SetEditor(editor));
            }
        }
        if ui.button("Environment…").clicked() {
            cx.state.env_dialog = Some(super::env::EnvDraft::global(cx.core, cx.services));
            ui.close();
        }
        ui.separator();
        let mut exclusive = settings.exclusive;
        if ui
            .checkbox(&mut exclusive, "Exclusive: only the active project")
            .on_hover_text("Hides every other project while screen sharing")
            .changed()
        {
            cx.dispatch(AppAction::SetExclusive(exclusive));
            ui.close();
        }
    });
}

pub fn bottom_bar(cx: &mut DrawCtx<'_>, ui: &mut Ui) {
    ui.spacing_mut().item_spacing.x = GAP;
    if let Some(error) = cx.core.host_error() {
        ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
    }
    ui.horizontal(|ui| {
        if let Some(notice) = cx.core.notice() {
            let text = if notice.is_error {
                RichText::new(&notice.text).color(ui.visuals().error_fg_color)
            } else {
                RichText::new(&notice.text)
            };
            ui.label(text);
            if ui.small_button("Dismiss").clicked() {
                cx.dispatch(AppAction::DismissNotice);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if cx.core.read_only() {
                ui.label(RichText::new("read-only").color(ui.visuals().warn_fg_color))
                    .on_hover_text(
                        "Another Switchboard holds the store lock; changes are not saved",
                    );
            }
        });
    });
}

/// A small filled circle, used as a status dot.
pub fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(GAP, GAP), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), GAP / 2.0, color);
}
