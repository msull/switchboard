//! One step of one ticket. The runner reads what Switchboard says about
//! the ticket's sessions and runs, decides, and acts through the port,
//! writing the ticket before and after every request. Nothing here
//! retries on its own: a failure is a decision.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow, bail};
use dispatch_control as wire_dispatch;
use switchboard_control::{self as wire, Body, Reply, Request, RunState};

use crate::UsageError;
use crate::bitbucket::{Bitbucket, bitbucket_repo};
use crate::git::{Adopted, Confine, Network, Push, Repo, branch_name, yyyymmdd};
use crate::github::{Checks, Gh, PullRequests, github_repo};
use crate::health::Health;
use crate::pipeline::{Context, Gate, Lane, OnDirty, Pipeline, Stage, StageKind, Write, env_key};
use crate::port::Port;
use crate::review::checks_key;
use crate::store::{
    DataDir, Lock, Settings, expand_home, read_project, read_ticket, shell_unsafe, write_project,
    write_ticket_stamped,
};
use crate::template::Vars;
use crate::ticket::{
    Attempt, AttemptKind, AttemptState, CheckGroup, CloseProgress, Decision, DecisionKind,
    DecisionState, GateRun, LaneRecord, Operation, OrphanKill, ProjectState, PullRequestRecord,
    PullRequestSource, RefreshConflict, Refreshed, SETTLE_POLLS, STOP_IDLE_POLLS, STUCK,
    ServiceState, Settle, SourceSnapshot, Ticket, TicketState,
};

/// How often a `pr-checks` gate reads the provider.
pub const PR_POLL_MS: u64 = 60_000;
/// How long lookups may keep failing before the gate asks.
pub const PR_ERROR_GRACE_MS: u64 = 3_600_000;
/// How long after the tree's head moves a `none` reading waits rather
/// than asks: GitHub creates a new head's check runs a little after the
/// push, so for a minute or so a repository with CI reads as one without.
pub const PR_YOUNG_HEAD_MS: u64 = 120_000;

/// The pseudo-stage a refresh rebaser's attempts and questions carry:
/// not in any pipeline, so no stage mistakes them for its own.
pub const REFRESH: &str = "refresh";

/// The pseudo-stage of the one review pass a conflicted bring-up gets
/// after the pipeline's last code review stage, and of its question: not
/// in any pipeline, so no stage mistakes them for its own.
pub const RESOLUTION: &str = dispatch_control::RESOLUTION;

/// The pseudo-stage the cut's questions carry: not in any pipeline, so
/// no stage mistakes them for its own.
pub const CUT: &str = "cut";

/// How long a killed check may take to read as gone before a park or a
/// close sends SIGKILL to its group and goes on, saying so in the
/// cancellation reason: well past the seconds a test run takes to exit
/// on TERM.
pub const STOP_LIMIT_MS: u64 = 120_000;

/// The line typed into an agent's session when it stops with its tree
/// not clean, before the question.
pub const NUDGE_TEXT: &str = "The stage is not done: the tree is not clean. Commit your work, or revert anything that is not part of it, then stop.";

/// The ledger intent of a nudge. Recorded on the attempt before it is
/// sent, so recovery neither repeats it nor asks about it.
pub(crate) const NUDGE: &str = "nudge";

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
    /// Set when `unlocked` could not take the writer lock back inside a
    /// transaction: what the transaction read before is stale, so every
    /// write is refused until it ends.
    lock_lost: bool,
    /// The disk hold as last logged, so a full disk is one line, not
    /// one a second.
    low_disk: Option<String>,
    /// Checks a park, a close or a lost check's restart killed and is
    /// waiting on, by key: when the first kill was sent. Not saved: a
    /// restarted runner rebuilds an entry from the gate's recorded
    /// group, with the time of that group's first kill.
    stopping: BTreeMap<String, u64>,
    /// What this runner's calls to Switchboard and the providers did,
    /// for `runner.json`. Kept in memory only. A `RefCell` because the
    /// provider calls run in `&self` methods that hold a borrow of
    /// `self` while they call; each use is one statement that never
    /// holds the borrow across another call.
    pub health: RefCell<Health>,
    /// Who the commands run through this runner are for: `supervisor`
    /// when a project's supervisor session ran them, `None` for the
    /// owner, the port and the runner itself. Stamped on what they take,
    /// answer, park, resume and close.
    pub actor: Option<String>,
    /// The resource each ticket was last logged waiting for, by ticket
    /// id, so a wait is one line, not one a second.
    pub(crate) held_back: BTreeMap<String, String>,
}

/// Where a stop of an attempt's checks stands: a park's, a close's, or
/// one before lost checks start again.
pub(crate) enum GateStop {
    /// Nothing of the checks is left, or none were running.
    Gone,
    /// Killed, and something of them still runs within the limit.
    Waiting,
    /// Still running at the limit, so stepped up to SIGKILL: the clause
    /// the cancellation reason carries.
    OverLimit(String),
}

/// The contexts a stage runs in: `(name, cwd, lane)`.
pub(crate) type Contexts = Vec<(String, PathBuf, Option<String>)>;

/// What one ticket's step changed in the project's counts: a slot
/// taken, and questions asked without one.
struct Stepped {
    took_slot: bool,
    asked: u32,
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
            lock_lost: false,
            low_disk: None,
            stopping: BTreeMap::new(),
            health: RefCell::new(Health::default()),
            actor: None,
            held_back: BTreeMap::new(),
        }
    }

    /// Whole gigabytes free on the volume that holds the worktrees;
    /// `None` when that cannot be read, which holds nothing back.
    #[must_use]
    pub fn free_gb(&self) -> Option<u32> {
        let mut dir = self.data.worktrees_dir();
        while !dir.exists() {
            dir = dir.parent()?.to_path_buf();
        }
        match self.git.free_bytes(&dir) {
            Ok(bytes) => Some(u32::try_from(bytes / 1_000_000_000).unwrap_or(u32::MAX)),
            Err(e) => {
                log::warn!("free space at {} unreadable: {e:#}", dir.display());
                None
            }
        }
    }

    /// Why a full disk holds new starts for `policy`, if it does; logged
    /// when it begins and when it ends.
    fn disk_hold(&mut self, policy: &crate::pipeline::Policy, free_gb: Option<u32>) -> bool {
        let hold = free_gb
            .filter(|free| *free < policy.min_free_gb)
            .map(|free| {
                format!(
                    "{free} GB free on the worktrees' volume, the policy wants {}",
                    policy.min_free_gb
                )
            });
        if hold != self.low_disk {
            match &hold {
                Some(why) => log::warn!("nothing new starts: {why}"),
                None => log::info!("free space is back above the policy's floor"),
            }
            self.low_disk.clone_from(&hold);
        }
        hold.is_some()
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
        self.lock_lost = false;
        result
    }

    /// Run slow external work with the writer lock let go, and take the
    /// lock again before returning if it was held. The work gets only
    /// the repository: `self` is borrowed by this call, so the compiler
    /// keeps the closure from reaching the runner's saves. It must touch
    /// no record either way, and the caller reads what it needs again
    /// once this returns. Paths are computed before the call. A lock
    /// that cannot be taken back fails the rest of the transaction:
    /// a later save would otherwise take the lock for itself and write
    /// over whatever landed in the gap.
    fn unlocked<T>(&mut self, slow: impl FnOnce(&mut dyn Repo) -> T) -> Result<T> {
        let was_held = self.held.take().is_some();
        let out = slow(&mut *self.git);
        if was_held {
            match self.data.lock() {
                Ok(lock) => self.held = Some(lock),
                Err(e) => {
                    self.lock_lost = true;
                    return Err(e);
                }
            }
        }
        Ok(out)
    }

    fn write_record(&self, write: impl FnOnce() -> Result<()>) -> Result<()> {
        if self.lock_lost {
            bail!("the writer lock was lost mid-transaction; nothing more is written");
        }
        if self.held.is_some() {
            write()
        } else {
            self.data.with_lock(write)
        }
    }
}

/// What `dispatch decide` writes as the answerer.
pub const BY_HAND: &str = "you";
/// Who answered a decision Dispatch resolved from a provider.
pub const BY_DISPATCH: &str = "dispatch";
/// Who answered the `rerun` a resume authorised for an attempt its park
/// cancelled mid-run.
pub const BY_RESUME: &str = "resume";
/// Who answered, parked, resumed, closed or took when the command came
/// from a project's supervisor session.
pub const BY_SUPERVISOR: &str = "supervisor";

/// Every decision Dispatch asks of its own accord, by name, beside the
/// ones a pipeline's gates name. A supervisor's `decides` may list
/// these; a test holds the list to the ask sites.
pub const DECISIONS: &[&str] = &[
    "finalize",
    "paused",
    "rerun",
    "pr",
    "branch",
    "lanes",
    REFRESH,
    "review-cap",
    "review-code",
    "message",
    RESOLUTION,
    "lost-send",
];

/// The decision a `pr-merged` gate asks when it names none.
pub const DEFAULT_MERGE: &str = "merge";

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

/// A lane the cut has yet to record: its index in the pipeline's lanes,
/// its pull request for a ticket from pull requests, and its branch.
struct LaneCut {
    index: usize,
    pr: Option<PullRequestSource>,
    branch: String,
}

/// A context the cut has yet to make: the ticket's tree or a lane with
/// a repository of its own, with everything its clone and worktree
/// need.
struct Uncut {
    what: String,
    url: String,
    clone: PathBuf,
    remote: String,
    dir: PathBuf,
    branch: String,
    start: String,
    pr: Option<PullRequestSource>,
    extra: Option<(String, String)>,
}

/// What the cut does with a branch an earlier ticket left with commits,
/// as the `branch` decision answered: check it out as it is, or rename
/// it to the name given and cut a new one.
#[derive(Debug, Clone)]
enum Existing {
    Reuse,
    Fresh(String),
}

impl Runner {
    // --- records

    pub fn load_ticket(&self, id: &str) -> Result<Ticket> {
        read_ticket(&self.data.ticket_file(id))
    }

    pub fn save_ticket(&self, t: &mut Ticket, now_ms: u64) -> Result<()> {
        self.write_record(|| write_ticket_stamped(&self.data, t, now_ms))
    }

    pub fn load_project(&self, name: &str) -> Result<ProjectState> {
        let path = self.data.project_file(name);
        if crate::store::record_exists(&path) {
            read_project(&path)
        } else {
            Ok(ProjectState {
                name: name.to_owned(),
                ..ProjectState::default()
            })
        }
    }

    pub fn save_project(&self, ps: &ProjectState) -> Result<()> {
        let path = self.data.project_file(&ps.name);
        self.write_record(|| write_project(&path, ps))
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
            .map(|p| read_ticket(p))
            .collect()
    }

    // --- take

    /// Make the ticket: its record, its copy of the pipeline, its place
    /// at the end of the queue. Nothing is asked of Switchboard yet.
    pub fn take(
        &mut self,
        project: &str,
        pipeline_text: &str,
        mut source: SourceSnapshot,
        now_ms: u64,
    ) -> Result<Ticket> {
        source.taken_by.clone_from(&self.actor);
        let pipeline = Pipeline::parse(pipeline_text)?;
        if pipeline.project.name != project {
            bail!(
                "the pipeline file names project {:?}, not {project:?}",
                pipeline.project.name
            );
        }
        self.transaction(|r| r.take_locked(project, &pipeline, pipeline_text, source, now_ms))
    }

    fn take_locked(
        &mut self,
        project: &str,
        pipeline: &Pipeline,
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
                    existing.state.label()
                );
            }
        }
        // The tree's path reaches the repository's own tooling; one it
        // may not survive is refused before anything is made.
        if pipeline.cuts_worktrees()
            && let Some(why) = shell_unsafe(&self.worktree_root(pipeline))
        {
            bail!("worktrees: {why}; set another with `dispatch worktrees <path>`");
        }
        let id = Ticket::new_id();
        let dir = self.data.ticket_dir(&id);
        std::fs::create_dir_all(&dir)?;
        let pipeline_file = dir.join("pipeline.toml");
        crate::store::atomic_write(&pipeline_file, pipeline_text.as_bytes())?;
        let mut ticket = Ticket {
            version: crate::store::RECORD_VERSION,
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
            refreshed_stage: None,
            state: TicketState::Active,
            state_by: None,
            close: CloseProgress::default(),
            restarts: Vec::new(),
            restart: None,
            entered: Vec::new(),
            holds: Vec::new(),
            services: Vec::new(),
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
    pub(crate) fn worktree_root(&self, p: &Pipeline) -> PathBuf {
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
                if matches!(
                    t.state,
                    TicketState::Closing { .. } | TicketState::Closed { .. }
                ) || tree.parent() == Some(&*root)
                {
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
                let lane_clone = self.data.lane_repo_dir(&p.project.name, &lane.name);
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
    pub(crate) fn send(
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
        t.ledger
            .push(Operation::new(op.clone(), &body, attempt, intent, now_ms));
        self.save_ticket(t, now_ms)?;
        let result = self.call(Some(&t.id), &Request::new(op.clone(), body));
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
        result
            .map_err(SocketDown::from)
            .with_context(|| format!("{intent}: the control socket failed"))
    }

    /// A query: never in the ledger, since it changes nothing.
    pub(crate) fn ask(&mut self, body: Body) -> Result<Reply> {
        self.call(
            None,
            &Request::new(format!("q-{}", uuid::Uuid::new_v4().simple()), body),
        )
        .map_err(SocketDown::from)
        .context("the control socket failed")
    }

    /// One request to Switchboard, timed and counted in `health` for
    /// `ticket`, or the ticket being stepped when `None`. Every call to
    /// the port goes through here rather than through a wrapper of the
    /// port, which a test that swaps `port` would bypass.
    pub(crate) fn call(
        &mut self,
        ticket: Option<&str>,
        request: &Request,
    ) -> std::io::Result<Reply> {
        let started = std::time::Instant::now();
        let result = self.port.call(request);
        self.health.borrow_mut().port_call(ticket, started, &result);
        result
    }

    // --- one step

    /// A request whose reply never came (the socket failed, or a
    /// restart) is resolved before anything else is asked. Recovery
    /// sends only queries and idempotent requests, so it launches
    /// nothing.
    fn recover_unanswered(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        now_ms: u64,
    ) -> Result<()> {
        let pending = t.unsettled();
        let recovered = !pending.is_empty();
        for i in pending {
            self.recover_one(t, ps, i, now_ms)?;
        }
        // A pass that changes nothing else must still keep the reply.
        if recovered {
            self.save_ticket(t, now_ms)?;
        }
        Ok(())
    }

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
        self.recover_unanswered(t, ps, now_ms)?;
        self.fail_stranded(t, ps, now_ms)?;
        self.act_on_answers(t, ps, p, now_ms)?;
        if !t.active() {
            return Ok(());
        }
        // A `waiting off` lost on the socket, or held back behind an
        // older request never answered, is sent again until it lands.
        if t.pending_decisions().is_empty() && marks_unsettled(t) {
            self.unmark(t, ps, now_ms)?;
        }
        // The ticket's trees come before any stage: a worktree of
        // Dispatch's clone on the ticket's branch, cut from what the
        // remote has now, and every lane inside it. The user's checkout
        // is never involved.
        if p.cuts_worktrees() && (t.tree.is_none() || t.lanes.len() < lanes_wanted(t, p)) {
            self.cut_trees(t, ps, p, now_ms)?;
            // A cut held by a question leaves the ticket active with its
            // trees not all there; no stage runs until they are.
            if !t.active() || t.tree.is_none() || t.lanes.len() < lanes_wanted(t, p) {
                return Ok(());
            }
        }
        // The first stage's entry: the trees as cut, before anything ran.
        if t.stage == 0 && t.entered.is_empty() && t.attempts.is_empty() && t.tree.is_some() {
            self.record_entry(t, now_ms);
            self.save_ticket(t, now_ms)?;
        }
        // Services the ticket has left behind are stopped and the holds
        // its stage no longer needs dropped before anything else, so
        // neither a refresh question nor the pipeline's end keeps a
        // resource held.
        if self.release_left(t, ps, p, now_ms)? || !t.active() {
            return Ok(());
        }
        let Some(stage) = p.stages.get(t.stage).cloned() else {
            return self.begin_close(t, ps, "every stage is done", now_ms);
        };
        if self.refresh_lanes(t, ps, p, &stage, now_ms)? || !t.active() {
            return Ok(());
        }
        if self.resolution_passes(t, ps, p, now_ms)? || !t.active() {
            return Ok(());
        }
        // A stage for lanes the ticket did not choose has nothing to run
        // in: it advances with no attempt, no hold and no question.
        if Self::skipped(t, &stage) {
            log::info!(
                "ticket {} {}: none of its lanes is chosen; skipped",
                t.id,
                stage.name
            );
            return self.advance(t, now_ms);
        }
        if self.take_holds(t, ps, p, &stage, now_ms)? || !t.active() {
            return Ok(());
        }
        // Agent, workflow and review stages are held per context
        // (`held_in`); attempts still running are watched either way.
        match stage.kind() {
            StageKind::GateOnly => {
                // A gate-only stage's questions are about the whole
                // stage, so any pending one for it holds it.
                let held = t
                    .decisions
                    .iter()
                    .any(|d| d.pending() && d.stage == stage.name);
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
            StageKind::Agent => self.agent_stage(t, ps, p, &stage, now_ms)?,
            StageKind::Workflow => self.workflow_stage(t, ps, p, &stage, now_ms)?,
            StageKind::Review => self.review_stage(t, ps, p, &stage, now_ms)?,
        }
        Ok(())
    }

    /// Each lane's branch brought up to its base once per stage entry,
    /// so a plan that sat is not implemented, reviewed or readied on
    /// stale code. A branch with no commits of its own just moves; one
    /// with commits is rebased, and when that conflicts the policy's
    /// `rebaser` continues the lane's last agent to resolve it, up to
    /// `max_rebases`, then the user is asked. True while the stage must
    /// wait: a rebaser at work, or a question open. Pull-request tickets
    /// are someone else's branch and are left alone, as is a stage that
    /// launches nothing (`lanes`, a human look, the merge watch).
    fn refresh_lanes(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        now_ms: u64,
    ) -> Result<bool> {
        if !p.policy.refresh || !p.cuts_worktrees() || !t.source.pull_requests.is_empty() {
            return Ok(false);
        }
        let open = t
            .attempts
            .iter()
            .filter(|a| a.stage == REFRESH && a.is_open())
            .max_by_key(|a| a.n)
            .cloned();
        if let Some(a) = open {
            let lane = t.lanes.iter().find(|l| l.name == a.context).cloned();
            let Some(lane) = lane else {
                return Ok(true);
            };
            let shape = Self::refresh_stage(p, stage);
            let trust = p.policy.trust_folders;
            self.poll_agent(
                t,
                ps,
                p,
                &a,
                &shape,
                &lane.worktree,
                Some(&lane.name),
                trust,
                now_ms,
            )?;
            let still_open = t
                .attempts
                .iter()
                .any(|x| x.stage == REFRESH && x.n == a.n && x.is_open());
            if !still_open {
                // Whatever the rebaser did is read on the next pass.
                t.refreshed_stage = None;
                self.save_ticket(t, now_ms)?;
            }
            return Ok(true);
        }
        if t.decisions.iter().any(|d| d.pending() && d.name == REFRESH) {
            return Ok(true);
        }
        if t.refreshed_stage == Some(t.stage) {
            if drop_stale_conflicts(t) {
                self.save_ticket(t, now_ms)?;
            }
            return Ok(false);
        }
        // A rerun question about a rebaser carries the pseudo-stage. It
        // holds only a stage not yet refreshed: a rebaser stopping
        // clears the mark, so one asked in this stage always lands here,
        // while one an older runner left pending past a refreshed stage
        // is about a superseded attempt and must not freeze it.
        if t.decisions
            .iter()
            .any(|d| d.pending() && d.stage == REFRESH)
        {
            return Ok(true);
        }
        // A stage that holds a resource or serves lanes looks at what was
        // inspected and deployed; moving the branch under it would not.
        if (stage.kind() == StageKind::GateOnly && !reads_pr(stage))
            || !stage.needs.is_empty()
            || !stage.services.is_empty()
        {
            t.refreshed_stage = Some(t.stage);
            drop_stale_conflicts(t);
            self.save_ticket(t, now_ms)?;
            return Ok(false);
        }
        let mut waits = false;
        for i in 0..t.lanes.len() {
            if t.lanes[i].chosen && self.refresh_lane(t, ps, p, stage, i, now_ms)? {
                waits = true;
            }
        }
        if !waits {
            drop_stale_conflicts(t);
            // A stage held on a lane is not refreshed yet: whatever
            // releases the hold (an answer, a rebaser stopping, a park
            // and resume that withdrew the question) must find the lanes
            // still to be read.
            t.refreshed_stage = Some(t.stage);
        }
        self.save_ticket(t, now_ms)?;
        Ok(waits)
    }

    /// The review of each chosen lane's conflict resolution, when its
    /// last bring-up resolved a conflict after the pipeline's last code
    /// review stage and pushed nothing: one reviewer reads only what the
    /// resolution changed, before the stage's own work. True while one
    /// is open, failed or asked about, which holds the stage.
    fn resolution_passes(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        now_ms: u64,
    ) -> Result<bool> {
        if !t.source.pull_requests.is_empty() {
            return Ok(false);
        }
        let Some(shape) = resolution_stage(p) else {
            return Ok(false);
        };
        let mut holds = false;
        for i in 0..t.lanes.len() {
            let lane = t.lanes[i].clone();
            let Some(moved) = resolution_due(p, &lane) else {
                continue;
            };
            let last = resolution_of(t, &lane.name, moved).cloned();
            let (ctx, cwd) = (lane.name.as_str(), lane.worktree.as_path());
            match last {
                Some(a) if a.state == AttemptState::Complete => continue,
                Some(a) if a.is_open() => {
                    self.poll_review(t, ps, p, &shape, &a, cwd, Some(ctx), now_ms)?;
                }
                Some(a) => {
                    if !held_in(t, RESOLUTION, ctx) && may_rerun(t, &a) {
                        self.start_review(t, ps, p, &shape, ctx, cwd, Some(ctx), now_ms)?;
                    } else if asks_again(t, &a, false) {
                        self.ask_rerun(t, ps, &a, now_ms)?;
                    }
                }
                None => {
                    if !held_in(t, RESOLUTION, ctx) {
                        self.start_review(t, ps, p, &shape, ctx, cwd, Some(ctx), now_ms)?;
                    }
                }
            }
            holds = true;
            if !t.active() {
                break;
            }
        }
        Ok(holds)
    }

    /// A lane's branch the refresh rewrote, pushed when the lane has an
    /// open pull request, with a lease on the head the lane's records
    /// last saw, so a stage that reads the PR does not find it at the old
    /// head and ask for the one push there is. A lane without a PR pushes
    /// nothing. A refused or failed push, or a provider that cannot be
    /// read, is logged, not an error: the `pr` question about the head is
    /// the fallback. Returns the head the remote now holds when the push
    /// went through or found it there already.
    fn push_refreshed(
        &mut self,
        t: &Ticket,
        p: &Pipeline,
        i: usize,
        remote: &str,
    ) -> Result<Option<String>> {
        let lane = &t.lanes[i];
        let (id, name, branch) = (&t.id, &lane.name, &lane.branch);
        // The PR is looked up the way the pipeline's PR stage will look
        // it up, through that stage's provider.
        let Some((stage, provider)) = pr_stage(p) else {
            return Ok(None);
        };
        let Some(seen) = pr_head_seen(t, lane) else {
            return Ok(None);
        };
        let head = self.git.head(&lane.worktree)?;
        if same_commit(&head, &seen) {
            return Ok(None);
        }
        let origin = self.git.remote_url(&lane.worktree)?;
        let target = match pr_target(t, p, stage, Some(name), provider, origin) {
            Ok(target) => target,
            Err(why) => {
                log::info!("ticket {id} lane {name}: {why}; {branch} not pushed");
                return Ok(None);
            }
        };
        match self.find_pr(&target) {
            Ok(Some(pr)) if pr.state == "open" => {}
            Ok(_) => {
                log::info!(
                    "ticket {id} lane {name}: no open pull request for {branch}; nothing pushed"
                );
                return Ok(None);
            }
            Err(e) => {
                log::warn!(
                    "ticket {id} lane {name}: the pull request for {branch} could not be read, so it is not pushed: {e:#}"
                );
                return Ok(None);
            }
        }
        match self
            .git
            .push_with_lease(&lane.worktree, remote, branch, &seen)
        {
            Ok(Push::Pushed) => {
                log::info!("ticket {id} lane {name}: pushed {branch} over {seen} (lease)");
                Ok(Some(head))
            }
            Ok(Push::UpToDate) => {
                log::info!(
                    "ticket {id} lane {name}: {remote}'s {branch} already at {head}; nothing pushed"
                );
                Ok(Some(head))
            }
            Ok(Push::Refused) => {
                log::info!(
                    "ticket {id} lane {name}: {remote} moved off {seen}; {branch} not pushed, left to the pr question"
                );
                Ok(None)
            }
            Err(e) => {
                log::warn!("ticket {id} lane {name}: push of {branch} failed: {e:#}");
                Ok(None)
            }
        }
    }

    /// `push_refreshed`, and the head it left on the remote recorded
    /// on the lane as its `pushed`.
    fn record_push(
        &mut self,
        t: &mut Ticket,
        p: &Pipeline,
        i: usize,
        remote: &str,
        now_ms: u64,
    ) -> Result<()> {
        if let Some(head) = self.push_refreshed(t, p, i, remote)? {
            t.lanes[i].pushed = Some(crate::ticket::PushedHead {
                head,
                at_ms: now_ms,
            });
        }
        Ok(())
    }

    /// One lane's branch against its base; true when the stage must
    /// wait on a rebaser or a question for it.
    fn refresh_lane(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        i: usize,
        now_ms: u64,
    ) -> Result<bool> {
        let Some(lane) = p.lane(&t.lanes[i].name).cloned() else {
            return Ok(false);
        };
        let (clone, remote, onto) = self.lane_base_ref(p, &lane);
        let worktree = t.lanes[i].worktree.clone();
        // Mid-rebase, `HEAD` is not the branch, so nothing read from it
        // is the branch's, and the stage must not run in a tree someone
        // is rebasing, whether or not the base moved.
        if self.git.rebase_in_progress(&worktree)? {
            let why = format!(
                "is mid-rebase{}; finish it (git rebase --continue) or abort it by hand",
                self.at_head(&worktree)
            );
            return self.hold_lane(t, ps, stage, i, &why, now_ms);
        }
        self.git.fetch(&clone, &remote)?;
        let onto_sha = self.git.rev_parse(&clone, &onto)?;
        if t.lanes[i].base_sha.as_deref() == Some(onto_sha.as_str()) {
            return Ok(false);
        }
        // Read before anything moves. A detached `HEAD` is not the
        // branch: a rebase there would move the detached commit and
        // record a bring-up of the branch it never touched.
        let Some(head_before) = self.git.branch_head(&worktree, &t.lanes[i].branch)? else {
            let why = format!("is not on {}; check it out by hand", t.lanes[i].branch);
            return self.hold_lane(t, ps, stage, i, &why, now_ms);
        };
        let behind = self.git.behind(&worktree, &onto)?;
        // A tree with work in it (uncommitted edits, say) is left
        // alone this stage; the base moves under it and is read again
        // on the next stage, or on a recheck.
        if behind > 0 && !self.git.is_clean(&worktree)? {
            log::info!(
                "ticket {} lane {}: {behind} behind {onto} but the tree is not clean; left alone",
                t.id,
                lane.name
            );
            return Ok(false);
        }
        // A lane with no `base_sha` (its base unread at the cut, or cut
        // before the field) sits on its fork point from `onto`. It is
        // recorded while the lane is behind, before anything moves, so
        // the bring-up after a rebaser still reads it. A lane not behind
        // whose fork point is `onto` never moved and has nothing to
        // record; one whose fork point cannot be read is brought up from
        // an unknown base. Once a rebaser has run since the lane last
        // moved, a fork point still missing was unreadable before it,
        // and the one read now is the rebaser's work, so the base stays
        // unknown and that work is checked.
        if t.lanes[i].base_sha.is_none() && !rebased_since_moved(t, &t.lanes[i]) {
            let fork = self.git.merge_base(&worktree, "HEAD", &onto).ok();
            let never_moved = behind == 0 && fork.as_deref() == Some(onto_sha.as_str());
            t.lanes[i].base_sha = fork;
            if never_moved {
                self.save_ticket(t, now_ms)?;
                return Ok(false);
            }
        }
        // A branch with no commits of its own sits at `sat_on`, the
        // commit it was cut from or last moved to, or `onto` when that
        // is unknown; the head read above says whether it still does.
        let from = t.lanes[i].base_sha.clone().unwrap_or_default();
        let sat_on = if from.is_empty() { &onto_sha } else { &from };
        let commits = head_before != *sat_on;
        let brought_up = behind == 0 || self.git.rebase_onto(&worktree, &onto)?;
        if brought_up {
            log::info!(
                "ticket {} lane {}: base moved {from} -> {onto_sha}; branch brought up ({behind} behind)",
                t.id,
                lane.name
            );
            // Before the save, so a crash repeats the push rather than
            // skipping it.
            self.record_push(t, p, i, &remote, now_ms)?;
            // Nothing behind means a rebaser (or a hand rebase) already
            // did the work; its notes go to the next reviewer.
            let notes = if behind == 0 && commits {
                rebaser_notes(t, &lane.name, t.lanes[i].refreshed.as_ref())
            } else {
                None
            };
            let after = self.git.head(&worktree)?;
            let conflict = Self::resolved_conflict(t, p, i, &head_before);
            t.lanes[i].refreshed = Some(Refreshed {
                from,
                to: onto_sha.clone(),
                commits,
                notes,
                at_ms: now_ms,
                conflict,
                after: Some(after),
            });
            t.lanes[i].base_sha = Some(onto_sha);
            self.save_ticket(t, now_ms)?;
            return Ok(false);
        }
        self.record_conflict(t, i, &head_before, &from, &onto_sha, now_ms)?;
        // A conflict: the rebaser, within the cap, else a question.
        let tried = t
            .attempts
            .iter()
            .filter(|a| a.stage == REFRESH && a.context == lane.name)
            .count();
        let rebaser = p.policy.rebaser.clone();
        match rebaser {
            Some(operator) if tried < p.policy.max_rebases as usize => {
                self.start_refresh_rebaser(t, ps, p, i, &operator, &onto, now_ms)?;
            }
            _ => {
                let why = if rebaser.is_none() {
                    "the policy names no rebaser".to_owned()
                } else {
                    format!("max_rebases ({}) is spent", p.policy.max_rebases)
                };
                let question = format!(
                    "{} ({}): the branch is behind {onto} and a rebase onto it conflicts; {why}. Rebase it by hand in {}, then answer recheck",
                    stage.name,
                    lane.name,
                    worktree.display()
                );
                self.ask_refresh(t, ps, stage, question, now_ms)?;
            }
        }
        Ok(true)
    }

    /// Lane `i`'s worktree is in a state only its owner can put right
    /// (`why` says which and how): the stage waits on a `refresh`
    /// question. Always true.
    fn hold_lane(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        stage: &Stage,
        i: usize,
        why: &str,
        now_ms: u64,
    ) -> Result<bool> {
        let lane = &t.lanes[i];
        log::info!(
            "ticket {} lane {}: the worktree {why}; the stage waits",
            t.id,
            lane.name
        );
        let question = format!(
            "{} ({}): the worktree {} {why}, then answer recheck",
            stage.name,
            lane.name,
            lane.worktree.display()
        );
        self.ask_refresh(t, ps, stage, question, now_ms)?;
        Ok(true)
    }

    /// The `refresh` question about a lane the stage waits on, answered
    /// `recheck` once the owner has put it right by hand.
    fn ask_refresh(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        stage: &Stage,
        question: String,
        now_ms: u64,
    ) -> Result<()> {
        self.ensure_decision(
            t,
            ps,
            Ask {
                stage: &stage.name,
                name: REFRESH,
                kind: DecisionKind::Permission,
                question,
                options: &["recheck", "park"],
                recommendation: None,
                attempt: None,
            },
            now_ms,
        )
    }

    /// A rebase that stopped, on the lane's record before anything is
    /// asked or launched: the head and base a review last read, kept from
    /// the first time it stopped, since the stage is held until a
    /// bring-up and nothing reads the branch meanwhile, and the base and
    /// commits that conflict now.
    fn record_conflict(
        &mut self,
        t: &mut Ticket,
        i: usize,
        head: &str,
        from: &str,
        onto_sha: &str,
        now_ms: u64,
    ) -> Result<()> {
        let worktree = t.lanes[i].worktree.clone();
        let base = if from.is_empty() {
            self.git.merge_base(&worktree, head, onto_sha).ok()
        } else {
            Some(from.to_owned())
        };
        let commits = match base.map(|base| {
            self.git
                .conflicting_commits(&worktree, &base, head, onto_sha)
        }) {
            Some(Ok(commits)) => commits,
            Some(Err(e)) => {
                log::warn!(
                    "ticket {} lane {}: the conflicting commits could not be read: {e:#}",
                    t.id,
                    t.lanes[i].name
                );
                Vec::new()
            }
            None => Vec::new(),
        };
        let stage = t.stage;
        let lane = &mut t.lanes[i];
        match &mut lane.conflict {
            Some(c) => {
                onto_sha.clone_into(&mut c.to);
                c.commits = commits;
                c.stage = stage;
            }
            None => {
                lane.conflict = Some(RefreshConflict {
                    before: head.to_owned(),
                    from: from.to_owned(),
                    to: onto_sha.to_owned(),
                    commits,
                    stage,
                    at_ms: now_ms,
                });
            }
        }
        self.save_ticket(t, now_ms)
    }

    /// The lane's recorded conflict, taken off it at a bring-up: kept
    /// for the bring-up when the branch was rewritten since the rebase
    /// stopped (a rebaser or a hand rebase resolved it), dropped when
    /// git rebased the untouched branch cleanly onto a newer base.
    fn resolved_conflict(
        t: &mut Ticket,
        p: &Pipeline,
        i: usize,
        head_before: &str,
    ) -> Option<RefreshConflict> {
        let mut conflict = t.lanes[i].conflict.take()?;
        if conflict.before == head_before {
            log::info!(
                "ticket {} lane {}: the branch was never rewritten after its conflict; nothing to review",
                t.id,
                t.lanes[i].name
            );
            return None;
        }
        conflict.stage = t.stage;
        if !p.stages.iter().any(|s| s.kind() == StageKind::Review) {
            log::info!(
                "ticket {} lane {}: the conflict's resolution gets no review: the pipeline has no code review stage",
                t.id,
                t.lanes[i].name
            );
        }
        Some(conflict)
    }

    /// The clone that holds a lane's commits: the lane's own, for a lane
    /// with a repository of its own, else the project's.
    pub fn lane_clone(&self, p: &Pipeline, lane: &Lane) -> PathBuf {
        if lane.repo.is_some() {
            self.data.lane_repo_dir(&p.project.name, &lane.name)
        } else {
            self.data.repo_dir(&p.project.name)
        }
    }

    /// Where a lane's base is read: its clone, the remote fetched there,
    /// and the remote-tracking ref of its base.
    fn lane_base_ref(&self, p: &Pipeline, lane: &Lane) -> (PathBuf, String, String) {
        if lane.repo.is_some() {
            (
                self.data.lane_repo_dir(&p.project.name, &lane.name),
                p.lane_remote(lane).to_owned(),
                format!("{}/{}", p.lane_remote(lane), p.lane_base(lane)),
            )
        } else {
            (
                self.data.repo_dir(&p.project.name),
                p.project.remote.clone(),
                format!("{}/{}", p.project.remote, p.project.base),
            )
        }
    }

    /// The shape a refresh rebaser's attempt is watched under: the
    /// stage it holds, with no gate and its notes as the one artifact.
    fn refresh_stage(p: &Pipeline, stage: &Stage) -> Stage {
        let mut shape = stage.clone();
        REFRESH.clone_into(&mut shape.name);
        shape.operator.clone_from(&p.policy.rebaser);
        shape.review = None;
        shape.gate = None;
        shape.writes = vec![Write::named("notes")];
        shape.prompt = None;
        shape
    }

    /// The policy's rebaser, continued from the lane's last finished
    /// agent so it knows the change's intent, asked to rebase the
    /// branch onto its moved base and to leave it alone when a
    /// conflict's intent is unclear.
    #[allow(clippy::too_many_arguments)]
    fn start_refresh_rebaser(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        lane: usize,
        operator: &str,
        onto: &str,
        now_ms: u64,
    ) -> Result<()> {
        let name = t.lanes[lane].name.clone();
        let cwd = t.lanes[lane].worktree.clone();
        let branch = t.lanes[lane].branch.clone();
        let n = next_n(t, REFRESH);
        let dir = self.attempt_dir(t, REFRESH, n, &name)?;
        let notes = dir.join("notes.md");
        let mut vars = vars_for(t, p, Some(&name));
        vars.set("notes", notes.display().to_string());
        let guidance = p
            .operators
            .get(operator)
            .map_or("", |o| o.guidance.as_str());
        let mut prompt = guidance_prelude(guidance, &vars);
        let plan = t
            .input("plan")
            .map(|plan| format!(" (the plan is at {})", plan.display()))
            .unwrap_or_default();
        let checks = p
            .stages
            .iter()
            .find_map(|s| match &s.gate {
                Some(gate @ Gate::Command { .. }) => Some(
                    lane_gate_argv(gate, Some(&name))
                        .cloned()
                        .unwrap_or_default()
                        .join(" "),
                ),
                _ => None,
            })
            .filter(|c| !c.is_empty())
            .map(|c| format!(" The checks are: {c}."))
            .unwrap_or_default();
        let _ = write!(
            prompt,
            "The branch {branch} in {} is behind {onto}, and a rebase onto it stops on conflicts. Fetch, rebase the branch onto {onto}, resolve every conflict keeping the change's intent{plan}, finish the rebase with git rebase --continue (a rebase left stopped holds the stage), run the checks, and do not push; Dispatch pushes the branch.{checks} If a conflict's intent is unclear, abort the rebase, leave the branch as it was, and say why. Write what you did to {}.",
            cwd.display(),
            notes.display()
        );
        let clone_of = lane_clone_of(t, &name);
        log::info!(
            "ticket {} lane {name}: the branch conflicts with {onto}; {operator} starting{}",
            t.id,
            clone_of
                .as_deref()
                .map(|s| format!(" from {s}"))
                .unwrap_or_default()
        );
        let spec = AgentSpec {
            operator: operator.to_owned(),
            prompt,
            artifacts: BTreeMap::from([("notes".to_owned(), notes)]),
            clone_of,
            pr: None,
            rework: None,
            env: BTreeMap::new(),
        };
        self.launch_agent(t, ps, p, REFRESH, &name, &cwd, n, spec, now_ms)
    }

    /// Closing is a sequence, not a flag, as parking is: the intent is
    /// written first (`write_close_intent`), then `finish_closing` runs
    /// the rest from that saved intent. Reaching the end of the pipeline
    /// enters here.
    fn begin_close(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        reason: &str,
        now_ms: u64,
    ) -> Result<()> {
        self.write_close_intent(t, ps, reason, now_ms)?;
        self.finish_closing(t, ps, now_ms)
    }

    /// The ticket saved `Closing` and moved from the queue to the
    /// project's closing list. Once this returns, a pass finishes the
    /// close whatever happens to the caller.
    fn write_close_intent(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        reason: &str,
        now_ms: u64,
    ) -> Result<()> {
        log::info!("ticket {} closing: {reason}", t.id);
        t.state = TicketState::Closing {
            reason: reason.into(),
        };
        t.state_by.clone_from(&self.actor);
        // A close supersedes a restart held part way.
        t.restart = None;
        self.save_ticket(t, now_ms)?;
        ps.queue.retain(|id| id != &t.id);
        if !ps.closing.contains(&t.id) {
            ps.closing.push(t.id.clone());
        }
        self.save_project(ps)
    }

    /// The rest of a close, from the saved intent, safe to run again at
    /// any point: unanswered requests resolved, every process read back
    /// as gone, pending decisions cancelled, the session unmarked, the
    /// trees removed (lanes of their own repositories first), the card
    /// taken off the set, and only then `Closed`. Each step is skipped
    /// once its flag is saved; one that cannot finish now (a process
    /// still alive, the socket down) leaves the ticket `Closing` for the
    /// next pass.
    pub(crate) fn finish_closing(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        now_ms: u64,
    ) -> Result<()> {
        let TicketState::Closing { reason } = t.state.clone() else {
            return Ok(());
        };
        // One finisher at a time: the tree removal below lets the writer
        // lock go, and a pass or a hand close reaching this ticket then
        // leaves it to whoever holds the claim. Held until this returns.
        let Some(_close) = self.data.claim_close(&t.id)? else {
            log::info!("ticket {} closing: another dispatch is finishing it", t.id);
            return Ok(());
        };
        // A kill between the ticket's save and the project's leaves the
        // id in the queue; from here on it is only in `closing`.
        let was_queued = ps.queue.contains(&t.id);
        ps.queue.retain(|id| id != &t.id);
        let was_listed = ps.closing.contains(&t.id);
        if !was_listed {
            ps.closing.push(t.id.clone());
        }
        if was_queued || !was_listed {
            self.save_project(ps)?;
        }
        // A lost `session.waiting off` or `set.sync` of an earlier run is
        // resolved before anything is asked again; `step` never visits a
        // closing ticket to do it. An op recovery already settled is
        // left alone; a close can take several passes.
        let pending = t.unsettled();
        if !pending.is_empty() {
            let mut failed = None;
            for i in pending {
                if let Err(e) = self.recover_one(t, ps, i, now_ms) {
                    failed = Some(e);
                    break;
                }
            }
            // Recovery decides about attempts as if the ticket were
            // running; `fail_attempt_with` never parks a closing ticket,
            // and the intent to close is saved again here before any
            // error is returned, so nothing recovery does can leave the
            // ticket in another state on the closing list.
            t.state = TicketState::Closing {
                reason: reason.clone(),
            };
            self.save_ticket(t, now_ms)?;
            if let Some(e) = failed {
                return Err(e);
            }
        }
        // A launch Switchboard is still working on would bring up a
        // session after the processes below were read back as gone.
        if !t.unsettled_creations().is_empty() {
            log::info!("ticket {} closing: a launch is still in flight", t.id);
            return Ok(());
        }
        // Every launch has settled, so no reply can open an attempt
        // again; one still open is the close's to end, before its
        // processes are killed below.
        if self.cancel_open_attempts(t, &reason, now_ms)? {
            log::info!("ticket {} closing: a check is still running", t.id);
            return Ok(());
        }
        if !self.retire_processes(t, ps, &t.processes.clone(), now_ms)? {
            log::info!("ticket {} closing: a process is still alive", t.id);
            return Ok(());
        }
        // A ticket at `try` with no tester yet is closable while its
        // services start, so their `before` may still run.
        let mut stopped = true;
        for i in 0..t.services.len() {
            if t.services[i].state != ServiceState::Stopped {
                stopped &= self.stop_service(t, ps, i, now_ms)?;
            }
        }
        if !stopped {
            log::info!("ticket {} closing: a service is still stopping", t.id);
            return Ok(());
        }
        let p = self.pipeline_of(t).ok();
        self.forget_secrets(t, p.as_ref(), now_ms)?;
        if !t.holds.is_empty() {
            t.holds.clear();
            self.save_ticket(t, now_ms)?;
        }
        if !t.close.decisions_cancelled || !t.pending_decisions().is_empty() {
            for d in &mut t.decisions {
                if d.pending() {
                    d.state = DecisionState::Cancelled;
                }
            }
            t.close.decisions_cancelled = true;
            // A decision raised since the last unmark marked the session
            // waiting again.
            t.close.waiting_cleared = false;
            self.save_ticket(t, now_ms)?;
        }
        // Sent on every run until a reply is recorded: it is idempotent,
        // and a mark left on would keep counting the ticket as waiting.
        if !t.close.waiting_cleared {
            self.unmark(t, ps, now_ms)?;
            t.close.waiting_cleared = true;
            self.save_ticket(t, now_ms)?;
        }
        if t.close.trees_kept.is_none() {
            // The removal lets the writer lock go, and a `take` or a
            // reorder may land in the gap: the project is read again
            // whatever the removal did, so no caller saves a stale copy
            // over it. Anything held only in memory is saved first.
            if self.load_project(&ps.name)? != *ps {
                self.save_project(ps)?;
            }
            let removed = self.remove_trees(t, now_ms);
            *ps = self.load_project(&ps.name)?;
            removed?;
        }
        if !t.close.card_cleared {
            let others: Vec<Ticket> = ps
                .queue
                .iter()
                .filter_map(|id| self.load_ticket(id).ok())
                .collect();
            let project = ps.name.clone();
            crate::view::sync_queue(self, ps, &project, &others, Some(t), now_ms)?;
            t.close.card_cleared = true;
            self.save_ticket(t, now_ms)?;
        }
        log::info!("ticket {} closed: {reason}", t.id);
        t.state = TicketState::Closed { reason };
        self.save_ticket(t, now_ms)?;
        ps.closing.retain(|id| id != &t.id);
        self.save_project(ps)
    }

    /// The ticket's trees out of Dispatch's clones with `git worktree
    /// remove`, never forced: each lane with a repository of its own
    /// first (it is nested in the ticket's tree, and would otherwise be
    /// untracked content there), then the ticket's tree, each flag saved
    /// as it is read back. A refusal does not hold the close, which
    /// would retry forever on a stray file: why goes in `trees_kept`,
    /// and the tree stays with its work. A pipeline that works in place
    /// removes nothing; that tree is the user's checkout.
    fn remove_trees(&mut self, t: &mut Ticket, now_ms: u64) -> Result<()> {
        let p = match self.pipeline_of(t) {
            Ok(p) => p,
            Err(e) => {
                t.close.trees_kept = Some(format!("pipeline copy unreadable: {e:#}"));
                return self.save_ticket(t, now_ms);
            }
        };
        let (lanes, tree) = close_trees(t, &p);
        // Owned copies: the borrowed paths would hold `t` borrowed
        // through the loop below, which replaces it.
        let lanes: Vec<LaneRecord> = lanes.iter().map(|&i| t.lanes[i].clone()).collect();
        let tree = tree.map(Path::to_path_buf);
        // Each removal lets the writer lock go and reads the record
        // back, so anything held only in memory is saved first.
        if (!lanes.is_empty() || tree.is_some()) && self.load_ticket(&t.id)? != *t {
            self.save_ticket(t, now_ms)?;
        }
        let state = t.state.clone();
        let mut kept: Vec<String> = Vec::new();
        for lane in lanes {
            let clone = self.data.lane_repo_dir(&p.project.name, &lane.name);
            let removed = self.unlocked(|git| git.worktree_remove(&clone, &lane.worktree))?;
            self.reread_unchanged(t, &state)?;
            match removed {
                Ok(()) => {
                    log::info!("ticket {} lane {} removed", t.id, lane.name);
                    for l in t.lanes.iter_mut().filter(|l| l.name == lane.name) {
                        l.removed = true;
                    }
                    self.save_ticket(t, now_ms)?;
                }
                Err(e) => kept.push(format!("lane {}: {e:#}", lane.name)),
            }
        }
        // A lane still in the tree would refuse the tree's removal too.
        if kept.is_empty()
            && let Some(tree) = tree
        {
            let repo = self.data.repo_dir(&p.project.name);
            let removed = self.unlocked(|git| git.worktree_remove(&repo, &tree))?;
            self.reread_unchanged(t, &state)?;
            match removed {
                Ok(()) => {
                    log::info!("ticket {} tree removed", t.id);
                    t.close.tree_removed = true;
                    for lane in &mut t.lanes {
                        lane.removed = true;
                    }
                    self.save_ticket(t, now_ms)?;
                }
                Err(e) => kept.push(format!("{e:#}")),
            }
        }
        // The mark is written as found, and only here: a retry killed
        // mid-removal keeps it, so the next `dispatch close` retries.
        let found = (!kept.is_empty()).then(|| kept.join("; "));
        if let Some(why) = &found {
            log::warn!("ticket {} trees kept: {why}", t.id);
        }
        if t.close.trees_kept != found {
            t.close.trees_kept = found;
            self.save_ticket(t, now_ms)?;
        }
        Ok(())
    }

    /// `t` read again after the writer lock was let go, refused unless
    /// it is still in `state`. Only the finisher holding the ticket's
    /// `closing.lock` writes a ticket being closed or retried, so a
    /// change here is a bug or a hand-edited record, and nothing is
    /// saved over it.
    fn reread_unchanged(&self, t: &mut Ticket, state: &TicketState) -> Result<()> {
        *t = self.load_ticket(&t.id)?;
        if &t.state != state {
            bail!(
                "ticket {} changed while its trees were removed: {}",
                t.id,
                t.state.label()
            );
        }
        Ok(())
    }

    /// The paths that would refuse a ticket's trees' removal, read
    /// before anything is written: each lane with a repository of its
    /// own on its own terms, and the ticket's tree with those lanes left
    /// out.
    ///
    /// A ticket whose pipeline copy cannot be read passes: without it
    /// nothing is removed (`remove_trees` keeps the trees and says why),
    /// so nothing can be lost, and the close a parked ticket needs most
    /// is not refused.
    fn preflight_trees(&self, t: &Ticket) -> Result<()> {
        let Ok(p) = self.pipeline_of(t) else {
            return Ok(());
        };
        let (lanes, tree) = close_trees(t, &p);
        let lanes: Vec<PathBuf> = lanes.iter().map(|&i| t.lanes[i].worktree.clone()).collect();
        let found = crate::git::uncommitted(&*self.git, tree, &lanes)?;
        if !found.is_empty() {
            let named: Vec<String> = found.iter().map(|f| f.display().to_string()).collect();
            bail!(
                "ticket {} has changes in its trees: {}; commit or clean it, then close again",
                t.id,
                named.join(", ")
            );
        }
        Ok(())
    }

    /// Close a ticket by hand: its worktrees removed, its branch, its
    /// directory, its record and its Switchboard projects kept. Refused,
    /// with nothing written, while anything of it runs (park it first,
    /// so the cancellation sequence runs) or a tree has changes. The
    /// ticket comes back as it stands: `Closed`, or `Closing` when a
    /// process is still going and the next pass finishes it. On a closed
    /// ticket whose trees were kept, the removal is tried again.
    pub fn close_by_hand(
        &mut self,
        ticket: &str,
        reason: Option<&str>,
        now_ms: u64,
    ) -> Result<Ticket> {
        self.close_checked(ticket, reason, true, now_ms)
    }

    /// `close_by_hand` for a caller that must not wait on Switchboard:
    /// the same checks, but only the intent is written and the ticket
    /// comes back `Closing` for the runner's next pass to finish. The
    /// port answers Switchboard's own page with this, because the rest
    /// of a close asks Switchboard's control socket, which the page's
    /// thread is blocked from answering until this reply arrives.
    pub fn request_close(
        &mut self,
        ticket: &str,
        reason: Option<&str>,
        now_ms: u64,
    ) -> Result<Ticket> {
        self.close_checked(ticket, reason, false, now_ms)
    }

    fn close_checked(
        &mut self,
        ticket: &str,
        reason: Option<&str>,
        finish: bool,
        now_ms: u64,
    ) -> Result<Ticket> {
        self.transaction(|r| {
            let mut t = r.load_ticket(ticket)?;
            match &t.state {
                TicketState::Parking { .. } => {
                    bail!("ticket {ticket} is still parking; try again when it is parked")
                }
                TicketState::Active if t.attempts.iter().any(Attempt::is_open) => {
                    let open: Vec<String> = t
                        .attempts
                        .iter()
                        .filter(|a| a.is_open())
                        .map(|a| format!("{}/{}", a.stage, a.context))
                        .collect();
                    bail!(
                        "ticket {ticket} has {} open; park it first, so the cancellation sequence runs",
                        open.join(", ")
                    )
                }
                TicketState::Closed { .. } if t.close.trees_kept.is_some() => {
                    r.retry_removal(&mut t, now_ms)?;
                    return Ok(t);
                }
                TicketState::Closed { .. } => bail!("ticket {ticket} is already closed"),
                TicketState::Closing { .. } => return Ok(t),
                TicketState::Active | TicketState::Parked { .. } => {}
            }
            r.preflight_trees(&t)?;
            let mut ps = r.load_project(&t.project)?;
            r.write_close_intent(&mut t, &mut ps, reason.unwrap_or("closed by hand"), now_ms)?;
            if finish {
                r.finish_closing(&mut t, &mut ps, now_ms).with_context(|| {
                    format!("ticket {ticket} is closing; the runner finishes it on a later pass")
                })?;
            }
            Ok(t)
        })
    }

    /// The removal of a closed ticket's kept trees, tried again; the
    /// state and reason stay. A refusal is kept again and reported.
    fn retry_removal(&mut self, t: &mut Ticket, now_ms: u64) -> Result<()> {
        let Some(_close) = self.data.claim_close(&t.id)? else {
            bail!(
                "ticket {}: its trees are being removed by another dispatch",
                t.id
            );
        };
        self.remove_trees(t, now_ms)?;
        if let Some(why) = &t.close.trees_kept {
            bail!("ticket {}: trees kept: {why}", t.id);
        }
        Ok(())
    }

    /// Parking is a sequence, not a flag: the intent is written first,
    /// with every open decision on the ticket withdrawn in the same
    /// write (see `withdraw_open_decisions`), the session's waiting mark
    /// is cleared and read back, open attempts are cancelled (a review
    /// run paused so its tick cannot start a round), every process on
    /// the ticket's list is killed, and the ticket reads as parked only
    /// once Switchboard reports them all gone. `finish_parking` runs the
    /// rest on later passes if anything is still alive now.
    pub(crate) fn park(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        reason: &str,
        now_ms: u64,
    ) -> Result<()> {
        log::warn!("ticket {} parking: {reason}", t.id);
        Self::parking_intent(t, reason, None);
        self.save_ticket(t, now_ms)?;
        self.finish_parking(t, ps, now_ms)
    }

    /// A park's first write, in memory: `Parking` with every open
    /// decision withdrawn. Shared by a park the runner decides and one
    /// asked for by `dispatch park`, so the two leave the same record.
    /// `by` is who asked: `None` for the owner and for Dispatch itself.
    fn parking_intent(t: &mut Ticket, reason: &str, by: Option<String>) {
        t.state = TicketState::Parking {
            reason: reason.into(),
        };
        t.state_by = by;
        Self::withdraw_open_decisions(t, false);
    }

    /// The rest of the sequence, from the saved intent: any decision
    /// still open withdrawn, the session's waiting mark cleared and
    /// answered, every open attempt cancelled (its run paused and read
    /// back as paused), everything on the process list killed and read
    /// back as gone, and only then `Parked`. Run again on every pass
    /// until it gets there, so a restart at any point resumes it whole.
    fn finish_parking(&mut self, t: &mut Ticket, ps: &mut ProjectState, now_ms: u64) -> Result<()> {
        let TicketState::Parking { reason } = t.state.clone() else {
            return Ok(());
        };
        // A launch Switchboard is still working on would bring up a
        // session after its attempt was cancelled, owned by nothing: it
        // is resolved first, and parking waits while one is in flight.
        // Only creations: nothing else can start what parking must stop.
        let creations = t.unsettled_creations();
        if !creations.is_empty() {
            for i in creations {
                self.recover_one(t, ps, i, now_ms)?;
            }
            self.save_ticket(t, now_ms)?;
        }
        if !t.unsettled_creations().is_empty() {
            log::info!("ticket {} parking: a launch is still in flight", t.id);
            return Ok(());
        }
        // A parking ticket asks nothing but `stuck`, which the stop of a
        // service asks while parking. `park` withdraws its questions with
        // the intent, so this only finds work on a `Parking` record whose
        // decisions are still open.
        if Self::withdraw_open_decisions(t, true) {
            self.save_ticket(t, now_ms)?;
        }
        // Every marked session stops reading as waiting, and parking is
        // not done until Switchboard has said so for each.
        let unmarked = self.clear_marks(t, ps, now_ms)?;
        // A deploy is never killed halfway: parking waits for it to exit.
        if !self.gate_only_commands_exited(t, now_ms)? {
            return Ok(());
        }
        let open: Vec<Attempt> = t.attempts.iter().filter(|a| a.is_open()).cloned().collect();
        let mut settled = true;
        for a in open {
            settled &= self.cancel_attempt(t, ps, &a, &reason, now_ms)?;
        }
        settled &= self.retire_processes(t, ps, &t.processes.clone(), now_ms)?;
        for i in 0..t.services.len() {
            if t.services[i].state != ServiceState::Stopped {
                settled &= self.stop_service(t, ps, i, now_ms)?;
            }
        }
        if settled && unmarked {
            // Nothing runs any more, so a secret the ticket holds is
            // deleted with the park, a restart riding on it or not: the
            // stack it was for is stopped.
            let p = self.pipeline_of(t).ok();
            let lost = p.as_ref().map(|p| secret_holds(t, p)).unwrap_or_default();
            self.forget_secrets(t, p.as_ref(), now_ms)?;
            // Everything is read back as gone: a restart that rode on the
            // park applies now, instead of the park ending. It keeps the
            // ticket's holds, except one a deleted secret was made under:
            // retaking that sends the ticket back through its range, so
            // the stage that writes the secret runs again before anything
            // reads it.
            if t.restart.is_some() {
                t.holds.retain(|h| !lost.contains(&h.resource));
                return self.apply_restart(t, now_ms);
            }
            log::warn!("ticket {} parked: {reason}", t.id);
            t.state = TicketState::Parked { reason };
            // A parked ticket holds nothing; its services are stopped.
            t.holds.clear();
            self.save_ticket(t, now_ms)?;
        }
        Ok(())
    }

    /// Every open gate-only command (a deploy) read: false while one
    /// still runs. An exit is written as it is read, since the real
    /// child is gone after that read; one lost to a restart goes on.
    fn gate_only_commands_exited(&mut self, t: &mut Ticket, now_ms: u64) -> Result<bool> {
        let running: Vec<Attempt> = t
            .attempts
            .iter()
            .filter(|a| {
                a.is_open()
                    && a.kind == AttemptKind::GateOnly
                    && a.gate.as_ref().is_some_and(|g| g.exit.is_none())
            })
            .cloned()
            .collect();
        for a in running {
            match self.git.poll_check(&gate_key(t, &a)) {
                None => {
                    log::info!("ticket {} parking: {} is still running", t.id, a.stage);
                    return Ok(false);
                }
                Some(Ok(code)) => {
                    if let Some(g) =
                        find_attempt_mut(t, &a.stage, a.n).and_then(|x| x.gate.as_mut())
                    {
                        g.exit = Some(code);
                    }
                    self.save_ticket(t, now_ms)?;
                }
                Some(Err(_)) => {}
            }
        }
        Ok(true)
    }

    /// An attempt Dispatch stops on purpose: its run paused and confirmed
    /// paused, its processes killed, its checks killed and read back as
    /// gone, and only then its state written as cancelled, so a record
    /// never says cancelled about something still going. True once it
    /// is. No decision follows.
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
                // Gone is stopped too; any other failure (the app too
                // busy to answer) says nothing about the run.
                Reply::Failed { reason } if reason == NO_SUCH_RUN => true,
                Reply::Failed { reason } => {
                    log::warn!(
                        "ticket {}: workflow query failed: {reason}; asking again",
                        t.id
                    );
                    false
                }
                other => bail!("workflow query answered {other:?}"),
            };
            if !paused {
                return Ok(false);
            }
        }
        let mine = self.processes_of(a)?;
        if !self.retire_processes(t, ps, &mine, now_ms)? {
            return Ok(false);
        }
        // Before the command reviewers: this is the call that sends the
        // checks' TERM and starts the clock, and a repeated kill of the
        // same key below sends nothing.
        let reason = match self.stop_gate(t, a, now_ms)? {
            GateStop::Gone => reason.to_owned(),
            GateStop::Waiting => return Ok(false),
            GateStop::OverLimit(s) => format!("{reason}; {s}"),
        };
        self.kill_review_commands(t, a);
        if let Some(attempt) = find_attempt_mut(t, &a.stage, a.n) {
            attempt.state = AttemptState::Cancelled { reason };
            attempt.ended_ms = Some(now_ms);
        }
        // A rebaser stopped here may have rewritten the branch before
        // it went; the next pass reads the lane again, as one that
        // finished on its own is, so its conflict reaches a bring-up.
        if a.stage == REFRESH {
            t.refreshed_stage = None;
        }
        self.save_ticket(t, now_ms)?;
        Ok(true)
    }

    /// Ends a closing ticket's open attempts; true while one's checks
    /// are still running. Its checks are stopped first, as a park does,
    /// so the record never says cancelled about checks still running;
    /// then its command reviewers. An attempt whose checks are done is
    /// cancelled now, so a reason past the limit is not lost to a pass
    /// that waits on another attempt's.
    fn cancel_open_attempts(&mut self, t: &mut Ticket, reason: &str, now_ms: u64) -> Result<bool> {
        let open: Vec<Attempt> = t.attempts.iter().filter(|a| a.is_open()).cloned().collect();
        let mut waiting = false;
        let mut cancelled = false;
        for a in &open {
            let cancelled_reason = match self.stop_gate(t, a, now_ms)? {
                GateStop::Gone => format!("the ticket closed: {reason}"),
                GateStop::Waiting => {
                    waiting = true;
                    continue;
                }
                GateStop::OverLimit(s) => format!("the ticket closed: {reason}; {s}"),
            };
            self.kill_review_commands(t, a);
            if let Some(attempt) = find_attempt_mut(t, &a.stage, a.n) {
                attempt.state = AttemptState::Cancelled {
                    reason: cancelled_reason,
                };
                attempt.ended_ms = Some(now_ms);
                cancelled = true;
            }
        }
        if cancelled {
            self.save_ticket(t, now_ms)?;
        }
        Ok(waiting)
    }

    /// The attempt's running checks killed with their process group and
    /// read back: a stage gate's, or a code review's latest round's.
    /// Checks a previous runner left running are found by the gate's
    /// recorded group and killed the same way. Past `STOP_LIMIT_MS` from
    /// the first kill the group gets SIGKILL and the stop goes on without
    /// reading it back.
    pub(crate) fn stop_gate(
        &mut self,
        t: &mut Ticket,
        a: &Attempt,
        now_ms: u64,
    ) -> Result<GateStop> {
        let Some(gate) = a.gate.as_ref().filter(|g| g.exit.is_none()) else {
            return Ok(GateStop::Gone);
        };
        let key = match a.rounds.last() {
            Some(round) => checks_key(t, &(a.stage.clone(), a.n), round.n),
            None => gate_key(t, a),
        };
        let started = if let Some(&started) = self.stopping.get(&key) {
            started
        } else {
            let adopted = match &gate.group {
                Some(group) => self.adopt_orphan(t, a, &key, group, &gate.head, now_ms)?,
                None => None,
            };
            let started = adopted.unwrap_or_else(|| {
                self.git.kill_check(&key);
                now_ms
            });
            self.stopping.insert(key.clone(), started);
            started
        };
        if self.git.check_gone(&key) {
            self.stopping.remove(&key);
            return Ok(GateStop::Gone);
        }
        if now_ms.saturating_sub(started) >= STOP_LIMIT_MS {
            self.git.escalate_check(&key);
            self.stopping.remove(&key);
            log::warn!(
                "ticket {}: checks {key} still running {}s after the kill; sent SIGKILL",
                t.id,
                STOP_LIMIT_MS / 1000
            );
            return Ok(GateStop::OverLimit(format!(
                "its checks ({key}) were still running after {}s",
                STOP_LIMIT_MS / 1000
            )));
        }
        Ok(GateStop::Waiting)
    }

    /// The gate's recorded group looked up after a restart: cleared from
    /// the record if nothing of ours is left under it, or killed and
    /// recorded as an orphan if it still runs. The time of the group's
    /// first kill when it was killed, so the limit counts from it across
    /// restarts while the leader lives; a leader that died of the TERM
    /// reads as gone and its members are left. A group under a reused id
    /// gets its own entry and the full limit.
    fn adopt_orphan(
        &mut self,
        t: &mut Ticket,
        a: &Attempt,
        key: &str,
        group: &CheckGroup,
        head: &str,
        now_ms: u64,
    ) -> Result<Option<u64>> {
        match self.git.adopt_check(key, group) {
            Adopted::Known => Ok(None),
            Adopted::Gone => {
                if let Some(attempt) = find_attempt_mut(t, &a.stage, a.n)
                    && let Some(run) = &mut attempt.gate
                {
                    run.group = None;
                }
                self.save_ticket(t, now_ms)?;
                Ok(None)
            }
            Adopted::Killed => {
                log::info!(
                    "ticket {}: checks {key} left running by a previous runner (group {}); waiting for them to stop",
                    t.id,
                    group.pgid
                );
                let Some(attempt) = find_attempt_mut(t, &a.stage, a.n) else {
                    return Ok(Some(now_ms));
                };
                let first = attempt
                    .orphans_killed
                    .iter()
                    .find(|o| o.pgid == group.pgid && o.leader_started == group.leader_started)
                    .map(|o| o.at_ms);
                if first.is_none() {
                    attempt.orphans_killed.push(OrphanKill {
                        pgid: group.pgid,
                        leader_started: group.leader_started.clone(),
                        head: head.to_owned(),
                        at_ms: now_ms,
                    });
                }
                self.save_ticket(t, now_ms)?;
                Ok(Some(first.unwrap_or(now_ms)))
            }
        }
    }

    /// The sessions an attempt owns: its own, or its run's reviewer and
    /// planner clone.
    fn processes_of(&mut self, a: &Attempt) -> Result<Vec<String>> {
        let mut ids: Vec<String> = a.session.iter().cloned().collect();
        for round in &a.rounds {
            ids.extend(round.reviewers.iter().filter_map(|r| r.session.clone()));
            ids.extend(round.implementer.clone());
        }
        if let Some(run) = a.run.clone()
            && let Reply::Workflow { run } = self.ask(Body::Workflow { run })?
        {
            ids.push(run.reviewer);
            ids.extend(run.planner);
        }
        Ok(ids)
    }

    /// Kill every session named that still runs, then read each back.
    /// True when none is running any more.
    pub(crate) fn retire_processes(
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
            .filter(|a| {
                a.is_open() && !matches!(a.kind, AttemptKind::GateOnly | AttemptKind::Review)
            })
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
        if t.decisions.iter().any(|d| {
            d.pending() && d.stage == ask.stage && d.name == ask.name && d.attempt == ask.attempt
        }) {
            return Ok(());
        }
        let d = new_decision(t, ask, now_ms);
        log::info!(
            "ticket {} decision {} ({}): {}",
            t.id,
            d.id,
            d.name,
            d.question
        );
        // What the session is told, taken before the push moves `d`.
        let Decision {
            id,
            name,
            question,
            options,
            ..
        } = &d;
        let notes = format!(
            "Dispatch ticket {} needs a decision ({id}): {question}\nAnswer with: dispatch decide {} {id} <{}>",
            t.id,
            t.id,
            options.join("|")
        );
        let waiting = format!("decision {id}: {name}");
        t.decisions.push(d);
        self.save_ticket(t, now_ms)?;
        // The ticket's current session shows it, so the badge and the
        // rail count it.
        if let Some(session) = t.current_session().cloned() {
            self.send(
                t,
                ps,
                None,
                "notes",
                Body::SessionNotes {
                    session: session.clone(),
                    text: notes,
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
                    reason: waiting,
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

    /// Clear the waiting marks decisions put on sessions, once none is
    /// pending: every session still marked in the ledger, which need not
    /// be the ticket's current one (another lane may have launched since).
    /// A replay still unanswered holds the rest back; `step` calls this
    /// again on every pass while `marks_unsettled`.
    pub(crate) fn unmark(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        now_ms: u64,
    ) -> Result<()> {
        if t.pending_decisions().is_empty() {
            self.clear_marks(t, ps, now_ms)?;
        }
        Ok(())
    }

    /// Clear one named session's waiting mark.
    fn unmark_session(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        session: String,
        now_ms: u64,
    ) -> Result<()> {
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
        Ok(())
    }

    /// Every decision on the ticket that is still open withdrawn in
    /// memory; the caller's next save writes it. True if there was one.
    /// Open means pending, answered and not yet acted on, or an answer
    /// acted on whose replacement has not launched: a `rerun` whose
    /// attempt is still the latest in its context, so `may_rerun` would
    /// launch on it, or a human gate's send-back, whose note is still on
    /// `t.rework` (`start_agent` takes it off only with the attempt that
    /// carries it), so `sent_back` would launch on it. Kept, either
    /// would launch on the resume and keep its context from being asked
    /// afresh. A withdrawn note is not lost: the sent-back attempt's
    /// cancellation reason quotes it, the resume's `rerun` question
    /// quotes that reason, and a `rerun` answer to it puts the note back
    /// (`rerun_note`).
    ///
    /// With `keep_stuck`, a `stuck` question, pending or answered and not
    /// yet acted on, is left: parking itself waits on it, and reads its
    /// answer. The intent to park passes false, so a `stuck` asked while
    /// the ticket was active is withdrawn with the rest and asked afresh.
    fn withdraw_open_decisions(t: &mut Ticket, keep_stuck: bool) -> bool {
        let unlaunched: Vec<bool> = t
            .decisions
            .iter()
            .map(|d| authorises_unlaunched_rerun(t, d))
            .collect();
        let mut withdrawn = false;
        for (d, unlaunched) in t.decisions.iter_mut().zip(unlaunched) {
            if keep_stuck && d.name == STUCK {
                continue;
            }
            if d.pending() || d.unacted_answer().is_some() || unlaunched {
                d.state = DecisionState::Cancelled;
                withdrawn = true;
            }
        }
        if !t.rework.is_empty() {
            t.rework.clear();
            withdrawn = true;
        }
        withdrawn
    }

    /// Resolve every `session.waiting` whose reply never came, in ledger
    /// order and under its own operation id (or settle it unsent when a
    /// later request for its session replaced it), and only then send
    /// `on: false` to each session whose last waiting request turned its
    /// mark on. True once every waiting request on the ledger that can be
    /// sent again is resolved and no session's last one is `on: true`,
    /// or there never was one.
    ///
    /// Only waiting requests are recovered here: recovering a lost
    /// creation can fail an attempt and ask a new question, which a
    /// parking ticket must not do. The rest wait for `step`'s recovery
    /// or startup's.
    ///
    /// One recorded without its body cannot be sent again or say which
    /// session it marked, so it is left alone rather than waited on.
    fn clear_marks(&mut self, t: &mut Ticket, ps: &mut ProjectState, now_ms: u64) -> Result<bool> {
        let unanswered = unanswered_waiting(t);
        let replayed = !unanswered.is_empty();
        let mut resolved = true;
        for i in unanswered {
            // Sent again, or settled unsent when a later request for
            // its session replaced it (`superseded`).
            self.recover_one(t, ps, i, now_ms)?;
            // Later requests wait for this one, so an older `on: true`
            // cannot land after the unmark.
            if t.ledger[i].unresolved() {
                resolved = false;
                break;
            }
        }
        if replayed {
            self.save_ticket(t, now_ms)?;
        }
        if !resolved {
            return Ok(false);
        }
        // The session a mark was put on is the one unmarked, even if a
        // later attempt has a session of its own.
        for session in still_marked(t) {
            self.unmark_session(t, ps, session, now_ms)?;
        }
        Ok(still_marked(t).is_empty())
    }

    /// Every answer not yet acted on.
    fn act_on_answers(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        now_ms: u64,
    ) -> Result<()> {
        // A `stuck` answer is read by the stop it is about.
        let answered: Vec<Answered> = t
            .decisions
            .iter()
            .enumerate()
            .filter(|(_, d)| d.name != STUCK)
            .filter_map(|(i, d)| {
                d.unacted_answer()
                    .map(|a| (i, d.name.clone(), a.to_owned(), d.attempt.clone()))
            })
            .collect();
        for (i, name, answer, attempt) in answered {
            // The acted mark is set in memory here and reaches disk with
            // the action's own first write (the parking state, the ledger
            // entry, the lane record), never before it: an answer is
            // either still unacted or its intent is durable. Two
            // exceptions: a `rerun` answer to a `rerun` decision, marked
            // only once the replaced attempt is confirmed gone, below,
            // since `may_rerun` launches on that mark; and a `reuse` or
            // `fresh` answer to `branch`, marked by the cut that uses it.
            if !matches!(
                (name.as_str(), answer.as_str()),
                ("rerun", "rerun") | ("branch", "reuse" | "fresh")
            ) && let DecisionState::Answered { acted, .. } = &mut t.decisions[i].state
            {
                *acted = true;
            }
            match (name.as_str(), answer.as_str()) {
                ("lanes", lanes) => {
                    self.choose_lanes(t, ps, p, &lane_names(lanes), now_ms)?;
                    // The investigator the question marked is no longer
                    // the current session once the lanes launch.
                    self.unmark(t, ps, now_ms)?;
                }
                ("finalize", "finalize") | ("paused", "continue") => {
                    if let Some(run) = attempt
                        .as_ref()
                        .and_then(|(s, n)| find_attempt(t, s, *n))
                        .and_then(|a| a.run.clone())
                    {
                        let body = if answer == "finalize" {
                            Body::WorkflowFinalize { run }
                        } else {
                            Body::WorkflowContinue { run }
                        };
                        self.send(t, ps, attempt.clone(), &answer, body, now_ms)?;
                    }
                    self.unmark(t, ps, now_ms)?;
                }
                ("rerun", "check") => {
                    self.check_again(t, ps, attempt.as_ref(), now_ms)?;
                }
                ("rerun", "keep") => self.keep_history(t, ps, p, attempt.as_ref(), now_ms)?,
                ("pr", "recheck") => recheck_pr(t, attempt.as_ref()),
                (REFRESH, "recheck") => t.refreshed_stage = None,
                ("review-code" | "review-cap" | RESOLUTION, "fix" | "accept" | "more") => {
                    self.review_answer(t, ps, &name, &answer, attempt.as_ref(), now_ms)?;
                }
                ("message", "accept" | "rewrite") => {
                    self.message_answer(t, ps, &answer, attempt.as_ref(), now_ms)?;
                }
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
                    let note = rerun_note(t, p, i);
                    if !self.retire_replaced(t, ps, i, attempt.as_ref(), note, now_ms)? {
                        continue;
                    }
                }
                // The cut reads this answer from the record, later on
                // this same pass, and marks it acted once it is used.
                ("branch", "reuse" | "fresh") => continue,
                (crate::services::SERVICE, "retry") => self.retry_services(t, ps, p, now_ms)?,
                (_, "park") => self.park_by_answer(t, ps, i, now_ms)?,
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

    /// Decision `i`'s `park` answer acted on: the acted mark is set in
    /// memory and reaches disk with the parking state. `act_on_answers`
    /// has set it already; the mark here is for `ask_again_unslotted`,
    /// which acts on a `park` answer outside that loop.
    fn park_by_answer(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        i: usize,
        now_ms: u64,
    ) -> Result<()> {
        if let DecisionState::Answered { acted, .. } = &mut t.decisions[i].state {
            *acted = true;
        }
        let name = t.decisions[i].name.clone();
        self.park(t, ps, &format!("parked by hand at decision {name}"), now_ms)
    }

    /// A rerun's replaced attempt is retired first, so an old and a new
    /// attempt never run together; still alive, the answer stays
    /// unacted for the next pass (false). Gone, the answer is acted,
    /// and a `note` for the replacement's prompt goes onto `t.rework`
    /// in the same write, so the launch `may_rerun` allows carries it.
    fn retire_replaced(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        decision: usize,
        attempt: Option<&(String, u32)>,
        note: Option<(String, String)>,
        now_ms: u64,
    ) -> Result<bool> {
        if let Some(a) = attempt.and_then(|(s, n)| find_attempt(t, s, *n)).cloned() {
            let mine = self.processes_of(&a)?;
            if !self.retire_processes(t, ps, &mine, now_ms)? {
                return Ok(false);
            }
        }
        if let DecisionState::Answered { acted, .. } = &mut t.decisions[decision].state {
            *acted = true;
        }
        if let Some((key, note)) = note {
            t.rework.insert(key, note);
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
    ///
    /// Every clone is fetched and surveyed before anything is cut. A
    /// branch of the ticket's name left by an earlier, closed ticket is
    /// deleted when it has nothing beyond its start, and asked about
    /// (`reuse | fresh | park`) when it has commits, with the cut held
    /// until the question is answered.
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
        // A question pending holds the cut whole, so a waiting ticket
        // does not fetch every clone on every pass.
        if t.decisions
            .iter()
            .any(|d| d.pending() && d.stage == CUT && d.name == "branch")
        {
            return Ok(());
        }
        let tree = t
            .tree
            .clone()
            .unwrap_or_else(|| self.worktree_root(p).join(&t.id));
        let lanes = lanes_to_cut(t, p);
        let todo = self.uncut(t, p, &url, &tree, &lanes);
        let Some(moved) = self.survey(t, ps, &todo, now_ms)? else {
            return Ok(());
        };
        let Some(existing) = self.branch_answer(t, ps, &todo, &moved, now_ms)? else {
            return Ok(());
        };

        let mut todo = todo.into_iter().zip(existing);
        if t.tree.is_none() {
            let (u, existing) = todo.next().expect("the tree is first when uncut");
            if !self.cut(t, ps, &u, existing, now_ms)? {
                return Ok(());
            }
            t.tree = Some(u.dir);
            self.save_ticket(t, now_ms)?;
        }
        for LaneCut { index, pr, branch } in lanes {
            let lane = &p.lanes[index];
            let dir = tree.join(&lane.path);
            if lane.repo.is_some() {
                let (u, existing) = todo
                    .next()
                    .expect("`uncut` lists every lane with a repository");
                if !self.cut(t, ps, &u, existing, now_ms)? {
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
            let base_sha = self.lane_base_sha(p, lane, pr.as_ref(), &branch);
            t.lanes.push(LaneRecord {
                name: lane.name.clone(),
                worktree: dir,
                branch,
                project: None,
                chosen: p.lanes.len() == 1 || pr.is_some(),
                setup_done: false,
                base_sha,
                refreshed: None,
                pushed: None,
                conflict: None,
                removed: false,
            });
            self.save_ticket(t, now_ms)?;
        }
        // The `branch` answer is spent once every context is cut. A cut
        // that parked first left it unacted, and the parking withdrew
        // it, so a resume asks again rather than repeat what failed.
        if let Some(d) = latest_branch_decision(t)
            && let DecisionState::Answered { acted, .. } = &mut d.state
            && !*acted
        {
            *acted = true;
            self.save_ticket(t, now_ms)?;
        }
        Ok(())
    }

    /// The contexts `cut_trees` has yet to make, in pipeline order: the
    /// tree if it is not cut, then each of `lanes` with a repository of
    /// its own.
    fn uncut(
        &self,
        t: &Ticket,
        p: &Pipeline,
        url: &str,
        tree: &Path,
        lanes: &[LaneCut],
    ) -> Vec<Uncut> {
        let mut todo = Vec::new();
        if t.tree.is_none() {
            let pr = tree_pr(t, p).cloned();
            todo.push(Uncut {
                what: "the tree".into(),
                url: url.to_owned(),
                clone: self.data.repo_dir(&p.project.name),
                remote: p.project.remote.clone(),
                dir: tree.to_path_buf(),
                branch: tree_branch(t, pr.as_ref()),
                start: format!("{}/{}", p.project.remote, p.project.base),
                extra: pr.as_ref().and_then(|pr| extra_remote(p, None, pr)),
                pr,
            });
        }
        for l in lanes {
            let lane = &p.lanes[l.index];
            let Some(url) = &lane.repo else {
                continue;
            };
            let remote = p.lane_remote(lane).to_owned();
            todo.push(Uncut {
                what: format!("lane {}", lane.name),
                url: url.clone(),
                clone: self.data.lane_repo_dir(&p.project.name, &lane.name),
                start: format!("{remote}/{}", p.lane_base(lane)),
                remote,
                dir: tree.join(&lane.path),
                branch: l.branch.clone(),
                extra: l.pr.as_ref().and_then(|pr| extra_remote(p, Some(lane), pr)),
                pr: l.pr.clone(),
            });
        }
        todo
    }

    /// Every context's clone fetched, then looked at for a branch of
    /// its name an earlier ticket left: one with nothing beyond its start
    /// is deleted, and one with commits is returned with how many (its
    /// index in `todo`). `None` when the ticket was parked instead.
    fn survey(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        todo: &[Uncut],
        now_ms: u64,
    ) -> Result<Option<Vec<(usize, u64)>>> {
        let mut moved: Vec<(usize, u64)> = Vec::new();
        for (i, u) in todo.iter().enumerate() {
            if !self.prepare(t, ps, u, now_ms)? {
                return Ok(None);
            }
            // A directory already at a context's place is adopted by
            // `cut`, and a pull request's branch is reset by `-B`, so
            // neither is surveyed.
            if u.pr.is_some() || u.dir.exists() {
                continue;
            }
            match self.clear_kept(&t.id, u) {
                Ok(Some(n)) => moved.push((i, n)),
                Ok(None) => {}
                Err(e) => {
                    self.park(
                        t,
                        ps,
                        &format!(
                            "{}: could not clear the kept branch {} in {}: {e:#}",
                            u.what,
                            u.branch,
                            u.clone.display()
                        ),
                        now_ms,
                    )?;
                    return Ok(None);
                }
            }
        }
        Ok(Some(moved))
    }

    /// A context's branch left by an earlier ticket: deleted when it has
    /// nothing beyond its start (`None`, as when there is none), else
    /// how many commits it has.
    fn clear_kept(&mut self, ticket: &str, u: &Uncut) -> Result<Option<u64>> {
        if !self.git.branch_exists(&u.clone, &u.branch)? {
            return Ok(None);
        }
        let n = self.git.branch_ahead(&u.clone, &u.branch, &u.start)?;
        if n > 0 {
            return Ok(Some(n));
        }
        self.git.delete_branch(&u.clone, &u.branch)?;
        log::info!(
            "ticket {ticket} {}: deleted the kept branch {} with nothing beyond {}",
            u.what,
            u.branch,
            u.start
        );
        Ok(None)
    }

    /// What the cut does with each context's branch, by the answer to
    /// the latest `branch` decision: one answer covers every context
    /// with commits (`moved`), and the rest are cut as usual. With no
    /// answer to go by (none yet, a `park`, or one a parking withdrew),
    /// the question is asked and `None` holds the cut, as it does when
    /// the ticket was parked instead.
    fn branch_answer(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        todo: &[Uncut],
        moved: &[(usize, u64)],
        now_ms: u64,
    ) -> Result<Option<Vec<Option<Existing>>>> {
        let mut existing: Vec<Option<Existing>> = vec![None; todo.len()];
        if moved.is_empty() {
            return Ok(Some(existing));
        }
        let answer = latest_branch_decision(t)
            .and_then(|d| d.unacted_answer().map(|a| (a.to_owned(), d.made_ms)));
        match answer {
            Some((a, _)) if a == "reuse" => {
                for &(i, _) in moved {
                    existing[i] = Some(Existing::Reuse);
                }
            }
            Some((a, asked_ms)) if a == "fresh" => {
                let Some(to) = self.free_name(t, ps, todo, moved, asked_ms, now_ms)? else {
                    return Ok(None);
                };
                for &(i, _) in moved {
                    existing[i] = Some(Existing::Fresh(to.clone()));
                }
            }
            _ => {
                let Some(fresh_to) = self.free_name(t, ps, todo, moved, now_ms, now_ms)? else {
                    return Ok(None);
                };
                let named: Vec<(&str, &Path, u64)> = moved
                    .iter()
                    .map(|&(i, n)| (todo[i].what.as_str(), todo[i].clone.as_path(), n))
                    .collect();
                let question = branch_question(&todo[moved[0].0].branch, &named, &fresh_to);
                self.ensure_decision(
                    t,
                    ps,
                    Ask {
                        stage: CUT,
                        name: "branch",
                        kind: DecisionKind::Permission,
                        question,
                        options: &["reuse", "fresh", "park"],
                        recommendation: None,
                        attempt: None,
                    },
                    now_ms,
                )?;
                return Ok(None);
            }
        }
        Ok(Some(existing))
    }

    /// The first of `<branch>.closed-<yyyymmdd>`, `…-2`, `…-3`, … that
    /// no clone in `moved` has, the date that of `ms`: each kept branch
    /// with commits is renamed there to make way for a fresh one, under
    /// the one name the question gave. The suffix covers a second close
    /// on the same day. `None` when a clone's branches could not be read
    /// and the ticket was parked instead.
    fn free_name(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        todo: &[Uncut],
        moved: &[(usize, u64)],
        ms: u64,
        now_ms: u64,
    ) -> Result<Option<String>> {
        let stem = format!("{}.closed-{}", todo[moved[0].0].branch, yyyymmdd(ms));
        let mut name = stem.clone();
        let mut n = 1;
        loop {
            let mut taken = false;
            for &(i, _) in moved {
                let clone = &todo[i].clone;
                match self.git.branch_exists(clone, &name) {
                    Ok(exists) => taken |= exists,
                    Err(e) => {
                        self.park(
                            t,
                            ps,
                            &format!(
                                "{}: could not read the branches in {}: {e:#}",
                                todo[i].what,
                                clone.display()
                            ),
                            now_ms,
                        )?;
                        return Ok(None);
                    }
                }
            }
            if !taken {
                return Ok(Some(name));
            }
            n += 1;
            name = format!("{stem}-{n}");
        }
    }

    /// The commit a lane is cut from, resolved at the cut: what a code
    /// review diffs against however far the remote moves later. Only a
    /// refresh that brings the lane onto a newer base moves it. A branch
    /// reused with commits of its own is based where it forked from
    /// `start`, so a review does not show upstream work as reverted.
    fn lane_base_sha(
        &self,
        p: &Pipeline,
        lane: &Lane,
        pr: Option<&PullRequestSource>,
        branch: &str,
    ) -> Option<String> {
        // A pull request's base is the branch it targets on its own
        // remote, where its branch forked from it: what its reviewers
        // see is what the pull request shows.
        if let Some(pr) = pr {
            let clone = self.lane_clone(p, lane);
            return self
                .git
                .merge_base(&clone, &format!("{}/{}", pr.remote, pr.base), pr.local())
                .ok();
        }
        let (clone, _, start) = self.lane_base_ref(p, lane);
        if self
            .git
            .branch_ahead(&clone, branch, &start)
            .is_ok_and(|n| n > 0)
        {
            return self.git.merge_base(&clone, &start, branch).ok();
        }
        self.git.rev_parse(&clone, &start).ok()
    }

    /// The pull requests' branches brought up to what the remote has,
    /// before the user is asked about them. A branch that no longer
    /// fast-forwards (a force push) parks the ticket, since what was
    /// looked at is gone.
    fn refresh_pull_requests(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        now_ms: u64,
    ) -> Result<()> {
        for pr in t.source.pull_requests.clone() {
            let Some(lane) = p.lane(&pr.lane) else {
                continue;
            };
            let Some(record) = t.lanes.iter().find(|l| l.name == pr.lane) else {
                continue;
            };
            let clone = self.lane_clone(p, lane);
            let extra = extra_remote(p, Some(lane), &pr);
            let worktree = record.worktree.clone();
            let refresh = self.fetch_pull_request(&clone, &pr, extra).and_then(|()| {
                let argv = [
                    "git",
                    "merge",
                    "--ff-only",
                    &format!("{}/{}", pr.remote, pr.local()),
                ]
                .map(str::to_owned);
                self.git.run(&worktree, &argv, &[])
            });
            if let Err(e) = refresh {
                return self.park(
                    t,
                    ps,
                    &format!(
                        "lane {}: PR #{} branch {} no longer fast-forwards (a force push?): {e:#}",
                        pr.lane, pr.number, pr.branch
                    ),
                    now_ms,
                );
            }
        }
        Ok(())
    }

    /// What the clone needs of a pull request: its remote pointed at
    /// the mirror when it is one, fetched, and on GitHub the pull ref
    /// itself.
    fn fetch_pull_request(
        &mut self,
        clone: &Path,
        pr: &PullRequestSource,
        extra: Option<(String, String)>,
    ) -> Result<()> {
        if let Some((name, url)) = extra {
            self.git.ensure_remote(clone, &name, &url)?;
        }
        self.git.fetch(clone, &pr.remote)?;
        if pr.provider == "github" {
            self.git.fetch_pull(clone, &pr.remote, pr.number)?;
        }
        Ok(())
    }

    /// A context's clone made if it is not there yet and fetched (with
    /// a pull request, its branch fetched), so a cut starts from what
    /// the remote has now. False when the ticket was parked instead.
    fn prepare(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        u: &Uncut,
        now_ms: u64,
    ) -> Result<bool> {
        let Uncut {
            what,
            url,
            clone,
            remote,
            pr,
            extra,
            ..
        } = u;
        if let Err(e) = self.git.ensure_clone(url, clone) {
            self.park(
                t,
                ps,
                &format!("{what}: could not clone {url}: {e:#}"),
                now_ms,
            )?;
            return Ok(false);
        }
        let fetched = match pr {
            Some(pr) => self.fetch_pull_request(clone, pr, extra.clone()),
            None => self.git.fetch(clone, remote),
        };
        let remote = pr.as_ref().map_or(remote, |pr| &pr.remote);
        if let Err(e) = fetched {
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
        Ok(true)
    }

    /// One worktree, its clone already prepared: cut from `start` on
    /// its branch at its directory, or adopted when git says it is
    /// already that. With a pull request, the worktree is that pull
    /// request's branch as its remote has it. With `existing`, a branch
    /// an earlier ticket left with commits is checked out as it is, or
    /// renamed out of the way first. False when the ticket was parked
    /// instead.
    fn cut(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        u: &Uncut,
        existing: Option<Existing>,
        now_ms: u64,
    ) -> Result<bool> {
        let Uncut {
            what,
            clone,
            remote,
            dir,
            branch,
            start,
            pr,
            ..
        } = u;
        let remote = pr.as_ref().map_or(remote, |pr| &pr.remote);
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
        let added = if pr.is_some() {
            self.git.worktree_track(clone, dir, branch, remote)
        } else {
            match existing {
                None => self.git.worktree_add(clone, dir, branch, start),
                Some(Existing::Reuse) => self.git.worktree_checkout(clone, dir, branch),
                Some(Existing::Fresh(to)) => self
                    .git
                    .rename_branch(clone, branch, &to)
                    .and_then(|()| self.git.worktree_add(clone, dir, branch, start)),
            }
        };
        if let Err(e) = added {
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
    pub(crate) fn ensure_setup(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        cwd: &Path,
        now_ms: u64,
    ) -> Result<bool> {
        match self.run_setup(t, p, cwd, now_ms)? {
            Some(reason) => {
                self.park(t, ps, &reason, now_ms)?;
                Ok(false)
            }
            None => Ok(true),
        }
    }

    /// The setup of each lane at `cwd` not yet run (or changed by a
    /// restart), each marked done and saved as it passes. The first
    /// failure stops the rest and is returned as the reason, with that
    /// lane's setup still pending; the caller decides what it costs.
    pub(crate) fn run_setup(
        &mut self,
        t: &mut Ticket,
        p: &Pipeline,
        cwd: &Path,
        now_ms: u64,
    ) -> Result<Option<String>> {
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
            let env = env_for(t, Some(&name), Some(&branch));
            let ran = match confine_for(t, p, Some(&name), &[], None) {
                Some(confine) => self
                    .git
                    .run_confined(cwd, &setup, &env, &confine)
                    .map(|header| {
                        log::info!("ticket {} lane {name}: setup done; {header}", t.id);
                    }),
                None => self.git.run(cwd, &setup, &env),
            };
            if let Err(e) = ran {
                let reason = format!("lane {name}: setup failed: {e:#}");
                log::info!("ticket {}: {reason}", t.id);
                return Ok(Some(reason));
            }
            t.lanes[i].setup_done = true;
            self.save_ticket(t, now_ms)?;
        }
        Ok(None)
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
                    self.finish_gate_only(t, stage, now_ms)?;
                    return Ok(());
                }
                let hinted = lane_hints(p, &t.source.labels);
                if p.dial("lanes") == "auto" && !hinted.is_empty() {
                    self.choose_lanes(t, ps, p, &hinted, now_ms)?;
                    if t.active() {
                        self.finish_gate_only(t, stage, now_ms)?;
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
                let decision = decision.clone().unwrap_or_else(|| DEFAULT_MERGE.to_owned());
                self.pr_merged_stage(t, ps, p, stage, &decision, now_ms)
            }
            Some(Gate::Human { decision, confirm }) => {
                self.human_gate(t, ps, p, stage, decision, *confirm, now_ms)
            }
            Some(Gate::Command { .. }) => self.command_gate_stage(t, ps, p, stage, now_ms),
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
        let Some(contexts) = self.contexts_or_park(t, ps, p, stage, now_ms)? else {
            return Ok(());
        };
        let mut all_complete = true;
        for (ctx, cwd, lane) in contexts {
            if self.poll_rebaser(t, ps, p, stage, &ctx, &cwd, lane.as_deref(), now_ms)? {
                all_complete = false;
                continue;
            }
            let Some(attempt) = self.open_gate_attempt(t, stage, &ctx, now_ms)? else {
                continue;
            };
            all_complete = false;
            let poll = PrPoll {
                stage,
                attempt: &attempt,
                cwd: &cwd,
                lane: lane.as_deref(),
            };
            self.poll_pr_checks(t, ps, p, &poll, now_ms)?;
            if !t.active() {
                return Ok(());
            }
        }
        if all_complete {
            self.advance(t, now_ms)?;
        }
        Ok(())
    }

    /// A gate-only stage with a command gate (a deploy): one attempt per
    /// context that runs the command as a child of the runner on the
    /// context's clean tree, as an agent stage's checks run, its exit
    /// bound to the head it ran at. A failure is a `rerun` question; a
    /// failed or cancelled attempt is never run again unasked, since
    /// whatever it does may have been done.
    fn command_gate_stage(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        now_ms: u64,
    ) -> Result<()> {
        let Some(contexts) = self.contexts_or_park(t, ps, p, stage, now_ms)? else {
            return Ok(());
        };
        let mut all_complete = true;
        for (ctx, cwd, lane) in contexts {
            let last = latest_attempt(t, &stage.name, &ctx).cloned();
            match last {
                Some(a) if a.state == AttemptState::Complete => {}
                Some(a) if a.is_open() => {
                    all_complete = false;
                    if a.gate.is_some() {
                        self.poll_gate(t, ps, p, &a, stage, &cwd, lane.as_deref(), now_ms)?;
                    } else {
                        self.start_gate(t, ps, p, &a, stage, &cwd, lane.as_deref(), now_ms)?;
                    }
                }
                Some(a) => {
                    all_complete = false;
                    if !held_in(t, &stage.name, &ctx) && may_rerun(t, &a) {
                        self.open_command_attempt(t, stage, &ctx, now_ms)?;
                    } else if asks_again(t, &a, false) {
                        self.ask_rerun(t, ps, &a, now_ms)?;
                    }
                }
                None => {
                    all_complete = false;
                    self.open_command_attempt(t, stage, &ctx, now_ms)?;
                }
            }
            if !t.active() {
                return Ok(());
            }
        }
        if all_complete {
            self.advance(t, now_ms)?;
        }
        Ok(())
    }

    /// A fresh gate-only attempt, written before its command starts on
    /// the next pass.
    fn open_command_attempt(
        &mut self,
        t: &mut Ticket,
        stage: &Stage,
        ctx: &str,
        now_ms: u64,
    ) -> Result<()> {
        let n = next_n(t, &stage.name);
        let artifacts = if stage.writes.is_empty() {
            BTreeMap::new()
        } else {
            artifact_paths(stage, &self.attempt_dir(t, &stage.name, n, ctx)?)
        };
        let mut a = new_attempt(
            &stage.name,
            n,
            ctx,
            AttemptKind::GateOnly,
            AttemptState::Running,
            artifacts,
            now_ms,
        );
        a.secret = stage.secret_writes().map(str::to_owned).collect();
        log::info!("ticket {} {}/{ctx} attempt {}", t.id, stage.name, a.n);
        t.attempts.push(a);
        self.save_ticket(t, now_ms)
    }

    /// The start of a PR-reading poll: where to look and the tree's
    /// head. `None` while the attempt's poll interval runs (at most one
    /// reading per `PR_POLL_MS` unless a recheck answer cleared the
    /// clock), or once the ticket is parked because the stage names a
    /// remote no provider serves.
    fn pr_poll_target(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        poll: &PrPoll<'_>,
        provider: Option<&str>,
        now_ms: u64,
    ) -> Result<Option<(PrTarget, String)>> {
        let PrPoll {
            stage,
            attempt: a,
            cwd,
            lane,
        } = *poll;
        if a.pr
            .as_ref()
            .is_some_and(|pr| pr.checked_ms != 0 && now_ms < pr.checked_ms + PR_POLL_MS)
        {
            return Ok(None);
        }
        // A project named by `root` has no remote in the pipeline; the
        // tree's own origin is what its PRs are against.
        let origin = self.git.remote_url(cwd)?;
        let target = match pr_target(t, p, stage, lane, provider, origin) {
            Ok(target) => target,
            Err(why) => {
                self.park(t, ps, &why, now_ms)?;
                return Ok(None);
            }
        };
        let head = self.git.head(cwd)?;
        Ok(Some((target, head)))
    }

    /// No pull request for the branch: the attempt's record of one
    /// cleared, and the question to ask.
    fn no_pr(
        &mut self,
        t: &mut Ticket,
        a: &Attempt,
        target: &PrTarget,
        now_ms: u64,
    ) -> Result<String> {
        record_of(t, &a.stage, a.n).pr = None;
        self.save_ticket(t, now_ms)?;
        Ok(format!(
            "no pull request for branch {} in {}; open one, then answer recheck",
            target.branch, target.repo
        ))
    }

    /// The end of a PR-reading poll that met a problem: the `pr`
    /// question, answered by a recheck or a park.
    fn ask_pr(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        a: &Attempt,
        question: &str,
        now_ms: u64,
    ) -> Result<()> {
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

    /// One reading of the PR for an open `pr-checks` attempt.
    fn poll_pr_checks(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        poll: &PrPoll<'_>,
        now_ms: u64,
    ) -> Result<()> {
        let PrPoll {
            stage,
            attempt: a,
            cwd,
            lane,
        } = *poll;
        let Some(Gate::External {
            provider, checks, ..
        }) = &stage.gate
        else {
            return Ok(());
        };
        let none_expected = checks.as_deref() == Some("none");
        let Some((target, head)) =
            self.pr_poll_target(t, ps, p, poll, provider.as_deref(), now_ms)?
        else {
            return Ok(());
        };
        let reading = self.read_pr(&target, none_expected);
        let question = match reading {
            Err(e) => match self.record_pr_error(t, a, &target, &head, &e, now_ms)? {
                None => return Ok(()),
                Some(question) => question,
            },
            Ok(None) => self.no_pr(t, a, &target, now_ms)?,
            Ok(Some((pr, _)))
                if pr.state == "open" && pr.mergeable.as_deref() == Some("conflicting") =>
            {
                record_of(t, &a.stage, a.n).pr = Some(PullRequestRecord {
                    number: pr.number,
                    url: pr.url.clone(),
                    checks: "conflicting".into(),
                    ..target.record(&pr.head, now_ms)
                });
                self.save_ticket(t, now_ms)?;
                let a = record_of(t, &a.stage, a.n).clone();
                return self.remedy(t, ps, p, &a, cwd, lane, &pr, &Remedy::Rebase, now_ms);
            }
            Ok(Some((pr, checks))) => {
                let young = now_ms < head_moved_ms(t, a).saturating_add(PR_YOUNG_HEAD_MS);
                let (summary, verdict) =
                    judge_pr(&pr, checks.as_ref(), &head, none_expected, young);
                let attempt = record_of(t, &a.stage, a.n);
                attempt.pr = Some(PullRequestRecord {
                    number: pr.number,
                    url: pr.url.clone(),
                    checks: summary.clone(),
                    ..target.record(&pr.head, now_ms)
                });
                // Red checks at the tree's head are the fixer's, when
                // the policy names one.
                if let (Err(_), Some(Checks::Failed(names))) = (&verdict, &checks)
                    && same_commit(&pr.head, &head)
                {
                    self.save_ticket(t, now_ms)?;
                    let a = record_of(t, &a.stage, a.n).clone();
                    let remedy = Remedy::Fix(names.clone());
                    return self.remedy(t, ps, p, &a, cwd, lane, &pr, &remedy, now_ms);
                }
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
        self.ask_pr(t, ps, a, &question, now_ms)
    }

    /// The PR for a branch and, when it is open and checks are
    /// expected, what its checks say.
    #[allow(clippy::type_complexity)]
    fn read_pr(
        &self,
        target: &PrTarget,
        none_expected: bool,
    ) -> Result<Option<(crate::github::PullRequest, Option<Checks>)>> {
        let Some(pr) = self.find_pr(target)? else {
            return Ok(None);
        };
        let prs = self.prs_for(&target.provider);
        if pr.state != "open" || none_expected {
            return Ok(Some((pr, None)));
        }
        let checks = prs.checks(&target.repo, pr.number);
        self.health.borrow_mut().gh_call(&checks);
        Ok(Some((pr, Some(checks?))))
    }

    /// The target's pull request, by number when the ticket's source
    /// gave one, else by branch; `None` when the branch has none.
    fn find_pr(&self, target: &PrTarget) -> Result<Option<crate::github::PullRequest>> {
        let prs = self.prs_for(&target.provider);
        let found = match target.number {
            Some(n) => prs.by_number(&target.repo, n).map(Some),
            None => prs.find(&target.repo, &target.branch),
        };
        self.health.borrow_mut().gh_call(&found);
        found
    }

    /// The provider a target names.
    pub(crate) fn prs_for(&self, provider: &str) -> &dyn PullRequests {
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
        let Some(contexts) = self.contexts_or_park(t, ps, p, stage, now_ms)? else {
            return Ok(());
        };
        // Someone else's branches are read as they are now, once per
        // opening of the gate.
        if t.source.is_pull_request() && !t.attempts_of(&stage.name).any(Attempt::is_open) {
            self.refresh_pull_requests(t, ps, p, now_ms)?;
            if !t.active() {
                return Ok(());
            }
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
            self.advance(t, now_ms)?;
        }
        Ok(())
    }

    /// A rebaser at work in the context is what is watched; true while
    /// one is open.
    #[allow(clippy::too_many_arguments)]
    fn poll_rebaser(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        ctx: &str,
        cwd: &Path,
        lane: Option<&str>,
        now_ms: u64,
    ) -> Result<bool> {
        let open = t
            .attempts
            .iter()
            .filter(|a| a.stage == stage.name && a.context == ctx && a.kind == AttemptKind::Agent)
            .max_by_key(|a| a.n)
            .filter(|a| a.is_open())
            .cloned();
        let Some(a) = open else {
            return Ok(false);
        };
        let trust = p.policy.trust_folders;
        self.poll_agent(t, ps, p, &a, stage, cwd, lane, trust, now_ms)?;
        Ok(true)
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
            .filter(|a| {
                a.stage == stage.name && a.context == ctx && a.kind == AttemptKind::GateOnly
            })
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
            let pr = t.source.pull_requests.iter().find(|pr| pr.lane == l.name);
            let base = match pr {
                Some(pr) if !pr.base.is_empty() => format!("{}/{}", pr.remote, pr.base),
                _ => p.lane(&l.name).map_or_else(
                    || p.project.base.clone(),
                    |lane| format!("{}/{}", p.lane_remote(lane), p.lane_base(lane)),
                ),
            };
            let short: String = head.chars().take(8).collect();
            let _ = write!(q, " branch {} at {short} over {base}.", l.branch);
            let summary = self.git.summary(cwd, &base)?;
            if !summary.trim().is_empty() {
                q.push_str("\n\n");
                q.push_str(summary.trim());
            }
        }
        let _ = write!(q, "\n\nTree: {}", cwd.display());
        if let Some((stage, notes)) = t.input_with_stage("notes") {
            let _ = write!(q, "\nNotes ({stage}): {}", notes.display());
        }
        q.push_str(&deployed_and_served(t, p));
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
        let Some(a) = find_attempt(t, stage, *n).cloned() else {
            return Ok(());
        };
        let head = match tree_of(t, p, &a.context) {
            Some(cwd) => Some(self.git.head(&cwd)?),
            None => None,
        };
        let record = record_of(t, &a.stage, a.n);
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
        let Some(gate) = find_attempt(t, stage, *n).cloned() else {
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
        let reason = format!("{SENT_BACK_FROM}{from}: {note}");
        let record = record_of(t, &gate.stage, gate.n);
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
        let Some(contexts) = self.contexts_or_park(t, ps, p, stage, now_ms)? else {
            return Ok(());
        };
        let mut all_complete = true;
        for (ctx, cwd, lane) in contexts {
            if self.poll_rebaser(t, ps, p, stage, &ctx, &cwd, lane.as_deref(), now_ms)? {
                all_complete = false;
                continue;
            }
            let Some(attempt) = self.open_gate_attempt(t, stage, &ctx, now_ms)? else {
                continue;
            };
            all_complete = false;
            let poll = PrPoll {
                stage,
                attempt: &attempt,
                cwd: &cwd,
                lane: lane.as_deref(),
            };
            self.poll_pr_merged(t, ps, p, &poll, decision, now_ms)?;
            if !t.active() {
                return Ok(());
            }
        }
        if all_complete {
            self.advance(t, now_ms)?;
        }
        Ok(())
    }

    /// One reading of the PR for an open `pr-merged` attempt, whose
    /// question while the PR is open is `decision`.
    fn poll_pr_merged(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        poll: &PrPoll<'_>,
        decision: &str,
        now_ms: u64,
    ) -> Result<()> {
        let PrPoll {
            stage,
            attempt: a,
            cwd,
            lane,
        } = *poll;
        let provider = match &stage.gate {
            Some(Gate::External { provider, .. }) => provider.as_deref(),
            _ => None,
        };
        let Some((target, head)) = self.pr_poll_target(t, ps, p, poll, provider, now_ms)? else {
            return Ok(());
        };
        let found = self
            .prs_for(&target.provider)
            .find(&target.repo, &target.branch);
        self.health.borrow_mut().gh_call(&found);
        let question = match found {
            Err(e) => match self.record_pr_error(t, a, &target, &head, &e, now_ms)? {
                None => return Ok(()),
                Some(question) => question,
            },
            Ok(None) => self.no_pr(t, a, &target, now_ms)?,
            Ok(Some(pr)) => {
                record_of(t, &a.stage, a.n).pr = Some(PullRequestRecord {
                    number: pr.number,
                    url: pr.url.clone(),
                    checks: pr.state.clone(),
                    ..target.record(&pr.head, now_ms)
                });
                self.save_ticket(t, now_ms)?;
                match pr.state.as_str() {
                    "merged" => return self.merged(t, ps, a, decision, &pr, now_ms),
                    "closed" => format!("PR #{} is closed without being merged", pr.number),
                    _ if pr.mergeable.as_deref() == Some("conflicting") => {
                        let a = record_of(t, &a.stage, a.n).clone();
                        let remedy = Remedy::Rebase;
                        return self.remedy(t, ps, p, &a, cwd, lane, &pr, &remedy, now_ms);
                    }
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
        self.ask_pr(t, ps, a, &question, now_ms)
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
        let record = record_of(t, &a.stage, a.n);
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
        record_of(t, &a.stage, a.n).pr = Some(PullRequestRecord {
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

    fn finish_gate_only(&mut self, t: &mut Ticket, stage: &Stage, now_ms: u64) -> Result<()> {
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
        self.advance(t, now_ms)
    }

    pub(crate) fn advance(&mut self, t: &mut Ticket, now_ms: u64) -> Result<()> {
        t.stage += 1;
        self.record_entry(t, now_ms);
        self.save_ticket(t, now_ms)
    }

    // --- contexts and projects

    /// The stage's contexts, or `None` once the ticket is parked because
    /// the stage needs lanes it has not cut.
    pub(crate) fn contexts_or_park(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        now_ms: u64,
    ) -> Result<Option<Contexts>> {
        let contexts = Self::contexts(t, p, stage);
        if contexts.is_empty() {
            self.park(
                t,
                ps,
                &format!("stage {} needs lanes the ticket has not cut", stage.name),
                now_ms,
            )?;
            return Ok(None);
        }
        Ok(Some(contexts))
    }

    /// The contexts a stage runs in: `(name, cwd, lane)`. Empty when the
    /// stage needs lanes the ticket has not cut. A named lane runs only
    /// when it is chosen; every lane's tree is cut, so one not chosen
    /// would otherwise run a stage the ticket's work never touched.
    pub(crate) fn contexts(t: &Ticket, p: &Pipeline, stage: &Stage) -> Contexts {
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
                .filter(|l| &l.name == name && l.chosen)
                .map(|l| (l.name.clone(), l.worktree.clone(), Some(l.name.clone())))
                .collect(),
            Context::Lanes(names) => t
                .lanes
                .iter()
                .filter(|l| names.contains(&l.name) && l.chosen)
                .map(|l| (l.name.clone(), l.worktree.clone(), Some(l.name.clone())))
                .collect(),
        }
    }

    /// Whether a stage runs nowhere: it names one lane or a list of
    /// lanes, and the ticket chose none of them (or did not cut them).
    /// `contexts` is empty for it too, but a skipped stage advances
    /// where an `each` stage with no lane parks.
    #[must_use]
    pub(crate) fn skipped(t: &Ticket, stage: &Stage) -> bool {
        let chosen = |name: &String| t.lanes.iter().any(|l| &l.name == name && l.chosen);
        match &stage.context {
            Context::Lane(lane) => !chosen(lane),
            Context::Lanes(lanes) => !lanes.iter().any(chosen),
            _ => false,
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
        if let Some(space) = self.known_space(ps, p)? {
            return Ok(space);
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

    /// The Switchboard workspace the pipeline names, when the record
    /// has it or Switchboard lists it (saved then); `None` when it has
    /// yet to be made.
    pub(crate) fn known_space(
        &mut self,
        ps: &mut ProjectState,
        p: &Pipeline,
    ) -> Result<Option<String>> {
        if let Some(space) = &ps.space {
            return Ok(Some(space.clone()));
        }
        if let Reply::Spaces { spaces } = self.ask(Body::Spaces)?
            // The global space is listed as a view; it holds no projects.
            && let Some(s) = spaces
                .iter()
                .find(|s| !s.view && s.name == p.project.space)
        {
            ps.space = Some(s.id.clone());
            self.save_project(ps)?;
            return Ok(Some(s.id.clone()));
        }
        Ok(None)
    }

    /// The ticket's one Switchboard project, `#<n> <title>`, rooted at
    /// the ticket's tree; sessions in other lanes carry their own cwd.
    pub(crate) fn ensure_project(
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
        now_ms: u64,
    ) -> Result<()> {
        // A command gate runs after the agent, in its context; an agent
        // stage with any other gate parks.
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
        let Some(contexts) = self.contexts_or_park(t, ps, p, stage, now_ms)? else {
            return Ok(());
        };
        // The stage's services come up before any of its agents starts;
        // an agent already running is watched either way.
        let served = stage.services.is_empty() || self.ensure_services(t, ps, p, stage, now_ms)?;
        if !t.active() {
            return Ok(());
        }
        let mut all_complete = true;
        for (ctx, cwd, lane) in contexts {
            let last = latest_attempt(t, &stage.name, &ctx).cloned();
            match last {
                Some(a) if a.state == AttemptState::Complete => {}
                Some(a) if a.is_open() => {
                    all_complete = false;
                    let trust = p.policy.trust_folders;
                    self.poll_agent(t, ps, p, &a, stage, &cwd, lane.as_deref(), trust, now_ms)?;
                }
                Some(a) => {
                    // Failed: a rerun waits on its decision, asked again
                    // if none is open (after a park). Sent back from a
                    // later human gate: the note is the answer.
                    all_complete = false;
                    let held = !served || held_in(t, &stage.name, &ctx);
                    let sent_back = sent_back(t, stage, &ctx);
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
                    } else if served && asks_again(t, &a, sent_back) {
                        // Asked once its services are up, so the answer
                        // is about a run that could start.
                        self.ask_rerun(t, ps, &a, now_ms)?;
                    }
                }
                None => {
                    all_complete = false;
                    if served && !held_in(t, &stage.name, &ctx) {
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
            self.advance(t, now_ms)?;
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
        let operator = stage.operator.clone().unwrap_or_default();
        if !p.operators.contains_key(&operator) {
            return self.park(
                t,
                ps,
                &format!("stage {} names no operator", stage.name),
                now_ms,
            );
        }
        let dir = self.attempt_dir(t, &stage.name, n, ctx)?;
        let artifacts = artifact_paths(stage, &dir);
        let mut vars = vars_for(t, p, lane);
        for (name, path) in &artifacts {
            vars.set(name.clone(), path.display().to_string());
        }
        let guidance = &p.operators[&operator].guidance;
        let template = stage.prompt.as_deref().unwrap_or_default();
        let env = input_env(&vars, &format!("{guidance}\n{template}"));
        let mut prompt = guidance_prelude(guidance, &vars);
        prompt.push_str(&vars.render(template));
        if let Some(moved) = lane
            .and_then(|l| t.lanes.iter().find(|x| x.name == l))
            .and_then(|l| l.refreshed.as_ref())
        {
            let _ = write!(
                prompt,
                "\n\nSince the plan was written the base moved from {} to {}, and the branch was brought up to it; read git log {}..{} for what changed.",
                moved.from, moved.to, moved.from, moved.to
            );
        }
        // The note is taken off only with the attempt that carries it:
        // a launch that stops short (a setup still running) keeps it.
        let key = rework_key(&stage.name, ctx);
        let rework = t.rework.get(&key).map(|note| {
            prompt.push_str("\n\nThe user looked at the previous attempt and sent it back: ");
            prompt.push_str(note);
            key
        });
        let spec = AgentSpec {
            operator,
            prompt,
            artifacts,
            clone_of: None,
            pr: None,
            rework,
            env,
        };
        self.launch_agent(t, ps, p, &stage.name, ctx, cwd, n, spec, now_ms)
    }

    /// The attempt's own directory under the ticket's, made.
    pub(crate) fn attempt_dir(
        &self,
        t: &Ticket,
        stage: &str,
        n: u32,
        ctx: &str,
    ) -> Result<PathBuf> {
        let dir = self
            .data
            .ticket_dir(&t.id)
            .join(stage)
            .join(n.to_string())
            .join(ctx);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// One agent attempt started: the record and its request written
    /// together. A `clone_of` makes the session from that session's
    /// transcript instead of fresh, so it knows what it is continuing.
    #[allow(clippy::too_many_arguments)]
    fn launch_agent(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &str,
        ctx: &str,
        cwd: &Path,
        n: u32,
        spec: AgentSpec,
        now_ms: u64,
    ) -> Result<()> {
        let operator = &p.operators[&spec.operator];
        let project = match self.ensure_project(t, ps, p, now_ms) {
            Ok(id) => id,
            // Switchboard refusing the project parks the ticket; the
            // socket failing says nothing about the ticket, so the
            // pass ends and the next one asks again.
            Err(e) if e.is::<SocketDown>() => return Err(e),
            Err(e) => return self.park(t, ps, &format!("stage {stage}: {e:#}"), now_ms),
        };
        if !self.ensure_setup(t, ps, p, cwd, now_ms)? {
            return Ok(());
        }
        let dir = self.attempt_dir(t, stage, n, ctx)?;
        let mut attempt = new_attempt(
            stage,
            n,
            ctx,
            AttemptKind::Agent,
            AttemptState::Starting,
            spec.artifacts,
            now_ms,
        );
        attempt.project = Some(project.clone());
        attempt.pr = spec.pr;
        // Not saved here: `send` writes the attempt and its request in
        // one go, so no record ever shows the one without the other.
        t.attempts.push(attempt);
        if let Some(key) = &spec.rework {
            t.rework.remove(key);
        }
        let notes = format!(
            "Dispatch ticket {} · #{} {} · stage {stage} attempt {n}",
            t.id,
            t.source.number.unwrap_or(0),
            t.source.title,
        );
        let body = if let Some(source) = spec.clone_of {
            Body::SessionClone {
                source,
                name: spec.operator,
                prompt: spec.prompt,
                notes,
            }
        } else {
            // The artifacts live outside the agent's cwd, in Dispatch's
            // own directory; Claude Code writes there unasked only under
            // an allow rule for the path (an added directory still asks
            // before creating a file).
            let mut args = operator.args.clone();
            args.extend(operator.kind.write_flags(&dir));
            let launch = if args.is_empty() {
                wire::Launch::Shell
            } else {
                wire::Launch::Argv(args)
            };
            Body::SessionNew {
                project,
                name: spec.operator,
                session_kind: session_kind(operator.kind),
                cwd: cwd.to_path_buf(),
                launch,
                prompt: Some(spec.prompt),
                notes,
                env: spec.env,
            }
        };
        let reply = self.send(t, ps, Some((stage.to_owned(), n)), "session", body, now_ms)?;
        if let Reply::Failed { reason } = reply {
            self.fail_attempt(
                t,
                ps,
                stage,
                n,
                &format!("could not start: {reason}"),
                now_ms,
            )?;
        }
        Ok(())
    }

    /// A PR the provider says cannot go in as it stands (it conflicts
    /// with its base, or its checks are red): the policy's operator for
    /// that runs in the lane, cloned from the lane's last finished agent
    /// so it knows the change, once per head and at most the policy's
    /// cap; otherwise, or without such an operator, it is a question.
    #[allow(clippy::too_many_arguments)]
    fn remedy(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        a: &Attempt,
        cwd: &Path,
        lane: Option<&str>,
        pr: &crate::github::PullRequest,
        remedy: &Remedy,
        now_ms: u64,
    ) -> Result<()> {
        // A failed agent's rerun question is answered first.
        if t.decisions
            .iter()
            .any(|d| d.pending() && d.stage == a.stage && d.name == "rerun")
        {
            return Ok(());
        }
        if let Some(why) = remedy_reason(t, p, a, pr, remedy) {
            return self.ensure_decision(
                t,
                ps,
                Ask {
                    stage: &a.stage,
                    name: "pr",
                    kind: DecisionKind::Permission,
                    question: format!(
                        "{} ({}): PR #{} {}; {why}",
                        a.stage,
                        a.context,
                        pr.number,
                        remedy.problem()
                    ),
                    options: &["recheck", "park"],
                    recommendation: None,
                    attempt: Some((a.stage.clone(), a.n)),
                },
                now_ms,
            );
        }
        self.start_remedy(t, ps, p, a, cwd, lane, pr, remedy, now_ms)
    }

    /// The remedy's operator started in the lane, cloned from the lane's
    /// last finished agent, with the PR's head on its record.
    #[allow(clippy::too_many_arguments)]
    fn start_remedy(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        a: &Attempt,
        cwd: &Path,
        lane: Option<&str>,
        pr: &crate::github::PullRequest,
        remedy: &Remedy,
        now_ms: u64,
    ) -> Result<()> {
        let operator = remedy.operator(p).unwrap_or_default();
        let n = next_n(t, &a.stage);
        let dir = self.attempt_dir(t, &a.stage, n, &a.context)?;
        let notes = dir.join("notes.md");
        let lane_record = lane.and_then(|l| t.lanes.iter().find(|x| x.name == l));
        let base = lane_record.and_then(|l| p.lane(&l.name)).map_or_else(
            || p.project.base.clone(),
            |l| format!("{}/{}", p.lane_remote(l), p.lane_base(l)),
        );
        let mut vars = vars_for(t, p, lane);
        vars.set("notes", notes.display().to_string());
        let mut prompt = guidance_prelude(&p.operators[&operator].guidance, &vars);
        let plan = t
            .input("plan")
            .map(|plan| format!(" (the plan is at {})", plan.display()))
            .unwrap_or_default();
        let branch = lane_record.map_or("", |l| l.branch.as_str());
        let _ = write!(
            prompt,
            "PR #{} ({}) for branch {branch} ",
            pr.number, pr.url
        );
        match remedy {
            Remedy::Rebase => {
                let _ = write!(
                    prompt,
                    "conflicts with {base}. In {}: fetch, rebase the branch onto {base}, resolve every conflict keeping the change's intent{plan}, run the checks, then push with --force-with-lease.",
                    cwd.display()
                );
            }
            Remedy::Fix(names) => {
                let _ = write!(
                    prompt,
                    "has failing checks: {}. In {}: read why they failed (gh pr checks {} and gh run view --log-failed for the failed run), fix the cause on the branch keeping the change's intent{plan}, run the checks here, commit, then push. Fix the change, not the checks, unless the check itself is what this change adds.",
                    names.join(", "),
                    cwd.display(),
                    pr.number
                );
            }
        }
        let _ = write!(
            prompt,
            " Write what you did and why to {}.",
            notes.display()
        );
        let clone_of = lane_clone_of(t, &a.context);
        let mut record = PullRequestRecord {
            provider: String::new(),
            repo: String::new(),
            number: pr.number,
            url: pr.url.clone(),
            head: pr.head.clone(),
            checks: remedy.tag(),
            checked_ms: now_ms,
            error_since_ms: None,
        };
        if let Some(seen) = &a.pr {
            record.provider.clone_from(&seen.provider);
            record.repo.clone_from(&seen.repo);
        }
        log::info!(
            "ticket {} {}/{} PR #{} {}; {operator} starting{}",
            t.id,
            a.stage,
            a.context,
            pr.number,
            remedy.problem(),
            clone_of
                .as_deref()
                .map(|s| format!(" from {s}"))
                .unwrap_or_default()
        );
        let spec = AgentSpec {
            operator,
            prompt,
            artifacts: BTreeMap::from([("notes".to_owned(), notes)]),
            clone_of,
            pr: Some(record),
            rework: None,
            env: BTreeMap::new(),
        };
        let (stage_name, ctx) = (a.stage.clone(), a.context.clone());
        self.launch_agent(t, ps, p, &stage_name, &ctx, cwd, n, spec, now_ms)
    }

    /// What Switchboard says about the attempt's session, and what that
    /// makes of the attempt.
    #[allow(clippy::too_many_arguments)]
    fn poll_agent(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
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
            return self.poll_gate(t, ps, p, a, stage, cwd, lane, now_ms);
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
        if !observe_stop(attempt, &view) {
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
        // A Stop with the card still `working` ended a turn, not the
        // work: background agents or a tool are still at it, and the
        // artifact may land in a later turn.
        if busy(&view) {
            attempt.polls_since_stop = 0;
            return self.save_ticket(t, now_ms);
        }
        let running = view.liveness == wire::Liveness::Running;
        let missing = missing_artifacts(attempt);
        if !missing.is_empty() {
            attempt.polls_since_stop = idle_polls(&view, attempt.polls_since_stop);
            if !running || attempt.polls_since_stop >= STOP_IDLE_POLLS {
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
        // Before the kill: a dirty tree may yet be committed by the
        // agent in the same session, once told.
        if gated && !self.git.is_clean(cwd)? {
            return self.dirty_after_agent(t, ps, a, stage, idx, session, running, cwd, now_ms);
        }
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
            let role = attempt.pr.as_ref().and_then(remedy_role);
            // The head is for the log line only, so a failed read must
            // not fail the poll.
            let head = role.and_then(|_| self.git.head(cwd).ok());
            log::info!(
                "{}",
                completion_line(&t.id, &a.stage, &a.context, role, head.as_deref())
            );
        }
        self.save_ticket(t, now_ms)?;
        if running {
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
            self.start_gate(t, ps, p, a, stage, cwd, lane, now_ms)?;
        }
        Ok(())
    }

    /// A gated agent stopped (or never answered its nudge) with its
    /// tree not clean: a nudge into its session while the stage's
    /// `on_dirty` has one left for a stop, else the session ends and
    /// the attempt fails at its checks, which asks.
    #[allow(clippy::too_many_arguments)]
    fn dirty_after_agent(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        a: &Attempt,
        stage: &Stage,
        idx: usize,
        session: String,
        running: bool,
        cwd: &Path,
        now_ms: u64,
    ) -> Result<()> {
        let on_dirty = self.pipeline_of(t)?.on_dirty(stage);
        let attempt = &t.attempts[idx];
        let step = dirty_step(
            on_dirty,
            &attempt.nudges,
            attempt.stop_at_ms,
            attempt.polls_since_stop,
            running,
            now_ms,
        );
        let tree = format!("the tree at {} is not clean after the agent", cwd.display());
        let suffix = match step {
            DirtyStep::Nudge { at_ms, k, of } => {
                let attempt = &mut t.attempts[idx];
                attempt.nudges.push(at_ms);
                attempt.polls_since_stop = 0;
                let key = (a.stage.clone(), a.n);
                match self.send_nudge(t, ps, key, &a.context, session.clone(), (k, of), now_ms)? {
                    None => return Ok(()),
                    Some(failed) => failed,
                }
            }
            DirtyStep::Fail(suffix) => suffix,
        };
        let reason = format!("{tree}{suffix}");
        self.save_ticket(t, now_ms)?;
        if running {
            self.send(
                t,
                ps,
                Some((a.stage.clone(), a.n)),
                "kill",
                Body::SessionKill { session },
                now_ms,
            )?;
        }
        self.fail_checks(t, ps, &a.stage, a.n, &reason, now_ms)
    }

    /// Types `NUDGE_TEXT` into `session`, nudge `k` of `of`; the caller
    /// has recorded it first. When Switchboard refused it, `Some` of the
    /// clause the dirty-tree failure ends with.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn send_nudge(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        key: (String, u32),
        context: &str,
        session: String,
        (k, of): (usize, u32),
        now_ms: u64,
    ) -> Result<Option<String>> {
        log::info!(
            "ticket {} {}/{context} tree not clean after the stop; nudge {k} of {of}",
            t.id,
            key.0
        );
        let body = Body::SessionSend {
            session,
            text: NUDGE_TEXT.into(),
        };
        match self.send(t, ps, Some(key), NUDGE, body, now_ms)? {
            Reply::Failed { reason } => Ok(Some(format!(
                "{}; the nudge failed: {reason}",
                dirty_suffix(k - 1)
            ))),
            _ => Ok(None),
        }
    }

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

    /// A session as Switchboard sees it, or why it has none.
    pub(crate) fn session_view(
        &mut self,
        session: &str,
    ) -> Result<Result<wire::SessionView, String>> {
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
        p: &Pipeline,
        a: &Attempt,
        stage: &Stage,
        cwd: &Path,
        lane: Option<&str>,
        now_ms: u64,
    ) -> Result<()> {
        let Some(gate @ Gate::Command { .. }) = &stage.gate else {
            return Ok(());
        };
        let argv = lane_gate_argv(gate, lane).cloned();
        let Some(argv) = argv.filter(|v| !v.is_empty()) else {
            let reason = format!("no checks command for context {}", a.context);
            return self.fail_attempt(t, ps, &a.stage, a.n, &reason, now_ms);
        };
        // A setup a restart changed runs before the checks a `check`
        // answer starts, which launch no agent to run it first. Failing
        // fails the checks rather than parking, so `check` is offered
        // again once the setup is fixed.
        if let Some(reason) = self.run_setup(t, p, cwd, now_ms)? {
            return self.fail_gate(t, ps, a, &reason, now_ms);
        }
        if !self.git.is_clean(cwd)? {
            let reason = if a.kind == AttemptKind::GateOnly {
                format!("the tree at {} is not clean", cwd.display())
            } else {
                format!("the tree at {} is not clean after the agent", cwd.display())
            };
            return self.fail_gate(t, ps, a, &reason, now_ms);
        }
        let head = self.git.head(cwd)?;
        let dir = self.attempt_dir(t, &a.stage, a.n, &a.context)?;
        let log = dir.join("checks.log");
        let lane_record = lane.and_then(|l| t.lanes.iter().find(|x| x.name == l));
        let mut env = checks_env(
            t,
            lane_record.map(|l| l.name.as_str()),
            lane_record.map(|l| l.branch.as_str()),
            a,
            cwd,
            &head,
        );
        if a.kind == AttemptKind::GateOnly
            && let Err(e) = prepare_writes(a, &dir, &mut env)
        {
            let reason = format!("the command's artifacts could not be prepared: {e:#}");
            return self.fail_attempt(t, ps, &a.stage, a.n, &reason, now_ms);
        }
        let key = gate_key(t, a);
        let started = match confine_for(t, p, lane, &[&dir], gate_network(p, stage)) {
            Some(confine) => self
                .git
                .start_check_confined(&key, cwd, &argv, &env, &log, &confine),
            None => self.git.start_check(&key, cwd, &argv, &env, &log),
        };
        if let Err(e) = started {
            let reason = format!("the checks could not start: {e:#}");
            return self.fail_attempt(t, ps, &a.stage, a.n, &reason, now_ms);
        }
        log::info!(
            "ticket {} {}/{} checks started at {head}",
            t.id,
            a.stage,
            a.context
        );
        if let Some(attempt) = find_attempt_mut(t, &a.stage, a.n) {
            attempt.gate = Some(GateRun {
                head,
                argv,
                log: log.clone(),
                started_ms: now_ms,
                exit: None,
                group: self.git.check_group(&key),
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
        p: &Pipeline,
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
            // A gate-only command may deploy: lost with the runner, it
            // may have run, so it is a question, never a second run.
            Some(Err(e)) if a.kind == AttemptKind::GateOnly => {
                log::warn!(
                    "ticket {} {}/{} command lost ({e:#})",
                    t.id,
                    a.stage,
                    a.context
                );
                let reason = "the runner restarted while it ran; it may have run";
                return self.fail_attempt(t, ps, &a.stage, a.n, reason, now_ms);
            }
            Some(Err(e)) => {
                // What a previous runner left running is stopped before
                // the checks start again in the same tree.
                if let GateStop::Waiting = self.stop_gate(t, a, now_ms)? {
                    return Ok(());
                }
                log::warn!(
                    "ticket {} {}/{} checks lost ({e:#}); starting again",
                    t.id,
                    a.stage,
                    a.context
                );
                if let Some(attempt) = find_attempt_mut(t, &a.stage, a.n) {
                    attempt.gate = None;
                }
                return self.start_gate(t, ps, p, a, stage, cwd, lane, now_ms);
            }
        };
        let clean = self.git.is_clean(cwd)?;
        let head = self.git.head(cwd)?;
        if let Some(attempt) = find_attempt_mut(t, &a.stage, a.n)
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
            return self.fail_gate(t, ps, a, &reason, now_ms);
        }
        if code != 0 {
            let reason = checks_reason(code, &gate.log);
            return self.fail_gate(t, ps, a, &reason, now_ms);
        }
        if a.kind == AttemptKind::GateOnly
            && let Some(reason) = seal_writes(a)
        {
            return self.fail_gate(t, ps, a, &reason, now_ms);
        }
        if let Some(attempt) = find_attempt_mut(t, &a.stage, a.n) {
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

    /// A command gate's failure: an agent stage's checks can run again
    /// on the same attempt (`check`); a gate-only command has no agent
    /// whose work would stand, so its options are `rerun` and `park`.
    fn fail_gate(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        a: &Attempt,
        reason: &str,
        now_ms: u64,
    ) -> Result<()> {
        if a.kind == AttemptKind::GateOnly {
            self.fail_attempt(t, ps, &a.stage, a.n, reason, now_ms)
        } else {
            self.fail_checks(t, ps, &a.stage, a.n, reason, now_ms)
        }
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
    pub(crate) fn fail_checks(
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

    /// A failure of a code review's rewrite of its commits, with the
    /// branch still at the head its checks passed at: the work stands,
    /// so the stage can complete there with the history as it is.
    pub(crate) fn fail_rewrite(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        stage: &str,
        n: u32,
        reason: &str,
        now_ms: u64,
    ) -> Result<()> {
        self.fail_attempt_with(t, ps, stage, n, reason, &["rerun", "keep", "park"], now_ms)
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
        let Some(attempt) = find_attempt_mut(t, stage, n) else {
            return Ok(());
        };
        attempt.state = AttemptState::Failed {
            reason: reason.into(),
        };
        attempt.ended_ms = Some(now_ms);
        // Kept on the attempt, not only on the question: a park past
        // `max_reruns` asks nothing, and a resume asks afresh.
        attempt.failed_at_checks = options.contains(&"check");
        attempt.failed_at_rewrite = options.contains(&"keep");
        let ctx = attempt.context.clone();
        log::warn!("ticket {} {stage}/{ctx} attempt {n} failed: {reason}", t.id);
        self.save_ticket(t, now_ms)?;
        // Recovery fails a closing or parking ticket's lost launches
        // too. The close or the park is what happens next: a rerun
        // decision would ask about a ticket the user is stopping, and
        // parking would save a state over the intent already saved.
        if matches!(
            t.state,
            TicketState::Closing { .. } | TicketState::Parking { .. }
        ) {
            return Ok(());
        }
        // Failures under a copy a restart replaced say nothing about
        // the new one. Counted by when they ended, not started: a `check`
        // after a restart carries the earlier attempt into the new copy.
        let since = t.restarts.last().map_or(0, |r| r.at_ms);
        let failed = t
            .attempts
            .iter()
            .filter(|a| {
                a.stage == stage
                    && a.context == ctx
                    && a.ended_ms.unwrap_or(a.started_ms) >= since
                    && matches!(a.state, AttemptState::Failed { .. })
            })
            .count();
        let max_reruns = self.pipeline_of(t).map_or(3, |p| p.policy.max_reruns);
        let choices =
            find_attempt(t, stage, n).map_or_else(String::new, |a| rewrite_choices(a, options));
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
                question: format!(
                    "{stage} ({ctx}) attempt {n} failed: {reason}. Run it again?{}{}{choices}",
                    self.mid_rebase_note(t, stage, &ctx),
                    rerun_carries(t, stage, n)
                ),
                options,
                recommendation: None,
                attempt: Some((stage.to_owned(), n)),
            },
            now_ms,
        )
    }

    /// What a rerun question about a rebaser adds when it left its lane
    /// mid-rebase: a rerun cannot start over a stopped rebase.
    fn mid_rebase_note(&self, t: &Ticket, stage: &str, ctx: &str) -> String {
        if stage != REFRESH {
            return String::new();
        }
        let Some(lane) = t.lanes.iter().find(|l| l.name == ctx) else {
            return String::new();
        };
        if !matches!(self.git.rebase_in_progress(&lane.worktree), Ok(true)) {
            return String::new();
        }
        format!(
            " The worktree {} is mid-rebase{}; finish or abort it by hand, then answer rerun.",
            lane.worktree.display(),
            self.at_head(&lane.worktree)
        )
    }

    /// " at <sha>" for a mid-rebase tree's `HEAD`, or nothing when it
    /// cannot be read, so a question never names an empty commit.
    fn at_head(&self, dir: &Path) -> String {
        self.git
            .head(dir)
            .map(|h| format!(" at {h}"))
            .unwrap_or_default()
    }

    /// A failed or cancelled attempt with no rerun question open about
    /// it (after a park) is asked about again, under a new id, with the
    /// options its failure was first offered. Nothing launches until the
    /// answer, and no rerun is spent.
    pub(crate) fn ask_rerun(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        a: &Attempt,
        now_ms: u64,
    ) -> Result<()> {
        let Some((question, options)) = rerun_ask(t, a) else {
            return Ok(());
        };
        self.ensure_decision(
            t,
            ps,
            Ask {
                stage: &a.stage,
                name: "rerun",
                kind: DecisionKind::Permission,
                question,
                options,
                recommendation: None,
                attempt: Some((a.stage.clone(), a.n)),
            },
            now_ms,
        )
    }

    /// The stage's re-asks alone, for a ticket that may not start
    /// anything this pass: unanswered requests are recovered as `step`
    /// recovers them, a `park` answer is acted on, then the same
    /// question `asks_again` lets a stage ask, for each context's
    /// latest attempt. The other answers wait for `step`, since they
    /// may launch. Returns how many questions are now pending that were
    /// not before.
    fn ask_again_unslotted(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        now_ms: u64,
    ) -> Result<usize> {
        let before = t.pending_decisions().len();
        self.recover_unanswered(t, ps, now_ms)?;
        if let Some(i) = t
            .decisions
            .iter()
            .position(|d| d.unacted_answer() == Some("park"))
        {
            self.park_by_answer(t, ps, i, now_ms)?;
            return Ok(0);
        }
        // A gate-only command stage (a deploy) asks about a failed run
        // as an agent stage does; other gate-only stages ask nothing
        // here.
        if let Some(stage) = p.stages.get(t.stage)
            && (stage.kind() != StageKind::GateOnly
                || matches!(stage.gate, Some(Gate::Command { .. })))
        {
            for (ctx, _, _) in Self::contexts(t, p, stage) {
                let Some(a) = latest_attempt(t, &stage.name, &ctx).cloned() else {
                    continue;
                };
                if asks_again(t, &a, sent_back(t, stage, &ctx)) {
                    self.ask_rerun(t, ps, &a, now_ms)?;
                }
            }
        }
        Ok(t.pending_decisions().len().saturating_sub(before))
    }

    // --- workflow stages

    fn workflow_stage(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        now_ms: u64,
    ) -> Result<()> {
        let Some(contexts) = self.contexts_or_park(t, ps, p, stage, now_ms)? else {
            return Ok(());
        };
        let mut all_complete = true;
        for (ctx, _cwd, lane) in contexts {
            let last = latest_attempt(t, &stage.name, &ctx).cloned();
            match last {
                Some(a) if a.state == AttemptState::Complete => {}
                Some(a) if a.is_open() => {
                    all_complete = false;
                    self.poll_workflow(t, ps, p, stage, &a, now_ms)?;
                }
                Some(a) => {
                    all_complete = false;
                    let held = held_in(t, &stage.name, &ctx);
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
                    } else if asks_again(t, &a, sent_back(t, stage, &ctx)) {
                        self.ask_rerun(t, ps, &a, now_ms)?;
                    }
                }
                None => {
                    all_complete = false;
                    if !held_in(t, &stage.name, &ctx) {
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
            self.advance(t, now_ms)?;
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
        let dir = self.attempt_dir(t, &stage.name, n, ctx)?;
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
            Reply::Failed { reason } if reason == NO_SUCH_RUN => {
                return self.fail_attempt(
                    t,
                    ps,
                    &a.stage,
                    a.n,
                    &format!("review run gone: {reason}"),
                    now_ms,
                );
            }
            // The app could not answer (busy, timed out on its own
            // thread): nothing is known about the run, so ask again.
            Reply::Failed { reason } => {
                log::warn!(
                    "ticket {} {}/{}: workflow query failed: {reason}; asking again",
                    t.id,
                    a.stage,
                    a.context
                );
                return self.save_ticket(t, now_ms);
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
    /// slots allow, then the queue view, each in a transaction of its
    /// own.
    ///
    /// The writer lock is held per ticket step, not for the pass: a
    /// step may fetch, read a provider or launch an agent, and an
    /// answer from the terminal or the window must not wait behind
    /// every ticket's slow work. Each step re-reads its ticket and the
    /// project under the lock, so what landed between steps is seen.
    pub fn step_project(&mut self, project: &str, now_ms: u64) -> Result<()> {
        // Read and pruned under the lock: a `take` or a reorder landing
        // between the read and the write would otherwise be lost.
        let mut tickets = self.transaction(|r| {
            let mut ps = r.load_project(project)?;
            let mut tickets: Vec<Ticket> = Vec::new();
            for id in ps.queue.iter().chain(&ps.closing) {
                if tickets.iter().any(|t| &t.id == id) {
                    continue;
                }
                match r.load_ticket(id) {
                    Ok(t) => tickets.push(t),
                    Err(e) => log::warn!("ticket {id}: {e}"),
                }
            }
            // A close finished but not yet saved to the project leaves
            // the id behind; a closed ticket is in neither list.
            let before = ps.clone();
            for t in &tickets {
                if matches!(t.state, TicketState::Closed { .. }) {
                    ps.queue.retain(|id| id != &t.id);
                    ps.closing.retain(|id| id != &t.id);
                }
            }
            requeue_strays(&mut ps, &tickets);
            if ps != before {
                r.save_project(&ps)?;
            }
            Ok(tickets)
        })?;
        tickets.retain(|t| !matches!(t.state, TicketState::Closed { .. }));
        let mut running = 0u32;
        let mut pending = 0u32;
        for t in &tickets {
            if ticket_costs_slot(t) {
                running += 1;
            }
            pending += u32::try_from(t.waiting_on_you().len()).unwrap_or(u32::MAX);
        }
        // Slots and the decision limit are operational knobs, not part
        // of what a ticket was promised, so they are read from the
        // project's live pipeline file: raising `slots` takes effect on
        // the next pass, for every ticket. A ticket's frozen copy
        // stands in only when the live file is unreadable.
        let live_policy = std::fs::read_to_string(self.data.pipeline(project))
            .ok()
            .and_then(|text| Pipeline::parse(&text).ok())
            .map(|p| p.policy);
        let free_gb = self.free_gb();
        for stale in &tickets {
            let id = stale.id.clone();
            let stepped = self.transaction(|r| {
                let mut ps = r.load_project(project)?;
                let mut t = match r.load_ticket(&id) {
                    Ok(t) => t,
                    Err(e) => {
                        log::warn!("ticket {id}: {e}");
                        return Ok(None);
                    }
                };
                let before = ps.clone();
                // A call made without the ticket in hand (a query, a
                // provider read) is put down to this one.
                r.health.borrow_mut().current = Some(id.clone());
                let result = r.step_one(
                    &mut t,
                    &mut ps,
                    live_policy.as_ref(),
                    (running, pending, free_gb),
                    now_ms,
                );
                r.health.borrow_mut().current = None;
                // A write is an fsync; most steps leave the project alone.
                if ps != before {
                    r.save_project(&ps)?;
                }
                result
            })?;
            if let Some(stepped) = stepped {
                if stepped.took_slot {
                    running += 1;
                }
                pending = pending.saturating_add(stepped.asked);
            }
        }
        // Whatever moved, the set follows: a sync is sent only when what
        // it would show differs from what it last showed.
        self.transaction(|r| {
            let mut ps = r.load_project(project)?;
            let refreshed: Vec<Ticket> = ps
                .queue
                .iter()
                .filter_map(|id| r.load_ticket(id).ok())
                .collect();
            if let Err(e) = crate::view::sync_queue(r, &mut ps, project, &refreshed, None, now_ms) {
                log::warn!("queue view: {e}");
            }
            r.save_project(&ps)
        })?;
        Ok(())
    }

    /// One ticket's step under the lock: what it changed in the
    /// project's counts, or `None` when the ticket was finishing a park
    /// or a close, inactive, or its pipeline copy unreadable.
    fn step_one(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        live_policy: Option<&crate::pipeline::Policy>,
        (running, pending, free_gb): (u32, u32, Option<u32>),
        now_ms: u64,
    ) -> Result<Option<Stepped>> {
        if matches!(t.state, TicketState::Parking { .. }) {
            if let Err(e) = self.finish_parking(t, ps, now_ms) {
                log_step_error(&t.id, &e);
            }
            return Ok(None);
        }
        if matches!(t.state, TicketState::Closing { .. }) {
            if let Err(e) = self.finish_closing(t, ps, now_ms) {
                log::error!("ticket {}: {e:#}", t.id);
            }
            return Ok(None);
        }
        if !t.active() {
            return Ok(None);
        }
        let p = match self.pipeline_of(t) {
            Ok(p) => p,
            Err(e) => {
                self.park(t, ps, &format!("pipeline copy unreadable: {e}"), now_ms)?;
                return Ok(None);
            }
        };
        let has_open = t.attempts.iter().any(Attempt::is_open);
        // Watching what runs is free, and so is a gate-only stage (a
        // lanes choice, a PR read) or closing a ticket past its last
        // stage: they launch nothing. Starting an agent takes a slot and
        // is refused while too much waits on the user. Taking a resource
        // takes a slot too, gate-only or not; a ticket that already
        // holds one has its slot and starts the stage's agent in it.
        let stage = p.stages.get(t.stage);
        let gate_only = stage.is_none_or(|s| s.kind() == StageKind::GateOnly);
        let takes_hold = stage.is_some_and(|s| {
            !Self::skipped(t, s)
                && s.needs
                    .iter()
                    .any(|n| !t.holds.iter().any(|h| &h.resource == n))
        });
        let policy = live_policy.unwrap_or(&p.policy);
        let may_start = has_open
            || (gate_only && !takes_hold)
            || ((running < policy.slots || ticket_costs_slot(t))
                && pending < policy.waiting_on_me
                && !self.disk_hold(policy, free_gb));
        if !may_start {
            // Letting go of what the ticket has left behind starts
            // nothing, so it does not wait for a slot either.
            if let Err(e) = self.release_left(t, ps, &p, now_ms) {
                log_step_error(&t.id, &e);
            }
            // Asking launches nothing, so a resumed ticket's
            // questions do not wait for a slot.
            let asked = match self.ask_again_unslotted(t, ps, &p, now_ms) {
                Ok(asked) => u32::try_from(asked).unwrap_or(u32::MAX),
                Err(e) => {
                    log_step_error(&t.id, &e);
                    0
                }
            };
            return Ok(Some(Stepped {
                took_slot: false,
                asked,
            }));
        }
        let had_slot = ticket_costs_slot(t);
        match self.step(t, ps, &p, now_ms) {
            Ok(()) => self.health.borrow_mut().stepped(&t.id),
            Err(e) => log_step_error(&t.id, &e),
        }
        let took_slot = !had_slot && ticket_costs_slot(t);
        Ok(Some(Stepped {
            took_slot,
            asked: 0,
        }))
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
            if let Err(e) = self.supervisor_intent(&project, now_ms) {
                log::error!("project {project}: supervisor: {e:#}");
            }
            if let Err(e) = self.step_project(&project, now_ms) {
                log::error!("project {project}: {e}");
            }
        }
        Ok(())
    }

    /// Write an answer on a decision that waits on the user; the runner
    /// acts on it. A closing ticket's pending decisions wait on no one
    /// (`Ticket::waiting_on_you`), so an answer to one is refused rather
    /// than left on a closed ticket for nothing to act on.
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
            let mut t = read_ticket(&path)?;
            if !t.waiting_on_you().iter().any(|d| d.id == decision) {
                if matches!(t.state, TicketState::Closing { .. }) {
                    bail!("ticket {ticket} is closing; its pending decisions are being cancelled");
                }
                bail!("ticket {ticket} has no pending decision {decision}");
            }
            let by = self.actor.as_deref().unwrap_or(BY_HAND);
            let d = t
                .decisions
                .iter_mut()
                .find(|d| d.id == decision)
                .with_context(|| format!("ticket {ticket} has no pending decision {decision}"))?;
            if !d.options.iter().any(|o| o == answer) && d.name != "lanes" {
                bail!("decision {decision} takes one of: {}", d.options.join(", "));
            }
            // A supervisor answers only what the live table lets it; a
            // refusal is saved on the decision, which still waits.
            if by == BY_SUPERVISOR
                && !crate::supervisor::decides(&self.data, &t.project).contains(&d.name)
            {
                let name = d.name.clone();
                d.refusals.push(crate::ticket::Refusal {
                    by: by.to_owned(),
                    answer: answer.to_owned(),
                    at_ms: now_ms,
                });
                write_ticket_stamped(&self.data, &mut t, now_ms)?;
                bail!("the supervisor may not answer `{name}`; the owner does");
            }
            d.state = DecisionState::Answered {
                answer: answer.into(),
                note: note.map(str::to_owned),
                by: by.into(),
                at_ms: now_ms,
                acted: false,
            };
            let d = d.clone();
            write_ticket_stamped(&self.data, &mut t, now_ms)?;
            Ok(d)
        })
    }
}

impl Runner {
    /// `dispatch park`: the parking intent written by command, with every
    /// open decision withdrawn in the same write, as a `park` answer's
    /// first write does. The runner's next pass cancels the open
    /// attempts and kills the processes (`finish_parking`), since the
    /// checks are its children. A closing or closed ticket is a
    /// `UsageError`; one already parking or parked is refused.
    pub fn request_park(&self, ticket: &str, reason: Option<&str>, now_ms: u64) -> Result<Ticket> {
        let reason = reason.unwrap_or("parked by hand");
        let path = self.data.ticket_file(ticket);
        self.data.with_lock(|| {
            let mut t = read_ticket(&path)?;
            match &t.state {
                TicketState::Active => {}
                TicketState::Closing { .. } => {
                    return Err(UsageError(format!("ticket {ticket} is closing")).into());
                }
                TicketState::Closed { .. } => {
                    return Err(UsageError(format!("ticket {ticket} is closed")).into());
                }
                TicketState::Parking { reason } => {
                    bail!("ticket {ticket} is already parking: {reason}")
                }
                TicketState::Parked { reason } => {
                    bail!("ticket {ticket} is already parked: {reason}")
                }
            }
            log::warn!("ticket {ticket} parking: {reason}");
            Self::parking_intent(&mut t, reason, self.actor.clone());
            write_ticket_stamped(&self.data, &mut t, now_ms)?;
            Ok(t)
        })
    }

    /// A parked ticket back to active, with the attempts its park
    /// cancelled mid-run authorised to run again (`reruns_on_resume`):
    /// the resume is the answer to each one's `rerun` question, written
    /// answered by `resume`. The runner asks about the rest, and takes
    /// the ticket from its current stage on its next pass.
    pub fn resume(&self, ticket: &str, now_ms: u64) -> Result<Resumed> {
        self.resume_with(ticket, true, now_ms)
    }

    /// A parked ticket back to active with nothing authorised: the
    /// runner asks about every failed or cancelled attempt.
    pub fn resume_asking(&self, ticket: &str, now_ms: u64) -> Result<Resumed> {
        self.resume_with(ticket, false, now_ms)
    }

    /// Nothing else is a resume: the answers and `Active` are one write.
    fn resume_with(&self, ticket: &str, rerun: bool, now_ms: u64) -> Result<Resumed> {
        let path = self.data.ticket_file(ticket);
        self.data.with_lock(|| {
            let mut t = read_ticket(&path)?;
            let TicketState::Parked { reason } = t.state.clone() else {
                bail!("ticket {ticket} is not parked");
            };
            // A resume would forget a branch reset half done.
            if let Some(intent) = &t.restart {
                let (at, command) = match &intent.stage {
                    Some(stage) => (stage.as_str(), format!("dispatch restart {ticket} {stage}")),
                    None => ("its stage", format!("dispatch restart {ticket}")),
                };
                bail!(
                    "ticket {ticket}: a restart at {at} is part done; run `{command}` again to finish it, or close the ticket"
                );
            }
            let mut no_reruns = None;
            let candidates = if rerun {
                match self.park_reruns(&t, &reason) {
                    Ok(reruns) => reruns,
                    Err(e) => {
                        log::warn!("ticket {ticket}: nothing reruns on the resume: {e:#}");
                        no_reruns = Some(format!("{e:#}"));
                        Vec::new()
                    }
                }
            } else {
                Vec::new()
            };
            let mut reruns = Vec::new();
            for a in candidates {
                let Some((question, options)) = rerun_ask(&t, &a) else {
                    continue;
                };
                let mut d = new_decision(
                    &t,
                    Ask {
                        stage: &a.stage,
                        name: "rerun",
                        kind: DecisionKind::Permission,
                        question,
                        options,
                        recommendation: None,
                        attempt: Some((a.stage.clone(), a.n)),
                    },
                    now_ms,
                );
                d.state = DecisionState::Answered {
                    answer: "rerun".into(),
                    note: None,
                    by: BY_RESUME.into(),
                    at_ms: now_ms,
                    acted: false,
                };
                t.decisions.push(d);
                reruns.push(a);
            }
            if reruns.is_empty() {
                log::info!("ticket {ticket} resumed (was parked: {reason})");
            } else {
                let names: Vec<String> = reruns.iter().map(attempt_label).collect();
                log::info!(
                    "ticket {ticket} resumed (was parked: {reason}), rerunning {}",
                    names.join(", ")
                );
            }
            t.state = TicketState::Active;
            t.state_by.clone_from(&self.actor);
            write_ticket_stamped(&self.data, &mut t, now_ms)?;
            Ok(Resumed {
                ticket: t,
                reruns,
                no_reruns,
            })
        })
    }

    /// What the park that left `t` parked cancelled and a resume reruns:
    /// the attempts the event log ends after that park's `parking` event
    /// keep an earlier park with the same reason out. An error says why
    /// nothing can rerun.
    fn park_reruns(&self, t: &Ticket, reason: &str) -> Result<Vec<Attempt>> {
        let p = self.pipeline_of(t)?;
        let ended =
            crate::events::ended_since_parking(&crate::events::log_path(&self.data), &t.id)?
                .context("the event log has no parking for it")?;
        Ok(reruns_on_resume(t, &p, reason, &ended))
    }
}

/// What a resume wrote.
pub struct Resumed {
    /// The ticket, active again.
    pub ticket: Ticket,
    /// The attempts it authorised to run again, as answers to their
    /// `rerun` questions.
    pub reruns: Vec<Attempt>,
    /// Why a resume that reruns found nothing to: the pipeline or the
    /// park's time could not be read.
    pub no_reruns: Option<String>,
}

/// An attempt as a person reads it: `stage (context) attempt n`.
#[must_use]
pub fn attempt_label(a: &Attempt) -> String {
    format!("{} ({}) attempt {}", a.stage, a.context, a.n)
}

impl Runner {
    /// What each running agent of the ticket's latest open attempt shows:
    /// `(label, text)`, the last `lines` of its pane with the project's
    /// secret values already replaced by Switchboard. A session that is
    /// no longer running is left out; an app too old to answer is an
    /// error that says to update it.
    pub fn screens(&mut self, t: &Ticket, lines: u32) -> Result<Vec<(String, String)>> {
        let Some(a) = t.attempts.iter().rev().find(|a| a.is_open()) else {
            return Ok(Vec::new());
        };
        let mut sessions: Vec<(String, String)> = Vec::new();
        match a.kind {
            AttemptKind::Agent => sessions.extend(
                a.session
                    .iter()
                    .map(|s| (format!("{}/{} agent", a.stage, a.context), s.clone())),
            ),
            AttemptKind::Review => {
                if let Some(round) = a.rounds.last() {
                    if let Some(implementer) = &round.implementer {
                        sessions.push((
                            format!("{}/{} r{} implementer", a.stage, a.context, round.n),
                            implementer.clone(),
                        ));
                    } else {
                        for r in round.reviewers.iter().filter(|r| r.result.is_none()) {
                            if let Some(session) = &r.session {
                                sessions.push((
                                    format!("{}/{} r{} {}", a.stage, a.context, round.n, r.name),
                                    session.clone(),
                                ));
                            }
                        }
                    }
                }
            }
            AttemptKind::Workflow => {
                if let Some(run) = a.run.clone()
                    && let Reply::Workflow { run } = self.ask(Body::Workflow { run })?
                {
                    let label = format!("{}/{}", a.stage, a.context);
                    sessions.extend(run.planner.map(|s| (format!("{label} planner"), s)));
                    sessions.push((format!("{label} reviewer"), run.reviewer));
                }
            }
            AttemptKind::GateOnly => {}
        }
        let mut out = Vec::new();
        for (label, session) in sessions {
            match self.ask(Body::SessionScreen {
                session,
                lines: Some(lines),
            })? {
                Reply::Screen { text } => out.push((label, text)),
                Reply::Failed { reason }
                    if reason == "not running" || reason == NO_SUCH_SESSION => {}
                Reply::Failed { reason }
                    if reason.starts_with("bad request") && reason.contains("session.screen") =>
                {
                    bail!("this Switchboard has no session.screen; update the app")
                }
                Reply::Failed { reason } => bail!("session.screen failed: {reason}"),
                other => bail!("session.screen answered {other:?}"),
            }
        }
        Ok(out)
    }
}

/// Switchboard's reply to a session query for a record it does not
/// have; the one failure that means the session is gone.
pub(crate) const NO_SUCH_SESSION: &str = "no such session";
/// Switchboard's reply to a `workflow` query for a run it does not have;
/// the one failure that means the run is gone.
pub(crate) const NO_SUCH_RUN: &str = "no such run";

/// A running pane whose card says the agent is at work: after a Stop,
/// a turn ended but the work did not (background agents, a tool).
pub(crate) fn busy(view: &wire::SessionView) -> bool {
    view.liveness == wire::Liveness::Running && view.card == wire::CARD_WORKING
}

/// The idle count after one more poll with an artifact missing: only a
/// card at the prompt counts towards giving up, any other starts again.
pub(crate) fn idle_polls(view: &wire::SessionView, polls: u32) -> u32 {
    if view.card == wire::CARD_IDLE {
        polls + 1
    } else {
        0
    }
}

/// Whether the agent has stopped since its last nudge: with none, any
/// stop counts; after one, only a stop later than it.
pub(crate) fn stopped_since(stop_at_ms: Option<u64>, last_nudge: Option<u64>) -> bool {
    match (stop_at_ms, last_nudge) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(stop), Some(nudge)) => stop > nudge,
    }
}

/// What a session view says about an attempt's agent, taken into the
/// record (running once seen, its last stop), and whether it counts as
/// stopped.
fn observe_stop(attempt: &mut Attempt, view: &wire::SessionView) -> bool {
    if attempt.state == AttemptState::Starting {
        attempt.state = AttemptState::Running;
    }
    if let Some(stop) = view.last_stop_at_ms {
        attempt.stop_at_ms = Some(stop);
    }
    stopped_after_nudges(
        view,
        attempt.stop_at_ms,
        &attempt.nudges,
        &mut attempt.polls_since_stop,
    )
}

/// Whether an agent counts as stopped: a stop since its last nudge, a
/// clean exit, a nudged session that has ended, or a nudge gone
/// unanswered past the idle grace, which would otherwise be waited on
/// forever. With a nudge out and the session running, this poll is
/// counted towards that grace.
pub(crate) fn stopped_after_nudges(
    view: &wire::SessionView,
    stop_at_ms: Option<u64>,
    nudges: &[u64],
    polls_since_stop: &mut u32,
) -> bool {
    if stopped_since(stop_at_ms, nudges.last().copied())
        || matches!(view.liveness, wire::Liveness::Exited { code: Some(0) })
    {
        return true;
    }
    if nudges.is_empty() {
        return false;
    }
    // A nudge goes out only after a stop, so a session that ended since
    // still stopped once: its tree is judged as that stop left it.
    if view.liveness != wire::Liveness::Running {
        return true;
    }
    *polls_since_stop = idle_polls(view, *polls_since_stop);
    unanswered(stop_at_ms, nudges, *polls_since_stop)
}

/// Whether the last nudge went unanswered: no stop came after it within
/// the idle grace. Read from the record alone, so a restart keeps it.
pub(crate) fn unanswered(stop_at_ms: Option<u64>, nudges: &[u64], polls_since_stop: u32) -> bool {
    !nudges.is_empty()
        && !stopped_since(stop_at_ms, nudges.last().copied())
        && polls_since_stop >= STOP_IDLE_POLLS
}

/// What a stop with the tree not clean comes to.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DirtyStep {
    /// Record a nudge stamped `at_ms` and send it, `k` of `of`.
    Nudge { at_ms: u64, k: usize, of: u32 },
    /// The failure, with the clause its reason ends with.
    Fail(String),
}

/// The nudge or the failure for an agent that stopped (or never
/// answered its nudge, or ended) with its tree not clean, read from its
/// record and `on_dirty`. A nudge is stamped after the stop it answers,
/// so that stop never counts as the reply to it, however late in the
/// pass Switchboard reported it.
pub(crate) fn dirty_step(
    on_dirty: OnDirty,
    nudges: &[u64],
    stop_at_ms: Option<u64>,
    polls_since_stop: u32,
    running: bool,
    now_ms: u64,
) -> DirtyStep {
    let sent = nudges.len();
    let unanswered = unanswered(stop_at_ms, nudges, polls_since_stop);
    let mut suffix = dirty_suffix(sent);
    match on_dirty {
        OnDirty::Nudge(of) if running && sent < of as usize && !unanswered => {
            let at_ms = stop_at_ms.map_or(now_ms, |stop| now_ms.max(stop + 1));
            return DirtyStep::Nudge {
                at_ms,
                k: sent + 1,
                of,
            };
        }
        _ if unanswered => suffix.push_str("; no stop came after the last nudge"),
        _ if !running && sent > 0 && !stopped_since(stop_at_ms, nudges.last().copied()) => {
            suffix.push_str("; the session ended with no stop after the last nudge");
        }
        OnDirty::Nudge(_) if !running && sent == 0 => {
            suffix.push_str("; the session had ended, so it was not nudged");
        }
        _ => {}
    }
    DirtyStep::Fail(suffix)
}

/// The clause a dirty-tree failure ends with: how many nudges came
/// before it.
pub(crate) fn dirty_suffix(sent: usize) -> String {
    match sent {
        0 => String::new(),
        1 => ", after 1 nudge".into(),
        n => format!(", after {n} nudges"),
    }
}

/// An open attempt with an agent or a review run in it. A gate-only
/// attempt launches nothing.
pub(crate) fn costs_slot(a: &Attempt) -> bool {
    a.is_open() && a.kind != AttemptKind::GateOnly
}

/// Whether a ticket counts against the policy's `slots`: active, with a
/// running attempt or a resource held.
#[must_use]
pub fn ticket_costs_slot(t: &Ticket) -> bool {
    t.active() && (t.attempts.iter().any(costs_slot) || !t.holds.is_empty())
}

/// A lane a close removes on its own: one not removed yet with a
/// repository of its own, cut as a worktree of that repository's clone.
/// Any other lane is a path inside the ticket's tree and goes with it.
fn removes_lane(p: &Pipeline, lane: &LaneRecord) -> bool {
    !lane.removed && p.lane(&lane.name).is_some_and(|l| l.repo.is_some())
}

/// The paths a close of `t` would remove, in the order `remove_trees`
/// removes them: lanes of their own repositories, then the ticket's
/// tree. Nothing for a pipeline that works in place.
#[must_use]
pub fn close_removes(t: &Ticket, p: &Pipeline) -> Vec<PathBuf> {
    let (lanes, tree) = close_trees(t, p);
    let mut paths: Vec<PathBuf> = lanes.iter().map(|&i| t.lanes[i].worktree.clone()).collect();
    paths.extend(tree.map(Path::to_path_buf));
    paths
}

/// The branches a close of `t` keeps, with the clone each is in: lanes
/// of their own repositories, then the ticket's tree, in the order
/// `close_removes` lists their trees. Nothing for a pipeline that works
/// in place or a ticket never cut.
#[must_use]
pub fn kept_branches(t: &Ticket, p: &Pipeline, data: &DataDir) -> Vec<(String, PathBuf)> {
    if !p.cuts_worktrees() || t.tree.is_none() {
        return Vec::new();
    }
    let mut kept: Vec<(String, PathBuf)> = t
        .lanes
        .iter()
        .filter(|l| p.lane(&l.name).is_some_and(|lane| lane.repo.is_some()))
        .map(|l| {
            (
                l.branch.clone(),
                data.lane_repo_dir(&p.project.name, &l.name),
            )
        })
        .collect();
    kept.push((
        tree_branch(t, tree_pr(t, p)),
        data.repo_dir(&p.project.name),
    ));
    kept
}

/// The pull request whose branch is the ticket's tree: one in a lane
/// without a repository of its own.
fn tree_pr<'a>(t: &'a Ticket, p: &Pipeline) -> Option<&'a PullRequestSource> {
    t.source
        .pull_requests
        .iter()
        .find(|pr| p.lane(&pr.lane).is_some_and(|l| l.repo.is_none()))
}

/// The branch of the ticket's tree: its pull request's (`tree_pr`),
/// else the one named after the issue.
pub(crate) fn tree_branch(t: &Ticket, pr: Option<&PullRequestSource>) -> String {
    pr.map_or_else(
        || branch_name(t.source.number.unwrap_or(0), &t.source.title),
        |pr| pr.local().to_owned(),
    )
}

/// The lanes the cut has yet to record, in pipeline order. A ticket
/// from pull requests checks their branches out instead of cutting its
/// own, so only its lanes with a pull request are cut.
fn lanes_to_cut(t: &Ticket, p: &Pipeline) -> Vec<LaneCut> {
    let prs = &t.source.pull_requests;
    let branch = tree_branch(t, tree_pr(t, p));
    p.lanes
        .iter()
        .enumerate()
        .filter(|(_, lane)| !t.lanes.iter().any(|l| l.name == lane.name))
        .filter_map(|(index, lane)| {
            let pr = prs.iter().find(|pr| pr.lane == lane.name);
            if !prs.is_empty() && pr.is_none() {
                return None;
            }
            Some(LaneCut {
                index,
                branch: pr.map_or_else(|| branch.clone(), |pr| pr.local().to_owned()),
                pr: pr.cloned(),
            })
        })
        .collect()
}

/// The latest `branch` decision of the cut, the one whose answer counts.
fn latest_branch_decision(t: &mut Ticket) -> Option<&mut Decision> {
    t.decisions
        .iter_mut()
        .rev()
        .find(|d| d.stage == CUT && d.name == "branch")
}

/// The `branch` decision's question: which contexts (`what`, clone,
/// commits) hold a branch of the ticket's name with commits, and what
/// each answer does. Contexts whose branch had nothing are not named;
/// the cut deleted theirs before asking.
fn branch_question(branch: &str, moved: &[(&str, &Path, u64)], fresh_to: &str) -> String {
    let named: Vec<String> = moved
        .iter()
        .map(|(what, clone, n)| {
            let commits = if *n == 1 { "commit" } else { "commits" };
            format!("{what}, {n} {commits}, in {}", clone.display())
        })
        .collect();
    format!(
        "the branch {branch} has commits from an earlier ticket: {}. \
         reuse: cut on it at its head and go on from that work; \
         fresh: rename it to {fresh_to} and cut a new one; \
         park: park the ticket and leave the branch with commits as it is",
        named.join("; ")
    )
}

/// The lanes a close of `t` removes on their own, as indices into
/// `t.lanes`, and the ticket's tree if it is still there: the one copy
/// of the rule that the removal follows, the preflight checks and the
/// confirmation lists. Indices, because the removal needs each lane's
/// name and flag as well as its path.
fn close_trees<'a>(t: &'a Ticket, p: &Pipeline) -> (Vec<usize>, Option<&'a Path>) {
    if !p.cuts_worktrees() {
        return (Vec::new(), None);
    }
    let lanes = t
        .lanes
        .iter()
        .enumerate()
        .filter(|(_, l)| removes_lane(p, l))
        .map(|(i, _)| i)
        .collect();
    let tree = t.tree.as_deref().filter(|_| !t.close.tree_removed);
    (lanes, tree)
}

/// Only a `Closing` ticket belongs on the closing list. Any other goes
/// back to the queue, where the set and `dispatch queue` see it, rather
/// than being stepped from a list nothing shows.
fn requeue_strays(ps: &mut ProjectState, tickets: &[Ticket]) {
    for t in tickets {
        if ps.closing.contains(&t.id) && !matches!(t.state, TicketState::Closing { .. }) {
            log::warn!(
                "ticket {} is on the closing list but {}; back to the queue",
                t.id,
                t.state.label()
            );
            ps.closing.retain(|id| id != &t.id);
            if !ps.queue.contains(&t.id) {
                ps.queue.push(t.id.clone());
            }
        }
    }
}

/// The PR is read again on the next pass instead of after the poll
/// interval.
fn recheck_pr(t: &mut Ticket, attempt: Option<&(String, u32)>) {
    if let Some((stage, n)) = attempt
        && let Some(a) = find_attempt_mut(t, stage, *n)
        && let Some(pr) = &mut a.pr
    {
        pr.checked_ms = 0;
    }
}

/// How a sent-back attempt's cancellation reason starts; the gate's
/// name and the note follow, as `send_back` writes them.
const SENT_BACK_FROM: &str = "sent back from ";

/// The note a `rerun` answer, on decision `decision`, gives its attempt's
/// replacement, under its `rework` key: the answer's own note, else the
/// one a send-back gave the attempt, quoted by its cancellation reason,
/// which a park took off `t.rework` before any attempt carried it. An
/// agent stage's prompt takes a note, and so does a code review's first
/// fix pass (where "start over" also drops the carried points).
fn rerun_note(t: &Ticket, p: &Pipeline, decision: usize) -> Option<(String, String)> {
    let d = &t.decisions[decision];
    let (stage, n) = d.attempt.as_ref()?;
    if !matches!(
        p.stages.iter().find(|s| &s.name == stage)?.kind(),
        StageKind::Agent | StageKind::Review
    ) {
        return None;
    }
    let replaced = find_attempt(t, stage, *n)?;
    let own = match &d.state {
        DecisionState::Answered { note, .. } => note.clone(),
        _ => None,
    };
    let note = own.or_else(|| match &replaced.state {
        AttemptState::Cancelled { reason } => reason
            .strip_prefix(SENT_BACK_FROM)
            .and_then(|rest| rest.split_once(": "))
            .map(|(_, note)| note.to_owned()),
        _ => None,
    })?;
    Some((rework_key(stage, &replaced.context), note))
}

/// Whether a refresh rebaser has run on the lane since it last moved.
fn rebased_since_moved(t: &Ticket, lane: &LaneRecord) -> bool {
    t.attempts.iter().any(|a| {
        a.stage == REFRESH
            && a.context == lane.name
            && lane
                .refreshed
                .as_ref()
                .is_none_or(|r| a.started_ms > r.at_ms)
    })
}

/// The notes of the latest finished refresh rebaser in `lane` that
/// served the move being recorded: one started after the lane's previous
/// bring-up, or any when there was none. A previous bring-up recorded
/// before its time was kept says nothing about which move an older
/// rebaser served, so nothing is attached after it.
fn rebaser_notes(t: &Ticket, lane: &str, previous: Option<&Refreshed>) -> Option<PathBuf> {
    let after = match previous {
        None => None,
        Some(r) if r.at_ms > 0 => Some(r.at_ms),
        Some(_) => return None,
    };
    t.attempts
        .iter()
        .filter(|a| a.stage == REFRESH && a.context == lane && !a.is_open())
        .filter(|a| after.is_none_or(|ms| a.started_ms > ms))
        .filter_map(|a| Some((a.n, a.artifacts.get("notes").filter(|p| p.is_file())?)))
        .max_by_key(|(n, _)| *n)
        .map(|(_, path)| path.clone())
}

/// Who rewrote `lane`'s branch for the bring-up `r`, read from the
/// attempts and answers recorded at or before it, so the event written
/// with it and a later `show` agree. A clean rebase or a move is git's.
/// After a conflict, only a rebaser started at or after the conflict
/// worked on it (any, for a conflict from before its time was kept); a
/// `recheck` answer to the `refresh` question or a `rerun` answer to the
/// lane's rebaser's `rerun` question newer than that rebaser adopted a
/// hand rebase, as does a bring-up with no such rebaser; else that
/// rebaser's state says, and one that failed or was cancelled cannot
/// tell its own work from a hand rebase made before a park.
pub(crate) fn brought_up_by(t: &Ticket, lane: &str, r: &Refreshed) -> wire_dispatch::BroughtUpBy {
    use wire_dispatch::BroughtUpBy;
    if !r.commits || (r.conflict.is_none() && r.notes.is_none()) {
        return BroughtUpBy::Git;
    }
    let since = r.conflict.as_ref().map_or(0, |c| c.at_ms);
    let rebaser = t
        .attempts
        .iter()
        .filter(|a| a.stage == REFRESH && a.context == lane)
        .filter(|a| (since..=r.at_ms).contains(&a.started_ms))
        .max_by_key(|a| (a.started_ms, a.n));
    let adopted = t
        .decisions
        .iter()
        .filter_map(|d| {
            let DecisionState::Answered { answer, at_ms, .. } = &d.state else {
                return None;
            };
            let counts = match d.name.as_str() {
                REFRESH => answer == "recheck",
                "rerun" => {
                    answer == "rerun"
                        && d.attempt.as_ref().is_some_and(|(stage, n)| {
                            stage == REFRESH
                                && find_attempt(t, REFRESH, *n).is_some_and(|a| a.context == lane)
                        })
                }
                _ => false,
            };
            (counts && *at_ms <= r.at_ms).then_some(*at_ms)
        })
        .max();
    match (adopted, rebaser) {
        (Some(h), Some(a)) if h > a.started_ms => BroughtUpBy::Hand,
        (_, Some(a)) if a.state == AttemptState::Complete => BroughtUpBy::Rebaser,
        (_, Some(_)) => BroughtUpBy::Stopped,
        (_, None) => BroughtUpBy::Hand,
    }
}

/// How many commits conflicted in the bring-up `r` as its event says
/// it: 0 for git's, else the recorded conflict's count (0 when they
/// could not be listed), `None` when none is on record.
pub(crate) fn conflict_count(r: &Refreshed, by: wire_dispatch::BroughtUpBy) -> Option<u32> {
    if by == wire_dispatch::BroughtUpBy::Git {
        return Some(0);
    }
    r.conflict
        .as_ref()
        .map(|c| u32::try_from(c.commits.len()).unwrap_or(u32::MAX))
}

/// The bring-up of `lane` whose conflict resolution is still to be
/// reviewed: a chosen lane, not removed, whose last bring-up resolved a
/// conflict after the pipeline's last code review stage and pushed
/// nothing since. Shared by the resolution loop and the resume, so a
/// resume authorises a rerun only where the loop would launch one.
pub(crate) fn resolution_due<'l>(p: &Pipeline, lane: &'l LaneRecord) -> Option<&'l Refreshed> {
    let last_review = p
        .stages
        .iter()
        .rposition(|s| s.kind() == StageKind::Review)
        .unwrap_or_default();
    if !lane.chosen || lane.removed {
        return None;
    }
    let moved = lane.refreshed.as_ref()?;
    // A conflict brought up at or before the last code review is read
    // there, by round 1's rebase check.
    if moved
        .conflict
        .as_ref()
        .is_none_or(|c| c.stage <= last_review)
    {
        return None;
    }
    // A bring-up that pushed has put the resolution on the PR already.
    if lane.pushed.as_ref().is_some_and(|p| p.at_ms >= moved.at_ms) {
        return None;
    }
    Some(moved)
}

/// The attempts a resume authorises to run again, as the answer to their
/// `rerun` question: each the latest in its context, cancelled by the
/// park `park` names (a reason that only adds what its checks did past
/// the limit still counts) and in `ended`, the attempts the log ends
/// after that park's `parking` event, since an earlier park may have
/// given the same reason; of an agent, code review or workflow stage;
/// and with no question about it withdrawn or answered `park` by that
/// park. An attempt that was waiting on a question when the park came
/// is asked about again instead, since a rerun would throw away what it
/// had come to, and so is one an earlier park cancelled. A resolution
/// review is rerun only where the resolution loop would still launch
/// one; a rebaser never is, since the next pass reads its lane again and
/// starts one if it is still needed.
pub(crate) fn reruns_on_resume(
    t: &Ticket,
    p: &Pipeline,
    park: &str,
    ended: &BTreeSet<(String, u32)>,
) -> Vec<Attempt> {
    let past_the_limit = format!("{park}; ");
    t.attempts
        .iter()
        .filter(|a| {
            let AttemptState::Cancelled { reason } = &a.state else {
                return false;
            };
            if reason != park && !reason.starts_with(&past_the_limit) {
                return false;
            }
            if !ended.contains(&(a.stage.clone(), a.n)) {
                return false;
            }
            if latest_attempt(t, &a.stage, &a.context).map(|l| l.n) != Some(a.n) {
                return false;
            }
            let key = Some((a.stage.clone(), a.n));
            let was_asked = t.decisions.iter().any(|d| {
                d.attempt == key
                    && match &d.state {
                        DecisionState::Cancelled => true,
                        DecisionState::Answered { answer, .. } => answer == "park",
                        DecisionState::Pending => false,
                    }
            });
            if was_asked || rerun_in_flight(t, a) || may_rerun(t, a) {
                return false;
            }
            if a.stage == RESOLUTION {
                return resolution_reruns(t, p, a);
            }
            p.stages
                .iter()
                .find(|s| s.name == a.stage)
                .is_some_and(|s| {
                    matches!(
                        s.kind(),
                        StageKind::Agent | StageKind::Review | StageKind::Workflow
                    )
                })
        })
        .cloned()
        .collect()
}

/// Whether the resolution loop would launch a rerun of `a`: its lane's
/// resolution is still due and `a` is that bring-up's latest review.
fn resolution_reruns(t: &Ticket, p: &Pipeline, a: &Attempt) -> bool {
    if !t.source.pull_requests.is_empty() || resolution_stage(p).is_none() {
        return false;
    }
    t.lanes
        .iter()
        .find(|l| l.name == a.context)
        .and_then(|lane| resolution_due(p, lane))
        .and_then(|moved| resolution_of(t, &a.context, moved))
        .is_some_and(|r| r.n == a.n)
}

/// The latest resolution review in `lane` that belongs to the bring-up
/// `moved`: started at or after it.
pub(crate) fn resolution_of<'t>(
    t: &'t Ticket,
    lane: &str,
    moved: &Refreshed,
) -> Option<&'t Attempt> {
    t.attempts
        .iter()
        .filter(|a| a.stage == RESOLUTION && a.context == lane && a.started_ms >= moved.at_ms)
        .max_by_key(|a| a.n)
}

/// Drops every lane's recorded conflict as a stage goes ahead on the
/// branches as they stand (a tree left alone): whatever the stage runs
/// reads and moves the branch, so the head the conflict kept is no
/// longer the last one reviewed, and a later rebase records a conflict
/// of its own. True when one was dropped.
fn drop_stale_conflicts(t: &mut Ticket) -> bool {
    let mut dropped = false;
    for lane in &mut t.lanes {
        if lane.conflict.take().is_some() {
            log::info!(
                "ticket {} lane {}: the stage goes ahead without a bring-up; its conflict is dropped",
                t.id,
                lane.name
            );
            dropped = true;
        }
    }
    dropped
}

/// The shape a resolution review runs under: the pipeline's last code
/// review stage with one reviewer (the policy's `resolution_reviewer`,
/// else that stage's first that is not the style reviewer), one pass
/// and the default sentinel, so the sentinel every reader falls back to
/// for a stage the pipeline does not name is the one the reviewer is
/// told. Its implementer, checks and `commits` are the stage's. `None`
/// when the pipeline has no code review stage.
fn resolution_stage(p: &Pipeline) -> Option<Stage> {
    let review = p
        .stages
        .iter()
        .rev()
        .find(|s| s.kind() == StageKind::Review)?;
    let mut shape = review.clone();
    RESOLUTION.clone_into(&mut shape.name);
    let reviewer = p.policy.resolution_reviewer.clone().or_else(|| {
        review
            .reviewers
            .iter()
            .find(|r| *r != crate::review::STYLE_REVIEWER)
            .or_else(|| review.reviewers.first())
            .cloned()
    })?;
    shape.reviewers = vec![reviewer];
    shape.cap = Some(1);
    shape.style_rounds = Some(1);
    shape.review_prompt = None;
    shape.no_feedback = None;
    Some(shape)
}

/// The decision `ask` makes on `t`, pending, with the next id on the
/// ticket: one scheme for the runner's questions and a resume's answers.
fn new_decision(t: &Ticket, ask: Ask<'_>, now_ms: u64) -> Decision {
    Decision {
        id: format!("d{}", t.decisions.len() + 1),
        stage: ask.stage.into(),
        name: ask.name.into(),
        kind: ask.kind,
        question: ask.question,
        options: ask.options.iter().map(|o| (*o).to_owned()).collect(),
        recommendation: ask.recommendation,
        attempt: ask.attempt,
        state: DecisionState::Pending,
        made_ms: now_ms,
        refusals: Vec::new(),
    }
}

/// The `rerun` question about a failed or cancelled attempt and its
/// options, as `ask_rerun` asks it and a resume answers it. `None` for
/// an attempt that is neither.
fn rerun_ask(t: &Ticket, a: &Attempt) -> Option<(String, &'static [&'static str])> {
    let (what, options): (String, &'static [&'static str]) = match &a.state {
        AttemptState::Failed { reason } => {
            // A failure at the checks was offered `check` too, and one
            // at the rewrite `keep`. A ticket written before the attempt
            // kept that has only the earlier question about it to say
            // so.
            let earlier = t
                .decisions
                .iter()
                .rev()
                .find(|d| d.name == "rerun" && d.attempt == Some((a.stage.clone(), a.n)));
            let offered = |o: &str| earlier.is_some_and(|d| d.options.iter().any(|x| x == o));
            let options: &'static [&'static str] = if a.failed_at_checks || offered("check") {
                &["rerun", "check", "park"]
            } else if a.failed_at_rewrite || offered("keep") {
                &["rerun", "keep", "park"]
            } else {
                &["rerun", "park"]
            };
            (format!("failed: {reason}"), options)
        }
        // A restart flags the attempts whose checks may be run again
        // under its new copy.
        AttemptState::Cancelled { reason } if a.failed_at_checks => (
            format!("was cancelled: {reason}"),
            &["rerun", "check", "park"],
        ),
        AttemptState::Cancelled { reason } => {
            (format!("was cancelled: {reason}"), &["rerun", "park"])
        }
        AttemptState::Starting | AttemptState::Running | AttemptState::Complete => return None,
    };
    let question = format!(
        "{} ({}) attempt {} {what}. Run it again?{}{}",
        a.stage,
        a.context,
        a.n,
        rerun_carries(t, &a.stage, a.n),
        rewrite_choices(a, options)
    );
    Some((question, options))
}

/// What a rerun question adds about a code review attempt: the next
/// attempt carries its points unless the note says "start over". A
/// resolution review carries nothing.
fn rerun_carries(t: &Ticket, stage: &str, n: u32) -> &'static str {
    if stage == RESOLUTION {
        " A rerun reviews the same resolution again with a fresh reviewer."
    } else if find_attempt(t, stage, n).is_some_and(|a| a.kind == AttemptKind::Review) {
        " The next attempt carries this one's settled and open points; a note saying \"start over\" reviews the whole branch again."
    } else {
        ""
    }
}

/// The key a sent-back note is kept under.
pub(crate) fn rework_key(stage: &str, ctx: &str) -> String {
    format!("{stage}/{ctx}")
}

/// What each answer to a failed rewrite does, when the question offers
/// `keep`; nothing otherwise.
fn rewrite_choices(a: &Attempt, options: &[&str]) -> String {
    match &a.rewrite {
        Some(r) if options.contains(&"keep") => format!(
            " `rerun` reviews the branch again and rewrites it the same way; `keep` completes the stage at {} with the history as it is, once the tree is clean; `park` stops.",
            r.before
        ),
        _ => String::new(),
    }
}

/// The tree a context runs in: the lane's worktree, or the ticket's.
pub(crate) fn tree_of(t: &Ticket, p: &Pipeline, ctx: &str) -> Option<PathBuf> {
    t.lanes
        .iter()
        .find(|l| l.name == ctx)
        .map(|l| l.worktree.clone())
        .or_else(|| primary_tree(t, p))
}

/// The session of the context's last finished agent, whose transcript
/// an operator sent to the lane continues from.
fn lane_clone_of(t: &Ticket, ctx: &str) -> Option<String> {
    t.attempts
        .iter()
        .filter(|x| {
            x.context == ctx
                && x.kind == AttemptKind::Agent
                && x.state == AttemptState::Complete
                && x.session.is_some()
        })
        .max_by_key(|x| x.started_ms)
        .and_then(|x| x.session.clone())
}

/// The argv a command gate runs in a lane: the lane's own from
/// `per_lane`, else the gate's `argv`. `None` for any other gate.
pub(crate) fn lane_gate_argv<'g>(gate: &'g Gate, lane: Option<&str>) -> Option<&'g Vec<String>> {
    let Gate::Command { argv, per_lane, .. } = gate else {
        return None;
    };
    lane.and_then(|l| per_lane.as_ref().and_then(|m| m.get(l)))
        .or(argv.as_ref())
}

/// The latest attempt of stage `stage` in context `ctx`.
pub(crate) fn latest_attempt<'t>(t: &'t Ticket, stage: &str, ctx: &str) -> Option<&'t Attempt> {
    t.attempts
        .iter()
        .filter(|a| a.stage == stage && a.context == ctx)
        .max_by_key(|a| a.n)
}

/// The attempt `(stage, n)` names.
pub(crate) fn find_attempt<'t>(t: &'t Ticket, stage: &str, n: u32) -> Option<&'t Attempt> {
    t.attempts.iter().find(|a| a.stage == stage && a.n == n)
}

/// The attempt `(stage, n)` names, to change.
pub(crate) fn find_attempt_mut<'t>(
    t: &'t mut Ticket,
    stage: &str,
    n: u32,
) -> Option<&'t mut Attempt> {
    t.attempts.iter_mut().find(|a| a.stage == stage && a.n == n)
}

/// The record behind an attempt known to exist: a copy the runner is
/// polling, or one it just made.
pub(crate) fn record_of<'t>(t: &'t mut Ticket, stage: &str, n: u32) -> &'t mut Attempt {
    find_attempt_mut(t, stage, n).expect("the attempt exists")
}

/// An agent attempt to start: who, with what prompt, writing which
/// artifacts, and from whose transcript.
struct AgentSpec {
    operator: String,
    prompt: String,
    artifacts: BTreeMap<String, PathBuf>,
    clone_of: Option<String>,
    pr: Option<PullRequestRecord>,
    /// The sent-back note in the prompt, by its `rework` key, taken off
    /// in the same write as the attempt.
    rework: Option<String>,
    /// `DISPATCH_INPUT_*` variables for a fresh session: the path of
    /// each input its prompt names.
    env: BTreeMap<String, String>,
}

/// One context's poll of a PR-reading stage.
#[derive(Clone, Copy)]
struct PrPoll<'a> {
    stage: &'a Stage,
    attempt: &'a Attempt,
    cwd: &'a Path,
    lane: Option<&'a str>,
}

/// Where a `pr-checks` or `pr-merged` gate looks: the provider, its
/// name for the repository, and the branch.
struct PrTarget {
    provider: String,
    repo: String,
    branch: String,
    /// Known from the ticket's source; read by number rather than by
    /// branch, since the lane's branch may be a pull ref's.
    number: Option<u64>,
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
    if let Some(pr) = t
        .source
        .pull_requests
        .iter()
        .find(|pr| pr.lane == lane_record.name)
    {
        return Ok(PrTarget {
            provider: pr.provider.clone(),
            repo: pr.repo.clone(),
            branch: pr.branch.clone(),
            number: Some(pr.number),
        });
    }
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
    let provider = provider.map_or_else(|| guess_provider(&remote).to_owned(), str::to_owned);
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
        number: None,
    })
}

/// The remote a pull request is fetched from when it is not the
/// lane's own: its name and URL from the pipeline's `remotes`.
fn extra_remote(
    p: &Pipeline,
    lane: Option<&Lane>,
    pr: &PullRequestSource,
) -> Option<(String, String)> {
    let default = lane
        .filter(|l| l.repo.is_some())
        .map_or(p.project.remote.as_str(), |l| p.lane_remote(l));
    if pr.remote.is_empty() || pr.remote == default {
        return None;
    }
    p.remote_url(lane, &pr.remote)
        .map(|url| (pr.remote.clone(), url))
}

/// The control socket failed before or while a request was answered:
/// Switchboard is down or was restarted. Nothing about the ticket is
/// wrong, so a pass that meets this ends and the next one tries again.
#[derive(Debug)]
pub struct SocketDown(pub String);

impl From<std::io::Error> for SocketDown {
    fn from(e: std::io::Error) -> Self {
        // The kind tells a timeout (`WouldBlock`) from a reply that did
        // not parse (`InvalidData`) in the log line.
        Self(format!("{:?}: {e}", e.kind()))
    }
}

/// A ticket's pass that ended in an error, logged: the socket down is
/// one warning, since the next pass asks again; anything else is an
/// error. `{e:#}` prints the whole chain, not only the outer context.
fn log_step_error(ticket: &str, e: &anyhow::Error) {
    if e.is::<SocketDown>() {
        log::warn!("ticket {ticket}: {e:#}; asking again next pass");
    } else {
        log::error!("ticket {ticket}: {e:#}");
    }
}

impl std::fmt::Display for SocketDown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SocketDown {}

/// What a provider says stops a PR, and what the policy does about it.
enum Remedy {
    /// The branch conflicts with its base: the `rebaser`.
    Rebase,
    /// These checks are red: the `fixer`.
    Fix(Vec<String>),
}

impl Remedy {
    fn operator(&self, p: &Pipeline) -> Option<String> {
        match self {
            Self::Rebase => p.policy.rebaser.clone(),
            Self::Fix(_) => p.policy.fixer.clone(),
        }
    }

    fn cap(&self, p: &Pipeline) -> (&'static str, u32) {
        match self {
            Self::Rebase => ("max_rebases", p.policy.max_rebases),
            Self::Fix(_) => ("max_fixes", p.policy.max_fixes),
        }
    }

    /// What the record's `checks` says for an attempt of this kind.
    fn tag(&self) -> String {
        match self {
            Self::Rebase => "conflicting".to_owned(),
            Self::Fix(names) => format!("failed: {}", names.join(", ")),
        }
    }

    /// Whether an earlier attempt's record is of this kind.
    fn owns(&self, record: &PullRequestRecord) -> bool {
        match self {
            Self::Rebase => record.checks == "conflicting",
            Self::Fix(_) => record.checks.starts_with("failed:"),
        }
    }

    fn problem(&self) -> String {
        match self {
            Self::Rebase => "conflicts with its base".to_owned(),
            Self::Fix(names) => format!("checks failed: {}", names.join(", ")),
        }
    }

    fn by_hand(&self) -> &'static str {
        match self {
            Self::Rebase => "rebase it onto its base by hand",
            Self::Fix(_) => "fix it by hand",
        }
    }
}

/// The role of the remedy an attempt with this record was launched as,
/// read from the `checks` that `Remedy::tag` wrote; none for any other.
/// The log names the role, not the policy's operator, whose name could
/// be anything.
fn remedy_role(record: &PullRequestRecord) -> Option<&'static str> {
    if record.checks == "conflicting" {
        Some("rebaser")
    } else if record.checks.starts_with("failed:") {
        Some("fixer")
    } else {
        None
    }
}

/// An agent stage's completion line: a remedy's names its role and,
/// when it was read, the head it left, so it is never mistaken for the
/// stage's own completion.
fn completion_line(
    id: &str,
    stage: &str,
    ctx: &str,
    role: Option<&str>,
    head: Option<&str>,
) -> String {
    match (role, head) {
        (None, _) => format!("ticket {id} {stage}/{ctx} complete"),
        (Some(role), Some(head)) => format!("ticket {id} {stage}/{ctx} {role} complete at {head}"),
        (Some(role), None) => format!("ticket {id} {stage}/{ctx} {role} complete"),
    }
}

/// Why a stopped PR is a question rather than a run: no operator in
/// the policy for it, the last run changed nothing, or the cap is spent.
fn remedy_reason(
    t: &Ticket,
    p: &Pipeline,
    a: &Attempt,
    pr: &crate::github::PullRequest,
    remedy: &Remedy,
) -> Option<String> {
    // Only an agent that ran counts: one that could not start spends
    // nothing and proves nothing about the head.
    let earlier: Vec<&Attempt> = t
        .attempts
        .iter()
        .filter(|x| {
            x.stage == a.stage
                && x.context == a.context
                && x.kind == AttemptKind::Agent
                && x.session.is_some()
                && x.pr.as_ref().is_some_and(|r| remedy.owns(r))
        })
        .collect();
    let count = u32::try_from(earlier.len()).unwrap_or(u32::MAX);
    let last_head = earlier
        .iter()
        .filter(|x| x.state == AttemptState::Complete)
        .max_by_key(|x| x.n)
        .and_then(|x| x.pr.as_ref())
        .map(|r| r.head.clone());
    let (cap_name, cap) = remedy.cap(p);
    let by_hand = remedy.by_hand();
    if remedy.operator(p).is_none() {
        Some(format!("{by_hand}, then answer recheck"))
    } else if last_head
        .as_deref()
        .is_some_and(|h| same_commit(h, &pr.head))
    {
        Some(format!(
            "the {} ran and the PR is still at the same head; {by_hand}, then answer recheck",
            remedy.operator(p).unwrap_or_default()
        ))
    } else if count >= cap {
        Some(format!(
            "the policy's {cap_name} of {cap} is spent ({count} done); {by_hand}, then answer recheck"
        ))
    } else {
        None
    }
}

/// A `lanes` answer's names, comma-separated.
fn lane_names(answer: &str) -> Vec<String> {
    answer
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Why a check failed, from its exit code. 127 is the shell saying a
/// command was not found: the lane's `setup` did not install it and
/// nothing on the runner's `PATH` stands in, so running the same checks
/// again cannot help and the question says so.
pub(crate) fn checks_reason(code: i32, log: &Path) -> String {
    let base = format!("checks exited {code}; output at {}", log.display());
    if code == 127 {
        format!(
            "{base}. Exit 127 means a command was not found: the lane's setup did not install it and the runner's PATH has no copy; fix the pipeline's setup or gate, then run `dispatch restart <ticket>` and answer `check`; checking again without a restart runs the old copy"
        )
    } else {
        base
    }
}

/// The provider a remote URL points at, by its host.
pub(crate) fn guess_provider(remote: &str) -> &'static str {
    if github_repo(remote).is_some() {
        "github"
    } else if bitbucket_repo(remote).is_some() {
        "bitbucket"
    } else {
        "unknown"
    }
}

/// How many lanes a ticket cuts: every lane of the pipeline, or for a
/// ticket from pull requests only the lanes they are in.
fn lanes_wanted(t: &Ticket, p: &Pipeline) -> usize {
    if t.source.is_pull_request() {
        p.lanes
            .iter()
            .filter(|l| t.source.pull_requests.iter().any(|pr| pr.lane == l.name))
            .count()
    } else {
        p.lanes.len()
    }
}

/// Whether a stage reads the lane's pull request: its gate is the
/// `pr-checks` watch.
fn reads_pr(stage: &Stage) -> bool {
    matches!(&stage.gate, Some(Gate::External { check, .. }) if check == "pr-checks")
}

/// The pipeline's first stage that reads a pull request (the
/// `pr-checks` or `pr-merged` watch), with the provider it names.
fn pr_stage(p: &Pipeline) -> Option<(&Stage, Option<&str>)> {
    p.stages.iter().find_map(|s| match &s.gate {
        Some(Gate::External {
            check, provider, ..
        }) if check == "pr-checks" || check == "pr-merged" => Some((s, provider.as_deref())),
        _ => None,
    })
}

/// Whether two hashes name one commit; a provider may report a short
/// one.
fn same_commit(a: &str, b: &str) -> bool {
    let n = a.len().min(b.len());
    n >= 7 && a[..n].eq_ignore_ascii_case(&b[..n])
}

/// When the tree's head last moved, as far as the records say: the gate
/// attempt's start (the stage before it pushed), the end of any attempt
/// in its context (a refresh rebaser, a remedy fixer or rebaser), or a
/// `recheck` answered about it (the user pushed, then said so).
fn head_moved_ms(t: &Ticket, a: &Attempt) -> u64 {
    let key = (a.stage.clone(), a.n);
    let ended = t
        .attempts
        .iter()
        .filter(|x| x.context == a.context)
        .filter_map(|x| x.ended_ms);
    let rechecked = t
        .decisions
        .iter()
        .filter(|d| d.attempt.as_ref() == Some(&key))
        .filter_map(|d| match &d.state {
            DecisionState::Answered { answer, at_ms, .. } if answer == "recheck" => Some(*at_ms),
            _ => None,
        });
    ended.chain(rechecked).fold(a.started_ms, u64::max)
}

/// The head the lane's records last saw on its branch, the lease for a
/// push: the newest attempt in the lane's context, refreshes aside,
/// that recorded one (the full hash from local git over the
/// provider's), or the head a refresh pushed after that attempt ended.
/// Before a stage reads the PR this is usually an agent's local head,
/// not a reading of the PR. Never the remote-tracking ref: a fetch
/// moves that with anyone's push, and a lease on it would overwrite
/// exactly the commit the lease is there to keep.
fn pr_head_seen(t: &Ticket, lane: &LaneRecord) -> Option<String> {
    let recorded = t
        .attempts
        .iter()
        .rev()
        .filter(|a| a.context == lane.name && a.stage != REFRESH)
        .find_map(|a| {
            let head = a
                .head
                .clone()
                .or_else(|| a.pr.as_ref().map(|p| p.head.clone()));
            head.map(|h| (h, a.ended_ms.unwrap_or(a.started_ms)))
        });
    match (recorded, &lane.pushed) {
        (Some((head, at_ms)), Some(pushed)) if at_ms > pushed.at_ms => Some(head),
        (_, Some(pushed)) => Some(pushed.head.clone()),
        (recorded, None) => recorded.map(|(head, _)| head),
    }
}

/// What a reading of the PR means: its summary for the record, and
/// pass (`Ok(true)`), wait (`Ok(false)`) or a question. `young` is a
/// head that moved too recently for its checks to exist yet.
fn judge_pr(
    pr: &crate::github::PullRequest,
    checks: Option<&Checks>,
    head: &str,
    none_expected: bool,
    young: bool,
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
        "none" if young => Ok(false),
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

/// One variable for each input `text` names that `vars` resolves, set
/// to its path: `{inputs.<name>}` sets `DISPATCH_INPUT_<NAME>` and
/// `{inputs.<stage>.<name>}` sets `DISPATCH_INPUT_<STAGE>_<NAME>`, since
/// the two forms can mean different files. Validation keeps the keys
/// one-to-one, so no two fields share one.
fn input_env(vars: &Vars, text: &str) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for (stage, name) in crate::template::input_names(text) {
        let (field, key) = match &stage {
            Some(s) => (
                format!("inputs.{s}.{name}"),
                format!("DISPATCH_INPUT_{}_{}", env_key(s), env_key(&name)),
            ),
            None => (
                format!("inputs.{name}"),
                format!("DISPATCH_INPUT_{}", env_key(&name)),
            ),
        };
        if let Some(path) = vars.0.get(&field) {
            env.insert(key, path.clone());
        }
    }
    env
}

/// The resources a live secret was made under: what each stage with a
/// completed attempt still holding a secret file needs.
fn secret_holds(t: &Ticket, p: &Pipeline) -> Vec<String> {
    t.attempts
        .iter()
        .filter(|a| {
            a.state == AttemptState::Complete
                && a.secret.iter().any(|n| !a.forgotten.contains_key(n))
        })
        .filter_map(|a| p.stages.iter().find(|s| s.name == a.stage))
        .flat_map(|s| s.needs.iter().filter(|n| p.resource(n).is_some()).cloned())
        .collect()
}

/// Where each artifact a stage writes goes in its attempt directory.
fn artifact_paths(stage: &Stage, dir: &Path) -> BTreeMap<String, PathBuf> {
    stage
        .write_names()
        .map(|w| (w.to_owned(), dir.join(format!("{w}.md"))))
        .collect()
}

/// A gate-only command's artifacts made ready before it starts: each
/// path in its environment as `DISPATCH_WRITES_<NAME>`, any file left
/// from an earlier run removed so that one existing afterwards is this
/// run's, and the attempt directory closed to other users when one of
/// them is secret (the child writes the file under its own umask).
fn prepare_writes(a: &Attempt, dir: &Path, env: &mut Vec<(String, String)>) -> Result<()> {
    for (name, path) in a.artifacts.iter().filter(|(n, _)| *n != "checks") {
        env.push((
            format!("DISPATCH_WRITES_{}", env_key(name)),
            path.display().to_string(),
        ));
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(e).with_context(|| format!("removing {name}"));
            }
            _ => {}
        }
    }
    if !a.secret.is_empty() {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .context("closing the attempt directory")?;
    }
    Ok(())
}

/// After a gate-only command exited 0: every artifact it was to write
/// is there, and each secret one is readable by this user alone. The
/// reason to fail it otherwise.
fn seal_writes(a: &Attempt) -> Option<String> {
    if let Some(name) = missing_artifacts(a).into_iter().find(|n| n != "checks") {
        return Some(format!(
            "exited 0 without writing {name} ({})",
            a.artifacts[&name].display()
        ));
    }
    for name in &a.secret {
        use std::os::unix::fs::PermissionsExt;
        let Some(path) = a.artifacts.get(name) else {
            continue;
        };
        if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            return Some(format!("could not make {name} private: {e}"));
        }
    }
    None
}

/// The artifacts an attempt was to write that are not files yet.
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
pub(crate) fn next_n(t: &Ticket, stage: &str) -> u32 {
    t.attempts_of(stage).map(|a| a.n).max().unwrap_or(0) + 1
}

/// The key a check is polled under: one per attempt.
fn gate_key(t: &Ticket, a: &Attempt) -> String {
    format!("{}/{}/{}", t.id, a.stage, a.n)
}

/// A fresh attempt record.
pub(crate) fn new_attempt(
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
        rounds: Vec::new(),
        extra_pass: false,
        failed_at_checks: false,
        failed_at_rewrite: false,
        carried_from: None,
        rework: None,
        rewrite: None,
        nudges: Vec::new(),
        orphans_killed: Vec::new(),
        secret: BTreeSet::new(),
        forgotten: BTreeMap::new(),
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
        let mut entry = attempt.settle.get(name).cloned();
        let settled = settle_file(path, &mut entry)?;
        attempt
            .settle
            .insert(name.clone(), entry.expect("settle_file fills it"));
        if !settled {
            all = false;
        }
    }
    Ok(all)
}

/// Whether a file has looked the same for `SETTLE_POLLS` polls.
pub(crate) fn settle_file(path: &Path, settle: &mut Option<Settle>) -> Result<bool> {
    let meta = std::fs::metadata(path)?;
    let mtime_ms = crate::epoch_ms(meta.modified()?);
    let len = meta.len();
    let entry = settle.get_or_insert(Settle {
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
    Ok(entry.polls >= SETTLE_POLLS)
}

/// The definition a reviewer operator installs, with Dispatch's
/// variables (`{worktree}`, `{branch}`, `{project.root}`, `{inputs.*}`)
/// rendered into its templates; Switchboard's own (`{plan}`,
/// `{feedback}`, ...) are left for it. The reviewer works in the attempt
/// directory, so the templates are where it learns which repository the
/// plan is about. It is named by a hash of the rendered text, so an
/// edited operator is a new name, a run in flight keeps its own, and
/// each worktree gets its own.
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
            crate::pipeline::OperatorKind::Claude | crate::pipeline::OperatorKind::Command => {
                wire::AgentKind::Claude
            }
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

/// The kind of session an operator runs in.
pub(crate) fn session_kind(kind: crate::pipeline::OperatorKind) -> wire::SessionKind {
    match kind {
        // A command never has a session; the pipeline refuses one as a
        // stage operator, so this arm is never a launch.
        crate::pipeline::OperatorKind::Claude | crate::pipeline::OperatorKind::Command => {
            wire::SessionKind::Claude
        }
        crate::pipeline::OperatorKind::Codex => wire::SessionKind::Codex,
    }
}

/// The ticket's own tree: the tree it was cut into, else its first
/// lane's worktree, or the project's root for a project that works in
/// place. None before the cut.
pub(crate) fn primary_tree(t: &Ticket, p: &Pipeline) -> Option<PathBuf> {
    if p.cuts_worktrees() {
        t.tree
            .clone()
            .or_else(|| t.lanes.first().map(|l| l.worktree.clone()))
    } else {
        p.project.root.clone()
    }
}

/// Whether a pending decision holds this stage in this context: one
/// about the whole stage (no attempt), or one about an attempt of this
/// stage in this context. One about an attempt that no longer exists
/// holds nothing, since there is nothing of that context to protect.
pub(crate) fn held_in(t: &Ticket, stage: &str, ctx: &str) -> bool {
    t.decisions
        .iter()
        .filter(|d| d.pending() && d.stage == stage)
        .any(|d| match &d.attempt {
            None => true,
            Some((s, n)) => t
                .attempts
                .iter()
                .any(|a| &a.stage == s && a.n == *n && a.context == ctx),
        })
}

/// Whether a note sent this context of the stage back, so the note is
/// the answer to its failure. A workflow stage takes no note.
pub(crate) fn sent_back(t: &Ticket, stage: &Stage, ctx: &str) -> bool {
    stage.kind() != StageKind::Workflow && t.rework.contains_key(&rework_key(&stage.name, ctx))
}

/// Whether the latest attempt in its context, failed or cancelled, is
/// to be asked about again: nothing holds the context, no rerun is
/// authorised or in flight, and no note sent it back (the note is the
/// answer).
pub(crate) fn asks_again(t: &Ticket, a: &Attempt, sent_back: bool) -> bool {
    matches!(
        a.state,
        AttemptState::Failed { .. } | AttemptState::Cancelled { .. }
    ) && !sent_back
        && !held_in(t, &a.stage, &a.context)
        && !may_rerun(t, a)
        && !rerun_in_flight(t, a)
}

/// A rerun question about this attempt still open, or answered and
/// waiting for the replaced attempt to be retired.
fn rerun_in_flight(t: &Ticket, a: &Attempt) -> bool {
    t.decisions.iter().any(|d| {
        d.name == "rerun"
            && d.attempt.as_ref() == Some(&(a.stage.clone(), a.n))
            && (d.pending() || d.unacted_answer().is_some())
    })
}

/// The sessions whose last waiting request on the ledger turned the
/// mark on, in the order they were first marked.
fn still_marked(t: &Ticket) -> Vec<String> {
    let mut last: Vec<(String, bool)> = Vec::new();
    for o in &t.ledger {
        if let Some(Body::SessionWaiting { session, on, .. }) = &o.body {
            match last.iter_mut().find(|(s, _)| s == session) {
                Some(entry) => entry.1 = *on,
                None => last.push((session.clone(), *on)),
            }
        }
    }
    last.into_iter()
        .filter_map(|(session, on)| on.then_some(session))
        .collect()
}

/// Whether a mark may still be on in Switchboard: a session whose last
/// waiting request turned it on, or a waiting request with no reply yet.
fn marks_unsettled(t: &Ticket) -> bool {
    !still_marked(t).is_empty() || !unanswered_waiting(t).is_empty()
}

/// The ledger indices of waiting requests recovery still has to resolve
/// and can: unanswered, unsettled, and recorded with their body.
fn unanswered_waiting(t: &Ticket) -> Vec<usize> {
    t.ledger
        .iter()
        .enumerate()
        .filter(|(_, o)| o.kind == "session.waiting" && o.unresolved() && o.body.is_some())
        .map(|(i, _)| i)
        .collect()
}

/// Whether a failed attempt's rerun was authorised and its cleanup done:
/// the answer is marked acted only once the replaced attempt's
/// processes were confirmed gone, and that mark is on disk.
pub(crate) fn may_rerun(t: &Ticket, failed: &Attempt) -> bool {
    t.decisions
        .iter()
        .any(|d| d.acted_rerun() && d.attempt.as_ref() == Some(&(failed.stage.clone(), failed.n)))
}

/// Whether `d` is a `rerun` answer acted on whose attempt is still the
/// latest in its context: the replacement it authorised has not
/// launched, and `may_rerun` would launch it.
fn authorises_unlaunched_rerun(t: &Ticket, d: &Decision) -> bool {
    if !d.acted_rerun() {
        return false;
    }
    let Some((stage, n)) = &d.attempt else {
        return false;
    };
    let Some(context) = find_attempt(t, stage, *n).map(|a| &a.context) else {
        return false;
    };
    !t.attempts
        .iter()
        .any(|a| &a.stage == stage && &a.context == context && a.n > *n)
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
pub(crate) fn vars_for(t: &Ticket, p: &Pipeline, lane: Option<&str>) -> Vars {
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
        );
    // Pipeline order, whatever order the records were cut in. Before the
    // lanes decision no lane is chosen, and every lane is the honest
    // answer to "which lanes" for the stages that run before it.
    let all: Vec<&str> = p.lanes.iter().map(|l| l.name.as_str()).collect();
    let chosen: Vec<&str> = all
        .iter()
        .copied()
        .filter(|name| t.lanes.iter().any(|l| l.name == *name && l.chosen))
        .collect();
    let lanes = if chosen.is_empty() { &all } else { &chosen };
    vars.set("lanes", lanes.join(", "))
        .set("lanes.all", all.join(", "));
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
    let names: BTreeSet<&String> = t.attempts.iter().flat_map(|a| a.artifacts.keys()).collect();
    for name in names {
        if let Some((_, path)) = t.input_where(name, |a| visible(p, lane, a)) {
            vars.set(format!("inputs.{name}"), path.display().to_string());
        }
    }
    // What an earlier stage left the tree at (a deploy's commit), or
    // that it was skipped. A per-lane stage with no attempt in this lane
    // leaves its fields unset rather than hand over another lane's.
    for s in p.stages.iter().take(t.stage) {
        let latest = t
            .attempts
            .iter()
            .filter(|a| a.stage == s.name && a.state == AttemptState::Complete)
            .filter(|a| visible(p, lane, a))
            .max_by_key(|a| a.n);
        // An artifact of a named stage, unless it was forgotten.
        for (name, path) in latest.iter().flat_map(|a| {
            a.artifacts
                .iter()
                .filter(|(name, _)| !a.forgotten.contains_key(*name))
        }) {
            vars.set(
                format!("inputs.{}.{name}", s.name),
                path.display().to_string(),
            );
        }
        let head = latest.and_then(|a| a.head.clone());
        let key = format!("inputs.{}.commit", s.name);
        if let Some(head) = head {
            vars.set(key, head);
        } else if Runner::skipped(t, s) {
            vars.set(key, format!("unknown ({} skipped)", s.name));
        }
    }
    if let Some(stage) = p.stages.get(t.stage) {
        for lane in &stage.services {
            let ready = t.services.iter().rev().find(|x| {
                x.stage == stage.name && &x.lane == lane && x.state == ServiceState::Ready
            });
            let chosen = t.lanes.iter().any(|l| &l.name == lane && l.chosen);
            if let Some(url) = ready.and_then(|x| x.url.clone()) {
                vars.set(format!("services.{lane}"), url);
            } else if !chosen {
                vars.set(
                    format!("services.{lane}"),
                    format!("not served (no {lane} lane)"),
                );
            }
        }
    }
    vars
}

/// Whether a reader in `lane` (none in the root) sees what attempt `a`
/// wrote: every attempt of a stage that runs in one context, and only
/// its own lane's of a stage that runs per lane. An attempt of no stage
/// of the pipeline (a refresh, a resolution) runs where it ran, so it is
/// seen from the root, the join, and its own lane.
fn visible(p: &Pipeline, lane: Option<&str>, a: &Attempt) -> bool {
    let own = Some(a.context.as_str()) == lane;
    match p.stages.iter().find(|s| s.name == a.stage) {
        Some(s) => !s.runs_per_lane() || own,
        None => a.context == "root" || a.context == "joined" || own,
    }
}

/// What a human gate after a deploy or with services up adds: the
/// commit each earlier gate-only command left (or that it was skipped),
/// and each lane being served. Nothing when there is neither.
fn deployed_and_served(t: &Ticket, p: &Pipeline) -> String {
    let mut lines: Vec<String> = Vec::new();
    for s in p.stages.iter().take(t.stage) {
        if !s.is_command_stage() {
            continue;
        }
        let contexts: std::collections::BTreeSet<&str> = t
            .attempts_of(&s.name)
            .filter(|a| a.state == AttemptState::Complete)
            .map(|a| a.context.as_str())
            .collect();
        for ctx in contexts {
            let head = latest_attempt(t, &s.name, ctx)
                .filter(|a| a.state == AttemptState::Complete)
                .and_then(|a| a.head.as_deref());
            if let Some(head) = head {
                let short: String = head.chars().take(8).collect();
                lines.push(format!("Deployed: {} at {short} ({ctx})", s.name));
            }
        }
        if Runner::skipped(t, s) {
            let named = match &s.context {
                Context::Lane(l) => l.clone(),
                Context::Lanes(ls) => ls.join(", "),
                _ => String::new(),
            };
            lines.push(format!("Deployed: {} skipped (no {named} lane)", s.name));
        }
    }
    lines.extend(
        t.services
            .iter()
            .filter(|x| x.state == ServiceState::Ready)
            .map(|x| {
                format!(
                    "Served: {} {}",
                    x.lane,
                    x.url.as_deref().unwrap_or_default()
                )
            }),
    );
    if lines.is_empty() {
        return String::new();
    }
    format!("\n{}", lines.join("\n"))
}

/// An operator's guidance, rendered like the stage prompt it opens (it
/// may name the branch, the worktree or an artifact), and the blank line
/// after it; nothing when the operator has none.
pub(crate) fn guidance_prelude(guidance: &str, vars: &Vars) -> String {
    let guidance = guidance.trim();
    if guidance.is_empty() {
        String::new()
    } else {
        format!("{}\n\n", vars.render(guidance))
    }
}

/// What a pipeline command of the ticket may write when the pipeline
/// confines its commands: every tree of the ticket, the primary tree
/// (the project's root for one that works in place), `extra` (the
/// attempt or round directory), and the lane's own `writable` paths. A
/// command with no lane (a root-context stage or round) gets the
/// `writable` paths of every lane whose tree is in the set: the
/// ticket's lanes, and a lane at `.`, whose tree is the primary one
/// even before any lane is chosen. `network` is a gate's own override of the policy's.
/// `None` when the pipeline does not confine.
pub(crate) fn confine_for(
    t: &Ticket,
    p: &Pipeline,
    lane: Option<&str>,
    extra: &[&Path],
    network: Option<Network>,
) -> Option<Confine> {
    if !p.policy.confine {
        return None;
    }
    let mut writable: Vec<PathBuf> = Vec::new();
    let mut add = |path: PathBuf| {
        if !writable.contains(&path) {
            writable.push(path);
        }
    };
    for l in &t.lanes {
        add(l.worktree.clone());
    }
    if let Some(tree) = primary_tree(t, p) {
        add(tree);
    }
    for path in extra {
        add(path.to_path_buf());
    }
    let lanes: Vec<&Lane> = match lane {
        Some(l) => p.lane(l).into_iter().collect(),
        None => p
            .lanes
            .iter()
            .filter(|l| l.path == Path::new(".") || t.lanes.iter().any(|r| r.name == l.name))
            .collect(),
    };
    for path in lanes.iter().flat_map(|l| &l.writable) {
        add(path.clone());
    }
    Some(Confine {
        writable,
        network: network.unwrap_or(p.policy.network),
    })
}

/// A command gate's own `network`, read from the gate the stage runs:
/// for a gate given by `like`, the one it names.
pub(crate) fn gate_network(p: &Pipeline, stage: &Stage) -> Option<Network> {
    match p.command_gate(stage) {
        Some(Gate::Command { network, .. }) => *network,
        _ => None,
    }
}

/// What a stage's checks get in their environment: `env_for`'s, and
/// which attempt they check, where, and at which head.
pub(crate) fn checks_env(
    t: &Ticket,
    lane: Option<&str>,
    branch: Option<&str>,
    a: &Attempt,
    cwd: &Path,
    head: &str,
) -> Vec<(String, String)> {
    let mut env = env_for(t, lane, branch);
    env.push(("DISPATCH_STAGE".to_owned(), a.stage.clone()));
    env.push(("DISPATCH_CONTEXT".to_owned(), a.context.clone()));
    env.push(("DISPATCH_TREE".to_owned(), cwd.display().to_string()));
    env.push(("DISPATCH_HEAD".to_owned(), head.to_owned()));
    env
}

/// What a command run for a ticket gets in its environment.
pub(crate) fn env_for(
    t: &Ticket,
    lane: Option<&str>,
    branch: Option<&str>,
) -> Vec<(String, String)> {
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
            let Some(attempt) = find_attempt_mut(t, &stage, n) else {
                return;
            };
            // Parking and closing wait for every launch to settle before
            // they end an attempt, so a reply finds it starting. Should
            // one find it ended anyway, what was made goes on the process
            // list that both of them kill, and the attempt stays ended.
            let starting = matches!(attempt.state, AttemptState::Starting);
            if intent == "session" {
                if let Some(id) = first(wire::RecordKind::Session) {
                    attempt.session = Some(id.clone());
                    if starting {
                        attempt.state = AttemptState::Running;
                    }
                    t.processes.push(id);
                }
            } else {
                if let Some(id) = first(wire::RecordKind::Run) {
                    attempt.run = Some(id);
                    if starting {
                        attempt.state = AttemptState::Running;
                    }
                }
                for m in made.iter().filter(|m| m.kind == wire::RecordKind::Session) {
                    t.processes.push(m.id.clone());
                }
            }
        }
        other => {
            // Older ledgers hold `lane-project:` requests; none is sent now.
            if let Some(lane) = other.strip_prefix("lane-project:")
                && let Some(id) = first(wire::RecordKind::Project)
                && let Some(l) = t.lanes.iter_mut().find(|l| l.name == lane)
            {
                l.project = Some(id);
            }
            if other.starts_with("reviewer:")
                || other.starts_with("implementer:")
                || other == "message"
            {
                crate::review::apply_review_reply(t, other, made);
            }
            if other.starts_with("service:") {
                crate::services::apply_service_reply(t, other, made);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every decision name Dispatch asks of its own accord, as a literal
    /// at its ask site, is one a supervisor's `decides` may list.
    #[test]
    fn decisions_holds_every_name_an_ask_site_spells() {
        for (file, text) in [
            ("scheduler.rs", include_str!("scheduler.rs")),
            ("review.rs", include_str!("review.rs")),
            ("recover.rs", include_str!("recover.rs")),
        ] {
            let code = text.split("#[cfg(test)]").next().unwrap();
            for rest in code.split("name: \"").skip(1) {
                let Some((name, after)) = rest.split_once('"') else {
                    continue;
                };
                if !after.starts_with(',') {
                    continue;
                }
                assert!(
                    DECISIONS.contains(&name),
                    "{file} asks `{name}`; add it to DECISIONS"
                );
            }
        }
        assert_eq!(
            DECISIONS
                .iter()
                .filter(|d| **d == REFRESH || **d == RESOLUTION)
                .count(),
            2
        );
    }

    #[test]
    fn a_remedys_completion_names_its_role_and_head() {
        let record = |checks: &str| PullRequestRecord {
            provider: "github".into(),
            repo: "o/r".into(),
            number: 7,
            url: String::new(),
            head: "h".into(),
            checks: checks.into(),
            checked_ms: 0,
            error_since_ms: None,
        };
        let conflicting = record("conflicting");
        let failed = record("failed: ci");
        assert_eq!(
            completion_line(
                "t",
                "merge",
                "repo",
                remedy_role(&conflicting),
                Some("rebased1")
            ),
            "ticket t merge/repo rebaser complete at rebased1"
        );
        assert_eq!(
            completion_line("t", "ready", "repo", remedy_role(&failed), Some("abc")),
            "ticket t ready/repo fixer complete at abc"
        );
        assert_eq!(
            completion_line("t", "merge", "repo", remedy_role(&failed), None),
            "ticket t merge/repo fixer complete"
        );
        assert_eq!(
            completion_line(
                "t",
                "merge",
                "repo",
                remedy_role(&record("merged")),
                Some("x")
            ),
            "ticket t merge/repo complete"
        );
        assert_eq!(
            completion_line("t", "merge", "repo", None, None),
            "ticket t merge/repo complete"
        );
        for checks in ["pending", "passed", "none", "merged"] {
            assert_eq!(remedy_role(&record(checks)), None, "{checks}");
        }
        for checks in [
            "conflicting",
            "failed: ci",
            "pending",
            "passed",
            "none",
            "merged",
        ] {
            let r = record(checks);
            assert_eq!(
                Remedy::Rebase.owns(&r),
                remedy_role(&r) == Some("rebaser"),
                "{checks}"
            );
            assert_eq!(
                Remedy::Fix(Vec::new()).owns(&r),
                remedy_role(&r) == Some("fixer"),
                "{checks}"
            );
        }
    }

    #[test]
    fn the_branch_question_names_only_the_moved_contexts() {
        let one = branch_question(
            "dispatch/42-x",
            &[("lane frontend", Path::new("/d/repos/O@frontend"), 1)],
            "dispatch/42-x.closed-19700101",
        );
        assert_eq!(
            one,
            "the branch dispatch/42-x has commits from an earlier ticket: \
             lane frontend, 1 commit, in /d/repos/O@frontend. \
             reuse: cut on it at its head and go on from that work; \
             fresh: rename it to dispatch/42-x.closed-19700101 and cut a new one; \
             park: park the ticket and leave the branch with commits as it is"
        );
        let two = branch_question(
            "dispatch/42-x",
            &[
                ("the tree", Path::new("/d/repos/O"), 2),
                ("lane backend", Path::new("/d/repos/O@backend"), 1),
            ],
            "dispatch/42-x.closed-19700101",
        );
        assert!(
            two.contains("the tree, 2 commits, in /d/repos/O; lane backend, 1 commit,"),
            "{two}"
        );
        assert!(two.ends_with("park: park the ticket and leave the branch with commits as it is"));
    }

    /// A root-context command has no lane of its own, so it gets the
    /// `writable` of every lane whose tree it has; a lane's command gets
    /// only its own.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn a_root_command_is_confined_with_every_lanes_writable() {
        let p = Pipeline::parse(
            r#"
version = 1

[project]
name = "P"
repo = "git@example.com:o/p.git"
space = "Dispatch · P"

[source]
kind = "github"
repo = "o/p"
label = "dispatch"

[[lanes]]
name = "api"
path = "."
writable = ["/cache/api"]

[[lanes]]
name = "web"
path = "web"
repo = "git@example.com:o/web.git"
writable = ["/cache/web"]

[operators.worker]
kind = "claude"

[[stages]]
name = "work"
operator = "worker"
context = "root"
writes = ["notes"]
prompt = "Write {notes}."

[policy]
confine = true
network = "deny"
"#,
        )
        .unwrap();
        let lane = |name: &str, worktree: &str| LaneRecord {
            name: name.into(),
            worktree: worktree.into(),
            branch: "dispatch/7-x".into(),
            project: None,
            chosen: true,
            setup_done: false,
            base_sha: None,
            refreshed: None,
            pushed: None,
            conflict: None,
            removed: false,
        };
        let t = Ticket {
            version: 0,
            id: "t1".into(),
            project: "P".into(),
            source: SourceSnapshot {
                kind: "github".into(),
                identity: "o/p#7".into(),
                pull_requests: Vec::new(),
                number: Some(7),
                title: "x".into(),
                body: String::new(),
                url: None,
                labels: vec![],
                taken_at_ms: 0,
                taken_by: None,
            },
            pipeline_fingerprint: String::new(),
            pipeline_file: PathBuf::new(),
            lanes: vec![lane("api", "/wt"), lane("web", "/wt/web")],
            tree: Some("/wt".into()),
            stage: 0,
            attempts: vec![],
            decisions: vec![],
            ledger: vec![],
            processes: vec![],
            root_project: None,
            rework: BTreeMap::new(),
            refreshed_stage: None,
            state: TicketState::Active,
            state_by: None,
            close: crate::ticket::CloseProgress::default(),
            restarts: Vec::new(),
            restart: None,
            entered: Vec::new(),
            holds: Vec::new(),
            services: Vec::new(),
            created_ms: 0,
            updated_ms: 0,
        };
        let root = confine_for(&t, &p, None, &[Path::new("/a")], None).unwrap();
        assert_eq!(root.network, Network::Deny);
        assert_eq!(
            root.writable,
            ["/wt", "/wt/web", "/a", "/cache/api", "/cache/web"].map(PathBuf::from)
        );
        // Before the lanes are chosen, the lane at `.` is the root's.
        let unchosen = Ticket {
            lanes: Vec::new(),
            ..t.clone()
        };
        assert_eq!(
            confine_for(&unchosen, &p, None, &[], None)
                .unwrap()
                .writable,
            ["/wt", "/cache/api"].map(PathBuf::from)
        );
        let web = confine_for(&t, &p, Some("web"), &[], Some(Network::Allow)).unwrap();
        assert_eq!(web.network, Network::Allow);
        assert_eq!(
            web.writable,
            ["/wt", "/wt/web", "/cache/web"].map(PathBuf::from)
        );
    }

    /// A pipeline with one stage of each kind: `plan` (agent), `review`
    /// (workflow), `review-code` (code review, the last), `inspect`
    /// (gate-only).
    fn kinds_pipeline() -> Pipeline {
        Pipeline::parse(
            r#"
version = 1

[project]
name = "P"
repo = "git@example.com:o/p.git"
space = "Dispatch · P"

[source]
kind = "github"
repo = "o/p"
label = "dispatch"

[[lanes]]
name = "api"
path = "."

[operators.planner]
kind = "claude"

[operators.implementer]
kind = "claude"

[operators.style]
kind = "claude"

[operators.reviewer]
kind = "codex"
[operators.reviewer.review]
reviewer = "codex"
review_first = "Review {plan}; write to {feedback}; else {no_feedback}"
review_round = "Again {response} {plan} {feedback} {no_feedback}"
respond = "Feedback at {feedback}; edit {plan}; answer at {response}."
respond_to_user = "{text} {plan} {response}"
handoff = "The plan at {plan} is final."
no_feedback = "No further feedback."
cap = 4

[[stages]]
name = "plan"
operator = "planner"
context = "each"
writes = ["plan"]
prompt = "Plan to {plan}."

[[stages]]
name = "review"
review = "reviewer"
context = "each"
subject = "plan"
gate = { kind = "external", check = "review-finalized" }

[[stages]]
name = "review-code"
context = "each"
reviewers = ["style"]
implementer = "implementer"
gate = { kind = "command", argv = ["sh", "-c", "cargo test"] }

[[stages]]
name = "inspect"
context = "each"
gate = { kind = "human", decision = "inspect" }
"#,
        )
        .unwrap()
    }

    fn bare_ticket() -> Ticket {
        Ticket {
            version: 0,
            id: "t1".into(),
            project: "P".into(),
            source: SourceSnapshot {
                kind: "github".into(),
                identity: "o/p#7".into(),
                pull_requests: Vec::new(),
                number: Some(7),
                title: "x".into(),
                body: String::new(),
                url: None,
                labels: vec![],
                taken_at_ms: 0,
                taken_by: None,
            },
            pipeline_fingerprint: String::new(),
            pipeline_file: PathBuf::new(),
            lanes: vec![LaneRecord {
                name: "api".into(),
                worktree: "/wt".into(),
                branch: "dispatch/7-x".into(),
                project: None,
                chosen: true,
                setup_done: false,
                base_sha: None,
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
            state: TicketState::Parked {
                reason: "parked".into(),
            },
            state_by: None,
            close: crate::ticket::CloseProgress::default(),
            restarts: Vec::new(),
            restart: None,
            entered: Vec::new(),
            holds: Vec::new(),
            services: Vec::new(),
            created_ms: 0,
            updated_ms: 0,
        }
    }

    fn cancelled(stage: &str, n: u32, kind: AttemptKind, reason: &str, at: u64) -> Attempt {
        new_attempt(
            stage,
            n,
            "api",
            kind,
            AttemptState::Cancelled {
                reason: reason.into(),
            },
            BTreeMap::new(),
            at,
        )
    }

    /// What reruns when the log ends every attempt after the park.
    fn rerun_of(t: &Ticket, p: &Pipeline) -> Vec<(String, u32)> {
        let ended = t.attempts.iter().map(|a| (a.stage.clone(), a.n)).collect();
        reruns_on_resume(t, p, "parked", &ended)
            .into_iter()
            .map(|a| (a.stage, a.n))
            .collect()
    }

    #[test]
    fn a_resume_reruns_the_latest_attempts_its_park_cancelled_of_a_stage_that_runs() {
        let p = kinds_pipeline();
        let mut t = bare_ticket();
        t.attempts = vec![
            cancelled("plan", 1, AttemptKind::Agent, "parked", 1),
            cancelled("review", 1, AttemptKind::Workflow, "parked", 1),
            cancelled(
                "review-code",
                1,
                AttemptKind::Review,
                "parked; its checks (t1/review-code/1) were still running after 120s",
                1,
            ),
        ];
        assert_eq!(
            rerun_of(&t, &p),
            [
                ("plan".to_owned(), 1),
                ("review".to_owned(), 1),
                ("review-code".to_owned(), 1)
            ]
        );
        // Another park's reason, one that only starts the same, a
        // failure, a gate-only stage and a rebaser are left to ask.
        let mut other = t.clone();
        other.attempts = vec![
            cancelled("plan", 1, AttemptKind::Agent, "parked by hand", 1),
            cancelled("review", 1, AttemptKind::Workflow, "an earlier park", 1),
            new_attempt(
                "review-code",
                1,
                "api",
                AttemptKind::Review,
                AttemptState::Failed {
                    reason: "parked".into(),
                },
                BTreeMap::new(),
                1,
            ),
            cancelled("inspect", 1, AttemptKind::GateOnly, "parked", 1),
            cancelled(REFRESH, 1, AttemptKind::Agent, "parked", 1),
        ];
        assert!(
            rerun_of(&other, &p).is_empty(),
            "{:?}",
            rerun_of(&other, &p)
        );
        // Only the latest in its context: an earlier cancelled attempt
        // under a later failed one is not rerun.
        let mut earlier = t.clone();
        earlier.attempts = vec![
            cancelled("plan", 1, AttemptKind::Agent, "parked", 1),
            new_attempt(
                "plan",
                2,
                "api",
                AttemptKind::Agent,
                AttemptState::Failed {
                    reason: "gone".into(),
                },
                BTreeMap::new(),
                2,
            ),
        ];
        assert!(rerun_of(&earlier, &p).is_empty());
        // The same reason the log ends before this park's `parking`
        // event: an earlier park's, asked about too.
        assert!(reruns_on_resume(&t, &p, "parked", &BTreeSet::new()).is_empty());
    }

    #[test]
    fn an_attempt_parked_at_its_question_is_asked_about_again() {
        let p = kinds_pipeline();
        let mut t = bare_ticket();
        t.attempts = vec![
            cancelled("review", 1, AttemptKind::Workflow, "parked", 1),
            cancelled("plan", 1, AttemptKind::Agent, "parked", 1),
        ];
        let about = |name: &str, stage: &str, state: DecisionState| Decision {
            id: format!("d-{name}"),
            stage: stage.into(),
            name: name.into(),
            kind: DecisionKind::Permission,
            question: String::new(),
            options: vec![],
            recommendation: None,
            attempt: Some((stage.into(), 1)),
            state,
            made_ms: 1,
            refusals: Vec::new(),
        };
        t.decisions = vec![
            about("finalize", "review", DecisionState::Cancelled),
            about(
                "paused",
                "plan",
                DecisionState::Answered {
                    answer: "park".into(),
                    note: None,
                    by: BY_HAND.into(),
                    at_ms: 1,
                    acted: true,
                },
            ),
        ];
        assert!(rerun_of(&t, &p).is_empty(), "{:?}", rerun_of(&t, &p));
    }

    #[test]
    fn a_resolution_review_is_rerun_only_where_the_loop_still_launches_one() {
        let p = kinds_pipeline();
        let mut t = bare_ticket();
        let moved = Refreshed {
            from: "base0".into(),
            to: "base1".into(),
            commits: true,
            notes: None,
            at_ms: 100,
            // Past `review-code`, the last code review.
            conflict: Some(RefreshConflict {
                before: "head0".into(),
                from: "base0".into(),
                to: "base1".into(),
                commits: vec![],
                stage: 3,
                at_ms: 90,
            }),
            after: Some("head1".into()),
        };
        t.lanes[0].refreshed = Some(moved.clone());
        t.attempts = vec![cancelled(RESOLUTION, 1, AttemptKind::Review, "parked", 150)];
        assert_eq!(rerun_of(&t, &p), [(RESOLUTION.to_owned(), 1)]);
        // The bring-up was pushed since: the resolution is on the PR.
        let mut pushed = t.clone();
        pushed.lanes[0].pushed = Some(crate::ticket::PushedHead {
            head: "head1".into(),
            at_ms: 100,
        });
        assert!(rerun_of(&pushed, &p).is_empty());
        // A newer bring-up: the review belongs to the one before it.
        let mut newer = t.clone();
        newer.lanes[0].refreshed = Some(Refreshed {
            at_ms: 200,
            ..moved.clone()
        });
        assert!(rerun_of(&newer, &p).is_empty());
        // A conflict a code review still reads is not the loop's.
        let mut read = t.clone();
        read.lanes[0]
            .refreshed
            .as_mut()
            .unwrap()
            .conflict
            .as_mut()
            .unwrap()
            .stage = 2;
        assert!(rerun_of(&read, &p).is_empty());
    }

    struct NoPort;
    impl crate::port::Port for NoPort {
        fn call(&mut self, _: &Request) -> std::io::Result<Reply> {
            Err(std::io::Error::other("no Switchboard in this test"))
        }
    }

    #[test]
    fn a_lock_not_taken_back_refuses_every_later_write_in_its_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let lock = data.root.join("lock");
        let mut r = Runner::new(
            data,
            Box::new(NoPort),
            Box::new(crate::git::FakeRepo::default()),
        );
        let ps = ProjectState {
            name: "P".into(),
            ..ProjectState::default()
        };
        let refused = r.transaction(|r| {
            // A directory where the lock file was: the take after the
            // slow work cannot open it.
            let lost = r.unlocked(|_| {
                std::fs::remove_file(&lock).unwrap();
                std::fs::create_dir(&lock).unwrap();
            });
            assert!(lost.is_err());
            std::fs::remove_dir(&lock).unwrap();
            // The lock could be taken now, but whatever the transaction
            // read before the gap is stale: the save is still refused.
            r.save_project(&ps)
        });
        assert!(format!("{:#}", refused.unwrap_err()).contains("writer lock was lost"),);
        assert!(!crate::store::record_exists(&r.data.project_file("P")));
        r.transaction(|r| r.save_project(&ps)).unwrap();
        assert!(crate::store::record_exists(&r.data.project_file("P")));
    }

    #[test]
    fn only_a_stop_after_the_last_nudge_counts() {
        assert!(!stopped_since(None, None));
        assert!(stopped_since(Some(5), None));
        assert!(!stopped_since(Some(5), Some(9)));
        assert!(!stopped_since(Some(9), Some(9)));
        assert!(stopped_since(Some(12), Some(9)));
    }

    #[test]
    fn the_dirty_reason_counts_its_nudges() {
        assert_eq!(dirty_suffix(0), "");
        assert_eq!(dirty_suffix(1), ", after 1 nudge");
        assert_eq!(dirty_suffix(2), ", after 2 nudges");
    }

    #[test]
    fn a_dirty_stop_is_a_nudge_while_one_is_left_then_the_reason() {
        let nudge = OnDirty::Nudge(2);
        let grace = STOP_IDLE_POLLS;
        assert_eq!(
            dirty_step(nudge, &[], Some(5), 0, true, 9),
            DirtyStep::Nudge {
                at_ms: 9,
                k: 1,
                of: 2
            }
        );
        // A stop Switchboard stamped after the pass began.
        assert_eq!(
            dirty_step(nudge, &[9], Some(20), 0, true, 15),
            DirtyStep::Nudge {
                at_ms: 21,
                k: 2,
                of: 2
            }
        );
        let fail = |s: &str| DirtyStep::Fail(s.into());
        assert_eq!(dirty_step(OnDirty::Ask, &[], Some(5), 0, true, 9), fail(""));
        assert_eq!(
            dirty_step(nudge, &[9, 21], Some(30), 0, true, 40),
            fail(", after 2 nudges")
        );
        assert_eq!(
            dirty_step(nudge, &[9], Some(5), grace, true, 40),
            fail(", after 1 nudge; no stop came after the last nudge")
        );
        assert_eq!(
            dirty_step(nudge, &[9], Some(5), 0, false, 40),
            fail(", after 1 nudge; the session ended with no stop after the last nudge")
        );
        assert_eq!(
            dirty_step(nudge, &[9], Some(12), 0, false, 40),
            fail(", after 1 nudge")
        );
        assert_eq!(
            dirty_step(nudge, &[], Some(5), 0, false, 9),
            fail("; the session had ended, so it was not nudged")
        );
    }

    #[test]
    fn a_nudge_goes_unanswered_at_the_idle_grace_with_no_stop_after_it() {
        let grace = STOP_IDLE_POLLS;
        assert!(!unanswered(None, &[], grace));
        assert!(!unanswered(Some(12), &[9], grace));
        assert!(!unanswered(Some(5), &[9], grace - 1));
        assert!(unanswered(Some(5), &[9], grace));
        assert!(unanswered(None, &[9], grace));
    }

    const PIPELINE: &str = r#"
version = 1

[project]
name = "Orchard"
repo = "git@example.com:example-org/orchard-workspace.git"
space = "Dispatch · Orchard"

[source]
kind = "manual"

[[lanes]]
name = "backend"
path = "orchard-backend"

[[lanes]]
name = "frontend"
path = "orchard-frontend"

[[stages]]
name = "deploy"
context = "lane:backend"
gate = { kind = "command", in = "lane:backend", argv = ["true"] }

[[stages]]
name = "both"
context = ["backend", "frontend"]
gate = { kind = "human", decision = "both" }

[[stages]]
name = "each"
context = "each"
gate = { kind = "human", decision = "each" }
"#;

    fn ticket(chosen: &[&str]) -> Ticket {
        let lane = |name: &str| LaneRecord {
            name: name.into(),
            worktree: PathBuf::from(format!("/wt/t/orchard-{name}")),
            branch: "dispatch/1-x".into(),
            project: None,
            chosen: chosen.contains(&name),
            setup_done: false,
            base_sha: None,
            refreshed: None,
            pushed: None,
            removed: false,
            conflict: None,
        };
        Ticket {
            version: 0,
            id: "t".into(),
            project: "Orchard".into(),
            source: SourceSnapshot {
                kind: "manual".into(),
                identity: "x".into(),
                number: Some(1),
                title: "x".into(),
                body: String::new(),
                url: None,
                labels: vec![],
                taken_at_ms: 0,
                pull_requests: vec![],
                taken_by: None,
            },
            pipeline_fingerprint: String::new(),
            pipeline_file: PathBuf::new(),
            lanes: vec![lane("backend"), lane("frontend")],
            tree: Some("/wt/t".into()),
            stage: 0,
            attempts: vec![],
            decisions: vec![],
            ledger: vec![],
            processes: vec![],
            root_project: None,
            rework: BTreeMap::new(),
            refreshed_stage: None,
            state: TicketState::Active,
            state_by: None,
            close: CloseProgress::default(),
            restarts: vec![],
            restart: None,
            entered: vec![],
            holds: vec![],
            services: vec![],
            created_ms: 0,
            updated_ms: 0,
        }
    }

    fn names(t: &Ticket, p: &Pipeline, stage: usize) -> Vec<String> {
        Runner::contexts(t, p, &p.stages[stage])
            .into_iter()
            .map(|(name, _, _)| name)
            .collect()
    }

    #[test]
    fn a_lane_context_skips_unchosen_lanes_but_each_still_parks_with_none() {
        let p = Pipeline::parse(PIPELINE).unwrap();
        let t = ticket(&["frontend"]);
        assert!(names(&t, &p, 0).is_empty());
        assert!(
            Runner::skipped(&t, &p.stages[0]),
            "lane:backend, not chosen"
        );
        assert_eq!(names(&t, &p, 1), ["frontend"]);
        assert!(
            !Runner::skipped(&t, &p.stages[1]),
            "one of its lanes is chosen"
        );
        assert_eq!(names(&t, &p, 2), ["frontend"]);
        let t = ticket(&["backend", "frontend"]);
        assert_eq!(names(&t, &p, 0), ["backend"]);
        assert!(!Runner::skipped(&t, &p.stages[0]));
        // `each` with nothing chosen has no context and is not skipped:
        // `contexts_or_park` parks it, as before.
        let t = ticket(&[]);
        assert!(names(&t, &p, 2).is_empty());
        assert!(!Runner::skipped(&t, &p.stages[2]));
        assert!(Runner::skipped(&t, &p.stages[1]));
    }

    #[test]
    fn a_skipped_deploy_reads_as_skipped_with_nothing_served() {
        let p = Pipeline::parse(PIPELINE).unwrap();
        let mut t = ticket(&["frontend"]);
        t.stage = 1;
        assert_eq!(
            deployed_and_served(&t, &p),
            "\nDeployed: deploy skipped (no backend lane)"
        );
        t.stage = 0;
        assert_eq!(deployed_and_served(&t, &p), "", "nothing before it");
    }

    /// Two lanes, `A` and `B`: a root `investigate` and a per-lane
    /// `implement` both write `notes`, a per-lane gate-only `deploy`
    /// records a head, and a per-lane `read` names them.
    fn lanes_pipeline() -> Pipeline {
        Pipeline::parse(
            r#"
version = 1

[project]
name = "P"
repo = "git@example.com:o/p.git"
space = "Dispatch · P"

[source]
kind = "github"
repo = "o/p"
label = "dispatch"

[[lanes]]
name = "A"
path = "a"

[[lanes]]
name = "B"
path = "b"

[operators.agent]
kind = "claude"

[[stages]]
name = "investigate"
operator = "agent"
context = "root"
writes = ["notes"]
prompt = "Write {notes}."

[[stages]]
name = "implement"
operator = "agent"
context = "each"
writes = ["notes"]
prompt = "Write {notes}."

[[stages]]
name = "deploy"
context = "each"
gate = { kind = "command", argv = ["make", "deploy"], in = "lane" }

[[stages]]
name = "read"
operator = "agent"
context = "each"
prompt = "Read {inputs.notes} and {inputs.implement.notes} at {inputs.deploy.commit}."
"#,
        )
        .unwrap()
    }

    /// A ticket at the `read` stage holding `attempts`, each a stage,
    /// a context, and the path it wrote `notes` to.
    fn notes_ticket(attempts: &[(&str, &str, &str)]) -> Ticket {
        let mut t = crate::ticket::blank();
        t.stage = 3;
        for (i, (stage, ctx, path)) in attempts.iter().enumerate() {
            t.attempts.push(new_attempt(
                stage,
                u32::try_from(i).unwrap() + 1,
                ctx,
                AttemptKind::Agent,
                AttemptState::Complete,
                BTreeMap::from([("notes".to_owned(), PathBuf::from(path))]),
                0,
            ));
        }
        t
    }

    fn field(v: &Vars, key: &str) -> Option<String> {
        v.0.get(key).cloned()
    }

    #[test]
    fn input_vars_for_both_forms_name_their_own_files() {
        let p = lanes_pipeline();
        let t = notes_ticket(&[
            ("implement", "A", "/impl/a.md"),
            ("investigate", "root", "/inv.md"),
        ]);
        let vars = vars_for(&t, &p, Some("A"));
        let env = input_env(
            &vars,
            "{inputs.notes} {inputs.investigate.notes} {inputs.implement.notes}",
        );
        assert_eq!(
            env,
            BTreeMap::from([
                ("DISPATCH_INPUT_NOTES".to_owned(), "/inv.md".to_owned()),
                (
                    "DISPATCH_INPUT_INVESTIGATE_NOTES".to_owned(),
                    "/inv.md".to_owned()
                ),
                (
                    "DISPATCH_INPUT_IMPLEMENT_NOTES".to_owned(),
                    "/impl/a.md".to_owned()
                ),
            ])
        );
    }

    #[test]
    fn a_lane_reader_gets_its_own_lanes_file_through_both_forms() {
        let p = lanes_pipeline();
        let t = notes_ticket(&[("implement", "A", "/a.md"), ("implement", "B", "/b.md")]);
        for (lane, path) in [("A", "/a.md"), ("B", "/b.md")] {
            let vars = vars_for(&t, &p, Some(lane));
            assert_eq!(
                field(&vars, "inputs.notes").as_deref(),
                Some(path),
                "{lane}"
            );
            assert_eq!(
                field(&vars, "inputs.implement.notes").as_deref(),
                Some(path),
                "{lane}"
            );
        }
    }

    #[test]
    fn a_lane_reader_still_gets_a_one_context_writers_file() {
        let p = lanes_pipeline();
        let t = notes_ticket(&[
            ("investigate", "root", "/inv.md"),
            ("implement", "B", "/b.md"),
        ]);
        let vars = vars_for(&t, &p, Some("A"));
        assert_eq!(field(&vars, "inputs.notes").as_deref(), Some("/inv.md"));
        assert_eq!(
            field(&vars, "inputs.investigate.notes").as_deref(),
            Some("/inv.md")
        );
        assert_eq!(
            field(&vars, "inputs.implement.notes"),
            None,
            "B's is not A's"
        );
        let vars = vars_for(&t, &p, Some("B"));
        assert_eq!(field(&vars, "inputs.notes").as_deref(), Some("/b.md"));
    }

    #[test]
    fn a_lanes_refresh_notes_stay_in_that_lane() {
        let p = lanes_pipeline();
        let t = notes_ticket(&[
            ("investigate", "root", "/inv.md"),
            (REFRESH, "B", "/refresh-b.md"),
        ]);
        let vars = vars_for(&t, &p, Some("A"));
        assert_eq!(field(&vars, "inputs.notes").as_deref(), Some("/inv.md"));
        let vars = vars_for(&t, &p, Some("B"));
        assert_eq!(
            field(&vars, "inputs.notes").as_deref(),
            Some("/refresh-b.md")
        );
    }

    #[test]
    fn a_per_lane_commit_follows_the_readers_lane() {
        let p = lanes_pipeline();
        let mut t = crate::ticket::blank();
        t.stage = 3;
        for (n, lane, head) in [(1, "A", "head-a"), (2, "B", "head-b")] {
            t.attempts.push(Attempt {
                head: Some(head.into()),
                ..new_attempt(
                    "deploy",
                    n,
                    lane,
                    AttemptKind::GateOnly,
                    AttemptState::Complete,
                    BTreeMap::new(),
                    0,
                )
            });
        }
        for (lane, head) in [("A", "head-a"), ("B", "head-b")] {
            let vars = vars_for(&t, &p, Some(lane));
            assert_eq!(field(&vars, "inputs.deploy.commit").as_deref(), Some(head));
        }
    }
}
