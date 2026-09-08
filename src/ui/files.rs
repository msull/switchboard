//! The file side of a board or a session: a lazily loaded tree of the
//! project root and a fuzzy finder over the whole project, both honoring
//! `.gitignore`, with a preview of the selected file in the bottom half.
//! Clicking a file previews it there (or, when the full document view is
//! already on screen, in that view); a right click offers the hand-offs
//! (default app, editor, Finder, copy path, pin) and, for a directory, a
//! shell there.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use egui::{Color32, RichText, Ui};

use super::session::append_path;
use super::{DrawCtx, GAP, document};
use crate::adapters::files::{Entry, Listing, children, fuzzy, scan};
use crate::adapters::git::{Change, GitState, inspect};
use crate::core::{AppAction, Launch, ProjectId, RecordId, SessionKind};

/// The drag-and-drop payload of a file row: its absolute path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraggedPath(pub PathBuf);

/// The index stops here; the finder says so when it does.
pub const MAX_INDEX_ENTRIES: usize = 50_000;
const MAX_HITS: usize = 60;
/// How often `git status` is re-run for a project on screen.
const GIT_INTERVAL: Duration = Duration::from_secs(5);

/// Per-project state of the file side.
#[derive(Default)]
pub struct FilesState {
    pub query: String,
    /// Absolute directories currently open in the tree.
    pub expanded: HashSet<PathBuf>,
    /// Children per absolute directory, read once until refreshed.
    pub children: HashMap<PathBuf, Result<Vec<Entry>, String>>,
    /// The whole-project listing the finder searches.
    pub listing: Option<Arc<Listing>>,
    /// A scan in progress, delivering the listing.
    pub scan: Option<Receiver<std::io::Result<Listing>>>,
    pub selected: Option<PathBuf>,
    /// Branches and changed paths, refreshed in the background.
    pub git: Option<Arc<GitState>>,
    pub git_scan: Option<Receiver<GitState>>,
    pub git_at: Option<Instant>,
}

impl FilesState {
    fn refresh(&mut self) {
        self.children.clear();
        self.listing = None;
        self.scan = None;
    }

    /// Refresh the git state every `GIT_INTERVAL`, off the UI thread.
    pub fn poll_git(&mut self, root: &Path, ctx: &egui::Context) {
        if let Some(rx) = &self.git_scan {
            if let Ok(state) = rx.try_recv() {
                self.git = Some(Arc::new(state));
                self.git_scan = None;
            }
            return;
        }
        if self.git_at.is_some_and(|t| t.elapsed() < GIT_INTERVAL) {
            return;
        }
        self.git_at = Some(Instant::now());
        let (tx, rx) = channel();
        let root = root.to_path_buf();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(inspect(&root));
            ctx.request_repaint();
        });
        self.git_scan = Some(rx);
    }

    /// Start (once) and poll the background scan.
    fn ensure_listing(&mut self, root: &Path, ctx: &egui::Context) {
        if self.listing.is_some() {
            return;
        }
        if let Some(rx) = &self.scan {
            match rx.try_recv() {
                Ok(Ok(listing)) => {
                    self.listing = Some(Arc::new(listing));
                    self.scan = None;
                }
                Ok(Err(e)) => {
                    log::warn!("file index of {}: {e}", root.display());
                    self.listing = Some(Arc::new(Listing {
                        entries: Vec::new(),
                        truncated: false,
                        scanned_at: std::time::SystemTime::now(),
                    }));
                    self.scan = None;
                }
                Err(_) => {}
            }
            return;
        }
        let (tx, rx) = channel();
        let root = root.to_path_buf();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = scan(&root, MAX_INDEX_ENTRIES);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.scan = Some(rx);
    }
}

/// Draw the file side for `pid`. With `inline` the selected file is
/// previewed in the bottom half of the side itself; without it a click
/// opens the full document view (which is then already on screen).
/// `message` is the session whose message box paths can be sent to:
/// Shift+click on a row, the row menu, and the pane's button do that.
pub fn show(
    cx: &mut DrawCtx<'_>,
    ui: &mut Ui,
    pid: ProjectId,
    inline: bool,
    message: Option<RecordId>,
) {
    let Some(root) = cx.core.workspace(pid).map(|w| w.project.root.clone()) else {
        return;
    };
    let pinned: Vec<PathBuf> = cx
        .core
        .workspace(pid)
        .map(|w| w.project.pinned.clone())
        .unwrap_or_default();
    let mut state = cx.state.files.remove(&pid).unwrap_or_default();
    state.poll_git(&root, ui.ctx());
    let git = state.git.clone();
    let mut actions = Vec::new();
    let mut to_message = Vec::new();
    let mut side = Side {
        pid,
        root: &root,
        pinned: &pinned,
        git: git.as_deref(),
        inline,
        can_message: message.is_some(),
        actions: &mut actions,
        to_message: &mut to_message,
    };
    ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
    // Panels claim their space before the rest of the side is laid out,
    // so the preview goes first even though it sits at the bottom.
    if inline {
        let selected = state.selected.clone();
        let outcome = egui::Panel::bottom(egui::Id::new(("files_preview", pid)))
            .resizable(true)
            .default_size(ui.available_height() / 2.0)
            .show(ui, |ui| {
                preview_pane(cx, ui, pid, selected.as_deref(), message.is_some())
            })
            .inner;
        match outcome {
            PaneClick::Clear => state.selected = None,
            PaneClick::ToMessage => side.to_message.extend(selected.clone()),
            PaneClick::None => {}
        }
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("Files").strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("Refresh")
                .on_hover_text("Re-read the tree and the finder index")
                .clicked()
            {
                state.refresh();
            }
        });
    });
    let mut open_first = false;
    ui.horizontal(|ui| {
        let label = ui.label("Find").id;
        let field = ui
            .add(
                egui::TextEdit::singleline(&mut state.query)
                    .hint_text("fuzzy path, Enter picks the first hit")
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(label);
        open_first = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    });
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt(("files", pid))
        .auto_shrink(false)
        .show(ui, |ui| {
            if state.query.trim().is_empty() {
                let root_dir = root.clone();
                directory(ui, &mut state, &mut side, &root_dir, 0);
            } else {
                finder(ui, &mut state, &mut side, open_first);
            }
            // The blank space under the rows: a click there clears the
            // selection, as in Finder.
            let blank = ui.available_rect_before_wrap();
            if blank.height() > 0.0 && ui.allocate_rect(blank, egui::Sense::click()).clicked() {
                state.selected = None;
            }
        });
    cx.state.files.insert(pid, state);
    if let Some(id) = message {
        let draft = cx.state.input_drafts.entry(id).or_default();
        for path in to_message {
            append_path(draft, &path);
        }
    }
    for action in actions {
        cx.dispatch(action);
    }
}

/// What the preview pane's header buttons asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaneClick {
    None,
    Clear,
    ToMessage,
}

/// The bottom half of the side: the selected file, drawn by the
/// document view's renderer, with a way to the full view.
fn preview_pane(
    cx: &mut DrawCtx<'_>,
    ui: &mut Ui,
    pid: ProjectId,
    path: Option<&Path>,
    can_message: bool,
) -> PaneClick {
    ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
    // A panel shrinks to its content; an empty pane would collapse and
    // then jump back when a file is selected. Hold the panel's height.
    ui.set_min_height(ui.available_height());
    let Some(path) = path else {
        ui.label(RichText::new("Select a file to preview it here.").weak());
        return PaneClick::None;
    };
    let mut click = PaneClick::None;
    document::ensure_loaded(cx.state, path);
    let (name, size) = cx.state.preview.as_ref().map_or_else(
        || (String::new(), 0),
        |p| {
            (
                p.path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                p.size,
            )
        },
    );
    ui.horizontal(|ui| {
        ui.label(RichText::new(name).strong());
        ui.label(RichText::new(document::size_text(size)).weak().small());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("×")
                .on_hover_text("Clear the selection")
                .clicked()
            {
                click = PaneClick::Clear;
            }
            if can_message
                && ui
                    .small_button("To message")
                    .on_hover_text("Put the path in the message box")
                    .clicked()
            {
                click = PaneClick::ToMessage;
            }
            if ui
                .small_button("Expand")
                .on_hover_text("Preview it full size")
                .clicked()
            {
                cx.dispatch(AppAction::ShowDocument(pid, path.to_path_buf()));
            }
            if ui.small_button("Open in editor").clicked() {
                cx.dispatch(AppAction::OpenInEditor(path.to_path_buf()));
            }
        });
    });
    ui.separator();
    let is_code = cx
        .state
        .preview
        .as_ref()
        .is_some_and(|p| matches!(p.body, document::Body::Text(_)));
    egui::ScrollArea::both()
        .id_salt(("files_preview_body", pid, path))
        .auto_shrink(false)
        .show(ui, |ui| {
            // Code keeps its lines and scrolls sideways; prose wraps.
            if is_code {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            }
            document::body(cx.state, ui);
        });
    click
}

/// What every row needs to build its actions.
struct Side<'a> {
    pid: ProjectId,
    root: &'a Path,
    pinned: &'a [PathBuf],
    git: Option<&'a GitState>,
    /// The selection is previewed in the side; a click does not navigate.
    inline: bool,
    /// A message box is on screen to send paths to.
    can_message: bool,
    actions: &'a mut Vec<AppAction>,
    /// Paths to put in the message box after the frame.
    to_message: &'a mut Vec<PathBuf>,
}

/// Row colors for git status.
#[must_use]
pub fn change_color(change: Change) -> Color32 {
    match change {
        Change::Modified => Color32::from_rgb(235, 140, 0),
        Change::Untracked => Color32::from_rgb(60, 170, 80),
        Change::Conflict => Color32::from_rgb(220, 50, 50),
    }
}

impl Side<'_> {
    /// The git glyph for a row, drawn after its name.
    fn decoration(&self, ui: &mut Ui, rel: &Path, is_dir: bool) {
        if let Some(change) = self.git.and_then(|g| g.status_of(rel, is_dir)) {
            let glyph = if is_dir { "•" } else { change.glyph() };
            ui.label(RichText::new(glyph).color(change_color(change)).small())
                .on_hover_text(match change {
                    Change::Modified => "modified",
                    Change::Untracked => "untracked",
                    Change::Conflict => "conflict",
                });
        }
    }

    /// A click on a file row: select and preview it, or with Shift (and
    /// a message box on screen) put its path in the message instead.
    fn pick(&mut self, state: &mut FilesState, path: &Path, shift: bool) {
        if shift && self.can_message {
            self.to_message.push(path.to_path_buf());
            return;
        }
        state.selected = Some(path.to_path_buf());
        self.preview(path);
    }

    /// What a click on a file row does beyond selecting it.
    fn preview(&mut self, path: &Path) {
        if !self.inline {
            self.expand(path);
        }
    }

    /// Open the full document view.
    fn expand(&mut self, path: &Path) {
        self.actions
            .push(AppAction::ShowDocument(self.pid, path.to_path_buf()));
    }

    /// The right-click menu of a file or directory row.
    fn context_menu(&mut self, ui: &mut Ui, path: &Path, is_dir: bool) {
        let rel = path.strip_prefix(self.root).ok().map(Path::to_path_buf);
        if is_dir {
            if ui.button("New shell here").clicked() {
                let name = path
                    .file_name()
                    .map_or_else(|| "shell".to_owned(), |n| n.to_string_lossy().into_owned());
                self.actions.push(AppAction::NewSession {
                    project: self.pid,
                    name,
                    kind: SessionKind::Shell,
                    cwd: path.to_path_buf(),
                    launch: Launch::Shell,
                });
                ui.close();
            }
        } else {
            if ui.button("Preview").clicked() {
                self.expand(path);
                ui.close();
            }
            if self.can_message
                && ui
                    .button("Add path to message")
                    .on_hover_text("Shift+click a row does the same")
                    .clicked()
            {
                self.to_message.push(path.to_path_buf());
                ui.close();
            }
            if ui.button("Open").on_hover_text("Default app").clicked() {
                self.actions
                    .push(AppAction::OpenDocument(path.to_path_buf()));
                ui.close();
            }
            if ui.button("Open in editor").clicked() {
                self.actions
                    .push(AppAction::OpenInEditor(path.to_path_buf()));
                ui.close();
            }
            if let Some(rel) = &rel {
                let pinned = self.pinned.contains(rel);
                if ui.button(if pinned { "Unpin" } else { "Pin" }).clicked() {
                    self.actions.push(if pinned {
                        AppAction::UnpinDocument(self.pid, rel.clone())
                    } else {
                        AppAction::PinDocument(self.pid, rel.clone())
                    });
                    ui.close();
                }
            }
        }
        if ui.button("Reveal in Finder").clicked() {
            self.actions
                .push(AppAction::RevealDocument(path.to_path_buf()));
            ui.close();
        }
        if ui.button("Copy path").clicked() {
            ui.ctx().copy_text(path.display().to_string());
            ui.close();
        }
    }
}

/// A file row: a selectable label that can also be dragged, carrying
/// its path for whatever accepts a [`DraggedPath`] (the message box).
/// The row keeps its click; the drag is a second interaction over the
/// same rect, and egui tells the two apart by pointer movement.
fn file_row(ui: &mut Ui, selected: bool, text: &str, path: &Path) -> egui::Response {
    let response = ui.selectable_label(selected, text);
    response
        .interact(egui::Sense::drag())
        .on_hover_cursor(egui::CursorIcon::Grab)
        .dnd_set_drag_payload(DraggedPath(path.to_path_buf()));
    response
}

/// One level of the tree: `dir`'s children, indented by `depth`.
fn directory(ui: &mut Ui, state: &mut FilesState, side: &mut Side<'_>, dir: &Path, depth: usize) {
    let root = side.root.to_path_buf();
    let entries = state
        .children
        .entry(dir.to_path_buf())
        .or_insert_with(|| children(&root, dir).map_err(|e| e.to_string()))
        .clone();
    let entries = match entries {
        Ok(entries) => entries,
        Err(e) => {
            ui.label(RichText::new(format!("cannot read: {e}")).weak());
            return;
        }
    };
    if entries.is_empty() && depth == 0 {
        ui.label(RichText::new("empty").weak());
    }
    for entry in entries {
        let path = root.join(&entry.rel);
        let name = entry
            .rel
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        ui.horizontal(|ui| {
            #[allow(clippy::cast_precision_loss)]
            ui.add_space(depth as f32 * 14.0);
            if entry.is_dir {
                let open = state.expanded.contains(&path);
                // Glyphs from egui's icon font; the text font lacks ▸/▾.
                let arrow = if open { "⏷" } else { "⏵" };
                let response = ui.selectable_label(false, format!("{arrow} {name}"));
                if response.clicked() {
                    if open {
                        state.expanded.remove(&path);
                    } else {
                        state.expanded.insert(path.clone());
                    }
                }
                response.context_menu(|ui| side.context_menu(ui, &path, true));
                side.decoration(ui, &entry.rel, true);
            } else {
                let selected = state.selected.as_deref() == Some(path.as_path());
                let response = file_row(ui, selected, &format!("  {name}"), &path);
                if response.clicked() {
                    side.pick(state, &path, ui.input(|i| i.modifiers.shift));
                }
                response.context_menu(|ui| side.context_menu(ui, &path, false));
                side.decoration(ui, &entry.rel, false);
            }
        });
        if entry.is_dir && state.expanded.contains(&path) {
            directory(ui, state, side, &path, depth + 1);
        }
    }
}

/// Finder results for the current query.
fn finder(ui: &mut Ui, state: &mut FilesState, side: &mut Side<'_>, open_first: bool) {
    state.ensure_listing(side.root, ui.ctx());
    let Some(listing) = state.listing.clone() else {
        ui.label(RichText::new("indexing…").weak());
        return;
    };
    let query = state.query.clone();
    let hits = fuzzy(&listing.entries, &query, MAX_HITS, false);
    if hits.is_empty() {
        ui.label(RichText::new("no matches").weak());
    }
    if listing.truncated {
        ui.label(
            RichText::new(format!("index stopped at {MAX_INDEX_ENTRIES} entries"))
                .weak()
                .small(),
        );
    }
    for (n, hit) in hits.iter().enumerate() {
        let path = side.root.join(&hit.entry.rel);
        let selected = state.selected.as_deref() == Some(path.as_path());
        let response = file_row(ui, selected, &hit.entry.rel.display().to_string(), &path);
        if response.clicked() || (open_first && n == 0) {
            side.pick(
                state,
                &path,
                response.clicked() && ui.input(|i| i.modifiers.shift),
            );
        }
        response.context_menu(|ui| side.context_menu(ui, &path, false));
    }
}
