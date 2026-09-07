//! The session view: header and notes for one record, and either an
//! embedded terminal (shells, commands, services) or, for agents, the
//! conversation read from the transcript with a message box docked at
//! the bottom. Agents run in Ghostty; the raw screen snapshot is kept
//! in a "Terminal" fold under the conversation.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

use std::time::SystemTime;

use egui::{Color32, CornerRadius, Frame, Margin, RichText, Stroke, Ui};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use egui_term::{BackendSettings, PtyEvent, TerminalBackend, TerminalView};

use super::cards::{is_running, kind_label, state_color};
use super::{DrawCtx, GAP, PAD, UiState};
use crate::core::{AppAction, RecordId, SessionKind, SessionRecord};
use crate::ports::host::HostId;
use crate::ports::transcript::{Activity, ActivityKind, Conversation, ToolDetail, Turn};

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
    match record.kind {
        SessionKind::Agent(_) => agent_body(cx, ui, &record),
        SessionKind::Shell | SessionKind::Command | SessionKind::Service => {
            terminal_body(cx, ui, &record);
        }
    }
}

/// `Margin` takes whole pixels as `i8`; these are `PAD` and `GAP` in that
/// form.
const PAD_PX: i8 = 12;
const GAP_PX: i8 = 8;

/// The 1 px line that bounds every region, from the theme so it reads
/// in light and dark.
fn border(ui: &Ui) -> Stroke {
    Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color)
}

/// A fill that hints at `accent` on top of the panel color: a few
/// percent in light mode, more in dark where the panel is near black.
fn tint(ui: &Ui, accent: Color32) -> Color32 {
    let t = if ui.visuals().dark_mode { 0.18 } else { 0.09 };
    ui.visuals().panel_fill.lerp_to_gamma(accent, t)
}

fn header(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    let state = cx.core.card_state(record.id);
    let running = is_running(cx.core, record.id);
    Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(border(ui))
        .corner_radius(4)
        .inner_margin(PAD)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                name_or_editor(cx, ui, record);
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
                    if !matches!(record.kind, SessionKind::Agent(_))
                        && ui
                            .button("Restart")
                            .on_hover_text("Stop it if it runs, then start it again")
                            .clicked()
                    {
                        cx.dispatch(AppAction::RestartSession(record.id));
                    }
                    if record.kind == SessionKind::Service {
                        let mut autostart = record.autostart;
                        if ui
                            .checkbox(&mut autostart, "Autostart")
                            .on_hover_text("Start with Switchboard when its pane is gone")
                            .changed()
                        {
                            cx.dispatch(AppAction::SetAutostart(record.id, autostart));
                        }
                    }
                    let open = if running {
                        "Open in terminal"
                    } else {
                        "Return"
                    };
                    if ui.button(open).clicked() {
                        cx.dispatch(AppAction::ReturnToSession(record.id));
                    }
                    if ui
                        .selectable_label(cx.state.files_open, "Files")
                        .on_hover_text("Show the project's files beside the session (Cmd+B)")
                        .clicked()
                    {
                        cx.state.files_open = !cx.state.files_open;
                    }
                });
            });
        });
}

/// The session name as a heading with a Rename button, or, while a
/// rename is under way, a text field: Enter commits, Esc cancels.
fn name_or_editor(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    let editing = cx
        .state
        .rename_draft
        .as_ref()
        .is_some_and(|(id, _)| *id == record.id);
    if !editing {
        ui.heading(&record.name);
        if ui.small_button("Rename").clicked() {
            cx.state.rename_draft = Some((record.id, record.name.clone()));
        }
        return;
    }
    let mut done = None;
    if let Some((_, draft)) = cx.state.rename_draft.as_mut() {
        let label = ui.label("Session name").id;
        let response = ui
            .add(egui::TextEdit::singleline(draft).desired_width(220.0))
            .labelled_by(label);
        response.request_focus();
        let (enter, escape) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if enter {
            done = Some(Some(draft.trim().to_owned()));
        } else if escape {
            done = Some(None);
        }
    }
    match done {
        Some(Some(name)) => {
            cx.state.rename_draft = None;
            if !name.is_empty() && name != record.name {
                cx.dispatch(AppAction::RenameSession(record.id, name));
            }
        }
        Some(None) => cx.state.rename_draft = None,
        None => {}
    }
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
        Frame::new()
            .stroke(border(ui))
            .corner_radius(4)
            .inner_margin(Margin::symmetric(PAD_PX, GAP_PX))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
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
            });
    }
    if let Some(text) = changed {
        cx.dispatch(AppAction::SetSessionNotes(record.id, text));
    }
}

/// Chat layout: the message box is a panel docked at the bottom, drawn
/// first so it claims its space; the conversation fills what is left.
fn agent_body(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    let fill = ui.visuals().widgets.inactive.weak_bg_fill;
    egui::Panel::bottom("message_panel")
        .resizable(false)
        .frame(Frame::new().fill(fill).inner_margin(PAD))
        .show(ui, |ui| message_box(cx, ui, record));
    Frame::new()
        .stroke(border(ui))
        .corner_radius(4)
        .inner_margin(GAP)
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            // Drawing needs several fields of the UI state at once; taking
            // them apart borrows each on its own, which the borrow checker
            // allows where `cx.state.x` next to `cx.state.y` would not.
            let UiState {
                conversations,
                conversation_errors,
                expand_activity,
                expand_applied,
                markdown,
                snapshots,
                ..
            } = &mut *cx.state;
            let snapshot = snapshots.get(&record.id);
            if let Some((_, conversation)) = conversations.get(&record.id) {
                conversation_view(
                    ui,
                    record.id,
                    conversation,
                    expand_activity,
                    expand_applied,
                    markdown,
                    snapshot,
                );
            } else {
                ui.label(
                    "This session runs in Ghostty. Use Open in terminal to bring its window up.",
                );
                if let Some(e) = conversation_errors.get(&record.id) {
                    ui.label(RichText::new(format!("No conversation view: {e}")).weak());
                }
                if let Some(text) = snapshot {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| code_block(ui, text));
                }
            }
        });
}

/// One-line message box: Enter (or Send) types the text into the
/// session's terminal and presses Enter there, without opening its window.
fn message_box(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    let running = is_running(cx.core, record.id);
    if cx
        .state
        .input_draft
        .as_ref()
        .is_none_or(|(id, _)| *id != record.id)
    {
        cx.state.input_draft = Some((record.id, String::new()));
    }
    let mut send = false;
    ui.horizontal(|ui| {
        let label = ui.label("Message");
        if let Some((_, draft)) = cx.state.input_draft.as_mut() {
            let response = ui
                .add_enabled(
                    running,
                    egui::TextEdit::singleline(draft)
                        .hint_text("type here and press Enter to send into the session")
                        .desired_width(ui.available_width() - 70.0),
                )
                .labelled_by(label.id);
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                send = true;
                response.request_focus();
            }
        }
        if ui.add_enabled(running, egui::Button::new("Send")).clicked() {
            send = true;
        }
    });
    if let (true, Some((_, draft))) = (send, cx.state.input_draft.as_mut()) {
        let text = std::mem::take(draft);
        if !text.trim().is_empty() {
            cx.dispatch(AppAction::SendInput {
                id: record.id,
                text,
            });
        }
    }
}

/// Header line, then the turns in a scroll area that follows new
/// content, with the raw terminal snapshot folded away at the end.
fn conversation_view(
    ui: &mut Ui,
    id: RecordId,
    conversation: &Conversation,
    expand: &mut bool,
    expand_applied: &mut Option<bool>,
    markdown: &mut CommonMarkCache,
    snapshot: Option<&String>,
) {
    ui.horizontal(|ui| {
        ui.strong(conversation.title.as_deref().unwrap_or("(untitled)"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.checkbox(expand, "Expand activity");
        });
    });
    ui.label(RichText::new(meta_line(conversation)).weak().small());
    ui.separator();
    // Push the toggle into every section only on the frame it changes,
    // so single sections can still be opened and closed by hand.
    let open = (*expand_applied != Some(*expand)).then_some(*expand);
    *expand_applied = Some(*expand);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            for turn in &conversation.turns {
                turn_block(ui, turn, open, markdown);
            }
            egui::CollapsingHeader::new("Terminal")
                .id_salt(("terminal", id))
                .show(ui, |ui| match snapshot {
                    Some(text) => code_block(ui, text),
                    None => {
                        ui.label(RichText::new("No snapshot yet.").weak());
                    }
                });
        });
}

fn meta_line(c: &Conversation) -> String {
    let mut parts = vec![
        format!(
            "{} → {} ({})",
            time_text(c.start),
            time_text(c.end),
            duration_text(c.start, c.end)
        ),
        format!("{} turns", c.turns.len()),
    ];
    parts.extend(c.model.clone());
    parts.extend(c.version.as_ref().map(|v| format!("v{v}")));
    parts.extend(c.branch.clone());
    parts.push(format!("out {:.1}k tok", f64_from(c.usage.output) / 1000.0));
    parts.push(format!(
        "cache read {:.1}M",
        f64_from(c.usage.cache_read) / 1e6
    ));
    parts.join(" · ")
}

/// Precision loss is fine for a display figure.
#[allow(clippy::cast_precision_loss)]
fn f64_from(n: u64) -> f64 {
    n as f64
}

fn time_text(t: Option<SystemTime>) -> String {
    t.map_or_else(String::new, |t| {
        chrono::DateTime::<chrono::Local>::from(t)
            .format("%Y-%m-%d %H:%M")
            .to_string()
    })
}

/// "5m" or "1.5h" between two times; empty when either is missing.
fn duration_text(a: Option<SystemTime>, b: Option<SystemTime>) -> String {
    let (Some(a), Some(b)) = (a, b) else {
        return String::new();
    };
    let secs = b.duration_since(a).map_or(0, |d| d.as_secs());
    let mins = secs.div_ceil(60).max(u64::from(secs > 30));
    if mins < 60 {
        format!("{mins}m")
    } else {
        format!("{:.1}h", f64_from(mins) / 60.0)
    }
}

/// One turn: the prompt, the folded activity list, the final answer.
fn turn_block(ui: &mut Ui, turn: &Turn, open: Option<bool>, markdown: &mut CommonMarkCache) {
    let user_fill = tint(ui, Color32::from_rgb(80, 110, 230));
    let final_fill = tint(ui, Color32::from_rgb(60, 170, 80));
    let activity_fill = ui.visuals().faint_bg_color;
    let stroke = border(ui);
    Frame::new().stroke(stroke).corner_radius(6).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        let user_rect = Frame::new()
            .fill(user_fill)
            .corner_radius(CornerRadius {
                nw: 6,
                ne: 6,
                sw: 0,
                se: 0,
            })
            .inner_margin(PAD)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP / 2.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("#{}", turn.n)).weak().small());
                    ui.label(RichText::new(time_text(turn.at)).weak().small());
                    if let Some(mode) = &turn.permission_mode {
                        ui.label(RichText::new(mode).weak().small());
                    }
                });
                ui.add(egui::Label::new(RichText::new(&turn.user).monospace()).wrap());
            })
            .response
            .rect;
        message_menu(ui, user_rect, ("user", turn.n), &turn.user);
        Frame::new()
            .fill(activity_fill)
            .stroke(stroke)
            .inner_margin(Margin::symmetric(PAD_PX, GAP_PX / 2))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP / 2.0);
                egui::CollapsingHeader::new(RichText::new(stats_line(turn)).weak().small())
                    .id_salt(("activity", turn.n))
                    .open(open)
                    .show(ui, |ui| {
                        if turn.activity.is_empty() {
                            ui.label(RichText::new("no tool activity").weak().italics());
                        }
                        for (i, a) in turn.activity.iter().enumerate() {
                            activity_row(ui, a, (turn.n, i));
                        }
                    });
            });
        let final_rect = Frame::new()
            .fill(final_fill)
            .corner_radius(CornerRadius {
                nw: 0,
                ne: 0,
                sw: 6,
                se: 6,
            })
            .inner_margin(PAD)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP / 2.0);
                if turn.final_text.is_empty() {
                    ui.label(
                        RichText::new("no final response (interrupted or tool-only)")
                            .weak()
                            .italics(),
                    );
                } else {
                    CommonMarkViewer::new().show(ui, markdown, &turn.final_text);
                }
            })
            .response
            .rect;
        if !turn.final_text.is_empty() {
            message_menu(ui, final_rect, ("final", turn.n), &turn.final_text);
        }
    });
}

/// The right-click menu of one message (the user's prompt or the
/// agent's answer) covering `rect`. Copy for now; more to come.
///
/// The block is not made clickable: that would put it above the labels
/// and links inside it in egui's hit test and take their clicks. The
/// pointer is checked directly instead, and the menu is opened by hand.
fn message_menu(ui: &mut Ui, rect: egui::Rect, salt: (&str, usize), text: &str) {
    let response = ui.interact(rect, ui.id().with(salt), egui::Sense::hover());
    let right_clicked = ui.input(|i| {
        i.pointer.button_clicked(egui::PointerButton::Secondary)
            && i.pointer.interact_pos().is_some_and(|p| rect.contains(p))
    });
    egui::Popup::menu(&response)
        .open_memory(right_clicked.then_some(egui::SetOpenCommand::Bool(true)))
        .at_pointer_fixed()
        .show(|ui| {
            if ui.button("Copy").clicked() {
                ui.ctx().copy_text(text.to_owned());
                ui.close();
            }
        });
}

fn stats_line(t: &Turn) -> String {
    let mut parts = vec![
        format!("{} msgs", t.assistant_msgs),
        format!("{} tools", t.tools),
    ];
    if t.thinking > 0 {
        parts.push(format!("{} thinking", t.thinking));
    }
    if t.errors > 0 {
        parts.push(format!("{} errors", t.errors));
    }
    let d = duration_text(t.at, t.end);
    parts.push(if d.is_empty() { "0m".into() } else { d });
    parts.join(" · ")
}

/// A tool row folds open to its input and result; text and system rows
/// are single lines.
fn activity_row(ui: &mut Ui, a: &Activity, salt: (usize, usize)) {
    let (glyph, color) = match (a.error, a.kind) {
        (true, _) => ("⚙", ui.visuals().error_fg_color),
        (false, ActivityKind::Tool) => ("⚙", ui.visuals().text_color()),
        (false, ActivityKind::Text) => ("…", ui.visuals().weak_text_color()),
        (false, ActivityKind::System) => ("·", ui.visuals().weak_text_color()),
    };
    let glyph_color = if a.error {
        color
    } else {
        ui.visuals().hyperlink_color
    };
    let mut line = RichText::new(&a.line).color(color);
    line = match a.kind {
        ActivityKind::Text => line.italics(),
        ActivityKind::Tool | ActivityKind::System => line.monospace(),
    };
    match &a.detail {
        Some(detail) => {
            egui::CollapsingHeader::new(
                RichText::new(format!("{glyph} {}", a.line))
                    .monospace()
                    .color(color),
            )
            .id_salt(("tool", salt))
            .show(ui, |ui| tool_detail(ui, detail, salt));
        }
        None => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(glyph).color(glyph_color));
                ui.add(egui::Label::new(line).truncate());
            });
        }
    }
}

fn tool_detail(ui: &mut Ui, detail: &ToolDetail, salt: (usize, usize)) {
    for (label, text) in [("input", &detail.input), ("result", &detail.result)] {
        ui.label(RichText::new(label).weak().small());
        if text.is_empty() {
            ui.label(RichText::new("(empty)").weak().italics());
            continue;
        }
        egui::ScrollArea::both()
            .id_salt((label, salt))
            .max_height(200.0)
            .show(ui, |ui| code_block(ui, text));
    }
}

/// Read-only monospace text, selectable.
fn code_block(ui: &mut Ui, text: &str) {
    ui.add(
        egui::TextEdit::multiline(&mut { text })
            .code_editor()
            .desired_width(f32::INFINITY),
    );
}

fn terminal_body(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    if !is_running(cx.core, record.id) {
        let note = if cx.state.snapshots.contains_key(&record.id) {
            "Not running. Return starts it again; below is the last output kept on disk."
        } else {
            "Not running. Return starts it again."
        };
        ui.label(RichText::new(note).weak());
        if let Some(text) = cx.state.snapshots.get(&record.id) {
            egui::ScrollArea::vertical().show(ui, |ui| code_block(ui, text));
        }
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
