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
pub mod config;
pub mod dialogs;
pub mod document;
pub mod env;
pub mod files;
pub mod markdown;
pub mod notes;
pub mod palette;
pub mod popout;
pub mod prompt_box;
mod rail;
mod run;
mod runbar;
pub mod runs;
mod session;
mod switchboard;
mod switcher;
pub mod theme;
pub mod workflow;
pub mod working_set;
pub mod zoom;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::SystemTime;

use egui::{Key, Modifiers, RichText, Ui};

use crate::app::{Services, SwitchboardApp};
use crate::core::{
    AppAction, AppCore, ProjectId, RecordId, SetId, SideTab, ThemeMode, View, WindowFrame,
    WorkflowId,
};
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
    /// The project config editor, while open.
    pub config_dialog: Option<config::ConfigDraft>,
    /// The "Review a plan" dialog while it is open.
    pub review_dialog: Option<workflow::ReviewDraft>,
    /// Which round each review page shows, its diff toggle, its note.
    pub review_views: workflow::ReviewViews,
    /// The review whose cleanup awaits confirmation.
    pub confirm_cleanup: Option<WorkflowId>,
    /// Run logs read from disk, by record and run, with the file time
    /// they were read at.
    pub run_logs: HashMap<(RecordId, u32), (Option<SystemTime>, String)>,
    /// The run a command's page shows; the latest when unset.
    pub run_selected: HashMap<RecordId, u32>,
    /// What a command's card body shows: its output or one artifact.
    pub run_modes: HashMap<RecordId, runs::RunCardMode>,
    /// PDF pages rasterized for preview, by file.
    pub pdf_renders: HashMap<PathBuf, document::PdfRender>,
    /// One message shown unformatted in a dialog ("View raw"), while
    /// open. The way to read a message whose Markdown renders badly.
    pub raw_message: Option<String>,
    /// The web links of one message, shown as a clickable list in a
    /// dialog ("View links"), while open.
    pub message_links: Option<Vec<String>>,
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
    /// A working set's name being edited in its header.
    pub set_rename: Option<(SetId, String)>,
    /// The working set whose deletion is being confirmed.
    pub delete_set: Option<SetId>,
    /// The space name dialog: a new space (`None`) or a rename, with
    /// the draft name.
    pub space_editor: Option<(Option<crate::core::SpaceId>, String)>,
    /// Messages being composed, one per session, so switching away and
    /// back does not lose a half-written prompt.
    pub input_drafts: HashMap<RecordId, String>,
    /// Prompts the core primed for a session's Prompt Box editor (a
    /// clone's or a discard's), taken when that editor is next drawn.
    /// Separate from the drafts, which the cards' quick-send lines share.
    pub primed: HashMap<RecordId, String>,
    /// Embedded terminals, only for the panes drawn last frame: a
    /// session's own, or the agent at work on a review page.
    pub terminals: HashMap<RecordId, EmbeddedTerminal>,
    /// The terminals drawn this frame; the rest are dropped at the next
    /// frame's start, which detaches their tmux client.
    pub terminals_drawn: HashSet<RecordId>,
    /// Session windows the core asked to raise; drawn to the front on
    /// the next frame.
    pub focus_windows: Vec<RecordId>,
    /// The session whose own window is being drawn right now, so the
    /// page knows it is not in the main window.
    pub in_popout: Option<RecordId>,
    /// Each session window's frame as last seen and since when, so a
    /// move is saved once it settles.
    pub popout_frames: HashMap<RecordId, (WindowFrame, std::time::Instant)>,
    /// Where each session window was opened, in its own zoomed points,
    /// kept for the life of the window: egui re-sends a builder's
    /// position whenever it changes, which would drag a window back to
    /// its saved place as its zoom changes between displays.
    pub popout_opened: HashMap<RecordId, Option<(egui::Pos2, egui::Vec2)>>,
    /// The main window's frame as last seen and since when, so a move
    /// is saved once it settles.
    pub main_frame: Option<(WindowFrame, std::time::Instant)>,
    /// When the first frame was drawn: for a moment after launch the
    /// main window is put back at its saved frame if the system moved
    /// it (a Dock launch drags a new window onto the Dock's display).
    pub first_frame: Option<std::time::Instant>,
    /// The main window's zoom factor as of the end of the last frame,
    /// put back before its next pass (see `ui/zoom.rs`).
    pub main_zoom: Option<f32>,
    /// The zoom, in percent, of the pop-out the pointer is over this
    /// frame, left installed once the frame ends.
    pub zoom_under_pointer: Option<u32>,
    /// The theme last pushed into egui; pushed again only when it changes.
    pub applied_theme: Option<ThemeMode>,
    /// The Prompt Box editors of agent sessions and the voice runtime.
    pub prompt_boxes: prompt_box::PromptBoxes,
    /// The Prompt Box settings being edited in the settings menu.
    pub voice_draft: Option<switcher::VoiceDraft>,
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
            config_dialog: None,
            review_dialog: None,
            review_views: HashMap::new(),
            confirm_cleanup: None,
            run_logs: HashMap::new(),
            run_selected: HashMap::new(),
            run_modes: HashMap::new(),
            pdf_renders: HashMap::new(),
            raw_message: None,
            message_links: None,
            message_view: dialogs::MessageView::Rendered,
            working_set_columns: 24,
            arrange: working_set::Arrange::default(),
            previews: HashMap::new(),
            file_modes: HashMap::new(),
            set_rename: None,
            delete_set: None,
            space_editor: None,
            input_drafts: HashMap::new(),
            primed: HashMap::new(),
            terminals: HashMap::new(),
            applied_theme: None,
            prompt_boxes: prompt_box::PromptBoxes::default(),
            voice_draft: None,
            next_terminal_id: 0,
            terminals_drawn: HashSet::new(),
            focus_windows: Vec::new(),
            in_popout: None,
            popout_frames: HashMap::new(),
            popout_opened: HashMap::new(),
            main_frame: None,
            first_frame: None,
            main_zoom: None,
            zoom_under_pointer: None,
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
    // Prompts the editors sent this frame go to their panes now that the
    // app is free to dispatch.
    let sent: Vec<(RecordId, String)> =
        std::mem::take(&mut *state.prompt_boxes.outbox.lock().expect("outbox mutex"));
    app.ui_state = state;
    for action in actions {
        app.dispatch(action);
    }
    for (id, text) in sent {
        app.dispatch(AppAction::SendInput { id, text });
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
    zoom::main_window(cx, ui.ctx());
    let view = cx.core.view();
    keyboard(cx, ui, &view);
    if let Some(delay) = prompt_box::pump(cx) {
        ui.ctx().request_repaint_after(delay);
    }

    // An embedded terminal only lives while its pane is on screen: the
    // ones drawn last frame stay, the rest are dropped, which closes the
    // pty, detaches the tmux client, and leaves the session running.
    // Keyed on what was drawn rather than on the view, because a review
    // page and a command page draw panes of their own.
    let drawn = std::mem::take(&mut cx.state.terminals_drawn);
    cx.state.terminals.retain(|id, _| drawn.contains(id));

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
        View::Board(pid) => Some((*pid, true, None, None)),
        View::Document(pid, _) => Some((*pid, false, None, None)),
        View::Session(id) if cx.core.settings().files_open => cx.core.session(*id).map(|s| {
            let message = matches!(s.kind, crate::core::SessionKind::Agent(_)).then_some(*id);
            (s.project, true, message, Some(*id))
        }),
        View::Session(_) | View::Switchboard | View::WorkingSet(_) | View::Workflow(_) => None,
    };
    if let Some((pid, inline, message, session)) = files_for {
        side_panel(cx, ui, pid, inline, message, session, None);
    }
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(page_fill)
                .inner_margin(page_margin(&view)),
        )
        .show(ui, |ui| {
            cx.state.working_set_columns = working_set::columns(ui.available_width());
            match view {
                View::Switchboard => switchboard::show(cx, ui),
                View::WorkingSet(set) => working_set::show(cx, ui, set),
                View::Board(pid) => board::show(cx, ui, pid),
                View::Session(id) => session::show(cx, ui, id),
                View::Document(pid, path) => document::show(cx, ui, pid, &path),
                View::Workflow(id) => workflow::show(cx, ui, id),
            }
        });

    popout::show_all(cx, ui.ctx());
    switcher::toasts(cx, ui.ctx(), None);
    prompt_box::overlays(cx.state, ui.ctx());
    dialogs::show(cx, ui.ctx());
    palette::show(cx, ui.ctx());
    env::show(cx, ui.ctx());
    config::show(cx, ui.ctx());
    workflow::dialog_show(cx, ui.ctx());
    zoom::end_frame(cx, ui.ctx());
}

/// The side panel (Files, Run, Notes) beside the page: on its right by
/// default, or on its left between the rail and the page.
fn side_panel(
    cx: &mut DrawCtx<'_>,
    ui: &mut Ui,
    pid: ProjectId,
    inline: bool,
    message: Option<RecordId>,
    session: Option<RecordId>,
    window: Option<RecordId>,
) {
    // On the left the side sits between the rail and the page, so
    // its wider margin faces the page either way. A session window's
    // panel has an id of its own, so its size is remembered apart from
    // the main window's.
    let on_left = cx.core.settings().side_left;
    let name = if on_left { "files-left" } else { "files" };
    let id = match window {
        Some(w) => egui::Id::new((name, w)),
        None => egui::Id::new(name),
    };
    let panel = if on_left {
        egui::Panel::left(id)
    } else {
        egui::Panel::right(id)
    };
    let (left_margin, right_margin) = if on_left { (20, 12) } else { (12, 20) };
    let side = panel
        .resizable(true)
        .default_size(360.0)
        .show_separator_line(false)
        .frame(
            egui::Frame::new()
                .fill(theme::palette(ui).bg)
                .inner_margin(egui::Margin {
                    left: left_margin,
                    right: right_margin,
                    top: 22,
                    bottom: 0,
                }),
        )
        .show(ui, |ui| {
            let tab = side_tabs(cx, ui, pid, session);
            match (tab, session) {
                (SideTab::Notes, Some(id)) => notes::show(cx, ui, id),
                (SideTab::Files | SideTab::Notes, _) => {
                    files::show(cx, ui, pid, inline, message);
                }
                (SideTab::Run, _) => run::show(cx, ui, pid),
            }
        });
    // The side shares the page's ground, so a hairline on the edge
    // facing the content is what marks it off from it.
    let rect = side.response.rect;
    let edge = if on_left { rect.right() } else { rect.left() };
    ui.painter()
        .vline(edge, rect.y_range(), theme::palette(ui).hairline());
}

/// Sessions and documents fill their area edge to edge (terminal,
/// preview); the board and the switchboard get the page margin.
fn page_margin(view: &View) -> egui::Margin {
    match view {
        View::Switchboard | View::Board(_) | View::WorkingSet(_) | View::Workflow(_) => {
            egui::Margin {
                left: 28,
                right: 28,
                top: 24,
                bottom: 20,
            }
        }
        View::Session(_) | View::Document(..) => egui::Margin {
            left: 24,
            right: 24,
            top: 20,
            bottom: 16,
        },
    }
}

/// The side panel's tab row: Files, Run, and beside a session Notes,
/// the current one in cyan, with Refresh at the right of the Files tab.
/// Beside a session a click on the tab already showing closes the side,
/// as its shortcut does. Returns the tab to draw: Notes stays chosen
/// while a board is on screen but the board's side shows Files.
fn side_tabs(
    cx: &mut DrawCtx<'_>,
    ui: &mut Ui,
    pid: ProjectId,
    session: Option<RecordId>,
) -> SideTab {
    let closable = session.is_some();
    let tab = match cx.core.settings().side_tab {
        SideTab::Notes if session.is_none() => SideTab::Files,
        tab => tab,
    };
    let p = theme::palette(ui);
    let tabs: &[SideTab] = if closable {
        &[SideTab::Files, SideTab::Run, SideTab::Notes]
    } else {
        &[SideTab::Files, SideTab::Run]
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        for &t in tabs {
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

/// Cmd+B, Cmd+R, and Cmd+N each name a tab of the side panel: they
/// show it, opening the side beside a session if it is closed, and a
/// second press on the tab already showing closes the side again.
/// Boards always have the side, so there the keys only switch tabs,
/// and Notes belongs to a session alone. `session` says which of the
/// two the keys are beside.
fn side_tab_keys(cx: &mut DrawCtx<'_>, ctx: &egui::Context, session: bool) {
    for (key, tab) in [
        (Key::B, SideTab::Files),
        (Key::R, SideTab::Run),
        (Key::N, SideTab::Notes),
    ] {
        let allowed = session || tab != SideTab::Notes;
        if !allowed || !ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, key)) {
            continue;
        }
        let settings = cx.core.settings();
        let showing = settings.side_tab == tab;
        if showing && settings.files_open && session {
            cx.dispatch(AppAction::SetFilesOpen(false));
            continue;
        }
        if !showing {
            cx.dispatch(AppAction::SetSideTab(tab));
        }
        if session && !settings.files_open {
            cx.dispatch(AppAction::SetFilesOpen(true));
        }
    }
}

/// Esc goes back, Cmd+1..9 switch project, Cmd+0 shows the switchboard,
/// Cmd+K opens the quick-switcher, Cmd+B, Cmd+R, and Cmd+N show the
/// Files, Run, and Notes tabs of the side panel (again to close it
/// beside a session; Notes only there),
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
        || cx.state.config_dialog.is_some()
        || cx.state.review_dialog.is_some()
        || cx.state.confirm_cleanup.is_some()
        || cx.state.raw_message.is_some()
        || cx.state.message_links.is_some()
        || cx.state.delete_set.is_some();

    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::K)) {
        cx.state.palette = Some(palette::PaletteDraft::default());
    }
    match view {
        View::Session(_) => side_tab_keys(cx, ctx, true),
        View::Board(_) => side_tab_keys(cx, ctx, false),
        _ => {}
    }
    // Cmd+Shift+P gives the session a window of its own.
    if let View::Session(id) = view
        && ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::P))
    {
        cx.dispatch(AppAction::PopOut(*id));
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
