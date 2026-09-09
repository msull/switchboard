//! The Run tab of the side panel: a project's commands and services with
//! their definitions, approval state, and last output. Entries defined
//! in `.switchboard/project.json` show what the file asks for, so the
//! user sees exactly what an approval would allow.

use std::time::{Duration, Instant};

use egui::{RichText, Ui};

use super::cards::{is_running, state_color};
use super::session::code_block;
use super::{DrawCtx, GAP};
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
        ui.label(
            RichText::new("No commands or services yet. Add one with New session, or let an agent write .switchboard/project.json.")
                .weak(),
        );
        return;
    }
    let defined = ensure_env(cx, pid);
    egui::ScrollArea::vertical()
        .id_salt(("run", pid))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for record in entries {
                egui::Frame::group(ui.style())
                    .inner_margin(GAP)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        row(cx, ui, record);
                        definition(cx, ui, record, &defined);
                        output(cx, ui, record);
                    });
            }
        });
}

/// What the definition file said, or that there is none.
fn status_lines(cx: &mut DrawCtx<'_>, ui: &mut Ui, pid: ProjectId) {
    match cx.core.config_status(pid) {
        Some(status) if status.present => {
            ui.label(RichText::new(".switchboard/project.json").weak().small());
            if let Some(e) = &status.error {
                ui.label(RichText::new(e).color(ui.visuals().error_fg_color).small());
            }
            for w in &status.warnings {
                ui.label(RichText::new(w).color(ui.visuals().warn_fg_color).small());
            }
        }
        _ => {
            ui.label(
                RichText::new(
                    "No .switchboard/project.json. An agent can write one; the README has the format.",
                )
                .weak()
                .small(),
            );
        }
    }
    ui.separator();
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
    let state = cx.core.card_state(record.id);
    let running = is_running(cx.core, record.id);
    let runnable = record.runnable();
    ui.horizontal(|ui| {
        let glyph = if record.kind == SessionKind::Service {
            "●"
        } else {
            "▶"
        };
        ui.label(RichText::new(glyph).color(state_color(ui, &state)));
        ui.strong(&record.name);
        ui.label(RichText::new(cx.core.state_text(record.id)).color(state_color(ui, &state)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("Show").clicked() {
                cx.dispatch(AppAction::ShowSession(record.id));
            }
            if record.kind == SessionKind::Service {
                if running {
                    if ui.small_button("Stop").clicked() {
                        cx.dispatch(AppAction::KillSession(record.id));
                    }
                } else if ui
                    .add_enabled(runnable, egui::Button::new("Start").small())
                    .clicked()
                {
                    cx.dispatch(AppAction::RestartSession(record.id));
                }
            } else if ui
                .add_enabled(runnable && !running, egui::Button::new("Run now").small())
                .on_hover_text("Run it again; the last output is kept until then")
                .clicked()
            {
                cx.dispatch(AppAction::RestartSession(record.id));
            }
            if !running
                && record.approval() == Approval::Orphaned
                && ui.small_button("Remove").clicked()
            {
                cx.dispatch(AppAction::RemoveSession(record.id));
            }
        });
    });
    if let Some(caption) = cx.state.captions.get(&record.id) {
        ui.label(RichText::new(caption).weak().small());
    }
}

/// The command line, directory, and variables, plus where the approval
/// stands for a defined entry.
fn definition(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord, defined: &[String]) {
    if let Launch::Command { command, .. } = &record.launch {
        ui.label(RichText::new(command).monospace());
    }
    ui.label(
        RichText::new(record.cwd.display().to_string())
            .weak()
            .small(),
    );
    let Some(source) = &record.source else {
        return;
    };
    if !source.env.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("env:").weak().small());
            for name in &source.env {
                if defined.contains(name) {
                    ui.label(RichText::new(name).monospace().small());
                } else {
                    ui.label(
                        RichText::new(format!("{name} (not defined)"))
                            .monospace()
                            .small()
                            .color(ui.visuals().warn_fg_color),
                    )
                    .on_hover_text("The definition asks for this variable, but the project's environment does not set it");
                }
            }
        });
    }
    if source.autostart {
        ui.label(
            RichText::new("autostart requested by the file")
                .weak()
                .small(),
        );
    }
    ui.horizontal(|ui| match record.approval() {
        Approval::NotApplicable => {}
        Approval::Pending => {
            ui.label(RichText::new("needs approval").color(ui.visuals().warn_fg_color));
            if ui
                .button("Approve")
                .on_hover_text("Allow exactly this command to run from this project")
                .clicked()
            {
                cx.dispatch(AppAction::ApproveDefinition(record.id));
            }
        }
        Approval::Approved => {
            ui.label(RichText::new("approved").weak());
            if ui.small_button("Revoke").clicked() {
                cx.dispatch(AppAction::RevokeApproval(record.id));
            }
        }
        Approval::Changed => {
            ui.label(
                RichText::new("definition changed since approval")
                    .color(ui.visuals().warn_fg_color),
            );
            if ui.button("Approve").clicked() {
                cx.dispatch(AppAction::ApproveDefinition(record.id));
            }
        }
        Approval::Orphaned => {
            ui.label(RichText::new("no longer in project.json").weak());
        }
    });
}

/// The pane's last lines, live or from disk.
fn output(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    let Some(text) = cx.state.snapshots.get(&record.id) else {
        return;
    };
    let row = ui.text_style_height(&egui::TextStyle::Monospace);
    egui::CollapsingHeader::new("Output")
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
