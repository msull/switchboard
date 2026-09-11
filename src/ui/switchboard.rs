//! The view the app is named for: every session across every project,
//! grouped by project, projects and cards with *waiting on you* first.

use egui::{RichText, Ui};

use super::cards::{SESSION_CARD_HEIGHT, card_key, grid, session_card};
use super::{DrawCtx, theme};
use crate::core::{AppAction, CardState, Workspace};

pub fn show(cx: &mut DrawCtx<'_>, ui: &mut Ui) {
    let p = theme::palette(ui);
    ui.spacing_mut().item_spacing = egui::vec2(10.0, 6.0);
    ui.label(RichText::new("All sessions").text_style(theme::h1()));

    let mut workspaces: Vec<Workspace> = cx.core.visible_workspaces().cloned().collect();
    // A project's rank is its most urgent card; ties go to the most
    // recently active project.
    let rank = |w: &Workspace| {
        w.sessions
            .iter()
            .map(|s| cx.core.card_state(s.id).rank())
            .min()
            .unwrap_or(u8::MAX)
    };
    workspaces.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then_with(|| b.project.last_active.cmp(&a.project.last_active))
    });
    let total: usize = workspaces.iter().map(|w| w.sessions.len()).sum();
    let waiting = cx.core.waiting_count();
    let working = workspaces
        .iter()
        .flat_map(|w| w.sessions.iter())
        .filter(|s| {
            matches!(
                cx.core.card_state(s.id),
                CardState::Working | CardState::Starting
            )
        })
        .count();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(theme::meta_text(
            ui,
            format!("{total} across {} projects", workspaces.len()),
        ));
        if waiting > 0 {
            ui.label(theme::meta_text(ui, "·"));
            ui.label(
                RichText::new(format!("{waiting} waiting on you"))
                    .text_style(theme::strong())
                    .color(p.accent_2_text),
            );
        }
        if working > 0 {
            ui.label(theme::meta_text(ui, "·"));
            ui.label(theme::meta_text(ui, format!("{working} working")));
        }
    });

    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            if workspaces.is_empty() {
                ui.label(theme::meta_text(
                    ui,
                    "No projects yet. Use Add project to start.",
                ));
            }
            for workspace in &workspaces {
                ui.add_space(22.0);
                ui.horizontal(|ui| {
                    ui.heading(&workspace.project.name);
                    ui.label(theme::meta_text(
                        ui,
                        format!("{} sessions", workspace.sessions.len()),
                    ));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::ghost(ui, "Open board →").clicked() {
                            cx.dispatch(AppAction::ShowBoard(workspace.project.id));
                        }
                    });
                });
                ui.add_space(4.0);
                // Agents and shells only: commands and services belong to
                // the board, where the run bar starts them.
                let mut sessions = cx.core.board_sessions(workspace.project.id);
                sessions.sort_by_key(|s| card_key(cx.core, s));
                if sessions.is_empty() {
                    ui.label(theme::meta_text(ui, "no sessions"));
                }
                grid(ui, sessions.len(), SESSION_CARD_HEIGHT, |ui, i| {
                    session_card(cx, ui, sessions[i]);
                });
            }
        });
}
