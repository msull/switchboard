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
mod session;
mod switchboard;
mod switcher;

use std::collections::HashMap;

use egui::{Key, Modifiers, Ui};

use crate::app::{Services, SwitchboardApp};
use crate::core::{AppAction, AppCore, RecordId, View};

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
    pub add_project: Option<AddProjectDraft>,
    pub new_session: Option<NewSessionDraft>,
    /// Notes text being edited in the session view, with its record.
    pub notes_draft: Option<(RecordId, String)>,
    /// Message being composed for a session, sent with Enter.
    pub input_draft: Option<(RecordId, String)>,
    /// Embedded terminals, only ever the one for the session on screen.
    pub terminals: HashMap<RecordId, EmbeddedTerminal>,
    next_terminal_id: u64,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            embed_terminals: true,
            captions: HashMap::new(),
            snapshots: HashMap::new(),
            add_project: None,
            new_session: None,
            notes_draft: None,
            input_draft: None,
            terminals: HashMap::new(),
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
    let view = cx.core.view();
    keyboard(cx, ui, &view);

    // An embedded terminal only lives while its session is on screen.
    // Dropping the backend closes the pty, which detaches the tmux client
    // and leaves the session running.
    let shown = match view {
        View::Session(id) => Some(id),
        View::Switchboard | View::Board(_) => None,
    };
    cx.state.terminals.retain(|id, _| Some(*id) == shown);

    egui::Panel::top("top_bar")
        .resizable(false)
        .show(ui, |ui| switcher::top_bar(cx, ui, &view));
    egui::Panel::bottom("bottom_bar")
        .resizable(false)
        .show(ui, |ui| switcher::bottom_bar(cx, ui));
    egui::CentralPanel::default().show(ui, |ui| match view {
        View::Switchboard => switchboard::show(cx, ui),
        View::Board(pid) => board::show(cx, ui, pid),
        View::Session(id) => session::show(cx, ui, id),
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
