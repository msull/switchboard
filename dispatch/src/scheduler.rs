//! One step of one ticket. The runner reads what Switchboard says about
//! the ticket's sessions and runs, decides, and acts through the port,
//! writing the ticket before and after every request. Nothing here
//! retries on its own: a failure is a decision.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow, bail};
use dispatch_control as wire_dispatch;
use switchboard_control::{self as wire, Body, Reply, Request, RunState};

use crate::bitbucket::{Bitbucket, bitbucket_repo};
use crate::git::{Repo, branch_name};
use crate::github::{Checks, Gh, PullRequests, github_repo};
use crate::pipeline::{Context, Gate, Pipeline, Stage, StageKind};
use crate::port::Port;
use crate::store::{DataDir, Lock, Settings, expand_home, read_json, shell_unsafe, write_json};
use crate::template::Vars;
use crate::ticket::{
    Attempt, AttemptKind, AttemptState, Decision, DecisionKind, DecisionState, GateRun, LaneRecord,
    Operation, ProjectState, PullRequestRecord, SETTLE_POLLS, Settle, SourceSnapshot, Ticket,
    TicketState,
};

/// How often a `pr-checks` gate reads the provider.
pub const PR_POLL_MS: u64 = 60_000;
/// How long lookups may keep failing before the gate asks.
pub const PR_ERROR_GRACE_MS: u64 = 3_600_000;

/// Everything the runner acts through.
pub struct Runner {
    pub data: DataDir,
    pub port: Box<dyn Port>,
    pub git: Box<dyn Repo>,
    /// Pull requests on GitHub, for the `pr-checks` and `pr-merged`
    /// gates; `gh` unless a test swaps in a fake.
    pub prs: Box<dyn PullRequests>,
    /// The same on Bitbucket Cloud, through its API.
    pub bitbucket: Box<dyn PullRequests>,
    /// The writer lock while a transaction runs; saves inside it write
    /// straight through, saves outside it take the lock for the write.
    held: Option<Lock>,
}

impl Runner {
    #[must_use]
    pub fn new(data: DataDir, port: Box<dyn Port>, git: Box<dyn Repo>) -> Self {
        // The field takes ownership of `data`, so the path it needs is
        // taken first.
        let env_file = data.root.join("env");
        Self {
            data,
            port,
            git,
            prs: Box::new(Gh),
            bitbucket: Box::new(Bitbucket::new(env_file)),
            held: None,
        }
    }

    /// Run `f` as one read-modify-write under the writer lock: nothing
    /// from the terminal lands between the reads and the writes inside.
    /// Nested calls share the outer lock.
    pub fn transaction<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        if self.held.is_some() {
            return f(self);
        }
        self.held = Some(self.data.lock()?);
        let result = f(self);
        self.held = None;
        result
    }

    fn write_record<T: serde::Serialize>(&self, path: &Path, value: &T) -> Result<()> {
        if self.held.is_some() {
            write_json(path, value)
        } else {
            self.data.with_lock(|| write_json(path, value))
        }
    }
}

/// What `dispatch decide` writes as the answerer.
pub const BY_HAND: &str = "you";
/// Who answered a decision Dispatch resolved from a provider.
pub const BY_DISPATCH: &str = "dispatch";

/// What a decision asks, before it is a record.
pub struct Ask<'a> {
    pub stage: &'a str,
    pub name: &'a str,
    pub kind: DecisionKind,
    pub question: String,
    pub options: &'a [&'a str],
    pub recommendation: Option<String>,
    pub attempt: Option<(String, u32)>,
}

/// An answered decision not yet acted on: its index, name, answer and
/// attempt.
type Answered = (usize, String, String, Option<(String, u32)>);

impl Runner {
    // --- records

    pub fn load_ticket(&self, id: &str) -> Result<Ticket> {
        read_json(&self.data.ticket_file(id))
    }

    pub fn save_ticket(&self, t: &mut Ticket, now_ms: u64) -> Result<()> {
        t.updated_ms = now_ms;
        self.write_record(&self.data.ticket_file(&t.id), t)
    }

    pub fn load_project(&self, name: &str) -> Result<ProjectState> {
        let path = self.data.project_file(name);
        if crate::store::record_exists(&path) {
            read_json(&path)
        } else {
            Ok(ProjectState {
                name: name.to_owned(),
                ..ProjectState::default()
            })
        }
    }

    pub fn save_project(&self, ps: &ProjectState) -> Result<()> {
        self.write_record(&self.data.project_file(&ps.name), ps)
    }

    /// Reorder a project's queue from the terminal: every id named comes
    /// first in that order, the rest keep theirs. One transaction.
    pub fn reorder_queue(&mut self, project: &str, order: &[&str]) -> Result<ProjectState> {
        self.transaction(|r| {
            let mut ps = r.load_project(project)?;
            let mut next: Vec<String> = Vec::new();
            for id in order {
                if !ps.queue.iter().any(|q| q == id) {
                    bail!("{id} is not in {project}'s queue");
                }
                next.push((*id).to_owned());
            }
            for id in &ps.queue {
                if !next.contains(id) {
                    next.push(id.clone());
                }
            }
            ps.queue = next;
            r.save_project(&ps)?;
            Ok(ps)
        })
    }

    /// The pipeline a ticket runs under: its own copy.
    pub fn pipeline_of(&self, t: &Ticket) -> Result<Pipeline> {
        let text = std::fs::read_to_string(&t.pipeline_file)
            .with_context(|| format!("read {}", t.pipeline_file.display()))?;
        Pipeline::parse(&text)
    }

    /// Every ticket on disk.
    pub fn tickets(&self) -> Result<Vec<Ticket>> {
        self.data
            .ticket_files()?
            .iter()
            .map(|p| read_json(p))
            .collect()
    }

    // --- take

    /// Make the ticket: its record, its copy of the pipeline, its place
    /// at the end of the queue. Nothing is asked of Switchboard yet.
    pub fn take(
        &mut self,
        project: &str,
        pipeline_text: &str,
        source: SourceSnapshot,
        now_ms: u64,
    ) -> Result<Ticket> {
        let pipeline = Pipeline::parse(pipeline_text)?;
        if pipeline.project.name != project {
            bail!(
                "the pipeline file names project {:?}, not {project:?}",
                pipeline.project.name
            );
        }
        self.transaction(|r| r.take_locked(project, pipeline_text, source, now_ms))
    }

    fn take_locked(
        &mut self,
        project: &str,
        pipeline_text: &str,
        source: SourceSnapshot,
        now_ms: u64,
    ) -> Result<Ticket> {
        for existing in self.tickets()? {
            if existing.source.identity == source.identity
                && !matches!(existing.state, TicketState::Closed { .. })
            {
                bail!(
                    "{} is already ticket {} ({})",
                    source.identity,
                    existing.id,
                    match &existing.state {
                        TicketState::Active => "active".to_owned(),
                        TicketState::Parking { reason } | TicketState::Parked { reason } => {
                            format!("parked: {reason}")
                        }
                        TicketState::Closed { .. } => unreachable!(),
                    }
                );
            }
        }
        // The tree's path reaches the repository's own tooling; one it
        // may not survive is refused before anything is made.
        let pipeline = Pipeline::parse(pipeline_text)?;
        if pipeline.cuts_worktrees()
            && let Some(why) = shell_unsafe(&self.worktree_root(&pipeline))
        {
            bail!("worktrees: {why}; set another with `dispatch worktrees <path>`");
        }
        let id = Ticket::new_id();
        let dir = self.data.ticket_dir(&id);
        std::fs::create_dir_all(&dir)?;
        let pipeline_file = dir.join("pipeline.toml");
        crate::store::atomic_write(&pipeline_file, pipeline_text.as_bytes())?;
        let mut ticket = Ticket {
            id: id.clone(),
            project: project.to_owned(),
            source,
            pipeline_fingerprint: Pipeline::fingerprint(pipeline_text),
            pipeline_file,
            lanes: Vec::new(),
            tree: None,
            stage: 0,
            attempts: Vec::new(),
            decisions: Vec::new(),
            ledger: Vec::new(),
            processes: Vec::new(),
            root_project: None,
            rework: BTreeMap::new(),
            state: TicketState::Active,
            created_ms: now_ms,
            updated_ms: now_ms,
        };
        self.save_ticket(&mut ticket, now_ms)?;
        let mut ps = self.load_project(project)?;
        ps.queue.push(id);
        self.save_project(&ps)?;
        Ok(ticket)
    }

    /// Where a pipeline's tickets' trees go: its own `worktrees`, else
    /// the data directory's setting or default.
    #[must_use]
    pub fn worktree_root(&self, p: &Pipeline) -> PathBuf {
        p.project
            .worktrees
            .clone()
            .unwrap_or_else(|| self.data.worktrees_dir())
    }

    /// Set where tickets' trees go (`path`; `None` leaves it), and with
    /// `migrate` move every ticket's tree that is not under the root
    /// there: git moves the ticket's tree, each lane clone is re-pointed
    /// at its tree inside it, the records and the Switchboard projects
    /// follow. A ticket with something running, or whose pipeline names
    /// its own `worktrees`, is left where it is.
    pub fn set_worktrees(
        &mut self,
        path: Option<PathBuf>,
        migrate: bool,
        now_ms: u64,
    ) -> Result<wire_dispatch::WorktreesView> {
        if let Some(path) = path {
            let path = expand_home(&path);
            if let Some(why) = shell_unsafe(&path) {
                bail!("{why}");
            }
            if !path.is_absolute() {
                bail!("{} is not an absolute path", path.display());
            }
            self.data.write_settings(&Settings {
                worktrees: Some(path),
            })?;
        }
        let root = self.data.worktrees_dir();
        let mut view = wire_dispatch::WorktreesView {
            root: root.clone(),
            ..Default::default()
        };
        if !migrate {
            return Ok(view);
        }
        self.transaction(|r| {
            for mut t in r.tickets()? {
                let Some(tree) = t.tree.clone() else {
                    continue;
                };
                if matches!(t.state, TicketState::Closed { .. }) || tree.parent() == Some(&*root) {
                    continue;
                }
                let p = match r.pipeline_of(&t) {
                    Ok(p) => p,
                    Err(e) => {
                        view.skipped.push((t.id.clone(), format!("pipeline: {e}")));
                        continue;
                    }
                };
                if p.project.worktrees.is_some() {
                    continue;
                }
                if t.attempts.iter().any(Attempt::is_open) {
                    view.skipped
                        .push((t.id.clone(), "something is running in it".into()));
                    continue;
                }
                let mut ps = r.load_project(&t.project)?;
                match r.move_tree(&mut t, &mut ps, &p, &root, now_ms) {
                    Ok(()) => view.moved.push(t.id.clone()),
                    Err(e) => view.skipped.push((t.id.clone(), format!("{e:#}"))),
                }
                r.save_project(&ps)?;
            }
            Ok(())
        })?;
        Ok(view)
    }

    fn move_tree(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        root: &Path,
        now_ms: u64,
    ) -> Result<()> {
        let from = t.tree.clone().expect("checked by the caller");
        let to = root.join(&t.id);
        let clone = self.data.repo_dir(&p.project.name);
        self.git.worktree_move(&clone, &from, &to)?;
        t.tree = Some(to.clone());
        for lane in &mut t.lanes {
            let Ok(rest) = lane.worktree.strip_prefix(&from) else {
                continue;
            };
            lane.worktree = to.join(rest);
            let has_repo = p.lane(&lane.name).is_some_and(|l| l.repo.is_some());
            if has_repo {
                let lane_clone = self
                    .data
                    .repo_dir(&format!("{}@{}", p.project.name, lane.name));
                self.git.worktree_repair(&lane_clone, &lane.worktree)?;
            }
        }
        self.save_ticket(t, now_ms)?;
        log::info!("ticket {} tree moved to {}", t.id, to.display());
        // The Switchboard projects point at the trees.
        let mut roots: Vec<(String, PathBuf)> = Vec::new();
        if let Some(project) = &t.root_project
            && let Some(tree) = &t.tree
        {
            roots.push((project.clone(), tree.clone()));
        }
        for lane in &t.lanes {
            if let Some(project) = &lane.project
                && !roots.iter().any(|(p, _)| p == project)
            {
                roots.push((project.clone(), lane.worktree.clone()));
            }
        }
        for (project, root) in roots {
            self.send(
                t,
                ps,
                None,
                "root",
                Body::ProjectRoot { project, root },
                now_ms,
            )?;
        }
        Ok(())
    }

    // --- the ledger

    /// One command to Switchboard, written down before and after. The
    /// reply's records are applied to the ticket by `intent`.
    fn send(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        attempt: Option<(String, u32)>,
        intent: &str,
        body: Body,
        now_ms: u64,
    ) -> Result<Reply> {
        let op = format!(
            "{}-{}",
            t.id,
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        let class = match body.class() {
            wire::Class::Creation => "creation",
            wire::Class::Idempotent => "idempotent",
            wire::Class::NonReplayable => "non-replayable",
            wire::Class::Query => "query",
        };
        t.ledger.push(Operation {
            op: op.clone(),
            kind: body.kind(),
            class: class.into(),
            attempt,
            intent: intent.into(),
            sent_ms: now_ms,
            body: Some(body.clone()),
            reply: None,
            error: None,
        });
        self.save_ticket(t, now_ms)?;
        let result = self.port.call(&Request::new(op.clone(), body));
        let entry = t
            .ledger
            .iter_mut()
            .rev()
            .find(|o| o.op == op)
            .expect("just pushed");
        match &result {
            Ok(reply) => entry.reply = Some(reply.clone()),
            Err(e) => entry.error = Some(e.to_string()),
        }
        // The outcome is written down either way: a recorded failure is
        // what recovery reads on the next start.
        if let Ok(reply) = &result {
            apply_reply(t, ps, intent, reply);
        }
        self.save_ticket(t, now_ms)?;
        result.with_context(|| format!("{intent}: the control socket failed"))
    }

    /// A query: never in the ledger, since it changes nothing.
    fn ask(&mut self, body: Body) -> Result<Reply> {
        self.port
            .call(&Request::new(
                format!("q-{}", uuid::Uuid::new_v4().simple()),
                body,
            ))
            .context("the control socket failed")
    }

    // --- one step

    /// Advance one ticket as far as this poll allows.
    pub fn step(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        now_ms: u64,
    ) -> Result<()> {
        if !t.active() {
            return Ok(());
        }
        // A request whose reply never came (the socket failed, or a
        // restart) is resolved before anything else is asked.
        let pending: Vec<usize> = t
            .ledger
            .iter()
            .enumerate()
            .filter(|(_, o)| o.reply.is_none())
            .map(|(i, _)| i)
            .collect();
        let recovered = !pending.is_empty();
        for i in pending {
            self.recover_one(t, ps, i, now_ms)?;
        }
        // A pass that changes nothing else must still keep the reply.
        if recovered {
            self.save_ticket(t, now_ms)?;
        }
        self.fail_stranded(t, ps, now_ms)?;
        self.act_on_answers(t, ps, p, now_ms)?;
        if !t.active() {
            return Ok(());
        }
        // The ticket's trees come before any stage: a worktree of
        // Dispatch's clone on the ticket's branch, cut from what the
        // remote has now, and every lane inside it. The user's checkout
        // is never involved.
        if p.cuts_worktrees() && (t.tree.is_none() || t.lanes.len() < p.lanes.len()) {
            self.cut_trees(t, ps, p, now_ms)?;
            if !t.active() {
                return Ok(());
            }
        }
        let Some(stage) = p.stages.get(t.stage).cloned() else {
            self.close(t, ps, "every stage is done", now_ms)?;
            return Ok(());
        };
        // A pending decision for this stage holds it, unless attempts
        // of it are still running and only need watching.
        let held = t
            .decisions
            .iter()
            .any(|d| d.pending() && d.stage == stage.name);
        match stage.kind() {
            StageKind::GateOnly => {
                // The merge decision is pending by design while the
                // provider is watched, so that gate polls through it.
                let watches = matches!(
                    &stage.gate,
                    Some(Gate::External { check, .. }) if check == "pr-merged"
                );
                if !held || watches {
                    self.gate_only(t, ps, p, &stage, now_ms)?;
                }
            }
            StageKind::Agent => self.agent_stage(t, ps, p, &stage, held, now_ms)?,
            StageKind::Workflow => self.workflow_stage(t, ps, p, &stage, held, now_ms)?,
        }
        Ok(())
    }

    fn close(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        reason: &str,
        now_ms: u64,
    ) -> Result<()> {
        t.state = TicketState::Closed {
            reason: reason.into(),
        };
        ps.queue.retain(|id| id != &t.id);
        self.save_ticket(t, now_ms)?;
        self.save_project(ps)
    }

    /// Parking is a sequence, not a flag: the intent is written first,
    /// open attempts are cancelled (a review run paused so its tick
    /// cannot start a round), every process on the ticket's list is
    /// killed, and the ticket reads as parked only once Switchboard
    /// reports them all gone. `finish_parking` runs the rest on later
    /// passes if anything is still alive now.
    fn park(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        reason: &str,
        now_ms: u64,
    ) -> Result<()> {
        log::warn!("ticket {} parking: {reason}", t.id);
        t.state = TicketState::Parking {
            reason: reason.into(),
        };
        self.save_ticket(t, now_ms)?;
        self.finish_parking(t, ps, now_ms)
    }

    /// The rest of the sequence, from the saved intent: every open
    /// attempt cancelled (its run paused and read back as paused),
    /// everything on the process list killed and read back as gone, and
    /// only then `Parked`. Run again on every pass until it gets there,
    /// so a restart at any point resumes it whole.
    pub(crate) fn finish_parking(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        now_ms: u64,
    ) -> Result<()> {
        let TicketState::Parking { reason } = t.state.clone() else {
            return Ok(());
        };
        let open: Vec<Attempt> = t.attempts.iter().filter(|a| a.is_open()).cloned().collect();
        let mut settled = true;
        for a in open {
            settled &= self.cancel_attempt(t, ps, &a, &reason, now_ms)?;
        }
        settled &= self.retire_processes(t, ps, &t.processes.clone(), now_ms)?;
        if settled {
            log::warn!("ticket {} parked: {reason}", t.id);
            t.state = TicketState::Parked { reason };
            self.save_ticket(t, now_ms)?;
        }
        Ok(())
    }

    /// An attempt Dispatch stops on purpose: its run paused and confirmed
    /// paused, its processes killed, and only then its state written as
    /// cancelled, so a record never says cancelled about something still
    /// going. True once it is. No decision follows.
    fn cancel_attempt(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        a: &Attempt,
        reason: &str,
        now_ms: u64,
    ) -> Result<bool> {
        if let Some(run) = a.run.clone() {
            self.send(
                t,
                ps,
                Some((a.stage.clone(), a.n)),
                "pause",
                Body::WorkflowPause { run: run.clone() },
                now_ms,
            )?;
            let paused = match self.ask(Body::Workflow { run })? {
                Reply::Workflow { run } => matches!(
                    run.state,
                    RunState::Paused { .. } | RunState::Finalized | RunState::HandedOff
                ),
                // Gone is stopped too.
                Reply::Failed { .. } => true,
                other => bail!("workflow query answered {other:?}"),
            };
            if !paused {
                return Ok(false);
            }
        }
        let mine = self.processes_of(t, a)?;
        if !self.retire_processes(t, ps, &mine, now_ms)? {
            return Ok(false);
        }
        if let Some(attempt) = t
            .attempts
            .iter_mut()
            .find(|x| x.stage == a.stage && x.n == a.n)
        {
            attempt.state = AttemptState::Cancelled {
                reason: reason.into(),
            };
            attempt.ended_ms = Some(now_ms);
        }
        self.save_ticket(t, now_ms)?;
        Ok(true)
    }

    /// The sessions an attempt owns: its own, or its run's reviewer and
    /// planner clone.
    fn processes_of(&mut self, t: &Ticket, a: &Attempt) -> Result<Vec<String>> {
        let mut ids: Vec<String> = a.session.iter().cloned().collect();
        if let Some(run) = a.run.clone()
            && let Reply::Workflow { run } = self.ask(Body::Workflow { run })?
        {
            ids.push(run.reviewer);
            ids.extend(run.planner);
        }
        let _ = t;
        Ok(ids)
    }

    /// Kill every session named that still runs, then read each back.
    /// True when none is running any more.
    fn retire_processes(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        ids: &[String],
        now_ms: u64,
    ) -> Result<bool> {
        let mut all_gone = true;
        for id in ids {
            let running = matches!(
                self.ask(Body::Session { session: id.clone() })?,
                Reply::Session { session } if session.liveness == wire::Liveness::Running
            );
            if !running {
                continue;
            }
            self.send(
                t,
                ps,
                None,
                "kill",
                Body::SessionKill {
                    session: id.clone(),
                },
                now_ms,
            )?;
            if matches!(
                self.ask(Body::Session { session: id.clone() })?,
                Reply::Session { session } if session.liveness == wire::Liveness::Running
            ) {
                all_gone = false;
            }
        }
        Ok(all_gone)
    }

    /// An open attempt with nothing to watch and no launch request on
    /// the books: Dispatch stopped between writing the attempt and
    /// writing the request, so nothing was ever asked of Switchboard.
    fn fail_stranded(&mut self, t: &mut Ticket, ps: &mut ProjectState, now_ms: u64) -> Result<()> {
        let stranded: Vec<(String, u32)> = t
            .attempts
            .iter()
            // A gate-only attempt launches nothing, so it has nothing
            // to be stranded without.
            .filter(|a| a.is_open() && a.kind != AttemptKind::GateOnly)
            .filter(|a| a.session.is_none() && a.run.is_none())
            .filter(|a| {
                !t.ledger.iter().any(|o| {
                    o.reply.is_none()
                        && matches!(o.intent.as_str(), "session" | "run")
                        && o.attempt.as_ref() == Some(&(a.stage.clone(), a.n))
                })
            })
            .map(|a| (a.stage.clone(), a.n))
            .collect();
        for (stage, n) in stranded {
            self.fail_attempt(t, ps, &stage, n, "its launch was never recorded", now_ms)?;
        }
        Ok(())
    }

    // --- decisions

    /// A pending decision, made once per stage, name and attempt.
    pub(crate) fn ensure_decision(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        ask: Ask<'_>,
        now_ms: u64,
    ) -> Result<()> {
        let Ask {
            stage,
            name,
            kind,
            question,
            options,
            recommendation,
            attempt,
        } = ask;
        if t.decisions
            .iter()
            .any(|d| d.pending() && d.stage == stage && d.name == name && d.attempt == attempt)
        {
            return Ok(());
        }
        let id = format!("d{}", t.decisions.len() + 1);
        t.decisions.push(Decision {
            id: id.clone(),
            stage: stage.into(),
            name: name.into(),
            kind,
            question: question.clone(),
            options: options.iter().map(|o| (*o).to_owned()).collect(),
            recommendation,
            attempt,
            state: DecisionState::Pending,
            made_ms: now_ms,
        });
        log::info!("ticket {} decision {id} ({name}): {question}", t.id);
        self.save_ticket(t, now_ms)?;
        // The ticket's current session shows it, so the badge and the
        // rail count it.
        if let Some(session) = t.current_session().cloned() {
            let text = format!(
                "Dispatch ticket {} needs a decision ({id}): {question}\nAnswer with: dispatch decide {} {id} <{}>",
                t.id,
                t.id,
                options.join("|")
            );
            self.send(
                t,
                ps,
                None,
                "notes",
                Body::SessionNotes {
                    session: session.clone(),
                    text,
                },
                now_ms,
            )?;
            self.send(
                t,
                ps,
                None,
                "waiting",
                Body::SessionWaiting {
                    session,
                    on: true,
                    reason: format!("decision {id}: {name}"),
                },
                now_ms,
            )?;
        }
        Ok(())
    }

    /// The same attempt, its checks again: the agent's work stands and
    /// the gate starts over on the next poll, no agent launched.
    fn check_again(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        attempt: Option<&(String, u32)>,
        now_ms: u64,
    ) -> Result<()> {
        if let Some(a) =
            attempt.and_then(|(s, n)| t.attempts.iter_mut().find(|a| &a.stage == s && a.n == *n))
        {
            a.state = AttemptState::Running;
            a.gate = None;
            a.ended_ms = None;
            log::info!("ticket {} {}/{} checks again", t.id, a.stage, a.context);
        }
        self.save_ticket(t, now_ms)?;
        self.unmark(t, ps, now_ms)
    }

    /// Clear the waiting mark a decision put on a session.
    fn unmark(&mut self, t: &mut Ticket, ps: &mut ProjectState, now_ms: u64) -> Result<()> {
        if t.pending_decisions().is_empty()
            && let Some(session) = t.current_session().cloned()
        {
            self.send(
                t,
                ps,
                None,
                "waiting",
                Body::SessionWaiting {
                    session,
                    on: false,
                    reason: String::new(),
                },
                now_ms,
            )?;
        }
        Ok(())
    }

    /// Every answer not yet acted on.
    fn act_on_answers(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        now_ms: u64,
    ) -> Result<()> {
        let answered: Vec<Answered> = t
            .decisions
            .iter()
            .enumerate()
            .filter_map(|(i, d)| {
                d.unacted_answer()
                    .map(|a| (i, d.name.clone(), a.to_owned(), d.attempt.clone()))
            })
            .collect();
        for (i, name, answer, attempt) in answered {
            // The acted mark is set in memory here and reaches disk with
            // the action's own first write (the parking state, the ledger
            // entry, the lane record), never before it: an answer is
            // either still unacted or its intent is durable. A rerun is
            // marked only once the replaced attempt is confirmed gone,
            // below, since `may_rerun` launches on that mark.
            if name != "rerun"
                && let DecisionState::Answered { acted, .. } = &mut t.decisions[i].state
            {
                *acted = true;
            }
            match (name.as_str(), answer.as_str()) {
                ("lanes", lanes) => {
                    let names: Vec<String> = lanes
                        .split(',')
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty())
                        .collect();
                    self.choose_lanes(t, ps, p, &names, now_ms)?;
                }
                ("finalize", "finalize") => {
                    if let Some(run) = attempt
                        .as_ref()
                        .and_then(|(s, n)| t.attempts.iter().find(|a| &a.stage == s && a.n == *n))
                        .and_then(|a| a.run.clone())
                    {
                        self.send(
                            t,
                            ps,
                            attempt.clone(),
                            "finalize",
                            Body::WorkflowFinalize { run },
                            now_ms,
                        )?;
                    }
                    self.unmark(t, ps, now_ms)?;
                }
                ("paused", "continue") => {
                    if let Some(run) = attempt
                        .as_ref()
                        .and_then(|(s, n)| t.attempts.iter().find(|a| &a.stage == s && a.n == *n))
                        .and_then(|a| a.run.clone())
                    {
                        self.send(
                            t,
                            ps,
                            attempt.clone(),
                            "continue",
                            Body::WorkflowContinue { run },
                            now_ms,
                        )?;
                    }
                    self.unmark(t, ps, now_ms)?;
                }
                ("rerun", "check") => {
                    if let DecisionState::Answered { acted, .. } = &mut t.decisions[i].state {
                        *acted = true;
                    }
                    self.check_again(t, ps, attempt.as_ref(), now_ms)?;
                }
                ("pr", "recheck") => recheck_pr(t, attempt.as_ref()),
                (_, "proceed" | "done") => {
                    self.pass_human_gate(t, ps, p, attempt.as_ref(), now_ms)?;
                }
                (name, "rerun") if name != "rerun" => {
                    let note = match &t.decisions[i].state {
                        DecisionState::Answered { note, .. } => note.clone(),
                        _ => None,
                    };
                    self.send_back(t, ps, p, name, attempt.as_ref(), note, now_ms)?;
                }
                ("rerun", "rerun") => {
                    if !self.retire_replaced(t, ps, i, attempt.as_ref(), now_ms)? {
                        continue;
                    }
                }
                (_, "park") => {
                    self.park(t, ps, &format!("parked by hand at decision {name}"), now_ms)?;
                }
                (name, other) => {
                    self.park(
                        t,
                        ps,
                        &format!("decision {name}: answer {other:?} is not one Dispatch knows"),
                        now_ms,
                    )?;
                }
            }
            // Arms that wrote nothing still need the mark on disk.
            self.save_ticket(t, now_ms)?;
            if !t.active() {
                break;
            }
        }
        Ok(())
    }

    /// A rerun's replaced attempt is retired first, so an old and a new
    /// attempt never run together; still alive, the answer stays
    /// unacted for the next pass (false). Gone, the answer is acted.
    fn retire_replaced(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        decision: usize,
        attempt: Option<&(String, u32)>,
        now_ms: u64,
    ) -> Result<bool> {
        if let Some(a) = attempt
            .and_then(|(s, n)| t.attempts.iter().find(|a| &a.stage == s && a.n == *n))
            .cloned()
        {
            let mine = self.processes_of(t, &a)?;
            if !self.retire_processes(t, ps, &mine, now_ms)? {
                return Ok(false);
            }
        }
        if let DecisionState::Answered { acted, .. } = &mut t.decisions[decision].state {
            *acted = true;
        }
        self.save_ticket(t, now_ms)?;
        self.unmark(t, ps, now_ms)?;
        Ok(true)
    }

    // --- lanes

    /// Cut the ticket's tree, then every lane inside it: the tree is a
    /// worktree of Dispatch's clone of the project repository; a lane
    /// with a repository of its own is a worktree of Dispatch's clone of
    /// that, at the lane's path in the tree; any other lane is a path in
    /// the tree. Setups wait for the first agent in the lane.
    fn cut_trees(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        now_ms: u64,
    ) -> Result<()> {
        let Some(url) = p.project.repo.clone() else {
            return Ok(());
        };
        let branch = branch_name(t.source.number.unwrap_or(0), &t.source.title);
        if t.tree.is_none() {
            let dir = self.worktree_root(p).join(&t.id);
            let clone = self.data.repo_dir(&p.project.name);
            let start = format!("{}/{}", p.project.remote, p.project.base);
            if !self.cut(
                t,
                ps,
                "the tree",
                &url,
                &clone,
                &p.project.remote,
                &dir,
                &branch,
                &start,
                now_ms,
            )? {
                return Ok(());
            }
            t.tree = Some(dir);
            self.save_ticket(t, now_ms)?;
        }
        let tree = t.tree.clone().expect("cut above");
        for lane in &p.lanes {
            if t.lanes.iter().any(|l| l.name == lane.name) {
                continue;
            }
            let dir = tree.join(&lane.path);
            if let Some(url) = &lane.repo {
                let clone = self
                    .data
                    .repo_dir(&format!("{}@{}", p.project.name, lane.name));
                let remote = p.lane_remote(lane).to_owned();
                let start = format!("{remote}/{}", p.lane_base(lane));
                let what = format!("lane {}", lane.name);
                if !self.cut(
                    t, ps, &what, url, &clone, &remote, &dir, &branch, &start, now_ms,
                )? {
                    return Ok(());
                }
            } else if !dir.is_dir() {
                return self.park(
                    t,
                    ps,
                    &format!("lane {}: {} is not in the tree", lane.name, dir.display()),
                    now_ms,
                );
            }
            t.lanes.push(LaneRecord {
                name: lane.name.clone(),
                worktree: dir,
                branch: branch.clone(),
                project: None,
                chosen: p.lanes.len() == 1,
                setup_done: false,
            });
            self.save_ticket(t, now_ms)?;
        }
        Ok(())
    }

    /// One worktree: the clone made or fetched, the worktree cut from
    /// `start` on `branch` at `dir`, or adopted when git says it is
    /// already that. False when the ticket was parked instead.
    #[allow(clippy::too_many_arguments)]
    fn cut(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        what: &str,
        url: &str,
        clone: &Path,
        remote: &str,
        dir: &Path,
        branch: &str,
        start: &str,
        now_ms: u64,
    ) -> Result<bool> {
        if let Err(e) = self.git.ensure_clone(url, clone) {
            self.park(
                t,
                ps,
                &format!("{what}: could not clone {url}: {e:#}"),
                now_ms,
            )?;
            return Ok(false);
        }
        if let Err(e) = self.git.fetch(clone, remote) {
            self.park(
                t,
                ps,
                &format!(
                    "{what}: could not fetch {remote} in {}: {e:#}",
                    clone.display()
                ),
                now_ms,
            )?;
            return Ok(false);
        }
        // A directory already there is adopted only if git says it is
        // this repository's worktree on this branch (cut before a stop
        // that came ahead of the record); anything else in the way is
        // not guessed at.
        if dir.exists() {
            if !self.git.is_worktree_of(clone, dir, branch)? {
                self.park(
                    t,
                    ps,
                    &format!(
                        "{what}: {} exists but is not a worktree of {} on {branch}",
                        dir.display(),
                        clone.display()
                    ),
                    now_ms,
                )?;
                return Ok(false);
            }
            log::info!("ticket {} {what}: adopting {}", t.id, dir.display());
            return Ok(true);
        }
        if let Err(e) = self.git.worktree_add(clone, dir, branch, start) {
            self.park(
                t,
                ps,
                &format!("{what}: could not cut {}: {e:#}", dir.display()),
                now_ms,
            )?;
            return Ok(false);
        }
        Ok(true)
    }

    /// The lanes decision's answer: those lanes are the ones the stages
    /// run in.
    fn choose_lanes(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        names: &[String],
        now_ms: u64,
    ) -> Result<()> {
        for name in names {
            if p.lane(name).is_none() {
                return self.park(
                    t,
                    ps,
                    &format!("lanes decision named an unknown lane {name:?}"),
                    now_ms,
                );
            }
        }
        for l in &mut t.lanes {
            l.chosen = names.contains(&l.name);
        }
        self.save_ticket(t, now_ms)
    }

    /// A lane's setup, once, before its first agent; the ticket is
    /// parked if it fails.
    fn ensure_setup(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        cwd: &Path,
        now_ms: u64,
    ) -> Result<bool> {
        let pending: Vec<(usize, Vec<String>, String)> = t
            .lanes
            .iter()
            .enumerate()
            .filter(|(_, l)| l.worktree == cwd && !l.setup_done)
            .filter_map(|(i, l)| {
                p.lane(&l.name)
                    .filter(|lane| !lane.setup.is_empty())
                    .map(|lane| (i, lane.setup.clone(), l.branch.clone()))
            })
            .collect();
        for (i, setup, branch) in pending {
            let name = t.lanes[i].name.clone();
            if let Err(e) = self
                .git
                .run(cwd, &setup, &env_for(t, Some(&name), Some(&branch)))
            {
                self.park(t, ps, &format!("lane {name}: setup failed: {e:#}"), now_ms)?;
                return Ok(false);
            }
            t.lanes[i].setup_done = true;
            self.save_ticket(t, now_ms)?;
        }
        Ok(true)
    }

    // --- gate-only stages

    fn gate_only(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        now_ms: u64,
    ) -> Result<()> {
        match &stage.gate {
            Some(Gate::Human { decision, .. }) if decision == "lanes" => {
                // Every lane's tree was cut before the first stage; this
                // gate chooses which lanes the issue's work runs in, so a
                // one-lane pipeline passes without a question and label
                // hints answer it when the dial says auto.
                let answered = t.decisions.iter().any(|d| {
                    d.name == "lanes"
                        && d.stage == stage.name
                        && matches!(d.state, DecisionState::Answered { .. })
                });
                if p.lanes.len() == 1 || !p.cuts_worktrees() || answered {
                    self.finish_gate_only(t, ps, stage, now_ms)?;
                    return Ok(());
                }
                let hinted = lane_hints(p, &t.source.labels);
                if p.dial("lanes") == "auto" && !hinted.is_empty() {
                    self.choose_lanes(t, ps, p, &hinted, now_ms)?;
                    if t.active() {
                        self.finish_gate_only(t, ps, stage, now_ms)?;
                    }
                    return Ok(());
                }
                let all: Vec<&str> = p.lanes.iter().map(|l| l.name.as_str()).collect();
                let notes = t.input("notes").map(|n| n.display().to_string());
                let question = format!(
                    "Which lanes does #{} need? Lanes: {}.{}",
                    t.source.number.unwrap_or(0),
                    all.join(", "),
                    notes.map_or(String::new(), |n| format!(" The investigator's notes: {n}"))
                );
                let options: Vec<&str> = all.clone();
                self.ensure_decision(
                    t,
                    ps,
                    Ask {
                        stage: &stage.name,
                        name: "lanes",
                        kind: DecisionKind::Permission,
                        question,
                        options: &options,
                        recommendation: (!hinted.is_empty()).then(|| hinted.join(",")),
                        attempt: None,
                    },
                    now_ms,
                )
            }
            Some(Gate::External { check, .. }) if check == "pr-checks" => {
                self.pr_checks_stage(t, ps, p, stage, now_ms)
            }
            Some(Gate::External {
                check, decision, ..
            }) if check == "pr-merged" => {
                let decision = decision.clone().unwrap_or_else(|| "merge".to_owned());
                self.pr_merged_stage(t, ps, p, stage, &decision, now_ms)
            }
            Some(Gate::Human { decision, confirm }) => {
                self.human_gate(t, ps, p, stage, decision, *confirm, now_ms)
            }
            Some(gate) => {
                let kind = match gate {
                    Gate::Command { .. } => "command gate",
                    Gate::External { check, .. } => check.as_str(),
                    Gate::Human { decision, .. } => decision.as_str(),
                };
                self.park(
                    t,
                    ps,
                    &format!("stage {} ({kind}) is not built in this slice", stage.name),
                    now_ms,
                )
            }
            None => self.park(t, ps, &format!("stage {} has no gate", stage.name), now_ms),
        }
    }

    /// A `pr-checks` stage: one gate-only attempt per context that
    /// finds the lane's pull request and waits for its checks to be
    /// green at the head the tree is at. Anything but green or pending
    /// is a question, never a failure: the work is fine, the world
    /// around it is what needs a look.
    fn pr_checks_stage(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        now_ms: u64,
    ) -> Result<()> {
        let contexts = Self::contexts(t, p, stage);
        if contexts.is_empty() {
            return self.park(
                t,
                ps,
                &format!("stage {} needs lanes the ticket has not cut", stage.name),
                now_ms,
            );
        }
        let mut all_complete = true;
        for (ctx, cwd, lane) in contexts {
            let last = t
                .attempts
                .iter()
                .filter(|a| a.stage == stage.name && a.context == ctx)
                .max_by_key(|a| a.n)
                .cloned();
            let attempt = match last {
                Some(a) if a.state == AttemptState::Complete => continue,
                Some(a) if a.is_open() => a,
                Some(_) => {
                    all_complete = false;
                    continue;
                }
                None => {
                    let a = new_attempt(
                        &stage.name,
                        next_n(t, &stage.name),
                        &ctx,
                        AttemptKind::GateOnly,
                        AttemptState::Running,
                        BTreeMap::new(),
                        now_ms,
                    );
                    t.attempts.push(a.clone());
                    self.save_ticket(t, now_ms)?;
                    a
                }
            };
            all_complete = false;
            self.poll_pr_checks(t, ps, p, stage, &attempt, &cwd, lane.as_deref(), now_ms)?;
            if !t.active() {
                return Ok(());
            }
        }
        if all_complete {
            self.advance(t, ps, now_ms)?;
        }
        Ok(())
    }

    /// One reading of the PR for an open `pr-checks` attempt, at most
    /// once per `PR_POLL_MS` unless a recheck answer cleared the clock.
    #[allow(clippy::too_many_arguments)]
    fn poll_pr_checks(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        a: &Attempt,
        cwd: &Path,
        lane: Option<&str>,
        now_ms: u64,
    ) -> Result<()> {
        if a.pr
            .as_ref()
            .is_some_and(|pr| pr.checked_ms != 0 && now_ms < pr.checked_ms + PR_POLL_MS)
        {
            return Ok(());
        }
        let Some(Gate::External {
            provider, checks, ..
        }) = &stage.gate
        else {
            return Ok(());
        };
        let none_expected = checks.as_deref() == Some("none");
        // A project named by `root` has no remote in the pipeline; the
        // tree's own origin is what its PRs are against.
        let origin = self.git.remote_url(cwd)?;
        let target = match pr_target(t, p, stage, lane, provider.as_deref(), origin) {
            Ok(target) => target,
            Err(why) => return self.park(t, ps, &why, now_ms),
        };
        let head = self.git.head(cwd)?;
        let reading = self.read_pr(&target, none_expected);
        let question = match reading {
            Err(e) => match self.record_pr_error(t, a, &target, &head, &e, now_ms)? {
                None => return Ok(()),
                Some(question) => question,
            },
            Ok(None) => {
                attempt_mut(t, a).pr = None;
                self.save_ticket(t, now_ms)?;
                format!(
                    "no pull request for branch {} in {}; open one, then answer recheck",
                    target.branch, target.repo
                )
            }
            Ok(Some((pr, checks))) => {
                let (summary, verdict) = judge_pr(&pr, checks.as_ref(), &head, none_expected);
                let attempt = attempt_mut(t, a);
                attempt.pr = Some(PullRequestRecord {
                    number: pr.number,
                    url: pr.url.clone(),
                    checks: summary.clone(),
                    ..target.record(&pr.head, now_ms)
                });
                match verdict {
                    Ok(true) => {
                        attempt.head = Some(head.clone());
                        attempt.state = AttemptState::Complete;
                        attempt.ended_ms = Some(now_ms);
                        log::info!(
                            "ticket {} {}/{} PR #{} {summary} at {head}",
                            t.id,
                            a.stage,
                            a.context,
                            pr.number
                        );
                        return self.save_ticket(t, now_ms);
                    }
                    Ok(false) => return self.save_ticket(t, now_ms),
                    Err(why) => {
                        self.save_ticket(t, now_ms)?;
                        why
                    }
                }
            }
        };
        self.ensure_decision(
            t,
            ps,
            Ask {
                stage: &a.stage,
                name: "pr",
                kind: DecisionKind::Permission,
                question: format!("{} ({}): {question}", a.stage, a.context),
                options: &["recheck", "park"],
                recommendation: None,
                attempt: Some((a.stage.clone(), a.n)),
            },
            now_ms,
        )
    }

    /// The PR for a branch and, when it is open and checks are
    /// expected, what its checks say.
    #[allow(clippy::type_complexity)]
    fn read_pr(
        &self,
        target: &PrTarget,
        none_expected: bool,
    ) -> Result<Option<(crate::github::PullRequest, Option<Checks>)>> {
        let prs = self.prs_for(&target.provider);
        let Some(pr) = prs.find(&target.repo, &target.branch)? else {
            return Ok(None);
        };
        if pr.state != "open" || none_expected {
            return Ok(Some((pr, None)));
        }
        let checks = prs.checks(&target.repo, pr.number)?;
        Ok(Some((pr, Some(checks))))
    }

    /// The provider a target names.
    fn prs_for(&self, provider: &str) -> &dyn PullRequests {
        if provider == "bitbucket" {
            self.bitbucket.as_ref()
        } else {
            self.prs.as_ref()
        }
    }

    /// A human gate-only stage other than `lanes`: one attempt per
    /// context that launches nothing and one decision each, with what
    /// the user needs to judge the work in the question. `proceed`
    /// completes it, `rerun` with a note sends the context back to the
    /// nearest earlier agent stage, `park` stops. A `confirm` gate is
    /// "you did this": its answer is `done`.
    #[allow(clippy::too_many_arguments)]
    fn human_gate(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        decision: &str,
        confirm: bool,
        now_ms: u64,
    ) -> Result<()> {
        let contexts = Self::contexts(t, p, stage);
        if contexts.is_empty() {
            return self.park(
                t,
                ps,
                &format!("stage {} needs lanes the ticket has not cut", stage.name),
                now_ms,
            );
        }
        let mut all_complete = true;
        for (ctx, cwd, lane) in contexts {
            let Some(attempt) = self.open_gate_attempt(t, stage, &ctx, now_ms)? else {
                continue;
            };
            all_complete = false;
            let question = self.human_question(t, p, stage, &attempt, &cwd, lane.as_deref())?;
            let (kind, options): (DecisionKind, &[&str]) = if confirm {
                (DecisionKind::Confirmation, &["done", "park"])
            } else {
                (DecisionKind::Permission, &["proceed", "rerun", "park"])
            };
            self.ensure_decision(
                t,
                ps,
                Ask {
                    stage: &stage.name,
                    name: decision,
                    kind,
                    question,
                    options,
                    recommendation: None,
                    attempt: Some((attempt.stage.clone(), attempt.n)),
                },
                now_ms,
            )?;
            if !t.active() {
                return Ok(());
            }
        }
        if all_complete {
            self.advance(t, ps, now_ms)?;
        }
        Ok(())
    }

    /// The open gate-only attempt of a stage in a context: the one
    /// there is, or a new one; `None` when the context is complete.
    fn open_gate_attempt(
        &mut self,
        t: &mut Ticket,
        stage: &Stage,
        ctx: &str,
        now_ms: u64,
    ) -> Result<Option<Attempt>> {
        let last = t
            .attempts
            .iter()
            .filter(|a| a.stage == stage.name && a.context == ctx)
            .max_by_key(|a| a.n)
            .cloned();
        Ok(match last {
            Some(a) if a.state == AttemptState::Complete => None,
            Some(a) if a.is_open() => Some(a),
            _ => {
                let a = new_attempt(
                    &stage.name,
                    next_n(t, &stage.name),
                    ctx,
                    AttemptKind::GateOnly,
                    AttemptState::Running,
                    BTreeMap::new(),
                    now_ms,
                );
                t.attempts.push(a.clone());
                self.save_ticket(t, now_ms)?;
                Some(a)
            }
        })
    }

    /// What a human gate shows: the branch and its head, what it adds
    /// over its base, the tree to open, and the latest notes.
    fn human_question(
        &self,
        t: &Ticket,
        p: &Pipeline,
        stage: &Stage,
        a: &Attempt,
        cwd: &Path,
        lane: Option<&str>,
    ) -> Result<String> {
        let mut q = format!("{} ({}):", stage.name, a.context);
        let lane_record = lane.and_then(|l| t.lanes.iter().find(|x| x.name == l));
        if let Some(l) = lane_record
            && p.cuts_worktrees()
        {
            let head = self.git.head(cwd)?;
            let base = p.lane(&l.name).map_or_else(
                || p.project.base.clone(),
                |lane| format!("{}/{}", p.lane_remote(lane), p.lane_base(lane)),
            );
            let short: String = head.chars().take(8).collect();
            let _ = write!(q, " branch {} at {short} over {base}.", l.branch);
            let summary = self.git.summary(cwd, &base)?;
            if !summary.trim().is_empty() {
                q.push_str("\n\n");
                q.push_str(summary.trim());
            }
        }
        let _ = write!(q, "\n\nTree: {}", cwd.display());
        if let Some(notes) = t.input("notes") {
            let _ = write!(q, "\nNotes: {}", notes.display());
        }
        Ok(q)
    }

    /// The gate's attempt completes, bound to the tree's head.
    fn pass_human_gate(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        attempt: Option<&(String, u32)>,
        now_ms: u64,
    ) -> Result<()> {
        let Some((stage, n)) = attempt else {
            return Ok(());
        };
        let Some(a) = t
            .attempts
            .iter()
            .find(|a| &a.stage == stage && a.n == *n)
            .cloned()
        else {
            return Ok(());
        };
        let head = match tree_of(t, p, &a.context) {
            Some(cwd) => Some(self.git.head(&cwd)?),
            None => None,
        };
        let record = attempt_mut(t, &a);
        record.head = head;
        record.state = AttemptState::Complete;
        record.ended_ms = Some(now_ms);
        log::info!("ticket {} {}/{} passed by hand", t.id, a.stage, a.context);
        self.save_ticket(t, now_ms)?;
        self.unmark(t, ps, now_ms)
    }

    /// A context sent back from a human gate: the gate's attempt is
    /// cancelled, the nearest earlier agent stage's result for that
    /// context is cancelled too, the note is kept for that stage's
    /// next prompt, and the ticket stands at that stage again. Other
    /// contexts' results are untouched.
    #[allow(clippy::too_many_arguments)]
    fn send_back(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        from: &str,
        attempt: Option<&(String, u32)>,
        note: Option<String>,
        now_ms: u64,
    ) -> Result<()> {
        let Some((stage, n)) = attempt else {
            return Ok(());
        };
        let Some(gate) = t
            .attempts
            .iter()
            .find(|a| &a.stage == stage && a.n == *n)
            .cloned()
        else {
            return Ok(());
        };
        let Some(back_to) = p
            .stages
            .iter()
            .take(t.stage)
            .rposition(|s| s.kind() == StageKind::Agent)
        else {
            return self.park(
                t,
                ps,
                &format!("{from}: nothing before {} can be run again", gate.stage),
                now_ms,
            );
        };
        let target = p.stages[back_to].name.clone();
        let note = note.unwrap_or_else(|| format!("sent back from {from} without a note"));
        let reason = format!("sent back from {from}: {note}");
        let record = attempt_mut(t, &gate);
        record.state = AttemptState::Cancelled {
            reason: reason.clone(),
        };
        record.ended_ms = Some(now_ms);
        if let Some(done) = t
            .attempts
            .iter_mut()
            .filter(|a| {
                a.stage == target && a.context == gate.context && a.state == AttemptState::Complete
            })
            .max_by_key(|a| a.n)
        {
            done.state = AttemptState::Cancelled { reason };
            done.ended_ms = Some(now_ms);
        }
        t.rework.insert(rework_key(&target, &gate.context), note);
        t.stage = back_to;
        log::info!(
            "ticket {} {}/{} sent back to {target}",
            t.id,
            gate.stage,
            gate.context
        );
        self.save_ticket(t, now_ms)?;
        self.unmark(t, ps, now_ms)
    }

    /// A `pr-merged` stage: one gate-only attempt per context that
    /// makes the merge decision, a confirmation the provider resolves.
    /// The PR is read once a minute; merged completes the attempt and
    /// answers the decision as Dispatch. Nothing here merges.
    fn pr_merged_stage(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        decision: &str,
        now_ms: u64,
    ) -> Result<()> {
        let contexts = Self::contexts(t, p, stage);
        if contexts.is_empty() {
            return self.park(
                t,
                ps,
                &format!("stage {} needs lanes the ticket has not cut", stage.name),
                now_ms,
            );
        }
        let mut all_complete = true;
        for (ctx, cwd, lane) in contexts {
            let Some(attempt) = self.open_gate_attempt(t, stage, &ctx, now_ms)? else {
                continue;
            };
            all_complete = false;
            let poll = PrPoll {
                stage,
                decision,
                attempt: &attempt,
                cwd: &cwd,
                lane: lane.as_deref(),
            };
            self.poll_pr_merged(t, ps, p, &poll, now_ms)?;
            if !t.active() {
                return Ok(());
            }
        }
        if all_complete {
            self.advance(t, ps, now_ms)?;
        }
        Ok(())
    }

    fn poll_pr_merged(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        poll: &PrPoll<'_>,
        now_ms: u64,
    ) -> Result<()> {
        let PrPoll {
            stage,
            decision,
            attempt: a,
            cwd,
            lane,
        } = *poll;
        if a.pr
            .as_ref()
            .is_some_and(|pr| pr.checked_ms != 0 && now_ms < pr.checked_ms + PR_POLL_MS)
        {
            return Ok(());
        }
        let provider = match &stage.gate {
            Some(Gate::External { provider, .. }) => provider.as_deref(),
            _ => None,
        };
        let origin = self.git.remote_url(cwd)?;
        let target = match pr_target(t, p, stage, lane, provider, origin) {
            Ok(target) => target,
            Err(why) => return self.park(t, ps, &why, now_ms),
        };
        let head = self.git.head(cwd)?;
        let found = self
            .prs_for(&target.provider)
            .find(&target.repo, &target.branch);
        let question = match found {
            Err(e) => match self.record_pr_error(t, a, &target, &head, &e, now_ms)? {
                None => return Ok(()),
                Some(question) => question,
            },
            Ok(None) => {
                attempt_mut(t, a).pr = None;
                self.save_ticket(t, now_ms)?;
                format!(
                    "no pull request for branch {} in {}; open one, then answer recheck",
                    target.branch, target.repo
                )
            }
            Ok(Some(pr)) => {
                attempt_mut(t, a).pr = Some(PullRequestRecord {
                    number: pr.number,
                    url: pr.url.clone(),
                    checks: pr.state.clone(),
                    ..target.record(&pr.head, now_ms)
                });
                self.save_ticket(t, now_ms)?;
                match pr.state.as_str() {
                    "merged" => return self.merged(t, ps, a, decision, &pr, now_ms),
                    "closed" => format!("PR #{} is closed without being merged", pr.number),
                    _ => {
                        let question = format!(
                            "{} ({}): PR #{} {} is open at {}; merge it there. Dispatch resolves this when the provider reports the merge.",
                            a.stage,
                            a.context,
                            pr.number,
                            pr.url,
                            pr.head.chars().take(8).collect::<String>()
                        );
                        return self.ensure_decision(
                            t,
                            ps,
                            Ask {
                                stage: &a.stage,
                                name: decision,
                                kind: DecisionKind::Confirmation,
                                question,
                                options: &["park"],
                                recommendation: None,
                                attempt: Some((a.stage.clone(), a.n)),
                            },
                            now_ms,
                        );
                    }
                }
            }
        };
        self.ensure_decision(
            t,
            ps,
            Ask {
                stage: &a.stage,
                name: "pr",
                kind: DecisionKind::Permission,
                question: format!("{} ({}): {question}", a.stage, a.context),
                options: &["recheck", "park"],
                recommendation: None,
                attempt: Some((a.stage.clone(), a.n)),
            },
            now_ms,
        )
    }

    /// The provider reports the merge: the attempt completes at the
    /// merged head and the merge decision reads as answered by Dispatch.
    fn merged(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        a: &Attempt,
        decision: &str,
        pr: &crate::github::PullRequest,
        now_ms: u64,
    ) -> Result<()> {
        let record = attempt_mut(t, a);
        record.head = Some(pr.head.clone());
        record.state = AttemptState::Complete;
        record.ended_ms = Some(now_ms);
        let key = (a.stage.clone(), a.n);
        for d in t
            .decisions
            .iter_mut()
            .filter(|d| d.pending() && d.name == decision && d.attempt.as_ref() == Some(&key))
        {
            d.state = DecisionState::Answered {
                answer: "merged".into(),
                note: None,
                by: BY_DISPATCH.into(),
                at_ms: now_ms,
                acted: true,
            };
        }
        log::info!(
            "ticket {} {}/{} PR #{} merged",
            t.id,
            a.stage,
            a.context,
            pr.number
        );
        self.save_ticket(t, now_ms)?;
        self.unmark(t, ps, now_ms)
    }

    /// A provider that could not be read: noted on the attempt and
    /// retried quietly, until it has failed for `PR_ERROR_GRACE_MS`;
    /// then the question to ask.
    fn record_pr_error(
        &mut self,
        t: &mut Ticket,
        a: &Attempt,
        target: &PrTarget,
        head: &str,
        e: &anyhow::Error,
        now_ms: u64,
    ) -> Result<Option<String>> {
        log::warn!("ticket {} {}/{}: {e:#}", t.id, a.stage, a.context);
        let since =
            a.pr.as_ref()
                .and_then(|pr| pr.error_since_ms)
                .unwrap_or(now_ms);
        attempt_mut(t, a).pr = Some(PullRequestRecord {
            number: 0,
            url: String::new(),
            checks: format!("error: {e:#}"),
            error_since_ms: Some(since),
            ..target.record(head, now_ms)
        });
        self.save_ticket(t, now_ms)?;
        if now_ms.saturating_sub(since) < PR_ERROR_GRACE_MS {
            return Ok(None);
        }
        Ok(Some(format!(
            "the pull request for {} in {} could not be read for an hour: {e:#}",
            target.branch, target.repo
        )))
    }

    fn finish_gate_only(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        stage: &Stage,
        now_ms: u64,
    ) -> Result<()> {
        let n = u32::try_from(t.attempts_of(&stage.name).count()).unwrap_or(u32::MAX) + 1;
        t.attempts.push(new_attempt(
            &stage.name,
            n,
            "root",
            AttemptKind::GateOnly,
            AttemptState::Complete,
            BTreeMap::new(),
            now_ms,
        ));
        self.advance(t, ps, now_ms)
    }

    fn advance(&mut self, t: &mut Ticket, ps: &mut ProjectState, now_ms: u64) -> Result<()> {
        t.stage += 1;
        self.save_ticket(t, now_ms)?;
        let _ = ps;
        Ok(())
    }

    // --- contexts and projects

    /// The contexts a stage runs in: `(name, cwd, lane)`. Empty when the
    /// stage needs lanes the ticket has not cut.
    fn contexts(t: &Ticket, p: &Pipeline, stage: &Stage) -> Vec<(String, PathBuf, Option<String>)> {
        let Some(tree) = primary_tree(t, p) else {
            return Vec::new();
        };
        match &stage.context {
            Context::Root => vec![("root".to_owned(), tree, None)],
            Context::Joined => vec![("joined".to_owned(), tree, None)],
            Context::Each => t
                .lanes
                .iter()
                .filter(|l| l.chosen)
                .map(|l| (l.name.clone(), l.worktree.clone(), Some(l.name.clone())))
                .collect(),
            Context::Lane(name) => t
                .lanes
                .iter()
                .filter(|l| &l.name == name)
                .map(|l| (l.name.clone(), l.worktree.clone(), Some(l.name.clone())))
                .collect(),
            Context::Lanes(names) => t
                .lanes
                .iter()
                .filter(|l| names.contains(&l.name))
                .map(|l| (l.name.clone(), l.worktree.clone(), Some(l.name.clone())))
                .collect(),
        }
    }

    /// The Switchboard workspace the pipeline names, found or made once.
    fn ensure_space(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        now_ms: u64,
    ) -> Result<String> {
        if let Some(space) = &ps.space {
            return Ok(space.clone());
        }
        if let Reply::Spaces { spaces } = self.ask(Body::Spaces)?
            && let Some(s) = spaces.iter().find(|s| s.name == p.project.space)
        {
            ps.space = Some(s.id.clone());
            self.save_project(ps)?;
            return Ok(s.id.clone());
        }
        let reply = self.send(
            t,
            ps,
            None,
            "space",
            Body::SpaceNew {
                name: p.project.space.clone(),
            },
            now_ms,
        )?;
        ps.space
            .clone()
            .ok_or_else(|| anyhow!("space.new: {reply:?}"))
    }

    /// The Switchboard project for a context, made once per ticket.
    /// The ticket's one Switchboard project, `#<n> <title>`, rooted at
    /// the ticket's tree; sessions in other lanes carry their own cwd.
    fn ensure_project(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        now_ms: u64,
    ) -> Result<String> {
        if let Some(id) = t.root_project.clone() {
            return Ok(id);
        }
        let root = primary_tree(t, p).ok_or_else(|| anyhow!("the ticket has no tree yet"))?;
        let space = self.ensure_space(t, ps, p, now_ms)?;
        let name = format!("#{} {}", t.source.number.unwrap_or(0), t.source.title);
        let reply = self.send(
            t,
            ps,
            None,
            "root-project",
            Body::ProjectAdd { space, name, root },
            now_ms,
        )?;
        t.root_project
            .clone()
            .ok_or_else(|| anyhow!("project.add: {reply:?}"))
    }

    // --- agent stages

    fn agent_stage(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        held: bool,
        now_ms: u64,
    ) -> Result<()> {
        // A command gate runs after the agent, in its context; the
        // other gates on an agent stage are later work.
        match &stage.gate {
            None | Some(Gate::Command { .. }) => {}
            Some(Gate::External { check, .. }) => {
                return self.park(
                    t,
                    ps,
                    &format!("stage {} ({check}) is not built in this slice", stage.name),
                    now_ms,
                );
            }
            Some(Gate::Human { decision, .. }) => {
                return self.park(
                    t,
                    ps,
                    &format!(
                        "stage {} ({decision}) is not built in this slice",
                        stage.name
                    ),
                    now_ms,
                );
            }
        }
        let contexts = Self::contexts(t, p, stage);
        if contexts.is_empty() {
            return self.park(
                t,
                ps,
                &format!("stage {} needs lanes the ticket has not cut", stage.name),
                now_ms,
            );
        }
        let mut all_complete = true;
        for (ctx, cwd, lane) in contexts {
            let last = t
                .attempts
                .iter()
                .filter(|a| a.stage == stage.name && a.context == ctx)
                .max_by_key(|a| a.n)
                .cloned();
            match last {
                Some(a) if a.state == AttemptState::Complete => {}
                Some(a) if a.is_open() => {
                    all_complete = false;
                    let trust = p.policy.trust_folders;
                    self.poll_agent(t, ps, &a, stage, &cwd, lane.as_deref(), trust, now_ms)?;
                }
                Some(a) => {
                    // Failed: a rerun waits on its decision. Sent back
                    // from a later human gate: the note is the answer.
                    all_complete = false;
                    let sent_back = t.rework.contains_key(&rework_key(&stage.name, &ctx));
                    if !held && (may_rerun(t, &a) || sent_back) {
                        self.start_agent(
                            t,
                            ps,
                            p,
                            stage,
                            &ctx,
                            &cwd,
                            lane.as_deref(),
                            next_n(t, &stage.name),
                            now_ms,
                        )?;
                    }
                }
                None => {
                    all_complete = false;
                    if !held {
                        let n = next_n(t, &stage.name);
                        self.start_agent(t, ps, p, stage, &ctx, &cwd, lane.as_deref(), n, now_ms)?;
                    }
                }
            }
            if !t.active() {
                return Ok(());
            }
        }
        if all_complete {
            self.advance(t, ps, now_ms)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn start_agent(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        ctx: &str,
        cwd: &Path,
        lane: Option<&str>,
        n: u32,
        now_ms: u64,
    ) -> Result<()> {
        let operator_name = stage.operator.clone().unwrap_or_default();
        let Some(operator) = p.operators.get(&operator_name) else {
            return self.park(
                t,
                ps,
                &format!("stage {} names no operator", stage.name),
                now_ms,
            );
        };
        let project = match self.ensure_project(t, ps, p, now_ms) {
            Ok(id) => id,
            Err(e) => return self.park(t, ps, &format!("stage {}: {e:#}", stage.name), now_ms),
        };
        if !self.ensure_setup(t, ps, p, cwd, now_ms)? {
            return Ok(());
        }
        let dir = self
            .data
            .ticket_dir(&t.id)
            .join(&stage.name)
            .join(n.to_string())
            .join(ctx);
        std::fs::create_dir_all(&dir)?;
        let artifacts: BTreeMap<String, PathBuf> = stage
            .writes
            .iter()
            .map(|w| (w.clone(), dir.join(format!("{w}.md"))))
            .collect();
        let mut vars = vars_for(t, p, lane);
        for (name, path) in &artifacts {
            vars.set(name.clone(), path.display().to_string());
        }
        let mut prompt = String::new();
        // Guidance is a template like the stage prompt: it may name the
        // branch, the worktree or an artifact.
        if !operator.guidance.trim().is_empty() {
            prompt.push_str(&vars.render(operator.guidance.trim()));
            prompt.push_str("\n\n");
        }
        prompt.push_str(&vars.render(stage.prompt.as_deref().unwrap_or_default()));
        if let Some(note) = t.rework.remove(&rework_key(&stage.name, ctx)) {
            prompt.push_str("\n\nThe user looked at the previous attempt and sent it back: ");
            prompt.push_str(&note);
        }
        let mut attempt = new_attempt(
            &stage.name,
            n,
            ctx,
            AttemptKind::Agent,
            AttemptState::Starting,
            artifacts,
            now_ms,
        );
        attempt.project = Some(project.clone());
        // Not saved here: `send` writes the attempt and its request in
        // one go, so no record ever shows the one without the other.
        t.attempts.push(attempt);
        // The artifacts live outside the agent's cwd, in Dispatch's own
        // directory; Claude Code writes there unasked only under an
        // allow rule for the path (an added directory still asks before
        // creating a file).
        let mut args = operator.args.clone();
        args.extend(operator.kind.write_flags(&dir));
        let kind = session_kind(operator.kind);
        let launch = if args.is_empty() {
            wire::Launch::Shell
        } else {
            wire::Launch::Argv(args)
        };
        let notes = format!(
            "Dispatch ticket {} · #{} {} · stage {} attempt {n}",
            t.id,
            t.source.number.unwrap_or(0),
            t.source.title,
            stage.name
        );
        let reply = self.send(
            t,
            ps,
            Some((stage.name.clone(), n)),
            "session",
            Body::SessionNew {
                project,
                name: operator_name,
                session_kind: kind,
                cwd: cwd.to_path_buf(),
                launch,
                prompt: Some(prompt),
                notes,
            },
            now_ms,
        )?;
        if let Reply::Failed { reason } = reply {
            self.fail_attempt(
                t,
                ps,
                &stage.name,
                n,
                &format!("could not start: {reason}"),
                now_ms,
            )?;
        }
        Ok(())
    }

    /// What Switchboard says about the attempt's session, and what that
    /// makes of the attempt.
    #[allow(clippy::too_many_arguments)]
    fn poll_agent(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        a: &Attempt,
        stage: &Stage,
        cwd: &Path,
        lane: Option<&str>,
        trust: bool,
        now_ms: u64,
    ) -> Result<()> {
        let Some(session) = a.session.clone() else {
            // Sent but no reply adopted yet: recovery's to resolve.
            return Ok(());
        };
        // The agent is done and the gate is running or about to: the
        // session is no longer what is watched.
        if a.gate.is_some() {
            return self.poll_gate(t, ps, a, stage, cwd, lane, now_ms);
        }
        let Some(view) = self.watched_view(t, ps, a, &session, now_ms)? else {
            return Ok(());
        };
        if view.trust_question && trust {
            return self.answer_trust(t, ps, a, session, now_ms);
        }
        let idx = t
            .attempts
            .iter()
            .position(|x| x.stage == a.stage && x.n == a.n)
            .expect("the attempt exists");
        let attempt = &mut t.attempts[idx];
        if attempt.state == AttemptState::Starting {
            attempt.state = AttemptState::Running;
        }
        if let Some(stop) = view.last_stop_at_ms {
            attempt.stop_at_ms = Some(stop);
        }
        let stopped = attempt.stop_at_ms.is_some()
            || matches!(view.liveness, wire::Liveness::Exited { code: Some(0) });
        if !stopped {
            return match view.liveness {
                wire::Liveness::Running => self.save_ticket(t, now_ms),
                wire::Liveness::Exited { code } => {
                    let reason = format!("exited {code:?} before finishing");
                    self.fail_attempt(t, ps, &a.stage, a.n, &reason, now_ms)
                }
                wire::Liveness::Missing => {
                    let reason = "pane gone with no stop recorded";
                    self.fail_attempt(t, ps, &a.stage, a.n, reason, now_ms)
                }
            };
        }
        attempt.polls_since_stop += 1;
        let missing = missing_artifacts(attempt);
        if !missing.is_empty() {
            if attempt.polls_since_stop >= SETTLE_POLLS || view.liveness != wire::Liveness::Running
            {
                return self.fail_attempt(
                    t,
                    ps,
                    &a.stage,
                    a.n,
                    &format!("stopped without writing {}", missing.join(", ")),
                    now_ms,
                );
            }
            return self.save_ticket(t, now_ms);
        }
        if !settle(attempt)? {
            return self.save_ticket(t, now_ms);
        }
        let gated = matches!(stage.gate, Some(Gate::Command { .. }));
        if gated {
            log::info!(
                "ticket {} {}/{} agent stopped; checks next",
                t.id,
                a.stage,
                a.context
            );
        } else {
            attempt.state = AttemptState::Complete;
            attempt.ended_ms = Some(now_ms);
            log::info!("ticket {} {}/{} complete", t.id, a.stage, a.context);
        }
        self.save_ticket(t, now_ms)?;
        if view.liveness == wire::Liveness::Running {
            self.send(
                t,
                ps,
                Some((a.stage.clone(), a.n)),
                "kill",
                Body::SessionKill { session },
                now_ms,
            )?;
        }
        if gated {
            self.start_gate(t, ps, a, stage, cwd, lane, now_ms)?;
        }
        Ok(())
    }

    /// A session as Switchboard sees it, or why it has none.
    /// The session view an open attempt is judged by. `None` when the
    /// pass is over for it: the record is gone (Switchboard's word, not
    /// a guess) and the attempt failed, or the query failed for another
    /// reason (the app busy starting, a bad moment on the socket), which
    /// says nothing about the session, so it is asked again next pass.
    fn watched_view(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        a: &Attempt,
        session: &str,
        now_ms: u64,
    ) -> Result<Option<wire::SessionView>> {
        match self.session_view(session)? {
            Ok(view) => Ok(Some(view)),
            Err(reason) if reason == NO_SUCH_SESSION => {
                self.fail_attempt(
                    t,
                    ps,
                    &a.stage,
                    a.n,
                    &format!("session gone: {reason}"),
                    now_ms,
                )?;
                Ok(None)
            }
            Err(reason) => {
                log::warn!(
                    "ticket {} {}/{}: session query failed: {reason}; asking again",
                    t.id,
                    a.stage,
                    a.context
                );
                self.save_ticket(t, now_ms)?;
                Ok(None)
            }
        }
    }

    /// Claude's folder trust question, which a fresh worktree asks
    /// before any hook: answered for the project when its policy says
    /// so, else left to the user.
    fn answer_trust(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        a: &Attempt,
        session: String,
        now_ms: u64,
    ) -> Result<()> {
        log::info!(
            "ticket {} {}/{} answers the folder trust question",
            t.id,
            a.stage,
            a.context
        );
        self.send(
            t,
            ps,
            Some((a.stage.clone(), a.n)),
            "trust",
            Body::SessionTrust { session },
            now_ms,
        )?;
        self.save_ticket(t, now_ms)
    }

    fn session_view(&mut self, session: &str) -> Result<Result<wire::SessionView, String>> {
        let reply = self.ask(Body::Session {
            session: session.to_owned(),
        })?;
        match reply {
            Reply::Session { session } => Ok(Ok(session)),
            Reply::Failed { reason } => Ok(Err(reason)),
            other => bail!("session query answered {other:?}"),
        }
    }

    /// The stage's command gate, once the agent has stopped: the tree
    /// must be clean, its head is recorded, and the command starts as a
    /// child of this runner in the context's tree with the ticket's
    /// values in its environment and its output in the attempt's
    /// `checks.log`. A dirty tree is a failed attempt, never a run.
    #[allow(clippy::too_many_arguments)]
    fn start_gate(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        a: &Attempt,
        stage: &Stage,
        cwd: &Path,
        lane: Option<&str>,
        now_ms: u64,
    ) -> Result<()> {
        let Some(Gate::Command { argv, per_lane, .. }) = &stage.gate else {
            return Ok(());
        };
        let argv = lane
            .and_then(|l| per_lane.as_ref().and_then(|m| m.get(l)))
            .or(argv.as_ref())
            .cloned();
        let Some(argv) = argv.filter(|v| !v.is_empty()) else {
            let reason = format!("no checks command for context {}", a.context);
            return self.fail_attempt(t, ps, &a.stage, a.n, &reason, now_ms);
        };
        if !self.git.is_clean(cwd)? {
            let reason = format!("the tree at {} is not clean after the agent", cwd.display());
            return self.fail_checks(t, ps, &a.stage, a.n, &reason, now_ms);
        }
        let head = self.git.head(cwd)?;
        let dir = self
            .data
            .ticket_dir(&t.id)
            .join(&a.stage)
            .join(a.n.to_string())
            .join(&a.context);
        std::fs::create_dir_all(&dir)?;
        let log = dir.join("checks.log");
        let lane_record = lane.and_then(|l| t.lanes.iter().find(|x| x.name == l));
        let mut env = vec![
            ("DISPATCH_TICKET".to_owned(), t.id.clone()),
            ("DISPATCH_STAGE".to_owned(), a.stage.clone()),
            ("DISPATCH_CONTEXT".to_owned(), a.context.clone()),
            ("DISPATCH_TREE".to_owned(), cwd.display().to_string()),
            ("DISPATCH_HEAD".to_owned(), head.clone()),
        ];
        if let Some(l) = lane_record {
            env.push(("DISPATCH_LANE".to_owned(), l.name.clone()));
            env.push(("DISPATCH_BRANCH".to_owned(), l.branch.clone()));
        }
        let key = gate_key(t, a);
        if let Err(e) = self.git.start_check(&key, cwd, &argv, &env, &log) {
            let reason = format!("the checks could not start: {e:#}");
            return self.fail_attempt(t, ps, &a.stage, a.n, &reason, now_ms);
        }
        log::info!(
            "ticket {} {}/{} checks started at {head}",
            t.id,
            a.stage,
            a.context
        );
        if let Some(attempt) = t
            .attempts
            .iter_mut()
            .find(|x| x.stage == a.stage && x.n == a.n)
        {
            attempt.gate = Some(GateRun {
                head,
                argv,
                log: log.clone(),
                started_ms: now_ms,
                exit: None,
            });
            attempt.artifacts.insert("checks".into(), log);
        }
        self.save_ticket(t, now_ms)
    }

    /// The gate's child: still running, exited, or gone with a runner
    /// that restarted (then started again on the same clean head, since
    /// a check is worth nothing until its result is bound). An exit is
    /// bound to the head only if the tree is still clean at it.
    #[allow(clippy::too_many_arguments)]
    fn poll_gate(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        a: &Attempt,
        stage: &Stage,
        cwd: &Path,
        lane: Option<&str>,
        now_ms: u64,
    ) -> Result<()> {
        let Some(gate) = a.gate.clone() else {
            return Ok(());
        };
        let key = gate_key(t, a);
        let code = match self.git.poll_check(&key) {
            None => return Ok(()),
            Some(Ok(code)) => code,
            Some(Err(e)) => {
                log::warn!(
                    "ticket {} {}/{} checks lost ({e:#}); starting again",
                    t.id,
                    a.stage,
                    a.context
                );
                if let Some(attempt) = t
                    .attempts
                    .iter_mut()
                    .find(|x| x.stage == a.stage && x.n == a.n)
                {
                    attempt.gate = None;
                }
                return self.start_gate(t, ps, a, stage, cwd, lane, now_ms);
            }
        };
        let clean = self.git.is_clean(cwd)?;
        let head = self.git.head(cwd)?;
        if let Some(attempt) = t
            .attempts
            .iter_mut()
            .find(|x| x.stage == a.stage && x.n == a.n)
            && let Some(g) = &mut attempt.gate
        {
            g.exit = Some(code);
        }
        if !clean || head != gate.head {
            let reason = format!(
                "the tree at {} changed while the checks ran (head {} then {head})",
                cwd.display(),
                gate.head
            );
            return self.fail_checks(t, ps, &a.stage, a.n, &reason, now_ms);
        }
        if code != 0 {
            let reason = format!("checks exited {code}; output at {}", gate.log.display());
            return self.fail_checks(t, ps, &a.stage, a.n, &reason, now_ms);
        }
        if let Some(attempt) = t
            .attempts
            .iter_mut()
            .find(|x| x.stage == a.stage && x.n == a.n)
        {
            attempt.head = Some(head);
            attempt.state = AttemptState::Complete;
            attempt.ended_ms = Some(now_ms);
        }
        log::info!(
            "ticket {} {}/{} checks passed at {}",
            t.id,
            a.stage,
            a.context,
            gate.head
        );
        self.save_ticket(t, now_ms)
    }

    pub(crate) fn fail_attempt(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        stage: &str,
        n: u32,
        reason: &str,
        now_ms: u64,
    ) -> Result<()> {
        self.fail_attempt_with(t, ps, stage, n, reason, &["rerun", "park"], now_ms)
    }

    /// A failure at the stage's checks: the work may be fine and the
    /// environment not, so the checks can be run again on the same
    /// attempt without another agent.
    fn fail_checks(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        stage: &str,
        n: u32,
        reason: &str,
        now_ms: u64,
    ) -> Result<()> {
        self.fail_attempt_with(t, ps, stage, n, reason, &["rerun", "check", "park"], now_ms)
    }

    /// The attempt fails and the user is asked what next, unless the
    /// stage has failed in this context as often as the policy's
    /// `max_reruns` allows: then the ticket parks, so a broken stage
    /// cannot spend agent runs on its own.
    #[allow(clippy::too_many_arguments)]
    fn fail_attempt_with(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        stage: &str,
        n: u32,
        reason: &str,
        options: &[&str],
        now_ms: u64,
    ) -> Result<()> {
        let Some(attempt) = t.attempts.iter_mut().find(|a| a.stage == stage && a.n == n) else {
            return Ok(());
        };
        attempt.state = AttemptState::Failed {
            reason: reason.into(),
        };
        attempt.ended_ms = Some(now_ms);
        let ctx = attempt.context.clone();
        log::warn!("ticket {} {stage}/{ctx} attempt {n} failed: {reason}", t.id);
        self.save_ticket(t, now_ms)?;
        let failed = t
            .attempts
            .iter()
            .filter(|a| {
                a.stage == stage
                    && a.context == ctx
                    && matches!(a.state, AttemptState::Failed { .. })
            })
            .count();
        let max_reruns = self.pipeline_of(t).map_or(3, |p| p.policy.max_reruns);
        if failed > max_reruns as usize {
            return self.park(
                t,
                ps,
                &format!(
                    "{stage} ({ctx}) failed {failed} times, more than the policy's max_reruns of {max_reruns}; last: {reason}"
                ),
                now_ms,
            );
        }
        self.ensure_decision(
            t,
            ps,
            Ask {
                stage,
                name: "rerun",
                kind: DecisionKind::Permission,
                question: format!("{stage} ({ctx}) attempt {n} failed: {reason}. Run it again?"),
                options,
                recommendation: None,
                attempt: Some((stage.to_owned(), n)),
            },
            now_ms,
        )
    }

    // --- workflow stages

    fn workflow_stage(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        held: bool,
        now_ms: u64,
    ) -> Result<()> {
        let contexts = Self::contexts(t, p, stage);
        if contexts.is_empty() {
            return self.park(
                t,
                ps,
                &format!("stage {} needs lanes the ticket has not cut", stage.name),
                now_ms,
            );
        }
        let mut all_complete = true;
        for (ctx, _cwd, lane) in contexts {
            let last = t
                .attempts
                .iter()
                .filter(|a| a.stage == stage.name && a.context == ctx)
                .max_by_key(|a| a.n)
                .cloned();
            match last {
                Some(a) if a.state == AttemptState::Complete => {}
                Some(a) if a.is_open() => {
                    all_complete = false;
                    self.poll_workflow(t, ps, p, stage, &a, now_ms)?;
                }
                Some(a) => {
                    all_complete = false;
                    if !held && may_rerun(t, &a) {
                        self.start_workflow(
                            t,
                            ps,
                            p,
                            stage,
                            &ctx,
                            lane.as_deref(),
                            next_n(t, &stage.name),
                            now_ms,
                        )?;
                    }
                }
                None => {
                    all_complete = false;
                    if !held {
                        let n = next_n(t, &stage.name);
                        self.start_workflow(t, ps, p, stage, &ctx, lane.as_deref(), n, now_ms)?;
                    }
                }
            }
            if !t.active() {
                return Ok(());
            }
        }
        if all_complete {
            self.advance(t, ps, now_ms)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn start_workflow(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        ctx: &str,
        lane: Option<&str>,
        n: u32,
        now_ms: u64,
    ) -> Result<()> {
        let reviewer_name = stage.review.clone().unwrap_or_default();
        let operator = p.operators.get(&reviewer_name);
        let Some(review) = operator.and_then(|o| o.review.clone()) else {
            return self.park(
                t,
                ps,
                &format!("stage {} names no reviewer", stage.name),
                now_ms,
            );
        };
        let Some(subject) = stage.subject.clone() else {
            return self.park(
                t,
                ps,
                &format!("stage {} names no subject", stage.name),
                now_ms,
            );
        };
        let (source_path, source_session) = match review_subject(t, &subject) {
            Ok(found) => found,
            Err(why) => return self.park(t, ps, &format!("stage {}: {why}", stage.name), now_ms),
        };
        let dir = self
            .data
            .ticket_dir(&t.id)
            .join(&stage.name)
            .join(n.to_string())
            .join(ctx);
        std::fs::create_dir_all(&dir)?;
        let copy = dir.join(format!("{subject}.md"));
        std::fs::copy(&source_path, &copy)
            .with_context(|| format!("copy {} to {}", source_path.display(), copy.display()))?;
        // The reviewer's feedback lives beside the copy, in Dispatch's
        // directory. A reviewer that can be told to write there runs in
        // the ticket's tree, with the code and the trust the earlier
        // agents already granted; one that can only write in its cwd
        // runs in the attempt directory and is told where the tree is.
        let mut reviewer_args = operator.map(|o| o.args.clone()).unwrap_or_default();
        let reviewer_cwd = match primary_tree(t, p) {
            Some(tree) if review.reviewer.reviews_in_tree() => {
                reviewer_args.extend(review.reviewer.write_flags(&dir));
                tree
            }
            _ => dir.clone(),
        };
        let definition = definition_of(&reviewer_name, review, &vars_for(t, p, lane));
        t.attempts.push(new_attempt(
            &stage.name,
            n,
            ctx,
            AttemptKind::Workflow,
            AttemptState::Starting,
            BTreeMap::from([(subject.clone(), copy.clone())]),
            now_ms,
        ));
        let name = definition.name.clone();
        self.send(
            t,
            ps,
            Some((stage.name.clone(), n)),
            "definition",
            Body::DefinitionInstall { definition },
            now_ms,
        )?;
        let reply = self.send(
            t,
            ps,
            Some((stage.name.clone(), n)),
            "run",
            Body::WorkflowStart {
                source: source_session,
                plan: copy,
                definition: name,
                reviewer_cwd: Some(reviewer_cwd),
                reviewer_args,
            },
            now_ms,
        )?;
        if let Reply::Failed { reason } = reply {
            self.fail_attempt(
                t,
                ps,
                &stage.name,
                n,
                &format!("could not start the review: {reason}"),
                now_ms,
            )?;
        }
        Ok(())
    }

    /// A pending finalize decision for a run that is working again is
    /// cancelled, and the session no longer reads as waiting on it.
    fn withdraw_finalize(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        a: &Attempt,
        now_ms: u64,
    ) -> Result<()> {
        let key = (a.stage.clone(), a.n);
        let mut withdrawn = false;
        for d in t
            .decisions
            .iter_mut()
            .filter(|d| d.pending() && d.name == "finalize" && d.attempt.as_ref() == Some(&key))
        {
            d.state = DecisionState::Cancelled;
            withdrawn = true;
        }
        if !withdrawn {
            return Ok(());
        }
        log::info!(
            "ticket {} {}/{} review is working again; finalize withdrawn",
            t.id,
            a.stage,
            a.context
        );
        self.save_ticket(t, now_ms)?;
        self.unmark(t, ps, now_ms)
    }

    /// The reviewer has nothing further, or the rounds ran out: the
    /// finalize decision, or finalized outright when the dial says so.
    #[allow(clippy::too_many_arguments)]
    fn review_done(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        a: &Attempt,
        view: &wire::RunView,
        now_ms: u64,
    ) -> Result<()> {
        let subject = stage.subject.clone().unwrap_or_default();
        let how = if view.state == RunState::Converged {
            format!("converged after {} round(s)", view.round)
        } else {
            format!("hit its cap of {} rounds", view.cap)
        };
        if p.dial("finalize") == "auto" {
            self.send(
                t,
                ps,
                Some((a.stage.clone(), a.n)),
                "finalize",
                Body::WorkflowFinalize {
                    run: view.id.clone(),
                },
                now_ms,
            )?;
            return Ok(());
        }
        let copy = a
            .artifacts
            .get(&subject)
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        self.ensure_decision(
            t,
            ps,
            Ask {
                stage: &stage.name,
                name: "finalize",
                kind: DecisionKind::Permission,
                question: format!(
                    "The review of {subject} {how}. The reviewed copy is {copy}. Finalize it?"
                ),
                options: &["finalize", "park"],
                recommendation: Some("finalize".into()),
                attempt: Some((a.stage.clone(), a.n)),
            },
            now_ms,
        )
    }

    fn poll_workflow(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        a: &Attempt,
        now_ms: u64,
    ) -> Result<()> {
        let Some(run) = a.run.clone() else {
            return Ok(());
        };
        let view = match self.ask(Body::Workflow { run: run.clone() })? {
            Reply::Workflow { run } => run,
            Reply::Failed { reason } => {
                return self.fail_attempt(
                    t,
                    ps,
                    &a.stage,
                    a.n,
                    &format!("review run gone: {reason}"),
                    now_ms,
                );
            }
            other => bail!("workflow query answered {other:?}"),
        };
        let idx = t
            .attempts
            .iter()
            .position(|x| x.stage == a.stage && x.n == a.n)
            .expect("the attempt exists");
        // The card for the ticket is the planner's while the review runs.
        let face = view
            .planner
            .clone()
            .unwrap_or_else(|| view.reviewer.clone());
        t.attempts[idx].session = Some(face);
        if t.attempts[idx].state == AttemptState::Starting {
            t.attempts[idx].state = AttemptState::Running;
        }
        // The planner clone is made after the start reply, so it joins
        // the process list here, the first time it is seen.
        for id in view.planner.iter().chain(std::iter::once(&view.reviewer)) {
            if !t.processes.contains(id) {
                t.processes.push(id.clone());
            }
        }
        self.save_ticket(t, now_ms)?;
        let subject = stage.subject.clone().unwrap_or_default();
        match view.state {
            RunState::Starting | RunState::AwaitingFeedback | RunState::AwaitingResponse => {
                // Back in a round after converging (the user's own
                // feedback round): the finalize question no longer
                // describes the run, and is asked again when it stops.
                self.withdraw_finalize(t, ps, a, now_ms)
            }
            RunState::Converged | RunState::AtCap => {
                self.review_done(t, ps, p, stage, a, &view, now_ms)
            }
            RunState::Paused { reason } => self.ensure_decision(
                t,
                ps,
                Ask {
                    stage: &stage.name,
                    name: "paused",
                    kind: DecisionKind::Permission,
                    question: format!("The review of {subject} paused: {reason}. Continue it?"),
                    options: &["continue", "park"],
                    recommendation: None,
                    attempt: Some((a.stage.clone(), a.n)),
                },
                now_ms,
            ),
            RunState::Finalized | RunState::HandedOff => {
                t.attempts[idx].state = AttemptState::Complete;
                t.attempts[idx].ended_ms = Some(now_ms);
                log::info!("ticket {} {}/{} finalized", t.id, a.stage, a.context);
                self.save_ticket(t, now_ms)?;
                for session in [Some(view.reviewer.clone()), view.planner.clone()]
                    .into_iter()
                    .flatten()
                {
                    self.send(
                        t,
                        ps,
                        Some((a.stage.clone(), a.n)),
                        "kill",
                        Body::SessionKill { session },
                        now_ms,
                    )?;
                }
                Ok(())
            }
        }
    }

    // --- across a project

    /// Every active ticket of `project`, in queue order, as far as the
    /// slots allow; then the queue view.
    pub fn step_project(&mut self, project: &str, now_ms: u64) -> Result<()> {
        self.transaction(|r| r.step_project_locked(project, now_ms))
    }

    fn step_project_locked(&mut self, project: &str, now_ms: u64) -> Result<()> {
        let mut ps = self.load_project(project)?;
        let mut tickets: Vec<Ticket> = Vec::new();
        for id in ps.queue.clone() {
            match self.load_ticket(&id) {
                Ok(t) => tickets.push(t),
                Err(e) => log::warn!("ticket {id}: {e}"),
            }
        }
        let mut running = 0u32;
        let mut pending = 0u32;
        for t in &tickets {
            if t.active() && t.attempts.iter().any(costs_slot) {
                running += 1;
            }
            pending += u32::try_from(t.pending_decisions().len()).unwrap_or(u32::MAX);
        }
        let mut pipeline: Option<Pipeline> = None;
        for t in &mut tickets {
            if matches!(t.state, TicketState::Parking { .. }) {
                if let Err(e) = self.finish_parking(t, &mut ps, now_ms) {
                    log::error!("ticket {}: {e}", t.id);
                }
                continue;
            }
            if !t.active() {
                continue;
            }
            let p = match self.pipeline_of(t) {
                Ok(p) => p,
                Err(e) => {
                    self.park(
                        t,
                        &mut ps,
                        &format!("pipeline copy unreadable: {e}"),
                        now_ms,
                    )?;
                    continue;
                }
            };
            let has_open = t.attempts.iter().any(Attempt::is_open);
            // Watching what runs is free, and so is a gate-only stage
            // (a lanes choice, a PR read) or closing a ticket past its
            // last stage: they launch nothing. Starting
            // an agent takes a slot and is refused while too much waits
            // on the user.
            let gate_only = p
                .stages
                .get(t.stage)
                .is_none_or(|s| s.kind() == StageKind::GateOnly);
            let may_start = has_open
                || gate_only
                || (running < p.policy.slots && pending < p.policy.waiting_on_me);
            if !may_start {
                continue;
            }
            let had_slot = t.attempts.iter().any(costs_slot);
            if let Err(e) = self.step(t, &mut ps, &p, now_ms) {
                log::error!("ticket {}: {e}", t.id);
            }
            if !had_slot && t.attempts.iter().any(costs_slot) {
                running += 1;
            }
            pipeline = Some(p);
        }
        if let Some(p) = pipeline {
            let refreshed: Vec<Ticket> = ps
                .queue
                .iter()
                .filter_map(|id| self.load_ticket(id).ok())
                .collect();
            if let Err(e) = crate::view::sync_queue(self, &mut ps, &p, &refreshed, now_ms) {
                log::warn!("queue view: {e}");
            }
        }
        self.save_project(&ps)
    }

    /// Every project with a state file.
    pub fn projects(&self) -> Result<Vec<String>> {
        let dir = self.data.root.join("projects");
        let mut names = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                // A project whose primary file is gone still exists
                // through its backup, and a pass restores the primary.
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let stem = name
                    .strip_suffix(".json.bak")
                    .or_else(|| name.strip_suffix(".json"));
                if let Some(stem) = stem
                    && !names.iter().any(|n| n == stem)
                {
                    names.push(stem.to_owned());
                }
            }
        }
        names.sort();
        Ok(names)
    }

    /// One pass over every project.
    pub fn step_all(&mut self, now_ms: u64) -> Result<()> {
        for project in self.projects()? {
            if let Err(e) = self.step_project(&project, now_ms) {
                log::error!("project {project}: {e}");
            }
        }
        Ok(())
    }

    /// Write an answer on a pending decision; the runner acts on it.
    pub fn decide(
        &self,
        ticket: &str,
        decision: &str,
        answer: &str,
        note: Option<&str>,
        now_ms: u64,
    ) -> Result<Decision> {
        let path = self.data.ticket_file(ticket);
        self.data.with_lock(|| {
            let mut t: Ticket = read_json(&path)?;
            let d = t
                .decisions
                .iter_mut()
                .find(|d| d.id == decision && d.pending())
                .with_context(|| format!("ticket {ticket} has no pending decision {decision}"))?;
            if !d.options.iter().any(|o| o == answer) && d.name != "lanes" {
                bail!("decision {decision} takes one of: {}", d.options.join(", "));
            }
            d.state = DecisionState::Answered {
                answer: answer.into(),
                note: note.map(str::to_owned),
                by: BY_HAND.into(),
                at_ms: now_ms,
                acted: false,
            };
            let d = d.clone();
            t.updated_ms = now_ms;
            write_json(&path, &t)?;
            Ok(d)
        })
    }
}

impl Runner {
    /// A parked ticket back to active; the runner takes it from its
    /// current stage on its next pass. Nothing else is a resume.
    pub fn resume(&self, ticket: &str, now_ms: u64) -> Result<Ticket> {
        let path = self.data.ticket_file(ticket);
        self.data.with_lock(|| {
            let mut t: Ticket = read_json(&path)?;
            let TicketState::Parked { reason } = &t.state else {
                bail!("ticket {ticket} is not parked");
            };
            log::info!("ticket {ticket} resumed (was parked: {reason})");
            t.state = TicketState::Active;
            t.updated_ms = now_ms;
            write_json(&path, &t)?;
            Ok(t)
        })
    }

    /// `send` for the queue view, which is not an attempt's.
    pub fn send_for_view(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        intent: &str,
        body: Body,
        now_ms: u64,
    ) -> Result<Reply> {
        self.send(t, ps, None, intent, body, now_ms)
    }
}

/// A fresh attempt record.
/// Switchboard's reply to a session query for a record it does not
/// have; the one failure that means the session is gone.
const NO_SUCH_SESSION: &str = "no such session";

/// The artifacts an attempt was to write that are not files yet.
/// An open attempt with an agent or a review run in it: what the
/// policy's `slots` count. A gate-only attempt launches nothing.
pub(crate) fn costs_slot(a: &Attempt) -> bool {
    a.is_open() && a.kind != AttemptKind::GateOnly
}

/// The PR is read again on the next pass instead of after the poll
/// interval.
fn recheck_pr(t: &mut Ticket, attempt: Option<&(String, u32)>) {
    if let Some((stage, n)) = attempt
        && let Some(a) = t
            .attempts
            .iter_mut()
            .find(|a| &a.stage == stage && a.n == *n)
        && let Some(pr) = &mut a.pr
    {
        pr.checked_ms = 0;
    }
}

/// The key a sent-back note is kept under.
fn rework_key(stage: &str, ctx: &str) -> String {
    format!("{stage}/{ctx}")
}

/// The tree a context runs in: the lane's worktree, or the ticket's.
fn tree_of(t: &Ticket, p: &Pipeline, ctx: &str) -> Option<PathBuf> {
    t.lanes
        .iter()
        .find(|l| l.name == ctx)
        .map(|l| l.worktree.clone())
        .or_else(|| primary_tree(t, p))
}

/// The record behind a copy of an attempt the runner is polling.
fn attempt_mut<'t>(t: &'t mut Ticket, a: &Attempt) -> &'t mut Attempt {
    t.attempts
        .iter_mut()
        .find(|x| x.stage == a.stage && x.n == a.n)
        .expect("the attempt polled exists")
}

/// One context's poll of a PR-reading stage.
#[derive(Clone, Copy)]
struct PrPoll<'a> {
    stage: &'a Stage,
    decision: &'a str,
    attempt: &'a Attempt,
    cwd: &'a Path,
    lane: Option<&'a str>,
}

/// Where a `pr-checks` gate looks: the provider, its name for the
/// repository, and the branch.
struct PrTarget {
    provider: String,
    repo: String,
    branch: String,
}

impl PrTarget {
    fn record(&self, head: &str, now_ms: u64) -> PullRequestRecord {
        PullRequestRecord {
            provider: self.provider.clone(),
            repo: self.repo.clone(),
            number: 0,
            url: String::new(),
            head: head.to_owned(),
            checks: String::new(),
            checked_ms: now_ms,
            error_since_ms: None,
        }
    }
}

/// The lane's remote as a provider knows it, or why the stage cannot
/// run (a reason to park: the pipeline names something not built).
fn pr_target(
    t: &Ticket,
    p: &Pipeline,
    stage: &Stage,
    lane: Option<&str>,
    provider: Option<&str>,
    origin: Option<String>,
) -> Result<PrTarget, String> {
    let lane_record = lane
        .and_then(|l| t.lanes.iter().find(|x| x.name == l))
        .or_else(|| t.lanes.first())
        .ok_or_else(|| format!("stage {} needs a lane with a branch", stage.name))?;
    let Some(remote) = p
        .lane(&lane_record.name)
        .and_then(|l| l.repo.clone())
        .or_else(|| p.project.repo.clone())
        .or(origin)
    else {
        return Err(format!(
            "stage {} reads a pull request, and the tree has no remote; use a human gate",
            stage.name
        ));
    };
    let provider = provider.map_or_else(
        || {
            if github_repo(&remote).is_some() {
                "github".to_owned()
            } else if bitbucket_repo(&remote).is_some() {
                "bitbucket".to_owned()
            } else {
                "unknown".to_owned()
            }
        },
        str::to_owned,
    );
    let repo = match provider.as_str() {
        "github" => github_repo(&remote),
        "bitbucket" => bitbucket_repo(&remote),
        other => {
            return Err(format!(
                "stage {} reads pull requests from {other}, which is not built",
                stage.name
            ));
        }
    };
    let Some(repo) = repo else {
        return Err(format!(
            "stage {}: {remote:?} is not a {provider} repository",
            stage.name
        ));
    };
    Ok(PrTarget {
        provider,
        repo,
        branch: lane_record.branch.clone(),
    })
}

/// Whether two hashes name one commit; a provider may report a short
/// one.
fn same_commit(a: &str, b: &str) -> bool {
    let n = a.len().min(b.len());
    n >= 7 && a[..n].eq_ignore_ascii_case(&b[..n])
}

/// What a reading of the PR means: its summary for the record, and
/// pass (`Ok(true)`), wait (`Ok(false)`) or a question.
fn judge_pr(
    pr: &crate::github::PullRequest,
    checks: Option<&Checks>,
    head: &str,
    none_expected: bool,
) -> (String, Result<bool, String>) {
    let summary = match (pr.state.as_str(), checks) {
        ("merged", _) => "merged".to_owned(),
        ("closed", _) => "closed".to_owned(),
        (_, None | Some(Checks::None)) => "none".to_owned(),
        (_, Some(Checks::Pending)) => "pending".to_owned(),
        (_, Some(Checks::Passed)) => "passed".to_owned(),
        (_, Some(Checks::Failed(names))) => format!("failed: {}", names.join(", ")),
    };
    let short = |h: &str| h.chars().take(8).collect::<String>();
    let verdict = match summary.as_str() {
        "merged" => Ok(true),
        "closed" => Err(format!("PR #{} is closed without being merged", pr.number)),
        _ if !same_commit(&pr.head, head) => Err(format!(
            "PR #{} is at {} but the tree is at {}; push the branch, then answer recheck",
            pr.number,
            short(&pr.head),
            short(head)
        )),
        "none" if none_expected => Ok(true),
        "none" => Err(format!(
            "PR #{} has no checks configured; add a workflow, or set checks = \"none\" on the stage",
            pr.number
        )),
        "pending" => Ok(false),
        "passed" => Ok(true),
        failed => Err(format!("PR #{} checks {failed}", pr.number)),
    };
    (summary, verdict)
}

fn missing_artifacts(attempt: &Attempt) -> Vec<String> {
    attempt
        .artifacts
        .iter()
        .filter(|(_, path)| !path.is_file())
        .map(|(name, _)| name.clone())
        .collect()
}

/// The number of a stage's next attempt: one more than any attempt of
/// the stage in any context, so `(stage, n)` names one attempt even in
/// an `each` stage where lanes run side by side, which is how the
/// ledger, decisions and the runner's own lookups identify one.
fn next_n(t: &Ticket, stage: &str) -> u32 {
    t.attempts_of(stage).map(|a| a.n).max().unwrap_or(0) + 1
}

/// The key a check is polled under: one per attempt.
fn gate_key(t: &Ticket, a: &Attempt) -> String {
    format!("{}/{}/{}", t.id, a.stage, a.n)
}

fn new_attempt(
    stage_name: &str,
    n: u32,
    ctx: &str,
    kind: AttemptKind,
    at: AttemptState,
    artifacts: BTreeMap<String, PathBuf>,
    now_ms: u64,
) -> Attempt {
    let done = at == AttemptState::Complete;
    Attempt {
        stage: stage_name.to_owned(),
        n,
        context: ctx.to_owned(),
        kind,
        state: at,
        project: None,
        session: None,
        run: None,
        artifacts,
        settle: BTreeMap::new(),
        stop_at_ms: None,
        polls_since_stop: 0,
        head: None,
        gate: None,
        pr: None,
        started_ms: now_ms,
        ended_ms: done.then_some(now_ms),
    }
}

/// One more look at every artifact: true once each has looked the same
/// for `SETTLE_POLLS` polls in a row.
fn settle(attempt: &mut Attempt) -> Result<bool> {
    let mut all = true;
    for (name, path) in &attempt.artifacts {
        let meta = std::fs::metadata(path)?;
        let mtime_ms = crate::epoch_ms(meta.modified()?);
        let len = meta.len();
        let entry = attempt.settle.entry(name.clone()).or_insert(Settle {
            mtime_ms,
            len,
            polls: 0,
        });
        if entry.mtime_ms == mtime_ms && entry.len == len {
            entry.polls += 1;
        } else {
            *entry = Settle {
                mtime_ms,
                len,
                polls: 1,
            };
        }
        if entry.polls < SETTLE_POLLS {
            all = false;
        }
    }
    Ok(all)
}

/// The definition a reviewer operator installs, named by its content so
/// an edited operator is a new name and a run in flight keeps its own.
/// The reviewer's definition with Dispatch's variables (`{worktree}`,
/// `{branch}`, `{project.root}`, `{inputs.*}`) rendered into its
/// templates; Switchboard's own (`{plan}`, `{feedback}`, ...) are left
/// for it. The reviewer works in the attempt directory, so the templates
/// are where it learns which repository the plan is about. The name's
/// hash is of the rendered text, so each worktree gets its own.
fn definition_of(
    reviewer_name: &str,
    review: crate::pipeline::Review,
    vars: &Vars,
) -> wire::Definition {
    // A template that names neither the tree nor the root gets told
    // where the repository is; the reviewer's cwd is the attempt
    // directory, and "CLAUDE.md" means nothing there.
    let tree = vars
        .0
        .get("worktree")
        .or_else(|| vars.0.get("project.root"))
        .cloned();
    let with_repo = |text: &str| {
        let rendered = vars.render(text);
        match &tree {
            Some(tree) if !text.contains("{worktree}") && !text.contains("{project.root}") => {
                format!(
                    "The repository this is about is at {tree}; paths like CLAUDE.md are relative to it. {rendered}"
                )
            }
            _ => rendered,
        }
    };
    let review = crate::pipeline::Review {
        reviewer: review.reviewer,
        review_first: with_repo(&review.review_first),
        review_round: with_repo(&review.review_round),
        respond: vars.render(&review.respond),
        respond_to_user: vars.render(&review.respond_to_user),
        handoff: vars.render(&review.handoff),
        no_feedback: review.no_feedback,
        cap: review.cap,
    };
    let text = serde_json::to_string(&review).unwrap_or_default();
    wire::Definition {
        name: format!("Dispatch: {reviewer_name}@{}", Pipeline::fingerprint(&text)),
        reviewer: match review.reviewer {
            crate::pipeline::OperatorKind::Claude => wire::AgentKind::Claude,
            crate::pipeline::OperatorKind::Codex => wire::AgentKind::Codex,
        },
        review_first: review.review_first,
        review_round: review.review_round,
        respond: review.respond,
        respond_to_user: review.respond_to_user,
        handoff: review.handoff,
        no_feedback: review.no_feedback,
        cap: review.cap,
    }
}

/// The subject of a review and the session that wrote it, from the
/// latest complete attempt: the review clones that session's
/// conversation for the planner.
fn review_subject(t: &Ticket, subject: &str) -> Result<(PathBuf, String), String> {
    let (path, session) = t
        .attempts
        .iter()
        .rev()
        .filter(|a| a.state == AttemptState::Complete)
        .find_map(|a| {
            a.artifacts
                .get(subject)
                .map(|path| (path.clone(), a.session.clone()))
        })
        .ok_or_else(|| format!("nothing wrote {subject}"))?;
    let session = session.ok_or_else(|| format!("{subject} was not written by a session"))?;
    Ok((path, session))
}

/// The ticket's own tree: its first lane's worktree, or the project's
/// root for a project that works in place. None before the cut.
fn session_kind(kind: crate::pipeline::OperatorKind) -> wire::SessionKind {
    match kind {
        crate::pipeline::OperatorKind::Claude => wire::SessionKind::Claude,
        crate::pipeline::OperatorKind::Codex => wire::SessionKind::Codex,
    }
}

fn primary_tree(t: &Ticket, p: &Pipeline) -> Option<PathBuf> {
    if p.cuts_worktrees() {
        t.tree
            .clone()
            .or_else(|| t.lanes.first().map(|l| l.worktree.clone()))
    } else {
        p.project.root.clone()
    }
}

/// Whether a failed attempt's rerun was authorised and its cleanup done:
/// the answer is marked acted only once the replaced attempt's
/// processes were confirmed gone, and that mark is on disk.
fn may_rerun(t: &Ticket, failed: &Attempt) -> bool {
    t.decisions.iter().any(|d| {
        d.name == "rerun"
            && d.attempt.as_ref() == Some(&(failed.stage.clone(), failed.n))
            && matches!(
                &d.state,
                DecisionState::Answered { answer, acted: true, .. } if answer == "rerun"
            )
    })
}

/// The lanes the issue's labels suggest, per the source's hints.
fn lane_hints(p: &Pipeline, labels: &[String]) -> Vec<String> {
    match &p.source {
        crate::pipeline::Source::Github { lane_hints, .. } => labels
            .iter()
            .filter_map(|l| lane_hints.get(l).cloned())
            .collect(),
        _ => Vec::new(),
    }
}

/// The prompt's fields for a ticket in a context.
fn vars_for(t: &Ticket, p: &Pipeline, lane: Option<&str>) -> Vars {
    let mut vars = Vars::default();
    vars.set("ticket", t.id.clone())
        .set("issue.number", t.source.number.unwrap_or(0).to_string())
        .set("issue.title", t.source.title.clone())
        .set("issue.body", t.source.body.clone())
        .set("issue.url", t.source.url.clone().unwrap_or_default())
        .set("task.text", t.source.title.clone())
        .set("task.context", t.source.body.clone())
        .set(
            "project.root",
            primary_tree(t, p).map_or(String::new(), |d| d.display().to_string()),
        )
        .set(
            "lanes",
            t.lanes
                .iter()
                .map(|l| l.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        );
    // The root context is the ticket's tree on the ticket's branch.
    if let Some(l) = lane.and_then(|name| t.lanes.iter().find(|x| x.name == name)) {
        vars.set("lane", l.name.clone())
            .set("branch", l.branch.clone())
            .set("worktree", l.worktree.display().to_string());
    } else if let Some(tree) = primary_tree(t, p) {
        vars.set("worktree", tree.display().to_string());
        if p.cuts_worktrees() {
            vars.set(
                "branch",
                branch_name(t.source.number.unwrap_or(0), &t.source.title),
            );
        }
    }
    let names: std::collections::BTreeSet<&String> =
        t.attempts.iter().flat_map(|a| a.artifacts.keys()).collect();
    for name in names {
        if let Some(path) = t.input(name) {
            vars.set(format!("inputs.{name}"), path.display().to_string());
        }
    }
    vars
}

/// What a command run for a ticket gets in its environment.
fn env_for(t: &Ticket, lane: Option<&str>, branch: Option<&str>) -> Vec<(String, String)> {
    let mut env = vec![("DISPATCH_TICKET".to_owned(), t.id.clone())];
    if let Some(l) = lane {
        env.push(("DISPATCH_LANE".to_owned(), l.to_owned()));
    }
    if let Some(b) = branch {
        env.push(("DISPATCH_BRANCH".to_owned(), b.to_owned()));
    }
    env
}

/// The records a reply made, applied to the ticket by what the request
/// was for. Recovery uses this too, with a reply rebuilt from `find`.
pub fn apply_reply(t: &mut Ticket, ps: &mut ProjectState, intent: &str, reply: &Reply) {
    let made = reply.made();
    let first = |kind: wire::RecordKind| made.iter().find(|m| m.kind == kind).map(|m| m.id.clone());
    match intent {
        "space" => {
            if let Some(id) = first(wire::RecordKind::Space) {
                ps.space = Some(id);
            }
        }
        "set" => {
            if let Some(id) = first(wire::RecordKind::Set) {
                ps.set = Some(id);
            }
        }
        "root-project" => {
            if let Some(id) = first(wire::RecordKind::Project) {
                t.root_project = Some(id);
            }
        }
        "session" | "run" => {
            let Some(op) = t.ledger.iter().rev().find(|o| o.intent == intent) else {
                return;
            };
            let Some((stage, n)) = op.attempt.clone() else {
                return;
            };
            let Some(attempt) = t.attempts.iter_mut().find(|a| a.stage == stage && a.n == n) else {
                return;
            };
            if intent == "session" {
                if let Some(id) = first(wire::RecordKind::Session) {
                    attempt.session = Some(id.clone());
                    attempt.state = AttemptState::Running;
                    t.processes.push(id);
                }
            } else {
                if let Some(id) = first(wire::RecordKind::Run) {
                    attempt.run = Some(id);
                    attempt.state = AttemptState::Running;
                }
                for m in made.iter().filter(|m| m.kind == wire::RecordKind::Session) {
                    t.processes.push(m.id.clone());
                }
            }
        }
        other => {
            if let Some(lane) = other.strip_prefix("lane-project:")
                && let Some(id) = first(wire::RecordKind::Project)
                && let Some(l) = t.lanes.iter_mut().find(|l| l.name == lane)
            {
                l.project = Some(id);
            }
        }
    }
}
