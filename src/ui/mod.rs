//! egui drawing. Reads state from the core, turns widget events into
//! actions, and nothing else. Logic that is not about pixels belongs in
//! the core.
//!
//! Every frame `draw` takes the UI's own transient state out of the app,
//! draws each widget against the core's read model while collecting the
//! actions those widgets produced, then puts the state back and dispatches
//! the actions. Collecting first keeps the borrow checker happy: drawing
//! needs `&AppCore` and `&mut UiState` at the same time, dispatching needs
//! `&mut SwitchboardApp`, and the two never overlap.

mod board;
mod cards;
pub mod dialogs;
pub mod document;
pub mod env;
pub mod files;
pub mod markdown;
pub mod palette;
mod rail;
mod run;
mod runbar;
mod session;
mod switchboard;
mod switcher;
pub mod theme;
pub mod working_set;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

use egui::{Key, Modifiers, RichText, Ui};

use crate::app::{Services, SwitchboardApp};
use crate::core::{AppAction, AppCore, ProjectId, RecordId, SideTab, ThemeMode, View};
use crate::ports::transcript::Conversation;

pub use dialogs::{AddProjectDraft, NewSessionDraft};
pub use session::EmbeddedTerminal;

/// State the UI owns between frames: dialog drafts, text being edited,
/// embedded terminals. Nothing here is persisted or read by the core.
pub struct UiState {
    /// Create real `egui_term` backends for shell sessions. Tests set
    /// this to `false` so nothing is spawned.
    pub embed_terminals: bool,
    /// One-line captions per session (last output), filled by the app.
    pub captions: HashMap<RecordId, String>,
    /// Screen snapshots per session, filled by the app; shown read-only
    /// for agent sessions and for sessions that are not running.
    pub snapshots: HashMap<RecordId, String>,
    /// Parsed transcripts per agent session with the file time they were
    /// read at, filled by the app; the session view draws them.
    pub conversations: HashMap<RecordId, (Option<SystemTime>, Conversation)>,
    /// Why a session has no conversation (unsupported agent, no file).
    pub conversation_errors: HashMap<RecordId, String>,
    /// Open every turn's activity list instead of just the summary line.
    pub expand_activity: bool,
    /// The last `expand_activity` value pushed into the collapsing
    /// headers; they follow the toggle only on the frame it changes, so
    /// individual sections can still be opened and closed by hand.
    pub expand_applied: Option<bool>,
    /// Show the raw pane as a panel under an agent's conversation.
    pub terminal_open: bool,
    /// Variable names the project's environment defines, for the Run
    /// tab's "not defined" marks: project, when resolved, names.
    pub run_env: Option<(ProjectId, std::time::Instant, Vec<String>)>,
    /// Layout cache for the markdown in final responses.
    pub markdown: egui_commonmark::CommonMarkCache,
    pub add_project: Option<AddProjectDraft>,
    pub new_session: Option<NewSessionDraft>,
    /// Notes text being edited in the session view, with its record.
    pub notes_draft: Option<(RecordId, String)>,
    /// A session name being edited in the session header.
    pub rename_draft: Option<(RecordId, String)>,
    /// The file side per project: tree, finder, index.
    pub files: HashMap<ProjectId, files::FilesState>,
    /// The document on screen, loaded once per path and file time.
    pub preview: Option<document::Preview>,
    /// The editor command being edited in the settings menu.
    pub editor_draft: Option<String>,
    /// The quick-switcher, while open.
    pub palette: Option<palette::PaletteDraft>,
    /// The Environment dialog, while open.
    pub env_dialog: Option<env::EnvDraft>,
    /// One message shown unformatted in a dialog ("View raw"), while
    /// open. The way to read a message whose Markdown renders badly.
    pub raw_message: Option<String>,
    /// How the message dialog shows its text; kept between openings.
    pub message_view: dialogs::MessageView,
    /// How many grid units the working set fits across right now, so
    /// a card added from another view lands where it will be seen.
    pub working_set_columns: u32,
    /// Arrange mode of the working set and the drag under way.
    pub arrange: working_set::Arrange,
    /// Previews for the working set's file cards, one per path.
    pub previews: HashMap<PathBuf, Option<document::Preview>>,
    /// How each file card shows its file: rendered or raw, wrapped or
    /// scrolling sideways.
    pub file_modes: HashMap<PathBuf, working_set::FileMode>,
    /// Messages being composed, one per session, so switching away and
    /// back does not lose a half-written prompt.
    pub input_drafts: HashMap<RecordId, String>,
    /// Embedded terminals, only ever the one for the session on screen.
    pub terminals: HashMap<RecordId, EmbeddedTerminal>,
    /// The theme last pushed into egui; pushed again only when it changes.
    pub applied_theme: Option<ThemeMode>,
    next_terminal_id: u64,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            embed_terminals: true,
            captions: HashMap::new(),
            snapshots: HashMap::new(),
            conversations: HashMap::new(),
            conversation_errors: HashMap::new(),
            expand_activity: false,
            expand_applied: None,
            terminal_open: false,
            run_env: None,
            markdown: egui_commonmark::CommonMarkCache::default(),
            add_project: None,
            new_session: None,
            notes_draft: None,
            rename_draft: None,
            files: HashMap::new(),
            preview: None,
            editor_draft: None,
            palette: None,
            env_dialog: None,
            raw_message: None,
            message_view: dialogs::MessageView::Raw,
            working_set_columns: 24,
            arrange: working_set::Arrange::default(),
            previews: HashMap::new(),
            file_modes: HashMap::new(),
            input_drafts: HashMap::new(),
            terminals: HashMap::new(),
            applied_theme: None,
            next_terminal_id: 0,
        }
    }
}

/// Everything one frame of drawing can see and produce.
pub struct DrawCtx<'a> {
    pub core: &'a AppCore,
    pub services: &'a Services,
    pub state: &'a mut UiState,
    actions: Vec<AppAction>,
}

impl DrawCtx<'_> {
    /// Queue an action; it is dispatched after the frame is drawn.
    pub fn dispatch(&mut self, action: AppAction) {
        self.actions.push(action);
    }
}

pub fn draw(app: &mut SwitchboardApp, ui: &mut Ui) {
    let mut state = std::mem::take(&mut app.ui_state);
    let actions = {
        let mut cx = DrawCtx {
            core: app.core(),
            services: app.services(),
            state: &mut state,
            actions: Vec::new(),
        };
        draw_frame(&mut cx, ui);
        cx.actions
    };
    app.ui_state = state;
    for action in actions {
        app.dispatch(action);
    }
}

fn draw_frame(cx: &mut DrawCtx<'_>, ui: &mut Ui) {
    let theme = cx.core.settings().theme;
    if cx.state.applied_theme != Some(theme) {
        ui.ctx().set_theme(match theme {
            ThemeMode::Auto => egui::ThemePreference::System,
            ThemeMode::Light => egui::ThemePreference::Light,
            ThemeMode::Dark => egui::ThemePreference::Dark,
        });
        cx.state.applied_theme = Some(theme);
    }
    let view = cx.core.view();
    keyboard(cx, ui, &view);

    // An embedded terminal only lives while its session is on screen.
    // Dropping the backend closes the pty, which detaches the tmux client
    // and leaves the session running.
    let shown = match &view {
        View::Session(id) => Some(*id),
        View::Switchboard | View::Board(_) | View::Document(..) | View::WorkingSet => None,
    };
    cx.state.terminals.retain(|id, _| Some(*id) == shown);

    // The rail's surface fill is the only edge between the columns.
    let palette = theme::palette(ui);
    let rail_fill = palette.surface;
    let page_fill = palette.bg;
    egui::Panel::left("rail")
        .resizable(true)
        .default_size(rail::DEFAULT_WIDTH)
        .size_range(56.0..=360.0)
        .show_separator_line(false)
        .frame(egui::Frame::new().fill(rail_fill))
        .show(ui, |ui| rail::show(cx, ui, &view));
    // The file side lives next to the board, next to the full preview it
    // opens, and, when toggled on, next to a session. The full preview
    // shows the selection itself; elsewhere the side previews inline.
    // Only an agent session has a message box for paths to go to.
    let files_for = match &view {
        View::Board(pid) => Some((*pid, true, None)),
        View::Document(pid, _) => Some((*pid, false, None)),
        View::Session(id) if cx.core.settings().files_open => cx.core.session(*id).map(|s| {
            let message = matches!(s.kind, crate::core::SessionKind::Agent(_)).then_some(*id);
            (s.project, true, message)
        }),
        View::Session(_) | View::Switchboard | View::WorkingSet => None,
    };
    if let Some((pid, inline, message)) = files_for {
        egui::Panel::right("files")
            .resizable(true)
            .default_size(360.0)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(page_fill)
                    .inner_margin(egui::Margin {
                        left: 8,
                        right: 20,
                        top: 22,
                        bottom: 0,
                    }),
            )
            .show(ui, |ui| {
                let tab = side_tabs(cx, ui, pid, matches!(view, View::Session(_)));
                match tab {
                    SideTab::Files => files::show(cx, ui, pid, inline, message),
                    SideTab::Run => run::show(cx, ui, pid),
                }
            });
    }
    // Sessions and documents fill their area edge to edge (terminal,
    // preview); the board and the switchboard get the page margin.
    let margin = match &view {
        View::Switchboard | View::Board(_) | View::WorkingSet => egui::Margin {
            left: 28,
            right: 28,
            top: 24,
            bottom: 20,
        },
        View::Session(_) | View::Document(..) => egui::Margin {
            left: 24,
            right: 24,
            top: 20,
            bottom: 16,
        },
    };
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(page_fill).inner_margin(margin))
        .show(ui, |ui| {
            cx.state.working_set_columns = working_set::columns(ui.available_width());
            match view {
                View::Switchboard => switchboard::show(cx, ui),
                View::WorkingSet => working_set::show(cx, ui),
                View::Board(pid) => board::show(cx, ui, pid),
                View::Session(id) => session::show(cx, ui, id),
                View::Document(pid, path) => document::show(cx, ui, pid, &path),
            }
        });

    switcher::toasts(cx, ui.ctx());
    dialogs::show(cx, ui.ctx());
    palette::show(cx, ui.ctx());
    env::show(cx, ui.ctx());
}

/// The side panel's tab row: Files and Run, the current one in cyan,
/// with Refresh at the right of the Files tab. Beside a session a click
/// on the tab already showing closes the side, as its shortcut does.
/// Returns the current tab.
fn side_tabs(cx: &mut DrawCtx<'_>, ui: &mut Ui, pid: ProjectId, closable: bool) -> SideTab {
    let tab = cx.core.settings().side_tab;
    let p = theme::palette(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        for t in [SideTab::Files, SideTab::Run] {
            let text = if tab == t {
                RichText::new(t.label())
                    .text_style(theme::strong())
                    .color(p.accent_text)
            } else {
                RichText::new(t.label()).color(p.n600)
            };
            let hint = if tab == t && closable {
                "Click again to close the side"
            } else {
                ""
            };
            if ui
                .add(egui::Button::new(text).frame_when_inactive(false))
                .on_hover_text(hint)
                .clicked()
            {
                if tab != t {
                    cx.dispatch(AppAction::SetSideTab(t));
                } else if closable {
                    cx.dispatch(AppAction::SetFilesOpen(false));
                }
            }
        }
        if tab == SideTab::Files {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let refresh = theme::ghost_muted(ui, "Refresh")
                    .on_hover_text("Re-read the tree and the finder index");
                if refresh.clicked()
                    && let Some(f) = cx.state.files.get_mut(&pid)
                {
                    f.refresh();
                }
            });
        }
    });
    ui.add_space(6.0);
    tab
}

/// Esc goes back, Cmd+1..9 switch project, Cmd+0 shows the switchboard,
/// Cmd+K opens the quick-switcher, Cmd+B and Cmd+R show the Files and
/// Run tabs of the side panel (again to close it beside a session),
/// Cmd+T the raw pane under a conversation, and Cmd+. sends Escape to
/// the session's terminal.
/// Esc is left alone while a text field, a dialog, or the terminal has
/// focus.
fn keyboard(cx: &mut DrawCtx<'_>, ui: &Ui, view: &View) {
    const DIGITS: [Key; 9] = [
        Key::Num1,
        Key::Num2,
        Key::Num3,
        Key::Num4,
        Key::Num5,
        Key::Num6,
        Key::Num7,
        Key::Num8,
        Key::Num9,
    ];
    let ctx = ui.ctx();
    let nothing_focused = ctx.memory(|m| m.focused().is_none());
    let has_dialog = cx.state.add_project.is_some()
        || cx.state.new_session.is_some()
        || cx.state.palette.is_some()
        || cx.state.env_dialog.is_some()
        || cx.state.raw_message.is_some();

    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::K)) {
        cx.state.palette = Some(palette::PaletteDraft::default());
    }
    // Cmd+B and Cmd+R each name a tab of the side panel: they show it,
    // opening the side beside a session if it is closed, and a second
    // press on the tab already showing closes the side again. Boards
    // always have the side, so there the keys only switch tabs.
    for (key, tab) in [(Key::B, SideTab::Files), (Key::R, SideTab::Run)] {
        if !matches!(view, View::Session(_) | View::Board(_))
            || !ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, key))
        {
            continue;
        }
        let settings = cx.core.settings();
        let showing = settings.side_tab == tab;
        if showing && settings.files_open && matches!(view, View::Session(_)) {
            cx.dispatch(AppAction::SetFilesOpen(false));
            continue;
        }
        if !showing {
            cx.dispatch(AppAction::SetSideTab(tab));
        }
        if matches!(view, View::Session(_)) && !settings.files_open {
            cx.dispatch(AppAction::SetFilesOpen(true));
        }
    }
    // Cmd+. is the macOS "stop" key; Escape already means Back here and
    // would also blur the message box.
    if let View::Session(id) = view
        && ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Period))
    {
        cx.dispatch(AppAction::Interrupt(*id));
    }
    if matches!(view, View::Session(_))
        && ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::T))
    {
        cx.state.terminal_open = !cx.state.terminal_open;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Num0)) {
        cx.dispatch(AppAction::ShowSwitchboard);
    }
    let projects: Vec<_> = rail::projects_by_recency(cx.core)
        .into_iter()
        .map(|p| p.id)
        .collect();
    for (n, key) in DIGITS.iter().enumerate() {
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, *key))
            && let Some(pid) = projects.get(n)
        {
            cx.dispatch(AppAction::ShowBoard(*pid));
        }
    }
    if nothing_focused
        && !has_dialog
        && *view != View::Switchboard
        && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
    {
        cx.dispatch(AppAction::Back);
    }
}

/// The gap between neighbouring items.
pub const GAP: f32 = 8.0;
/// Inner padding of every filled block (turn blocks, preview pane,
/// docked message panel), so boundaries line up across views.
pub const PAD: f32 = 14.0;
