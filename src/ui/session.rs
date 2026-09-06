//! The session view: header and notes for one record, and either an
//! embedded terminal (shells, commands, services) or a note that the
//! session lives in Ghostty plus its last snapshot (agents).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

use egui::{RichText, Ui};
use egui_term::{BackendSettings, PtyEvent, TerminalBackend, TerminalView};

use super::cards::{is_running, kind_label, state_color};
use super::{DrawCtx, GAP};
use crate::core::{AppAction, RecordId, SessionKind, SessionRecord};
use crate::ports::host::HostId;

/// An `egui_term` backend attached to one host session, plus the channel
/// that tells us when its pty closed.
pub struct EmbeddedTerminal {
    backend: TerminalBackend,
    events: Receiver<(u64, PtyEvent)>,
    /// The egui id of the terminal widget after its first frame, so focus
    /// can be handed to it only when nothing else has it.
    widget_id: Option<egui::Id>,
    detached: bool,
}

/// The user's login shell, which runs the attach command and user
/// commands so their PATH and aliases apply.
#[must_use]
pub fn login_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into())
}

fn shell_quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "'\\''"))
}

impl EmbeddedTerminal {
    /// Spawn `attach` (an argv) through the login shell in `cwd`.
    fn attach(
        id: u64,
        ctx: egui::Context,
        attach: &[String],
        cwd: PathBuf,
    ) -> anyhow::Result<Self> {
        let command = attach
            .iter()
            .map(|a| shell_quote(a))
            .collect::<Vec<_>>()
            .join(" ");
        let env: HashMap<String, String> = [
            ("TERM".to_string(), "xterm-256color".to_string()),
            ("COLORTERM".to_string(), "truecolor".to_string()),
        ]
        .into_iter()
        .collect();
        let (tx, rx) = channel();
        let backend = TerminalBackend::new(
            id,
            ctx,
            tx,
            BackendSettings {
                shell: login_shell(),
                args: vec!["-lc".into(), format!("exec {command}")],
                working_directory: Some(cwd),
                env,
            },
        )?;
        Ok(Self {
            backend,
            events: rx,
            widget_id: None,
            detached: false,
        })
    }

    fn poll(&mut self) {
        while let Ok((_, event)) = self.events.try_recv() {
            if matches!(event, PtyEvent::Exit) {
                self.detached = true;
            }
        }
    }
}

pub fn show(cx: &mut DrawCtx<'_>, ui: &mut Ui, id: RecordId) {
    let Some(record) = cx.core.session(id).cloned() else {
        ui.label("This session no longer exists.");
        return;
    };
    ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
    header(cx, ui, &record);
    notes(cx, ui, &record);
    ui.separator();
    match record.kind {
        SessionKind::Agent(_) => agent_body(cx, ui, &record),
        SessionKind::Shell | SessionKind::Command | SessionKind::Service => {
            terminal_body(cx, ui, &record);
        }
    }
}

fn header(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    let state = cx.core.card_state(record.id);
    let running = is_running(cx.core, record.id);
    ui.horizontal(|ui| {
        ui.heading(&record.name);
        ui.label(RichText::new(kind_label(record.kind)).weak());
        ui.label(RichText::new(state.label()).color(state_color(ui, &state)));
        ui.label(RichText::new(record.cwd.display().to_string()).weak());
        if let Some(handle) = &record.resume {
            ui.label(
                RichText::new(format!("resume {}", handle.provider_id()))
                    .weak()
                    .small(),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Back").clicked() {
                cx.dispatch(AppAction::Back);
            }
            if ui.button("Kill").clicked() {
                cx.dispatch(AppAction::KillSession(record.id));
            }
            let open = if running {
                "Open in terminal"
            } else {
                "Return"
            };
            if ui.button(open).clicked() {
                cx.dispatch(AppAction::ReturnToSession(record.id));
            }
        });
    });
}

fn notes(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    // The draft follows the record on screen; a different record means a
    // fresh draft from its stored notes.
    let stale = cx
        .state
        .notes_draft
        .as_ref()
        .is_none_or(|(id, _)| *id != record.id);
    if stale {
        cx.state.notes_draft = Some((record.id, record.notes.clone()));
    }
    let mut changed = None;
    if let Some((_, draft)) = cx.state.notes_draft.as_mut() {
        ui.horizontal(|ui| {
            let label = ui.label("Notes").id;
            let response = ui.add(
                egui::TextEdit::multiline(draft)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            );
            if response.changed() {
                changed = Some(draft.clone());
            }
            response.labelled_by(label);
        });
    }
    if let Some(text) = changed {
        cx.dispatch(AppAction::SetSessionNotes(record.id, text));
    }
}

fn agent_body(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    ui.label("This session runs in Ghostty. Use Open in terminal to bring its window up.");
    snapshot(cx, ui, record.id);
}

fn snapshot(cx: &mut DrawCtx<'_>, ui: &mut Ui, id: RecordId) {
    if let Some(text) = cx.state.snapshots.get(&id) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut text.as_str())
                    .code_editor()
                    .desired_width(f32::INFINITY),
            );
        });
    }
}

fn terminal_body(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    if !is_running(cx.core, record.id) {
        ui.label(RichText::new("Not running. Return starts it again.").weak());
        snapshot(cx, ui, record.id);
        return;
    }
    if !cx.state.embed_terminals {
        ui.label(RichText::new("Embedded terminal disabled.").weak());
        return;
    }
    if !cx.state.terminals.contains_key(&record.id) {
        let attach = cx
            .services
            .host
            .attach_command(&HostId(record.id.host_name()));
        cx.state.next_terminal_id += 1;
        let id = cx.state.next_terminal_id;
        match EmbeddedTerminal::attach(id, ui.ctx().clone(), &attach, record.cwd.clone()) {
            Ok(term) => {
                cx.state.terminals.insert(record.id, term);
            }
            Err(e) => {
                ui.label(
                    RichText::new(format!("Could not open a terminal: {e}"))
                        .color(ui.visuals().error_fg_color),
                );
                return;
            }
        }
    }
    let focused = ui.ctx().memory(egui::Memory::focused);
    let Some(term) = cx.state.terminals.get_mut(&record.id) else {
        return;
    };
    term.poll();
    if term.detached {
        ui.label(RichText::new("Terminal detached.").weak());
        if ui.button("Reconnect").clicked() {
            cx.state.terminals.remove(&record.id);
        }
        return;
    }
    // Take focus when nothing else has it (so typing just works) and keep
    // it once we have it, but never steal it from the notes field.
    let want_focus = match (focused, term.widget_id) {
        (None, _) => true,
        (Some(f), Some(w)) => f == w,
        (Some(_), None) => false,
    };
    let size = ui.available_size();
    let view = TerminalView::new(ui, &mut term.backend)
        .set_focus(want_focus)
        .set_size(size);
    let response = ui.add(view);
    term.widget_id = Some(response.id);
}
