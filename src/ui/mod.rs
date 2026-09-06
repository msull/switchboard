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
mod dialogs;
pub mod document;
pub mod files;
mod session;
mod switchboard;
mod switcher;

use std::collections::HashMap;
use std::time::SystemTime;

use egui::{Key, Modifiers, Ui};

use crate::app::{Services, SwitchboardApp};
use crate::core::{AppAction, AppCore, ProjectId, RecordId, ThemeMode, View};
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
    /// Message being composed for a session, sent with Enter.
    pub input_draft: Option<(RecordId, String)>,
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
            markdown: egui_commonmark::CommonMarkCache::default(),
            add_project: None,
            new_session: None,
            notes_draft: None,
            rename_draft: None,
            files: HashMap::new(),
            preview: None,
            editor_draft: None,
            input_draft: None,
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
        View::Switchboard | View::Board(_) | View::Document(..) => None,
    };
    cx.state.terminals.retain(|id, _| Some(*id) == shown);

    egui::Panel::top("top_bar")
        .resizable(false)
        .show(ui, |ui| switcher::top_bar(cx, ui, &view));
    egui::Panel::bottom("bottom_bar")
        .resizable(false)
        .show(ui, |ui| switcher::bottom_bar(cx, ui));
    // The file side lives next to the board and the preview it opens.
    if let View::Board(pid) | View::Document(pid, _) = &view {
        let pid = *pid;
        egui::Panel::right("files")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| files::show(cx, ui, pid));
    }
    egui::CentralPanel::default().show(ui, |ui| match view {
        View::Switchboard => switchboard::show(cx, ui),
        View::Board(pid) => board::show(cx, ui, pid),
        View::Session(id) => session::show(cx, ui, id),
        View::Document(pid, path) => document::show(cx, ui, pid, &path),
    });

    dialogs::show(cx, ui.ctx());
}

/// Esc goes back, Cmd+1..9 switch project, Cmd+0 shows the switchboard.
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
    let has_dialog = cx.state.add_project.is_some() || cx.state.new_session.is_some();

    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Num0)) {
        cx.dispatch(AppAction::ShowSwitchboard);
    }
    let projects: Vec<_> = switcher::projects_by_recency(cx.core)
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

/// Consistent spacing everywhere: the 8 px grid from the design.
pub const GAP: f32 = 8.0;
/// Inner padding of every framed region (header strip, turn blocks,
/// docked message panel), so boundaries line up across views.
pub const PAD: f32 = 12.0;
