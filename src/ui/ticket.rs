//! One Dispatch ticket's page: its header and the decisions pending on
//! it, then tabs for its timeline, issue, plan with its review rounds,
//! notes, code review and branch changes. Everything shown came through
//! Dispatch's port as a view or was read from the ticket's tree on a
//! thread; a button is an action the core turns into one call.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use egui::{RichText, Ui};

use super::cards::{ago_ms, at_local, now_ms};
use super::dialogs::{dialog, dialog_actions};
use super::dispatch::{decision_card, title_of};
use super::{DrawCtx, GAP, markdown, theme};
use crate::core::dispatch::{TimelineRow, close_offered, parked, revisable};
use crate::core::{AppAction, RecordId};
use crate::ports::changes::Changes;
use crate::ports::dispatch::{
    AttemptView, DecisionView, LaneView, RewriteView, TicketView, nudged,
};

/// The ticket page's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum TicketTab {
    #[default]
    Timeline,
    Issue,
    Plan,
    Notes,
    Review,
    Changes,
}

impl TicketTab {
    /// Every tab, in the strip's order.
    pub const ALL: [Self; 6] = [
        Self::Timeline,
        Self::Issue,
        Self::Plan,
        Self::Notes,
        Self::Review,
        Self::Changes,
    ];

    /// The tab's name on its button and in a script line.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Timeline => "Timeline",
            Self::Issue => "Issue",
            Self::Plan => "Plan",
            Self::Notes => "Notes",
            Self::Review => "Review",
            Self::Changes => "Changes",
        }
    }

    /// The tab a script line names, by its label in any case.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|t| t.label().eq_ignore_ascii_case(word))
    }
}

/// A lane's changes as read on a thread.
#[derive(Debug, Default)]
pub struct ChangesScan {
    /// What the last read was started for: the ticket's `updated_ms`,
    /// and the directory, base and head it read.
    pub key: Option<(u64, PathBuf, String, String)>,
    /// The read on its way.
    pub rx: Option<Receiver<Result<Changes, String>>>,
    /// The last answer.
    pub last: Option<Result<Changes, String>>,
}

/// One ticket: the header, its pending decisions pinned on every tab,
/// the tab strip, and the chosen tab's body.
pub fn ticket(cx: &mut DrawCtx<'_>, ui: &mut Ui, id: &str) {
    let p = theme::palette(ui);
    ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
    let Some(t) = cx.core.ticket(id).cloned() else {
        theme::kicker(ui, "Dispatch ticket", p.n600);
        ui.label("This ticket is not in Dispatch's last status.");
        if theme::ghost(ui, "Back").clicked() {
            go_back(cx);
        }
        return;
    };
    ask_for_reads(cx, &t);
    ticket_header(cx, ui, &t);
    confirm_close(cx, ui.ctx(), &t);
    for d in t.decisions.iter().filter(|d| d.state == "pending") {
        decision_card(cx, ui, d, Some(&t), false);
    }
    let tab = tab_strip(cx, ui, &t.id);
    if tab == TicketTab::Plan
        && let Some(d) = revisable(&t)
    {
        plan_feedback(cx, ui, d);
    } else {
        // A request for a box not drawn this frame has lost its moment;
        // left set, it would take the focus whenever the tab next opens.
        cx.state.focus_feedback = None;
    }
    let now = now_ms();
    // The header stays put so the pending decisions and the tabs are
    // always in view. Its meta row wraps by whole items, so the header
    // alone stays short; several long pending decisions under it can
    // still push the tabs down on a short window.
    egui::ScrollArea::vertical()
        .id_salt(("ticket-tab", tab))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            match tab {
                TicketTab::Timeline => timeline(cx, ui, &t, now),
                TicketTab::Issue => issue(cx, ui, &t),
                TicketTab::Plan => plan(cx, ui, &t),
                TicketTab::Notes => notes(cx, ui, &t, now),
                TicketTab::Review => review(cx, ui, &t),
                TicketTab::Changes => changes(cx, ui, &t, now),
            }
        });
}

/// The id of the box that takes the owner's objection to a plan, so the
/// card's "Revise…" can focus it.
pub(super) fn finalize_note_id(d: &DecisionView) -> egui::Id {
    egui::Id::new(("finalize-note", &d.id))
}

/// The owner's objection to the plan under review, pinned above the
/// plan while its `finalize` is pending; sending it answers `revise`.
/// The draft is the card's, one per decision.
fn plan_feedback(cx: &mut DrawCtx<'_>, ui: &mut Ui, d: &DecisionView) {
    let meta = cx
        .core
        .ticket_details(&d.ticket)
        .map_or(String::new(), |details| {
            match details.paths.plan_rounds.len() {
                1 => "1 round so far".to_owned(),
                n => format!("{n} rounds so far"),
            }
        });
    let mut send = None;
    theme::surface(ui)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::section(ui, "Your feedback on the plan");
            let field = finalize_note_id(d);
            // Asked until held; see `UiState::focus_feedback`.
            if cx.state.focus_feedback == Some(field) {
                if ui.memory(|m| m.has_focus(field)) {
                    cx.state.focus_feedback = None;
                } else {
                    ui.memory_mut(|m| m.request_focus(field));
                }
            }
            let draft = cx
                .state
                .dispatch_note_drafts
                .entry(d.id.clone())
                .or_default();
            let hint = "What the planner should change; Dispatch sends it as the next round";
            if super::workflow::feedback_block(ui, draft, field, hint, &meta) {
                send = Some(draft.trim().to_owned());
            }
        });
    if let Some(note) = send {
        cx.state.dispatch_note_drafts.remove(&d.id);
        cx.dispatch(AppAction::DispatchDecide {
            ticket: d.ticket.clone(),
            decision: d.id.clone(),
            answer: "revise".into(),
            note: Some(note),
        });
    }
}

/// The ticket's events and its full view, asked once per change of its
/// `updated_ms`: the core says when a read is due, and a read the core
/// would drop is not dispatched at all.
fn ask_for_reads(cx: &mut DrawCtx<'_>, t: &TicketView) {
    if cx.core.events_read_due(&t.id, t.updated_ms) {
        cx.dispatch(AppAction::DispatchReadEvents {
            ticket: t.id.clone(),
            updated_ms: t.updated_ms,
        });
    }
    if cx.core.ticket_read_due(&t.id, t.updated_ms) {
        cx.dispatch(AppAction::DispatchReadTicket {
            id: t.id.clone(),
            updated_ms: t.updated_ms,
        });
    }
}

/// The tabs as buttons, the chosen one filled. A click only changes the
/// page's own state.
fn tab_strip(cx: &mut DrawCtx<'_>, ui: &mut Ui, id: &str) -> TicketTab {
    let chosen = cx
        .state
        .dispatch_ticket_tabs
        .get(id)
        .copied()
        .unwrap_or_default();
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for tab in TicketTab::ALL {
            let clicked = if tab == chosen {
                theme::primary(ui, tab.label())
            } else {
                theme::secondary(ui, tab.label())
            }
            .clicked();
            if clicked {
                cx.state.dispatch_ticket_tabs.insert(id.to_owned(), tab);
            }
        }
    });
    ui.separator();
    chosen
}

/// What the runner being away means for a body that has nothing cached.
fn not_running(cx: &DrawCtx<'_>, ui: &mut Ui) -> bool {
    if cx.core.dispatch_state().connected {
        return false;
    }
    ui.label(theme::meta_text(ui, "Dispatch is not running."));
    true
}

/// A file of the ticket's as markdown, read through the port when the
/// core says a read is due: the first time it is drawn and again
/// whenever the ticket changes. The text already read stays drawn while
/// it is read again.
fn artifact_text(cx: &mut DrawCtx<'_>, ui: &mut Ui, t: &TicketView, path: &Path) {
    if cx.core.artifact_read_due(t, path) {
        cx.dispatch(AppAction::DispatchReadArtifact {
            ticket: t.id.clone(),
            path: path.to_path_buf(),
        });
    }
    let text = cx.core.dispatch_state().artifacts.get(path).cloned();
    match text {
        Some(text) if text.trim().is_empty() => {
            ui.label(theme::meta_text(ui, "Empty."));
        }
        Some(text) => markdown::show(ui, &mut cx.state.markdown, &text),
        None if not_running(cx, ui) => {}
        None => {
            let said = match cx.core.artifact_read(path) {
                Some(r) if r.missing() => "Not written yet.".to_owned(),
                Some(r) => r
                    .failed
                    .as_ref()
                    .map_or_else(|| "Reading…".to_owned(), |why| format!("Not read: {why}")),
                None => "Reading…".to_owned(),
            };
            ui.label(theme::meta_text(ui, said));
        }
    }
}

/// A fold that reads its body only once opened, headed by `title`.
fn fold(ui: &mut Ui, id: egui::Id, title: &str, open: bool, body: impl FnOnce(&mut Ui)) {
    let st = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open);
    st.show_header(ui, |ui| {
        ui.label(theme::strong_text(title));
    })
    .body(|ui| body(ui));
}

/// "3 min", "1 h 5 min": how long an attempt ran.
fn duration_text(ms: u64) -> String {
    let mins = ms / 60_000;
    match mins {
        0 => "under a minute".to_owned(),
        m if m < 60 => format!("{m} min"),
        m if m % 60 == 0 => format!("{} h", m / 60),
        m => format!("{} h {} min", m / 60, m % 60),
    }
}

/// The time a row happened, relative, with the absolute local time on
/// hover.
fn when(ui: &mut Ui, at_ms: u64, now: u64) {
    let text = ago_ms(at_ms, now);
    if text.is_empty() {
        return;
    }
    ui.label(theme::meta_text(ui, text))
        .on_hover_text(at_local(at_ms));
}

/// Groups of rows under their stage, newest first, each attempt's card
/// under the row the core chose for it; the attempts no row carries
/// close the page.
fn timeline(cx: &mut DrawCtx<'_>, ui: &mut Ui, t: &TicketView, now: u64) {
    let timeline = cx.core.ticket_timeline(t);
    if timeline.groups.is_empty() && timeline.earlier.is_empty() {
        ui.label(theme::meta_text(ui, "Nothing has happened yet."));
        return;
    }
    let attempt = |key: &(String, u32)| {
        t.attempts
            .iter()
            .find(|a| (&a.stage, a.n) == (&key.0, key.1))
    };
    for g in &timeline.groups {
        theme::section(ui, &g.stage);
        for row in &g.rows {
            timeline_row(ui, t, row, now);
            if let Some(a) = row.details.as_ref().and_then(attempt) {
                attempt_row(cx, ui, t, a);
            }
        }
    }
    if !timeline.earlier.is_empty() {
        theme::section(ui, "Earlier attempts");
        for a in timeline.earlier.iter().filter_map(attempt) {
            attempt_row(cx, ui, t, a);
        }
    }
}

/// One timeline row: when, what kind, and its text; a decision's row
/// shows the decision itself rather than the log's capped line.
fn timeline_row(ui: &mut Ui, t: &TicketView, row: &TimelineRow, now: u64) {
    let p = theme::palette(ui);
    let decision: Option<&DecisionView> = row
        .decision
        .as_ref()
        .and_then(|id| t.decisions.iter().find(|d| &d.id == id));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        when(ui, row.at_ms, now);
        ui.label(
            RichText::new(&row.kind)
                .text_style(theme::meta())
                .color(p.n700),
        );
        match (row.kind.as_str(), decision) {
            ("decision", Some(d)) if d.state == "pending" => {
                ui.label(theme::meta_text(
                    ui,
                    format!("{}: waiting on you, above", d.name),
                ));
            }
            ("decision", Some(d)) => {
                ui.add(egui::Label::new(&d.question).wrap());
                if let Some(answer) = &d.answer {
                    let by = d
                        .answered_by
                        .as_ref()
                        .map_or(String::new(), |by| format!(" by {by}"));
                    let note = d.note.as_ref().map_or(String::new(), |n| format!(" — {n}"));
                    ui.label(theme::meta_text(ui, format!("→ {answer}{by}{note}")));
                }
            }
            _ => {
                ui.add(egui::Label::new(&row.text).wrap());
            }
        }
        if let Some(ms) = row.duration_ms {
            ui.label(theme::meta_text(ui, format!("ran {}", duration_text(ms))));
        }
    });
}

/// The issue as it was taken: title linked out, labels, body.
fn issue(cx: &mut DrawCtx<'_>, ui: &mut Ui, t: &TicketView) {
    let p = theme::palette(ui);
    ui.horizontal_wrapped(|ui| {
        ui.label(theme::strong_text(title_of(t)));
        if let Some(url) = &t.url {
            ui.hyperlink_to(RichText::new("Open ↗").color(p.accent_text), url);
        }
    });
    if !t.labels.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for label in &t.labels {
                chip(ui, label, p.text);
            }
        });
    }
    if t.body.trim().is_empty() {
        ui.label(theme::meta_text(ui, "No description."));
    } else {
        markdown::show(ui, &mut cx.state.markdown, &t.body);
    }
}

/// The ticket read in full, or what to say while it is not.
fn details<'a>(cx: &DrawCtx<'a>, ui: &mut Ui, t: &TicketView) -> Option<&'a TicketView> {
    let core = cx.core;
    if let Some(d) = core.ticket_details(&t.id) {
        return Some(d);
    }
    if !not_running(cx, ui) {
        ui.label(theme::meta_text(ui, "Reading…"));
    }
    None
}

/// Where the plan shown came from: a review's copy and its round, or
/// the stage that wrote it; nothing from a runner that does not say.
fn plan_label(p: &dispatch_control::PathsView) -> Option<String> {
    match (p.plan_reviewing, p.plan_reviewed, p.plan_round) {
        (true, _, Some(n)) => Some(format!("Reviewed copy, round {n}: review open")),
        (true, _, None) => Some("Reviewed copy: review open".to_owned()),
        (false, true, _) => Some("Reviewed copy".to_owned()),
        _ => p.plan_stage.as_ref().map(|s| format!("From stage {s}")),
    }
}

/// The plan, then each round of its review, folded.
fn plan(cx: &mut DrawCtx<'_>, ui: &mut Ui, t: &TicketView) {
    let Some(d) = details(cx, ui, t) else {
        return;
    };
    match &d.paths.plan {
        Some(path) => {
            ui.label(
                theme::mono_text(ui, path.display().to_string()).color(theme::palette(ui).n700),
            );
            if let Some(said) = plan_label(&d.paths) {
                ui.label(theme::meta_text(ui, said));
            }
            artifact_text(cx, ui, t, path);
        }
        None => {
            ui.label(theme::meta_text(ui, "No plan yet."));
        }
    }
    if d.paths.plan_rounds.is_empty() {
        return;
    }
    theme::section(ui, "Plan review");
    for round in &d.paths.plan_rounds {
        let id = ui.make_persistent_id(("plan-round", &t.id, round.n));
        let title = match round.by.as_deref() {
            None => format!("Round {}", round.n),
            Some("supervisor") => format!("Round {} · owner, via the supervisor", round.n),
            Some(_) => format!("Round {} · owner", round.n),
        };
        fold(ui, id, &title, false, |ui| {
            let what = if round.by.is_some() {
                "The owner's objection"
            } else {
                "Feedback"
            };
            ui.label(theme::meta_text(ui, what));
            artifact_text(cx, ui, t, &round.feedback);
            match &round.response {
                Some(response) => {
                    ui.label(theme::meta_text(ui, "Response"));
                    artifact_text(cx, ui, t, response);
                }
                None => {
                    ui.label(theme::meta_text(ui, "No response."));
                }
            }
        });
    }
}

/// Every attempt's notes, newest first, the newest open.
fn notes(cx: &mut DrawCtx<'_>, ui: &mut Ui, t: &TicketView, now: u64) {
    let with_notes: Vec<(&AttemptView, &PathBuf)> = t
        .attempts
        .iter()
        .rev()
        .filter_map(|a| {
            a.artifacts
                .iter()
                .find(|(name, _)| name == "notes")
                .map(|(_, path)| (a, path))
        })
        .collect();
    if with_notes.is_empty() {
        ui.label(theme::meta_text(ui, "No notes yet."));
        return;
    }
    for (i, (a, path)) in with_notes.into_iter().enumerate() {
        let ago = ago_ms(a.ended_ms.unwrap_or(a.started_ms), now);
        let title = if ago.is_empty() {
            format!("{} · {}", a.stage, a.context)
        } else {
            format!("{} · {} · {ago}", a.stage, a.context)
        };
        let id = ui.make_persistent_id(("notes", &t.id, &a.stage, a.n));
        fold(ui, id, &title, i == 0, |ui| {
            artifact_text(cx, ui, t, path);
        });
    }
}

/// Each code review: its rounds with their findings and responses, the
/// summary, and what it did to the branch's commits.
fn review(cx: &mut DrawCtx<'_>, ui: &mut Ui, t: &TicketView) {
    let reviews: Vec<&AttemptView> = t
        .attempts
        .iter()
        .rev()
        .filter(|a| a.kind == "review")
        .collect();
    if reviews.is_empty() {
        ui.label(theme::meta_text(ui, "No code review yet."));
        return;
    }
    let p = theme::palette(ui);
    for a in reviews {
        theme::section(ui, &format!("{} · {} · #{}", a.stage, a.context, a.n));
        ui.label(
            RichText::new(&a.state)
                .text_style(theme::meta())
                .color(p.n700),
        );
        for round in &a.rounds {
            round_line(ui, round);
            if round.feedback.is_none() && round.response.is_none() {
                continue;
            }
            let id = ui.make_persistent_id(("review-round", &t.id, &a.stage, a.n, round.n));
            fold(ui, id, &format!("Round {}", round.n), false, |ui| {
                if let Some(feedback) = &round.feedback {
                    ui.label(theme::meta_text(ui, "Findings"));
                    artifact_text(cx, ui, t, feedback);
                }
                if let Some(response) = &round.response {
                    ui.label(theme::meta_text(ui, "Response"));
                    artifact_text(cx, ui, t, response);
                }
            });
        }
        if let Some((_, summary)) = a.artifacts.iter().find(|(name, _)| name == "summary") {
            let id = ui.make_persistent_id(("review-summary", &t.id, &a.stage, a.n));
            fold(ui, id, "Summary", true, |ui| {
                artifact_text(cx, ui, t, summary);
            });
        }
        if let Some(text) = a.rewrite.as_ref().and_then(rewrite_label) {
            ui.label(theme::meta_text(ui, text));
        }
    }
}

/// Where a lane's changes are read: its tree while it stands, else
/// Dispatch's clone, which only the full ticket names; the base it was
/// cut from or brought up to; and its head, else its branch. The base
/// and head come from the status's lane, which is current every poll,
/// where a full ticket may still be the one read before the last move.
fn lane_range(l: &LaneView, clone: Option<&PathBuf>) -> Option<(PathBuf, String, String)> {
    let dir = if l.removed {
        clone?.clone()
    } else {
        l.worktree.clone()
    };
    let base = l.base_sha.clone()?;
    let head = l.head.clone().unwrap_or_else(|| l.branch.clone());
    Some((dir, base, head))
}

/// Each lane's commits over its base and the files they change, read
/// on a thread when the ticket or the lane's range changes.
fn changes(cx: &mut DrawCtx<'_>, ui: &mut Ui, t: &TicketView, now: u64) {
    let core = cx.core;
    if t.lanes.is_empty() {
        ui.label(theme::meta_text(ui, "No branch."));
        return;
    }
    for l in &t.lanes {
        if t.lanes.len() > 1 {
            theme::section(ui, &l.name);
        }
        let clone = core
            .ticket_details(&t.id)
            .and_then(|d| d.lanes.iter().find(|x| x.name == l.name))
            .and_then(|x| x.clone.as_ref());
        let Some((dir, base, head)) = lane_range(l, clone) else {
            ui.label(theme::meta_text(ui, "No branch."));
            continue;
        };
        let scan = scan_changes(cx, ui.ctx(), t, &l.name, (dir, base, head));
        match scan {
            None => {
                ui.label(theme::meta_text(ui, "Reading…"));
            }
            Some(Err(e)) => {
                ui.label(theme::meta_text(ui, format!("git: {e}")));
            }
            Some(Ok(c)) => {
                let tree = (!l.removed).then_some(l.worktree.as_path());
                show_changes(cx, ui, &c, tree, now);
            }
        }
    }
}

/// The lane's last read, starting a new one on a thread when the ticket
/// or the range changed since the last.
fn scan_changes(
    cx: &mut DrawCtx<'_>,
    ctx: &egui::Context,
    t: &TicketView,
    lane: &str,
    (dir, base, head): (PathBuf, String, String),
) -> Option<Result<Changes, String>> {
    let key = (t.id.clone(), lane.to_owned());
    let scan = cx.state.ticket_changes.entry(key).or_default();
    if let Some(rx) = &scan.rx
        && let Ok(result) = rx.try_recv()
    {
        scan.last = Some(result);
        scan.rx = None;
    }
    let range = (t.updated_ms, dir.clone(), base.clone(), head.clone());
    let stale = scan.key.as_ref() != Some(&range) || (scan.last.is_none() && scan.rx.is_none());
    if stale && scan.rx.is_none() {
        scan.key = Some(range);
        let (tx, rx) = channel();
        let reader = std::sync::Arc::clone(&cx.services.changes);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(reader.read(&dir, &base, &head));
            ctx.request_repaint();
        });
        scan.rx = Some(rx);
    }
    scan.last.clone()
}

/// Commits, newest first, then files with their line counts; a file can
/// be opened or revealed while the lane's tree stands.
fn show_changes(cx: &mut DrawCtx<'_>, ui: &mut Ui, c: &Changes, tree: Option<&Path>, now: u64) {
    let p = theme::palette(ui);
    theme::section(ui, &format!("Commits · {}", c.commits.len()));
    if c.commits.is_empty() {
        ui.label(theme::meta_text(ui, "No commits over the base."));
    }
    for commit in &c.commits {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(theme::mono_text(ui, short_sha(&commit.sha)).color(p.n700));
            ui.label(&commit.subject);
            when(ui, commit.at_ms, now);
        });
    }
    theme::section(ui, &format!("Files · {}", c.files.len()));
    for file in &c.files {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(theme::mono_text(ui, &file.path));
            ui.label(theme::meta_text(
                ui,
                format!("+{} −{}", file.added, file.removed),
            ));
            let Some(tree) = tree else {
                return;
            };
            let path = tree.join(&file.path);
            if theme::ghost_muted(ui, "Open").clicked() {
                cx.dispatch(if cx.core.settings().editor.is_empty() {
                    AppAction::OpenDocument(path.clone())
                } else {
                    AppAction::OpenInEditor(path.clone())
                });
            }
            if theme::ghost_muted(ui, "Reveal").clicked() {
                cx.dispatch(AppAction::RevealDocument(path));
            }
        });
    }
}

fn go_back(cx: &mut DrawCtx<'_>) {
    if cx.state.surface == super::Surface::DispatchWindow {
        cx.state.dispatch_window_ticket = None;
    } else {
        cx.dispatch(AppAction::Back);
    }
}

/// The stages in order, the current one in the text colour, the rest
/// muted; done when past the end.
fn stage_strip(ui: &mut Ui, t: &TicketView) {
    let p = theme::palette(ui);
    ui.spacing_mut().item_spacing.x = 3.0;
    for (i, name) in t.stages.iter().enumerate() {
        if i > 0 {
            ui.label(theme::meta_text(ui, "›").color(p.n600));
        }
        let color = match i.cmp(&t.stage) {
            std::cmp::Ordering::Less => p.n600,
            std::cmp::Ordering::Equal => p.text,
            std::cmp::Ordering::Greater => p.n700,
        };
        ui.label(RichText::new(name).text_style(theme::meta()).color(color));
    }
    if t.stage >= t.stages.len() && !t.stages.is_empty() {
        ui.label(theme::meta_text(ui, "› done"));
    }
}

/// The close dialog's draft, cleared once a frame before anything is
/// drawn when the core says it no longer stands
/// (`AppCore::close_dialog_stands`).
pub fn drop_stale_close_dialog(cx: &mut DrawCtx<'_>) {
    let Some(id) = cx.state.confirm_close_ticket.as_deref() else {
        return;
    };
    if !cx
        .core
        .close_dialog_stands(id, cx.state.dispatch_window_ticket.as_deref())
    {
        cx.state.confirm_close_ticket = None;
    }
}

/// Kicker, title with the issue link and Back, and the meta line with
/// the lanes and the pull request as chips.
fn ticket_header(cx: &mut DrawCtx<'_>, ui: &mut Ui, t: &TicketView) {
    let p = theme::palette(ui);
    theme::kicker(ui, "Dispatch ticket", p.n600);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::ghost(ui, "Back").clicked() {
                go_back(cx);
            }
            ticket_actions(cx, ui, t);
            if let Some(url) = &t.url {
                let what = if t.kind == "pull-request" {
                    "Pull request"
                } else {
                    "Issue"
                };
                ui.hyperlink_to(RichText::new(what).color(p.accent_text), url);
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(RichText::new(title_of(t)).text_style(theme::h1())).truncate(),
                );
            });
        });
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(theme::strong_text(&t.project));
        ui.label(theme::meta_text(ui, "·"));
        stage_strip(ui, t);
        ui.label(theme::meta_text(ui, "·"));
        ui.label(
            RichText::new(cx.core.ticket_standing(t))
                .text_style(theme::meta())
                .color(if cx.core.ticket_urgent(t) {
                    p.accent_2_text
                } else {
                    p.n700
                }),
        );
        if let Some(tree) = &t.tree {
            ui.label(theme::meta_text(ui, "·"));
            if t.tree_removed {
                ui.label(theme::meta_text(
                    ui,
                    format!("{} · removed", tree.display()),
                ));
            } else {
                ui.label(theme::mono_text(ui, tree.display().to_string()));
            }
        }
        if let Some(why) = &t.trees_kept {
            ui.label(theme::meta_text(ui, "·"));
            ui.label(
                RichText::new(format!("tree kept: {why}"))
                    .text_style(theme::meta())
                    .color(p.accent_2_text),
            );
        }
        for lane in &t.lanes {
            let text = format!(
                "{}{}{}{}",
                lane.name,
                if lane.chosen { "" } else { " (not chosen)" },
                if lane.setup_done { " · set up" } else { "" },
                if lane.removed { " · removed" } else { "" }
            );
            chip(ui, &text, if lane.chosen { p.text } else { p.n700 }).on_hover_text(format!(
                "{} on {}",
                lane.worktree.display(),
                lane.branch
            ));
        }
        // The page's own PR chip names the checks too, so it is not
        // the attempt's link under another label.
        if let Some(pr) = t.attempts.iter().rev().find_map(|a| a.pr.as_ref())
            && pr.number > 0
        {
            // On one line, so it moves to the next row whole rather
            // than breaking between its words.
            ui.scope(|ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                ui.hyperlink_to(
                    RichText::new(format!("PR #{} · {}", pr.number, pr.checks))
                        .text_style(theme::meta())
                        .color(p.accent_text),
                    &pr.url,
                );
            });
        }
    });
    holds_and_services(ui, t);
}

/// What the ticket holds, and the lanes it serves, each a link to
/// where it answers. A resource it waits for is its standing's to say.
fn holds_and_services(ui: &mut Ui, t: &TicketView) {
    let p = theme::palette(ui);
    if t.holds.is_empty() && t.services.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if !t.holds.is_empty() {
            ui.label(theme::meta_text(
                ui,
                format!("Holds: {}", t.holds.join(", ")),
            ));
        }
        if !t.services.is_empty() {
            ui.label(theme::meta_text(ui, "Services:"));
            for service in &t.services {
                match &service.url {
                    Some(url) => {
                        ui.hyperlink_to(RichText::new(&service.lane).color(p.accent_text), url);
                    }
                    None => {
                        ui.label(RichText::new(&service.lane).color(p.text));
                    }
                }
                ui.label(theme::meta_text(ui, &service.state));
            }
        }
    });
}

/// A small framed label on one line, for a lane on the meta row or an
/// issue's label from the tracker on the Issue tab. It is measured and
/// its size asked of the layout before it is drawn, because a Frame
/// takes whatever is left of a wrapping row and never moves to the next
/// one itself, so a chip at a full row's end would break letter by
/// letter. A chip wider than a whole row, such as an unusually long
/// tracker label, is clipped rather than broken.
fn chip(ui: &mut Ui, text: &str, color: egui::Color32) -> egui::Response {
    let frame = egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, theme::palette(ui).n600))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(5, 1));
    let rich = RichText::new(text).text_style(theme::meta()).color(color);
    let galley = egui::WidgetText::from(rich.clone()).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        theme::meta(),
    );
    // The total margin counts the stroke as well as the inner margin;
    // the frame takes both out of the space it is given.
    let size = galley.size() + frame.total_margin().sum();
    ui.allocate_ui(size, |ui| {
        frame
            .show(ui, |ui| {
                ui.add(egui::Label::new(rich).wrap_mode(egui::TextWrapMode::Extend))
            })
            .inner
    })
    .inner
}

/// Resume and Close, in the header's right-to-left row, where the
/// ticket's state allows them.
fn ticket_actions(cx: &mut DrawCtx<'_>, ui: &mut Ui, t: &TicketView) {
    if parked(t)
        && theme::secondary(ui, "Resume")
            .on_hover_text(super::dispatch::RESUME_HINT)
            .clicked()
    {
        cx.dispatch(AppAction::DispatchResume(t.id.clone()));
    }
    if !close_offered(t) {
        return;
    }
    let (label, hover) = if t.trees_retryable {
        ("Remove trees", "Try the removal again; the branch stays")
    } else {
        (
            "Close",
            "Remove the ticket's worktrees and close it; the branch and the record stay",
        )
    };
    if theme::secondary(ui, label).on_hover_text(hover).clicked() {
        cx.state.confirm_close_ticket = Some(t.id.clone());
    }
}

/// The close confirmation: what goes, by path, and what stays. A
/// confirmation rather than an undo, since a removed tree cannot be put
/// back as it was (its ignored build output is gone) and a closed
/// ticket does not resume.
fn confirm_close(cx: &mut DrawCtx<'_>, ctx: &egui::Context, t: &TicketView) {
    if cx.state.confirm_close_ticket.as_deref() != Some(t.id.as_str()) {
        return;
    }
    let retry = t.trees_retryable;
    let title = if retry {
        "Remove the ticket's trees"
    } else {
        "Close this ticket"
    };
    let mut done = false;
    dialog(ctx, title, |ui| {
        ui.label(
            "Removed with git worktree remove, never forced: git refuses a tree with changes.",
        );
        for path in &t.removes {
            ui.label(theme::mono_text(ui, path.display().to_string()));
        }
        ui.label(
            "The branch, the ticket's directory with its attempts and artifacts, and its \
             Switchboard projects stay.",
        );
        let (confirmed, cancelled) =
            dialog_actions(ui, if retry { "Try again" } else { "Close ticket" }, true);
        if confirmed {
            cx.dispatch(AppAction::DispatchClose(t.id.clone()));
            done = true;
        }
        if cancelled {
            done = true;
        }
    });
    if done || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        cx.state.confirm_close_ticket = None;
    }
}

/// The pull request a `ready` attempt is bound to: a link when one was
/// found, and what its checks said at the head it was read at.
fn pr_labels(ui: &mut Ui, pr: &crate::ports::dispatch::PullRequestView) {
    let p = theme::palette(ui);
    if pr.number > 0 {
        ui.hyperlink_to(
            RichText::new(format!("PR #{}", pr.number))
                .text_style(theme::meta())
                .color(p.accent_text),
            &pr.url,
        );
    }
    let short = short_sha(&pr.head);
    ui.label(theme::meta_text(
        ui,
        if short.is_empty() {
            pr.checks.clone()
        } else {
            format!("{} at {short}", pr.checks)
        },
    ));
}

/// An attempt's notes beside its state: why it ended, how often it was
/// nudged, and what a resolution review read.
fn attempt_notes(ui: &mut Ui, t: &TicketView, a: &AttemptView) {
    if let Some(reason) = &a.reason {
        ui.label(theme::meta_text(ui, reason));
    }
    if let Some(text) = nudged(a.nudges.len()) {
        ui.label(theme::meta_text(ui, text));
    }
    if let Some(text) = t.resolution_conflict(a) {
        ui.label(theme::meta_text(ui, text));
    }
}

/// An attempt in full: its state and notes, checks, PR, rewrite,
/// rounds, and buttons for its sessions, review and artifacts. The
/// artifact chosen is read and shown under the buttons.
fn attempt_row(cx: &mut DrawCtx<'_>, ui: &mut Ui, t: &TicketView, a: &AttemptView) {
    let p = theme::palette(ui);
    theme::surface(ui)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(theme::strong_text(format!(
                    "{} · {} · #{}",
                    a.stage, a.context, a.n
                )));
                ui.label(RichText::new(&a.state).text_style(theme::meta()).color(
                    match a.state.as_str() {
                        "failed" | "cancelled" => p.accent_2_text,
                        "complete" => p.n700,
                        _ => p.accent_text,
                    },
                ));
                attempt_notes(ui, t, a);
                if let Some(w) = cx
                    .core
                    .waiting_agents_of(t)
                    .into_iter()
                    .find(|w| w.stage == a.stage && w.context == a.context)
                {
                    ui.label(
                        RichText::new(format!("waiting on you: {}", w.reason))
                            .text_style(theme::meta())
                            .color(p.accent_2_text),
                    );
                }
                if let Some(checks) = &a.checks {
                    let short = short_sha(&checks.head);
                    ui.label(theme::meta_text(
                        ui,
                        match checks.exit {
                            None => format!("checks running at {short}"),
                            Some(0) => format!("checks passed at {short}"),
                            Some(code) => format!("checks exited {code} at {short}"),
                        },
                    ));
                }
                if let Some(pr) = &a.pr {
                    pr_labels(ui, pr);
                }
                // Why a lane at `merge` with an open PR has no question:
                // its merge is held behind another lane's.
                if let Some(waits) = &a.waits {
                    ui.label(theme::meta_text(ui, format!("waits for {waits}")));
                }
                // A fold with stale messages has moved the branch and
                // holds the attempt open while it asks or rewords, so it
                // is shown before the attempt completes.
                if let Some(text) = a
                    .rewrite
                    .as_ref()
                    .filter(|r| a.state == "complete" || !r.stale.is_empty())
                    .and_then(rewrite_label)
                {
                    ui.label(theme::meta_text(ui, text));
                }
            });
            for round in &a.rounds {
                round_line(ui, round);
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                session_button(cx, ui, "Session", a.session.as_deref());
                let rewriter = a
                    .rewrite
                    .as_ref()
                    .and_then(|r| r.message_session.as_deref());
                session_button(cx, ui, "Rewriter", rewriter);
                if let Some(run) = a
                    .run
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(crate::core::WorkflowId)
                    .filter(|id| cx.core.workflow(*id).is_some())
                    && theme::ghost(ui, "Review").clicked()
                {
                    cx.dispatch(AppAction::ShowWorkflow(run));
                }
                artifact_buttons(cx, ui, t, a);
            });
            if let Some(path) = cx.state.dispatch_artifact.clone().filter(|chosen| {
                a.artifacts.iter().any(|(_, p)| p == chosen) && !a.secret_at(chosen)
            }) {
                ui.label(theme::mono_text(ui, path.display().to_string()).color(p.n700));
                artifact_text(cx, ui, t, &path);
            }
        });
}

/// A button per artifact of the attempt; the chosen one is filled, and
/// a click on it again hides it. A secret artifact is a plain label:
/// Dispatch never reads it, so there is nothing to show.
fn artifact_buttons(cx: &mut DrawCtx<'_>, ui: &mut Ui, t: &TicketView, a: &AttemptView) {
    for (name, path) in &a.artifacts {
        if a.is_secret(name) {
            let label = if a.forgotten.contains(name) {
                format!("{name} (deleted)")
            } else {
                name.clone()
            };
            ui.label(theme::meta_text(ui, label))
                .on_hover_text("secret; deleted when its hold is released");
            continue;
        }
        let selected = cx.state.dispatch_artifact.as_ref() == Some(path);
        let button = if selected {
            theme::secondary(ui, name)
        } else {
            theme::ghost(ui, name)
        };
        if button.on_hover_text(path.display().to_string()).clicked() {
            cx.state.dispatch_artifact = (!selected).then(|| path.clone());
            // A click asks again whatever the last read said: the file
            // may have been written since.
            if !selected {
                cx.dispatch(AppAction::DispatchReadArtifact {
                    ticket: t.id.clone(),
                    path: path.clone(),
                });
            }
        }
    }
}

/// A button that shows `session`, when it names a session this app has.
fn session_button(cx: &mut DrawCtx<'_>, ui: &mut Ui, label: &str, session: Option<&str>) {
    if let Some(session) = session
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .map(RecordId)
        .filter(|id| cx.core.session(*id).is_some())
        && theme::ghost(ui, label).clicked()
    {
        cx.dispatch(AppAction::ShowSession(session));
    }
}

/// One line per code review round: its head, its state and what each
/// reviewer said.
fn round_line(ui: &mut Ui, round: &crate::ports::dispatch::ReviewRoundView) {
    let p = theme::palette(ui);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let short = short_sha(&round.head);
        ui.label(theme::meta_text(
            ui,
            format!("round {} at {short}", round.n),
        ));
        let points = match round.open_points {
            0 => String::new(),
            n => format!(", {n} open"),
        };
        ui.label(
            RichText::new(format!("{}{points}", round.state))
                .text_style(theme::meta())
                .color(if round.state.starts_with("failed") {
                    p.accent_2_text
                } else {
                    p.n700
                }),
        );
        for (name, state) in &round.reviewers {
            ui.label(theme::meta_text(ui, format!("{name}: {state}")));
        }
        if let Some(after) = &round.head_after {
            let short = short_sha(after);
            ui.label(theme::meta_text(ui, format!("fixed to {short}")));
        }
        if let Some(text) = nudged(round.nudges.len()) {
            ui.label(theme::meta_text(ui, text));
        }
    });
}

/// What a code review did to its branch's commits, and to the folded
/// messages it asked about; nothing when it left the commits as they
/// were.
fn rewrite_label(r: &RewriteView) -> Option<String> {
    if let Some(why) = &r.skipped {
        return Some(format!("commits kept: {why}"));
    }
    let after = r.after.as_deref().filter(|a| *a != r.before)?;
    let (before, after) = (short_sha(&r.before), short_sha(after));
    let rewrite = if r.mode == "one" {
        format!("squashed {} commits to one: {before} → {after}", r.from)
    } else {
        format!("commits folded {} → {}: {before} → {after}", r.from, r.to)
    };
    if r.stale.is_empty() {
        return Some(rewrite);
    }
    if !r.message_failed
        && let Some(head) = &r.message_head
    {
        return Some(format!(
            "{rewrite} · message rewritten → {}",
            short_sha(head)
        ));
    }
    let names: Vec<String> = r.stale.iter().map(|n| format!("`{n}`")).collect();
    Some(format!(
        "{rewrite} · message names {} ({})",
        names.join(", "),
        r.message.as_deref().unwrap_or("asked")
    ))
}

/// A commit's first eight characters, as the page names it.
fn short_sha(sha: &str) -> String {
    sha.chars().take(8).collect()
}
