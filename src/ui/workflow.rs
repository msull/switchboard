//! Plan review: the dialog that starts one from a session, and the run's
//! page. The page reads the round files itself (the plan, feedback, and
//! response, live for the round in progress and from the run's snapshot
//! directory for finished ones) the way the document preview does; every
//! control is an action for the core.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use egui::{Context, RichText, Ui};

use super::dialogs::{dialog, dialog_actions, field};
use super::{DrawCtx, GAP, UiState, document, theme};
use crate::core::{
    AppAction, BUILTIN_WORKFLOW, HandoffMode, RecordId, Round, RunState, Verdict, WorkflowId,
    WorkflowRun, snapshot_dir,
};
use crate::ports::transcript::Conversation;

/// The dialog's draft while it is open.
#[derive(Debug, Clone)]
pub struct ReviewDraft {
    pub source: RecordId,
    pub plan: String,
    /// Markdown files the session wrote, newest first, as offered.
    pub candidates: Vec<PathBuf>,
    pub definition: String,
}

impl ReviewDraft {
    /// Offer the files the session's transcript shows it wrote; the
    /// newest is the starting value.
    #[must_use]
    pub fn open(cx: &DrawCtx<'_>, source: RecordId) -> Self {
        let candidates = cx
            .state
            .conversations
            .get(&source)
            .map(|(_, c)| written_markdown(c))
            .unwrap_or_default();
        let plan = candidates
            .first()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        Self {
            source,
            plan,
            candidates,
            definition: BUILTIN_WORKFLOW.into(),
        }
    }
}

/// Tools whose input names the file they change.
const WRITING_TOOLS: [&str; 4] = ["Write", "Edit", "MultiEdit", "NotebookEdit"];

/// Markdown files the conversation's file-writing tool calls name,
/// newest first, each once.
#[must_use]
pub fn written_markdown(conversation: &Conversation) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for turn in conversation.turns.iter().rev() {
        for activity in turn.activity.iter().rev() {
            let Some(detail) = &activity.detail else {
                continue;
            };
            if !WRITING_TOOLS.contains(&detail.name.as_str()) {
                continue;
            }
            let Some(path) = detail
                .path
                .clone()
                .or_else(|| file_path_in(&detail.input).map(PathBuf::from))
            else {
                continue;
            };
            let markdown = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("md"));
            if markdown && !out.contains(&path) {
                out.push(path);
            }
        }
    }
    out
}

/// The `file_path` of a tool's input. The input is JSON, but capped, so
/// a scan for the key stands in when it no longer parses.
fn file_path_in(input: &str) -> Option<String> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(input)
        && let Some(p) = v.get("file_path").and_then(|p| p.as_str())
    {
        return Some(p.to_owned());
    }
    let key = "\"file_path\":";
    let start = input.find(key)? + key.len();
    let rest = input[start..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
}

/// The typed path as an absolute one: `~` is the home directory and a
/// relative path is under the session's directory. `None` for blank.
#[must_use]
pub fn resolve_plan(typed: &str, cwd: &Path) -> Option<PathBuf> {
    if typed.is_empty() {
        return None;
    }
    let path = if let Some(rest) = typed.strip_prefix("~/") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest))?
    } else if typed == "~" {
        return None;
    } else {
        PathBuf::from(typed)
    };
    Some(if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    })
}

/// The dialog, while a draft exists.
pub fn dialog_show(cx: &mut DrawCtx<'_>, ctx: &Context) {
    let Some(mut draft) = cx.state.review_dialog.take() else {
        return;
    };
    let Some(source) = cx.core.session(draft.source).cloned() else {
        return;
    };
    let names: Vec<String> = std::iter::once(BUILTIN_WORKFLOW.to_owned())
        .chain(
            cx.core
                .settings()
                .workflows
                .iter()
                .map(|d| d.name.clone())
                .filter(|n| n != BUILTIN_WORKFLOW),
        )
        .collect();
    let cap = cx
        .core
        .settings()
        .workflow(&draft.definition)
        .and_then(|d| d.cap)
        .unwrap_or(cx.core.settings().workflow_round_cap);
    let mut keep = true;
    dialog(ctx, "Review a plan", |ui| {
        let p = theme::palette(ui);
        ui.label(theme::meta_text(
            ui,
            format!(
                "A fresh reviewer critiques the plan; a clone of {} answers each round. \
             {} itself is left as it is for the handoff.",
                source.name, source.name
            ),
        ));
        if !draft.candidates.is_empty() {
            ui.label(
                RichText::new("Files this session wrote")
                    .text_style(theme::meta())
                    .color(p.n600),
            );
            for candidate in draft.candidates.clone() {
                let shown = candidate
                    .strip_prefix(&source.cwd)
                    .unwrap_or(&candidate)
                    .display()
                    .to_string();
                if theme::ghost(ui, &shown).clicked() {
                    draft.plan = candidate.display().to_string();
                }
            }
        }
        field(ui, "Plan path", &mut draft.plan);
        ui.label(theme::meta_text(
            ui,
            format!("Absolute, or relative to {}", source.cwd.display()),
        ));
        if names.len() > 1 {
            ui.horizontal_wrapped(|ui| {
                for name in &names {
                    ui.radio_value(&mut draft.definition, name.clone(), name);
                }
            });
        }
        ui.label(theme::meta_text(
            ui,
            format!("Up to {cap} rounds before it stops to ask (Settings)"),
        ));
        let plan = resolve_plan(draft.plan.trim(), &source.cwd);
        let (confirmed, cancelled) = dialog_actions(ui, "Start review", plan.is_some());
        if confirmed && let Some(plan) = plan {
            cx.dispatch(AppAction::StartWorkflow {
                source: draft.source,
                plan,
                definition: draft.definition.clone(),
            });
            keep = false;
        }
        if cancelled {
            keep = false;
        }
    });
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        keep = false;
    }
    if keep {
        cx.state.review_dialog = Some(draft);
    }
}

/// What the page remembers per run between frames.
#[derive(Debug, Clone, Default)]
pub struct ReviewView {
    /// The round shown; `None` is the latest.
    pub round: Option<u32>,
    pub diff: bool,
    pub note: String,
}

/// Width of the round list at the left.
const ROUNDS_WIDTH: f32 = 150.0;
/// What a section label takes: the space above it plus its line.
const SECTION_HEIGHT: f32 = 48.0;
/// The note box with its button.
const NOTE_HEIGHT: f32 = 110.0;

pub fn show(cx: &mut DrawCtx<'_>, ui: &mut Ui, id: WorkflowId) {
    let Some(run) = cx.core.workflow(id).cloned() else {
        ui.label("This review no longer exists.");
        return;
    };
    ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
    header(cx, ui, &run);
    confirm_cleanup(cx, ui.ctx(), &run);
    let Some(round) = selected_round(cx.state, &run).cloned() else {
        ui.label(theme::meta_text(ui, "No rounds yet."));
        return;
    };
    let data_dir = cx.services.store.data_dir();
    let (plan, feedback, response) = round_files(&data_dir, &run, &round);
    let previous = run
        .rounds
        .iter()
        .find(|r| r.n + 1 == round.n)
        .map(|prev| round_files(&data_dir, &run, prev).0);
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(ROUNDS_WIDTH, ui.available_height()),
            egui::Layout::top_down(egui::Align::Min),
            |ui| rounds_list(cx, ui, &run),
        );
        let rest = ui.available_width() - GAP;
        ui.allocate_ui_with_layout(
            egui::vec2(rest * 0.5, ui.available_height()),
            egui::Layout::top_down(egui::Align::Min),
            |ui| plan_column(cx, ui, &run, &round, &plan, previous.as_deref()),
        );
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), ui.available_height()),
            egui::Layout::top_down(egui::Align::Min),
            |ui| exchange_column(cx, ui, &run, &round, &feedback, &response),
        );
    });
}

/// The files of a round: live for the round in progress, from the
/// snapshot once the round was copied there.
fn round_files(data_dir: &Path, run: &WorkflowRun, round: &Round) -> (PathBuf, PathBuf, PathBuf) {
    if !round.snapshot {
        return (
            run.plan.clone(),
            round.feedback.clone(),
            round.response.clone(),
        );
    }
    let dir = snapshot_dir(data_dir, run.id, round.n);
    let name = |p: &Path| dir.join(p.file_name().unwrap_or_default());
    (
        name(&run.plan),
        name(&round.feedback),
        name(&round.response),
    )
}

fn selected_round<'a>(state: &UiState, run: &'a WorkflowRun) -> Option<&'a Round> {
    let chosen = state.review_views.get(&run.id).and_then(|v| v.round);
    match chosen {
        Some(n) => run.rounds.iter().find(|r| r.n == n).or(run.rounds.last()),
        None => run.rounds.last(),
    }
}

fn header(cx: &mut DrawCtx<'_>, ui: &mut Ui, run: &WorkflowRun) {
    let p = theme::palette(ui);
    theme::kicker(ui, "Plan review", p.n600);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            controls(cx, ui, run);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let name = run
                    .plan
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("plan");
                ui.add(egui::Label::new(RichText::new(name).text_style(theme::h1())).truncate());
                ui.label(
                    RichText::new(run.state.label())
                        .text_style(theme::meta())
                        .color(match run.state {
                            RunState::Paused(_) => p.accent_2_text,
                            _ => p.n700,
                        }),
                );
            });
        });
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(theme::mono_text(ui, run.plan.display().to_string()));
        ui.label(theme::meta_text(ui, "·"));
        ui.label(theme::meta_text(
            ui,
            format!("{} of up to {} rounds", run.rounds.len(), run.cap),
        ));
        for (label, id) in [
            ("Planning session", Some(run.source)),
            ("Reviewer", Some(run.reviewer)),
            ("Planner clone", run.planner),
        ] {
            let Some(id) = id else {
                continue;
            };
            let Some(name) = cx.core.session(id).map(|s| s.name.clone()) else {
                continue;
            };
            ui.label(theme::meta_text(ui, "·"));
            if theme::ghost_muted(ui, &name).on_hover_text(label).clicked() {
                cx.dispatch(AppAction::ShowSession(id));
            }
        }
    });
    ui.add_space(4.0);
}

/// The actions the run's state allows, drawn right to left.
fn controls(cx: &mut DrawCtx<'_>, ui: &mut Ui, run: &WorkflowRun) {
    ui.spacing_mut().item_spacing.x = 2.0;
    let waiting = run.state.waiting();
    if theme::ghost_muted(ui, "Back").clicked() {
        cx.dispatch(AppAction::Back);
    }
    if !waiting
        && theme::ghost_muted(ui, "Remove review")
            .on_hover_text("Forget this run; its sessions and files stay")
            .clicked()
    {
        cx.dispatch(AppAction::RemoveWorkflow(run.id));
    }
    if !waiting
        && !run.cleaned
        && !run.rounds.is_empty()
        && theme::ghost(ui, "Clean up")
            .on_hover_text("Delete the feedback and response files from the project")
            .clicked()
    {
        cx.state.confirm_cleanup = Some(run.id);
    }
    match run.state {
        RunState::AwaitingFeedback | RunState::AwaitingResponse | RunState::Starting => {
            if theme::secondary(ui, "Pause").clicked() {
                cx.dispatch(AppAction::PauseWorkflow(run.id));
            }
        }
        RunState::Paused(_) => {
            if theme::primary(ui, "Continue").clicked() {
                cx.dispatch(AppAction::ContinueWorkflow(run.id));
            }
            if theme::secondary(ui, "Finalize").clicked() {
                cx.dispatch(AppAction::FinalizeWorkflow(run.id));
            }
        }
        RunState::AtCap => {
            if theme::primary(ui, "One more round").clicked() {
                cx.dispatch(AppAction::ContinueWorkflow(run.id));
            }
            if theme::secondary(ui, "Raise cap by 2").clicked() {
                cx.dispatch(AppAction::RaiseWorkflowCap {
                    run: run.id,
                    cap: run.cap + 2,
                });
            }
            if theme::secondary(ui, "Finalize").clicked() {
                cx.dispatch(AppAction::FinalizeWorkflow(run.id));
            }
        }
        RunState::Converged => {
            if theme::primary(ui, "Finalize")
                .on_hover_text("You have reviewed the plan; next comes the handoff")
                .clicked()
            {
                cx.dispatch(AppAction::FinalizeWorkflow(run.id));
            }
            if theme::secondary(ui, "One more round").clicked() {
                cx.dispatch(AppAction::ContinueWorkflow(run.id));
            }
        }
        RunState::Finalized | RunState::HandedOff => hand_off_menu(cx, ui, run),
    }
}

fn hand_off_menu(cx: &mut DrawCtx<'_>, ui: &mut Ui, run: &WorkflowRun) {
    let response = theme::primary(ui, "Hand off")
        .on_hover_text("Send the revised plan back to the planning session");
    egui::Popup::menu(&response).show(|ui| {
        for (label, mode, hint) in [
            (
                "As is",
                HandoffMode::AsIs,
                "Send the handoff prompt to the planning session",
            ),
            (
                "Compact first",
                HandoffMode::Compact,
                "Send /compact, then the prompt (the session must be running)",
            ),
            (
                "Fresh session",
                HandoffMode::Fresh,
                "A new session in the same directory, with the prompt",
            ),
        ] {
            if ui.button(label).on_hover_text(hint).clicked() {
                cx.dispatch(AppAction::HandOffWorkflow { run: run.id, mode });
                ui.close();
            }
        }
    });
}

/// The cleanup confirmation lists exactly what will be deleted.
fn confirm_cleanup(cx: &mut DrawCtx<'_>, ctx: &Context, run: &WorkflowRun) {
    if cx.state.confirm_cleanup != Some(run.id) {
        return;
    }
    let mut done = false;
    dialog(ctx, "Delete the round files", |ui| {
        ui.label("These files are deleted from the project. The copies under the review stay.");
        for file in run.round_files() {
            ui.label(theme::mono_text(ui, file.display().to_string()));
        }
        let (confirmed, cancelled) = dialog_actions(ui, "Delete files", true);
        if confirmed {
            cx.dispatch(AppAction::CleanUpWorkflow(run.id));
            done = true;
        }
        if cancelled {
            done = true;
        }
    });
    if done || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        cx.state.confirm_cleanup = None;
    }
}

fn rounds_list(cx: &mut DrawCtx<'_>, ui: &mut Ui, run: &WorkflowRun) {
    let p = theme::palette(ui);
    theme::section(ui, "Rounds");
    let selected = selected_round(cx.state, run).map(|r| r.n);
    for round in &run.rounds {
        let verdict = match (round.user_feedback.is_some(), round.verdict) {
            (true, _) => "your feedback",
            (false, Some(Verdict::Nothing)) => "nothing further",
            (false, Some(Verdict::Changes)) if round.responded => "answered",
            (false, Some(Verdict::Changes)) => "changes asked",
            (false, None) => "reviewing",
        };
        let label = format!("Round {}", round.n);
        let is_selected = selected == Some(round.n);
        let text = if is_selected {
            RichText::new(&label).text_style(theme::strong())
        } else {
            RichText::new(&label)
        };
        if ui.add(egui::Button::new(text).frame(false)).clicked() {
            cx.state.review_views.entry(run.id).or_default().round = Some(round.n);
        }
        ui.label(
            RichText::new(verdict)
                .text_style(theme::meta())
                .color(p.n600),
        );
        ui.add_space(4.0);
    }
}

fn plan_column(
    cx: &mut DrawCtx<'_>,
    ui: &mut Ui,
    run: &WorkflowRun,
    round: &Round,
    plan: &Path,
    previous: Option<&Path>,
) {
    theme::section(ui, "Plan");
    if previous.is_some() {
        let view = cx.state.review_views.entry(run.id).or_default();
        ui.checkbox(&mut view.diff, "Diff with previous round");
    }
    let diff = cx.state.review_views.get(&run.id).is_some_and(|v| v.diff);
    let height = ui.available_height() - 4.0;
    egui::ScrollArea::vertical()
        .id_salt(("plan", run.id, round.n))
        .max_height(height)
        .auto_shrink(false)
        .show(ui, |ui| match (diff, previous) {
            (true, Some(previous)) => diff_view(cx.state, ui, previous, plan),
            _ => file_body(cx.state, ui, plan),
        });
}

fn exchange_column(
    cx: &mut DrawCtx<'_>,
    ui: &mut Ui,
    run: &WorkflowRun,
    round: &Round,
    feedback: &Path,
    response: &Path,
) {
    let note = !run.state.waiting() && run.planner.is_some();
    // Two section labels, and the note box when it is shown, come out
    // of the height before it is split between the two panes.
    let reserve = 2.0 * SECTION_HEIGHT + if note { NOTE_HEIGHT } else { 0.0 };
    let half = ((ui.available_height() - reserve) * 0.5).max(80.0);
    theme::section(ui, "Feedback");
    egui::ScrollArea::vertical()
        .id_salt(("feedback", run.id, round.n))
        .max_height(half)
        .auto_shrink(false)
        .show(ui, |ui| match &round.user_feedback {
            Some(text) => {
                document::markdown_style(ui);
                super::markdown::show(ui, &mut cx.state.markdown, text);
            }
            None => file_body(cx.state, ui, feedback),
        });
    theme::section(ui, "Response");
    egui::ScrollArea::vertical()
        .id_salt(("response", run.id, round.n))
        .max_height(half)
        .auto_shrink(false)
        .show(ui, |ui| {
            if round.verdict == Some(Verdict::Nothing) {
                ui.label(theme::meta_text(ui, "Nothing to answer."));
            } else {
                file_body(cx.state, ui, response);
            }
        });
    if note {
        note_box(cx, ui, run);
    }
}

/// The user's own feedback, sent as one more round for the planner.
fn note_box(cx: &mut DrawCtx<'_>, ui: &mut Ui, run: &WorkflowRun) {
    let p = theme::palette(ui);
    let view = cx.state.review_views.entry(run.id).or_default();
    let response = ui.add(
        egui::TextEdit::multiline(&mut view.note)
            .hint_text("Your own feedback for the planner")
            .desired_rows(2)
            .desired_width(f32::INFINITY)
            .background_color(p.surface)
            .margin(egui::Margin::symmetric(10, 8)),
    );
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_label("Your feedback"));
    let ready = !view.note.trim().is_empty();
    let text = view.note.clone();
    if ui
        .add_enabled_ui(ready, |ui| theme::secondary(ui, "Send my feedback"))
        .inner
        .clicked()
    {
        cx.dispatch(AppAction::UserFeedback { run: run.id, text });
        if let Some(view) = cx.state.review_views.get_mut(&run.id) {
            view.note.clear();
        }
    }
}

/// A file's body through the preview cache; a missing file is a note.
fn file_body(state: &mut UiState, ui: &mut Ui, path: &Path) {
    let slot = state.previews.entry(path.to_path_buf()).or_insert(None);
    document::ensure_in(slot, path);
    let UiState {
        previews, markdown, ..
    } = state;
    let Some(preview) = previews.get(path).and_then(Option::as_ref) else {
        return;
    };
    match &preview.body {
        document::Body::Missing(_) => {
            ui.label(theme::meta_text(ui, "Not written yet."));
        }
        _ => document::draw_body(preview, markdown, ui),
    }
}

/// One line of a diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Same,
    Added,
    Removed,
}

/// Lines above this on either side are shown without a diff; the
/// quadratic table would be too big.
const DIFF_LINE_CAP: usize = 3000;

/// A line diff of `old` against `new`: the longest common subsequence,
/// so a moved paragraph shows as removed and added, not garbled.
#[must_use]
pub fn diff_lines<'a>(old: &'a str, new: &'a str) -> Vec<(Change, &'a str)> {
    let olds: Vec<&str> = old.lines().collect();
    let news: Vec<&str> = new.lines().collect();
    if olds.len() > DIFF_LINE_CAP || news.len() > DIFF_LINE_CAP {
        return news.into_iter().map(|l| (Change::Same, l)).collect();
    }
    let (rows, cols) = (olds.len(), news.len());
    // lcs[i][j]: length of the LCS of olds[i..] and news[j..].
    let mut lcs = vec![vec![0u32; cols + 1]; rows + 1];
    for i in (0..rows).rev() {
        for j in (0..cols).rev() {
            lcs[i][j] = if olds[i] == news[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut out = Vec::with_capacity(rows.max(cols));
    let (mut i, mut j) = (0, 0);
    while i < rows && j < cols {
        if olds[i] == news[j] {
            out.push((Change::Same, news[j]));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            out.push((Change::Removed, olds[i]));
            i += 1;
        } else {
            out.push((Change::Added, news[j]));
            j += 1;
        }
    }
    out.extend(olds[i..].iter().map(|l| (Change::Removed, *l)));
    out.extend(news[j..].iter().map(|l| (Change::Added, *l)));
    out
}

fn diff_view(state: &mut UiState, ui: &mut Ui, previous: &Path, current: &Path) {
    for path in [previous, current] {
        let slot = state.previews.entry(path.to_path_buf()).or_insert(None);
        document::ensure_in(slot, path);
    }
    let text_of = |path: &Path| -> String {
        match state
            .previews
            .get(path)
            .and_then(Option::as_ref)
            .map(|p| &p.body)
        {
            Some(document::Body::Markdown(t) | document::Body::Text(t)) => t.clone(),
            _ => String::new(),
        }
    };
    let (old, new) = (text_of(previous), text_of(current));
    let p = theme::palette(ui);
    let changed = diff_lines(&old, &new)
        .iter()
        .filter(|(c, _)| *c != Change::Same)
        .count();
    if changed == 0 {
        ui.label(theme::meta_text(ui, "No changes from the previous round."));
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    for (change, line) in diff_lines(&old, &new) {
        let (prefix, color, fill) = match change {
            Change::Same => (" ", p.n700, None),
            Change::Added => ("+", p.text, Some(p.accent_2_text.gamma_multiply(0.18))),
            Change::Removed => ("-", p.n600, Some(p.accent_text.gamma_multiply(0.18))),
        };
        let text = RichText::new(format!("{prefix} {line}"))
            .monospace()
            .color(color);
        let response = ui.add(egui::Label::new(text).wrap());
        if let Some(fill) = fill {
            ui.painter().rect_filled(response.rect, 0.0, fill);
        }
    }
}

/// Per-run page state, keyed by run.
pub type ReviewViews = HashMap<WorkflowId, ReviewView>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_marks_added_and_removed_lines() {
        let d = diff_lines("a\nb\nc", "a\nc\nd");
        assert_eq!(
            d,
            vec![
                (Change::Same, "a"),
                (Change::Removed, "b"),
                (Change::Same, "c"),
                (Change::Added, "d"),
            ]
        );
        assert!(diff_lines("", "").is_empty());
    }

    #[test]
    fn file_path_is_read_from_json_or_scanned_from_a_cut_input() {
        assert_eq!(
            file_path_in("{\"file_path\": \"/p/plan.md\", \"content\": \"x\"}").as_deref(),
            Some("/p/plan.md")
        );
        assert_eq!(
            file_path_in("{\n  \"file_path\": \"/p/plan.md\",\n  \"content\": \"cut off")
                .as_deref(),
            Some("/p/plan.md")
        );
        assert_eq!(file_path_in("{\"command\": \"ls\"}"), None);
    }

    #[test]
    fn typed_plan_paths_resolve_against_the_session_directory() {
        let cwd = Path::new("/w/proj");
        assert_eq!(resolve_plan("", cwd), None);
        assert_eq!(
            resolve_plan("docs/plan.md", cwd),
            Some(PathBuf::from("/w/proj/docs/plan.md"))
        );
        assert_eq!(
            resolve_plan("/abs/plan.md", cwd),
            Some(PathBuf::from("/abs/plan.md"))
        );
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            resolve_plan("~/p.md", cwd),
            Some(PathBuf::from(home).join("p.md"))
        );
    }
}
