//! Dispatch's own port: `<data dir>/dispatch.sock`, served while
//! `dispatch run` is up. A client (Switchboard's Dispatch page, a
//! remote shell) reads tickets as views and does what the command line
//! does: answer a decision, reorder a queue, take an issue. Nothing here
//! bypasses the scheduler: every command goes through the same `Runner`
//! methods the CLI calls, under the same writer lock, so the runner's
//! loop and a request never interleave inside a record.

use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use anyhow::{Context, Result, bail};
use dispatch_control::{
    AttemptView, Body, DecisionView, EventView, EventsView, LaneFile, LaneView, PathsView,
    PlanRoundView, ProjectView, Reply, Request, SOCKET_FILE, Status, TicketView,
};

use crate::epoch_ms;
use crate::events;
use crate::github::Issues;
use crate::pipeline::{DeployWait, Pipeline, Source};
use crate::scheduler::{Runner, lane_files, shown_lane_files};
use crate::store::DataDir;
use crate::ticket::{
    Attempt, AttemptKind, AttemptState, Decision, DecisionState, PullRequestSource, SourceSnapshot,
    Ticket, TicketState,
};

/// A Unix socket path may be at most 104 bytes on macOS (108 on Linux);
/// binding a longer one fails obscurely, so it is refused with a reason.
const MAX_SOCKET_PATH: usize = 100;

/// What a request needs: the runner (its data directory, its way to
/// Switchboard and git) and the issue source, shared with every
/// connection under one lock.
pub struct Handler {
    pub runner: Runner,
    pub issues: Box<dyn Issues>,
}

impl Handler {
    /// Answer one request. Never panics on a bad one; the reply says.
    pub fn handle(&mut self, request: &Request, now_ms: u64) -> Reply {
        match self.answer(&request.body, now_ms) {
            Ok(reply) => reply,
            Err(e) => Reply::failed(format!("{e:#}")),
        }
    }

    fn answer(&mut self, body: &Body, now_ms: u64) -> Result<Reply> {
        Ok(match body {
            Body::Status => Reply::Status(status(&self.runner)?),
            Body::Ticket { id } => {
                let t = self.runner.load_ticket(id)?;
                let p = self.runner.pipeline_of(&t).ok();
                let mut view = TicketView {
                    paths: ticket_paths(&t, p.as_ref()),
                    ..self.view(&t)
                };
                if let Some(p) = p {
                    for l in &mut view.lanes {
                        if let Some(lane) = p.lanes.iter().find(|x| x.name == l.name) {
                            l.clone = Some(self.runner.lane_clone(&p, lane));
                        }
                    }
                }
                Reply::Ticket(view)
            }
            Body::Events { ticket, since } => Reply::Events(ticket_events(
                &events::log_path(&self.runner.data),
                ticket,
                *since,
            )?),
            Body::Artifact { ticket, path } => {
                let dir = self.runner.data.ticket_dir(ticket);
                let (dir, file) = (
                    dir.canonicalize().unwrap_or(dir),
                    path.canonicalize().unwrap_or_else(|_| path.clone()),
                );
                if !file.starts_with(&dir) {
                    bail!("{} is not a file of ticket {ticket}", path.display());
                }
                let t = self.runner.load_ticket(ticket)?;
                if let Some(name) = t.secret_at(&file) {
                    bail!("{name} is secret; Dispatch never reads it");
                }
                let text = fs::read_to_string(&file)
                    .with_context(|| format!("read {}", file.display()))?;
                Reply::Artifact { text }
            }
            Body::Decide {
                ticket,
                decision,
                answer,
                note,
            } => {
                let d = self
                    .runner
                    .decide(ticket, decision, answer, note.as_deref(), now_ms)?;
                let t = self.runner.load_ticket(ticket)?;
                Reply::Decided(decision_view(&t, &d))
            }
            Body::Queue { project, order } => {
                let ps = if order.is_empty() {
                    self.runner.load_project(project)?
                } else {
                    let order: Vec<&str> = order.iter().map(String::as_str).collect();
                    self.runner.reorder_queue(project, &order)?
                };
                Reply::Queue { order: ps.queue }
            }
            Body::Worktrees { path, migrate } => {
                Reply::Worktrees(self.runner.set_worktrees(path.clone(), *migrate, now_ms)?)
            }
            Body::Resume { ticket } => {
                let t = self.runner.resume(ticket, now_ms)?.ticket;
                Reply::Ticket(self.view(&t))
            }
            Body::Close { ticket, reason } => {
                let t = self
                    .runner
                    .request_close(ticket, reason.as_deref(), now_ms)?;
                Reply::Ticket(self.view(&t))
            }
            Body::Take { project, issue } => {
                let t = take_issue(&mut self.runner, &*self.issues, project, issue, now_ms)?;
                Reply::Taken(self.view(&t))
            }
            Body::SupervisorFresh { project } => {
                self.runner.request_supervisor_fresh(project)?;
                Reply::Status(status(&self.runner)?)
            }
        })
    }

    fn view(&self, t: &Ticket) -> TicketView {
        ticket_view(t, self.runner.pipeline_of(t).ok().as_ref())
    }
}

/// Make a ticket from the project's source, as `dispatch take` does.
pub fn take_issue(
    runner: &mut Runner,
    issues: &dyn Issues,
    project: &str,
    issue: &str,
    now_ms: u64,
) -> Result<Ticket> {
    let path = runner.data.pipeline(project);
    let text = fs::read_to_string(&path)
        .with_context(|| format!("no pipeline for {project} at {}", path.display()))?;
    let pipeline = Pipeline::parse(&text)?;
    let Source::Github { repo, .. } = &pipeline.source else {
        bail!("only a GitHub source is taken by number");
    };
    let number: u64 = issue
        .trim_start_matches('#')
        .parse()
        .context("an issue number")?;
    let source = issues.fetch(repo, number, now_ms)?;
    runner.take(project, &text, source, now_ms)
}

/// Make a ticket from someone else's pull requests, one per lane, on
/// the project's pull-request pipeline. A spec is `<lane>/<number>`,
/// or `<number>` alone where the pipeline has one lane.
pub fn take_pull_requests(
    runner: &mut Runner,
    project: &str,
    specs: &[&str],
    now_ms: u64,
) -> Result<Ticket> {
    let path = runner.data.pr_pipeline(project);
    let text = fs::read_to_string(&path).with_context(|| {
        format!(
            "no pull-request pipeline for {project} at {}",
            path.display()
        )
    })?;
    let pipeline = Pipeline::parse(&text)?;
    if pipeline.source != Source::PullRequest {
        bail!(
            "{} is not a pull-request pipeline: its source is not kind = \"pull-request\"",
            path.display()
        );
    }
    let lanes: Vec<&str> = pipeline.lanes.iter().map(|l| l.name.as_str()).collect();
    let mut prs: Vec<PullRequestSource> = Vec::new();
    for spec in specs {
        let pr = pull_request_source(runner, &pipeline, &lanes, spec)?;
        if prs.iter().any(|x| x.lane == pr.lane) {
            bail!("lane {} is named twice", pr.lane);
        }
        if pipeline.lane(&pr.lane).is_some_and(|l| l.repo.is_none())
            && let Some(other) = prs
                .iter()
                .find(|x| pipeline.lane(&x.lane).is_some_and(|l| l.repo.is_none()))
        {
            bail!(
                "lanes {} and {} share the project's repository, so one pull request covers both",
                other.lane,
                pr.lane
            );
        }
        prs.push(pr);
    }
    let Some(first) = prs.first() else {
        bail!("name at least one pull request as <lane>/<number>");
    };
    let identity = prs
        .iter()
        .map(|pr| format!("{}:{}!{}", pr.provider, pr.repo, pr.number))
        .collect::<Vec<_>>()
        .join("+");
    let body = prs
        .iter()
        .map(|pr| format!("{}: PR #{} {} ({})", pr.lane, pr.number, pr.title, pr.url))
        .collect::<Vec<_>>()
        .join("\n");
    let source = SourceSnapshot {
        kind: "pull-request".into(),
        identity,
        number: Some(first.number),
        title: first.title.clone(),
        body,
        url: Some(first.url.clone()),
        labels: Vec::new(),
        taken_at_ms: now_ms,
        taken_by: None,
        pull_requests: prs,
    };
    runner.take(project, &text, source, now_ms)
}

/// One `<remote>:<lane>/<n>` (or `<lane>/<n>`, or `<n>` with one lane)
/// read from its provider.
fn pull_request_source(
    runner: &Runner,
    pipeline: &Pipeline,
    lanes: &[&str],
    spec: &str,
) -> Result<PullRequestSource> {
    // `<remote>:<lane>/<n>` takes the PR from a named mirror.
    let (remote_name, spec_rest) = match spec.split_once(':') {
        Some((remote, rest)) => (Some(remote), rest),
        None => (None, spec),
    };
    let (lane, number) = match spec_rest.split_once('/') {
        Some((lane, number)) => (lane.to_owned(), number),
        None if lanes.len() == 1 => (lanes[0].to_owned(), spec_rest),
        None => bail!(
            "name the lane as <lane>/<number>; the lanes are {}",
            lanes.join(", ")
        ),
    };
    let number: u64 = number
        .trim_start_matches('#')
        .parse()
        .with_context(|| format!("a pull request number in {spec:?}"))?;
    let def = pipeline
        .lane(&lane)
        .with_context(|| format!("no lane {lane:?}; the lanes are {}", lanes.join(", ")))?;
    let default_remote = if def.repo.is_some() {
        pipeline.lane_remote(def)
    } else {
        pipeline.project.remote.as_str()
    };
    let remote_name = remote_name.unwrap_or(default_remote).to_owned();
    let remote = pipeline
        .remote_url(Some(def), &remote_name)
        .with_context(|| {
            format!("lane {lane} has no remote {remote_name:?} to read a pull request from")
        })?;
    let provider = crate::scheduler::guess_provider(&remote).to_owned();
    let repo = match provider.as_str() {
        "github" => crate::github::github_repo(&remote),
        "bitbucket" => crate::bitbucket::bitbucket_repo(&remote),
        _ => None,
    }
    .with_context(|| format!("lane {lane}: {remote:?} is not on a provider Dispatch reads"))?;
    let pr = runner.prs_for(&provider).by_number(&repo, number)?;
    if pr.state != "open" {
        bail!("PR #{number} in {repo} is {}", pr.state);
    }
    // GitHub serves every PR's head as a pull ref, from whichever
    // fork; Bitbucket has the branch itself.
    let local = if provider == "github" {
        format!("pr/{number}")
    } else {
        pr.branch.clone()
    };
    Ok(PullRequestSource {
        lane,
        provider,
        repo,
        number,
        url: pr.url,
        branch: pr.branch,
        base: pr.base,
        remote: remote_name,
        local,
        head: pr.head,
        title: pr.title,
    })
}

/// A code review attempt's rounds, for the page.
fn round_views(a: &crate::ticket::Attempt) -> Vec<dispatch_control::ReviewRoundView> {
    a.rounds
        .iter()
        .map(|r| dispatch_control::ReviewRoundView {
            n: r.n,
            base: r.base.clone(),
            head: r.head.clone(),
            state: match &r.state {
                crate::ticket::RoundState::Reviewing => "reviewing".to_owned(),
                crate::ticket::RoundState::Converged => "converged".to_owned(),
                crate::ticket::RoundState::Findings => "findings".to_owned(),
                crate::ticket::RoundState::Fixing => "fixing".to_owned(),
                crate::ticket::RoundState::Fixed => "fixed".to_owned(),
                crate::ticket::RoundState::Accepted => "accepted".to_owned(),
                crate::ticket::RoundState::Failed { reason } => {
                    format!("failed: {reason}")
                }
            },
            open_points: r.open_points,
            head_after: r.head_after.clone(),
            feedback: r.feedback.clone(),
            response: r.response.clone(),
            nudges: r.nudges.clone(),
            reviewers: r
                .reviewers
                .iter()
                .map(|x| {
                    (
                        x.name.clone(),
                        match &x.result {
                            None if x.session.is_some() || x.launched => "running".to_owned(),
                            None => "starting".to_owned(),
                            Some(crate::ticket::ReviewerResult::Clean) => "clean".to_owned(),
                            Some(crate::ticket::ReviewerResult::Findings) => "findings".to_owned(),
                            Some(crate::ticket::ReviewerResult::Failed { reason }) => {
                                format!("failed: {reason}")
                            }
                        },
                    )
                })
                .collect(),
        })
        .collect()
}

/// Every project's queue and every ticket.
pub fn status(runner: &Runner) -> Result<Status> {
    let records = runner.tickets()?;
    let mut projects = Vec::new();
    for name in runner.projects()? {
        let ps = runner.load_project(&name)?;
        // The limits come from the project's current pipeline file; the
        // counts are the scheduler's: an active ticket with an open
        // attempt holds a slot, and every pending decision counts.
        let live = fs::read_to_string(runner.data.pipeline(&name))
            .ok()
            .and_then(|text| Pipeline::parse(&text).ok());
        let supervisor = live
            .as_ref()
            .and_then(|p| p.supervisor.as_ref())
            .map(|table| supervisor_view(&name, &ps.supervisor, table));
        let policy = live.map(|p| p.policy).unwrap_or_default();
        let mine = records.iter().filter(|t| t.project == name);
        let running = mine
            .clone()
            .filter(|t| crate::scheduler::ticket_costs_slot(t))
            .count();
        let pending = mine.map(|t| t.waiting_on_you().len()).sum::<usize>();
        projects.push(ProjectView {
            name,
            queue: ps.queue,
            slots: policy.slots,
            waiting_on_me: policy.waiting_on_me,
            running: u32::try_from(running).unwrap_or(u32::MAX),
            pending: u32::try_from(pending).unwrap_or(u32::MAX),
            min_free_gb: policy.min_free_gb,
            free_gb: runner.free_gb(),
            supervisor,
        });
    }
    let mut tickets = Vec::new();
    for t in &records {
        let p = runner.pipeline_of(t).ok();
        let mut view = ticket_view(t, p.as_ref());
        view.waiting_for = p
            .as_ref()
            .and_then(|p| crate::services::waiting_for(t, p, &records));
        tickets.push(view);
    }
    tickets.sort_by_key(|t| std::cmp::Reverse(t.updated_ms));
    Ok(Status {
        data_dir: runner.data.root.clone(),
        worktrees: runner.data.worktrees_dir(),
        projects,
        tickets,
    })
}

/// A project's supervisor as the page shows it.
fn supervisor_view(
    project: &str,
    s: &crate::ticket::Supervision,
    table: &crate::pipeline::Supervisor,
) -> dispatch_control::SupervisorView {
    dispatch_control::SupervisorView {
        session: s.current.as_ref().map(|c| c.session.clone()),
        created_ms: s.current.as_ref().map_or(0, |c| c.created_ms),
        seed_stale: s.seed_stale(project, table),
        replaced: u32::try_from(s.past.len()).unwrap_or(u32::MAX),
        fresh_pending: s.intent.is_some(),
        error: s.error.clone(),
    }
}

/// An attempt's state as a word, and the reason a failed or cancelled
/// one gave.
#[must_use]
pub fn attempt_state(state: &AttemptState) -> (&'static str, Option<String>) {
    match state {
        AttemptState::Starting => ("starting", None),
        AttemptState::Running => ("running", None),
        AttemptState::Complete => ("complete", None),
        AttemptState::Failed { reason } => ("failed", Some(reason.clone())),
        AttemptState::Cancelled { reason } => ("cancelled", Some(reason.clone())),
    }
}

/// `ticket`'s events after `since`, with the seqs its voids in the same
/// batch withdraw. No writer lock: `read_since` skips a torn tail.
fn ticket_events(log: &Path, ticket: &str, since: u64) -> Result<EventsView> {
    let batch = events::read_since(log, since)?;
    let last = batch.iter().map(|e| e.seq).max().unwrap_or(since);
    let mine: Vec<&events::Event> = batch.iter().filter(|e| e.ticket == ticket).collect();
    let withdrawn: Vec<u64> = mine
        .iter()
        .filter(|e| e.kind == events::Kind::Void)
        .flat_map(|e| e.voids.iter().copied())
        .collect();
    let events = mine
        .into_iter()
        .filter(|e| e.kind != events::Kind::Void && !withdrawn.contains(&e.seq))
        .map(|e| EventView {
            seq: e.seq,
            at_ms: e.at_ms,
            stage: e.stage.clone(),
            kind: e.kind.as_str().to_owned(),
            text: e.text.clone(),
            attempt: e.attempt.clone(),
            decision: e.decision.clone(),
            head: e.head.clone(),
            url: e.url.clone(),
        })
        .collect();
    Ok(EventsView {
        events,
        last,
        withdrawn,
    })
}

/// Every plan review round beside the workflow attempt's reviewed copy
/// whose feedback exists, with its response when that exists too, and
/// who opened it when it was the owner's objection. A round with no
/// file (one from the app's notes box) is skipped and keeps its number.
fn plan_rounds(a: &Attempt) -> Vec<PlanRoundView> {
    let Some(subject) = a.artifacts.values().next() else {
        return Vec::new();
    };
    crate::report::plan_round_numbers(subject)
        .into_iter()
        .map(|n| PlanRoundView {
            n,
            feedback: crate::report::round_file(subject, n),
            response: Some(crate::report::response_file(subject, n)).filter(|r| r.exists()),
            by: a
                .revisions
                .iter()
                .find(|x| x.round == n)
                .map(|x| x.by.clone()),
        })
        .collect()
}

/// The newest round of the plan review `a` is, from its feedback files
/// or a `revise` answer, since a round opened from the app's box has no
/// feedback file; none for an attempt that is not a review.
fn review_round(a: &Attempt) -> Option<u32> {
    if a.kind != AttemptKind::Workflow {
        return None;
    }
    let file = plan_rounds(a).last().map(|r| r.n);
    let revised = a.revisions.iter().map(|r| r.round).max();
    file.max(revised)
}

/// Where a ticket's documents are, for `show` and the port's
/// single-ticket reply; the one part of a view that touches files.
#[must_use]
pub fn ticket_paths(t: &Ticket, p: Option<&Pipeline>) -> PathsView {
    let plan_files = shown_lane_files(t, p, "plan")
        .into_iter()
        .map(|(lane, a, path)| LaneFile {
            lane: lane.map(str::to_owned),
            stage: a.stage.clone(),
            path: path.clone(),
            reviewing: a.kind == AttemptKind::Workflow && a.is_open(),
            round: review_round(a),
        })
        .collect();
    let notes_files = lane_files(t, p, "notes")
        .into_iter()
        .map(|(lane, stage, path)| LaneFile {
            lane: lane.map(str::to_owned),
            stage: stage.to_owned(),
            path: path.clone(),
            ..LaneFile::default()
        })
        .collect();
    let last_round = t
        .attempts
        .iter()
        .rev()
        .flat_map(|a| a.rounds.iter().rev())
        .find_map(|r| r.feedback.clone());
    let review = t
        .attempts
        .iter()
        .rev()
        .filter(|a| a.kind == AttemptKind::Workflow)
        .find(|a| !a.artifacts.is_empty());
    let shown = t.shown("plan");
    let reviewed = shown.filter(|(a, _)| a.kind == AttemptKind::Workflow);
    let plan_rounds = review.map_or_else(Vec::new, plan_rounds);
    let round_file = last_round.or_else(|| plan_rounds.last().map(|r| r.feedback.clone()));
    let pr = t.attempts.iter().rev().find_map(|a| a.pr.as_ref());
    PathsView {
        plan: shown.map(|(_, path)| path.clone()),
        plan_stage: shown.map(|(a, _)| a.stage.clone()),
        plan_reviewed: reviewed.is_some(),
        plan_reviewing: reviewed.is_some_and(|(a, _)| a.is_open()),
        plan_round: reviewed.and_then(|(a, _)| review_round(a)),
        round_file,
        review_summary: t
            .attempts
            .iter()
            .rev()
            .find_map(|a| a.artifacts.get("summary").cloned()),
        notes: t.input("notes").cloned(),
        pr_url: pr.map(|pr| pr.url.clone()),
        pr_head: pr.map(|pr| pr.head.clone()),
        plan_rounds,
        plan_files,
        notes_files,
    }
}

/// The latest head an attempt in the lane recorded: the record has no
/// head of its own for a lane, and `show` reads no git.
fn lane_head(t: &Ticket, lane: &str) -> Option<String> {
    t.attempts
        .iter()
        .filter(|a| a.context == lane && a.head.is_some())
        .max_by_key(|a| a.started_ms)
        .and_then(|a| a.head.clone())
}

fn lane_view(t: &Ticket, p: Option<&Pipeline>, l: &crate::ticket::LaneRecord) -> LaneView {
    let spec = p.and_then(|p| p.lane(&l.name));
    LaneView {
        name: l.name.clone(),
        worktree: l.worktree.clone(),
        branch: l.branch.clone(),
        chosen: l.chosen,
        setup_done: l.setup_done,
        removed: l.removed,
        base_sha: l.base_sha.clone(),
        head: lane_head(t, &l.name),
        pushed_head: l.pushed.as_ref().map(|p| p.head.clone()),
        rebase_conflicts: l
            .refreshed
            .as_ref()
            .and_then(|r| r.conflict.as_ref())
            .map(|c| u32::try_from(c.commits.len()).unwrap_or(u32::MAX)),
        resolution: l
            .refreshed
            .as_ref()
            .filter(|r| r.conflict.is_some())
            .and_then(|r| crate::scheduler::resolution_of(t, &l.name, r))
            .map(|a| a.n),
        brought_up_by: l
            .refreshed
            .as_ref()
            .map(|r| crate::scheduler::brought_up_by(t, &l.name, r)),
        brought_up_commits: l.refreshed.as_ref().is_some_and(|r| r.commits),
        clone: None,
        merge_after: spec.map_or_else(Vec::new, |s| s.merge_after.clone()),
        merge_after_run: spec.is_some_and(|s| s.deploy_wait() == DeployWait::Run),
        merge_after_step: spec.and_then(|s| match s.deploy_wait() {
            DeployWait::Step(step) => Some(step.to_owned()),
            DeployWait::Merge | DeployWait::Run => None,
        }),
    }
}

/// A ticket as a reader sees it, with what its frozen pipeline copy
/// says (stage names, the paths a close removes) when it can be read.
/// Its `paths` are left empty; `ticket_paths` fills them.
#[must_use]
pub fn ticket_view(t: &Ticket, p: Option<&Pipeline>) -> TicketView {
    let (state, reason) = match &t.state {
        TicketState::Active => ("active", None),
        TicketState::Parking { reason } => ("parking", Some(reason.clone())),
        TicketState::Parked { reason } => ("parked", Some(reason.clone())),
        TicketState::Closing { reason } => ("closing", Some(reason.clone())),
        TicketState::Closed { reason } => ("closed", Some(reason.clone())),
    };
    TicketView {
        id: t.id.clone(),
        project: t.project.clone(),
        kind: t.source.kind.clone(),
        number: t.source.number,
        title: t.source.title.clone(),
        body: t.source.body.clone(),
        url: t.source.url.clone(),
        labels: t.source.labels.clone(),
        state: state.into(),
        reason,
        stages: p.map_or_else(Vec::new, |p| {
            p.stages.iter().map(|s| s.name.clone()).collect()
        }),
        stage_checks: p.map_or_else(Vec::new, |p| {
            p.stages
                .iter()
                .map(|s| match &s.gate {
                    Some(crate::pipeline::Gate::External { check, .. }) => Some(check.clone()),
                    _ => None,
                })
                .collect()
        }),
        stage: t.stage,
        tree: t.tree.clone(),
        tree_removed: t.close.tree_removed,
        trees_kept: t.close.trees_kept.clone(),
        closable: t.closable(),
        trees_retryable: t.trees_retryable(),
        removes: p.map_or_else(Vec::new, |p| crate::scheduler::close_removes(t, p)),
        lanes: t.lanes.iter().map(|l| lane_view(t, p, l)).collect(),
        attempts: t.attempts.iter().map(attempt_view).collect(),
        decisions: t.decisions.iter().map(|d| decision_view(t, d)).collect(),
        root_project: t.root_project.clone(),
        current_session: t.current_session().cloned(),
        holds: t.holds.iter().map(|h| h.resource.clone()).collect(),
        // Filled by `status`, which reads every ticket.
        waiting_for: None,
        services: service_views(t),
        created_ms: t.created_ms,
        updated_ms: t.updated_ms,
        paths: PathsView::default(),
        restarts: t.restarts.iter().map(restart_view).collect(),
    }
}

/// One attempt as the port shows it: secret artifacts by name only.
fn attempt_view(a: &crate::ticket::Attempt) -> AttemptView {
    let (state, reason) = attempt_state(&a.state);
    // An attempt ended by a send-back or a hand merge may keep a wait
    // that never released; it holds nothing once the attempt is over.
    let held = a.waits.as_ref().filter(|w| a.is_open() && w.holds());
    AttemptView {
        stage: a.stage.clone(),
        n: a.n,
        context: a.context.clone(),
        kind: match a.kind {
            AttemptKind::Agent => "agent",
            AttemptKind::Workflow => "workflow",
            AttemptKind::GateOnly => "gate-only",
            AttemptKind::Review => "review",
        }
        .into(),
        rounds: round_views(a),
        nudges: a.nudges.clone(),
        secret: a.secret.iter().cloned().collect(),
        forgotten: a.forgotten.keys().cloned().collect(),
        state: state.into(),
        reason,
        session: a.session.clone(),
        run: a.run.clone(),
        artifacts: a
            .artifacts
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        head: a.head.clone(),
        checks: a.gate.as_ref().map(|g| dispatch_control::ChecksView {
            head: g.head.clone(),
            exit: g.exit,
            log: g.log.clone(),
        }),
        pr: a.pr.as_ref().map(|pr| dispatch_control::PullRequestView {
            provider: pr.provider.clone(),
            repo: pr.repo.clone(),
            number: pr.number,
            url: pr.url.clone(),
            head: pr.head.clone(),
            checks: pr.checks.clone(),
            merge_commit: pr.merge_commit.clone(),
        }),
        waits: held.map(crate::ticket::MergeWait::describe),
        waits_since_ms: held.map(|w| w.since_ms),
        held: a
            .held
            .iter()
            .filter(|_| a.is_open())
            .map(|(session, h)| dispatch_control::HeldView {
                session: session.clone(),
                pending: h.pending.clone(),
                wakeup_at_ms: h.wakeup_at_ms,
                until_ms: h.until_ms,
            })
            .collect(),
        rewrite: a.rewrite.as_ref().map(|r| dispatch_control::RewriteView {
            mode: r.mode.as_str().to_owned(),
            before: r.before.clone(),
            after: r.after.clone(),
            from: r.from,
            to: r.to,
            skipped: r.skipped.clone(),
            stale: r.stale_names(),
            message: r.message_outcome(),
            message_head: r.message.as_ref().and_then(|m| m.to.clone()),
            message_failed: r.message.as_ref().is_some_and(|m| m.failed.is_some()),
            message_session: r.message.as_ref().and_then(|m| m.session.clone()),
        }),
        started_ms: a.started_ms,
        ended_ms: a.ended_ms,
    }
}

fn restart_view(r: &crate::ticket::Restart) -> dispatch_control::RestartView {
    dispatch_control::RestartView {
        at_ms: r.at_ms,
        from: r.from.clone(),
        to: r.to.clone(),
        before: r.before.clone(),
        after: r.after.clone(),
        discarded: r
            .discarded
            .iter()
            .map(|(s, n)| format!("{s}/{n}"))
            .collect(),
        reset: r.reset.iter().map(ToString::to_string).collect(),
        setup_again: r.setup_again.clone(),
    }
}

/// The ticket's services not yet stopped.
fn service_views(t: &Ticket) -> Vec<dispatch_control::ServiceView> {
    t.services
        .iter()
        .filter(|s| s.state != crate::ticket::ServiceState::Stopped)
        .map(|s| dispatch_control::ServiceView {
            lane: s.lane.clone(),
            url: s.url.clone(),
            state: s.state.label(),
            session: s.session.clone(),
        })
        .collect()
}

/// A pending decision the ticket no longer waits on (it is closing)
/// reads as `cancelling`, so no view offers to answer it or counts it.
fn decision_view(t: &Ticket, d: &Decision) -> DecisionView {
    let waiting = t.waiting_on_you().iter().any(|w| w.id == d.id);
    let (state, answer, note) = match &d.state {
        DecisionState::Pending if waiting => ("pending", None, None),
        DecisionState::Pending => ("cancelling", None, None),
        DecisionState::Answered {
            answer,
            note,
            acted,
            ..
        } => (
            if *acted { "acted" } else { "answered" },
            Some(answer.clone()),
            note.clone(),
        ),
        DecisionState::Cancelled => ("cancelled", None, None),
    };
    let (answered_by, answered_ms) = match &d.state {
        DecisionState::Answered { by, at_ms, .. } => (Some(by.clone()), Some(*at_ms)),
        _ => (None, None),
    };
    DecisionView {
        id: d.id.clone(),
        ticket: t.id.clone(),
        stage: d.stage.clone(),
        name: d.name.clone(),
        question: d.question.clone(),
        options: d.options.clone(),
        recommendation: d.recommendation.clone(),
        multiple: d.name == "lanes",
        state: state.into(),
        answer,
        note,
        made_ms: d.made_ms,
        answered_by,
        answered_ms,
        needs_note: d
            .options
            .iter()
            .filter(|o| crate::needs_note(&d.name, o))
            .cloned()
            .collect(),
    }
}

/// The socket, owner-only, with one thread accepting and one per
/// connection; each request is answered under the handler's lock.
/// Dropping it closes the socket and removes the file.
pub struct Server {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    pub fn bind(data: &DataDir, handler: Handler) -> io::Result<Self> {
        fs::create_dir_all(&data.root)?;
        let path = data.root.join(SOCKET_FILE);
        if path.as_os_str().len() > MAX_SOCKET_PATH {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "socket path {} is longer than {MAX_SOCKET_PATH} bytes; use a shorter data directory",
                    path.display()
                ),
            ));
        }
        // Only the runner that holds `runner.lock` gets here, so a file
        // left by a dead runner is stale.
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let handler = Arc::new(Mutex::new(handler));
        let thread = {
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("dispatch-port".into())
                .spawn(move || {
                    for stream in listener.incoming() {
                        if stop.load(Ordering::SeqCst) {
                            break;
                        }
                        let Ok(stream) = stream else { continue };
                        let handler = Arc::clone(&handler);
                        let _ = std::thread::Builder::new()
                            .name("dispatch-port-conn".into())
                            .spawn(move || serve_connection(stream, &handler));
                    }
                })?
        };
        Ok(Self {
            path,
            stop,
            thread: Some(thread),
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(&self.path);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = fs::remove_file(&self.path);
    }
}

fn serve_connection(stream: UnixStream, handler: &Mutex<Handler>) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match Request::parse(&line) {
            Err(e) => Reply::failed(format!("bad request: {e}")),
            Ok(request) => {
                let now = epoch_ms(std::time::SystemTime::now());
                match handler.lock() {
                    Ok(mut h) => h.handle(&request, now),
                    Err(_) => Reply::failed("the runner's handler is poisoned"),
                }
            }
        };
        if writer.write_all(reply.to_line().as_bytes()).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use dispatch_control::Client;

    use super::*;
    use crate::git::FakeRepo;
    use crate::github::FakeIssues;
    use crate::port::Port;

    struct NoPort;
    impl Port for NoPort {
        fn call(
            &mut self,
            _: &switchboard_control::Request,
        ) -> io::Result<switchboard_control::Reply> {
            Err(io::Error::other("no Switchboard in this test"))
        }
    }

    const PIPELINE: &str = r#"
version = 1
[project]
name = "P"
root = "/tmp/p"
space = "S"
[source]
kind = "github"
repo = "o/r"
label = "dispatch"
[[lanes]]
name = "repo"
path = "."
[operators.a]
kind = "claude"
[[stages]]
name = "investigate"
operator = "a"
context = "root"
writes = ["notes"]
prompt = "go {notes}"
[policy]
slots = 1
"#;

    fn handler(dir: &Path) -> Handler {
        let data = DataDir::new(dir.join("d"));
        fs::create_dir_all(data.root.join("pipelines")).unwrap();
        fs::write(data.pipeline("P"), PIPELINE).unwrap();
        Handler {
            runner: Runner::new(data, Box::new(NoPort), Box::new(FakeRepo::default())),
            issues: Box::new(FakeIssues {
                issues: vec![("o/r".into(), 7, "Seven".into(), "body".into())],
            }),
        }
    }

    #[test]
    fn the_port_takes_reads_and_decides_like_the_command_line() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = handler(dir.path());
        let taken = h.handle(
            &Request::new(
                "1",
                Body::Take {
                    project: "P".into(),
                    issue: "#7".into(),
                },
            ),
            1_000,
        );
        let Reply::Taken(t) = taken else {
            panic!("{taken:?}")
        };
        assert_eq!(
            (t.number, t.stages.len(), t.state.as_str()),
            (Some(7), 1, "active")
        );
        let Reply::Status(status) = h.handle(&Request::new("2", Body::Status), 2_000) else {
            panic!("a status")
        };
        assert_eq!(status.projects[0].queue, vec![t.id.clone()]);
        assert_eq!(
            (status.projects[0].slots, status.projects[0].running),
            (1, 0),
            "the pipeline's limit and nothing running yet"
        );
        assert_eq!(status.tickets[0].title, "Seven");
        assert_eq!(status.data_dir, h.runner.data.root);
        // A file outside the ticket's directory is refused by path.
        let outside = h.handle(
            &Request::new(
                "3",
                Body::Artifact {
                    ticket: t.id.clone(),
                    path: h.runner.data.pipeline("P"),
                },
            ),
            3_000,
        );
        assert!(matches!(outside, Reply::Failed { .. }), "{outside:?}");
        let file = h.runner.data.ticket_dir(&t.id).join("notes.md");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "# notes").unwrap();
        assert_eq!(
            h.handle(
                &Request::new(
                    "4",
                    Body::Artifact {
                        ticket: t.id.clone(),
                        path: file,
                    },
                ),
                4_000,
            ),
            Reply::Artifact {
                text: "# notes".into()
            }
        );
        let no_such = h.handle(
            &Request::new(
                "5",
                Body::Decide {
                    ticket: t.id.clone(),
                    decision: "d9".into(),
                    answer: "yes".into(),
                    note: None,
                },
            ),
            5_000,
        );
        assert!(matches!(no_such, Reply::Failed { .. }));
    }

    /// A close through the port: only the intent in the reply, the
    /// ticket `closing`; the runner's pass removes the tree and closes
    /// it; a second close refused with the reason.
    #[test]
    fn the_port_closes_a_ticket_and_refuses_a_second_close() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = handler(dir.path());
        // A pipeline that cuts its own trees, with the tree cut by hand:
        // stepping would need a Switchboard this test does not have.
        let text = PIPELINE.replace("root = \"/tmp/p\"", "repo = \"git@example.com:o/r.git\"");
        fs::write(h.runner.data.pipeline("P"), text).unwrap();
        let Reply::Taken(view) = h.handle(
            &Request::new(
                "1",
                Body::Take {
                    project: "P".into(),
                    issue: "7".into(),
                },
            ),
            1_000,
        ) else {
            panic!("taken")
        };
        let tree = dir.path().join("wt").join(&view.id);
        fs::create_dir_all(&tree).unwrap();
        let mut t = h.runner.load_ticket(&view.id).unwrap();
        t.tree = Some(tree.clone());
        h.runner.save_ticket(&mut t, 1_500).unwrap();
        let close = Body::Close {
            ticket: view.id.clone(),
            reason: Some("done elsewhere".into()),
        };
        let reply = h.handle(&Request::new("2", close.clone()), 2_000);
        let Reply::Ticket(closing) = reply else {
            panic!("{reply:?}")
        };
        assert_eq!(
            (closing.state.as_str(), closing.reason.as_deref()),
            ("closing", Some("done elsewhere"))
        );
        assert!(tree.exists(), "the port leaves the rest to the runner");
        let mut t = h.runner.load_ticket(&view.id).unwrap();
        let mut ps = h.runner.load_project("P").unwrap();
        h.runner.finish_closing(&mut t, &mut ps, 2_500).unwrap();
        let closed = h.view(&h.runner.load_ticket(&view.id).unwrap());
        assert_eq!(
            (closed.state.as_str(), closed.reason.as_deref()),
            ("closed", Some("done elsewhere"))
        );
        assert!(closed.tree_removed && closed.trees_kept.is_none());
        assert!(!closed.closable && !closed.trees_retryable);
        assert!(!tree.exists());
        let again = h.handle(&Request::new("3", close), 3_000);
        assert!(
            matches!(&again, Reply::Failed { reason } if reason.contains("already closed")),
            "{again:?}"
        );
    }

    fn take(h: &mut Handler) -> TicketView {
        let reply = h.handle(
            &Request::new(
                "t",
                Body::Take {
                    project: "P".into(),
                    issue: "7".into(),
                },
            ),
            1_000,
        );
        let Reply::Taken(t) = reply else {
            panic!("{reply:?}")
        };
        t
    }

    fn event(seq: u64, ticket: &str, kind: events::Kind, text: &str) -> events::Event {
        events::Event {
            v: events::EVENT_VERSION,
            seq,
            at_ms: seq * 1_000,
            ticket: ticket.into(),
            project: "P".into(),
            stage: "investigate".into(),
            kind,
            text: text.into(),
            attempt: Some(("investigate".into(), 1)),
            decision: None,
            head: None,
            url: None,
            voids: Vec::new(),
            by: None,
            conflicts: None,
            actor: None,
        }
    }

    #[test]
    fn the_port_reads_a_tickets_events_after_a_cursor_without_the_withdrawn() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = handler(dir.path());
        let log = events::log_path(&h.runner.data);
        let mut batch = [
            event(0, "t1", events::Kind::AttemptStarted, "started"),
            event(0, "t2", events::Kind::AttemptStarted, "other ticket"),
            event(0, "t1", events::Kind::AttemptEnded, "ended"),
        ];
        events::append(&log, &mut batch).unwrap();
        events::append_void(&log, &batch[2], vec![batch[2].seq], "disk full").unwrap();
        let ask = |h: &mut Handler, since: u64| {
            let body = Body::Events {
                ticket: "t1".into(),
                since,
            };
            match h.handle(&Request::new("e", body), 9_000) {
                Reply::Events(v) => v,
                other => panic!("{other:?}"),
            }
        };
        let v = ask(&mut h, 0);
        assert_eq!(
            v.events
                .iter()
                .map(|e| (e.seq, e.kind.as_str(), e.text.as_str()))
                .collect::<Vec<_>>(),
            vec![(1, "attempt-started", "started")]
        );
        assert_eq!(v.events[0].attempt, Some(("investigate".into(), 1)));
        assert_eq!((v.last, v.withdrawn), (4, vec![3]));
        let again = ask(&mut h, v.last);
        assert!(again.events.is_empty() && again.withdrawn.is_empty());
        assert_eq!(again.last, 4);
    }

    #[test]
    fn the_paths_list_a_per_lane_plan_once_per_lane() {
        let text = PIPELINE.replace(
            "[operators.a]",
            "[[lanes]]\nname = \"docs\"\npath = \"docs\"\n[operators.a]",
        ) + "[[stages]]\nname = \"outline\"\noperator = \"a\"\ncontext = \"root\"\nwrites = [\"plan\"]\nprompt = \"go {plan}\"\n[[stages]]\nname = \"plan\"\noperator = \"a\"\ncontext = \"each\"\nwrites = [\"plan\"]\nprompt = \"go {plan}\"\n";
        let p = Pipeline::parse(&text).unwrap();
        let attempt = |stage: &str, n: u32, ctx: &str, path: &str| {
            crate::scheduler::new_attempt(
                stage,
                n,
                ctx,
                AttemptKind::Agent,
                AttemptState::Complete,
                std::collections::BTreeMap::from([("plan".to_owned(), PathBuf::from(path))]),
                0,
            )
        };
        let mut t = crate::ticket::blank();
        t.attempts.push(attempt("outline", 1, "root", "/plan.md"));
        let paths = ticket_paths(&t, Some(&p));
        assert_eq!(
            paths.plan_files,
            [LaneFile {
                lane: None,
                stage: "outline".into(),
                path: "/plan.md".into(),
                ..LaneFile::default()
            }]
        );
        assert_eq!(paths.plan, Some(PathBuf::from("/plan.md")));
        for lane in ["repo", "docs"] {
            t.lanes.push(crate::ticket::chosen_lane(lane));
        }
        t.attempts.push(attempt("plan", 1, "repo", "/repo.md"));
        t.attempts.push(attempt("plan", 1, "docs", "/docs.md"));
        let paths = ticket_paths(&t, Some(&p));
        let file = |lane: &str, path: &str| LaneFile {
            lane: Some(lane.into()),
            stage: "plan".into(),
            path: path.into(),
            ..LaneFile::default()
        };
        assert_eq!(
            paths.plan_files,
            [file("repo", "/repo.md"), file("docs", "/docs.md")]
        );
        // The single field is the newest plan, as older clients read it.
        assert_eq!(paths.plan, Some(PathBuf::from("/docs.md")));
        assert!(paths.notes_files.is_empty());
    }

    #[test]
    fn the_paths_name_an_open_review_copy_with_its_round() {
        let dir = tempfile::tempdir().unwrap();
        let p = Pipeline::parse(PIPELINE).unwrap();
        let copy = dir.path().join("review/1/plan.md");
        fs::create_dir_all(copy.parent().unwrap()).unwrap();
        let attempt = |stage: &str, kind: AttemptKind, state: AttemptState, path: &Path| {
            crate::scheduler::new_attempt(
                stage,
                1,
                "root",
                kind,
                state,
                std::collections::BTreeMap::from([("plan".to_owned(), path.to_path_buf())]),
                0,
            )
        };
        let mut t = crate::ticket::blank();
        t.attempts.push(attempt(
            "plan",
            AttemptKind::Agent,
            AttemptState::Complete,
            Path::new("/plan/1/plan.md"),
        ));
        let paths = ticket_paths(&t, Some(&p));
        assert_eq!(paths.plan, Some(PathBuf::from("/plan/1/plan.md")));
        assert_eq!(paths.plan_stage.as_deref(), Some("plan"));
        assert!(!paths.plan_reviewed && !paths.plan_reviewing);
        assert_eq!(paths.plan_round, None);

        t.attempts.push(attempt(
            "review-plan",
            AttemptKind::Workflow,
            AttemptState::Running,
            &copy,
        ));
        // Round 1 before the reviewer writes its first file.
        let paths = ticket_paths(&t, Some(&p));
        assert_eq!(paths.plan, Some(copy.clone()));
        assert_eq!(
            paths.plan_files,
            [LaneFile {
                lane: None,
                stage: "review-plan".into(),
                path: copy.clone(),
                reviewing: true,
                round: None,
            }]
        );
        assert_eq!(paths.plan_stage.as_deref(), Some("review-plan"));
        assert!(paths.plan_reviewed && paths.plan_reviewing);
        assert_eq!(paths.plan_round, None);

        fs::write(crate::report::round_file(&copy, 1), "x").unwrap();
        assert_eq!(ticket_paths(&t, Some(&p)).plan_round, Some(1));
        // A round from the app's box writes no file; its revision counts.
        t.attempts[1].revisions.push(crate::ticket::Revision {
            round: 2,
            by: "you".into(),
            at_ms: 5,
        });
        let paths = ticket_paths(&t, Some(&p));
        assert_eq!(paths.plan_round, Some(2));
        assert_eq!(paths.plan_files[0].round, Some(2));

        t.attempts[1].revisions.clear();
        fs::remove_file(crate::report::round_file(&copy, 1)).unwrap();
        t.attempts[1].state = AttemptState::Complete;
        let paths = ticket_paths(&t, Some(&p));
        assert!(paths.plan_reviewed && !paths.plan_reviewing);
        assert_eq!(paths.plan_round, None);
    }

    #[test]
    fn a_lanes_open_review_copy_stands_in_for_that_lane_only() {
        let text = PIPELINE.replace(
            "[operators.a]",
            "[[lanes]]\nname = \"docs\"\npath = \"docs\"\n[operators.a]",
        ) + "[[stages]]\nname = \"plan\"\noperator = \"a\"\ncontext = \"each\"\nwrites = [\"plan\"]\nprompt = \"go {plan}\"\n[[stages]]\nname = \"review-plan\"\noperator = \"a\"\ncontext = \"each\"\nwrites = [\"plan\"]\nprompt = \"go {plan}\"\n";
        let p = Pipeline::parse(&text).unwrap();
        let attempt =
            |stage: &str, ctx: &str, kind: AttemptKind, state: AttemptState, path: &str| {
                crate::scheduler::new_attempt(
                    stage,
                    1,
                    ctx,
                    kind,
                    state,
                    std::collections::BTreeMap::from([("plan".to_owned(), PathBuf::from(path))]),
                    0,
                )
            };
        let mut t = crate::ticket::blank();
        for lane in ["repo", "docs"] {
            t.lanes.push(crate::ticket::chosen_lane(lane));
        }
        for lane in ["repo", "docs"] {
            t.attempts.push(attempt(
                "plan",
                lane,
                AttemptKind::Agent,
                AttemptState::Complete,
                &format!("/plan/1/{lane}/plan.md"),
            ));
        }
        t.attempts.push(attempt(
            "review-plan",
            "repo",
            AttemptKind::Workflow,
            AttemptState::Running,
            "/review/1/repo/plan.md",
        ));
        let file = |lane: &str, stage: &str, path: &str, reviewing: bool| LaneFile {
            lane: Some(lane.into()),
            stage: stage.into(),
            path: path.into(),
            reviewing,
            round: None,
        };
        assert_eq!(
            ticket_paths(&t, Some(&p)).plan_files,
            [
                file("repo", "review-plan", "/review/1/repo/plan.md", true),
                file("docs", "plan", "/plan/1/docs/plan.md", false),
            ]
        );
        // A later lane's finished review leaves the open one marked in
        // its own lane, whatever the newest attempt says.
        t.attempts.push(attempt(
            "review-plan",
            "docs",
            AttemptKind::Workflow,
            AttemptState::Complete,
            "/review/1/docs/plan.md",
        ));
        let paths = ticket_paths(&t, Some(&p));
        assert!(!paths.plan_reviewing);
        assert_eq!(
            paths.plan_files,
            [
                file("repo", "review-plan", "/review/1/repo/plan.md", true),
                file("docs", "review-plan", "/review/1/docs/plan.md", false),
            ]
        );
        // A lane's reader is still handed only a complete plan.
        assert_eq!(
            crate::scheduler::lane_plans(&t, &p, None)
                .into_iter()
                .map(|(_, plan)| plan.clone())
                .collect::<Vec<_>>(),
            [
                PathBuf::from("/plan/1/repo/plan.md"),
                PathBuf::from("/review/1/docs/plan.md"),
            ]
        );
    }

    #[test]
    fn a_single_ticket_names_its_plan_rounds_and_its_lanes_clone() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = handler(dir.path());
        let view = take(&mut h);
        let tdir = h.runner.data.ticket_dir(&view.id);
        fs::create_dir_all(&tdir).unwrap();
        let plan = tdir.join("plan.md");
        for f in [
            "plan.md",
            "plan.feedback-1.md",
            "plan.response-1.md",
            "plan.feedback-2.md",
        ] {
            fs::write(tdir.join(f), "x").unwrap();
        }
        let mut t = h.runner.load_ticket(&view.id).unwrap();
        t.attempts.push(crate::scheduler::new_attempt(
            "review-plan",
            1,
            "root",
            AttemptKind::Workflow,
            AttemptState::Complete,
            [("plan".to_owned(), plan)].into(),
            1_100,
        ));
        t.lanes.push(crate::ticket::LaneRecord {
            name: "repo".into(),
            worktree: dir.path().join("wt"),
            branch: "dispatch/7-seven".into(),
            project: None,
            chosen: true,
            setup_done: true,
            base_sha: Some("base0000".into()),
            refreshed: None,
            pushed: None,
            conflict: None,
            removed: true,
        });
        t.decisions.push(Decision {
            id: "d1".into(),
            stage: "investigate".into(),
            name: "finalize".into(),
            kind: crate::ticket::DecisionKind::Permission,
            question: "Finalize it?".into(),
            options: vec!["finalize".into()],
            recommendation: None,
            attempt: None,
            state: DecisionState::Answered {
                answer: "finalize".into(),
                note: None,
                by: "user".into(),
                at_ms: 1_234,
                acted: false,
            },
            made_ms: 1_200,
            refusals: Vec::new(),
        });
        h.runner.save_ticket(&mut t, 1_500).unwrap();
        let reply = h.handle(&Request::new("1", Body::Ticket { id: view.id }), 2_000);
        let Reply::Ticket(t) = reply else {
            panic!("{reply:?}")
        };
        let rounds: Vec<_> = t
            .paths
            .plan_rounds
            .iter()
            .map(|r| (r.n, r.feedback.clone(), r.response.clone()))
            .collect();
        assert_eq!(
            rounds,
            vec![
                (
                    1,
                    tdir.join("plan.feedback-1.md"),
                    Some(tdir.join("plan.response-1.md"))
                ),
                (2, tdir.join("plan.feedback-2.md"), None),
            ]
        );
        assert_eq!(t.lanes[0].clone, Some(h.runner.data.repo_dir("P")));
        assert_eq!(
            (
                t.decisions[0].answered_by.as_deref(),
                t.decisions[0].answered_ms
            ),
            (Some("user"), Some(1_234))
        );
        let Reply::Status(status) = h.handle(&Request::new("2", Body::Status), 3_000) else {
            panic!("a status")
        };
        assert_eq!(
            status.tickets[0].lanes[0].clone, None,
            "a status reads no pipeline paths"
        );
    }

    /// A round with no file (from the app's notes box) is skipped and
    /// keeps its number; the owner's round names who sent it.
    #[test]
    fn plan_rounds_skip_a_missing_round_and_name_the_owners() {
        let dir = tempfile::tempdir().unwrap();
        let plan = dir.path().join("plan.md");
        for f in [
            "plan.md",
            "plan.feedback-1.md",
            "plan.feedback-3.md",
            "plan.response-3.md",
            "plan.feedback-4.md",
        ] {
            fs::write(dir.path().join(f), "x").unwrap();
        }
        let mut a = crate::scheduler::new_attempt(
            "review-plan",
            1,
            "root",
            AttemptKind::Workflow,
            AttemptState::Complete,
            [("plan".to_owned(), plan)].into(),
            1_100,
        );
        a.revisions.push(crate::ticket::Revision {
            round: 3,
            by: "supervisor".into(),
            at_ms: 1_200,
        });
        let rounds = plan_rounds(&a);
        let seen: Vec<_> = rounds
            .iter()
            .map(|r| (r.n, r.response.is_some(), r.by.as_deref()))
            .collect();
        assert_eq!(
            seen,
            [
                (1, false, None),
                (3, true, Some("supervisor")),
                (4, false, None)
            ]
        );
    }

    #[test]
    fn only_revise_on_finalize_needs_a_note() {
        let decision = |name: &str, options: &[&str]| Decision {
            id: "d1".into(),
            stage: "review-plan".into(),
            name: name.into(),
            kind: crate::ticket::DecisionKind::Permission,
            question: "?".into(),
            options: options.iter().map(|o| (*o).to_owned()).collect(),
            recommendation: None,
            attempt: None,
            state: DecisionState::Pending,
            made_ms: 0,
            refusals: Vec::new(),
        };
        let t = crate::ticket::blank();
        let view = decision_view(&t, &decision("finalize", &["finalize", "revise", "park"]));
        assert_eq!(view.needs_note, ["revise"]);
        let view = decision_view(&t, &decision("inspect", &["proceed", "rerun", "park"]));
        assert!(view.needs_note.is_empty());
    }

    #[test]
    fn the_socket_answers_a_client_and_leaves_no_file_behind() {
        let dir = std::env::temp_dir().join(format!("dsv-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let h = handler(&dir);
        let data = DataDir::new(h.runner.data.root.clone());
        let server = Server::bind(&data, h).unwrap();
        let mut client = Client::connect(server.path()).unwrap();
        let reply = client.call(&Request::new("s", Body::Status)).unwrap();
        assert!(matches!(reply, Reply::Status(_)), "{reply:?}");
        let path = server.path().to_path_buf();
        drop(server);
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
