//! One project's board: its session cards in a wrapping grid, pinned
//! documents, and read-only notes.

use egui::{RichText, Ui};

use super::cards::{card_key, document_card, session_card};
use super::dialogs::NewSessionDraft;
use super::{DrawCtx, GAP, PAD};
use crate::core::ProjectId;

pub fn show(cx: &mut DrawCtx<'_>, ui: &mut Ui, pid: ProjectId) {
    let Some(workspace) = cx.core.workspace(pid).cloned() else {
        ui.label("This project no longer exists.");
        return;
    };
    ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);

    // The project strip is a framed region so the board's own heading is
    // told apart from the switcher above it.
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(4)
        .inner_margin(PAD)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.heading(&workspace.project.name);
                ui.label(RichText::new(workspace.project.root.display().to_string()).weak());
                if ui.button("New session").clicked() {
                    cx.state.new_session = Some(NewSessionDraft::new(&workspace.project));
                }
            });
            if !workspace.project.notes.is_empty() {
                ui.label(&workspace.project.notes);
            }
        });

    let mut sessions: Vec<_> = workspace.sessions.iter().collect();
    sessions.sort_by_key(|s| card_key(cx.core, s));

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.label(RichText::new("Sessions").strong());
        ui.separator();
        if sessions.is_empty() {
            ui.label(RichText::new("No sessions yet. Start one with New session.").weak());
        }
        ui.horizontal_wrapped(|ui| {
            for record in sessions {
                session_card(cx, ui, record);
            }
        });
        if !workspace.project.pinned.is_empty() {
            ui.add_space(GAP);
            ui.label(RichText::new("Pinned").strong());
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                for rel in &workspace.project.pinned {
                    let path = workspace.project.root.join(rel);
                    document_card(cx, ui, pid, rel, &path);
                }
            });
        }
    });
}
