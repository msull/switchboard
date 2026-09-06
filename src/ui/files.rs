//! The file side of a board: a lazily loaded tree of the project root
//! and a fuzzy finder over the whole project, both honoring
//! `.gitignore`. Clicking a file previews it; a right click offers the
//! hand-offs (default app, editor, Finder, copy path, pin) and, for a
//! directory, a shell there.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use egui::{RichText, Ui};

use super::{DrawCtx, GAP};
use crate::adapters::files::{Entry, Listing, children, fuzzy, scan};
use crate::core::{AppAction, Launch, ProjectId, SessionKind};

/// The index stops here; the finder says so when it does.
pub const MAX_INDEX_ENTRIES: usize = 50_000;
const MAX_HITS: usize = 60;

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
}

impl FilesState {
    fn refresh(&mut self) {
        self.children.clear();
        self.listing = None;
        self.scan = None;
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

pub fn show(cx: &mut DrawCtx<'_>, ui: &mut Ui, pid: ProjectId) {
    let Some(root) = cx.core.workspace(pid).map(|w| w.project.root.clone()) else {
        return;
    };
    let pinned: Vec<PathBuf> = cx
        .core
        .workspace(pid)
        .map(|w| w.project.pinned.clone())
        .unwrap_or_default();
    let mut state = cx.state.files.remove(&pid).unwrap_or_default();
    let mut actions = Vec::new();
    let mut side = Side {
        pid,
        root: &root,
        pinned: &pinned,
        actions: &mut actions,
    };
    ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
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
                    .hint_text("fuzzy path, Enter previews the first hit")
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
        });
    cx.state.files.insert(pid, state);
    for action in actions {
        cx.dispatch(action);
    }
}

/// What every row needs to build its actions.
struct Side<'a> {
    pid: ProjectId,
    root: &'a Path,
    pinned: &'a [PathBuf],
    actions: &'a mut Vec<AppAction>,
}

impl Side<'_> {
    fn preview(&mut self, path: &Path) {
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
                self.preview(path);
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
            } else {
                let selected = state.selected.as_deref() == Some(path.as_path());
                let response = ui.selectable_label(selected, format!("  {name}"));
                if response.clicked() {
                    state.selected = Some(path.clone());
                    side.preview(&path);
                }
                response.context_menu(|ui| side.context_menu(ui, &path, false));
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
        let response = ui.selectable_label(selected, hit.entry.rel.display().to_string());
        if response.clicked() || (open_first && n == 0) {
            state.selected = Some(path.clone());
            side.preview(&path);
        }
        response.context_menu(|ui| side.context_menu(ui, &path, false));
    }
}
