//! What a ticket's sessions spent: tokens by model and time in turns,
//! read from Claude Code's transcripts after the fact. Only counts,
//! model names and timestamps are kept, in `<ticket dir>/usage.json`;
//! never message content.
//!
//! A transcript writes one message once per content block, every record
//! with the same `message.id` and the usage growing across them, so the
//! last record per id is the message. A clone starts as a copy of its
//! source's transcript, timestamps included, so a session counts only
//! from the time the attempt naming it started. Agent-tool subagents
//! write files of their own beside the transcript, folded into the
//! session that spawned them.
//!
//! The counting is pure: `scan` reads one file's text, `sessions_of`
//! lists what a ticket names, `attribute` sums. `rescan` and
//! `bring_up_to_date` are the I/O around them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use dispatch_control::{
    AttemptUsage, SessionUsage, StageUsage, Tokens, UsageSource, UsageView, add_models,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::pipeline::{OperatorKind, Pipeline};
use crate::scheduler::REFRESH;
use crate::store::{self, DataDir};
use crate::ticket::{Attempt, AttemptKind, ReviewRound, Ticket, TranscriptRef};

/// The model Claude Code names on messages it made up itself (an
/// interruption, an error); they cost nothing.
const SYNTHETIC: &str = "<synthetic>";

/// The tool whose call waits on the user's answer.
const ASK_TOOL: &str = "AskUserQuestion";

/// A ticket's counts cache: one entry per transcript file read, keyed by
/// its path.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cache {
    /// See `store::USAGE_VERSION`.
    pub version: u32,
    /// Keyed by the transcript's absolute path.
    pub files: BTreeMap<String, FileUsage>,
}

/// One transcript file's rows, as last read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FileUsage {
    /// The file's length in bytes when read.
    pub size: u64,
    /// The file's modification time when read, in ms since the epoch.
    pub mtime_ms: u64,
    /// When this entry was made, so two writers keep the newer one.
    pub read_ms: u64,
    /// The file has vanished since; the rows are kept.
    pub gone: bool,
    /// One per `message.id`, its last record's usage.
    pub messages: Vec<MessageRow>,
    /// A human prompt to the turn's last record.
    pub turns: Vec<Span>,
    /// An `AskUserQuestion` call to its result.
    pub asks: Vec<Span>,
}

/// One message's usage.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MessageRow {
    /// The last record's timestamp, in ms since the epoch.
    pub at_ms: u64,
    /// The model the message names.
    pub model: String,
    /// Uncached input tokens (`input_tokens`); cache writes and reads
    /// are not in it.
    pub input: u64,
    /// Output tokens.
    pub output: u64,
    /// Tokens written to the cache, at any lifetime.
    pub cache_write: u64,
    /// The part of `cache_write` cached for an hour.
    pub cache_write_1h: u64,
    /// Tokens read from the cache.
    pub cache_read: u64,
    /// Kept for a price table later: a fast message costs more.
    pub speed: Option<String>,
}

/// A stretch of time, in ms.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    /// Since the epoch.
    pub start_ms: u64,
    /// Since the epoch; never before `start_ms`.
    pub end_ms: u64,
}

/// One transcript's rows. Lines that are not JSON, and records of other
/// types, are passed over. Sidechain records and records before the
/// first prompt count: they were paid for.
#[must_use]
pub fn scan(text: &str) -> FileUsage {
    let mut messages: Vec<MessageRow> = Vec::new();
    let mut by_id: BTreeMap<String, usize> = BTreeMap::new();
    let mut turns: Vec<Span> = Vec::new();
    let mut turn: Option<Span> = None;
    let mut asking: BTreeMap<String, u64> = BTreeMap::new();
    let mut asks: Vec<Span> = Vec::new();
    for line in text.lines() {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let at = record
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_time);
        let message = &record["message"];
        match record.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                let model = message["model"].as_str().unwrap_or_default();
                if let Some(id) = message["id"].as_str()
                    && model != SYNTHETIC
                {
                    let row = row_of(message, model, at.unwrap_or(0));
                    if let Some(&i) = by_id.get(id) {
                        messages[i] = row;
                    } else {
                        by_id.insert(id.to_owned(), messages.len());
                        messages.push(row);
                    }
                }
                for block in blocks(message) {
                    if block["type"] == "tool_use"
                        && block["name"] == ASK_TOOL
                        && let (Some(id), Some(at)) = (block["id"].as_str(), at)
                    {
                        asking.insert(id.to_owned(), at);
                    }
                }
            }
            Some("user") => {
                let results: Vec<&Value> = blocks(message)
                    .filter(|b| b["type"] == "tool_result")
                    .collect();
                if results.is_empty() {
                    let meta = record["isMeta"].as_bool().unwrap_or(false);
                    if !meta && let Some(at) = at {
                        turns.extend(turn.take());
                        turn = Some(Span {
                            start_ms: at,
                            end_ms: at,
                        });
                    }
                }
                for r in results {
                    if let (Some(start), Some(end)) = (
                        r["tool_use_id"].as_str().and_then(|id| asking.remove(id)),
                        at,
                    ) {
                        asks.push(Span {
                            start_ms: start,
                            end_ms: end.max(start),
                        });
                    }
                }
            }
            _ => continue,
        }
        if let (Some(t), Some(at)) = (turn.as_mut(), at) {
            t.end_ms = t.end_ms.max(at);
        }
    }
    turns.extend(turn);
    FileUsage {
        messages,
        turns,
        asks,
        ..FileUsage::default()
    }
}

/// The content blocks of a message; none when its content is a string.
fn blocks(message: &Value) -> impl Iterator<Item = &Value> {
    message["content"].as_array().into_iter().flatten()
}

fn row_of(message: &Value, model: &str, at_ms: u64) -> MessageRow {
    let usage = &message["usage"];
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    MessageRow {
        at_ms,
        model: model.to_owned(),
        input: n(&usage["input_tokens"]),
        output: n(&usage["output_tokens"]),
        cache_write: n(&usage["cache_creation_input_tokens"]),
        cache_write_1h: n(&usage["cache_creation"]["ephemeral_1h_input_tokens"]),
        cache_read: n(&usage["cache_read_input_tokens"]),
        speed: usage["speed"].as_str().map(str::to_owned),
    }
}

/// An RFC 3339 time as ms since the epoch.
fn parse_time(text: &str) -> Option<u64> {
    let t = chrono::DateTime::parse_from_rfc3339(text).ok()?;
    u64::try_from(t.timestamp_millis()).ok()
}

/// One session a ticket names, with the window its rows count in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    /// The naming attempt's stage.
    pub stage: String,
    /// The naming attempt's number within its stage.
    pub n: u32,
    /// The naming attempt's context.
    pub context: String,
    /// See `SessionUsage::role`.
    pub role: String,
    /// `claude`, `codex` or `command`, when anything says.
    pub kind: Option<String>,
    /// `kind` came from the pipeline copy.
    pub kind_from_pipeline: bool,
    /// Switchboard's record id; none for a command reviewer.
    pub session: Option<String>,
    /// The attempt's start, or the round's for a round's sessions.
    pub from_ms: u64,
    /// Where the next naming of the same record id starts.
    pub until_ms: Option<u64>,
    /// The attempt's or round's end, for a session whose span is all
    /// that is known.
    pub ended_ms: Option<u64>,
}

/// Every session the ticket names, in attempt order. A plan review's
/// own `session` is the card's face, the planner or the reviewer, so it
/// is left to `run_sessions`; within one attempt a record id is listed
/// once, under its more specific role.
#[must_use]
pub fn sessions_of(t: &Ticket, p: Option<&Pipeline>) -> Vec<Named> {
    let mut out: Vec<Named> = t.attempts.iter().flat_map(|a| named_in(t, p, a)).collect();
    // One record id named twice (a session two attempts share) is split
    // at the second naming.
    let starts: Vec<(Option<String>, u64)> =
        out.iter().map(|x| (x.session.clone(), x.from_ms)).collect();
    for x in out.iter_mut().filter(|x| x.session.is_some()) {
        x.until_ms = starts
            .iter()
            .filter(|(s, from)| s == &x.session && *from > x.from_ms)
            .map(|(_, from)| *from)
            .min();
    }
    out
}

/// One attempt's sessions, as `sessions_of` lists them.
fn named_in(t: &Ticket, p: Option<&Pipeline>, a: &Attempt) -> Vec<Named> {
    let stage = p.and_then(|p| p.stages.iter().find(|s| s.name == a.stage));
    let operator_kind =
        |name: Option<&String>| -> Option<OperatorKind> { Some(p?.operators.get(name?)?.kind) };
    let mut n = Namer {
        t,
        a,
        mine: Vec::new(),
    };
    if a.kind != AttemptKind::Workflow
        && let Some(session) = &a.session
    {
        let (role, guess) = if a.stage == REFRESH {
            ("rebaser", p.and_then(|p| p.policy.rebaser.as_ref()))
        } else {
            ("agent", stage.and_then(|s| s.operator.as_ref()))
        };
        n.name(
            role.to_owned(),
            Some(session.clone()),
            a.agent.clone(),
            operator_kind(guess),
            a.started_ms,
            a.ended_ms,
        );
    }
    for (key, run) in &a.run_sessions {
        let (role, guess) = match key.as_str() {
            "reviewer" => (
                "plan reviewer".to_owned(),
                p.zip(stage.and_then(|s| s.review.as_ref()))
                    .and_then(|(p, r)| Some(p.operators.get(r)?.review.as_ref()?.reviewer)),
            ),
            "planner" => (
                "planner".to_owned(),
                operator_kind(planner_operator(p, stage.and_then(|s| s.subject.as_ref()))),
            ),
            other => (other.to_owned(), None),
        };
        n.name(
            role,
            Some(run.session.clone()),
            Some(run.agent.clone()).filter(|k| !k.is_empty()),
            guess,
            a.started_ms,
            a.ended_ms,
        );
    }
    let implementer = operator_kind(stage.and_then(|s| s.implementer.as_ref()));
    for round in &a.rounds {
        n.round(round, implementer);
    }
    if let Some(session) = a
        .rewrite
        .as_ref()
        .and_then(|w| w.message.as_ref())
        .and_then(|m| m.session.clone())
    {
        n.name(
            "rewriter".to_owned(),
            Some(session),
            None,
            implementer,
            a.started_ms,
            a.ended_ms,
        );
    }
    n.mine
}

/// One attempt's sessions as they are named.
struct Namer<'a> {
    t: &'a Ticket,
    a: &'a Attempt,
    mine: Vec<Named>,
}

impl Namer<'_> {
    /// A session named, its kind resolved; a record id already named in
    /// this attempt keeps the more specific of the two roles.
    fn name(
        &mut self,
        role: String,
        session: Option<String>,
        recorded: Option<String>,
        guess: Option<OperatorKind>,
        from_ms: u64,
        ended_ms: Option<u64>,
    ) {
        let (kind, kind_from_pipeline) = kind_of(self.t, recorded, session.as_deref(), guess);
        let named = Named {
            stage: self.a.stage.clone(),
            n: self.a.n,
            context: self.a.context.clone(),
            role,
            kind,
            kind_from_pipeline,
            session,
            from_ms,
            until_ms: None,
            ended_ms,
        };
        let same = named.session.as_ref().and_then(|id| {
            self.mine
                .iter()
                .position(|x| x.session.as_ref() == Some(id))
        });
        match same {
            Some(i) if matches!(self.mine[i].role.as_str(), "agent" | "rebaser") => {
                self.mine[i] = named;
            }
            Some(_) => {}
            None => self.mine.push(named),
        }
    }

    /// A review round's reviewers that started, and its fixer.
    fn round(&mut self, round: &ReviewRound, implementer: Option<OperatorKind>) {
        for r in &round.reviewers {
            // A reviewer that never started spent nothing.
            if r.session.is_none() && !(r.kind == "command" && r.launched) {
                continue;
            }
            self.name(
                format!("round {} reviewer {}", round.n, r.name),
                r.session.clone(),
                Some(r.kind.clone()),
                None,
                round.started_ms,
                round.ended_ms,
            );
        }
        if let Some(fixer) = &round.implementer {
            self.name(
                format!("round {} fixer", round.n),
                Some(fixer.clone()),
                None,
                implementer,
                round.started_ms,
                round.ended_ms,
            );
        }
    }
}

/// The plan review's planner is a clone of the agent that wrote the
/// subject: the operator of the stage whose `writes` name it.
fn planner_operator<'a>(p: Option<&'a Pipeline>, subject: Option<&String>) -> Option<&'a String> {
    let subject = subject?;
    p?.stages
        .iter()
        .find(|s| s.writes.iter().any(|w| &w.name == subject))?
        .operator
        .as_ref()
}

/// A session's kind: recorded beside its id, else from any transcript
/// seen for it, else from the pipeline copy (marked as such).
fn kind_of(
    t: &Ticket,
    recorded: Option<String>,
    session: Option<&str>,
    guess: Option<OperatorKind>,
) -> (Option<String>, bool) {
    if let Some(kind) = recorded {
        return (Some(kind), false);
    }
    if let Some(r) = session
        .and_then(|id| t.transcripts.get(id))
        .and_then(|refs| refs.first())
    {
        return (Some(r.agent.clone()), false);
    }
    match guess {
        Some(kind) => (Some(kind.word().to_owned()), true),
        None => (None, false),
    }
}

/// The main transcript paths of a record id's Claude refs.
fn claude_paths(refs: Option<&Vec<TranscriptRef>>) -> impl Iterator<Item = &Path> {
    refs.into_iter()
        .flatten()
        .filter(|r| r.agent == "claude")
        .filter_map(|r| r.path.as_deref())
}

/// Where a transcript's subagents write theirs: `<path minus .jsonl>/subagents/`.
#[must_use]
pub fn subagents_dir(main: &Path) -> PathBuf {
    main.with_extension("").join("subagents")
}

/// Each named session's tokens and time, grouped by attempt in the
/// order named. A missing transcript is `NotFound`, never zero.
#[must_use]
pub fn attribute(
    named: &[Named],
    refs: &BTreeMap<String, Vec<TranscriptRef>>,
    cache: &Cache,
    now_ms: u64,
) -> Vec<AttemptUsage> {
    let mut out: Vec<AttemptUsage> = Vec::new();
    for x in named {
        let s = session_usage(x, refs, cache, now_ms);
        let i = out
            .iter()
            .position(|a| a.stage == x.stage && a.n == x.n && a.context == x.context)
            .unwrap_or_else(|| {
                out.push(AttemptUsage {
                    stage: x.stage.clone(),
                    n: x.n,
                    context: x.context.clone(),
                    ..AttemptUsage::default()
                });
                out.len() - 1
            });
        add_models(&mut out[i].models, &s.models);
        out[i].sessions.push(s);
    }
    out
}

fn session_usage(
    x: &Named,
    refs: &BTreeMap<String, Vec<TranscriptRef>>,
    cache: &Cache,
    now_ms: u64,
) -> SessionUsage {
    let mut s = SessionUsage {
        role: x.role.clone(),
        kind: x.kind.clone().unwrap_or_default(),
        kind_from_pipeline: x.kind_from_pipeline,
        session: x.session.clone(),
        ..SessionUsage::default()
    };
    match x.kind.as_deref() {
        None => {
            s.source = UsageSource::KindUnknown;
            return s;
        }
        Some("claude") => {}
        Some(_) => {
            s.source = UsageSource::NotCounted;
            s.span_ms = x.ended_ms.unwrap_or(now_ms).saturating_sub(x.from_ms);
            return s;
        }
    }
    let mains: Vec<&Path> = claude_paths(x.session.as_ref().and_then(|id| refs.get(id))).collect();
    let mut found: Vec<(&FileUsage, bool)> = Vec::new();
    for main in &mains {
        let key = main.to_string_lossy();
        if let Some(f) = cache.files.get(key.as_ref()) {
            found.push((f, true));
        }
        let prefix = format!("{}/", subagents_dir(main).to_string_lossy());
        found.extend(
            cache
                .files
                .range(prefix.clone()..)
                .take_while(|(k, _)| k.starts_with(&prefix))
                .map(|(_, f)| (f, false)),
        );
    }
    if found.is_empty() {
        s.source = UsageSource::NotFound;
        return s;
    }
    s.source = if found.iter().any(|(f, _)| f.gone) {
        UsageSource::Record
    } else {
        UsageSource::Transcript
    };
    let lo = x.from_ms;
    let hi = x.until_ms.unwrap_or(u64::MAX);
    let inside = |at: u64| at >= lo && at < hi;
    let mut first = u64::MAX;
    let mut last = 0;
    let mut turns: Vec<Span> = Vec::new();
    let mut asks: Vec<Span> = Vec::new();
    for (f, main) in &found {
        for m in f.messages.iter().filter(|m| inside(m.at_ms)) {
            let t = s.models.entry(m.model.clone()).or_default();
            t.add(&Tokens {
                input: m.input,
                output: m.output,
                cache_write: m.cache_write,
                cache_write_1h: m.cache_write_1h,
                cache_read: m.cache_read,
                messages: 1,
            });
            first = first.min(m.at_ms);
            last = last.max(m.at_ms);
        }
        // A subagent runs inside its parent's turn: only the main file's
        // turns and asks are the session's time.
        if !main {
            continue;
        }
        turns.extend(f.turns.iter().filter_map(|t| clip(*t, lo, hi)));
        asks.extend(f.asks.iter().filter_map(|t| clip(*t, lo, hi)));
    }
    for t in &turns {
        first = first.min(t.start_ms);
        last = last.max(t.end_ms);
    }
    if first <= last {
        s.span_ms = last - first;
    }
    let turns = merged(turns);
    let asks = merged(asks);
    s.asking_ms = measure(&asks);
    s.active_ms = measure(&turns).saturating_sub(overlap(&turns, &asks));
    s.idle_ms = s.span_ms.saturating_sub(s.active_ms + s.asking_ms);
    s
}

/// `t` within `[lo, hi)`, or `None` when nothing of it is.
fn clip(t: Span, lo: u64, hi: u64) -> Option<Span> {
    let start_ms = t.start_ms.max(lo);
    let end_ms = t.end_ms.min(hi);
    (start_ms <= end_ms && t.end_ms >= lo && t.start_ms < hi).then_some(Span { start_ms, end_ms })
}

/// Spans sorted and joined where they touch.
fn merged(mut spans: Vec<Span>) -> Vec<Span> {
    spans.sort_by_key(|s| s.start_ms);
    let mut out: Vec<Span> = Vec::new();
    for s in spans {
        match out.last_mut() {
            Some(last) if s.start_ms <= last.end_ms => last.end_ms = last.end_ms.max(s.end_ms),
            _ => out.push(s),
        }
    }
    out
}

fn measure(spans: &[Span]) -> u64 {
    spans.iter().map(|s| s.end_ms - s.start_ms).sum()
}

/// The time two sets of merged spans share.
fn overlap(a: &[Span], b: &[Span]) -> u64 {
    a.iter()
        .flat_map(|x| {
            b.iter().map(move |y| {
                x.end_ms
                    .min(y.end_ms)
                    .saturating_sub(x.start_ms.max(y.start_ms))
            })
        })
        .sum()
}

/// A ticket's attempts' usage rolled up for a reader.
#[must_use]
pub fn view(ticket: &str, attempts: Vec<AttemptUsage>) -> UsageView {
    let mut v = UsageView {
        ticket: ticket.to_owned(),
        ..UsageView::default()
    };
    for a in &attempts {
        add_models(&mut v.models, &a.models);
        let i = v
            .stages
            .iter()
            .position(|s| s.stage == a.stage)
            .unwrap_or_else(|| {
                v.stages.push(StageUsage {
                    stage: a.stage.clone(),
                    ..StageUsage::default()
                });
                v.stages.len() - 1
            });
        add_models(&mut v.stages[i].models, &a.models);
        for s in &a.sessions {
            match s.source {
                UsageSource::NotCounted => v.not_counted += 1,
                UsageSource::NotFound => v.not_found += 1,
                UsageSource::KindUnknown => v.kind_unknown += 1,
                UsageSource::Transcript | UsageSource::Record => {}
            }
        }
    }
    v.attempts = attempts;
    v
}

/// The file system as `rescan` sees it, so a test can stand in for it.
pub trait Disk {
    /// Size in bytes and modification time in ms since the epoch, or
    /// `None` when the file is gone.
    fn stat(&self, path: &Path) -> Option<(u64, u64)>;
    /// The file's text, or `None` when it cannot be read.
    fn read(&self, path: &Path) -> Option<String>;
    /// The files in `dir`; none when it is missing.
    fn list(&self, dir: &Path) -> Vec<PathBuf>;
}

/// The real file system.
pub struct RealDisk;

impl Disk for RealDisk {
    fn stat(&self, path: &Path) -> Option<(u64, u64)> {
        let meta = std::fs::metadata(path).ok()?;
        let mtime = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?;
        Some((
            meta.len(),
            u64::try_from(mtime.as_millis()).unwrap_or(u64::MAX),
        ))
    }

    fn read(&self, path: &Path) -> Option<String> {
        // A record cut off mid-character must not lose the whole file.
        let bytes = std::fs::read(path).ok()?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn list(&self, dir: &Path) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        entries.filter_map(|e| Some(e.ok()?.path())).collect()
    }
}

/// The entries of `cache` that are out of date for `refs`, read again:
/// a file whose size or time changed is scanned afresh, and one gone is
/// marked `gone` with its rows kept. Only changed entries are returned,
/// for `merge`. A `codex` ref is never read: its file is a rollout,
/// not a Claude transcript.
#[must_use]
pub fn rescan(
    cache: &Cache,
    refs: &BTreeMap<String, Vec<TranscriptRef>>,
    disk: &dyn Disk,
    now_ms: u64,
) -> Cache {
    let mut paths: Vec<PathBuf> = Vec::new();
    for main in refs.values().flat_map(|r| claude_paths(Some(r))) {
        paths.push(main.to_path_buf());
        let dir = subagents_dir(main);
        paths.extend(disk.list(&dir).into_iter().filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("agent-"))
                && p.extension().is_some_and(|e| e == "jsonl")
        }));
        let prefix = format!("{}/", dir.to_string_lossy());
        paths.extend(
            cache
                .files
                .range(prefix.clone()..)
                .take_while(|(k, _)| k.starts_with(&prefix))
                .map(|(k, _)| PathBuf::from(k)),
        );
    }
    paths.sort();
    paths.dedup();
    let mut changed = Cache::default();
    for path in paths {
        let key = path.to_string_lossy().into_owned();
        let old = cache.files.get(&key);
        match disk.stat(&path) {
            Some((size, mtime_ms)) => {
                if old.is_some_and(|f| !f.gone && f.size == size && f.mtime_ms == mtime_ms) {
                    continue;
                }
                let Some(text) = disk.read(&path) else {
                    continue;
                };
                changed.files.insert(
                    key,
                    FileUsage {
                        size,
                        mtime_ms,
                        read_ms: now_ms,
                        ..scan(&text)
                    },
                );
            }
            None => {
                if let Some(f) = old.filter(|f| !f.gone) {
                    changed.files.insert(
                        key,
                        FileUsage {
                            gone: true,
                            read_ms: now_ms,
                            ..f.clone()
                        },
                    );
                }
            }
        }
    }
    changed
}

/// `scanned` merged into `into`, keeping per path the entry read later,
/// so two writers at once never undo each other's newer entries.
pub fn merge(into: &mut Cache, scanned: Cache) {
    for (path, f) in scanned.files {
        match into.files.get(&path) {
            Some(old) if old.read_ms > f.read_ms => {}
            _ => {
                into.files.insert(path, f);
            }
        }
    }
}

/// The ticket's cache brought up to date with its transcripts and
/// returned. The files are read with no lock held, since a cold read
/// can be large and every pass waits on the writer lock; the lock is
/// taken only to merge and write.
pub fn bring_up_to_date(data: &DataDir, t: &Ticket, now_ms: u64) -> Result<Cache> {
    let path = data.usage_file(&t.id);
    let cache = store::read_usage(&path)?;
    let scanned = rescan(&cache, &t.transcripts, &RealDisk, now_ms);
    if scanned.files.is_empty() {
        return Ok(cache);
    }
    data.with_lock(|| merge_and_write(&path, scanned))
}

/// `scanned` merged into the cache at `path` as it stands now, and the
/// result written and returned. The caller holds the writer lock: the
/// read and the write must not let another writer in between.
pub fn merge_and_write(path: &Path, scanned: Cache) -> Result<Cache> {
    let mut cache = store::read_usage(path)?;
    merge(&mut cache, scanned);
    store::write_usage(path, &cache)?;
    Ok(cache)
}

/// A ticket's usage per attempt from its cache as it stands.
#[must_use]
pub fn of(t: &Ticket, p: Option<&Pipeline>, cache: &Cache, now_ms: u64) -> Vec<AttemptUsage> {
    attribute(&sessions_of(t, p), &t.transcripts, cache, now_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::new_attempt;
    use crate::ticket::{
        Attempt, AttemptState, MessageFix, ReviewRound, ReviewerRun, Rewrite, RoundState,
        RunSession,
    };

    const HAIKU: &str = "claude-haiku-5-5";
    const OPUS: &str = "claude-opus-5-5";

    fn ts(ms: u64) -> String {
        chrono::DateTime::from_timestamp_millis(i64::try_from(ms).unwrap())
            .unwrap()
            .to_rfc3339()
    }

    fn said(at: u64, text: &str) -> String {
        format!(
            r#"{{"type":"user","timestamp":"{}","message":{{"role":"user","content":"{text}"}}}}"#,
            ts(at)
        )
    }

    fn reply(at: u64, id: &str, model: &str, output: u64) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{}","message":{{"id":"{id}","model":"{model}","usage":{{"input_tokens":1,"output_tokens":{output},"cache_creation_input_tokens":30,"cache_creation":{{"ephemeral_1h_input_tokens":20}},"cache_read_input_tokens":100,"speed":"standard"}}}}}}"#,
            ts(at)
        )
    }

    fn ask(at: u64, id: &str, tool: &str) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{}","message":{{"id":"a{tool}","model":"{OPUS}","content":[{{"type":"tool_use","id":"{tool}","name":"{id}"}}],"usage":{{"output_tokens":1}}}}}}"#,
            ts(at)
        )
    }

    fn answered(at: u64, tool: &str) -> String {
        format!(
            r#"{{"type":"user","timestamp":"{}","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"{tool}","content":"yes"}}]}}}}"#,
            ts(at)
        )
    }

    fn lines(ls: &[String]) -> String {
        ls.join("\n")
    }

    /// The file system in memory: path to text and mtime.
    #[derive(Default)]
    struct Fake(BTreeMap<PathBuf, (String, u64)>);

    impl Disk for Fake {
        fn stat(&self, path: &Path) -> Option<(u64, u64)> {
            self.0.get(path).map(|(t, m)| (t.len() as u64, *m))
        }
        fn read(&self, path: &Path) -> Option<String> {
            self.0.get(path).map(|(t, _)| t.clone())
        }
        fn list(&self, dir: &Path) -> Vec<PathBuf> {
            self.0
                .keys()
                .filter(|p| p.parent() == Some(dir))
                .cloned()
                .collect()
        }
    }

    fn reference(agent: &str, path: &str) -> TranscriptRef {
        TranscriptRef {
            agent: agent.into(),
            provider_id: path.into(),
            path: Some(path.into()),
            seen_ms: 0,
        }
    }

    fn attempt(stage: &str, n: u32, kind: AttemptKind, start: u64) -> Attempt {
        new_attempt(
            stage,
            n,
            "repo",
            kind,
            AttemptState::Running,
            BTreeMap::new(),
            start,
        )
    }

    fn agent(stage: &str, n: u32, session: &str, start: u64) -> Attempt {
        Attempt {
            session: Some(session.into()),
            agent: Some("claude".into()),
            ..attempt(stage, n, AttemptKind::Agent, start)
        }
    }

    /// The cache as `rescan` reads `files` for `t`.
    fn cache_of(t: &Ticket, files: &[(&str, String)]) -> Cache {
        let disk = Fake(
            files
                .iter()
                .map(|(p, text)| (PathBuf::from(p), (text.clone(), 1)))
                .collect(),
        );
        rescan(&Cache::default(), &t.transcripts, &disk, 1)
    }

    fn sessions(t: &Ticket, p: Option<&Pipeline>, files: &[(&str, String)]) -> Vec<SessionUsage> {
        of(t, p, &cache_of(t, files), 1_000_000)
            .into_iter()
            .flat_map(|a| a.sessions)
            .collect()
    }

    fn pipeline() -> Pipeline {
        Pipeline::parse(
            r#"
version = 1
[project]
name = "P"
repo = "git@example.com:o/p.git"
space = "S"
[source]
kind = "github"
repo = "o/p"
label = "dispatch"
[[lanes]]
name = "repo"
path = "."
[operators.planner]
kind = "claude"
[operators.implementer]
kind = "codex"
[operators.reviewer]
kind = "codex"
[operators.reviewer.review]
reviewer = "codex"
review_first = "{plan} {feedback} {no_feedback}"
review_round = "{response} {plan} {feedback} {no_feedback}"
respond = "{feedback} {plan} {response}"
respond_to_user = "{text} {plan} {response}"
handoff = "{plan}"
no_feedback = "none"
[[stages]]
name = "plan"
operator = "planner"
context = "each"
writes = ["plan"]
prompt = "{plan}"
[[stages]]
name = "review"
review = "reviewer"
context = "each"
subject = "plan"
gate = { kind = "external", check = "review-finalized" }
[[stages]]
name = "implement"
operator = "implementer"
context = "each"
prompt = "go"
"#,
        )
        .unwrap()
    }

    #[test]
    fn one_message_written_in_three_records_counts_once_at_its_last_usage() {
        let f = scan(&lines(&[
            said(0, "go"),
            reply(1_000, "m1", OPUS, 5),
            reply(1_001, "m1", OPUS, 50),
            reply(1_002, "m1", OPUS, 500),
        ]));
        assert_eq!(f.messages.len(), 1);
        assert_eq!(f.messages[0].output, 500);
        assert_eq!(f.messages[0].at_ms, 1_002);
    }

    #[test]
    fn two_models_keep_two_rows_and_the_synthetic_one_none() {
        let f = scan(&lines(&[
            reply(1, "m1", OPUS, 5),
            reply(2, "m2", HAIKU, 6),
            reply(3, "m3", SYNTHETIC, 0),
        ]));
        let models: Vec<&str> = f.messages.iter().map(|m| m.model.as_str()).collect();
        assert_eq!(models, [OPUS, HAIKU]);
    }

    #[test]
    fn the_hour_long_cache_write_and_the_speed_are_read() {
        let f = scan(&reply(1, "m1", OPUS, 5));
        let m = &f.messages[0];
        assert_eq!((m.cache_write, m.cache_write_1h), (30, 20));
        assert_eq!((m.input, m.cache_read), (1, 100));
        assert_eq!(m.speed.as_deref(), Some("standard"));
    }

    #[test]
    fn a_sidechain_record_counts() {
        let side = reply(1, "m1", OPUS, 5).replacen(
            r#""type":"assistant""#,
            r#""type":"assistant","isSidechain":true"#,
            1,
        );
        assert_eq!(scan(&side).messages.len(), 1);
    }

    #[test]
    fn a_tool_result_is_not_a_turn_and_an_ask_waits_to_its_answer() {
        let f = scan(&lines(&[
            said(0, "go"),
            reply(1_000, "m1", OPUS, 5),
            ask(2_000, ASK_TOOL, "t1"),
            answered(7_000, "t1"),
            ask(7_500, "Bash", "t2"),
            answered(8_000, "t2"),
            reply(9_000, "m2", OPUS, 5),
            said(20_000, "again"),
            reply(21_000, "m3", OPUS, 5),
        ]));
        assert_eq!(
            f.turns,
            [
                Span {
                    start_ms: 0,
                    end_ms: 9_000
                },
                Span {
                    start_ms: 20_000,
                    end_ms: 21_000
                }
            ]
        );
        assert_eq!(
            f.asks,
            [Span {
                start_ms: 2_000,
                end_ms: 7_000
            }]
        );
        // The session's time: a span of 21 s; the turns' 10 s less the
        // 5 s asking is active; the 11 s between the turns is idle.
        let mut t = Ticket {
            transcripts: BTreeMap::from([("s1".into(), vec![reference("claude", "/c/a.jsonl")])]),
            ..crate::ticket::blank()
        };
        t.attempts.push(agent("plan", 1, "s1", 0));
        let text = lines(&[
            said(0, "go"),
            reply(1_000, "m1", OPUS, 5),
            ask(2_000, ASK_TOOL, "t1"),
            answered(7_000, "t1"),
            reply(9_000, "m2", OPUS, 5),
            said(20_000, "again"),
            reply(21_000, "m3", OPUS, 5),
        ]);
        let s = &sessions(&t, None, &[("/c/a.jsonl", text)])[0];
        assert_eq!(s.span_ms, 21_000);
        assert_eq!(s.asking_ms, 5_000);
        assert_eq!(s.active_ms, 5_000);
        assert_eq!(s.idle_ms, 11_000);
    }

    /// A planner clone copies its source's history, timestamps and all:
    /// the plan session and the clone together count the source's
    /// tokens once.
    #[test]
    fn a_clones_copied_history_is_counted_once() {
        let source = lines(&[said(100, "plan"), reply(500, "m1", OPUS, 1_000)]);
        let clone = lines(&[
            said(100, "plan"),
            reply(500, "m1", OPUS, 1_000),
            said(2_000, "feedback"),
            reply(2_500, "m2", OPUS, 7),
        ]);
        let mut review = attempt("review", 1, AttemptKind::Workflow, 1_000);
        review.session = Some("clone".into());
        review.run_sessions = BTreeMap::from([
            (
                "planner".into(),
                RunSession {
                    session: "clone".into(),
                    agent: "claude".into(),
                },
            ),
            (
                "reviewer".into(),
                RunSession {
                    session: "rev".into(),
                    agent: "codex".into(),
                },
            ),
        ]);
        let t = Ticket {
            transcripts: BTreeMap::from([
                ("plan".into(), vec![reference("claude", "/c/plan.jsonl")]),
                ("clone".into(), vec![reference("claude", "/c/clone.jsonl")]),
            ]),
            attempts: vec![agent("plan", 1, "plan", 0), review],
            ..crate::ticket::blank()
        };
        let v = view(
            "t",
            of(
                &t,
                None,
                &cache_of(&t, &[("/c/plan.jsonl", source), ("/c/clone.jsonl", clone)]),
                9_000,
            ),
        );
        assert_eq!(v.models[OPUS].output, 1_007);
        assert_eq!(v.models[OPUS].messages, 2);
        // The workflow attempt's face is not listed again as `agent`.
        let roles: Vec<&str> = v.attempts[1]
            .sessions
            .iter()
            .map(|s| s.role.as_str())
            .collect();
        assert_eq!(roles, ["planner", "plan reviewer"]);
        assert_eq!(v.not_counted, 1);
    }

    /// Before the planner exists the face is the reviewer: it is listed
    /// once, as the plan reviewer.
    #[test]
    fn a_workflow_attempts_face_is_its_reviewer_counted_once() {
        let mut review = attempt("review", 1, AttemptKind::Workflow, 0);
        review.session = Some("rev".into());
        review.run_sessions = BTreeMap::from([(
            "reviewer".into(),
            RunSession {
                session: "rev".into(),
                agent: "claude".into(),
            },
        )]);
        let t = Ticket {
            transcripts: BTreeMap::from([("rev".into(), vec![reference("claude", "/c/r.jsonl")])]),
            attempts: vec![review],
            ..crate::ticket::blank()
        };
        let s = sessions(&t, None, &[("/c/r.jsonl", reply(5, "m1", OPUS, 9))]);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].role, "plan reviewer");
        assert_eq!(s[0].models[OPUS].output, 9);
    }

    #[test]
    fn one_record_named_by_two_attempts_splits_at_the_second_start() {
        let t = Ticket {
            transcripts: BTreeMap::from([("s".into(), vec![reference("claude", "/c/s.jsonl")])]),
            attempts: vec![agent("plan", 1, "s", 0), agent("plan", 2, "s", 1_000)],
            ..crate::ticket::blank()
        };
        let text = lines(&[reply(500, "m1", OPUS, 3), reply(1_500, "m2", OPUS, 4)]);
        let s = sessions(&t, None, &[("/c/s.jsonl", text)]);
        assert_eq!(s[0].models[OPUS].output, 3);
        assert_eq!(s[1].models[OPUS].output, 4);
    }

    #[test]
    fn a_kind_is_recorded_else_seen_else_the_pipelines_else_unknown() {
        let p = pipeline();
        // Recorded Codex with no ref: not counted, not "not found".
        let mut a = agent("plan", 1, "s", 0);
        a.agent = Some("codex".into());
        let t = Ticket {
            attempts: vec![a.clone()],
            ..crate::ticket::blank()
        };
        let s = &sessions(&t, Some(&p), &[])[0];
        assert_eq!(s.source, UsageSource::NotCounted);
        assert!(!s.kind_from_pipeline);
        // Nothing recorded, a Codex ref: the ref decides.
        a.agent = None;
        let t = Ticket {
            attempts: vec![a.clone()],
            transcripts: BTreeMap::from([("s".into(), vec![reference("codex", "/x/r.jsonl")])]),
            ..crate::ticket::blank()
        };
        assert_eq!(sessions(&t, None, &[])[0].source, UsageSource::NotCounted);
        // Neither: the pipeline copy's operator, marked.
        let t = Ticket {
            attempts: vec![attempt_named(&a, "implement")],
            ..crate::ticket::blank()
        };
        let s = &sessions(&t, Some(&p), &[])[0];
        assert_eq!((s.kind.as_str(), s.kind_from_pipeline), ("codex", true));
        assert_eq!(s.source, UsageSource::NotCounted);
        let t = Ticket {
            attempts: vec![attempt_named(&a, "plan")],
            ..crate::ticket::blank()
        };
        let s = &sessions(&t, Some(&p), &[])[0];
        assert_eq!((s.kind.as_str(), s.kind_from_pipeline), ("claude", true));
        assert_eq!(s.source, UsageSource::NotFound);
        // Nothing at all: kind unknown, apart from not found.
        let s = &sessions(&t, None, &[])[0];
        assert_eq!(s.source, UsageSource::KindUnknown);
    }

    fn attempt_named(a: &Attempt, stage: &str) -> Attempt {
        Attempt {
            stage: stage.into(),
            ..a.clone()
        }
    }

    #[test]
    fn the_merge_keeps_each_writers_newer_entry() {
        let entry = |read_ms| FileUsage {
            read_ms,
            ..FileUsage::default()
        };
        let mut disk = Cache::default();
        disk.files.insert("/a".into(), entry(5));
        disk.files.insert("/b".into(), entry(1));
        let mut scanned = Cache::default();
        scanned.files.insert("/a".into(), entry(3));
        scanned.files.insert("/b".into(), entry(4));
        scanned.files.insert("/c".into(), entry(2));
        merge(&mut disk, scanned);
        let kept: Vec<u64> = disk.files.values().map(|f| f.read_ms).collect();
        assert_eq!(kept, [5, 4, 2]);
    }

    #[test]
    fn a_subagents_rows_go_to_the_session_that_spawned_it() {
        let t = Ticket {
            transcripts: BTreeMap::from([("s".into(), vec![reference("claude", "/c/s.jsonl")])]),
            attempts: vec![agent("plan", 1, "s", 0)],
            ..crate::ticket::blank()
        };
        let s = sessions(
            &t,
            None,
            &[
                ("/c/s.jsonl", reply(5, "m1", OPUS, 3)),
                ("/c/s/subagents/agent-1.jsonl", reply(6, "m1", HAIKU, 4)),
                ("/c/s/subagents/notes.txt", reply(6, "m9", HAIKU, 99)),
            ],
        );
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].models[OPUS].output, 3);
        assert_eq!(s[0].models[HAIKU].output, 4);
    }

    fn run(name: &str, kind: &str, session: Option<&str>) -> ReviewerRun {
        ReviewerRun {
            name: name.into(),
            kind: kind.into(),
            dir: "/d".into(),
            feedback: "/d/f.md".into(),
            session: session.map(str::to_owned),
            launched: true,
            stop_at_ms: None,
            polls_since_stop: 0,
            settle: None,
            result: None,
            group: None,
        }
    }

    #[test]
    fn codex_and_command_reviewers_are_not_counted_with_their_rounds_span() {
        let mut code = attempt("review-code", 1, AttemptKind::Review, 0);
        code.rounds = vec![ReviewRound {
            n: 1,
            base: "b".into(),
            head: "h".into(),
            reviewers: vec![run("cx", "codex", Some("c1")), run("lint", "command", None)],
            state: RoundState::Converged,
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
            started_ms: 1_000,
            ended_ms: Some(4_000),
        }];
        code.rewrite = Some(Rewrite {
            mode: crate::history::Commits::Fold,
            before: "h".into(),
            after: None,
            from: 1,
            to: 1,
            skipped: None,
            stale: Vec::new(),
            message: Some(MessageFix {
                session: Some("rw".into()),
                ..MessageFix::default()
            }),
            at_ms: 0,
        });
        let t = Ticket {
            attempts: vec![code],
            ..crate::ticket::blank()
        };
        let s = sessions(&t, None, &[]);
        let got: Vec<(&str, UsageSource, u64)> = s
            .iter()
            .map(|x| (x.role.as_str(), x.source, x.span_ms))
            .collect();
        assert_eq!(
            got,
            [
                ("round 1 reviewer cx", UsageSource::NotCounted, 3_000),
                ("round 1 reviewer lint", UsageSource::NotCounted, 3_000),
                ("rewriter", UsageSource::KindUnknown, 0),
            ]
        );
    }

    #[test]
    fn a_gone_file_reports_its_record_and_a_file_never_read_is_not_found() {
        let t = Ticket {
            transcripts: BTreeMap::from([
                ("s".into(), vec![reference("claude", "/c/s.jsonl")]),
                ("u".into(), vec![reference("claude", "/c/u.jsonl")]),
            ]),
            attempts: vec![agent("plan", 1, "s", 0), agent("implement", 1, "u", 0)],
            ..crate::ticket::blank()
        };
        let mut cache = cache_of(&t, &[("/c/s.jsonl", reply(5, "m1", OPUS, 3))]);
        let gone = rescan(&cache, &t.transcripts, &Fake::default(), 9);
        assert!(gone.files["/c/s.jsonl"].gone);
        merge(&mut cache, gone);
        let s: Vec<SessionUsage> = of(&t, None, &cache, 10)
            .into_iter()
            .flat_map(|a| a.sessions)
            .collect();
        assert_eq!(s[0].source, UsageSource::Record);
        assert_eq!(s[0].models[OPUS].output, 3);
        assert_eq!(s[1].source, UsageSource::NotFound);
        assert!(s[1].models.is_empty());
        // Unchanged since, the gone entry is not written again.
        assert!(
            rescan(&cache, &t.transcripts, &Fake::default(), 11)
                .files
                .is_empty()
        );
    }

    #[test]
    fn a_codex_ref_with_a_path_is_never_read() {
        let refs = BTreeMap::from([("c".into(), vec![reference("codex", "/x/rollout.jsonl")])]);
        let disk = Fake(BTreeMap::from([(
            PathBuf::from("/x/rollout.jsonl"),
            (reply(5, "m1", OPUS, 3), 1),
        )]));
        assert!(rescan(&Cache::default(), &refs, &disk, 1).files.is_empty());
    }

    #[test]
    fn an_unchanged_file_is_not_read_again_and_a_grown_one_is() {
        let refs = BTreeMap::from([("s".into(), vec![reference("claude", "/c/s.jsonl")])]);
        let mut disk = Fake(BTreeMap::from([(
            PathBuf::from("/c/s.jsonl"),
            (reply(5, "m1", OPUS, 3), 1),
        )]));
        let mut cache = rescan(&Cache::default(), &refs, &disk, 1);
        assert!(rescan(&cache, &refs, &disk, 2).files.is_empty());
        disk.0.insert(
            "/c/s.jsonl".into(),
            (
                lines(&[reply(5, "m1", OPUS, 3), reply(6, "m2", OPUS, 4)]),
                2,
            ),
        );
        let grown = rescan(&cache, &refs, &disk, 3);
        merge(&mut cache, grown);
        assert_eq!(cache.files["/c/s.jsonl"].messages.len(), 2);
        assert_eq!(cache.files["/c/s.jsonl"].read_ms, 3);
    }
}
