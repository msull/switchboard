//! The event log: `<data>/events.jsonl`, one JSON object per line, one
//! line per transition of a ticket. It is not a record. Every ticket
//! write diffs the record it replaces against the one it writes
//! (`between`) and appends what changed before the record's rename, so
//! a supervising agent can follow what happened without rereading every
//! ticket.
//!
//! Readers must tolerate two things. A crash between the append and the
//! rename leaves an event whose write did not land; the next pass does
//! the transition again and logs it again, so a transition can repeat
//! but is never missed. A write that fails without a crash is withdrawn
//! by a `void` event whose `voids` names the seqs it takes back; a
//! reader drops those. Only when the `void` append fails too can a
//! withdrawn event stay unnamed.

use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use dispatch_control::{BroughtUpBy, brought_up};
use serde::{Deserialize, Serialize};

use crate::scheduler;
use crate::scheduler::BY_RESUME;
use crate::store::{DataDir, read_ticket};
use crate::ticket::{
    Attempt, AttemptKind, AttemptState, Decision, DecisionState, RoundState, Ticket, TicketState,
};

/// The log's file name in the data directory.
pub const EVENTS_FILE: &str = "events.jsonl";

/// The most bytes an event's `text` keeps; a longer one is cut on a
/// character boundary and ends `…`.
pub const TEXT_CAP: usize = 512;

/// The line format this build writes and reads; a line of another is
/// skipped.
pub const EVENT_VERSION: u32 = 1;

/// How long a follower sleeps between looks at the file.
pub const FOLLOW_POLL_MS: u64 = 250;

/// The first window `append` reads from the end of the file for the
/// last `seq`; doubled until it holds a whole line.
const TAIL_WINDOW: u64 = 8 * 1024;

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// A ticket record written for the first time.
    Taken,
    /// The ticket moved on to a later stage.
    Stage,
    /// The ticket's stage moved back: a later gate sent the work back.
    SentBack,
    /// An attempt was recorded running.
    AttemptStarted,
    /// Complete, failed or cancelled; `head` is the attempt's.
    AttemptEnded,
    /// A new decision; `decision` is its id.
    Decision,
    /// A decision was answered, by the user or by Dispatch.
    Answered,
    /// A pending decision was cancelled, by a park or a close.
    DecisionCancelled,
    /// An attempt's pull request first bound, or its url changed.
    Pr,
    /// The first word of a pull request's checks changed.
    PrChecks,
    /// A lane's branch was pushed by a refresh.
    Pushed,
    /// A lane was brought up to a moved base.
    Refreshed,
    /// A code review attempt's history rewrite, started or settled.
    Rewrite,
    /// A code review round began or changed state.
    Round,
    /// A park was asked for; the attempts are being stopped.
    Parking,
    /// The ticket is parked: nothing runs until it is resumed.
    Parked,
    /// Parked back to active.
    Resumed,
    /// A close was asked for; the attempts are being stopped.
    Closing,
    /// The ticket is closed and never writes again.
    Closed,
    /// The write these events described failed; `voids` names them.
    Void,
    /// A nudge was typed into an agent's session after it stopped with
    /// its tree not clean.
    Nudged,
    /// A check group a previous runner left running was signalled by
    /// this one, before the checks ran again or the attempt was
    /// cancelled.
    CheckOrphanKilled,
    /// The ticket was put at a stage under a fresh copy of the live
    /// pipeline; it replaces the stage move and the resume the same
    /// write would otherwise log.
    Restarted,
    /// A supervisor answered a decision its `decides` does not list; the
    /// decision still waits on the owner.
    Refused,
    /// A secret artifact's file was deleted; the text names it and why,
    /// never what it held.
    Forgotten,
    /// The owner's objection to a finished plan review was sent as its
    /// next round.
    Revised,
    /// A lane's merge question is held behind another lane's merge or
    /// that merge's base pipeline (`merge_after`).
    Waits,
}

impl Kind {
    /// The kebab-case word, as the line carries it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Taken => "taken",
            Self::Stage => "stage",
            Self::SentBack => "sent-back",
            Self::AttemptStarted => "attempt-started",
            Self::AttemptEnded => "attempt-ended",
            Self::Decision => "decision",
            Self::Answered => "answered",
            Self::DecisionCancelled => "decision-cancelled",
            Self::Pr => "pr",
            Self::PrChecks => "pr-checks",
            Self::Pushed => "pushed",
            Self::Refreshed => "refreshed",
            Self::Rewrite => "rewrite",
            Self::Round => "round",
            Self::Parking => "parking",
            Self::Parked => "parked",
            Self::Resumed => "resumed",
            Self::Closing => "closing",
            Self::Closed => "closed",
            Self::Void => "void",
            Self::Nudged => "nudged",
            Self::CheckOrphanKilled => "check-orphan-killed",
            Self::Restarted => "restarted",
            Self::Refused => "refused",
            Self::Forgotten => "forgotten",
            Self::Revised => "revised",
            Self::Waits => "waits",
        }
    }
}

/// One line of the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// The line's format; a reader skips any other.
    pub v: u32,
    /// Global and strictly increasing; 0 for an event made from a
    /// record rather than read from the log.
    pub seq: u64,
    /// When the write that made it happened.
    pub at_ms: u64,
    /// The ticket's id.
    pub ticket: String,
    /// The ticket's project.
    pub project: String,
    /// The attempt's or decision's stage, else the ticket's current
    /// stage, `done` past the last.
    pub stage: String,
    /// What happened.
    pub kind: Kind,
    /// One short line for a person, at most `TEXT_CAP` bytes.
    pub text: String,
    /// The attempt it is about, as stage and number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<(String, u32)>,
    /// The decision it is about, by id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    /// The commit it names: an attempt's head, a push's, a PR's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    /// The pull request it names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// For a `void`: the seqs it withdraws.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub voids: Vec<u64>,
    /// For a `refreshed`: who rewrote the branch onto its new base.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<BroughtUpBy>,
    /// For a `refreshed`: how many commits conflicted, 0 for git's
    /// bring-up and when they could not be listed; absent when no
    /// conflict was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflicts: Option<u32>,
    /// Who did it, when not the owner: `supervisor` on a take, park,
    /// resume, close, answer or refusal a supervisor session made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
}

impl Event {
    fn new(t: &Ticket, at_ms: u64, kind: Kind, stage: &str, text: String) -> Self {
        Self {
            v: EVENT_VERSION,
            seq: 0,
            at_ms,
            ticket: t.id.clone(),
            project: t.project.clone(),
            stage: stage.to_owned(),
            kind,
            text: capped(text),
            attempt: None,
            decision: None,
            head: None,
            url: None,
            voids: Vec::new(),
            by: None,
            conflicts: None,
            actor: None,
        }
    }

    fn of_attempt(t: &Ticket, at_ms: u64, kind: Kind, a: &Attempt, text: String) -> Self {
        Self {
            attempt: Some((a.stage.clone(), a.n)),
            ..Self::new(t, at_ms, kind, &a.stage, text)
        }
    }

    fn of_decision(t: &Ticket, at_ms: u64, kind: Kind, d: &Decision, text: String) -> Self {
        Self {
            decision: Some(d.id.clone()),
            attempt: d.attempt.clone(),
            ..Self::new(t, at_ms, kind, &d.stage, text)
        }
    }

    /// The `decision` event a pending decision would have logged, for a
    /// reader that needs one when the log has none (a decision raised
    /// before this build). Its seq is 0.
    #[must_use]
    pub fn from_decision(t: &Ticket, d: &Decision) -> Self {
        Self::of_decision(t, d.made_ms, Kind::Decision, d, decision_text(d))
    }

    /// The event the ticket's state would have logged, from the record.
    #[must_use]
    pub fn from_state(t: &Ticket, stage: &str) -> Option<Self> {
        let (kind, reason) = state_kind(&t.state)?;
        Some(Self::new(t, t.updated_ms, kind, stage, reason.to_owned()))
    }

    /// The line as stored, without its newline.
    #[must_use]
    pub fn to_line(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// `text` cut to `TEXT_CAP` bytes on a character boundary, with `…`
/// marking the cut; one line, so a newline in a question stays out.
fn capped(text: String) -> String {
    let text = if text.contains('\n') {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    } else {
        text
    };
    if text.len() <= TEXT_CAP {
        return text;
    }
    let ellipsis = '…';
    let mut end = TEXT_CAP - ellipsis.len_utf8();
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = text[..end].to_owned();
    out.push(ellipsis);
    out
}

fn decision_text(d: &Decision) -> String {
    format!("{}: {} [{}]", d.name, d.question, d.options.join("|"))
}

/// The state's event kind and reason; `None` for active.
fn state_kind(state: &TicketState) -> Option<(Kind, &str)> {
    match state {
        TicketState::Active => None,
        TicketState::Parking { reason } => Some((Kind::Parking, reason)),
        TicketState::Parked { reason } => Some((Kind::Parked, reason)),
        TicketState::Closing { reason } => Some((Kind::Closing, reason)),
        TicketState::Closed { reason } => Some((Kind::Closed, reason)),
    }
}

fn attempt_ended_text(a: &Attempt) -> String {
    match &a.state {
        AttemptState::Complete => "complete".to_owned(),
        AttemptState::Failed { reason } => format!("failed: {reason}"),
        AttemptState::Cancelled { reason } => format!("cancelled: {reason}"),
        AttemptState::Starting => "starting".to_owned(),
        AttemptState::Running => "running".to_owned(),
    }
}

fn kind_word(kind: AttemptKind) -> &'static str {
    match kind {
        AttemptKind::Agent => "agent",
        AttemptKind::Workflow => "workflow",
        AttemptKind::GateOnly => "gate-only",
        AttemptKind::Review => "review",
    }
}

fn round_word(state: &RoundState) -> String {
    match state {
        RoundState::Reviewing => "reviewing".to_owned(),
        RoundState::Converged => "converged".to_owned(),
        RoundState::Findings => "findings".to_owned(),
        RoundState::Fixing => "fixing".to_owned(),
        RoundState::Fixed => "fixed".to_owned(),
        RoundState::Accepted => "accepted".to_owned(),
        RoundState::Failed { reason } => format!("failed: {reason}"),
    }
}

/// The word before `:` of a pull request's checks: `failed: lint` and
/// `failed: test` are one reading.
fn checks_word(checks: &str) -> &str {
    checks.split(':').next().unwrap_or(checks).trim()
}

/// The stage name at `index`: from `names`, `done` past the last, and
/// the bare index when the pipeline copy could not be read.
#[must_use]
pub fn stage_name(names: &[String], index: usize) -> String {
    match names.get(index) {
        Some(name) => name.clone(),
        None if names.is_empty() => format!("#{index}"),
        None => "done".to_owned(),
    }
}

/// The ticket's stage names from its pipeline copy; empty when the copy
/// cannot be read.
#[must_use]
pub fn stage_names(t: &Ticket) -> Vec<String> {
    std::fs::read_to_string(&t.pipeline_file)
        .ok()
        .and_then(|text| crate::pipeline::Pipeline::parse(&text).ok())
        .map(|p| p.stages.iter().map(|s| s.name.clone()).collect())
        .unwrap_or_default()
}

/// What changed from `old` to `new`, as events with seq 0. `names`
/// gives the pipeline's stage names and is called only when something
/// happened, so a write that changes nothing reads no file. The ledger,
/// settle counts, polls and `updated_ms` make no event.
#[must_use]
pub fn between(
    old: Option<&Ticket>,
    new: &Ticket,
    at_ms: u64,
    names: &dyn Fn() -> Vec<String>,
) -> Vec<Event> {
    let mut out: Vec<Event> = Vec::new();
    // Events of the ticket as a whole take its current stage, filled in
    // once the names are read.
    let mut whole: Vec<usize> = Vec::new();
    let Some(old) = old else {
        whole.push(out.len());
        out.push(taken_event(new, at_ms));
        name_stages(&mut out, &whole, new, None, names);
        return out;
    };
    let mut moved: Option<Kind> = None;
    // A restart renumbers the stages under a new copy, so its index can
    // move without the ticket moving: its own event says what happened.
    let restarted = new.restarts.len() > old.restarts.len();
    if let Some(e) = restarted.then(|| restart_event(new, at_ms)).flatten() {
        whole.push(out.len());
        out.push(e);
    }
    if new.stage != old.stage && !restarted {
        let kind = if new.stage > old.stage {
            Kind::Stage
        } else {
            Kind::SentBack
        };
        moved = Some(kind);
        whole.push(out.len());
        out.push(Event::new(new, at_ms, kind, "", String::new()));
    }
    // The restart's own event says the ticket is active again.
    let state = state_event(old, new).filter(|_| !(restarted && new.active()));
    // A resume comes before the reruns it answered in the same write:
    // a waiter on a parked ticket wants the resume, and must never be
    // handed a `rerun` question that was born answered.
    if let Some((Kind::Resumed, text)) = &state {
        whole.push(out.len());
        out.push(state_by(
            new,
            Event::new(new, at_ms, Kind::Resumed, "", text.clone()),
        ));
    }
    for a in &new.attempts {
        let before = old
            .attempts
            .iter()
            .find(|x| x.stage == a.stage && x.n == a.n);
        attempt_events(&mut out, new, a, before, at_ms);
    }
    revision_events(&mut out, old, new, at_ms);
    for d in &new.decisions {
        let before = old.decisions.iter().find(|x| x.id == d.id);
        decision_events(&mut out, new, d, before, at_ms);
    }
    for lane in &new.lanes {
        let before = old.lanes.iter().find(|x| x.name == lane.name);
        if let Some(r) = &lane.refreshed
            && before.is_none_or(|b| b.refreshed.as_ref() != Some(r))
        {
            let by = scheduler::brought_up_by(new, &lane.name, r);
            let conflicts = scheduler::conflict_count(r, by);
            whole.push(out.len());
            out.push(Event {
                head: Some(r.to.clone()),
                by: Some(by),
                conflicts,
                ..Event::new(
                    new,
                    at_ms,
                    Kind::Refreshed,
                    "",
                    format!(
                        "{} from {} to {}, {}",
                        lane.name,
                        short(&r.from),
                        short(&r.to),
                        brought_up(by, r.commits, conflicts)
                    ),
                )
            });
        }
        if let Some(pushed) = &lane.pushed
            && before.is_none_or(|b| b.pushed.as_ref() != Some(pushed))
        {
            whole.push(out.len());
            out.push(Event {
                head: Some(pushed.head.clone()),
                ..Event::new(
                    new,
                    at_ms,
                    Kind::Pushed,
                    "",
                    format!("{} {} at {}", lane.name, lane.branch, short(&pushed.head)),
                )
            });
        }
    }
    tree_event(&mut out, &mut whole, old, new, at_ms);
    if let Some((kind, text)) = state
        && kind != Kind::Resumed
    {
        whole.push(out.len());
        out.push(state_by(new, Event::new(new, at_ms, kind, "", text)));
    }
    if !out.is_empty() {
        name_stages(&mut out, &whole, new, moved.map(|_| old.stage), names);
    }
    out
}

/// The tree's bring-up, as a lane's is logged, under the name `root`.
/// A restart can put back an older bring-up; that is not a new one.
fn tree_event(
    out: &mut Vec<Event>,
    whole: &mut Vec<usize>,
    old: &Ticket,
    new: &Ticket,
    at_ms: u64,
) {
    if let Some(r) = &new.tree_refreshed
        && old.tree_refreshed.as_ref() != Some(r)
        && old
            .tree_refreshed
            .as_ref()
            .is_none_or(|o| r.at_ms > o.at_ms)
    {
        // No rebaser or question is ever about the tree: its bring-up
        // is a clean rebase or nothing.
        let (by, conflicts) = (BroughtUpBy::Git, Some(0));
        whole.push(out.len());
        out.push(Event {
            head: Some(r.to.clone()),
            by: Some(by),
            conflicts,
            ..Event::new(
                new,
                at_ms,
                Kind::Refreshed,
                "",
                format!(
                    "root from {} to {}, {}",
                    short(&r.from),
                    short(&r.to),
                    brought_up(by, r.commits, conflicts)
                ),
            )
        });
    }
}

/// The `taken` event of a ticket written for the first time.
fn taken_event(t: &Ticket, at_ms: u64) -> Event {
    let text = format!(
        "{} {}{}",
        t.source.label(),
        t.source.title,
        by_clause(t.source.taken_by.as_deref())
    );
    Event {
        actor: t.source.taken_by.clone(),
        ..Event::new(t, at_ms, Kind::Taken, "", text)
    }
}

/// ` (by supervisor)` after the text of an event someone other than the
/// owner caused, or nothing.
fn by_clause(actor: Option<&str>) -> String {
    actor.map_or_else(String::new, |a| format!(" (by {a})"))
}

/// A park, resume or close event with the ticket's `state_by` as its
/// actor, named in its text; any other event as it is, since `parked`
/// and `closed` finish what the actor asked for.
fn state_by(t: &Ticket, e: Event) -> Event {
    if !matches!(e.kind, Kind::Parking | Kind::Resumed | Kind::Closing) {
        return e;
    }
    let Some(actor) = t.state_by.clone() else {
        return e;
    };
    let text = capped(format!("{}{}", e.text, by_clause(Some(&actor))));
    Event {
        actor: Some(actor),
        text,
        ..e
    }
}

/// The ticket's own state change from `old` to `new`, as an event's
/// kind and text. A resume names what it reran: each `rerun` it
/// answered in the same write, as `stage (context)`.
fn state_event(old: &Ticket, new: &Ticket) -> Option<(Kind, String)> {
    if std::mem::discriminant(&new.state) == std::mem::discriminant(&old.state) {
        return None;
    }
    if let Some((kind, reason)) = state_kind(&new.state) {
        return Some((kind, reason.to_owned()));
    }
    if !matches!(
        old.state,
        TicketState::Parked { .. } | TicketState::Parking { .. }
    ) {
        return None;
    }
    let reruns: Vec<String> = new
        .decisions
        .iter()
        .filter(|d| d.name == "rerun" && !old.decisions.iter().any(|x| x.id == d.id))
        .filter(|d| matches!(&d.state, DecisionState::Answered { by, .. } if by == BY_RESUME))
        .filter_map(|d| d.attempt.as_ref())
        .map(|(stage, n)| {
            let ctx = new
                .attempts
                .iter()
                .find(|a| &a.stage == stage && a.n == *n)
                .map_or("?", |a| a.context.as_str());
            format!("{stage} ({ctx})")
        })
        .collect();
    let text = if reruns.is_empty() {
        "active again".to_owned()
    } else {
        format!("active again, rerunning {}", reruns.join(", "))
    };
    Some((Kind::Resumed, text))
}

/// The `restarted` event for the ticket's last restart.
fn restart_event(t: &Ticket, at_ms: u64) -> Option<Event> {
    let r = t.restarts.last()?;
    Some(Event::new(
        t,
        at_ms,
        Kind::Restarted,
        "",
        format!("at {} from {} under {}", r.to, r.from, base_name(&r.after)),
    ))
}

/// The stage of every whole-ticket event, and a move's text, from the
/// pipeline's names.
fn name_stages(
    out: &mut [Event],
    whole: &[usize],
    new: &Ticket,
    from: Option<usize>,
    names: &dyn Fn() -> Vec<String>,
) {
    if whole.is_empty() {
        return;
    }
    let names = names();
    let current = stage_name(&names, new.stage);
    for &i in whole {
        out[i].stage.clone_from(&current);
        if matches!(out[i].kind, Kind::Stage | Kind::SentBack)
            && let Some(from) = from
        {
            out[i].text = capped(format!("{} → {current}", stage_name(&names, from)));
        }
    }
}

/// A commit's first seven characters, as git shows it.
#[must_use]
pub fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// A path's last component, as a message names a pipeline copy; empty
/// for a path with none.
#[must_use]
pub fn base_name(path: &std::path::Path) -> String {
    path.file_name()
        .map_or_else(String::new, |f| f.to_string_lossy().into_owned())
}

/// Names as a message shows them: "`a`, `b`".
pub(crate) fn names_list(names: &[String]) -> String {
    names
        .iter()
        .map(|n| format!("`{n}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A `waits` line when a merge hold begins, or moves to another lane
/// or to another thing; the release is the question that follows.
fn waits_event(
    out: &mut Vec<Event>,
    t: &Ticket,
    a: &Attempt,
    before: Option<&Attempt>,
    at_ms: u64,
) {
    let Some(w) = a.waits.as_ref().filter(|w| w.holds()) else {
        return;
    };
    let old = before.and_then(|b| b.waits.as_ref()).filter(|o| o.holds());
    if old.is_none_or(|o| o.lane != w.lane || o.until != w.until) {
        out.push(Event::of_attempt(
            t,
            at_ms,
            Kind::Waits,
            a,
            format!("{} ({}) waits for {}", a.stage, a.context, w.describe()),
        ));
    }
}

fn attempt_events(
    out: &mut Vec<Event>,
    t: &Ticket,
    a: &Attempt,
    before: Option<&Attempt>,
    at_ms: u64,
) {
    if before.is_none() {
        out.push(Event::of_attempt(
            t,
            at_ms,
            Kind::AttemptStarted,
            a,
            format!("{} #{} {}", a.context, a.n, kind_word(a.kind)),
        ));
    }
    let was_open = before.is_none_or(Attempt::is_open);
    if was_open && !a.is_open() {
        out.push(Event {
            head: a.head.clone(),
            ..Event::of_attempt(t, at_ms, Kind::AttemptEnded, a, attempt_ended_text(a))
        });
    }
    if let Some(pr) = &a.pr {
        let old = before.and_then(|b| b.pr.as_ref());
        if old.is_none_or(|o| o.url != pr.url) {
            out.push(Event {
                url: Some(pr.url.clone()),
                head: Some(pr.head.clone()),
                ..Event::of_attempt(
                    t,
                    at_ms,
                    Kind::Pr,
                    a,
                    format!("PR #{} {} checks {}", pr.number, pr.url, pr.checks),
                )
            });
        } else if let Some(o) = old
            && checks_word(&o.checks) != checks_word(&pr.checks)
        {
            out.push(Event {
                url: Some(pr.url.clone()),
                head: Some(pr.head.clone()),
                ..Event::of_attempt(t, at_ms, Kind::PrChecks, a, pr.checks.clone())
            });
        }
    }
    waits_event(out, t, a, before, at_ms);
    if let Some(r) = &a.rewrite {
        let old = before.and_then(|b| b.rewrite.as_ref());
        let changed = match old {
            None => true,
            Some(o) => {
                (o.after.is_none() && r.after.is_some())
                    || (o.skipped.is_none() && r.skipped.is_some())
            }
        };
        if changed {
            let text = match (&r.after, &r.skipped) {
                (_, Some(why)) => format!("{} commits kept: {why}", r.mode.as_str()),
                (Some(after), None) => format!(
                    "{} {} → {} commits, {} → {}",
                    r.mode.as_str(),
                    r.from,
                    r.to,
                    short(&r.before),
                    short(after)
                ),
                (None, None) => format!("{} from {} started", r.mode.as_str(), short(&r.before)),
            };
            out.push(Event {
                head: r.after.clone().or_else(|| Some(r.before.clone())),
                ..Event::of_attempt(t, at_ms, Kind::Rewrite, a, text)
            });
        }
        message_events(out, t, a, r, old, at_ms);
    }
    nudge_events(out, t, a, before, at_ms);
    orphan_events(out, t, a, before, at_ms);
    forgotten_events(out, t, a, before, at_ms);
    for round in &a.rounds {
        let old = before.and_then(|b| b.rounds.iter().find(|x| x.n == round.n));
        if old.is_none_or(|o| o.state != round.state) {
            out.push(Event {
                head: Some(round.head.clone()),
                ..Event::of_attempt(
                    t,
                    at_ms,
                    Kind::Round,
                    a,
                    format!(
                        "r{} {} open {}",
                        round.n,
                        round_word(&round.state),
                        round.open_points
                    ),
                )
            });
        }
    }
}

/// What became of a rewrite's folded messages: found naming what the
/// tree lacks, rewritten, or kept as written.
fn message_events(
    out: &mut Vec<Event>,
    t: &Ticket,
    a: &Attempt,
    r: &crate::ticket::Rewrite,
    old: Option<&crate::ticket::Rewrite>,
    at_ms: u64,
) {
    let old_message = old.and_then(|o| o.message.as_ref());
    let mut texts = Vec::new();
    if old.is_none_or(|o| o.stale.is_empty()) && !r.stale.is_empty() {
        texts.push(format!(
            "folded message names {}, which the tree does not have",
            names_list(&r.stale_names())
        ));
    }
    if let Some(m) = &r.message {
        if let Some(to) = &m.to
            && old_message.is_none_or(|o| o.to.is_none())
        {
            texts.push(format!(
                "message rewritten {} → {}",
                short(&m.from),
                short(to)
            ));
        }
        if m.answer == "accept" && old_message.is_none_or(|o| o.answer != "accept") {
            texts.push(if m.to.is_some() {
                "message kept as rewritten".to_owned()
            } else {
                "message kept as written".to_owned()
            });
        }
    }
    for text in texts {
        out.push(Event {
            head: r.after.clone(),
            ..Event::of_attempt(t, at_ms, Kind::Rewrite, a, text)
        });
    }
}

/// One `nudged` event per nudge recorded since `before`, on the
/// attempt or on one of its rounds.
fn nudge_events(
    out: &mut Vec<Event>,
    t: &Ticket,
    a: &Attempt,
    before: Option<&Attempt>,
    at_ms: u64,
) {
    for k in before.map_or(0, |b| b.nudges.len())..a.nudges.len() {
        let text = format!("nudge {}: the tree is not clean", k + 1);
        out.push(Event::of_attempt(t, at_ms, Kind::Nudged, a, text));
    }
    for round in &a.rounds {
        let old = before.and_then(|b| b.rounds.iter().find(|x| x.n == round.n));
        for k in old.map_or(0, |o| o.nudges.len())..round.nudges.len() {
            let text = format!("r{} nudge {}: the tree is not clean", round.n, k + 1);
            out.push(Event::of_attempt(t, at_ms, Kind::Nudged, a, text));
        }
    }
}

/// One `revised` event per owner's objection whose send settled since
/// `old` without a refusal: answered, or with its reply lost (the round
/// may have started). The revision is on the attempt from the write
/// before the send, so a refused objection, whose revision is popped,
/// is never logged as one.
fn revision_events(out: &mut Vec<Event>, old: &Ticket, new: &Ticket, at_ms: u64) {
    let settled = |o: &crate::ticket::Operation| o.reply.is_some() || o.error.is_some();
    for op in new
        .ledger
        .iter()
        .filter(|o| o.intent == scheduler::REVISE && settled(o))
    {
        if old.ledger.iter().any(|o| o.op == op.op && settled(o))
            || matches!(op.reply, Some(switchboard_control::Reply::Failed { .. }))
        {
            continue;
        }
        let Some(switchboard_control::Body::WorkflowObject { round, .. }) = &op.body else {
            continue;
        };
        let Some((stage, n)) = &op.attempt else {
            continue;
        };
        let Some(a) = new.attempts.iter().find(|a| &a.stage == stage && a.n == *n) else {
            continue;
        };
        let Some(r) = a.revisions.iter().find(|r| r.round == *round) else {
            continue;
        };
        let text = format!("r{}: the owner's objection, by {}", r.round, r.by);
        out.push(Event {
            actor: (r.by == scheduler::BY_SUPERVISOR).then(|| r.by.clone()),
            ..Event::of_attempt(new, at_ms, Kind::Revised, a, text)
        });
    }
}

/// One `check-orphan-killed` event per check or command reviewer group
/// a previous runner left running that was signalled since `before`.
fn orphan_events(
    out: &mut Vec<Event>,
    t: &Ticket,
    a: &Attempt,
    before: Option<&Attempt>,
    at_ms: u64,
) {
    let seen = before.map_or(0, |b| b.orphans_killed.len());
    for o in a.orphans_killed.iter().skip(seen) {
        let what = match &o.reviewer {
            Some(name) => format!("reviewer {name}"),
            None => "checks".to_owned(),
        };
        let text = format!(
            "{what} at {} left running by a previous runner (group {}) stopped",
            short(&o.head),
            o.pgid
        );
        out.push(Event {
            head: Some(o.head.clone()),
            ..Event::of_attempt(t, at_ms, Kind::CheckOrphanKilled, a, text)
        });
    }
}

/// One `forgotten` event per secret artifact deleted since `before`.
fn forgotten_events(
    out: &mut Vec<Event>,
    t: &Ticket,
    a: &Attempt,
    before: Option<&Attempt>,
    at_ms: u64,
) {
    for (name, f) in &a.forgotten {
        if before.is_none_or(|b| !b.forgotten.contains_key(name)) {
            let text = format!("{name} deleted: {}", f.why);
            out.push(Event::of_attempt(t, at_ms, Kind::Forgotten, a, text));
        }
    }
}

fn decision_events(
    out: &mut Vec<Event>,
    t: &Ticket,
    d: &Decision,
    before: Option<&Decision>,
    at_ms: u64,
) {
    if before.is_none() {
        out.push(Event::of_decision(
            t,
            at_ms,
            Kind::Decision,
            d,
            decision_text(d),
        ));
    }
    let was = before.map(|b| &b.state);
    let answered_before = matches!(was, Some(DecisionState::Answered { .. }));
    let cancelled_before = matches!(was, Some(DecisionState::Cancelled));
    if let DecisionState::Answered {
        answer, note, by, ..
    } = &d.state
        && !answered_before
    {
        let note = note.as_ref().map_or(String::new(), |n| format!(" — {n}"));
        out.push(Event {
            actor: (by == scheduler::BY_SUPERVISOR).then(|| by.clone()),
            ..Event::of_decision(
                t,
                at_ms,
                Kind::Answered,
                d,
                format!("{} by {by}: {answer}{note}", d.name),
            )
        });
    }
    // A refusal leaves the decision pending: its own event says who
    // asked what.
    for r in d
        .refusals
        .iter()
        .skip(before.map_or(0, |b| b.refusals.len()))
    {
        out.push(Event {
            actor: Some(r.by.clone()),
            ..Event::of_decision(
                t,
                at_ms,
                Kind::Refused,
                d,
                format!(
                    "{}: the {} asked {}; refused, the owner answers",
                    d.name, r.by, r.answer
                ),
            )
        });
    }
    if d.state == DecisionState::Cancelled && !cancelled_before {
        out.push(Event::of_decision(
            t,
            at_ms,
            Kind::DecisionCancelled,
            d,
            d.name.clone(),
        ));
    }
}

/// The log's path in a data directory.
#[must_use]
pub fn log_path(data: &DataDir) -> PathBuf {
    data.root.join(EVENTS_FILE)
}

/// Append `events` with the next seqs, written into each, and synced
/// before returning so they are on disk before the record they describe
/// is renamed into place. The caller holds the writer lock, which keeps
/// seqs unique. A torn last line (a crash mid-append) is closed with a
/// newline first, so it stays a line of its own that readers skip.
pub fn append(path: &Path, events: &mut [Event]) -> Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = open_log(path)?;
    let len = file.metadata()?.len();
    let mut seq = last_seq_in(&mut file, len)?;
    let mut bytes = Vec::new();
    if len > 0 {
        let mut last = [0u8; 1];
        file.seek(SeekFrom::Start(len - 1))?;
        file.read_exact(&mut last)?;
        if last[0] != b'\n' {
            bytes.push(b'\n');
        }
    }
    for e in events.iter_mut() {
        seq += 1;
        e.seq = seq;
        bytes.extend_from_slice(e.to_line().as_bytes());
        bytes.push(b'\n');
    }
    file.write_all(&bytes)
        .with_context(|| format!("append to {}", path.display()))?;
    file.sync_data()?;
    Ok(())
}

/// One `void` withdrawing `seqs`, appended as `append` does.
pub fn append_void(path: &Path, about: &Event, seqs: Vec<u64>, why: &str) -> Result<()> {
    let mut void = [Event {
        v: EVENT_VERSION,
        seq: 0,
        at_ms: about.at_ms,
        ticket: about.ticket.clone(),
        project: about.project.clone(),
        stage: about.stage.clone(),
        kind: Kind::Void,
        text: capped(format!("the write failed: {why}")),
        attempt: None,
        decision: None,
        head: None,
        url: None,
        voids: seqs,
        by: None,
        conflicts: None,
        actor: None,
    }];
    append(path, &mut void)
}

fn open_log(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .with_context(|| format!("open {}", path.display()))
}

/// The last seq in the log; 0 when it is empty or absent.
pub fn last_seq(path: &Path) -> Result<u64> {
    match File::open(path) {
        Ok(mut file) => {
            let len = file.metadata()?.len();
            last_seq_in(&mut file, len)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e).with_context(|| format!("open {}", path.display())),
    }
}

/// The seq of the last complete line that parses, read from a window at
/// the end that widens until it holds that line whole, up to the whole
/// file. A torn last line is passed over for the one before it.
fn last_seq_in(file: &mut File, len: u64) -> Result<u64> {
    let mut window = TAIL_WINDOW;
    loop {
        let start = len.saturating_sub(window);
        file.seek(SeekFrom::Start(start))?;
        let mut buf = Vec::new();
        Read::by_ref(file).take(len - start).read_to_end(&mut buf)?;
        let mut pieces: Vec<&[u8]> = buf.split(|b| *b == b'\n').collect();
        // After the last newline: empty, or a torn line.
        pieces.pop();
        // Before the first newline of a window that starts mid-file: a
        // line cut by the window, read whole on a wider pass.
        let cut = start > 0 && !pieces.is_empty();
        let whole = if cut { &pieces[1..] } else { &pieces[..] };
        for line in whole.iter().rev() {
            if let Some(seq) = seq_of(line) {
                return Ok(seq);
            }
        }
        if start == 0 {
            return Ok(0);
        }
        window = window.saturating_mul(2);
    }
}

fn seq_of(line: &[u8]) -> Option<u64> {
    let value: serde_json::Value = serde_json::from_slice(line).ok()?;
    value.get("seq")?.as_u64()
}

/// One complete line as an event of this format, else `None`.
fn parse(line: &[u8]) -> Option<Event> {
    let event: Event = serde_json::from_slice(line).ok()?;
    (event.v == EVENT_VERSION).then_some(event)
}

/// Every event after `since`, in order, from complete lines; a line
/// that does not parse, of another format, or unterminated at the end
/// is skipped.
pub fn read_since(path: &Path, since: u64) -> Result<Vec<Event>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    };
    Ok(complete_lines(&bytes)
        .filter_map(parse)
        .filter(|e| e.seq > since)
        .collect())
}

/// The lines of `bytes` that end in a newline.
fn complete_lines(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    let end = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    bytes[..end]
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
}

/// The seqs every `void` in `events` withdraws.
#[must_use]
pub fn withdrawn(events: &[Event]) -> BTreeSet<u64> {
    events
        .iter()
        .filter(|e| e.kind == Kind::Void)
        .flat_map(|e| e.voids.iter().copied())
        .collect()
}

/// The attempts of `ticket`, by stage and number, whose end the log at
/// `path` holds after the ticket's latest `parking` event that no
/// `void` withdrew; `None` when the log has no such event. The log's
/// order decides, not the events' times: a pass stamps every write with
/// the time it began, so a park written while it ran can carry a later
/// time than the cancellations it caused.
pub fn ended_since_parking(path: &Path, ticket: &str) -> Result<Option<BTreeSet<(String, u32)>>> {
    let events = read_since(path, 0)?;
    let voided = withdrawn(&events);
    let live = |e: &&Event| e.ticket == ticket && !voided.contains(&e.seq);
    let Some(parking) = events
        .iter()
        .filter(live)
        .rfind(|e| e.kind == Kind::Parking)
        .map(|e| e.seq)
    else {
        return Ok(None);
    };
    Ok(Some(
        events
            .iter()
            .filter(live)
            .filter(|e| e.seq > parking && e.kind == Kind::AttemptEnded)
            .filter_map(|e| e.attempt.clone())
            .collect(),
    ))
}

/// A reader of the log from a seq on, which returns what was appended
/// since its last look. The cursor is the reader's own: nothing is
/// stored for it.
pub struct Follow {
    path: PathBuf,
    /// Bytes read so far, up to the end of the last complete line.
    offset: u64,
    /// The last seq returned; anything at or below it is not again.
    seen: u64,
}

/// Follow the log at `path` from after `since`.
#[must_use]
pub fn follow(path: &Path, since: u64) -> Follow {
    Follow {
        path: path.to_path_buf(),
        offset: 0,
        seen: since,
    }
}

impl Follow {
    /// The events appended since the last call, as complete lines.
    pub fn next_batch(&mut self) -> Result<Vec<Event>> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).with_context(|| format!("open {}", self.path.display())),
        };
        let len = file.metadata()?.len();
        if len < self.offset {
            // Replaced by hand: read it again, the seq filter keeps out
            // what was already returned.
            self.offset = 0;
        }
        if len == self.offset {
            return Ok(Vec::new());
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut buf = Vec::new();
        file.take(len - self.offset).read_to_end(&mut buf)?;
        let Some(end) = buf.iter().rposition(|b| *b == b'\n') else {
            return Ok(Vec::new());
        };
        self.offset += end as u64 + 1;
        let events: Vec<Event> = complete_lines(&buf[..=end])
            .filter_map(parse)
            .filter(|e| e.seq > self.seen)
            .collect();
        if let Some(last) = events.last() {
            self.seen = last.seq;
        }
        Ok(events)
    }
}

/// What `wait` waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum For {
    /// A decision waiting on the user.
    Decision,
    /// A stage move, forward or sent back, or a restart.
    Stage,
    /// A pull request bound to an attempt.
    Pr,
    /// The close.
    Closed,
    /// The next event of any kind.
    Any,
    /// What a supervisor acts on: what `Stage` and `Pr` match (a stage
    /// move, a send-back, a restart, a pull request bound), plus a
    /// decision asked, a `pr-checks` line and a merge held (`waits`).
    Move,
}

impl For {
    /// `decision`, `stage`, `pr`, `closed`, `move` or `any`.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "decision" => Self::Decision,
            "stage" => Self::Stage,
            "pr" => Self::Pr,
            "closed" => Self::Closed,
            "any" => Self::Any,
            "move" => Self::Move,
            _ => return None,
        })
    }

    /// Whether a line of `kind` is what this filter waits for; a park
    /// or close ends every wait besides.
    #[must_use]
    pub fn candidate(self, kind: Kind) -> bool {
        match self {
            Self::Decision => kind == Kind::Decision,
            Self::Stage => matches!(kind, Kind::Stage | Kind::SentBack | Kind::Restarted),
            Self::Pr => kind == Kind::Pr,
            Self::Closed => kind == Kind::Closed,
            Self::Any => kind != Kind::Void,
            Self::Move => matches!(
                kind,
                Kind::Stage
                    | Kind::SentBack
                    | Kind::Restarted
                    | Kind::Decision
                    | Kind::Pr
                    | Kind::PrChecks
                    | Kind::Waits
            ),
        }
    }

    /// Whether `wait_burst` keeps following after a match.
    fn bunches(self) -> bool {
        matches!(self, Self::Any | Self::Move)
    }
}

/// How a wait ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Waited {
    /// What was waited for happened.
    Matched(Event),
    /// The ticket parked or closed, so it will not.
    Ended(Event),
    /// The deadline passed first.
    TimedOut,
}

/// Wait until `ticket` does what `what` names, it parks or closes, or
/// `deadline_ms` (by `now`) passes. The record is read first, so what
/// already holds (a pending decision, a parked or closed ticket) returns
/// at once. Then the log is followed from `since`, or from its tail when
/// `since` is `None`, and each candidate is confirmed against a fresh read
/// of the record, since an event can be one whose write failed and whose
/// `void` is not read yet. Any candidate, live or replayed, that the
/// record does not bear out yet is held while its write may be in flight
/// (`in_flight`) and checked again at each look, until the record bears
/// it out or its `void` drops it. One the record has passed and still
/// contradicts, whether first read or held, is dropped.
/// `pause` runs between looks at the log: a sleep, or a test's step.
pub fn wait(
    data: &DataDir,
    ticket: &str,
    what: For,
    since: Option<u64>,
    deadline_ms: Option<u64>,
    now: &mut dyn FnMut() -> u64,
    pause: &mut dyn FnMut(),
) -> Result<Waited> {
    let log = log_path(data);
    // Read before `start`: a write appends before it renames, so every
    // event after `start` is a change this record does not hold yet,
    // which is what confirms a live event of a kind with no check of its
    // own. An event at or before `start` may be in `before` already.
    let before = read_ticket(&data.ticket_file(ticket))?;
    let start = last_seq(&log)?;
    let first = read_ticket(&data.ticket_file(ticket))?;
    let names = stage_names(&first);
    if let Some(done) = already(&log, &first, what, &names)? {
        return Ok(done);
    }
    // A cursor past the tail would hide the events still to come.
    let mut follow = follow(&log, since.map_or(start, |s| s.min(start)));
    let mut held: Vec<Event> = Vec::new();
    // The oldest held event `fresh` bears out, so log order holds even
    // when a write lands between the reads of one batch. A held event the
    // record has passed and contradicts is dropped, so it cannot match
    // later when the record happens to agree with it again.
    // One borne out whose transition a later line logs again gives way
    // to that line: it is the write that landed, and the loop reaches it.
    // The batch loop does the same for an event borne out at first read.
    let landed = |held: &mut Vec<Event>, fresh: &Ticket| -> Result<Option<Event>> {
        for e in std::mem::take(held) {
            if confirmed(&e, &before, fresh, what, &names, e.seq <= start) {
                if superseded(&log, &e)? {
                    continue;
                }
                return Ok(Some(e));
            }
            if in_flight(&e, fresh) {
                held.push(e);
            }
        }
        Ok(None)
    };
    loop {
        let batch: Vec<Event> = follow
            .next_batch()?
            .into_iter()
            .filter(|e| e.ticket == ticket)
            .collect();
        let gone = withdrawn(&batch);
        held.retain(|e| !gone.contains(&e.seq));
        if !held.is_empty() {
            let fresh = read_ticket(&data.ticket_file(ticket))?;
            if let Some(e) = landed(&mut held, &fresh)? {
                return Ok(Waited::Matched(e));
            }
        }
        for e in batch {
            if e.kind == Kind::Void || gone.contains(&e.seq) {
                continue;
            }
            let ends = matches!(e.kind, Kind::Parked | Kind::Closed);
            if !what.candidate(e.kind) && !ends {
                continue;
            }
            let fresh = read_ticket(&data.ticket_file(ticket))?;
            let replayed = e.seq <= start;
            if !confirmed(&e, &before, &fresh, what, &names, replayed) {
                if in_flight(&e, &fresh) {
                    held.push(e);
                }
                continue;
            }
            if superseded(&log, &e)? {
                continue;
            }
            if let Some(older) = landed(&mut held, &fresh)? {
                return Ok(Waited::Matched(older));
            }
            return Ok(if what.candidate(e.kind) {
                Waited::Matched(e)
            } else {
                Waited::Ended(e)
            });
        }
        if deadline_ms.is_some_and(|d| now() >= d) {
            return Ok(Waited::TimedOut);
        }
        pause();
    }
}

/// How long a burst stays open after its last line.
pub const SETTLE_MS: u64 = 2_000;

/// How long a burst stays open after its first line, however busy the
/// ticket.
pub const SETTLE_CAP_MS: u64 = 10_000;

/// How a `wait_burst` ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Burst {
    /// What was waited for happened: the matching lines in log order,
    /// never empty, the last one the cursor to follow from.
    Lines(Vec<Event>),
    /// The ticket parked or closed: the matching lines logged before
    /// the end, in log order, and the park or close itself, which is
    /// always printed, even when it has no line of its own.
    Ended {
        /// The matching lines before the end.
        lines: Vec<Event>,
        /// The park or close, boxed to keep the other variants small.
        end: Box<Event>,
    },
    /// The deadline passed before any line.
    TimedOut,
}

/// Wait as `wait` does, then, for `--for any` and `--for move`, keep
/// following for `SETTLE_MS` after each match (`SETTLE_CAP_MS` after the
/// first at most, and never past `deadline_ms`) and return every match in
/// log order. The window holds by the clock and by the lines' own
/// `at_ms` (`settles`), so a burst replayed from an old cursor stops
/// where a live one would have. A decision ends the burst at once; a
/// park or close ends it with the lines logged before it. With a cursor,
/// a ticket already parked or closed first gives the matching lines
/// logged after the cursor, ending at a decision among them or else at
/// the park or close. Every other filter returns its first match.
///
/// Each follow-up is a fresh `wait` from the last line's seq, the same
/// hand-off a supervisor makes between two watches, so every hold rule
/// stays inside `wait`.
pub fn wait_burst(
    data: &DataDir,
    ticket: &str,
    what: For,
    since: Option<u64>,
    deadline_ms: Option<u64>,
    now: &mut dyn FnMut() -> u64,
    pause: &mut dyn FnMut(),
) -> Result<Burst> {
    let log = log_path(data);
    let mut cursor = since;
    loop {
        let first = match wait(data, ticket, what, cursor, deadline_ms, now, pause)? {
            Waited::TimedOut => return Ok(Burst::TimedOut),
            Waited::Matched(e) if !what.bunches() => return Ok(Burst::Lines(vec![e])),
            // `--for any` matches a park or close line; it still ends
            // the burst, and anything before it was not borne out.
            Waited::Matched(e) if ends(&e) => {
                return Ok(Burst::Ended {
                    lines: Vec::new(),
                    end: Box::new(e),
                });
            }
            Waited::Matched(e) => e,
            Waited::Ended(end) => {
                return Ok(match cursor {
                    // `already` answered without reading the lines
                    // between the cursor and the end: the call after a
                    // burst that stopped at a decision just before a park.
                    Some(from) if what.bunches() => {
                        let before = ended_lines(data, &log, ticket, what, from, &end)?;
                        end_rule(Vec::new(), before, end)
                    }
                    _ => Burst::Ended {
                        lines: Vec::new(),
                        end: Box::new(end),
                    },
                });
            }
        };
        let opened = now();
        let mut lines = vec![first];
        let ended = loop {
            let last = &lines[lines.len() - 1];
            if last.kind == Kind::Decision {
                break None;
            }
            let from = last.seq;
            let mut until = (now() + SETTLE_MS).min(opened + SETTLE_CAP_MS);
            if let Some(d) = deadline_ms {
                until = until.min(d);
            }
            match wait(data, ticket, what, Some(from), Some(until), now, pause)? {
                Waited::Matched(e) if !settles(&lines, &e) => break None,
                Waited::Matched(e) if ends(&e) => break Some((Vec::new(), e)),
                Waited::Matched(e) => lines.push(e),
                Waited::Ended(end) => {
                    let before = ended_lines(data, &log, ticket, what, from, &end)?;
                    break Some((before, end));
                }
                Waited::TimedOut => break None,
            }
        };
        // A `void` can land inside the window after its line was taken.
        let seen = lines.last().map_or(0, |e| e.seq);
        let gone = withdrawn(&read_since(&log, lines[0].seq.saturating_sub(1))?);
        lines.retain(|e| !gone.contains(&e.seq));
        match ended {
            Some((before, end)) => return Ok(end_rule(lines, before, end)),
            None if lines.is_empty() => cursor = Some(seen),
            None => return Ok(Burst::Lines(lines)),
        }
    }
}

/// The burst a subscription is due, for a pass that cannot wait: what
/// `wait_burst` with `For::Move` returns from `since`, with the clock
/// stepped by `FOLLOW_POLL_MS` at each look instead of sleeping, so the
/// log is read as it is now. `None` when nothing is due: no line, a
/// burst that may still grow (its last line neither a decision nor an
/// end, within `SETTLE_MS` of `now_ms` and its first within
/// `SETTLE_CAP_MS`), or a park the subscriber already has
/// (`park_seen`, and the park's line at or before `since` or none).
/// The park or close of an `Ended` burst is the last line returned.
///
/// The park rule compares the end's kind and seq, never its time: a park
/// with no line of its own is made from the record, with `updated_ms`
/// for its time, and every write to the parked ticket moves that.
pub fn next_burst(
    data: &DataDir,
    ticket: &str,
    since: u64,
    park_seen: bool,
    now_ms: u64,
) -> Result<Option<Vec<Event>>> {
    let log = log_path(data);
    // Most passes find nothing new: skip the whole read of the log
    // unless the record holds an end the subscriber may lack.
    if last_seq(&log)? <= since {
        let t = read_ticket(&data.ticket_file(ticket))?;
        let unseen_end = match t.state {
            TicketState::Closed { .. } => true,
            TicketState::Parked { .. } => !park_seen,
            _ => false,
        };
        if !unseen_end {
            return Ok(None);
        }
    }
    let clock = std::cell::Cell::new(now_ms);
    let burst = wait_burst(
        data,
        ticket,
        For::Move,
        Some(since),
        Some(now_ms),
        &mut || clock.get(),
        &mut || clock.set(clock.get() + FOLLOW_POLL_MS),
    )?;
    let lines = match burst {
        Burst::TimedOut => return Ok(None),
        Burst::Ended { lines, end } => {
            let seen = end.kind == Kind::Parked && park_seen && (end.seq == 0 || end.seq <= since);
            if seen && lines.is_empty() {
                return Ok(None);
            }
            let mut lines = lines;
            if !seen {
                lines.push(*end);
            }
            return Ok(Some(lines));
        }
        Burst::Lines(lines) => lines,
    };
    let (Some(first), Some(last)) = (lines.first(), lines.last()) else {
        return Ok(None);
    };
    let open = last.kind != Kind::Decision
        && last.at_ms.saturating_add(SETTLE_MS) > now_ms
        && first.at_ms.saturating_add(SETTLE_CAP_MS) > now_ms;
    if open {
        return Ok(None);
    }
    Ok(Some(lines))
}

/// An event as a line: as stored with `json`, else
/// `seq  hh:mm:ss  ticket  stage  kind  text`.
#[must_use]
pub fn line(e: &Event, json: bool) -> String {
    if json {
        return e.to_line();
    }
    let text = if e.kind == Kind::Void {
        let seqs: Vec<String> = e.voids.iter().map(u64::to_string).collect();
        format!("withdraws {}: {}", seqs.join(", "), e.text)
    } else {
        e.text.clone()
    };
    format!(
        "{}  {}  {}  {}  {}  {text}",
        e.seq,
        clock(e.at_ms),
        e.ticket,
        e.stage,
        e.kind.as_str()
    )
}

/// The `decide` line after a `decision` event, while that decision
/// still waits on the user. `command` is how the reader runs it:
/// `dispatch decide`, or the full path a supervisor's permission needs.
#[must_use]
pub fn decide_hint(e: &Event, t: &Ticket, command: &str) -> Option<String> {
    let d = e.decision.as_ref()?;
    t.waiting_on_you().iter().any(|w| &w.id == d).then(|| {
        format!(
            "    {command} {} {d} <answer> [--note <text> | --file <path>]",
            t.id
        )
    })
}

/// `hh:mm:ss` of `ms` since the epoch, in the local zone: the reader is
/// a person or an agent on this machine, and a UTC clock next to a local
/// one (the shell's `date`, a log file) misleads twice a day. Falls back
/// to UTC only when the moment is out of range.
#[must_use]
pub fn clock(ms: u64) -> String {
    use chrono::{DateTime, Local, TimeZone as _};
    let secs = i64::try_from(ms / 1000).unwrap_or(i64::MAX);
    if let Some(utc) = DateTime::from_timestamp(secs, 0) {
        return Local
            .from_utc_datetime(&utc.naive_utc())
            .format("%H:%M:%S")
            .to_string();
    }
    let s = (ms / 1000) % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

/// Whether `e` is a park or close.
fn ends(e: &Event) -> bool {
    matches!(e.kind, Kind::Parked | Kind::Closed)
}

/// Whether `e` falls inside the window `lines` opened, by the lines'
/// own times: within `SETTLE_MS` of the last and `SETTLE_CAP_MS` of the
/// first. The clock closes a live burst, but lines replayed from an old
/// cursor all arrive before it moves. Any line opens an empty window.
fn settles(lines: &[Event], e: &Event) -> bool {
    let (Some(first), Some(last)) = (lines.first(), lines.last()) else {
        return true;
    };
    e.at_ms <= last.at_ms.saturating_add(SETTLE_MS)
        && e.at_ms <= first.at_ms.saturating_add(SETTLE_CAP_MS)
}

/// A burst a park or close ended: `lines`, then `before`, the lines
/// logged before the end, then the end. A decision among `before` still
/// ends the burst at once, and so does a line, or an end line, past the
/// window (`settles`); the burst stops there and the next watch, from
/// the last line's seq, returns the rest and the end.
fn end_rule(mut lines: Vec<Event>, before: Vec<Event>, end: Event) -> Burst {
    for e in before {
        if !settles(&lines, &e) {
            return Burst::Lines(lines);
        }
        let decision = e.kind == Kind::Decision;
        lines.push(e);
        if decision {
            return Burst::Lines(lines);
        }
    }
    if end.seq != 0 && !settles(&lines, &end) {
        return Burst::Lines(lines);
    }
    Burst::Ended {
        lines,
        end: Box::new(end),
    }
}

/// The lines of `ticket` after `from` and before `end` (every one after
/// `from` when `end` has no line of its own) that `what` matches, that
/// no `void` withdrew, and that the record bears out as replayed lines.
/// Nothing can be held here: the record that reads parked or closed was
/// written after every line before the end line.
fn ended_lines(
    data: &DataDir,
    log: &Path,
    ticket: &str,
    what: For,
    from: u64,
    end: &Event,
) -> Result<Vec<Event>> {
    let events = read_since(log, from)?;
    let gone = withdrawn(&events);
    let fresh = read_ticket(&data.ticket_file(ticket))?;
    let names = stage_names(&fresh);
    let mut lines = Vec::new();
    for e in events {
        if e.ticket != ticket
            || e.kind == Kind::Void
            || gone.contains(&e.seq)
            || (end.seq != 0 && e.seq >= end.seq)
            || ends(&e)
            || !what.candidate(e.kind)
        {
            continue;
        }
        if confirmed(&e, &fresh, &fresh, what, &names, true) && !superseded(log, &e)? {
            lines.push(e);
        }
    }
    Ok(lines)
}

/// Whether a later line of `e`'s ticket, not withdrawn, logs the same
/// transition. A write that agrees with a phantom (one whose rename and
/// `void` both failed) makes the transition the record lacks, so it logs
/// the transition again, and appends that line before its rename: by
/// the time a record bears `e` out, such a line is already in the log.
fn superseded(log: &Path, e: &Event) -> Result<bool> {
    let later = read_since(log, e.seq)?;
    let gone = withdrawn(&later);
    Ok(later
        .iter()
        .any(|x| x.ticket == e.ticket && !gone.contains(&x.seq) && same_transition(e, x)))
}

/// Whether `b` logs the transition `a` does: a decision by its id, or a
/// pull request by its url on the same attempt. Those subjects happen
/// once, so a second line for one is the phantom asked again. Every
/// other kind can recur for real (a second nudge, a stage entered again,
/// a park after a resume), and a later line of it is a new transition
/// the caller must not miss, so it never stands in for an earlier one.
fn same_transition(a: &Event, b: &Event) -> bool {
    a.kind == b.kind
        && match a.kind {
            Kind::Decision => a.decision == b.decision,
            Kind::Pr => a.url == b.url && a.attempt == b.attempt,
            _ => false,
        }
}

/// Whether the write that logged `e` may not have landed in `fresh` yet.
/// A line is appended before its record is renamed in, so a record older
/// than the line cannot speak for it. Nor can one at the line's own
/// `at_ms`: the scheduler saves a ticket twice in one millisecond (a
/// failed attempt, then the decision it asks), so a record at `at_ms`
/// may be the first of the two. Only a record strictly past the line
/// has certainly taken its write.
fn in_flight(e: &Event, fresh: &Ticket) -> bool {
    fresh.updated_ms <= e.at_ms
}

/// What the record already says before any event is read: a pending
/// decision, or a ticket parked or closed, which ends every wait but
/// `--for closed` on a closed one, as a park or close seen while waiting
/// does. `wait_burst` puts the matching lines between a cursor and the
/// end before it, for `--for any` and `--for move`.
fn already(log: &Path, t: &Ticket, what: For, names: &[String]) -> Result<Option<Waited>> {
    let latest = |kind: Kind, decision: Option<&str>| -> Result<Option<Event>> {
        let events = read_since(log, 0)?;
        let gone = withdrawn(&events);
        Ok(events.into_iter().rev().find(|e| {
            e.ticket == t.id
                && e.kind == kind
                && !gone.contains(&e.seq)
                && decision.is_none_or(|d| e.decision.as_deref() == Some(d))
        }))
    };
    let stage = stage_name(names, t.stage);
    if what == For::Decision
        && let Some(d) = t.waiting_on_you().last().copied()
    {
        let event =
            latest(Kind::Decision, Some(&d.id))?.unwrap_or_else(|| Event::from_decision(t, d));
        return Ok(Some(Waited::Matched(event)));
    }
    let ended = matches!(
        t.state,
        TicketState::Parked { .. } | TicketState::Closed { .. }
    );
    if !ended {
        return Ok(None);
    }
    let Some(kind) = state_kind(&t.state).map(|(k, _)| k) else {
        return Ok(None);
    };
    let event = latest(kind, None)?.or_else(|| Event::from_state(t, &stage));
    let Some(event) = event else {
        return Ok(None);
    };
    Ok(match (what, kind) {
        (For::Closed, Kind::Closed) => Some(Waited::Matched(event)),
        _ => Some(Waited::Ended(event)),
    })
}

/// Whether the record bears out a candidate event: the transition it
/// names is there, so it is not a write that failed. `before` is the
/// record as it was before the first event followed. A decision waited
/// for must still be waiting on the user: one Dispatch or another
/// terminal answered already is not something to answer. A `replayed`
/// event may be in `before` already, so one with no check of its own is
/// borne out once the record's `updated_ms` reaches the write it logs.
/// A live one, held or not, is checked against what changed since
/// `before`, which cannot hold it yet.
fn confirmed(
    e: &Event,
    before: &Ticket,
    fresh: &Ticket,
    what: For,
    names: &[String],
    replayed: bool,
) -> bool {
    match e.kind {
        Kind::Decision => e.decision.as_ref().is_some_and(|id| {
            if what == For::Decision {
                fresh.waiting_on_you().iter().any(|d| &d.id == id)
            } else {
                fresh.decisions.iter().any(|d| &d.id == id)
            }
        }),
        Kind::Stage | Kind::SentBack => stage_name(names, fresh.stage) == e.stage,
        // A plain restart leaves the stage's name as it was, so only the
        // restart itself, saved in the same write, bears the line out.
        Kind::Restarted => fresh.restarts.iter().any(|r| r.at_ms == e.at_ms),
        Kind::Pr => e.url.as_ref().is_some_and(|url| {
            fresh
                .attempts
                .iter()
                .any(|a| a.pr.as_ref().is_some_and(|pr| &pr.url == url))
        }),
        Kind::Closed => matches!(fresh.state, TicketState::Closed { .. }),
        Kind::Parked => matches!(fresh.state, TicketState::Parked { .. }),
        kind => {
            what.candidate(kind)
                && if replayed {
                    fresh.updated_ms >= e.at_ms
                } else {
                    between(Some(before), fresh, 0, &Vec::new)
                        .iter()
                        .any(|x| x.kind == kind)
                }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::scheduler::new_attempt;
    use crate::ticket::{
        CloseProgress, DecisionKind, LaneRecord, PullRequestRecord, PushedHead, ReviewRound,
        SourceSnapshot,
    };

    fn ticket() -> Ticket {
        Ticket {
            version: 0,
            id: "t1".into(),
            project: "p".into(),
            source: SourceSnapshot {
                kind: "github".into(),
                identity: "o/r#7".into(),
                pull_requests: Vec::new(),
                number: Some(7),
                title: "a title".into(),
                body: String::new(),
                url: None,
                labels: vec![],
                taken_at_ms: 0,
                taken_by: None,
            },
            pipeline_fingerprint: String::new(),
            pipeline_file: PathBuf::new(),
            lanes: vec![LaneRecord {
                name: "backend".into(),
                worktree: "/wt".into(),
                branch: "dispatch/7-a-title".into(),
                project: None,
                chosen: true,
                setup_done: false,
                base_sha: Some("base0000".into()),
                refreshed: None,
                pushed: None,
                conflict: None,
                removed: false,
            }],
            tree: None,
            stage: 0,
            attempts: vec![],
            decisions: vec![],
            ledger: vec![],
            processes: vec![],
            root_project: None,
            rework: BTreeMap::new(),
            refreshed_stage: None,
            tree_refreshed: None,
            state: TicketState::Active,
            state_by: None,
            close: CloseProgress::default(),
            restarts: Vec::new(),
            restart: None,
            entered: Vec::new(),
            holds: Vec::new(),
            services: Vec::new(),
            created_ms: 0,
            updated_ms: 0,
        }
    }

    fn names() -> Vec<String> {
        vec!["plan".into(), "implement".into(), "merge".into()]
    }

    fn kinds(old: Option<&Ticket>, new: &Ticket) -> Vec<Kind> {
        between(old, new, 5, &names)
            .iter()
            .map(|e| e.kind)
            .collect()
    }

    fn running(stage: &str, n: u32) -> Attempt {
        new_attempt(
            stage,
            n,
            "root",
            AttemptKind::Agent,
            AttemptState::Running,
            BTreeMap::new(),
            1,
        )
    }

    fn decision(id: &str) -> Decision {
        Decision {
            id: id.into(),
            stage: "plan".into(),
            name: "finalize".into(),
            kind: DecisionKind::Permission,
            question: "Finalize it?".into(),
            options: vec!["finalize".into(), "park".into()],
            recommendation: None,
            attempt: Some(("plan".into(), 1)),
            state: DecisionState::Pending,
            made_ms: 3,
            refusals: Vec::new(),
        }
    }

    #[test]
    fn a_first_write_is_taken_and_a_stage_move_is_named_from_the_pipeline() {
        let t = ticket();
        let taken = between(None, &t, 5, &names);
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].kind, Kind::Taken);
        assert_eq!(taken[0].stage, "plan");
        assert_eq!(taken[0].text, "#7 a title");
        let mut next = t.clone();
        next.stage = 1;
        let moved = between(Some(&t), &next, 5, &names);
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].kind, Kind::Stage);
        assert_eq!(
            (moved[0].stage.as_str(), moved[0].text.as_str()),
            ("implement", "plan → implement")
        );
        assert_eq!(kinds(Some(&next), &t), [Kind::SentBack]);
        next.stage = 3;
        assert_eq!(between(Some(&t), &next, 5, &names)[0].stage, "done");
    }

    #[test]
    fn the_ledger_polls_and_the_clock_make_nothing_and_read_no_names() {
        let t = ticket();
        let mut next = t.clone();
        next.updated_ms = 99;
        next.processes.push("s".into());
        let mut a = running("plan", 1);
        let mut moved = t.clone();
        moved.attempts.push(a.clone());
        next.attempts.push(a.clone());
        a.polls_since_stop = 4;
        a.stop_at_ms = Some(8);
        let mut polled = next.clone();
        polled.attempts[0] = a;
        let never = || -> Vec<String> { panic!("no names for no events") };
        assert!(between(Some(&next), &polled, 5, &never).is_empty());
        assert!(between(Some(&moved), &next, 5, &never).is_empty());
    }

    #[test]
    fn a_nudge_on_an_attempt_is_one_nudged_event() {
        let mut started = ticket();
        started.attempts.push(running("implement", 1));
        let mut nudged = started.clone();
        nudged.attempts[0].nudges.push(7);
        let events = between(Some(&started), &nudged, 5, &names);
        assert_eq!(kinds(Some(&started), &nudged), [Kind::Nudged]);
        assert_eq!(events[0].text, "nudge 1: the tree is not clean");
        assert_eq!(Kind::Nudged.as_str(), "nudged");
        assert!(between(Some(&nudged), &nudged, 5, &names).is_empty());
    }

    /// An objection is logged once its send settles without a refusal,
    /// not when the revision is written ahead of it.
    #[test]
    fn an_owners_round_is_one_revised_event_once_its_send_settles() {
        use switchboard_control::{Body, Reply};
        let mut done = ticket();
        let mut a = running("review", 1);
        a.state = AttemptState::Complete;
        done.attempts.push(a);
        let object = |t: &Ticket, op: &str, round: u32, by: &str| {
            let mut t = t.clone();
            t.attempts[0].revisions.push(crate::ticket::Revision {
                round,
                by: by.into(),
                at_ms: 7,
            });
            t.ledger.push(crate::ticket::Operation::new(
                op.into(),
                &Body::WorkflowObject {
                    run: "run-1".into(),
                    round,
                    text: "no".into(),
                },
                Some(("review".into(), 1)),
                scheduler::REVISE,
                7,
            ));
            t
        };
        let settle = |t: &Ticket, reply: Option<Reply>, error: Option<&str>| {
            let mut t = t.clone();
            let op = t.ledger.last_mut().unwrap();
            op.reply = reply;
            op.error = error.map(str::to_owned);
            t
        };
        let sent = object(&done, "op-1", 2, scheduler::BY_HAND);
        assert!(between(Some(&done), &sent, 5, &names).is_empty());
        let answered = settle(&sent, Some(Reply::Persisted { made: vec![] }), None);
        let events = between(Some(&sent), &answered, 5, &names);
        assert_eq!(kinds(Some(&sent), &answered), [Kind::Revised]);
        assert_eq!(events[0].text, "r2: the owner's objection, by you");
        assert_eq!(events[0].actor, None);
        assert_eq!(Kind::Revised.as_str(), "revised");
        assert!(between(Some(&answered), &answered, 5, &names).is_empty());
        // Refused: the revision is popped in the same write.
        let sent = object(&answered, "op-2", 3, scheduler::BY_SUPERVISOR);
        let mut refused = settle(&sent, Some(Reply::failed("no")), None);
        refused.attempts[0].revisions.pop();
        assert!(between(Some(&sent), &refused, 5, &names).is_empty());
        // Lost: the round may have started, and is logged.
        let sent = object(&refused, "op-3", 3, scheduler::BY_SUPERVISOR);
        let lost = settle(&sent, None, Some("the socket closed"));
        let events = between(Some(&sent), &lost, 5, &names);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].text, "r3: the owner's objection, by supervisor");
        assert_eq!(events[0].actor.as_deref(), Some(scheduler::BY_SUPERVISOR));
    }

    #[test]
    fn a_secret_deleted_is_one_forgotten_event_with_the_name_and_why() {
        let mut done = ticket();
        let mut a = running("implement", 1);
        a.state = AttemptState::Complete;
        a.secret.insert("personas".into());
        done.attempts.push(a);
        let mut deleted = done.clone();
        deleted.attempts[0].forgotten.insert(
            "personas".into(),
            crate::ticket::Forgotten {
                at_ms: 7,
                why: "dev-stack released".into(),
            },
        );
        let events = between(Some(&done), &deleted, 5, &names);
        assert_eq!(kinds(Some(&done), &deleted), [Kind::Forgotten]);
        assert_eq!(events[0].text, "personas deleted: dev-stack released");
        assert_eq!(Kind::Forgotten.as_str(), "forgotten");
        assert!(between(Some(&deleted), &deleted, 5, &names).is_empty());
    }

    #[test]
    fn an_orphaned_check_stopped_is_one_check_orphan_killed_event() {
        let mut started = ticket();
        started.attempts.push(running("implement", 1));
        let mut stopped = started.clone();
        stopped.attempts[0]
            .orphans_killed
            .push(crate::ticket::OrphanKill {
                pgid: 4000,
                leader_started: "fake-1".into(),
                head: "abcdef0123".into(),
                at_ms: 7,
                reviewer: None,
            });
        let events = between(Some(&started), &stopped, 5, &names);
        assert_eq!(kinds(Some(&started), &stopped), [Kind::CheckOrphanKilled]);
        assert_eq!(
            events[0].text,
            "checks at abcdef0 left running by a previous runner (group 4000) stopped"
        );
        assert_eq!(Kind::CheckOrphanKilled.as_str(), "check-orphan-killed");
        assert!(between(Some(&stopped), &stopped, 5, &names).is_empty());
    }

    #[test]
    fn an_orphaned_command_reviewer_stopped_names_the_reviewer() {
        let mut started = ticket();
        started.attempts.push(running("implement", 1));
        let mut stopped = started.clone();
        stopped.attempts[0]
            .orphans_killed
            .push(crate::ticket::OrphanKill {
                pgid: 4001,
                leader_started: "fake-2".into(),
                head: "abcdef0123".into(),
                at_ms: 7,
                reviewer: Some("lint".into()),
            });
        let events = between(Some(&started), &stopped, 5, &names);
        assert_eq!(kinds(Some(&started), &stopped), [Kind::CheckOrphanKilled]);
        assert_eq!(
            events[0].text,
            "reviewer lint at abcdef0 left running by a previous runner (group 4001) stopped"
        );
    }

    #[test]
    fn a_held_merge_logs_waits_when_it_begins_or_changes_and_not_on_release() {
        use crate::ticket::{MergeWait, Released, WaitUntil};
        let mut started = ticket();
        let mut a = running("merge", 1);
        a.context = "docs".into();
        started.attempts.push(a);
        let wait = MergeWait {
            lane: "repo".into(),
            until: WaitUntil::Merge,
            step: None,
            commit: None,
            run: None,
            since_ms: 1_000,
            released: None,
        };
        let mut held = started.clone();
        held.attempts[0].waits = Some(wait.clone());
        let events = between(Some(&started), &held, 5, &names);
        assert_eq!(kinds(Some(&started), &held), [Kind::Waits]);
        assert_eq!(events[0].text, "merge (docs) waits for repo's merge");
        assert_eq!(Kind::Waits.as_str(), "waits");
        // The same hold read again, with a new run word: nothing.
        let mut again = held.clone();
        again.attempts[0].waits.as_mut().unwrap().run = Some("pending".into());
        assert!(kinds(Some(&held), &again).is_empty());
        // The wait moves on to the base pipeline.
        let mut deploy = again.clone();
        let w = deploy.attempts[0].waits.as_mut().unwrap();
        w.until = WaitUntil::Deploy;
        w.step = Some("Deploy to dev".into());
        let events = between(Some(&again), &deploy, 5, &names);
        assert_eq!(
            events[0].text,
            "merge (docs) waits for repo's base pipeline past \"Deploy to dev\""
        );
        // A release logs nothing; the question follows.
        let mut released = deploy.clone();
        released.attempts[0].waits.as_mut().unwrap().released = Some(Released {
            clause: "repo merged".into(),
            plain: true,
        });
        assert!(kinds(Some(&deploy), &released).is_empty());
        assert!(For::Move.candidate(Kind::Waits));
    }

    #[test]
    fn attempts_start_end_and_bind_a_pr_whose_checks_change() {
        let t = ticket();
        let mut started = t.clone();
        started.attempts.push(running("merge", 1));
        assert_eq!(kinds(Some(&t), &started), [Kind::AttemptStarted]);
        let mut pr = started.clone();
        pr.attempts[0].pr = Some(PullRequestRecord {
            provider: "github".into(),
            repo: "o/r".into(),
            number: 3,
            url: "https://example.com/pr/3".into(),
            head: "head0001".into(),
            checks: "pending".into(),
            checked_ms: 0,
            error_since_ms: None,
            merge_commit: None,
        });
        let events = between(Some(&started), &pr, 5, &names);
        assert_eq!(events[0].kind, Kind::Pr);
        assert_eq!(events[0].url.as_deref(), Some("https://example.com/pr/3"));
        assert_eq!(events[0].head.as_deref(), Some("head0001"));
        let mut failed = pr.clone();
        failed.attempts[0].pr.as_mut().unwrap().checks = "failed: lint".into();
        assert_eq!(kinds(Some(&pr), &failed), [Kind::PrChecks]);
        let mut again = failed.clone();
        again.attempts[0].pr.as_mut().unwrap().checks = "failed: lint, test".into();
        assert!(kinds(Some(&failed), &again).is_empty(), "the same word");
        let mut ended = again.clone();
        ended.attempts[0].state = AttemptState::Failed {
            reason: "checks failed".into(),
        };
        ended.attempts[0].head = Some("head0001".into());
        let events = between(Some(&again), &ended, 5, &names);
        assert_eq!(events[0].kind, Kind::AttemptEnded);
        assert_eq!(events[0].text, "failed: checks failed");
        assert_eq!(events[0].stage, "merge");
        assert_eq!(events[0].attempt, Some(("merge".into(), 1)));
    }

    #[test]
    fn decisions_are_raised_answered_and_cancelled() {
        let t = ticket();
        let mut raised = t.clone();
        raised.decisions.push(decision("d1"));
        let events = between(Some(&t), &raised, 5, &names);
        assert_eq!(events[0].kind, Kind::Decision);
        assert_eq!(events[0].decision.as_deref(), Some("d1"));
        assert_eq!(events[0].text, "finalize: Finalize it? [finalize|park]");
        let mut answered = raised.clone();
        answered.decisions[0].state = DecisionState::Answered {
            answer: "finalize".into(),
            note: Some("go".into()),
            by: "you".into(),
            at_ms: 4,
            acted: false,
        };
        let events = between(Some(&raised), &answered, 5, &names);
        assert_eq!(events[0].kind, Kind::Answered);
        assert_eq!(events[0].text, "finalize by you: finalize — go");
        let mut acted = answered.clone();
        if let DecisionState::Answered { acted: a, .. } = &mut acted.decisions[0].state {
            *a = true;
        }
        assert!(kinds(Some(&answered), &acted).is_empty());
        let mut cancelled = raised.clone();
        cancelled.decisions[0].state = DecisionState::Cancelled;
        assert_eq!(kinds(Some(&raised), &cancelled), [Kind::DecisionCancelled]);
    }

    #[test]
    fn lanes_rounds_rewrites_and_states_each_have_their_event() {
        let t = ticket();
        let mut pushed = t.clone();
        pushed.lanes[0].pushed = Some(PushedHead {
            head: "push0001".into(),
            at_ms: 1,
        });
        pushed.lanes[0].refreshed = Some(crate::ticket::Refreshed {
            from: "base0000".into(),
            to: "main0002".into(),
            commits: true,
            notes: None,
            at_ms: 1,
            conflict: None,
            after: None,
        });
        assert_eq!(kinds(Some(&t), &pushed), [Kind::Refreshed, Kind::Pushed]);

        let mut review = t.clone();
        let mut a = running("review-code", 1);
        a.kind = AttemptKind::Review;
        review.attempts.push(a);
        let mut round = review.clone();
        round.attempts[0].rounds.push(ReviewRound {
            n: 1,
            base: "base0000".into(),
            head: "head0001".into(),
            reviewers: vec![],
            state: RoundState::Reviewing,
            feedback: None,
            open_points: 0,
            fix_authorised: false,
            implementer: None,
            response: None,
            head_after: None,
            stop_at_ms: None,
            polls_since_stop: 0,
            settle: None,
            dirty_polls: 0,
            dirty_since_ms: None,
            nudges: Vec::new(),
            started_ms: 1,
            ended_ms: None,
        });
        assert_eq!(kinds(Some(&review), &round), [Kind::Round]);
        let mut found = round.clone();
        found.attempts[0].rounds[0].state = RoundState::Findings;
        found.attempts[0].rounds[0].open_points = 2;
        let events = between(Some(&round), &found, 5, &names);
        assert_eq!(events[0].text, "r1 findings open 2");
        let mut rewrite = found.clone();
        rewrite.attempts[0].rewrite = Some(crate::ticket::Rewrite {
            mode: crate::history::Commits::Fold,
            before: "head0001".into(),
            after: None,
            from: 3,
            to: 0,
            skipped: None,
            stale: Vec::new(),
            message: None,
            at_ms: 1,
        });
        assert_eq!(kinds(Some(&found), &rewrite), [Kind::Rewrite]);
        let mut done = rewrite.clone();
        done.attempts[0].rewrite.as_mut().unwrap().after = Some("fold0001".into());
        done.attempts[0].rewrite.as_mut().unwrap().to = 1;
        assert_eq!(kinds(Some(&rewrite), &done), [Kind::Rewrite]);
        let mut nudged = found.clone();
        nudged.attempts[0].rounds[0].nudges.push(7);
        let events = between(Some(&found), &nudged, 5, &names);
        assert_eq!(kinds(Some(&found), &nudged), [Kind::Nudged]);
        assert_eq!(events[0].text, "r1 nudge 1: the tree is not clean");

        let parking = |reason: &str| TicketState::Parking {
            reason: reason.into(),
        };
        let mut s1 = t.clone();
        s1.state = parking("by hand");
        let mut s2 = t.clone();
        s2.state = TicketState::Parked {
            reason: "by hand".into(),
        };
        let mut s3 = t.clone();
        s3.state = TicketState::Closing {
            reason: "done".into(),
        };
        let mut s4 = t.clone();
        s4.state = TicketState::Closed {
            reason: "done".into(),
        };
        assert_eq!(kinds(Some(&t), &s1), [Kind::Parking]);
        assert_eq!(kinds(Some(&s1), &s2), [Kind::Parked]);
        assert_eq!(kinds(Some(&s2), &t), [Kind::Resumed]);
        assert_eq!(kinds(Some(&t), &s3), [Kind::Closing]);
        assert_eq!(kinds(Some(&s3), &s4), [Kind::Closed]);
    }

    #[test]
    fn a_folded_messages_fate_has_its_event() {
        let mut done = ticket();
        let mut a = running("review-code", 1);
        a.kind = AttemptKind::Review;
        a.rewrite = Some(crate::ticket::Rewrite {
            mode: crate::history::Commits::Fold,
            before: "head0001".into(),
            after: Some("fold0001".into()),
            from: 3,
            to: 1,
            skipped: None,
            stale: Vec::new(),
            message: None,
            at_ms: 1,
        });
        done.attempts.push(a);
        let texts = |before: &Ticket, after: &Ticket| -> Vec<String> {
            between(Some(before), after, 5, &names)
                .into_iter()
                .map(|e| e.text)
                .collect()
        };
        let mut stale = done.clone();
        stale.attempts[0].rewrite.as_mut().unwrap().stale = vec![crate::ticket::StaleMessage {
            index: 0,
            subject: "A".into(),
            names: vec!["old_name".into(), "other".into()],
        }];
        assert_eq!(
            texts(&done, &stale),
            ["folded message names `old_name`, `other`, which the tree does not have"]
        );
        let mut asked = stale.clone();
        asked.attempts[0].rewrite.as_mut().unwrap().message = Some(crate::ticket::MessageFix {
            answer: "rewrite".into(),
            from: "fold0001".into(),
            ..crate::ticket::MessageFix::default()
        });
        assert!(
            texts(&stale, &asked).is_empty(),
            "an answer alone is not news"
        );
        let mut moved = asked.clone();
        moved.attempts[0]
            .rewrite
            .as_mut()
            .unwrap()
            .message
            .as_mut()
            .unwrap()
            .to = Some("word0001".into());
        assert_eq!(
            texts(&asked, &moved),
            ["message rewritten fold000 → word000"]
        );
        let mut kept = asked.clone();
        kept.attempts[0]
            .rewrite
            .as_mut()
            .unwrap()
            .message
            .as_mut()
            .unwrap()
            .answer = "accept".into();
        assert_eq!(texts(&asked, &kept), ["message kept as written"]);
        let mut kept_reworded = moved.clone();
        let m = kept_reworded.attempts[0]
            .rewrite
            .as_mut()
            .unwrap()
            .message
            .as_mut()
            .unwrap();
        m.failed = Some("the rewritten message still names `old_name`".into());
        m.answer = "accept".into();
        assert_eq!(
            texts(&moved, &kept_reworded),
            ["message kept as rewritten"],
            "a rewording that landed is what an accept keeps"
        );
    }

    fn event(t: &Ticket, kind: Kind) -> Event {
        Event::new(t, 1, kind, "plan", "x".into())
    }

    #[test]
    fn seqs_follow_the_last_line_and_a_torn_line_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(EVENTS_FILE);
        let t = ticket();
        let mut batch: Vec<Event> = (0..10).map(|_| event(&t, Kind::Round)).collect();
        append(&path, &mut batch).unwrap();
        let seqs: Vec<u64> = read_since(&path, 5)
            .unwrap()
            .iter()
            .map(|e| e.seq)
            .collect();
        assert_eq!(seqs, [6, 7, 8, 9, 10]);
        // A crash mid-append leaves half a line.
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(br#"{"v":1,"seq":11,"at"#).unwrap();
        drop(file);
        assert_eq!(read_since(&path, 0).unwrap().len(), 10);
        let mut more = [event(&t, Kind::Stage)];
        append(&path, &mut more).unwrap();
        assert_eq!(more[0].seq, 11, "the torn line had no seq to take");
        let all = read_since(&path, 0).unwrap();
        assert_eq!(all.len(), 11);
        assert_eq!(all.last().unwrap().kind, Kind::Stage);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"at\n{"), "the torn line stays its own");
        let mut follow = follow(&path, 9);
        let seqs: Vec<u64> = follow.next_batch().unwrap().iter().map(|e| e.seq).collect();
        assert_eq!(seqs, [10, 11]);
        assert!(follow.next_batch().unwrap().is_empty());
        let mut last = [event(&t, Kind::Closed)];
        append(&path, &mut last).unwrap();
        assert_eq!(follow.next_batch().unwrap()[0].seq, 12);
    }

    #[test]
    fn a_line_longer_than_the_window_still_gives_its_seq() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(EVENTS_FILE);
        let t = ticket();
        let mut first = [event(&t, Kind::Taken)];
        append(&path, &mut first).unwrap();
        let mut long = event(&t, Kind::Round);
        long.seq = 41;
        long.text = "y".repeat(12 * 1024);
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(format!("{}\n", long.to_line()).as_bytes())
            .unwrap();
        drop(file);
        let mut next = [event(&t, Kind::Stage)];
        append(&path, &mut next).unwrap();
        assert_eq!(next[0].seq, 42);
        assert_eq!(last_seq(&path).unwrap(), 42);
    }

    #[test]
    fn a_long_question_is_capped_on_a_character_boundary() {
        let mut t = ticket();
        let mut d = decision("d1");
        d.question = "é".repeat(1024);
        t.decisions.push(d);
        let e = Event::from_decision(&t, &t.decisions[0]);
        assert!(e.text.len() <= TEXT_CAP, "{}", e.text.len());
        assert!(e.text.len() > TEXT_CAP - 4);
        assert!(e.text.ends_with('…'));
        assert_eq!(capped("a\nb".into()), "a b");
    }

    /// An event whose write failed is never what a wait returns: first
    /// read before its `void`, it is not borne out by the record; read
    /// with it, it is dropped. Its `void` is never a match either.
    #[test]
    fn a_wait_never_matches_a_withdrawn_event_or_its_void() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let t = ticket();
        crate::store::write_ticket(&data.ticket_file(&t.id), &t).unwrap();
        let log = log_path(&data);
        for what in [For::Decision, For::Any, For::Move] {
            let mut phantom = t.clone();
            phantom.decisions.push(decision(&format!("d-{what:?}")));
            // Read by `now` and moved by `pause`: a `Cell` lets both
            // closures hold it at once.
            let clock = std::cell::Cell::new(0);
            let mut looks = 0;
            let waited = wait(
                &data,
                &t.id,
                what,
                None,
                Some(1_000),
                &mut || clock.get(),
                &mut || {
                    looks += 1;
                    clock.set(clock.get() + 250);
                    let mut events = between(Some(&t), &phantom, 1, &names);
                    if looks == 1 {
                        // The write's events, its void not yet appended.
                        append(&log, &mut events).unwrap();
                    } else if looks == 2 {
                        let seqs = vec![last_seq(&log).unwrap()];
                        append_void(&log, &events[0], seqs, "disk full").unwrap();
                    }
                },
            )
            .unwrap();
            assert_eq!(waited, Waited::TimedOut, "{what:?}");
        }
        let all = read_since(&log, 0).unwrap();
        assert_eq!(
            all.iter().map(|e| e.kind).collect::<Vec<_>>(),
            [
                Kind::Decision,
                Kind::Void,
                Kind::Decision,
                Kind::Void,
                Kind::Decision,
                Kind::Void
            ]
        );
    }

    /// A decision answered before the waiter reads its event is not
    /// returned to `--for decision`: its `dispatch decide` line would
    /// fail. `--for any` still sees it, as the event it is.
    #[test]
    fn a_decision_answered_before_it_is_read_is_passed_over() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let t = ticket();
        crate::store::write_ticket(&data.ticket_file(&t.id), &t).unwrap();
        let log = log_path(&data);
        for what in [For::Decision, For::Any, For::Move] {
            let mut asked = t.clone();
            asked.decisions.push(decision(&format!("d-{what:?}")));
            let mut answered = asked.clone();
            answered.decisions.last_mut().unwrap().state = DecisionState::Answered {
                answer: "finalize".into(),
                note: None,
                by: "dispatch".into(),
                at_ms: 4,
                acted: false,
            };
            // The answered record is a write past the ask's line, so it
            // has taken that line's write and contradicts it: dropped.
            answered.updated_ms = 2;
            let clock = std::cell::Cell::new(0);
            let mut looks = 0;
            let waited = wait(
                &data,
                &t.id,
                what,
                None,
                Some(1_000),
                &mut || clock.get(),
                &mut || {
                    looks += 1;
                    clock.set(clock.get() + 250);
                    if looks == 1 {
                        let mut events = between(Some(&t), &asked, 1, &names);
                        append(&log, &mut events).unwrap();
                        crate::store::write_ticket(&data.ticket_file(&t.id), &answered).unwrap();
                    }
                },
            )
            .unwrap();
            crate::store::write_ticket(&data.ticket_file(&t.id), &t).unwrap();
            match what {
                For::Decision => assert_eq!(waited, Waited::TimedOut),
                _ => assert!(
                    matches!(&waited, Waited::Matched(e) if e.kind == Kind::Decision),
                    "{waited:?}"
                ),
            }
        }
    }

    /// A ticket already parked or closed ends every wait at once, as a
    /// park or close seen while waiting does; only `--for closed` on a
    /// closed ticket is a match.
    #[test]
    fn a_wait_on_a_parked_or_closed_ticket_returns_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let mut parked = ticket();
        parked.state = TicketState::Parked {
            reason: "by hand".into(),
        };
        let mut closed = ticket();
        closed.state = TicketState::Closed {
            reason: "done".into(),
        };
        for (t, what, matched) in [
            (&closed, For::Any, false),
            (&closed, For::Stage, false),
            (&closed, For::Closed, true),
            (&parked, For::Closed, false),
            (&parked, For::Any, false),
            (&parked, For::Stage, false),
            (&parked, For::Pr, false),
            (&parked, For::Decision, false),
        ] {
            crate::store::write_ticket(&data.ticket_file(&t.id), t).unwrap();
            let waited = wait(&data, &t.id, what, None, None, &mut || 0, &mut || {
                panic!("{what:?} waited on a ticket that will not move")
            })
            .unwrap();
            assert_eq!(
                matches!(waited, Waited::Matched(_)),
                matched,
                "{what:?}: {waited:?}"
            );
            assert!(!matches!(waited, Waited::TimedOut));
        }
    }

    /// Waits `what` on `id` from `since` with a 1 s deadline; each look
    /// runs `step` with its count and moves the clock on 250 ms.
    fn wait_looking(
        data: &DataDir,
        id: &str,
        what: For,
        since: Option<u64>,
        step: &mut dyn FnMut(u32),
    ) -> Waited {
        let clock = std::cell::Cell::new(0);
        let mut looks = 0;
        wait(
            data,
            id,
            what,
            since,
            Some(1_000),
            &mut || clock.get(),
            &mut || {
                looks += 1;
                clock.set(clock.get() + 250);
                step(looks);
            },
        )
        .unwrap()
    }

    #[test]
    fn a_pending_decision_is_returned_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let mut t = ticket();
        t.decisions.push(decision("d1"));
        crate::store::write_ticket(&data.ticket_file(&t.id), &t).unwrap();
        let waited = wait(
            &data,
            &t.id,
            For::Decision,
            None,
            None,
            &mut || 0,
            &mut || panic!("waited on a decision already pending"),
        )
        .unwrap();
        let Waited::Matched(e) = waited else {
            panic!("{waited:?}");
        };
        assert_eq!(e.kind, Kind::Decision);
        assert_eq!(e.decision.as_deref(), Some("d1"));
    }

    #[test]
    fn a_wait_without_a_cursor_matches_the_next_move() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let t = ticket();
        let file = data.ticket_file(&t.id);
        crate::store::write_ticket(&file, &t).unwrap();
        let log = log_path(&data);
        let mut moved = t.clone();
        moved.stage = 1;
        // No pipeline copy, so the wait names stages by index: log them so.
        let waited = wait_looking(&data, &t.id, For::Stage, None, &mut |looks| {
            if looks == 1 {
                append(&log, &mut between(Some(&t), &moved, 1, &Vec::new)).unwrap();
                crate::store::write_ticket(&file, &moved).unwrap();
            }
        });
        assert!(
            matches!(&waited, Waited::Matched(e) if e.kind == Kind::Stage && e.stage == "#1"),
            "{waited:?}"
        );
    }

    /// A move made between taking the tail and waiting is returned from
    /// a cursor at once; without one, the wait is for the next move.
    #[test]
    fn a_cursor_replays_a_move_made_before_the_wait() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let t = ticket();
        let file = data.ticket_file(&t.id);
        crate::store::write_ticket(&file, &t).unwrap();
        let log = log_path(&data);
        let seq = last_seq(&log).unwrap();
        let mut moved = t.clone();
        moved.stage = 1;
        append(&log, &mut between(Some(&t), &moved, 1, &Vec::new)).unwrap();
        crate::store::write_ticket(&file, &moved).unwrap();
        let waited = wait(
            &data,
            &t.id,
            For::Stage,
            Some(seq),
            None,
            &mut || 0,
            &mut || panic!("waited on a move already made"),
        )
        .unwrap();
        assert!(
            matches!(&waited, Waited::Matched(e) if e.kind == Kind::Stage),
            "{waited:?}"
        );
        let waited = wait_looking(&data, &t.id, For::Stage, None, &mut |_| {});
        assert_eq!(waited, Waited::TimedOut);
    }

    /// A decision raised between two watches is returned by the watch
    /// that re-arms from the seq the last one printed; one armed at the
    /// tail misses it.
    #[test]
    fn a_watch_from_its_cursor_returns_a_decision_raised_since() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let t = ticket();
        let file = data.ticket_file(&t.id);
        crate::store::write_ticket(&file, &t).unwrap();
        let log = log_path(&data);
        let s = last_seq(&log).unwrap();
        let mut asked = t.clone();
        asked.decisions.push(decision("d1"));
        append(&log, &mut between(Some(&t), &asked, 1, &Vec::new)).unwrap();
        crate::store::write_ticket(&file, &asked).unwrap();
        let waited = wait_looking(&data, &t.id, For::Any, Some(s), &mut |_| {
            panic!("waited on a decision raised before the watch");
        });
        assert!(
            matches!(&waited, Waited::Matched(e)
                if e.kind == Kind::Decision && e.decision.as_deref() == Some("d1")),
            "{waited:?}"
        );
        let waited = wait_looking(&data, &t.id, For::Any, None, &mut |_| {});
        assert_eq!(waited, Waited::TimedOut);
    }

    /// A watch re-armed from the seq of a decision it returned does not
    /// return that decision again while it stays pending: it waits for
    /// the ticket's next event.
    #[test]
    fn a_watch_past_a_pending_decision_waits_for_the_next_event() {
        for what in [For::Any, For::Move] {
            let dir = tempfile::tempdir().unwrap();
            let data = DataDir::new(dir.path());
            let t = ticket();
            let file = data.ticket_file(&t.id);
            crate::store::write_ticket(&file, &t).unwrap();
            let log = log_path(&data);
            let mut asked = t.clone();
            asked.decisions.push(decision("d1"));
            append(&log, &mut between(Some(&t), &asked, 1, &Vec::new)).unwrap();
            crate::store::write_ticket(&file, &asked).unwrap();
            let d = read_since(&log, 0)
                .unwrap()
                .into_iter()
                .find(|e| e.kind == Kind::Decision)
                .unwrap()
                .seq;
            let waited = wait_looking(&data, &t.id, what, Some(d), &mut |_| {});
            assert_eq!(waited, Waited::TimedOut, "{what:?}");
            let mut moved = asked.clone();
            moved.stage = 1;
            let waited = wait_looking(&data, &t.id, what, Some(d), &mut |looks| {
                if looks == 1 {
                    append(&log, &mut between(Some(&asked), &moved, 2, &Vec::new)).unwrap();
                    crate::store::write_ticket(&file, &moved).unwrap();
                }
            });
            assert!(
                matches!(&waited, Waited::Matched(e) if e.kind == Kind::Stage && e.stage == "#1"),
                "{what:?}: {waited:?}"
            );
        }
    }

    /// A line is appended before its record is renamed in, so a wait can
    /// read it while the record still lacks it: live or replayed, of any
    /// kind, it is held until the record takes its write. The record
    /// before it may stand at the line's own millisecond, as the first of
    /// two saves in one millisecond does.
    #[test]
    fn a_live_event_read_before_its_rename_is_held_until_it_lands() {
        let mut t = ticket();
        t.attempts.push(running("implement", 1));
        let mut asked = t.clone();
        asked.decisions.push(decision("d1"));
        asked.updated_ms = 50;
        let mut nudged = t.clone();
        nudged.attempts[0].nudges.push(7);
        nudged.updated_ms = 50;
        let mut moved = t.clone();
        moved.stage = 1;
        moved.updated_ms = 50;
        let by_name: &dyn Fn() -> Vec<String> = &names;
        let by_index: &dyn Fn() -> Vec<String> = &Vec::new;
        for prior in [40, 50] {
            t.updated_ms = prior;
            // Live cases. The stage case is logged by index, as the wait
            // names stages without a pipeline copy.
            for (what, next, named) in [
                (For::Any, &asked, by_name),
                (For::Decision, &asked, by_name),
                (For::Any, &nudged, by_name),
                (For::Stage, &moved, by_index),
            ] {
                let dir = tempfile::tempdir().unwrap();
                let data = DataDir::new(dir.path());
                let file = data.ticket_file(&t.id);
                let log = log_path(&data);
                crate::store::write_ticket(&file, &t).unwrap();
                let mut logged = None;
                let mut looked = 0;
                let waited = wait_looking(&data, &t.id, what, None, &mut |looks| {
                    looked = looks;
                    if looks == 1 {
                        let mut events = between(Some(&t), next, 50, named);
                        assert_eq!(events.len(), 1, "{what:?} after {prior}");
                        append(&log, &mut events).unwrap();
                        logged = Some(events.remove(0));
                    } else if looks == 2 {
                        crate::store::write_ticket(&file, next).unwrap();
                    }
                });
                let logged = logged.unwrap();
                assert_eq!(waited, Waited::Matched(logged), "{what:?} after {prior}");
                assert_eq!(looked, 2, "{what:?} after {prior}: looks");
            }
            // A replayed decision whose write lands after the wait begins.
            let dir = tempfile::tempdir().unwrap();
            let data = DataDir::new(dir.path());
            let file = data.ticket_file(&t.id);
            let log = log_path(&data);
            crate::store::write_ticket(&file, &t).unwrap();
            let mut events = between(Some(&t), &asked, 50, &names);
            assert_eq!(events.len(), 1);
            append(&log, &mut events).unwrap();
            let event = events.remove(0);
            let mut looked = 0;
            let waited = wait_looking(&data, &t.id, For::Any, Some(event.seq - 1), &mut |looks| {
                looked = looks;
                if looks == 1 {
                    crate::store::write_ticket(&file, &asked).unwrap();
                }
            });
            assert_eq!(waited, Waited::Matched(event), "replayed after {prior}");
            assert_eq!(looked, 1, "replayed after {prior}: looks");
        }
    }

    /// A held line the record passes and contradicts is dropped, so the
    /// record coming back to agree with it later does not return it.
    #[test]
    fn a_held_event_the_record_overtakes_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let mut t = ticket();
        t.attempts.push(running("implement", 1));
        t.updated_ms = 40;
        let file = data.ticket_file(&t.id);
        let log = log_path(&data);
        crate::store::write_ticket(&file, &t).unwrap();
        let mut moved = t.clone();
        moved.stage = 1;
        moved.updated_ms = 50;
        let mut past = t.clone();
        past.stage = 2;
        past.updated_ms = 60;
        let mut back = t.clone();
        back.stage = 1;
        back.updated_ms = 70;
        let waited = wait_looking(&data, &t.id, For::Stage, None, &mut |looks| match looks {
            1 => {
                let mut events = between(Some(&t), &moved, 50, &Vec::new);
                assert_eq!(events.len(), 1);
                append(&log, &mut events).unwrap();
            }
            2 => crate::store::write_ticket(&file, &past).unwrap(),
            3 => crate::store::write_ticket(&file, &back).unwrap(),
            _ => {}
        });
        assert_eq!(waited, Waited::TimedOut);
    }

    /// A replayed event of a kind the record does not check by itself is
    /// returned once the record's `updated_ms` reaches its write, and
    /// never once its `void` is read, even when a later write lands.
    #[test]
    fn a_replayed_event_waits_for_its_write_to_land() {
        let mut started = ticket();
        started.attempts.push(running("implement", 1));
        started.updated_ms = 40;
        let mut nudged = started.clone();
        nudged.attempts[0].nudges.push(7);
        nudged.updated_ms = 50;
        // A write after the failed one, which would bear the event out
        // by its time alone if the `void` were not honoured.
        let mut later = started.clone();
        later.updated_ms = 60;
        // Landed, in flight, failed while waiting, failed before.
        for case in 0..4 {
            let dir = tempfile::tempdir().unwrap();
            let data = DataDir::new(dir.path());
            let file = data.ticket_file(&started.id);
            let log = log_path(&data);
            let mut events = between(Some(&started), &nudged, 50, &names);
            assert_eq!(events.len(), 1);
            append(&log, &mut events).unwrap();
            let event = events[0].clone();
            let cursor = event.seq - 1;
            let on_disk = if case == 0 { &nudged } else { &started };
            crate::store::write_ticket(&file, on_disk).unwrap();
            if case == 3 {
                append_void(&log, &event, vec![event.seq], "disk full").unwrap();
                crate::store::write_ticket(&file, &later).unwrap();
            }
            let mut looked = 0;
            let waited = wait_looking(&data, &started.id, For::Any, Some(cursor), &mut |looks| {
                looked = looks;
                if looks == 1 && case == 1 {
                    crate::store::write_ticket(&file, &nudged).unwrap();
                } else if looks == 1 && case == 2 {
                    append_void(&log, &event, vec![event.seq], "disk full").unwrap();
                    crate::store::write_ticket(&file, &later).unwrap();
                }
            });
            match case {
                0 | 1 => {
                    assert_eq!(waited, Waited::Matched(event), "case {case}");
                    assert_eq!(looked, case, "case {case}: looks");
                }
                _ => assert_eq!(waited, Waited::TimedOut, "case {case}"),
            }
        }
    }

    /// A decision raised after the wait starts, through the runner's
    /// write, is returned with the time the record carries.
    #[test]
    fn a_stamped_decision_is_matched_at_the_records_time() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let mut t = ticket();
        t.attempts.push(running("plan", 1));
        crate::store::write_ticket_stamped(&data, &mut t, 10).unwrap();
        let waited = wait_looking(&data, "t1", For::Decision, None, &mut |looks| {
            if looks == 1 {
                t.decisions.push(decision("d1"));
                crate::store::write_ticket_stamped(&data, &mut t, 50).unwrap();
            }
        });
        let Waited::Matched(e) = waited else {
            panic!("{waited:?}")
        };
        assert_eq!(e.decision.as_deref(), Some("d1"));
        assert_eq!(e.at_ms, 50);
        let back = crate::store::read_ticket(&data.ticket_file(&t.id)).unwrap();
        assert_eq!(back.updated_ms, e.at_ms);
    }

    /// A write of poll bookkeeping alone keeps `updated_ms`, so it does
    /// not let a replayed event through; the event's `void` drops it.
    #[test]
    fn a_quiet_write_does_not_let_a_held_event_through() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let log = log_path(&data);
        let mut started = ticket();
        started.attempts.push(running("implement", 1));
        crate::store::write_ticket_stamped(&data, &mut started, 40).unwrap();
        let mut nudged = started.clone();
        nudged.attempts[0].nudges.push(7);
        let mut events = between(Some(&started), &nudged, 50, &names);
        append(&log, &mut events).unwrap();
        let event = events[0].clone();
        let waited = wait_looking(&data, "t1", For::Any, Some(event.seq - 1), &mut |looks| {
            if looks == 1 {
                started.attempts[0].polls_since_stop += 1;
                crate::store::write_ticket_stamped(&data, &mut started, 60).unwrap();
                assert_eq!(started.updated_ms, 40);
            } else if looks == 2 {
                append_void(&log, &event, vec![event.seq], "disk full").unwrap();
            }
        });
        assert_eq!(waited, Waited::TimedOut);
    }

    /// A `decision` line for `d1` on `t` at 50, appended with no rename
    /// and no `void`, as a write whose rename and withdrawal both failed
    /// leaves it; the record it would have written, and the line.
    fn phantom_decision_on(data: &DataDir, t: &Ticket) -> (Ticket, Event) {
        let mut asked = t.clone();
        asked.decisions.push(decision("d1"));
        asked.updated_ms = 50;
        let mut events = between(Some(t), &asked, 50, &names);
        assert_eq!(events.len(), 1);
        append(&log_path(data), &mut events).unwrap();
        (asked, events.remove(0))
    }

    /// The same decision asked again after a phantom of it returns the
    /// line of the write that landed, even with a quiet write between.
    #[test]
    fn a_reasked_decision_returns_the_write_that_landed_not_its_phantom() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let mut t = ticket();
        t.attempts.push(running("plan", 1));
        crate::store::write_ticket_stamped(&data, &mut t, 40).unwrap();
        let mut phantom = None;
        let waited = wait_looking(&data, "t1", For::Decision, None, &mut |looks| match looks {
            1 => phantom = Some(phantom_decision_on(&data, &t).1),
            2 => {
                t.attempts[0].polls_since_stop += 1;
                crate::store::write_ticket_stamped(&data, &mut t, 60).unwrap();
            }
            3 => {
                t.decisions.push(decision("d1"));
                crate::store::write_ticket_stamped(&data, &mut t, 70).unwrap();
            }
            _ => {}
        });
        let Waited::Matched(e) = waited else {
            panic!("{waited:?}")
        };
        let phantom = phantom.unwrap();
        assert!(e.seq > phantom.seq, "{e:?}");
        assert_eq!(e.decision.as_deref(), Some("d1"));
        assert_eq!(e.at_ms, 70);
    }

    /// A phantom decision a later write contradicts is dropped, as it is
    /// by any write that moves `updated_ms` past it; asking it later
    /// returns that line.
    #[test]
    fn a_phantom_decision_a_real_write_contradicts_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let mut t = ticket();
        t.attempts.push(running("plan", 1));
        crate::store::write_ticket_stamped(&data, &mut t, 40).unwrap();
        let mut phantom = None;
        let waited = wait_looking(&data, "t1", For::Decision, None, &mut |looks| match looks {
            1 => phantom = Some(phantom_decision_on(&data, &t).1),
            2 => {
                t.attempts[0].nudges.push(65);
                crate::store::write_ticket_stamped(&data, &mut t, 65).unwrap();
                assert_eq!(t.updated_ms, 65);
            }
            3 => {
                t.decisions.push(decision("d1"));
                crate::store::write_ticket_stamped(&data, &mut t, 80).unwrap();
            }
            _ => {}
        });
        let Waited::Matched(e) = waited else {
            panic!("{waited:?}")
        };
        assert!(e.seq > phantom.unwrap().seq, "{e:?}");
        assert_eq!(e.at_ms, 80);
    }

    /// A held event whose own write lands late, with no later line for
    /// the same transition, is still the one returned.
    #[test]
    fn a_held_event_whose_write_lands_late_is_returned() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let mut t = ticket();
        t.attempts.push(running("plan", 1));
        crate::store::write_ticket_stamped(&data, &mut t, 40).unwrap();
        let mut phantom = None;
        let waited = wait_looking(&data, "t1", For::Decision, None, &mut |looks| match looks {
            1 => phantom = Some(phantom_decision_on(&data, &t)),
            2 => {
                let (asked, _) = phantom.as_ref().unwrap();
                crate::store::write_ticket(&data.ticket_file(&t.id), asked).unwrap();
            }
            _ => {}
        });
        assert_eq!(waited, Waited::Matched(phantom.unwrap().1));
    }

    /// A replayed phantom the record already bears out at the first look
    /// gives way to the re-ask that landed.
    #[test]
    fn a_replayed_phantom_borne_out_at_once_returns_the_write_that_landed() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let mut t = ticket();
        t.attempts.push(running("plan", 1));
        crate::store::write_ticket_stamped(&data, &mut t, 40).unwrap();
        let (_, phantom) = phantom_decision_on(&data, &t);
        t.decisions.push(decision("d1"));
        crate::store::write_ticket_stamped(&data, &mut t, 70).unwrap();
        let waited = wait_looking(&data, "t1", For::Any, Some(phantom.seq - 1), &mut |_| {});
        let Waited::Matched(e) = waited else {
            panic!("{waited:?}")
        };
        assert!(e.seq > phantom.seq, "{e:?}");
        assert_eq!(e.kind, Kind::Decision);
        assert_eq!(e.at_ms, 70);
    }

    /// A held event whose write lands late is returned even when a second
    /// transition of its kind follows before the next look.
    #[test]
    fn a_held_nudge_is_returned_before_the_next_nudge() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let log = log_path(&data);
        let mut t = ticket();
        t.attempts.push(running("implement", 1));
        crate::store::write_ticket_stamped(&data, &mut t, 40).unwrap();
        let mut first = None;
        let waited = wait_looking(&data, "t1", For::Any, None, &mut |looks| match looks {
            1 => {
                let mut nudged = t.clone();
                nudged.attempts[0].nudges.push(50);
                nudged.updated_ms = 50;
                let mut events = between(Some(&t), &nudged, 50, &names);
                append(&log, &mut events).unwrap();
                first = Some(events.remove(0));
                t = nudged;
            }
            2 => {
                crate::store::write_ticket(&data.ticket_file(&t.id), &t).unwrap();
                t.attempts[0].nudges.push(60);
                crate::store::write_ticket_stamped(&data, &mut t, 60).unwrap();
            }
            _ => {}
        });
        let first = first.unwrap();
        assert_eq!(first.kind, Kind::Nudged);
        assert_eq!(waited, Waited::Matched(first));
    }

    /// A cursor taken at the tail just before a resume returns the
    /// resume; one from before an older event of the ticket returns that.
    #[test]
    fn a_cursor_at_the_tail_returns_the_resume_not_an_older_event() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let file = data.ticket_file("t1");
        let log = log_path(&data);
        let mut asked = ticket();
        asked.state = TicketState::Parked {
            reason: "by hand".into(),
        };
        asked.decisions.push(decision("d1"));
        let mut answered = asked.clone();
        answered.decisions[0].state = DecisionState::Answered {
            answer: "finalize".into(),
            note: None,
            by: "dispatch".into(),
            at_ms: 30,
            acted: false,
        };
        answered.updated_ms = 30;
        let mut events = between(Some(&asked), &answered, 30, &names);
        append(&log, &mut events).unwrap();
        let answer = events[0].clone();
        assert_eq!(answer.kind, Kind::Answered);
        crate::store::write_ticket(&file, &answered).unwrap();
        let tail = last_seq(&log).unwrap();
        let mut resumed = answered.clone();
        resumed.state = TicketState::Active;
        resumed.updated_ms = 50;
        append(&log, &mut between(Some(&answered), &resumed, 50, &names)).unwrap();
        crate::store::write_ticket(&file, &resumed).unwrap();
        for (cursor, kind) in [(tail, Kind::Resumed), (answer.seq - 1, Kind::Answered)] {
            let waited = wait(
                &data,
                "t1",
                For::Any,
                Some(cursor),
                None,
                &mut || 0,
                &mut || panic!("waited on a cursor with events after it"),
            )
            .unwrap();
            assert!(
                matches!(&waited, Waited::Matched(e) if e.kind == kind),
                "{cursor}: {waited:?}"
            );
        }
    }

    /// A parked ticket and the same ticket resumed, the resume
    /// answering a `rerun` about its cancelled implementer.
    fn parked_and_resumed_with_a_rerun() -> (Ticket, Ticket) {
        let mut parked = ticket();
        parked.state = TicketState::Parked {
            reason: "by hand".into(),
        };
        let mut a = running("implement", 1);
        a.state = AttemptState::Cancelled {
            reason: "by hand".into(),
        };
        parked.attempts.push(a);
        let mut resumed = parked.clone();
        resumed.state = TicketState::Active;
        resumed.updated_ms = 50;
        resumed.decisions.push(Decision {
            name: "rerun".into(),
            stage: "implement".into(),
            attempt: Some(("implement".into(), 1)),
            state: DecisionState::Answered {
                answer: "rerun".into(),
                note: None,
                by: BY_RESUME.into(),
                at_ms: 50,
                acted: false,
            },
            ..decision("d1")
        });
        (parked, resumed)
    }

    #[test]
    fn a_resume_names_its_reruns_and_comes_before_them() {
        let (parked, resumed) = parked_and_resumed_with_a_rerun();
        let events = between(Some(&parked), &resumed, 5, &names);
        assert_eq!(
            events.iter().map(|e| e.kind).collect::<Vec<_>>(),
            [Kind::Resumed, Kind::Decision, Kind::Answered]
        );
        assert_eq!(events[0].text, "active again, rerunning implement (root)");
        let mut plain = parked.clone();
        plain.state = TicketState::Active;
        let events = between(Some(&parked), &plain, 5, &names);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].text, "active again");
    }

    /// `--for any` from a cursor taken on the parked ticket returns the
    /// resume, never the `rerun` question the resume wrote already
    /// answered.
    #[test]
    fn a_wait_for_any_across_a_resume_returns_the_resume() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let (parked, resumed) = parked_and_resumed_with_a_rerun();
        let file = data.ticket_file(&parked.id);
        crate::store::write_ticket(&file, &parked).unwrap();
        let log = log_path(&data);
        let tail = last_seq(&log).unwrap();
        append(&log, &mut between(Some(&parked), &resumed, 50, &names)).unwrap();
        crate::store::write_ticket(&file, &resumed).unwrap();
        let waited = wait(
            &data,
            &parked.id,
            For::Any,
            Some(tail),
            None,
            &mut || 0,
            &mut || panic!("waited on a cursor with events after it"),
        )
        .unwrap();
        assert!(
            matches!(&waited, Waited::Matched(e) if e.kind == Kind::Resumed),
            "{waited:?}"
        );
    }

    /// A restart's write renumbers the stage and turns the parking
    /// ticket active: one `restarted` line says so, with no `sent-back`
    /// or `resumed` beside it, and `--for stage` waits for it.
    #[test]
    fn a_restart_is_one_restarted_event_in_place_of_the_move_and_the_resume() {
        let mut old = ticket();
        old.stage = 2;
        old.state = TicketState::Parking {
            reason: "restarting at plan".into(),
        };
        let mut new = ticket();
        new.restarts.push(crate::ticket::Restart {
            at_ms: 9,
            from: "merge".into(),
            to: "plan".into(),
            before: "/t/pipeline.toml".into(),
            after: "/t/pipeline.2.toml".into(),
            discarded: vec![],
            reset: vec![],
            setup_again: vec![],
            note: None,
        });
        let events = between(Some(&old), &new, 9, &names);
        assert_eq!(
            events.iter().map(|e| e.kind).collect::<Vec<_>>(),
            [Kind::Restarted]
        );
        assert_eq!(events[0].stage, "plan");
        assert_eq!(events[0].text, "at plan from merge under pipeline.2.toml");
        assert!(For::Stage.candidate(Kind::Restarted));
        // Held, the restart writes `parked` and no restart.
        let mut held = old.clone();
        held.state = TicketState::Parked {
            reason: "restart at plan held: dirty".into(),
        };
        assert_eq!(kinds(Some(&old), &held), [Kind::Parked]);
        // A line whose write did not land is not borne out, though a
        // plain restart's stage reads the same before it lands.
        let confirms =
            |fresh: &Ticket| confirmed(&events[0], &old, fresh, For::Stage, &names(), false);
        assert!(confirms(&new));
        let mut unlanded = old.clone();
        unlanded.stage = new.stage;
        assert!(!confirms(&unlanded));
        assert!(!confirms(&held));
    }

    #[test]
    fn a_line_of_another_format_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(EVENTS_FILE);
        let t = ticket();
        let mut future = event(&t, Kind::Stage);
        future.v = 2;
        future.seq = 1;
        std::fs::write(&path, format!("{}\nnot json\n", future.to_line())).unwrap();
        assert!(read_since(&path, 0).unwrap().is_empty());
        let mut next = [event(&t, Kind::Stage)];
        append(&path, &mut next).unwrap();
        assert_eq!(next[0].seq, 2, "its seq still counts");
    }

    /// A bring-up of `backend` at 100, with `conflict` commits
    /// conflicting since 5 when some.
    fn brought_up_at_100(commits: bool, conflict: Option<usize>) -> crate::ticket::Refreshed {
        crate::ticket::Refreshed {
            from: "base0000".into(),
            to: "main0000".into(),
            commits,
            notes: None,
            at_ms: 100,
            conflict: conflict.map(|k| crate::ticket::RefreshConflict {
                before: "head0000".into(),
                from: "base0000".into(),
                to: "main0000".into(),
                commits: (0..k).map(|i| format!("c{i}")).collect(),
                stage: 0,
                at_ms: 5,
            }),
            after: None,
        }
    }

    /// The `refreshed` event `t` makes once `r` is its lane's bring-up.
    fn refreshed(t: &Ticket, r: crate::ticket::Refreshed) -> Event {
        let mut new = t.clone();
        new.lanes[0].refreshed = Some(r);
        between(Some(t), &new, 5, &names)
            .into_iter()
            .find(|e| e.kind == Kind::Refreshed)
            .expect("a refreshed event")
    }

    fn rebaser(n: u32, context: &str, state: AttemptState, started_ms: u64) -> Attempt {
        new_attempt(
            "refresh",
            n,
            context,
            AttemptKind::Agent,
            state,
            BTreeMap::new(),
            started_ms,
        )
    }

    fn answered(name: &str, answer: &str, attempt: Option<u32>, at_ms: u64) -> Decision {
        Decision {
            id: format!("{name}-{at_ms}"),
            stage: "implement".into(),
            name: name.into(),
            kind: DecisionKind::Permission,
            question: "?".into(),
            options: vec![answer.into(), "park".into()],
            recommendation: None,
            attempt: attempt.map(|n| ("refresh".into(), n)),
            state: DecisionState::Answered {
                answer: answer.into(),
                note: None,
                by: "cli".into(),
                at_ms,
                acted: true,
            },
            made_ms: at_ms,
            refusals: Vec::new(),
        }
    }

    fn with(attempts: Vec<Attempt>, decisions: Vec<Decision>) -> Ticket {
        let mut t = ticket();
        t.attempts = attempts;
        t.decisions = decisions;
        t
    }

    fn says(e: &Event) -> (&str, Option<BroughtUpBy>, Option<u32>) {
        let words = e.text.split_once(", ").map_or("", |(_, w)| w);
        (words, e.by, e.conflicts)
    }

    #[test]
    fn a_refreshed_event_says_who_brought_the_lane_up_and_how_many_commits_conflicted() {
        use BroughtUpBy::{Git, Rebaser};
        let t = ticket();
        let moved = refreshed(&t, brought_up_at_100(false, None));
        assert_eq!(
            moved.text,
            "backend from base000 to main000, brought up with no commits of its own"
        );
        assert_eq!((moved.by, moved.conflicts), (Some(Git), Some(0)));
        assert_eq!(
            says(&refreshed(&t, brought_up_at_100(true, None))),
            ("rebased cleanly", Some(Git), Some(0))
        );

        let done = with(
            vec![rebaser(1, "backend", AttemptState::Complete, 10)],
            vec![],
        );
        assert_eq!(
            says(&refreshed(&done, brought_up_at_100(true, Some(2)))),
            (
                "rebased by the rebaser, conflicts in 2 commits",
                Some(Rebaser),
                Some(2)
            )
        );
        assert_eq!(
            says(&refreshed(&done, brought_up_at_100(true, Some(0)))),
            (
                "rebased by the rebaser, conflicts in commits it could not list",
                Some(Rebaser),
                Some(0)
            )
        );
        let mut noted = brought_up_at_100(true, None);
        noted.notes = Some("/d/t1/notes.md".into());
        assert_eq!(
            says(&refreshed(&done, noted)),
            ("rebased by the rebaser", Some(Rebaser), None)
        );
    }

    #[test]
    fn a_refreshed_event_names_root_for_the_tree_and_a_restored_bring_up_is_not_one() {
        let t = ticket();
        let mut up = t.clone();
        up.tree_refreshed = Some(brought_up_at_100(false, None));
        let events: Vec<Event> = between(Some(&t), &up, 5, &names)
            .into_iter()
            .filter(|e| e.kind == Kind::Refreshed)
            .collect();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].text,
            "root from base000 to main000, brought up with no commits of its own"
        );
        assert_eq!(
            (events[0].by, events[0].conflicts, events[0].head.as_deref()),
            (Some(BroughtUpBy::Git), Some(0), Some("main0000"))
        );

        let quiet = |old: &Ticket, new: &Ticket| {
            !between(Some(old), new, 5, &names)
                .iter()
                .any(|e| e.kind == Kind::Refreshed)
        };
        assert!(quiet(&up, &t));
        let mut later = up.clone();
        later.tree_refreshed = Some(crate::ticket::Refreshed {
            to: "main0002".into(),
            at_ms: 200,
            ..brought_up_at_100(false, None)
        });
        assert!(quiet(&later, &up));
        assert!(!quiet(&up, &later));
    }

    #[test]
    fn a_refreshed_event_after_a_conflict_says_whether_a_hand_rebase_was_adopted() {
        use BroughtUpBy::{Hand, Rebaser, Stopped};
        let failed = AttemptState::Failed {
            reason: "gone".into(),
        };
        let stopped_words = (
            "rebased after the rebaser stopped, conflicts in 2 commits",
            Some(Stopped),
            Some(2),
        );
        let mut withdrawn = answered("rerun", "rerun", Some(1), 20);
        withdrawn.state = DecisionState::Cancelled;
        for t in [
            with(vec![rebaser(1, "backend", failed.clone(), 10)], vec![]),
            with(
                vec![rebaser(
                    1,
                    "backend",
                    AttemptState::Cancelled {
                        reason: "parked".into(),
                    },
                    10,
                )],
                vec![],
            ),
            with(
                vec![rebaser(1, "backend", failed.clone(), 10)],
                vec![withdrawn],
            ),
            with(
                vec![rebaser(1, "backend", failed.clone(), 10)],
                vec![answered("rerun", "park", Some(1), 20)],
            ),
        ] {
            assert_eq!(
                says(&refreshed(&t, brought_up_at_100(true, Some(2)))),
                stopped_words
            );
        }

        let hand_words = (
            "rebased by hand (adopted), conflicts in 1 commit",
            Some(Hand),
            Some(1),
        );
        for t in [
            with(
                vec![rebaser(1, "backend", failed.clone(), 10)],
                vec![answered("rerun", "rerun", Some(1), 20)],
            ),
            with(
                vec![rebaser(1, "backend", AttemptState::Complete, 10)],
                vec![answered("refresh", "recheck", None, 20)],
            ),
            with(vec![], vec![answered("refresh", "recheck", None, 20)]),
            with(
                vec![rebaser(1, "backend", AttemptState::Complete, 10)],
                vec![answered("rerun", "rerun", Some(1), 20)],
            ),
        ] {
            assert_eq!(
                says(&refreshed(&t, brought_up_at_100(true, Some(1)))),
                hand_words
            );
        }

        let relaunched = with(
            vec![
                rebaser(1, "backend", failed.clone(), 10),
                rebaser(2, "backend", AttemptState::Complete, 30),
            ],
            vec![answered("rerun", "rerun", Some(1), 20)],
        );
        assert_eq!(
            says(&refreshed(&relaunched, brought_up_at_100(true, Some(1)))).1,
            Some(Rebaser)
        );
        let elsewhere = with(
            vec![
                rebaser(1, "backend", AttemptState::Complete, 10),
                rebaser(2, "frontend", failed.clone(), 15),
            ],
            vec![answered("rerun", "rerun", Some(2), 20)],
        );
        assert_eq!(
            says(&refreshed(&elsewhere, brought_up_at_100(true, Some(1)))).1,
            Some(Rebaser)
        );
        let later = with(
            vec![rebaser(1, "backend", AttemptState::Complete, 10)],
            vec![answered("refresh", "recheck", None, 200)],
        );
        assert_eq!(
            says(&refreshed(&later, brought_up_at_100(true, Some(1)))).1,
            Some(Rebaser),
            "an answer after the bring-up does not count"
        );
    }

    #[test]
    fn a_rebaser_from_before_the_conflict_did_not_bring_the_lane_up() {
        // The rebaser served an earlier bring-up; the conflict came at 50
        // with max_rebases spent, and its question was withdrawn by a
        // park before a hand rebase was read.
        let t = with(
            vec![rebaser(1, "backend", AttemptState::Complete, 10)],
            vec![],
        );
        let mut r = brought_up_at_100(true, Some(2));
        r.conflict.as_mut().unwrap().at_ms = 50;
        assert_eq!(
            says(&refreshed(&t, r.clone())),
            (
                "rebased by hand (adopted), conflicts in 2 commits",
                Some(BroughtUpBy::Hand),
                Some(2)
            )
        );

        let relaunched = with(
            vec![
                rebaser(1, "backend", AttemptState::Complete, 10),
                rebaser(2, "backend", AttemptState::Complete, 50),
            ],
            vec![],
        );
        assert_eq!(
            says(&refreshed(&relaunched, r.clone())).1,
            Some(BroughtUpBy::Rebaser),
            "a rebaser launched in the pass that recorded the conflict counts"
        );

        r.conflict.as_mut().unwrap().at_ms = 0;
        assert_eq!(
            says(&refreshed(&t, r)).1,
            Some(BroughtUpBy::Rebaser),
            "a conflict from before its time was kept bounds nothing"
        );
    }

    #[test]
    fn a_refreshed_event_keeps_who_and_how_many_through_a_line() {
        let mut t = ticket();
        t.attempts
            .push(rebaser(1, "backend", AttemptState::Complete, 10));
        let e = refreshed(&t, brought_up_at_100(true, Some(2)));
        let line = e.to_line();
        assert!(line.contains(r#""by":"rebaser""#), "{line}");
        let back: Event = serde_json::from_str(&line).unwrap();
        assert_eq!(
            (back.by, back.conflicts),
            (Some(BroughtUpBy::Rebaser), Some(2))
        );

        let old = r#"{"v":1,"seq":3,"at_ms":5,"ticket":"t1","project":"p","stage":"","kind":"refreshed","text":"backend from base000 to main000, rebased","head":"main0000"}"#;
        let back: Event = serde_json::from_str(old).unwrap();
        assert_eq!((back.by, back.conflicts), (None, None));
    }

    /// Bursts `what` on `id` from `since` with a 30 s deadline; each look
    /// runs `step` with its count and moves the clock on 250 ms, one
    /// count and one clock across every follow-up `wait`.
    fn burst_looking(
        data: &DataDir,
        id: &str,
        what: For,
        since: Option<u64>,
        step: &mut dyn FnMut(u32, u64),
    ) -> Burst {
        let clock = std::cell::Cell::new(0);
        let mut looks = 0;
        wait_burst(
            data,
            id,
            what,
            since,
            Some(30_000),
            &mut || clock.get(),
            &mut || {
                looks += 1;
                clock.set(clock.get() + 250);
                step(looks, clock.get());
            },
        )
        .unwrap()
    }

    fn kinds_of(lines: &[Event]) -> Vec<Kind> {
        lines.iter().map(|e| e.kind).collect()
    }

    fn pr_record(n: u64) -> PullRequestRecord {
        PullRequestRecord {
            provider: "github".into(),
            repo: "o/r".into(),
            number: n,
            url: format!("https://example.com/pr/{n}"),
            head: "head0001".into(),
            checks: "pending".into(),
            checked_ms: 0,
            error_since_ms: None,
            merge_commit: None,
        }
    }

    /// A stage change logs several lines within a second or two; one
    /// watch returns them all, and a line past the settle is the next
    /// watch's. Look 14 is 2.5 s past the last line: the clock closes the
    /// window before it, and the line's own time would leave its nudge
    /// out if it did not.
    #[test]
    fn a_burst_within_the_settle_is_one_return_and_a_line_after_it_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let mut t = ticket();
        t.attempts.push(running("plan", 1));
        crate::store::write_ticket_stamped(&data, &mut t, 10).unwrap();
        let mut reached = 0;
        let burst = burst_looking(&data, "t1", For::Any, None, &mut |looks, at| {
            reached = looks;
            match looks {
                1 => t.attempts[0].state = AttemptState::Complete,
                2 => t.stage = 1,
                3 => t.attempts.push(running("implement", 1)),
                4 | 14 => t.attempts[1].nudges.push(at),
                _ => return,
            }
            crate::store::write_ticket_stamped(&data, &mut t, at).unwrap();
        });
        let Burst::Lines(lines) = burst else {
            panic!("{burst:?}");
        };
        assert_eq!(
            kinds_of(&lines),
            [
                Kind::AttemptEnded,
                Kind::Stage,
                Kind::AttemptStarted,
                Kind::Nudged
            ]
        );
        assert!(reached < 14, "{reached}");
        assert!(lines.windows(2).all(|w| w[0].seq < w[1].seq));
        let last = lines.last().unwrap().seq;
        t.attempts[1].nudges.push(5_000);
        crate::store::write_ticket_stamped(&data, &mut t, 5_000).unwrap();
        let burst = burst_looking(&data, "t1", For::Any, Some(last), &mut |_, _| {});
        let Burst::Lines(lines) = burst else {
            panic!("{burst:?}");
        };
        assert_eq!(kinds_of(&lines), [Kind::Nudged]);
        assert!(lines[0].seq > last);
    }

    /// A ticket that keeps logging does not hold the watch open: the
    /// burst closes `SETTLE_CAP_MS` after its first line.
    #[test]
    fn a_burst_closes_at_the_cap_on_a_ticket_that_keeps_logging() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let mut t = ticket();
        t.attempts.push(running("plan", 1));
        crate::store::write_ticket_stamped(&data, &mut t, 10).unwrap();
        let burst = burst_looking(&data, "t1", For::Any, None, &mut |_, at| {
            if at % 1_000 == 0 && at <= 15_000 {
                t.attempts[0].nudges.push(at);
                crate::store::write_ticket_stamped(&data, &mut t, at).unwrap();
            }
        });
        let Burst::Lines(lines) = burst else {
            panic!("{burst:?}");
        };
        assert_eq!(lines[0].at_ms, 1_000);
        assert_eq!(lines.len(), 11, "{lines:?}");
        assert!(
            lines
                .iter()
                .all(|e| e.kind == Kind::Nudged && e.at_ms <= 1_000 + SETTLE_CAP_MS)
        );
    }

    /// Lines replayed from an old cursor all arrive at once, so their own
    /// times close the window: a line more than `SETTLE_MS` after the
    /// last one, or `SETTLE_CAP_MS` after the first, is the next watch's.
    #[test]
    fn a_replayed_burst_closes_by_its_lines_times() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let mut t = ticket();
        t.attempts.push(running("plan", 1));
        crate::store::write_ticket_stamped(&data, &mut t, 10).unwrap();
        let since = last_seq(&log_path(&data)).unwrap();
        let mut at = 1_000;
        for gap in [
            1_000, 3_000, 1_500, 1_500, 1_500, 1_500, 1_500, 1_500, 1_500,
        ] {
            at += gap;
            t.attempts[0].nudges.push(at);
            crate::store::write_ticket_stamped(&data, &mut t, at).unwrap();
        }
        let mut cursor = since;
        let mut times = Vec::new();
        for _ in 0..3 {
            let burst = burst_looking(&data, "t1", For::Any, Some(cursor), &mut |_, _| {});
            let Burst::Lines(lines) = burst else {
                panic!("{burst:?}");
            };
            cursor = lines.last().unwrap().seq;
            times.push(lines.iter().map(|e| e.at_ms).collect::<Vec<_>>());
        }
        assert_eq!(
            times,
            [
                vec![2_000],
                vec![5_000, 6_500, 8_000, 9_500, 11_000, 12_500, 14_000],
                vec![15_500],
            ]
        );
    }

    /// `--for move` passes over an attempt's start and returns on a
    /// stage line; the table holds the kinds it returns on and the ones
    /// it does not.
    #[test]
    fn a_move_watch_returns_on_a_stage_line_and_not_on_an_attempt_start() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let mut t = ticket();
        crate::store::write_ticket_stamped(&data, &mut t, 10).unwrap();
        let burst = burst_looking(&data, "t1", For::Move, None, &mut |looks, at| {
            if looks == 1 {
                t.attempts.push(running("plan", 1));
                crate::store::write_ticket_stamped(&data, &mut t, at).unwrap();
            }
        });
        assert_eq!(burst, Burst::TimedOut);
        let burst = burst_looking(&data, "t1", For::Move, None, &mut |looks, at| {
            if looks == 1 {
                t.stage = 1;
                crate::store::write_ticket_stamped(&data, &mut t, at).unwrap();
            }
        });
        let Burst::Lines(lines) = burst else {
            panic!("{burst:?}");
        };
        assert_eq!(kinds_of(&lines), [Kind::Stage]);

        // Replayed lines the record bears out, one kind at a time.
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let mut t = ticket();
        t.attempts.push(running("plan", 1));
        t.attempts[0].pr = Some(pr_record(3));
        t.decisions.push(decision("d1"));
        t.restarts.push(crate::ticket::Restart {
            at_ms: 50,
            from: "#0".into(),
            to: "#0".into(),
            before: "/t/pipeline.toml".into(),
            after: "/t/pipeline.2.toml".into(),
            discarded: vec![],
            reset: vec![],
            setup_again: vec![],
            note: None,
        });
        t.updated_ms = 100;
        crate::store::write_ticket(&data.ticket_file(&t.id), &t).unwrap();
        let log = log_path(&data);
        for (kind, returned) in [
            (Kind::SentBack, true),
            (Kind::Restarted, true),
            (Kind::Pr, true),
            (Kind::PrChecks, true),
            (Kind::Decision, true),
            (Kind::Round, false),
            (Kind::Rewrite, false),
            (Kind::Nudged, false),
            (Kind::Answered, false),
        ] {
            let since = last_seq(&log).unwrap();
            let mut e = Event::new(&t, 50, kind, "#0", "x".into());
            e.decision = Some("d1".into());
            e.url = Some("https://example.com/pr/3".into());
            e.attempt = Some(("plan".into(), 1));
            append(&log, std::slice::from_mut(&mut e)).unwrap();
            let burst = burst_looking(&data, "t1", For::Move, Some(since), &mut |_, _| {});
            if returned {
                assert_eq!(burst, Burst::Lines(vec![e]), "{kind:?}");
            } else {
                assert_eq!(burst, Burst::TimedOut, "{kind:?}");
            }
        }
    }

    /// A runner pass cannot wait: `next_burst` holds a burst while it
    /// may still grow, and returns it once it settles or reaches the cap.
    /// A park the subscriber already has is not returned again.
    #[test]
    fn next_burst_holds_an_open_burst_until_it_settles_or_hits_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let log = log_path(&data);
        let mut t = ticket();
        crate::store::write_ticket_stamped(&data, &mut t, 10).unwrap();
        let since = last_seq(&log).unwrap();
        assert_eq!(next_burst(&data, "t1", since, false, 500).unwrap(), None);
        t.stage = 1;
        crate::store::write_ticket_stamped(&data, &mut t, 1_000).unwrap();
        assert_eq!(next_burst(&data, "t1", since, false, 1_500).unwrap(), None);
        let lines = next_burst(&data, "t1", since, false, 3_001)
            .unwrap()
            .unwrap();
        assert_eq!(kinds_of(&lines), [Kind::Stage]);

        // A ticket that keeps moving: held while the first line is
        // within the cap, then returned up to it.
        t.updated_ms = 25_000;
        crate::store::write_ticket(&data.ticket_file(&t.id), &t).unwrap();
        let since = last_seq(&log).unwrap();
        for at in (10_000..=20_500).step_by(1_500) {
            let mut e = Event::new(&t, at, Kind::PrChecks, "#1", "pending".into());
            append(&log, std::slice::from_mut(&mut e)).unwrap();
        }
        assert_eq!(next_burst(&data, "t1", since, false, 19_500).unwrap(), None);
        let lines = next_burst(&data, "t1", since, false, 21_000)
            .unwrap()
            .unwrap();
        assert_eq!(lines.len(), 7, "{lines:?}");
        assert!(lines.iter().all(|e| e.at_ms <= 10_000 + SETTLE_CAP_MS));

        let since = last_seq(&log).unwrap();
        t.state = TicketState::Parked {
            reason: "by hand".into(),
        };
        crate::store::write_ticket_stamped(&data, &mut t, 30_000).unwrap();
        let lines = next_burst(&data, "t1", since, false, 30_001)
            .unwrap()
            .unwrap();
        assert_eq!(lines.last().unwrap().kind, Kind::Parked);
        let parked = last_seq(&log).unwrap();
        assert_eq!(next_burst(&data, "t1", parked, true, 40_000).unwrap(), None);
        // Seen, but delivered from an older cursor: the line is news.
        assert!(
            next_burst(&data, "t1", since, true, 40_000)
                .unwrap()
                .is_some()
        );
    }

    /// A decision ends a burst at once: nothing after it is looked for.
    #[test]
    fn a_decision_ends_a_burst_at_once() {
        for (what, want) in [
            (For::Any, vec![Kind::AttemptEnded, Kind::Decision]),
            (For::Move, vec![Kind::Decision]),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let data = DataDir::new(dir.path());
            let mut t = ticket();
            t.attempts.push(running("plan", 1));
            crate::store::write_ticket_stamped(&data, &mut t, 10).unwrap();
            let burst = burst_looking(&data, "t1", what, None, &mut |looks, at| {
                match looks {
                    1 => t.attempts[0].state = AttemptState::Complete,
                    2 => t.decisions.push(decision("d1")),
                    _ => panic!("{what:?} looked on after the decision"),
                }
                crate::store::write_ticket_stamped(&data, &mut t, at).unwrap();
            });
            let Burst::Lines(lines) = burst else {
                panic!("{what:?}: {burst:?}");
            };
            assert_eq!(kinds_of(&lines), want, "{what:?}");
        }
    }

    /// A line taken into a burst whose `void` lands inside the window is
    /// left out of what the burst returns.
    #[test]
    fn a_line_withdrawn_inside_the_window_is_left_out_of_the_burst() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let log = log_path(&data);
        let mut t = ticket();
        t.attempts.push(running("plan", 1));
        crate::store::write_ticket_stamped(&data, &mut t, 10).unwrap();
        let burst = burst_looking(&data, "t1", For::Any, None, &mut |looks, at| match looks {
            1 | 3 => {
                t.attempts[0].nudges.push(at);
                crate::store::write_ticket_stamped(&data, &mut t, at).unwrap();
            }
            2 => {
                let first = read_since(&log, 0)
                    .unwrap()
                    .into_iter()
                    .rfind(|e| e.kind == Kind::Nudged)
                    .unwrap();
                let seqs = vec![first.seq];
                append_void(&log, &first, seqs, "disk full").unwrap();
            }
            _ => {}
        });
        let Burst::Lines(lines) = burst else {
            panic!("{burst:?}");
        };
        assert_eq!(kinds_of(&lines), [Kind::Nudged]);
        assert_eq!(lines[0].at_ms, 750);
    }

    /// The ticket a park reaches inside the window: the follow-up `wait`
    /// reads the parked record first, and the lines logged before the
    /// park are still returned, then the park.
    #[test]
    fn a_park_inside_the_window_keeps_the_lines_logged_before_it() {
        let parked = |t: &mut Ticket| {
            t.state = TicketState::Parked {
                reason: "by hand".into(),
            };
        };
        for what in [For::Any, For::Move] {
            // The park logged.
            let dir = tempfile::tempdir().unwrap();
            let data = DataDir::new(dir.path());
            let mut t = ticket();
            t.attempts.push(running("plan", 1));
            crate::store::write_ticket_stamped(&data, &mut t, 10).unwrap();
            let burst = burst_looking(&data, "t1", what, None, &mut |looks, at| {
                if looks == 1 {
                    t.stage = 1;
                    crate::store::write_ticket_stamped(&data, &mut t, at).unwrap();
                    t.attempts[0].pr = Some(pr_record(3));
                    crate::store::write_ticket_stamped(&data, &mut t, at + 1).unwrap();
                    parked(&mut t);
                    crate::store::write_ticket_stamped(&data, &mut t, at + 2).unwrap();
                }
            });
            let Burst::Ended { lines, end } = burst else {
                panic!("{what:?}: {burst:?}");
            };
            assert_eq!(kinds_of(&lines), [Kind::Stage, Kind::Pr], "{what:?}");
            assert_eq!(end.kind, Kind::Parked, "{what:?}");
            assert!(end.seq > lines[1].seq, "{what:?}");

            // No park line: the end is made from the record.
            let dir = tempfile::tempdir().unwrap();
            let data = DataDir::new(dir.path());
            let mut t = ticket();
            t.attempts.push(running("plan", 1));
            crate::store::write_ticket_stamped(&data, &mut t, 10).unwrap();
            let burst = burst_looking(&data, "t1", what, None, &mut |looks, at| {
                if looks == 1 {
                    t.stage = 1;
                    crate::store::write_ticket_stamped(&data, &mut t, at).unwrap();
                    t.attempts[0].pr = Some(pr_record(3));
                    crate::store::write_ticket_stamped(&data, &mut t, at + 1).unwrap();
                    parked(&mut t);
                    t.updated_ms = at + 2;
                    crate::store::write_ticket(&data.ticket_file(&t.id), &t).unwrap();
                }
            });
            let Burst::Ended { lines, end } = burst else {
                panic!("{what:?}: {burst:?}");
            };
            assert_eq!(kinds_of(&lines), [Kind::Stage, Kind::Pr], "{what:?}");
            assert_eq!((end.kind, end.seq), (Kind::Parked, 0), "{what:?}");

            // A decision among the lines before the park still ends the
            // burst at once; the next watch returns the rest and the park.
            let dir = tempfile::tempdir().unwrap();
            let data = DataDir::new(dir.path());
            let mut t = ticket();
            t.attempts.push(running("plan", 1));
            crate::store::write_ticket_stamped(&data, &mut t, 10).unwrap();
            let burst = burst_looking(&data, "t1", what, None, &mut |looks, at| {
                if looks == 1 {
                    t.stage = 1;
                    crate::store::write_ticket_stamped(&data, &mut t, at).unwrap();
                    t.decisions.push(decision("d1"));
                    crate::store::write_ticket_stamped(&data, &mut t, at + 1).unwrap();
                    t.attempts[0].pr = Some(pr_record(3));
                    crate::store::write_ticket_stamped(&data, &mut t, at + 2).unwrap();
                    parked(&mut t);
                    t.decisions[0].state = DecisionState::Cancelled;
                    crate::store::write_ticket_stamped(&data, &mut t, at + 3).unwrap();
                }
            });
            let Burst::Lines(lines) = burst else {
                panic!("{what:?}: {burst:?}");
            };
            assert_eq!(kinds_of(&lines), [Kind::Stage, Kind::Decision], "{what:?}");
            // Parked, so the decision no longer waits: no `decide` line.
            let fresh = read_ticket(&data.ticket_file("t1")).unwrap();
            assert!(fresh.waiting_on_you().is_empty());
            let from = lines[1].seq;
            let burst = burst_looking(&data, "t1", what, Some(from), &mut |_, _| {
                panic!("{what:?} followed a parked ticket")
            });
            let Burst::Ended { lines, end } = burst else {
                panic!("{what:?}: {burst:?}");
            };
            let want: &[Kind] = if what == For::Any {
                &[Kind::Pr, Kind::DecisionCancelled]
            } else {
                &[Kind::Pr]
            };
            assert_eq!(kinds_of(&lines), want, "{what:?}");
            assert_eq!(end.kind, Kind::Parked, "{what:?}");
        }
    }

    /// A watch re-armed from an older cursor on a ticket already parked
    /// returns the lines between the cursor and the park, then the park;
    /// the other filters, and a watch with no cursor, end at once.
    #[test]
    fn a_rearm_on_a_parked_ticket_from_an_older_cursor_returns_the_lines_then_the_park() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let log = log_path(&data);
        let mut t = ticket();
        t.attempts.push(running("plan", 1));
        crate::store::write_ticket_stamped(&data, &mut t, 10).unwrap();
        let cursor = last_seq(&log).unwrap();
        t.stage = 1;
        crate::store::write_ticket_stamped(&data, &mut t, 20).unwrap();
        t.attempts[0].pr = Some(pr_record(3));
        crate::store::write_ticket_stamped(&data, &mut t, 30).unwrap();
        t.state = TicketState::Parked {
            reason: "by hand".into(),
        };
        crate::store::write_ticket_stamped(&data, &mut t, 40).unwrap();
        for (what, since, want) in [
            (For::Any, Some(cursor), vec![Kind::Stage, Kind::Pr]),
            (For::Move, Some(cursor), vec![Kind::Stage, Kind::Pr]),
            (For::Stage, Some(cursor), vec![]),
            (For::Any, None, vec![]),
        ] {
            let burst = burst_looking(&data, "t1", what, since, &mut |_, _| {
                panic!("{what:?} followed a parked ticket")
            });
            let Burst::Ended { lines, end } = burst else {
                panic!("{what:?}: {burst:?}");
            };
            assert_eq!(kinds_of(&lines), want, "{what:?} from {since:?}");
            assert_eq!(end.kind, Kind::Parked, "{what:?} from {since:?}");
        }
    }
}
