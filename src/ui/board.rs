//! One project's board: title, root, run bar, then its agent and shell
//! cards in a grid with a "+ New session" cell last, its commands and
//! services in a second grid, and pinned documents.

use egui::{RichText, Ui};

use super::cards::{
    ENTRY_CARD_HEIGHT, SESSION_CARD_HEIGHT, document_card, grid, new_session_cell, session_card,
};
use super::dialogs::NewSessionDraft;
use super::{DrawCtx, runbar, theme};
use crate::core::{AppAction, ProjectId, WorkflowRun};

/// Branch and change count per repository the project holds, from the
/// file side's git state (refreshed there).
fn git_line(cx: &mut DrawCtx<'_>, ui: &mut Ui, pid: ProjectId) {
    let p = theme::palette(ui);
    let Some(git) = cx.state.files.get(&pid).and_then(|f| f.git.clone()) else {
        return;
    };
    for repo in &git.repos {
        let name = if repo.rel.as_os_str().is_empty() {
            String::new()
        } else {
            format!("{} ", repo.rel.display())
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(theme::meta_text(ui, format!("{name}on")));
            ui.label(
                RichText::new(&repo.branch)
                    .text_style(theme::meta())
                    .color(p.text),
            )
            .on_hover_text("git branch");
            if repo.changed > 0 {
                ui.label(
                    RichText::new(format!("· {} changed", repo.changed))
                        .text_style(theme::meta())
                        .color(p.accent_2_text),
                );
            }
        });
    }
}

/// The project's plan reviews, newest first, each a link to its page.
fn reviews(cx: &mut DrawCtx<'_>, ui: &mut Ui, runs: &[WorkflowRun]) {
    if runs.is_empty() {
        return;
    }
    let p = theme::palette(ui);
    theme::section(ui, "Plan reviews");
    for run in runs.iter().rev() {
        let name = run
            .plan
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("plan");
        ui.horizontal(|ui| {
            if theme::ghost(ui, &format!("Review: {name}")).clicked() {
                cx.dispatch(AppAction::ShowWorkflow(run.id));
            }
            ui.label(
                RichText::new(run.state.label())
                    .text_style(theme::meta())
                    .color(p.n700),
            );
        });
    }
}

pub fn show(cx: &mut DrawCtx<'_>, ui: &mut Ui, pid: ProjectId) {
    // `cx.core` is a shared reference with the frame's lifetime, so a
    // copy of it can be borrowed from while `cx` itself is lent out
    // mutably to the cards below; cloning the workspace is not needed.
    let core = cx.core;
    let Some(workspace) = core.workspace(pid) else {
        ui.label("This project no longer exists.");
        return;
    };
    let p = theme::palette(ui);
    ui.spacing_mut().item_spacing = egui::vec2(10.0, 6.0);

    egui::ScrollArea::vertical()
        .id_salt(("board", pid))
        .auto_shrink(false)
        .show(ui, |ui| {
            // The buttons claim the right end first; the title gets what
            // is left and is cut rather than run under them.
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::primary(ui, "New session").clicked() {
                        cx.state.new_session = Some(NewSessionDraft::new(&workspace.project));
                    }
                    if theme::secondary(ui, "Environment")
                        .on_hover_text("Variables and secrets new sessions get")
                        .clicked()
                    {
                        cx.state.env_dialog =
                            super::env::EnvDraft::project(cx.core, cx.services, pid);
                    }
                    if theme::secondary(ui, "Config")
                        .on_hover_text(
                            "Edit .switchboard/project.json: commands, services, shown folders",
                        )
                        .clicked()
                    {
                        cx.state.config_dialog = super::config::ConfigDraft::open(cx, pid);
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&workspace.project.name).text_style(theme::h1()),
                            )
                            .truncate(),
                        );
                    });
                });
            });
            ui.horizontal(|ui| {
                ui.label(theme::mono_text(
                    ui,
                    workspace.project.root.display().to_string(),
                ));
                git_line(cx, ui, pid);
            });
            if !workspace.project.notes.is_empty() {
                ui.label(
                    RichText::new(&workspace.project.notes)
                        .text_style(theme::meta())
                        .color(p.n700),
                );
            }
            ui.add_space(6.0);
            runbar::show(cx, ui, pid, false);

            let sessions = core.board_sessions(pid);
            theme::section(ui, "Agents and shells");
            let mut new_session = false;
            grid(ui, sessions.len() + 1, SESSION_CARD_HEIGHT, |ui, i| {
                if let Some(record) = sessions.get(i) {
                    session_card(cx, ui, record);
                } else {
                    new_session = new_session_cell(ui);
                }
            });
            if new_session {
                cx.state.new_session = Some(NewSessionDraft::new(&workspace.project));
            }

            let entries = core.run_entries(pid);
            if !entries.is_empty() {
                theme::section(ui, "Commands and services");
                grid(ui, entries.len(), ENTRY_CARD_HEIGHT, |ui, i| {
                    session_card(cx, ui, entries[i]);
                });
            }
            reviews(cx, ui, &workspace.workflows);
            if !workspace.project.pinned.is_empty() {
                theme::section(ui, "Pinned");
                let pinned = &workspace.project.pinned;
                grid(ui, pinned.len(), 110.0, |ui, i| {
                    let rel = &pinned[i];
                    let path = workspace.project.root.join(rel);
                    document_card(cx, ui, pid, rel, &path);
                });
            }
        });
}
