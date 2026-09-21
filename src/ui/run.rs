//! The Run tab of the side panel: a project's commands and services with
//! their definitions, approval state, and last output. Entries defined
//! in `.switchboard/project.json` show what the file asks for, so the
//! user sees exactly what an approval would allow.

use std::time::{Duration, Instant};

use egui::{RichText, Ui};

use super::cards::{is_running, kind_label};
use super::session::code_block;
use super::{DrawCtx, theme};
use crate::app::resolve_project_env;
use crate::core::{AppAction, Approval, Launch, ProjectId, SessionKind, SessionRecord};

/// How often the project's environment is resolved again for the "not
/// defined" marks; it reads `.env` files, so not every frame.
const ENV_INTERVAL: Duration = Duration::from_secs(5);

/// Rows of output shown under an entry before it scrolls.
const OUTPUT_ROWS: f32 = 12.0;

pub fn show(cx: &mut DrawCtx<'_>, ui: &mut Ui, pid: ProjectId) {
    // Shared for the frame, so copies of it can be read while `cx` is
    // lent out mutably to the rows below.
    let core = cx.core;
    status_lines(cx, ui, pid);
    let entries: Vec<&SessionRecord> = core.run_entries(pid);
    if entries.is_empty() {
        ui.label(theme::meta_text(ui,
            "No commands or services yet. Add one with New session, or let an agent write .switchboard/project.json.",
        ));
        return;
    }
    let defined = ensure_env(cx, pid);
    egui::ScrollArea::vertical()
        .id_salt(("run", pid))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            for record in entries {
                theme::surface(ui)
                    .inner_margin(egui::Margin::symmetric(14, 12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 4.0;
                        row(cx, ui, record);
                        definition(cx, ui, record, &defined);
                        output(cx, ui, record);
                    });
            }
        });
}

/// What the definition file said, or that there is none.
fn status_lines(cx: &mut DrawCtx<'_>, ui: &mut Ui, pid: ProjectId) {
    let p = theme::palette(ui);
    match cx.core.config_status(pid) {
        Some(status) if status.present => {
            ui.label(theme::mono_text(ui, ".switchboard/project.json"));
            if let Some(e) = &status.error {
                ui.label(
                    RichText::new(e)
                        .text_style(theme::meta())
                        .color(p.accent_2_text),
                );
            }
            for w in &status.warnings {
                ui.label(
                    RichText::new(w)
                        .text_style(theme::meta())
                        .color(p.accent_2_text),
                );
            }
        }
        _ => {
            ui.label(theme::meta_text(
                ui,
                "No .switchboard/project.json. An agent can write one; the README has the format.",
            ));
        }
    }
    ui.add_space(4.0);
}

/// Names the project's environment defines right now, refreshed every
/// few seconds while the tab is open.
fn ensure_env(cx: &mut DrawCtx<'_>, pid: ProjectId) -> Vec<String> {
    let fresh = cx
        .state
        .run_env
        .as_ref()
        .is_some_and(|(p, at, _)| *p == pid && at.elapsed() < ENV_INTERVAL);
    if !fresh {
        let names = resolve_project_env(cx.core, cx.services, pid)
            .vars
            .into_iter()
            .map(|v| v.name)
            .collect();
        cx.state.run_env = Some((pid, Instant::now(), names));
    }
    cx.state
        .run_env
        .as_ref()
        .map(|(_, _, names)| names.clone())
        .unwrap_or_default()
}

/// One entry's line: kind glyph, name, state, and its buttons. Shared
/// with the board, which lists commands and services the same way.
pub fn row(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    let p = theme::palette(ui);
    let state = cx.core.card_state(record.id);
    let running = is_running(cx.core, record.id);
    ui.horizontal(|ui| {
        theme::kicker(
            ui,
            &format!(
                "{} · {}",
                kind_label(record.kind),
                super::runs::kicker(record, running, std::time::SystemTime::now())
            ),
            p.state_text(&state),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if record.kind == SessionKind::Service {
                theme::status_dot(ui, &state, 8.0);
            } else {
                ui.label(RichText::new("▶").small().color(p.n600));
            }
        });
    });
    ui.label(RichText::new(&record.name).text_style(theme::card_title()));
    if let Some(caption) = cx.state.captions.get(&record.id) {
        ui.add(egui::Label::new(theme::mono_text(ui, caption).color(p.n800)).truncate());
    }
    ui.horizontal(|ui| super::runs::actions(cx, ui, record, running));
}

/// The command line, directory, and variables, plus where the approval
/// stands for a defined entry.
fn definition(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord, defined: &[String]) {
    let p = theme::palette(ui);
    if let Launch::Command { command, .. } = &record.launch {
        ui.label(RichText::new(command).monospace().color(p.n800));
    }
    ui.label(theme::mono_text(ui, record.cwd.display().to_string()));
    let Some(source) = &record.source else {
        return;
    };
    if !source.env.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(theme::meta_text(ui, "env:"));
            for name in &source.env {
                if defined.contains(name) {
                    ui.label(theme::mono_text(ui, name).color(p.n800));
                } else {
                    ui.label(
                        RichText::new(format!("{name} (not defined)"))
                            .monospace()
                            .color(p.accent_2_text),
                    )
                    .on_hover_text("The definition asks for this variable, but the project's environment does not set it");
                }
            }
        });
    }
    if source.autostart {
        ui.label(theme::meta_text(ui, "autostart requested by the file"));
    }
    ui.horizontal(|ui| match record.approval() {
        Approval::NotApplicable => {}
        Approval::Pending => {
            ui.label(
                RichText::new("needs approval")
                    .text_style(theme::meta())
                    .color(p.accent_2_text),
            );
            if theme::ghost(ui, "Approve")
                .on_hover_text("Allow exactly this command to run from this project")
                .clicked()
            {
                cx.dispatch(AppAction::ApproveDefinition(record.id));
            }
        }
        Approval::Approved => {
            ui.label(theme::meta_text(ui, "approved"));
            if theme::ghost_muted(ui, "Revoke").clicked() {
                cx.dispatch(AppAction::RevokeApproval(record.id));
            }
        }
        Approval::Changed => {
            ui.label(
                RichText::new("definition changed since approval")
                    .text_style(theme::meta())
                    .color(p.accent_2_text),
            );
            if theme::ghost(ui, "Approve").clicked() {
                cx.dispatch(AppAction::ApproveDefinition(record.id));
            }
        }
        Approval::Orphaned => {
            ui.label(theme::meta_text(ui, "no longer in project.json"));
        }
    });
}

/// The pane's last lines, live or from disk.
fn output(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    let Some(text) = cx.state.snapshots.get(&record.id) else {
        return;
    };
    let row = ui.text_style_height(&egui::TextStyle::Monospace);
    egui::CollapsingHeader::new(RichText::new("Output").text_style(theme::meta()))
        .id_salt(("run-output", record.id))
        .default_open(true)
        .show(ui, |ui| {
            egui::ScrollArea::both()
                .id_salt(("run-scroll", record.id))
                .max_height(row * OUTPUT_ROWS)
                .stick_to_bottom(true)
                .show(ui, |ui| code_block(ui, text));
        });
}
