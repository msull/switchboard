//! The view the app is named for: every session across every project,
//! grouped by project, projects and cards with *waiting on you* first.

use egui::{RichText, Ui};

use super::cards::{card_key, session_card};
use super::{DrawCtx, GAP};
use crate::core::Workspace;

pub fn show(cx: &mut DrawCtx<'_>, ui: &mut Ui) {
    ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
    ui.heading("All sessions");

    let mut workspaces: Vec<Workspace> = cx.core.workspaces().to_vec();
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

    egui::ScrollArea::vertical().show(ui, |ui| {
        if workspaces.is_empty() {
            ui.label(RichText::new("No projects yet. Use Add project to start.").weak());
        }
        for workspace in &workspaces {
            ui.label(RichText::new(&workspace.project.name).strong());
            ui.separator();
            let mut sessions: Vec<_> = workspace.sessions.iter().collect();
            sessions.sort_by_key(|s| card_key(cx.core, s));
            if sessions.is_empty() {
                ui.label(RichText::new("no sessions").weak());
            }
            ui.horizontal_wrapped(|ui| {
                for record in sessions {
                    session_card(cx, ui, record);
                }
            });
            ui.add_space(GAP);
        }
    });
}
