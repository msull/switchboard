//! The project rail: the left column that names the app, lists every
//! project with its most urgent state as a dot, and pins Go to and
//! Settings at the bottom. Beside a session it lists that project's
//! entries instead, so the neighbours are one click away. Below about
//! 120 px it draws only dots and initials.

use egui::{
    Align2, FontId, Response, RichText, Sense, TextStyle, Ui, Vec2, WidgetInfo, WidgetType,
};

use super::dialogs::AddProjectDraft;
use super::{DrawCtx, theme};
use crate::core::{AppAction, AppCore, CardState, ProjectId, RecordId, SideTab, View};

/// The rail's width when it opens; the user may drag it.
pub const DEFAULT_WIDTH: f32 = 200.0;
/// Below this the rail draws dots and initials only.
pub const COMPACT_BELOW: f32 = 120.0;
/// Inner padding: 22 top, 16 right, 20 bottom, 22 left in the design;
/// one value keeps rows aligned with the brand.
const PAD: i8 = 16;

/// Projects most recently active first: the rail order, also used for
/// Cmd+1..9.
pub fn projects_by_recency(core: &AppCore) -> Vec<&crate::core::Project> {
    let mut projects: Vec<_> = core.visible_workspaces().map(|w| &w.project).collect();
    projects.sort_by_key(|p| std::cmp::Reverse(p.last_active));
    projects
}

/// The most urgent state among a project's sessions, or `None` when it
/// has none: the dot next to its name.
#[must_use]
pub fn project_state(core: &AppCore, project: ProjectId) -> Option<CardState> {
    core.workspace(project)?
        .sessions
        .iter()
        .map(|s| core.card_state(s.id))
        .min_by_key(CardState::rank)
}

fn waiting_in(core: &AppCore, project: ProjectId) -> usize {
    core.workspace(project).map_or(0, |w| {
        w.sessions
            .iter()
            .filter(|s| core.card_state(s.id) == CardState::WaitingOnYou)
            .count()
    })
}

pub fn show(cx: &mut DrawCtx<'_>, ui: &mut Ui, view: &View) {
    let compact = ui.available_width() < COMPACT_BELOW;
    ui.spacing_mut().item_spacing = Vec2::new(0.0, 2.0);
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: PAD,
            right: PAD,
            top: 20,
            bottom: 16,
        })
        .show(ui, |ui| {
            ui.set_min_height(ui.available_height());
            // The bottom items are laid out first from the bottom up so
            // they stay pinned; the rest fills from the top.
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                bottom(cx, ui, view, compact);
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    top(cx, ui, view, compact);
                });
            });
        });
}

fn top(cx: &mut DrawCtx<'_>, ui: &mut Ui, view: &View, compact: bool) {
    let p = theme::palette(ui);
    let brand = if compact { "S" } else { "Switchboard" };
    if ui
        .add(egui::Button::new(RichText::new(brand).text_style(theme::brand())).frame(false))
        .on_hover_text("Every session across every project (Cmd+0)")
        .clicked()
    {
        cx.dispatch(AppAction::ShowSwitchboard);
    }
    ui.add_space(14.0);
    super::prompt_box::rail_indicator(cx, ui, compact);

    let waiting = cx.core.waiting_count();
    let all = row(
        ui,
        &RowSpec {
            text: "All sessions",
            dot: None,
            selected: *view == View::Switchboard,
            muted: false,
            count: waiting,
            compact,
            initial: "A",
        },
    );
    if all.clicked() {
        cx.dispatch(AppAction::ShowSwitchboard);
    }
    working_set_rows(cx, ui, view, compact);

    match view {
        View::Session(id) => session_neighbours(cx, ui, *id, compact),
        View::Switchboard | View::Board(_) | View::Document(..) | View::WorkingSet(_) => {
            let active = match view {
                View::Board(pid) | View::Document(pid, _) => Some(*pid),
                View::Switchboard | View::Session(_) | View::WorkingSet(_) => None,
            };
            ui.add_space(12.0);
            if !compact {
                theme::kicker(ui, "Projects", p.n600);
                ui.add_space(4.0);
            }
            let projects: Vec<_> = projects_by_recency(cx.core)
                .into_iter()
                .map(|p| (p.id, p.name.clone()))
                .collect();
            for (n, (pid, name)) in projects.iter().enumerate() {
                let state = project_state(cx.core, *pid);
                let running = state.as_ref().is_some_and(|s| {
                    !matches!(
                        s,
                        CardState::NotRunning | CardState::NotResumable | CardState::Exited(_)
                    )
                });
                let response = row(
                    ui,
                    &RowSpec {
                        text: name,
                        dot: Some(state.unwrap_or(CardState::NotRunning)),
                        selected: active == Some(*pid),
                        muted: !running,
                        count: waiting_in(cx.core, *pid),
                        compact,
                        initial: &initial(name),
                    },
                );
                let hint = if n < 9 {
                    format!("{name} (Cmd+{})", n + 1)
                } else {
                    name.clone()
                };
                if response.on_hover_text(hint).clicked() {
                    cx.dispatch(AppAction::ShowBoard(*pid));
                }
            }
            let add = if compact { "+" } else { "+ Add project" };
            if ui
                .add(
                    egui::Button::new(
                        RichText::new(add)
                            .text_style(theme::meta())
                            .color(p.accent_text),
                    )
                    .frame_when_inactive(false),
                )
                .on_hover_text("Add project")
                .clicked()
            {
                cx.state.add_project = Some(AddProjectDraft::default());
            }
        }
    }
}

/// The "Working sets" section: one row per set, the empty ones greyed,
/// and a way to make another.
fn working_set_rows(cx: &mut DrawCtx<'_>, ui: &mut Ui, view: &View, compact: bool) {
    let p = theme::palette(ui);
    ui.add_space(12.0);
    if !compact {
        theme::kicker(ui, "Working sets", p.n600);
        ui.add_space(4.0);
    }
    let sets: Vec<(crate::core::SetId, String, usize)> = cx
        .core
        .working_sets()
        .iter()
        .map(|s| (s.id, s.name.clone(), s.items.len()))
        .collect();
    for (id, name, count) in &sets {
        let response = row(
            ui,
            &RowSpec {
                text: name,
                dot: None,
                selected: *view == View::WorkingSet(*id),
                muted: *count == 0,
                count: 0,
                compact,
                initial: &initial(name),
            },
        );
        if response.on_hover_text(name).clicked() {
            cx.dispatch(AppAction::ShowWorkingSet(*id));
        }
    }
    let add = if compact { "+" } else { "+ New working set" };
    if ui
        .add(
            egui::Button::new(
                RichText::new(add)
                    .text_style(theme::meta())
                    .color(p.accent_text),
            )
            .frame_when_inactive(false),
        )
        .on_hover_text("A new, empty working set")
        .clicked()
    {
        cx.dispatch(AppAction::NewWorkingSet {
            name: None,
            clone_of: None,
            with: None,
            columns: cx.state.working_set_columns,
        });
    }
}

/// Beside a session: its project's name as a kicker, a way back to the
/// board, and every entry of the project with the current one selected.
fn session_neighbours(cx: &mut DrawCtx<'_>, ui: &mut Ui, id: RecordId, compact: bool) {
    let p = theme::palette(ui);
    let Some((pid, project_name)) = cx.core.session(id).and_then(|s| {
        cx.core
            .workspace(s.project)
            .map(|w| (s.project, w.project.name.clone()))
    }) else {
        return;
    };
    ui.add_space(12.0);
    if !compact {
        theme::kicker(ui, &project_name, p.n600);
        ui.add_space(4.0);
    }
    let back = row(
        ui,
        &RowSpec {
            text: "← Board",
            dot: None,
            selected: false,
            muted: false,
            count: 0,
            compact,
            initial: "←",
        },
    );
    if back.on_hover_text("This project's board").clicked() {
        cx.dispatch(AppAction::ShowBoard(pid));
    }
    // Agents and shells first, then the project's commands and services
    // under their own label, the way the board keeps them apart.
    let sessions: Vec<(RecordId, String)> = cx
        .core
        .board_sessions(pid)
        .into_iter()
        .map(|s| (s.id, s.name.clone()))
        .collect();
    let entries: Vec<(RecordId, String)> = cx
        .core
        .run_entries(pid)
        .into_iter()
        .map(|s| (s.id, s.name.clone()))
        .collect();
    for (sid, name) in sessions {
        entry_row(cx, ui, sid, &name, id, compact);
    }
    if !entries.is_empty() {
        ui.add_space(12.0);
        if !compact {
            theme::kicker(ui, "Commands and services", p.n600);
            ui.add_space(4.0);
        }
        for (sid, name) in entries {
            entry_row(cx, ui, sid, &name, id, compact);
        }
    }
}

/// One of the project's entries as a rail row; `current` is selected.
fn entry_row(
    cx: &mut DrawCtx<'_>,
    ui: &mut Ui,
    sid: RecordId,
    name: &str,
    current: RecordId,
    compact: bool,
) {
    let state = cx.core.card_state(sid);
    let running = super::cards::is_running(cx.core, sid);
    let response = row(
        ui,
        &RowSpec {
            text: name,
            dot: Some(state),
            selected: sid == current,
            muted: !running,
            count: 0,
            compact,
            initial: &initial(name),
        },
    );
    if response.on_hover_text(name).clicked() && sid != current {
        cx.dispatch(AppAction::ShowSession(sid));
    }
}

fn bottom(cx: &mut DrawCtx<'_>, ui: &mut Ui, view: &View, compact: bool) {
    let p = theme::palette(ui);
    ui.spacing_mut().item_spacing.y = 4.0;
    let settings = cx.core.settings().clone();
    // Bottom-up layout: the first item drawn is the lowest.
    if cx.core.read_only() {
        ui.label(
            RichText::new("read-only")
                .text_style(theme::meta())
                .color(p.accent_2_text),
        )
        .on_hover_text("Another Switchboard holds the store lock; changes are not saved");
    }
    if settings.exclusive {
        ui.label(
            RichText::new("exclusive")
                .text_style(theme::meta())
                .color(p.n600),
        )
        .on_hover_text("Other projects are hidden (Settings)");
    }
    super::switcher::settings_menu(cx, ui, &settings, compact);
    if bottom_item(ui, "Go to", "⌘K", compact)
        .on_hover_text("Find a project or session (Cmd+K)")
        .clicked()
    {
        cx.state.palette = Some(super::palette::PaletteDraft::default());
    }
    if matches!(view, View::Session(_)) {
        if bottom_item(ui, "Terminal", "⌘T", compact)
            .on_hover_text("Show the raw pane under the conversation (Cmd+T)")
            .clicked()
        {
            cx.state.terminal_open = !cx.state.terminal_open;
        }
        if bottom_item(ui, "Files", "⌘B", compact)
            .on_hover_text("Show the project's files beside the session (Cmd+B)")
            .clicked()
        {
            cx.dispatch(AppAction::SetFilesOpen(!settings.files_open));
        }
        let notes_showing = settings.files_open && settings.side_tab == SideTab::Notes;
        if bottom_item(ui, "Notes", "⌘N", compact)
            .on_hover_text("Show the session's notes beside it (Cmd+N)")
            .clicked()
        {
            if notes_showing {
                cx.dispatch(AppAction::SetFilesOpen(false));
            } else {
                cx.dispatch(AppAction::SetSideTab(SideTab::Notes));
                cx.dispatch(AppAction::SetFilesOpen(true));
            }
        }
    }
}

/// A pinned bottom row: label in neutral-700 with its shortcut in mono
/// neutral-500 after it.
pub fn bottom_item(ui: &mut Ui, label: &str, shortcut: &str, compact: bool) -> Response {
    let p = theme::palette(ui);
    let text = if compact {
        RichText::new(shortcut)
            .text_style(TextStyle::Monospace)
            .color(p.n500)
    } else {
        RichText::new(label).text_style(theme::meta()).color(p.n700)
    };
    let mut button = egui::Button::new(text).frame_when_inactive(false);
    if !compact {
        button = button.right_text(
            RichText::new(shortcut)
                .text_style(TextStyle::Monospace)
                .color(p.n500),
        );
    }
    ui.add(button)
}

/// First letter of a name, uppercased: the compact rail's label.
fn initial(name: &str) -> String {
    name.chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default()
}

struct RowSpec<'a> {
    text: &'a str,
    /// The state dot before the text, or nothing for plain rows.
    dot: Option<CardState>,
    selected: bool,
    /// Nothing runs here: the name in neutral-700.
    muted: bool,
    /// Sessions waiting on you: a magenta count at the right.
    count: usize,
    compact: bool,
    /// What stands for the text when compact.
    initial: &'a str,
}

/// One row of the rail: a full-width click target that paints its own
/// dot, text, and count, so the selected row can carry the paper fill
/// and semibold the design asks for. It reports itself as a button so
/// tests and screen readers find it by its text.
fn row(ui: &mut Ui, spec: &RowSpec<'_>) -> Response {
    let p = theme::palette(ui);
    let width = ui.available_width();
    let height = if spec.compact { 44.0 } else { 28.0 };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let text = spec.text.to_owned();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &text));
    let painter = ui.painter();
    if spec.selected {
        painter.rect_filled(rect, 2.0, p.bg);
    } else if response.hovered() {
        painter.rect_filled(rect, 2.0, p.text_alpha(0.07));
    }
    let color = if spec.selected {
        p.accent_text
    } else if spec.muted {
        p.n700
    } else {
        p.text
    };
    let font = if spec.selected {
        FontId::new(14.0, theme::bold())
    } else {
        FontId::new(14.0, egui::FontFamily::Proportional)
    };
    if spec.compact {
        let mut y = rect.center().y;
        if let Some(state) = &spec.dot {
            dot(
                painter,
                egui::pos2(rect.center().x, rect.top() + 12.0),
                7.0,
                state,
                p,
            );
            y = rect.top() + 30.0;
        }
        painter.text(
            egui::pos2(rect.center().x, y),
            Align2::CENTER_CENTER,
            spec.initial,
            font,
            color,
        );
        if spec.count > 0 {
            painter.circle_filled(
                egui::pos2(rect.right() - 6.0, rect.top() + 6.0),
                3.0,
                p.accent_2,
            );
        }
        return response;
    }
    let mut x = rect.left() + 8.0;
    if let Some(state) = &spec.dot {
        dot(painter, egui::pos2(x + 3.5, rect.center().y), 7.0, state, p);
        x += 16.0;
    }
    painter.text(
        egui::pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        spec.text,
        font,
        color,
    );
    if spec.count > 0 {
        painter.text(
            egui::pos2(rect.right() - 8.0, rect.center().y),
            Align2::RIGHT_CENTER,
            spec.count.to_string(),
            FontId::new(12.0, egui::FontFamily::Proportional),
            p.accent_2_text,
        );
    }
    response
}

fn dot(
    painter: &egui::Painter,
    center: egui::Pos2,
    size: f32,
    state: &CardState,
    p: &theme::Palette,
) {
    match p.dot_fill(state) {
        Some(fill) => painter.circle_filled(center, size / 2.0, fill),
        None => painter.circle_stroke(center, size / 2.0 - 0.5, egui::Stroke::new(1.0, p.n500)),
    };
}
