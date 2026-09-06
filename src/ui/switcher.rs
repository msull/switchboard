//! Top bar (project switcher, switchboard button, waiting badge, add
//! project, back) and bottom bar (notice, host error, read-only tag).

use egui::{Button, Color32, RichText, Ui};

use super::cards::{project_dot_color, state_color};
use super::dialogs::AddProjectDraft;
use super::{DrawCtx, GAP};
use crate::core::{AppAction, AppCore, CardState, Project, View};

/// Projects most recently active first: the switcher order, also used
/// for Cmd+1..9.
pub fn projects_by_recency(core: &AppCore) -> Vec<&Project> {
    let mut projects: Vec<&Project> = core.workspaces().iter().map(|w| &w.project).collect();
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
            View::Board(pid) => Some(*pid),
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

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if *view != View::Switchboard && ui.button("Back").clicked() {
                cx.dispatch(AppAction::Back);
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
