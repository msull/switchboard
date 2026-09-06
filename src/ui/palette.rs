//! The quick-switcher (Cmd+K): one field that fuzzy-matches every
//! project and session you may see, Enter opens the best hit, Esc closes.

use std::path::PathBuf;

use egui::{Context, RichText};

use super::cards::state_color;
use super::{DrawCtx, GAP};
use crate::adapters::files::{Entry, fuzzy};
use crate::core::{AppAction, CardState, ProjectId, RecordId};

/// Open while the palette shows.
#[derive(Debug, Default, Clone)]
pub struct PaletteDraft {
    pub query: String,
}

/// What a row opens.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Board(ProjectId),
    Session(RecordId),
}

struct Row {
    target: Target,
    title: String,
    detail: String,
    state: Option<CardState>,
    /// What the fuzzy matcher sees: `project/session`.
    key: PathBuf,
}

const MAX_ROWS: usize = 12;

pub fn show(cx: &mut DrawCtx<'_>, ctx: &Context) {
    let Some(mut draft) = cx.state.palette.take() else {
        return;
    };
    let rows = rows(cx);
    let entries: Vec<Entry> = rows
        .iter()
        .map(|r| Entry {
            rel: r.key.clone(),
            is_dir: false,
            size: 0,
        })
        .collect();
    let shown: Vec<usize> = if draft.query.trim().is_empty() {
        (0..rows.len().min(MAX_ROWS)).collect()
    } else {
        fuzzy(&entries, &draft.query, MAX_ROWS, false)
            .into_iter()
            .filter_map(|h| entries.iter().position(|e| e.rel == h.entry.rel))
            .collect()
    };

    let mut keep = true;
    let mut chosen: Option<Target> = None;
    egui::Window::new("Go to")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 80.0))
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
            ui.set_min_width(420.0);
            let label = ui.label("Search").id;
            let field = ui
                .add(
                    egui::TextEdit::singleline(&mut draft.query)
                        .hint_text("project or session")
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(label);
            field.request_focus();
            let (enter, escape) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::Enter),
                    i.key_pressed(egui::Key::Escape),
                )
            });
            if escape {
                keep = false;
            }
            if enter && let Some(&first) = shown.first() {
                chosen = Some(rows[first].target.clone());
            }
            ui.separator();
            if shown.is_empty() {
                ui.label(RichText::new("no matches").weak());
            }
            for &i in &shown {
                let row = &rows[i];
                ui.horizontal(|ui| {
                    if ui.selectable_label(false, &row.title).clicked() {
                        chosen = Some(row.target.clone());
                    }
                    ui.label(RichText::new(&row.detail).weak().small());
                    if let Some(state) = &row.state {
                        ui.label(
                            RichText::new(state.label())
                                .color(state_color(ui, state))
                                .small(),
                        );
                    }
                });
            }
        });
    if let Some(target) = chosen {
        cx.dispatch(match target {
            Target::Board(pid) => AppAction::ShowBoard(pid),
            Target::Session(id) => AppAction::ShowSession(id),
        });
        keep = false;
    }
    if keep {
        cx.state.palette = Some(draft);
    }
}

/// Projects (most recent first) and their sessions, waiting ones first.
fn rows(cx: &DrawCtx<'_>) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut workspaces: Vec<_> = cx.core.visible_workspaces().collect();
    workspaces.sort_by_key(|w| std::cmp::Reverse(w.project.last_active));
    for w in workspaces {
        rows.push(Row {
            target: Target::Board(w.project.id),
            title: w.project.name.clone(),
            detail: w.project.root.display().to_string(),
            state: None,
            key: PathBuf::from(&w.project.name),
        });
        for s in cx.core.sessions_sorted(w.project.id) {
            rows.push(Row {
                target: Target::Session(s.id),
                title: format!("{} / {}", w.project.name, s.name),
                detail: super::cards::kind_label(s.kind).to_owned(),
                state: Some(cx.core.card_state(s.id)),
                key: PathBuf::from(format!("{}/{}", w.project.name, s.name)),
            });
        }
    }
    rows
}
