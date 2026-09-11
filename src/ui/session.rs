//! The session view: header and notes for one record, and either an
//! embedded terminal (shells, commands, services) or, for agents, the
//! conversation read from the transcript with a message box docked at
//! the bottom. Agents run in Ghostty; the raw screen snapshot is kept
//! in a "Terminal" fold under the conversation.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

use std::time::SystemTime;

use egui::{CornerRadius, Frame, Margin, RichText, Stroke, Ui};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use egui_term::{BackendSettings, PtyEvent, TerminalBackend, TerminalView};

use super::cards::{is_running, kind_label};
use super::files::DraggedPath;
use super::{DrawCtx, GAP, PAD, UiState, theme};
use crate::core::{AppAction, CardState, RecordId, SessionKind, SessionRecord};
use crate::ports::host::HostId;
use crate::ports::transcript::{
    Activity, ActivityKind, Conversation, ToolDetail, Turn, context_window,
};

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

/// `Margin` takes whole pixels as `i8`; this is `GAP` in that form.
const GAP_PX: i8 = 8;

/// Below this width the header's actions get their own row.
const TIGHT_HEADER: f32 = 640.0;

/// The header: the name as a title with the state beside it and the
/// actions at the right, then a meta line (kind, directory, resume
/// handle), then the run bar. No frame; the whitespace is the edge.
///
/// Every row here can shrink: a row that cannot widens everything drawn
/// after it, and the conversation would then run under the side panel
/// instead of wrapping. The title truncates, and when the column is
/// tight the actions move to a wrapped row of their own.
fn header(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    let p = theme::palette(ui);
    let state = cx.core.card_state(record.id);
    let running = is_running(cx.core, record.id);
    if ui.available_width() < TIGHT_HEADER {
        ui.horizontal(|ui| title_row(cx, ui, record, &state));
        ui.horizontal_wrapped(|ui| header_actions(cx, ui, record, running, false));
    } else {
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                header_actions(cx, ui, record, running, true);
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    title_row(cx, ui, record, &state);
                });
            });
        });
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(theme::meta_text(ui, kind_label(record.kind)).color(p.n700));
        ui.label(theme::meta_text(ui, "·"));
        let mut path = record.cwd.display().to_string();
        if let Some(handle) = &record.resume {
            path = format!("{path} · resume {}", handle.provider_id());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.spacing_mut().button_padding = egui::vec2(6.0, 3.0);
            let files_open = cx.core.settings().files_open;
            let side_button = if files_open {
                theme::ghost(ui, "Side")
            } else {
                theme::ghost_muted(ui, "Side")
            };
            if side_button
                .on_hover_text(
                    "Show the project's files and its Run tab beside the session (Cmd+B, Cmd+R)",
                )
                .clicked()
            {
                cx.dispatch(AppAction::SetFilesOpen(!files_open));
            }
            // A defined service's autostart is the file's request,
            // shown on the Run tab; only the user's own records
            // carry the checkbox.
            if record.kind == SessionKind::Service && record.source.is_none() {
                let mut autostart = record.autostart;
                if ui
                    .checkbox(
                        &mut autostart,
                        RichText::new("Autostart").text_style(theme::meta()),
                    )
                    .on_hover_text("Start with Switchboard when its pane is gone")
                    .changed()
                {
                    cx.dispatch(AppAction::SetAutostart(record.id, autostart));
                }
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(theme::mono_text(ui, path)).truncate());
            });
        });
    });
    super::runbar::show(cx, ui, record.project, true);
    ui.add_space(6.0);
}

/// The title, Rename, and the state with its dot. The title truncates to
/// the room the other three leave, measured first: a truncating label in
/// a row would otherwise take the whole width and push them past the
/// edge.
fn title_row(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord, state: &CardState) {
    let p = theme::palette(ui);
    let state_text = cx.core.state_text(record.id);
    let meta_font = theme::meta().resolve(ui.style());
    let measure = |ui: &Ui, text: &str, font: egui::FontId| {
        ui.painter()
            .layout_no_wrap(text.to_owned(), font, p.text)
            .size()
            .x
    };
    let button_font = egui::TextStyle::Button.resolve(ui.style());
    let reserve = measure(ui, "Rename", button_font)
        + 2.0 * ui.spacing().button_padding.x
        + 8.0
        + measure(ui, &state_text, meta_font)
        + 4.0 * ui.spacing().item_spacing.x;
    ui.scope(|ui| {
        ui.set_max_width((ui.available_width() - reserve).max(60.0));
        name_or_editor(cx, ui, record);
    });
    if cx.state.rename_draft.is_none() {
        ui.spacing_mut().button_padding = egui::vec2(6.0, 3.0);
        if theme::ghost_muted(ui, "Rename").clicked() {
            cx.state.rename_draft = Some((record.id, record.name.clone()));
        }
    }
    theme::status_dot(ui, state, 8.0);
    ui.add(
        egui::Label::new(
            RichText::new(state_text)
                .text_style(theme::meta())
                .color(p.state_text(state)),
        )
        .truncate(),
    );
}

/// Open in terminal (or Return), Restart, Kill, Back. `reversed` draws
/// them last to first, for a right-to-left row.
fn header_actions(
    cx: &mut DrawCtx<'_>,
    ui: &mut Ui,
    record: &SessionRecord,
    running: bool,
    reversed: bool,
) {
    ui.spacing_mut().item_spacing.x = 2.0;
    let agent = matches!(record.kind, SessionKind::Agent(_));
    let open = if running {
        "Open in terminal"
    } else {
        "Return"
    };
    let mut order = vec![open, "Restart", "Kill", "Back"];
    if agent {
        order.retain(|b| *b != "Restart");
    }
    if reversed {
        order.reverse();
    }
    for button in order {
        let response = match button {
            "Restart" => {
                theme::ghost(ui, button).on_hover_text("Stop it if it runs, then start it again")
            }
            "Kill" | "Back" => theme::ghost_muted(ui, button),
            _ => theme::secondary(ui, button),
        };
        if !response.clicked() {
            continue;
        }
        match button {
            "Restart" => cx.dispatch(AppAction::RestartSession(record.id)),
            "Kill" => cx.dispatch(AppAction::KillSession(record.id)),
            "Back" => cx.dispatch(AppAction::Back),
            _ => cx.dispatch(AppAction::ReturnToSession(record.id)),
        }
    }
}

/// The session name as a heading or, while a rename is under way, a
/// text field: Enter commits, Esc cancels.
fn name_or_editor(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    let editing = cx
        .state
        .rename_draft
        .as_ref()
        .is_some_and(|(id, _)| *id == record.id);
    if !editing {
        ui.add(egui::Label::new(RichText::new(&record.name).text_style(theme::h1())).truncate());
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
        let p = theme::palette(ui);
        Frame::new()
            .inner_margin(Margin::symmetric(0, GAP_PX / 2))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let label = ui
                        .label(
                            RichText::new("Notes")
                                .text_style(theme::meta())
                                .color(p.n600),
                        )
                        .id;
                    let response = ui.add(
                        egui::TextEdit::multiline(draft)
                            .desired_rows(1)
                            .font(theme::meta())
                            .text_color(p.n700)
                            .background_color(p.surface)
                            .margin(Margin::symmetric(10, 6))
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
    let p = theme::palette(ui);
    let fill = p.surface;
    // A file row dragged from the file side lights the panel up and, on
    // release, lands in the draft as its path.
    let hovering = egui::DragAndDrop::has_payload_of_type::<DraggedPath>(ui.ctx());
    let stroke = if hovering {
        Stroke::new(2.0, p.accent)
    } else {
        Stroke::NONE
    };
    let panel = egui::Panel::bottom("message_panel")
        .resizable(false)
        .frame(Frame::new().stroke(stroke).inner_margin(Margin {
            left: 0,
            right: 0,
            top: 12,
            bottom: 4,
        }))
        .show(ui, |ui| message_box(cx, ui, record))
        .response;
    if let Some(dropped) = panel.dnd_release_payload::<DraggedPath>() {
        let draft = cx.state.input_drafts.entry(record.id).or_default();
        append_path(draft, &dropped.0);
    }
    // The raw pane is its own panel above the message box, so the control
    // that hides it never scrolls away with the conversation.
    if cx.state.terminal_open {
        egui::Panel::bottom("terminal_panel")
            .resizable(true)
            .default_size(240.0)
            .frame(Frame::new().fill(fill).corner_radius(2).inner_margin(PAD))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    theme::kicker(ui, "Terminal", p.n600);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::ghost_muted(ui, "Hide").clicked() {
                            cx.state.terminal_open = false;
                        }
                    });
                });
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| match cx.state.snapshots.get(&record.id) {
                        Some(text) => code_block(ui, text),
                        None => {
                            ui.label(RichText::new("No snapshot yet.").weak());
                        }
                    });
            });
    }
    Frame::new()
        .inner_margin(Margin::symmetric(0, GAP_PX / 2))
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
                terminal_open,
                markdown,
                snapshots,
                raw_message,
                ..
            } = &mut *cx.state;
            let snapshot = snapshots.get(&record.id);
            if let Some((_, conversation)) = conversations.get(&record.id) {
                conversation_view(
                    ui,
                    conversation,
                    expand_activity,
                    expand_applied,
                    terminal_open,
                    markdown,
                    raw_message,
                );
            } else {
                ui.label(
                    RichText::new(
                        "This session runs in Ghostty. Use Open in terminal to bring its window up.",
                    )
                    .color(p.n700),
                );
                if let Some(e) = conversation_errors.get(&record.id) {
                    ui.label(theme::meta_text(ui, format!("No conversation view: {e}")));
                }
                if let Some(text) = snapshot {
                    egui::ScrollArea::both()
                        .auto_shrink([false, false])
                        .show(ui, |ui| code_block(ui, text));
                }
            }
        });
}

/// Rows the message box shows before it scrolls.
const MESSAGE_MAX_ROWS: usize = 8;

/// The message box: a multi-line field docked under the conversation.
/// Enter (or Send) types the text into the session's terminal and
/// presses Enter there, without opening its window; Shift+Enter adds a
/// line. Files dropped on the window, or rows dragged in from the file
/// side, land in the draft as paths, which is how an agent is pointed
/// at an image or a document.
fn message_box(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    let running = is_running(cx.core, record.id);
    let dropped: Vec<PathBuf> = ui.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_path_buf())
            .collect()
    });
    let draft = cx.state.input_drafts.entry(record.id).or_default();
    for path in dropped {
        append_path(draft, &path);
    }
    let mut send = false;
    let mut interrupt = false;
    let field_id = ui.id().with(("message", record.id));
    let p = theme::palette(ui);
    // The label is invisible but keeps the field findable by name.
    let label = ui.add(egui::Label::new(RichText::new("Message").size(0.1)));
    let row_height = ui.text_style_height(&egui::TextStyle::Body);
    #[allow(clippy::cast_precision_loss)]
    let max_height = row_height * MESSAGE_MAX_ROWS as f32 + GAP;
    egui::ScrollArea::vertical()
        .id_salt(("message_scroll", record.id))
        .max_height(max_height)
        .show(ui, |ui| {
            let response = ui
                .add_enabled(
                    running,
                    egui::TextEdit::multiline(draft)
                        .id(field_id)
                        .hint_text(
                            "Reply… Enter to send, Shift+Enter for a line, drop files for paths",
                        )
                        .desired_rows(3)
                        .background_color(p.surface)
                        .margin(Margin::symmetric(12, 10))
                        .desired_width(f32::INFINITY)
                        .return_key(egui::KeyboardShortcut::new(
                            egui::Modifiers::SHIFT,
                            egui::Key::Enter,
                        )),
                )
                .labelled_by(label.id);
            // Plain Enter is not the field's return key any more, so it
            // reaches us here; Cmd+Enter sends too, for the habit.
            let enter = ui.input(|i| {
                i.key_pressed(egui::Key::Enter)
                    && (i.modifiers.is_none() || i.modifiers.command_only())
            });
            if response.has_focus() && enter {
                send = true;
            }
        });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if ui
            .add_enabled_ui(running, |ui| theme::primary(ui, "Send"))
            .inner
            .clicked()
        {
            send = true;
        }
        if ui
            .add_enabled_ui(running, |ui| theme::ghost_muted(ui, "Stop"))
            .inner
            .on_hover_text("Send Escape to the agent (Cmd+.)")
            .clicked()
        {
            interrupt = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new("Drag a file from the tree, or Shift+click it, for its path")
                        .small()
                        .color(p.n600),
                )
                .truncate(),
            );
        });
    });
    // The draft stays in the box until the app reports the pane took it
    // (`SwitchboardApp` clears it after a successful write), so a dead
    // session or a failed write does not lose what was typed.
    if send && !draft.trim().is_empty() {
        let text = draft.clone();
        cx.dispatch(AppAction::SendInput {
            id: record.id,
            text,
        });
        ui.memory_mut(|m| m.request_focus(field_id));
    }
    // After the draft's last use, so the borrow of `cx.state` has ended.
    if interrupt {
        cx.dispatch(AppAction::Interrupt(record.id));
    }
}

/// Add a dropped file's path to a draft as its own word, quoted when
/// it holds spaces so the agent reads it as one path.
pub(super) fn append_path(draft: &mut String, path: &std::path::Path) {
    let shown = path.display().to_string();
    let word = if shown.contains(' ') {
        format!("'{shown}'")
    } else {
        shown
    };
    if !draft.is_empty() && !draft.ends_with([' ', '\n']) {
        draft.push(' ');
    }
    draft.push_str(&word);
}

/// Header line, then the turns in a scroll area that follows new
/// content, with the raw terminal snapshot folded away at the end.
fn conversation_view(
    ui: &mut Ui,
    conversation: &Conversation,
    expand: &mut bool,
    expand_applied: &mut Option<bool>,
    terminal_open: &mut bool,
    markdown: &mut CommonMarkCache,
    raw_message: &mut Option<String>,
) {
    let p = theme::palette(ui);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.spacing_mut().button_padding = egui::vec2(6.0, 3.0);
            let terminal = if *terminal_open {
                theme::ghost(ui, "Terminal")
            } else {
                theme::ghost_muted(ui, "Terminal")
            };
            if terminal
                .on_hover_text("Show the raw pane below the conversation (Cmd+T)")
                .clicked()
            {
                *terminal_open = !*terminal_open;
            }
            let expand_button = if *expand {
                theme::ghost(ui, "Expand activity")
            } else {
                theme::ghost_muted(ui, "Expand activity")
            };
            if expand_button.clicked() {
                *expand = !*expand;
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(theme::strong_text(
                        conversation.title.as_deref().unwrap_or("(untitled)"),
                    ))
                    .truncate(),
                );
            });
        });
    });
    ui.label(theme::meta_text(ui, meta_line(conversation)).color(p.n700));
    ui.add_space(4.0);
    // Push the toggle into every section only on the frame it changes,
    // so single sections can still be opened and closed by hand.
    let open = (*expand_applied != Some(*expand)).then_some(*expand);
    *expand_applied = Some(*expand);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            // Measured once, before any turn: a word egui cannot break
            // widens the layout for everything after it, and a cap read
            // back per turn would only carry that widening along.
            let width = ui.available_width().min(MAX_READING_WIDTH);
            for turn in &conversation.turns {
                turn_block(ui, turn, open, markdown, width, raw_message);
            }
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
    if let (Some(used), Some(model)) = (c.context_tokens(), c.model.as_deref()) {
        let window = context_window(model);
        parts.push(format!(
            "ctx {:.0}k / {:.0}k ({}%)",
            f64_from(used) / 1000.0,
            f64_from(window) / 1000.0,
            used * 100 / window.max(1)
        ));
    }
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
fn turn_block(
    ui: &mut Ui,
    turn: &Turn,
    open: Option<bool>,
    markdown: &mut CommonMarkCache,
    width: f32,
    raw_message: &mut Option<String>,
) {
    let p = theme::palette(ui);
    // Back to the reading width whatever the turns above did to it.
    ui.set_max_width(width);
    ui.spacing_mut().item_spacing.y = 10.0;
    // The user's turn: cyan-tinted block, kicker "YOU · time".
    let user_rect = Frame::new()
        .fill(p.accent_fill)
        .corner_radius(CornerRadius::same(2))
        .inner_margin(Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = egui::vec2(GAP, 6.0);
            ui.horizontal(|ui| {
                let mut kicker = format!("You · {}", time_text(turn.at));
                if let Some(mode) = &turn.permission_mode {
                    kicker = format!("{kicker} · {mode}");
                }
                theme::kicker(ui, &kicker, p.accent_text);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("#{}", turn.n))
                            .small()
                            .color(p.accent_on_fill.gamma_multiply(0.6)),
                    );
                });
            });
            scrolls_sideways(ui, ("user", turn.n), |ui| {
                ui.add(egui::Label::new(RichText::new(&turn.user).color(p.accent_on_fill)).wrap());
            });
        })
        .response
        .rect;
    message_menu(ui, user_rect, ("user", turn.n), &turn.user, raw_message);
    // The activity list: neutral rows, no frame, 4 px inset.
    Frame::new()
        .inner_margin(Margin::symmetric(4, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = egui::vec2(GAP, 4.0);
            egui::CollapsingHeader::new(
                RichText::new(stats_line(turn))
                    .text_style(theme::meta())
                    .color(p.n700),
            )
            .id_salt(("activity", turn.n))
            .open(open)
            .show(ui, |ui| {
                if turn.activity.is_empty() {
                    ui.label(
                        RichText::new("no tool activity")
                            .text_style(theme::excerpt())
                            .color(p.n600),
                    );
                }
                for (i, a) in turn.activity.iter().enumerate() {
                    activity_row(ui, a, (turn.n, i));
                }
            });
        });
    // The answer: surface block, kicker "CLAUDE · time", Markdown.
    let final_rect = theme::surface(ui)
        .inner_margin(Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = egui::vec2(GAP, 6.0);
            theme::kicker(ui, &format!("Agent · {}", time_text(turn.end)), p.n600);
            if turn.final_text.is_empty() {
                ui.label(
                    RichText::new("no final response (interrupted or tool-only)")
                        .text_style(theme::excerpt())
                        .color(p.n600),
                );
            } else {
                scrolls_sideways(ui, ("final", turn.n), |ui| {
                    super::document::markdown_style(ui);
                    CommonMarkViewer::new().show(ui, markdown, &turn.final_text);
                });
            }
        })
        .response
        .rect;
    if !turn.final_text.is_empty() {
        message_menu(
            ui,
            final_rect,
            ("final", turn.n),
            &turn.final_text,
            raw_message,
        );
    }
    ui.add_space(6.0);
}

/// Prose stops here, however wide the window; the mock reads at 860.
const MAX_READING_WIDTH: f32 = 860.0;

/// Text that wraps at the visible width but, where a word or a table
/// cannot wrap, scrolls sideways inside its block instead of widening
/// the block and everything under it.
fn scrolls_sideways(ui: &mut Ui, salt: (&str, usize), add: impl FnOnce(&mut Ui)) {
    let width = ui.available_width();
    egui::ScrollArea::horizontal()
        .id_salt(salt)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.set_max_width(width);
            add(ui);
        });
}

/// The right-click menu of one message (the user's prompt or the
/// agent's answer) covering `rect`: Copy, and View raw, which opens the
/// text unformatted in a dialog for when the Markdown renders badly.
///
/// The block is not made clickable: that would put it above the labels
/// and links inside it in egui's hit test and take their clicks. The
/// pointer is checked directly instead, and the menu is opened by hand.
fn message_menu(
    ui: &mut Ui,
    rect: egui::Rect,
    salt: (&str, usize),
    text: &str,
    raw_message: &mut Option<String>,
) {
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
            if ui.button("View raw").clicked() {
                *raw_message = Some(text.to_owned());
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
    let p = theme::palette(ui);
    let (glyph, color) = match (a.error, a.kind) {
        (true, _) => ("⚙", p.accent_2_text),
        (false, ActivityKind::Tool) => ("⚙", p.n700),
        (false, ActivityKind::Text) => ("…", p.n600),
        (false, ActivityKind::System) => ("·", p.n600),
    };
    let glyph_color = if a.error { color } else { p.accent_text };
    let mut line = RichText::new(&a.line).color(color);
    line = match a.kind {
        ActivityKind::Text => line.text_style(theme::excerpt()),
        ActivityKind::Tool | ActivityKind::System => line.text_style(theme::meta()),
    };
    match &a.detail {
        Some(detail) => {
            // Not a `CollapsingHeader`: its title never wraps or truncates,
            // and a long tool line would widen the whole conversation.
            let id = ui.make_persistent_id(("tool", salt));
            let mut open = ui.data(|d| d.get_temp(id).unwrap_or(false));
            let arrow = if open { "▾" } else { "▸" };
            let row = ui.add(
                egui::Button::new(
                    RichText::new(format!("{arrow} {glyph} {}", a.line))
                        .text_style(theme::meta())
                        .color(color),
                )
                .frame_when_inactive(false)
                .truncate(),
            );
            if row.on_hover_text(&a.line).clicked() {
                open = !open;
                ui.data_mut(|d| d.insert_temp(id, open));
            }
            if open {
                ui.indent(id, |ui| tool_detail(ui, detail, salt));
            }
        }
        None => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(glyph).color(glyph_color));
                ui.add(egui::Label::new(line).wrap());
            });
        }
    }
}

fn tool_detail(ui: &mut Ui, detail: &ToolDetail, salt: (usize, usize)) {
    let p = theme::palette(ui);
    for (label, text) in [("input", &detail.input), ("result", &detail.result)] {
        theme::kicker(ui, label, p.n600);
        if text.is_empty() {
            ui.label(
                RichText::new("(empty)")
                    .text_style(theme::excerpt())
                    .color(p.n600),
            );
            continue;
        }
        egui::ScrollArea::both()
            .id_salt((label, salt))
            .max_height(200.0)
            .show(ui, |ui| code_block(ui, text));
    }
}

/// Read-only monospace text, selectable: the dark code block of the
/// design on both themes.
pub(super) fn code_block(ui: &mut Ui, text: &str) {
    let p = theme::palette(ui);
    // A read-only `TextEdit` paints no background of its own, so the
    // dark fill comes from an explicit frame.
    ui.add(
        egui::TextEdit::multiline(&mut { text })
            .code_editor()
            .text_color(p.code_text)
            .frame(
                Frame::new()
                    .fill(p.code_fill)
                    .corner_radius(2)
                    .inner_margin(Margin::symmetric(14, 12)),
            )
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
        ui.label(RichText::new(note).color(theme::palette(ui).n700));
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

#[cfg(test)]
mod tests {
    use super::append_path;
    use std::path::Path;

    #[test]
    fn dropped_paths_join_the_draft_as_words() {
        let mut draft = String::new();
        append_path(&mut draft, Path::new("/tmp/a.png"));
        assert_eq!(draft, "/tmp/a.png");
        append_path(&mut draft, Path::new("/tmp/with space.png"));
        assert_eq!(draft, "/tmp/a.png '/tmp/with space.png'");
        let mut draft = "look at\n".to_owned();
        append_path(&mut draft, Path::new("/x"));
        assert_eq!(draft, "look at\n/x");
    }
}
