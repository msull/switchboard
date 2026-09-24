//! The session view: header for one record, and either an
//! embedded terminal (shells, commands, services) or, for agents, the
//! conversation read from the transcript with a message box docked at
//! the bottom. Agents run in Ghostty; the raw screen snapshot is kept
//! in a "Terminal" fold under the conversation.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

use std::time::SystemTime;

use egui::{CornerRadius, Frame, Margin, RichText, Stroke, Ui};
use egui_commonmark::CommonMarkCache;
use egui_term::{BackendSettings, PtyEvent, TerminalBackend, TerminalView};

use super::cards::{is_running, kind_label};
use super::files::DraggedPath;
use super::{DrawCtx, GAP, PAD, UiState, theme};
use crate::core::{
    AgentKind, AppAction, CardState, PinTarget, RecordId, ResumeHandle, SessionKind, SessionRecord,
};
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
    // The page is drawn in one place: while the session has a window
    // of its own, the main window only points there.
    if cx.core.popped_out(id) && cx.state.in_popout != Some(id) {
        ui.label(theme::strong_text(&record.name));
        ui.label(theme::meta_text(
            ui,
            "This session is open in its own window.",
        ));
        if theme::secondary(ui, "Show its window").clicked() {
            cx.dispatch(AppAction::PopOut(id));
        }
        return;
    }
    header(cx, ui, &record);
    match record.kind {
        SessionKind::Agent(_) => agent_body(cx, ui, &record),
        SessionKind::Shell => terminal_body(cx, ui, &record),
        SessionKind::Command | SessionKind::Service => super::runs::page(cx, ui, &record),
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
        let path = record.cwd.display().to_string();
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
            // The conversation's toggles sit on this line rather than
            // on one of their own, so the conversation starts higher.
            if matches!(record.kind, SessionKind::Agent(_)) {
                let terminal = if cx.state.terminal_open {
                    theme::ghost(ui, "Terminal")
                } else {
                    theme::ghost_muted(ui, "Terminal")
                };
                if terminal
                    .on_hover_text("Show the raw pane below the conversation (Cmd+T)")
                    .clicked()
                {
                    cx.state.terminal_open = !cx.state.terminal_open;
                }
                let expand = if cx.state.expand_activity {
                    theme::ghost(ui, "Expand activity")
                } else {
                    theme::ghost_muted(ui, "Expand activity")
                };
                if expand.clicked() {
                    cx.state.expand_activity = !cx.state.expand_activity;
                }
            }
            if let Some(handle) = &record.resume {
                id_menu(ui, handle);
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
}

/// The resume handle behind a small menu: copy the session id, or the
/// path of the conversation on disk. The id itself is not shown; it
/// took most of the line and is only ever wanted on the clipboard.
fn id_menu(ui: &mut Ui, handle: &ResumeHandle) {
    let button = theme::ghost_muted(ui, "ID").on_hover_text("Copy the session id or its file");
    egui::Popup::menu(&button).show(|ui| {
        if ui.button("Copy session id").clicked() {
            ui.ctx().copy_text(handle.provider_id());
            ui.close();
        }
        if let Some(path) = handle.transcript()
            && ui.button("Copy transcript path").clicked()
        {
            ui.ctx().copy_text(path.display().to_string());
            ui.close();
        }
    });
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
    let entry = matches!(record.kind, SessionKind::Command | SessionKind::Service);
    let mut order = vec![open, "Restart", "Kill", "Working sets", "Back"];
    if agent {
        order.retain(|b| *b != "Restart");
    }
    // A command's page speaks the card's language: ▶ Run (or Start) and
    // Stop, never Return or Restart, which read as something else.
    if entry {
        let run = if record.kind == SessionKind::Service {
            "▶ Start"
        } else {
            "▶ Run"
        };
        order = if running {
            vec!["Stop", "Working sets", "Back"]
        } else {
            vec![run, "Working sets", "Back"]
        };
    }
    // A plan review clones the conversation, so only a Claude Code
    // session with one to clone can start it.
    let reviewable = record.kind == SessionKind::Agent(AgentKind::ClaudeCode)
        && matches!(record.resume, Some(ResumeHandle::ClaudeCode { .. }));
    if reviewable {
        order.insert(1, "Review plan");
    }
    // Undo stays offered while the conversation is still the cut one: a
    // message sent from Switchboard clears it in the record, one typed
    // in the terminal shows as a turn past the cut.
    let undo = record.discard.as_ref().is_some_and(|d| {
        cx.state
            .conversations
            .get(&record.id)
            .is_none_or(|(_, c)| c.turns.len() < d.before)
    });
    if undo {
        order.insert(1, "Undo discard");
    }
    // In its own window Back means nothing; Close window puts the page
    // back in the main window. Elsewhere, Pop out gives it a window.
    let in_popout = cx.state.in_popout == Some(record.id);
    if in_popout {
        order.retain(|b| *b != "Back");
        order.push("Close window");
    } else {
        let at = order.len() - 1;
        order.insert(at, "Pop out");
    }
    if reversed {
        order.reverse();
    }
    for button in order {
        let response = match button {
            "Restart" => {
                theme::ghost(ui, button).on_hover_text("Stop it if it runs, then start it again")
            }
            "Working sets" => {
                let response = theme::ghost_muted(ui, button)
                    .on_hover_text("Which working sets show this session");
                egui::Popup::menu(&response).show(|ui| {
                    if super::working_set::set_menu(cx, ui, &PinTarget::Session(record.id)) {
                        ui.close();
                    }
                });
                continue;
            }
            "Kill" | "Back" => theme::ghost_muted(ui, button),
            "Pop out" => theme::ghost_muted(ui, button)
                .on_hover_text("Open this session in a window of its own (Cmd+Shift+P)"),
            "Close window" => theme::ghost_muted(ui, button)
                .on_hover_text("Close this window; the session shows in the main one (Cmd+W)"),
            "Undo discard" => theme::ghost(ui, button)
                .on_hover_text("Put back the conversation the last discard cut away"),
            "Review plan" => theme::ghost(ui, button).on_hover_text(
                "Have a fresh reviewer and a clone of this session revise a plan it wrote",
            ),
            "Stop" => theme::ghost_muted(ui, button).on_hover_text("Kill the process"),
            "▶ Run" | "▶ Start" => ui
                .add_enabled_ui(record.runnable(), |ui| theme::secondary(ui, button))
                .inner
                .on_hover_text("Run it now; every run keeps its own output"),
            _ => theme::secondary(ui, button),
        };
        if !response.clicked() {
            continue;
        }
        match button {
            "Restart" | "▶ Run" | "▶ Start" => {
                cx.dispatch(AppAction::RestartSession(record.id));
            }
            "Review plan" => {
                let draft = super::workflow::ReviewDraft::open(cx, record.id);
                cx.state.review_dialog = Some(draft);
            }
            "Kill" | "Stop" => cx.dispatch(AppAction::KillSession(record.id)),
            "Undo discard" => cx.dispatch(AppAction::UndoDiscard(record.id)),
            "Back" => cx.dispatch(AppAction::Back),
            "Pop out" => cx.dispatch(AppAction::PopOut(record.id)),
            "Close window" => cx.dispatch(AppAction::ClosePopout(record.id)),
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
    let prompt_box = cx.core.settings().prompt_box;
    let mut panel = egui::Panel::bottom("message_panel").resizable(prompt_box);
    if prompt_box {
        panel = panel.default_size(super::prompt_box::default_height());
    }
    let panel = panel
        .frame(Frame::new().stroke(stroke).inner_margin(Margin {
            left: 0,
            right: 0,
            top: if prompt_box { 2 } else { 12 },
            bottom: 4,
        }))
        .show(ui, |ui| {
            if prompt_box {
                super::prompt_box::panel(cx, ui, record);
            } else {
                message_box(cx, ui, record);
            }
        })
        .response;
    if let Some(dropped) = panel.dnd_release_payload::<DraggedPath>() {
        if prompt_box {
            if let Some(editor) = cx.state.prompt_boxes.editors.get_mut(&record.id) {
                let mut text = editor.core().doc().rendered();
                append_path(&mut text, &dropped.0);
                editor.set_text(&text);
            }
        } else {
            let draft = cx.state.input_drafts.entry(record.id).or_default();
            append_path(draft, &dropped.0);
        }
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
        .show(ui, |ui| conversation_or_pane(cx, ui, record));
}

/// The conversation when the transcript is readable, otherwise the
/// pointer to the terminal and the pane snapshot. Fills what is left.
fn conversation_or_pane(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    let p = theme::palette(ui);
    // Cloning is offered on the user's own messages of a Claude Code
    // session that has a transcript; the core refuses the rest anyway.
    let cloneable = matches!(record.kind, SessionKind::Agent(AgentKind::ClaudeCode))
        && record
            .resume
            .as_ref()
            .is_some_and(|h| h.transcript().is_some());
    let mut clone_at: Option<usize> = None;
    let mut discard_at: Option<usize> = None;
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
        raw_message,
        message_links,
        ..
    } = &mut *cx.state;
    let snapshot = snapshots.get(&record.id);
    let Some((_, conversation)) = conversations.get(&record.id) else {
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
        return;
    };
    conversation_view(
        ui,
        &record.name,
        conversation,
        Toggles {
            expand: expand_activity,
            expand_applied,
        },
        markdown,
        Menus {
            raw_message,
            message_links,
            clone_at: cloneable.then_some(&mut clone_at),
            discard_at: cloneable.then_some(&mut discard_at),
        },
    );
    // Both borrow the conversation, so the prompts are looked up before
    // dispatching, which needs the whole context again.
    let prompt_of = |before: usize| {
        conversation
            .turns
            .iter()
            .find(|t| t.n == before)
            .map(|t| t.user.clone())
            .unwrap_or_default()
    };
    let clone = clone_at.map(|before| (before, prompt_of(before)));
    let discard = discard_at.map(|before| (before, prompt_of(before)));
    if let Some((before, prompt)) = clone {
        cx.dispatch(AppAction::CloneSession {
            id: record.id,
            before,
            prompt,
        });
    }
    if let Some((before, prompt)) = discard {
        cx.dispatch(AppAction::DiscardTo {
            id: record.id,
            before,
            prompt,
        });
    }
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
/// The activity fold's state, written back to the UI state.
struct Toggles<'a> {
    expand: &'a mut bool,
    expand_applied: &'a mut Option<bool>,
}

/// The conversation under its toggles. Claude Code's own name for the
/// conversation (`/rename`) is shown only when it differs from the
/// record's name, marked as Claude's, so a rename in Switchboard does
/// not leave the old name sitting under the new one.
fn conversation_view(
    ui: &mut Ui,
    name: &str,
    conversation: &Conversation,
    toggles: Toggles<'_>,
    markdown: &mut CommonMarkCache,
    mut menus: Menus<'_>,
) {
    let Toggles {
        expand,
        expand_applied,
    } = toggles;
    let p = theme::palette(ui);
    // Both labels truncate: a row that cannot shrink would widen the
    // conversation under the side panel.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add(
            egui::Label::new(theme::meta_text(ui, meta_line(conversation)).color(p.n700))
                .truncate(),
        );
        if let Some(title) = conversation.title.as_deref().filter(|t| *t != name) {
            ui.label(theme::meta_text(ui, "·"));
            ui.add(egui::Label::new(theme::meta_text(ui, format!("Claude: {title}"))).truncate())
                .on_hover_text(
                    "The conversation's name inside Claude Code (/rename); Rename above \
                 changes only Switchboard's name for the session",
                );
        }
    });
    ui.add_space(2.0);
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
                turn_block(ui, turn, open, markdown, width, menus.reborrow());
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

/// One turn: the prompt, then the agent's messages in order, each with
/// the folded tool activity that led to it, and the final answer.
fn turn_block(
    ui: &mut Ui,
    turn: &Turn,
    open: Option<bool>,
    markdown: &mut CommonMarkCache,
    width: f32,
    mut menus: Menus<'_>,
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
    message_menu(
        ui,
        user_rect,
        ("user", turn.n),
        turn.n,
        &turn.user,
        menus.reborrow(),
    );
    // Text the agent wrote along the way splits the activity into groups:
    // the tools before each message fold under it, the message itself
    // reads like the answer. The first group carries the turn's totals.
    let mut group_start = 0;
    let mut groups = 0;
    for (i, a) in turn.activity.iter().enumerate() {
        if a.kind != ActivityKind::Text {
            continue;
        }
        activity_group(ui, turn, group_start..i, groups, open);
        groups += 1;
        group_start = i + 1;
        let text = a.text.as_deref().unwrap_or(&a.line);
        let rect = agent_block(ui, &time_text(a.at), text, ("agent", turn.n, i), markdown);
        message_menu(
            ui,
            rect,
            ("agent", turn.n, i),
            turn.n,
            text,
            menus.without_clone(),
        );
    }
    activity_group(ui, turn, group_start..turn.activity.len(), groups, open);
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
                    super::markdown::show(ui, markdown, &turn.final_text);
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
            turn.n,
            &turn.final_text,
            menus.without_clone(),
        );
    }
    ui.add_space(6.0);
}

/// One folded list of activity rows. The first group of a turn shows
/// the turn's totals and appears even when empty; a later group shows
/// its own count and appears only with rows to show.
fn activity_group(
    ui: &mut Ui,
    turn: &Turn,
    range: std::ops::Range<usize>,
    group: usize,
    open: Option<bool>,
) {
    if group > 0 && range.is_empty() {
        return;
    }
    let p = theme::palette(ui);
    let rows = &turn.activity[range.clone()];
    let title = if group == 0 {
        stats_line(turn)
    } else {
        group_line(rows)
    };
    Frame::new()
        .inner_margin(Margin::symmetric(4, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = egui::vec2(GAP, 4.0);
            egui::CollapsingHeader::new(
                RichText::new(title).text_style(theme::meta()).color(p.n700),
            )
            .id_salt(("activity", turn.n, group))
            .open(open)
            .show(ui, |ui| {
                if rows.is_empty() {
                    ui.label(
                        RichText::new("no tool activity")
                            .text_style(theme::excerpt())
                            .color(p.n600),
                    );
                }
                for (i, a) in rows.iter().enumerate() {
                    activity_row(ui, a, (turn.n, range.start + i));
                }
            });
        });
}

/// "3 tools · 1 error" for one group of activity rows.
fn group_line(rows: &[Activity]) -> String {
    let tools = rows.iter().filter(|a| a.kind == ActivityKind::Tool).count();
    let errors = rows.iter().filter(|a| a.error).count();
    let mut parts = vec![format!("{tools} tools")];
    if errors > 0 {
        parts.push(format!("{errors} errors"));
    }
    parts.join(" · ")
}

/// A message from the agent: surface block, kicker "AGENT · time",
/// Markdown. Returns its rect for the context menu.
fn agent_block(
    ui: &mut Ui,
    when: &str,
    text: &str,
    salt: (&str, usize, usize),
    markdown: &mut CommonMarkCache,
) -> egui::Rect {
    let p = theme::palette(ui);
    theme::surface(ui)
        .inner_margin(Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = egui::vec2(GAP, 6.0);
            theme::kicker(ui, &format!("Agent · {when}"), p.n600);
            scrolls_sideways(ui, salt, |ui| {
                super::document::markdown_style(ui);
                super::markdown::show(ui, markdown, text);
            });
        })
        .response
        .rect
}

/// Prose stops here, however wide the window; the mock reads at 860.
const MAX_READING_WIDTH: f32 = 860.0;

/// Text that wraps at the visible width but, where a word or a table
/// cannot wrap, scrolls sideways inside its block instead of widening
/// the block and everything under it.
fn scrolls_sideways(ui: &mut Ui, salt: impl egui::AsIdSalt, add: impl FnOnce(&mut Ui)) {
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
/// agent's answer) covering `rect`: Copy; View raw, which opens the
/// text unformatted in a dialog for when the Markdown renders badly;
/// and View links, which lists the message's web links to click.
///
/// The block is not made clickable: that would put it above the labels
/// and links inside it in egui's hit test and take their clicks. The
/// pointer is checked directly instead, and the menu is opened by hand.
/// What the message menus write back: the text to show in the raw
/// dialog, the links to list, and (when the session can be cloned) the
/// turn number the user chose to fork before.
struct Menus<'a> {
    raw_message: &'a mut Option<String>,
    message_links: &'a mut Option<Vec<String>>,
    clone_at: Option<&'a mut Option<usize>>,
    discard_at: Option<&'a mut Option<usize>>,
}

impl Menus<'_> {
    fn reborrow(&mut self) -> Menus<'_> {
        Menus {
            raw_message: self.raw_message,
            message_links: self.message_links,
            clone_at: self.clone_at.as_deref_mut(),
            discard_at: self.discard_at.as_deref_mut(),
        }
    }
    /// The same menus without the clone and discard items, for the
    /// agent's messages.
    fn without_clone(&mut self) -> Menus<'_> {
        Menus {
            raw_message: self.raw_message,
            message_links: self.message_links,
            clone_at: None,
            discard_at: None,
        }
    }
}

/// The right-click menu on a message. "Clone session" is offered on the
/// user's own messages of a cloneable session.
fn message_menu(
    ui: &mut Ui,
    rect: egui::Rect,
    salt: impl egui::AsIdSalt,
    turn: usize,
    text: &str,
    menus: Menus<'_>,
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
                *menus.raw_message = Some(text.to_owned());
                ui.close();
            }
            // Offered only when there is something to list: an empty
            // dialog would say less than a missing item.
            let links = super::dialogs::web_links(text);
            if !links.is_empty() && ui.button("View links").clicked() {
                *menus.message_links = Some(links);
                ui.close();
            }
            if let Some(clone_at) = menus.clone_at
                && ui
                    .button("Clone session")
                    .on_hover_text(
                        "A new session with the conversation up to here, this message ready to send",
                    )
                    .clicked()
            {
                *clone_at = Some(turn);
                ui.close();
            }
            if let Some(discard_at) = menus.discard_at
                && ui
                    .button("Discard to here")
                    .on_hover_text(
                        "Cut this session back to before this message, with it ready to send \
                         again; Undo discard puts everything back until the next message is sent",
                    )
                    .clicked()
            {
                *discard_at = Some(turn);
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
pub fn code_block(ui: &mut Ui, text: &str) {
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
    live_pane(cx, ui, record);
}

/// The embedded terminal attached to the record's running pane.
pub(super) fn live_pane(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    cx.state.terminals_drawn.insert(record.id);
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
    use super::{EmbeddedTerminal, append_path};
    use std::path::Path;

    /// Open descriptors of this process, as the kernel lists them.
    fn open_fds() -> usize {
        std::fs::read_dir("/dev/fd").map_or(0, Iterator::count)
    }

    /// A dropped terminal gives back its pty, its poller, and its two
    /// threads; a page that opens one per frame must not run out of
    /// files.
    #[test]
    #[cfg(unix)]
    fn a_dropped_terminal_releases_its_descriptors() {
        // The login shell runs the attach; a CI box without it skips.
        if !Path::new(&super::login_shell()).exists() {
            return;
        }
        let ctx = egui::Context::default();
        let attach = vec!["sleep".to_owned(), "30".to_owned()];
        let cwd = std::env::temp_dir();
        // Warm up: the first terminal loads what the rest share.
        drop(EmbeddedTerminal::attach(0, ctx.clone(), &attach, cwd.clone()).unwrap());
        std::thread::sleep(std::time::Duration::from_millis(300));
        let baseline = open_fds();
        for id in 1..=20 {
            let term = EmbeddedTerminal::attach(id, ctx.clone(), &attach, cwd.clone()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(50));
            drop(term);
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        let after = open_fds();
        assert!(
            after <= baseline + 2,
            "{baseline} descriptors before, {after} after twenty terminals"
        );
    }

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
