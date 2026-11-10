//! A project's brief and a ticket's report, built from the runner's
//! records: what `dispatch brief` and `dispatch report <ticket> --json`
//! print, and what the port serves for them, so the command line and a
//! remote reader never drift.

use anyhow::Result;
use dispatch_control::{
    AskView, BriefDecision, BriefSubscription, BriefTicket, BriefView, BriefWorktree,
};

use crate::events::{self, Event, Kind};
use crate::pipeline::Pipeline;
use crate::report::{self, TicketReport};
use crate::scheduler::Runner;
use crate::subscribe;
use crate::supervisor::Standing;
use crate::ticket::{Ask, AskState, Ticket, TicketState};

/// How many of the project's last events a brief carries.
pub const BRIEF_EVENTS: usize = 20;

/// How many undelivered lines a brief shows per subscription.
pub const BRIEF_UNDELIVERED: usize = 10;

/// The brief of `project`. `decide` is the command a printed `decide`
/// line starts with: `dispatch decide` for the owner, the full path for
/// a supervisor, whose only permission matches it.
pub fn build(runner: &Runner, project: &str, decide: &str, now: u64) -> Result<BriefView> {
    let supervisor = match runner.supervisor_standing(project) {
        Ok(s) => supervisor_line(project, &s, now),
        Err(e) => format!("supervisor: {e:#}"),
    };
    let build = crate::health::build_line(
        crate::health::read_runner_file(&runner.data)
            .as_ref()
            .map(|f| &f.build),
        &crate::own_build(),
    );
    let tickets: Vec<Ticket> = runner
        .tickets()?
        .into_iter()
        .filter(|t| t.project == project && !matches!(t.state, TicketState::Closed { .. }))
        .collect();
    let mut waiting = Vec::new();
    for t in &tickets {
        for d in t.waiting_on_you() {
            waiting.push(BriefDecision {
                ticket: t.id.clone(),
                decision: d.id.clone(),
                stage: d.stage.clone(),
                name: d.name.clone(),
                question: d.question.clone(),
                options: d.options.clone(),
                decide: format!("{decide} {} {}", t.id, d.id),
            });
        }
    }
    let log = events::log_path(&runner.data);
    let all = events::read_since(&log, 0)?;
    let gone = events::withdrawn(&all);
    let mine: Vec<&Event> = all
        .iter()
        .filter(|e| e.project == project && e.kind != Kind::Void && !gone.contains(&e.seq))
        .collect();
    let events = mine[mine.len().saturating_sub(BRIEF_EVENTS)..]
        .iter()
        .map(|e| events::line(e, false))
        .collect();
    let follow_from = events::last_seq(&log)?;
    let ps = runner.load_project(project)?;
    let mut subscriptions = Vec::new();
    for sub in &ps.supervisor.subscriptions {
        let lines = subscribe::undelivered(&runner.data, sub)?;
        subscriptions.push(BriefSubscription {
            ticket: sub.ticket.clone(),
            since: sub.since,
            undelivered: lines.len(),
            lines: lines[lines.len().saturating_sub(BRIEF_UNDELIVERED)..]
                .iter()
                .map(|e| events::line(e, false))
                .collect(),
        });
    }
    let worktrees = tickets
        .iter()
        .flat_map(|t| {
            t.lanes
                .iter()
                .filter(|l| !l.removed)
                .map(|l| BriefWorktree {
                    ticket: t.id.clone(),
                    lane: l.name.clone(),
                    path: l.worktree.clone(),
                })
        })
        .collect();
    let handoff_path = runner.data.supervisor_dir(project).join("handoff.md");
    let handoff = std::fs::read_to_string(&handoff_path).ok();
    let asks = ps
        .supervisor
        .asks
        .iter()
        .filter(|a| a.state == AskState::Queued)
        .map(|a| ask_view(project, a))
        .collect();
    Ok(BriefView {
        project: project.to_owned(),
        supervisor,
        build,
        tickets: tickets
            .iter()
            .map(|t| BriefTicket {
                id: t.id.clone(),
                line: ticket_line(runner, t, now),
            })
            .collect(),
        waiting,
        events,
        follow_from,
        subscriptions,
        worktrees,
        handoff_path,
        handoff,
        asks,
    })
}

/// The brief as `dispatch brief` prints it, without a final newline.
#[must_use]
pub fn render(b: &BriefView) -> String {
    let mut out: Vec<String> = vec![b.supervisor.clone(), b.build.clone(), "\ntickets:".into()];
    if b.tickets.is_empty() {
        out.push("  none".into());
    }
    for t in &b.tickets {
        out.push(format!("  {}", t.line));
    }
    out.push("\nwaiting on you:".into());
    if b.waiting.is_empty() {
        out.push("  nothing".into());
    }
    for d in &b.waiting {
        out.push(format!(
            "  {} {} [{}] {}: {}\n    options: {}\n    {} <answer> [--note <text> | --file <path>]",
            d.ticket,
            d.decision,
            d.stage,
            d.name,
            d.question,
            d.options.join(" | "),
            d.decide
        ));
    }
    out.push("\nlast events:".into());
    for e in &b.events {
        out.push(format!("  {e}"));
    }
    out.push(format!("  follow from seq {}", b.follow_from));
    if !b.subscriptions.is_empty() {
        out.push(
            "\nsubscriptions (the runner types these into your pane when you are idle):".into(),
        );
    }
    for sub in &b.subscriptions {
        out.push(format!(
            "  {} from seq {}: {} undelivered",
            sub.ticket, sub.since, sub.undelivered
        ));
        for line in &sub.lines {
            out.push(format!("    {line}"));
        }
    }
    if !b.asks.is_empty() {
        out.push("\nowner messages not typed yet:".into());
    }
    for a in &b.asks {
        out.push(format!("  {} from {}: {}", a.id, a.by, a.text));
    }
    out.push("\nopen worktrees:".into());
    if b.worktrees.is_empty() {
        out.push("  none".into());
    }
    for w in &b.worktrees {
        out.push(format!("  {} {}: {}", w.ticket, w.lane, w.path.display()));
    }
    out.push(format!("\nhand-off ({}):", b.handoff_path.display()));
    match &b.handoff {
        Some(text) => out.push(text.trim_end().to_owned()),
        None => out.push("(no hand-off yet)".into()),
    }
    out.join("\n")
}

/// An owner-channel message as the port shows it.
#[must_use]
pub fn ask_view(project: &str, a: &Ask) -> AskView {
    AskView {
        id: a.id.clone(),
        project: project.to_owned(),
        text: a.text.clone(),
        by: a.by.clone(),
        state: match a.state {
            AskState::Queued => "queued",
            AskState::Delivered => "delivered",
        }
        .to_owned(),
        session: a.session.clone(),
        created_ms: a.created_ms,
        delivered_ms: a.delivered_ms,
    }
}

/// The supervisor in one line: its session, age and seed.
#[must_use]
pub fn supervisor_line(project: &str, s: &Standing, now: u64) -> String {
    if s.table.is_none() {
        return format!("supervisor: none (no [supervisor] table in pipelines/{project}.toml)");
    }
    let Some(c) = &s.ps.supervisor.current else {
        return format!("supervisor: none; `dispatch supervisor {project} --fresh` starts one");
    };
    let seed = if s.stale {
        "stale: `[supervisor]` changed, `--fresh` to reseed"
    } else {
        "current"
    };
    format!(
        "supervisor: session {}, {} old, seed {} ({seed})",
        c.session,
        span(now.saturating_sub(c.created_ms)),
        c.seed_hash
    )
}

/// An open ticket on one line: its source, stage, state and cost so far.
/// The cost reads the project's pipeline file once per ticket
/// (`running_cost`), so a brief of many tickets is that many file reads.
#[must_use]
pub fn ticket_line(runner: &Runner, t: &Ticket, now: u64) -> String {
    let pipeline = runner.pipeline_of(t).ok();
    let stage = pipeline
        .as_ref()
        .and_then(|p| p.stages.get(t.stage).map(|s| s.name.clone()))
        .unwrap_or_else(|| "done".to_owned());
    format!(
        "{} {} {} · stage {stage} · {}{}",
        t.id,
        t.source.label(),
        t.source.title,
        t.state.label(),
        running_cost(runner, t, pipeline.as_ref(), now)
    )
}

/// ` · ~$1.23 so far` from the ticket's counts cache as it stands: no
/// transcript is read, so a brief stays cheap and is as of the last
/// report or Usage tab read. Nothing when nothing is counted.
fn running_cost(runner: &Runner, t: &Ticket, pipeline: Option<&Pipeline>, now: u64) -> String {
    let Ok(cache) = crate::store::read_usage(&runner.data.usage_file(&t.id)) else {
        return String::new();
    };
    let mut tokens = std::collections::BTreeMap::new();
    for a in crate::usage::of(t, pipeline, &cache, now) {
        dispatch_control::add_models(&mut tokens, &a.models);
    }
    if tokens.is_empty() {
        return String::new();
    }
    let cost = report::price(&tokens, &runner.rates_for(t));
    if cost.priced == 0 {
        return format!(" · no rate for {}", cost.unpriced.join(", "));
    }
    format!(" · ~{} so far", cost_cell(&cost))
}

/// A ticket's report, with git asked for the size of its PR's range in
/// Dispatch's clone of the lane's repository.
#[must_use]
pub fn report_of(runner: &Runner, t: &Ticket, now: u64) -> TicketReport {
    let names = events::stage_names(t);
    let pipeline = runner.pipeline_of(t).ok();
    let range = report::pr_range(t).and_then(|(lane, base, head)| {
        let p = pipeline.as_ref()?;
        let dir = runner.lane_clone(p, p.lane(&lane)?);
        runner.git.range_size(&dir, &base, &head).ok()
    });
    // The counts are read as far as they can be: a cache that cannot be
    // brought up to date still says what it held.
    let cache = crate::usage::bring_up_to_date(&runner.data, t, now).unwrap_or_else(|e| {
        log::warn!("ticket {}: usage not read: {e:#}", t.id);
        crate::store::read_usage(&runner.data.usage_file(&t.id)).unwrap_or_default()
    });
    let usage = crate::usage::of(t, pipeline.as_ref(), &cache, now);
    let mut r = report::of(
        t,
        pipeline.as_ref(),
        &names,
        &|p| std::fs::read_to_string(p).ok(),
        &report::plan_round_numbers,
        range,
        usage,
        now,
    );
    r.cost = report::price(&r.tokens, &runner.rates_for(t));
    r
}

/// A cost in a table cell or a brief line: `$1.23`, with a `+` when a
/// model's tokens or cache had no rate, and `-` when nothing was priced,
/// so a ticket with no rate or no tokens is not read as costing nothing.
#[must_use]
pub fn cost_cell(c: &report::Cost) -> String {
    if c.priced == 0 {
        return "-".to_owned();
    }
    format!("{}{}", c.dollars(), if c.short() { "+" } else { "" })
}

/// A duration for a person: `2h05m`, `4m10s`, `12s`.
#[must_use]
pub fn span(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}h{:02}m", s / 3600, (s / 60) % 60)
    } else if s >= 60 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}
