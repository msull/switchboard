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
        out.push(Event::new(
            new,
            at_ms,
            Kind::Taken,
            "",
            format!("{} {}", new.source.label(), new.source.title),
        ));
        name_stages(&mut out, &whole, new, None, names);
        return out;
    };
    let mut moved: Option<Kind> = None;
    if new.stage != old.stage {
        let kind = if new.stage > old.stage {
            Kind::Stage
        } else {
            Kind::SentBack
        };
        moved = Some(kind);
        whole.push(out.len());
        out.push(Event::new(new, at_ms, kind, "", String::new()));
    }
    for a in &new.attempts {
        let before = old
            .attempts
            .iter()
            .find(|x| x.stage == a.stage && x.n == a.n);
        attempt_events(&mut out, new, a, before, at_ms);
    }
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
    if std::mem::discriminant(&new.state) != std::mem::discriminant(&old.state) {
        let event = match state_kind(&new.state) {
            Some((kind, reason)) => Some((kind, reason.to_owned())),
            None if matches!(
                old.state,
                TicketState::Parked { .. } | TicketState::Parking { .. }
            ) =>
            {
                Some((Kind::Resumed, "active again".to_owned()))
            }
            None => None,
        };
        if let Some((kind, text)) = event {
            whole.push(out.len());
            out.push(Event::new(new, at_ms, kind, "", text));
        }
    }
    if !out.is_empty() {
        name_stages(&mut out, &whole, new, moved.map(|_| old.stage), names);
    }
    out
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

/// Names as a message shows them: "`a`, `b`".
pub(crate) fn names_list(names: &[String]) -> String {
    names
        .iter()
        .map(|n| format!("`{n}`"))
        .collect::<Vec<_>>()
        .join(", ")
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

/// One `check-orphan-killed` event per check group a previous runner
/// left running that was signalled since `before`.
fn orphan_events(
    out: &mut Vec<Event>,
    t: &Ticket,
    a: &Attempt,
    before: Option<&Attempt>,
    at_ms: u64,
) {
    let seen = before.map_or(0, |b| b.orphans_killed.len());
    for o in a.orphans_killed.iter().skip(seen) {
        let text = format!(
            "checks at {} left running by a previous runner (group {}) stopped",
            short(&o.head),
            o.pgid
        );
        out.push(Event {
            head: Some(o.head.clone()),
            ..Event::of_attempt(t, at_ms, Kind::CheckOrphanKilled, a, text)
        });
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
    match &d.state {
        DecisionState::Answered {
            answer, note, by, ..
        } if !answered_before => {
            let note = note.as_ref().map_or(String::new(), |n| format!(" — {n}"));
            out.push(Event::of_decision(
                t,
                at_ms,
                Kind::Answered,
                d,
                format!("{} by {by}: {answer}{note}", d.name),
            ));
        }
        DecisionState::Cancelled if !cancelled_before => {
            out.push(Event::of_decision(
                t,
                at_ms,
                Kind::DecisionCancelled,
                d,
                d.name.clone(),
            ));
        }
        _ => {}
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
    /// A stage move, forward or sent back.
    Stage,
    /// A pull request bound to an attempt.
    Pr,
    /// The close.
    Closed,
    /// The next event of any kind.
    Any,
}

impl For {
    /// `decision`, `stage`, `pr`, `closed` or `any`.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "decision" => Self::Decision,
            "stage" => Self::Stage,
            "pr" => Self::Pr,
            "closed" => Self::Closed,
            "any" => Self::Any,
            _ => return None,
        })
    }

    fn candidate(self, kind: Kind) -> bool {
        match self {
            Self::Decision => kind == Kind::Decision,
            Self::Stage => matches!(kind, Kind::Stage | Kind::SentBack),
            Self::Pr => kind == Kind::Pr,
            Self::Closed => kind == Kind::Closed,
            Self::Any => kind != Kind::Void,
        }
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
/// `void` is not read yet. An event replayed from before the wait, of a
/// kind with no record check of its own, is held until the record's
/// `updated_ms` reaches its write or its `void` drops it.
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
    // when a write lands between the reads of one batch.
    let landed = |held: &mut Vec<Event>, fresh: &Ticket| {
        held.iter()
            .position(|e| confirmed(e, &before, fresh, what, &names, true))
            .map(|i| held.remove(i))
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
            if let Some(e) = landed(&mut held, &fresh) {
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
                if replayed && !checked(e.kind) {
                    held.push(e);
                }
                continue;
            }
            if let Some(older) = landed(&mut held, &fresh) {
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

/// Whether `confirmed` checks an event of `kind` against the record
/// itself, rather than against what changed since the wait began: the
/// kinds with an arm of their own there. Change the two together.
fn checked(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Decision | Kind::Stage | Kind::SentBack | Kind::Pr | Kind::Closed | Kind::Parked
    )
}

/// What the record already says before any event is read: a pending
/// decision, or a ticket parked or closed, which ends every wait but
/// `--for closed` on a closed one, as a park or close seen while waiting
/// does.
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
fn confirmed(
    e: &Event,
    before: &Ticket,
    fresh: &Ticket,
    what: For,
    names: &[String],
    replayed: bool,
) -> bool {
    // Every kind with an arm of its own here is one `checked` lists.
    match e.kind {
        Kind::Decision => e.decision.as_ref().is_some_and(|id| {
            if what == For::Decision {
                fresh.waiting_on_you().iter().any(|d| &d.id == id)
            } else {
                fresh.decisions.iter().any(|d| &d.id == id)
            }
        }),
        Kind::Stage | Kind::SentBack => stage_name(names, fresh.stage) == e.stage,
        Kind::Pr => e.url.as_ref().is_some_and(|url| {
            fresh
                .attempts
                .iter()
                .any(|a| a.pr.as_ref().is_some_and(|pr| &pr.url == url))
        }),
        Kind::Closed => matches!(fresh.state, TicketState::Closed { .. }),
        Kind::Parked => matches!(fresh.state, TicketState::Parked { .. }),
        kind => {
            what == For::Any
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
            state: TicketState::Active,
            close: CloseProgress::default(),
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
        for what in [For::Decision, For::Any] {
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
            [Kind::Decision, Kind::Void, Kind::Decision, Kind::Void]
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
        for what in [For::Decision, For::Any] {
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
}
