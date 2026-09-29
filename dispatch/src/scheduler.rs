//! One step of one ticket. The runner reads what Switchboard says about
//! the ticket's sessions and runs, decides, and acts through the port,
//! writing the ticket before and after every request. Nothing here
//! retries on its own: a failure is a decision.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow, bail};
use switchboard_control::{self as wire, Body, Reply, Request, RunState};

use crate::git::{Repo, branch_name};
use crate::pipeline::{Context, Gate, Pipeline, Stage, StageKind};
use crate::port::Port;
use crate::store::{DataDir, read_json, write_json};
use crate::template::Vars;
use crate::ticket::{
    Attempt, AttemptKind, AttemptState, Decision, DecisionKind, DecisionState, LaneRecord,
    Operation, ProjectState, SETTLE_POLLS, Settle, SourceSnapshot, Ticket, TicketState,
};

/// Everything the runner acts through.
pub struct Runner {
    pub data: DataDir,
    pub port: Box<dyn Port>,
    pub git: Box<dyn Repo>,
}

/// What `dispatch decide` writes as the answerer.
pub const BY_HAND: &str = "you";

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
        let path = self.data.ticket_file(&t.id);
        self.data.with_lock(|| write_json(&path, t))
    }

    pub fn load_project(&self, name: &str) -> Result<ProjectState> {
        let path = self.data.project_file(name);
        if path.exists() {
            read_json(&path)
        } else {
            Ok(ProjectState {
                name: name.to_owned(),
                ..ProjectState::default()
            })
        }
    }

    pub fn save_project(&self, ps: &ProjectState) -> Result<()> {
        let path = self.data.project_file(&ps.name);
        self.data.with_lock(|| write_json(&path, ps))
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
                        TicketState::Parked { reason } => format!("parked: {reason}"),
                        TicketState::Closed { .. } => unreachable!(),
                    }
                );
            }
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
            stage: 0,
            attempts: Vec::new(),
            decisions: Vec::new(),
            ledger: Vec::new(),
            processes: Vec::new(),
            root_project: None,
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
        for i in pending {
            self.recover_one(t, ps, i, now_ms)?;
        }
        self.act_on_answers(t, ps, p, now_ms)?;
        if !t.active() {
            return Ok(());
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
                if !held {
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

    fn park(&mut self, t: &mut Ticket, reason: &str, now_ms: u64) -> Result<()> {
        log::warn!("ticket {} parked: {reason}", t.id);
        t.state = TicketState::Parked {
            reason: reason.into(),
        };
        self.save_ticket(t, now_ms)
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
            if let DecisionState::Answered { acted, .. } = &mut t.decisions[i].state {
                *acted = true;
            }
            self.save_ticket(t, now_ms)?;
            match (name.as_str(), answer.as_str()) {
                ("lanes", lanes) => {
                    let names: Vec<String> = lanes
                        .split(',')
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty())
                        .collect();
                    self.cut_lanes(t, p, &names, now_ms)?;
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
                ("rerun", "rerun") => self.unmark(t, ps, now_ms)?,
                (_, "park") => {
                    self.park(t, &format!("parked by hand at decision {name}"), now_ms)?;
                }
                (name, other) => {
                    self.park(
                        t,
                        &format!("decision {name}: answer {other:?} is not one Dispatch knows"),
                        now_ms,
                    )?;
                }
            }
            if !t.active() {
                break;
            }
        }
        Ok(())
    }

    // --- lanes

    /// Cut a worktree and branch per lane named, then the lane's setup.
    fn cut_lanes(
        &mut self,
        t: &mut Ticket,
        p: &Pipeline,
        names: &[String],
        now_ms: u64,
    ) -> Result<()> {
        for name in names {
            if t.lanes.iter().any(|l| &l.name == name) {
                continue;
            }
            let Some(lane) = p.lane(name) else {
                self.park(
                    t,
                    &format!("lanes decision named an unknown lane {name:?}"),
                    now_ms,
                )?;
                return Ok(());
            };
            let Some(worktrees) = &lane.worktrees else {
                self.park(
                    t,
                    &format!("lane {name:?} works in place, which is not built in this slice"),
                    now_ms,
                )?;
                return Ok(());
            };
            let repo = p.project.root.join(&lane.path);
            let dir = worktrees.join(&t.id);
            let branch = branch_name(t.source.number.unwrap_or(0), &t.source.title);
            if let Err(e) = self.git.worktree_add(&repo, &dir, &branch, &lane.base) {
                self.park(t, &format!("could not cut lane {name}: {e}"), now_ms)?;
                return Ok(());
            }
            if !lane.setup.is_empty()
                && let Err(e) =
                    self.git
                        .run(&dir, &lane.setup, &env_for(t, Some(name), Some(&branch)))
            {
                self.park(t, &format!("lane {name}: setup failed: {e}"), now_ms)?;
                return Ok(());
            }
            t.lanes.push(LaneRecord {
                name: name.clone(),
                worktree: dir,
                branch,
                project: None,
            });
            self.save_ticket(t, now_ms)?;
        }
        Ok(())
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
                if !t.lanes.is_empty() {
                    self.finish_gate_only(t, ps, stage, now_ms)?;
                    return Ok(());
                }
                if p.dial("lanes") == "auto" && p.lanes.len() == 1 {
                    let names = vec![p.lanes[0].name.clone()];
                    self.cut_lanes(t, p, &names, now_ms)?;
                    if t.active() {
                        self.finish_gate_only(t, ps, stage, now_ms)?;
                    }
                    return Ok(());
                }
                let hinted = lane_hints(p, &t.source.labels);
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
            Some(gate) => {
                let kind = match gate {
                    Gate::Command { .. } => "command gate",
                    Gate::External { check, .. } => check.as_str(),
                    Gate::Human { decision, .. } => decision.as_str(),
                };
                self.park(
                    t,
                    &format!("stage {} ({kind}) is not built in this slice", stage.name),
                    now_ms,
                )
            }
            None => self.park(t, &format!("stage {} has no gate", stage.name), now_ms),
        }
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
        let root = ("root".to_owned(), p.project.root.clone(), None);
        match &stage.context {
            Context::Root => vec![root],
            Context::Joined => vec![("joined".to_owned(), p.project.root.clone(), None)],
            Context::Each => t
                .lanes
                .iter()
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
    fn ensure_project(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        lane: Option<&str>,
        cwd: &Path,
        now_ms: u64,
    ) -> Result<String> {
        let existing = match lane {
            None => t.root_project.clone(),
            Some(l) => t
                .lanes
                .iter()
                .find(|x| x.name == l)
                .and_then(|x| x.project.clone()),
        };
        if let Some(id) = existing {
            return Ok(id);
        }
        let space = self.ensure_space(t, ps, p, now_ms)?;
        let number = t.source.number.unwrap_or(0);
        let (intent, name) = match lane {
            None => ("root-project".to_owned(), format!("#{number}")),
            Some(l) => (format!("lane-project:{l}"), format!("#{number} {l}")),
        };
        let reply = self.send(
            t,
            ps,
            None,
            &intent,
            Body::ProjectAdd {
                space,
                name,
                root: cwd.to_path_buf(),
            },
            now_ms,
        )?;
        match lane {
            None => t.root_project.clone(),
            Some(l) => t
                .lanes
                .iter()
                .find(|x| x.name == l)
                .and_then(|x| x.project.clone()),
        }
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
        if let Some(gate) = &stage.gate {
            let kind = match gate {
                Gate::Command { .. } => "command gate",
                Gate::External { check, .. } => check.as_str(),
                Gate::Human { decision, .. } => decision.as_str(),
            };
            return self.park(
                t,
                &format!("stage {} ({kind}) is not built in this slice", stage.name),
                now_ms,
            );
        }
        let contexts = Self::contexts(t, p, stage);
        if contexts.is_empty() {
            return self.park(
                t,
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
                    self.poll_agent(t, ps, &a, now_ms)?;
                }
                Some(a) => {
                    // Failed: a rerun waits on its decision.
                    all_complete = false;
                    if !held && may_rerun(t, &a) {
                        self.start_agent(
                            t,
                            ps,
                            p,
                            stage,
                            &ctx,
                            &cwd,
                            lane.as_deref(),
                            a.n + 1,
                            now_ms,
                        )?;
                    }
                }
                None => {
                    all_complete = false;
                    if !held {
                        self.start_agent(t, ps, p, stage, &ctx, &cwd, lane.as_deref(), 1, now_ms)?;
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
                &format!("stage {} names no operator", stage.name),
                now_ms,
            );
        };
        let project = match self.ensure_project(t, ps, p, lane, cwd, now_ms) {
            Ok(id) => id,
            Err(e) => return self.park(t, &format!("stage {}: {e:#}", stage.name), now_ms),
        };
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
        if !operator.guidance.trim().is_empty() {
            prompt.push_str(operator.guidance.trim());
            prompt.push_str("\n\n");
        }
        prompt.push_str(&vars.render(stage.prompt.as_deref().unwrap_or_default()));
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
        t.attempts.push(attempt);
        self.save_ticket(t, now_ms)?;
        // The artifacts live outside the agent's cwd, in Dispatch's own
        // directory; Claude Code writes there unasked only under an
        // allow rule for the path (an added directory still asks before
        // creating a file).
        let (kind, launch) = match operator.kind {
            crate::pipeline::OperatorKind::Claude => {
                let mut args = operator.args.clone();
                args.push("--allowedTools".into());
                args.push(format!("Edit(//{}/**)", dir.display()));
                (wire::SessionKind::Claude, wire::Launch::Argv(args))
            }
            crate::pipeline::OperatorKind::Codex => (
                wire::SessionKind::Codex,
                if operator.args.is_empty() {
                    wire::Launch::Shell
                } else {
                    wire::Launch::Argv(operator.args.clone())
                },
            ),
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
    fn poll_agent(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        a: &Attempt,
        now_ms: u64,
    ) -> Result<()> {
        let Some(session) = a.session.clone() else {
            // Sent but no reply adopted yet: recovery's to resolve.
            return Ok(());
        };
        let reply = self.ask(Body::Session {
            session: session.clone(),
        })?;
        let view = match reply {
            Reply::Session { session } => session,
            Reply::Failed { reason } => {
                return self.fail_attempt(
                    t,
                    ps,
                    &a.stage,
                    a.n,
                    &format!("session gone: {reason}"),
                    now_ms,
                );
            }
            other => bail!("session query answered {other:?}"),
        };
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
        let missing: Vec<String> = attempt
            .artifacts
            .iter()
            .filter(|(_, path)| !path.is_file())
            .map(|(name, _)| name.clone())
            .collect();
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
        attempt.state = AttemptState::Complete;
        attempt.ended_ms = Some(now_ms);
        log::info!("ticket {} {}/{} complete", t.id, a.stage, a.context);
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
        Ok(())
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
        self.ensure_decision(
            t,
            ps,
            Ask {
                stage,
                name: "rerun",
                kind: DecisionKind::Permission,
                question: format!("{stage} ({ctx}) attempt {n} failed: {reason}. Run it again?"),
                options: &["rerun", "park"],
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
                            a.n + 1,
                            now_ms,
                        )?;
                    }
                }
                None => {
                    all_complete = false;
                    if !held {
                        self.start_workflow(t, ps, p, stage, &ctx, lane.as_deref(), 1, now_ms)?;
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
        let Some(review) = p
            .operators
            .get(&reviewer_name)
            .and_then(|o| o.review.clone())
        else {
            return self.park(
                t,
                &format!("stage {} names no reviewer", stage.name),
                now_ms,
            );
        };
        let Some(subject) = stage.subject.clone() else {
            return self.park(t, &format!("stage {} names no subject", stage.name), now_ms);
        };
        // The subject and the session that wrote it: the review clones
        // that session's conversation for the planner.
        let Some((source_path, source_session)) = t
            .attempts
            .iter()
            .rev()
            .filter(|a| a.state == AttemptState::Complete)
            .find_map(|a| {
                a.artifacts
                    .get(&subject)
                    .map(|path| (path.clone(), a.session.clone()))
            })
        else {
            return self.park(
                t,
                &format!("stage {}: nothing wrote {subject}", stage.name),
                now_ms,
            );
        };
        let Some(source_session) = source_session else {
            return self.park(
                t,
                &format!(
                    "stage {}: {subject} was not written by a session",
                    stage.name
                ),
                now_ms,
            );
        };
        let _ = lane;
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
        let definition = definition_of(&reviewer_name, review);
        t.attempts.push(new_attempt(
            &stage.name,
            n,
            ctx,
            AttemptKind::Workflow,
            AttemptState::Starting,
            BTreeMap::from([(subject.clone(), copy.clone())]),
            now_ms,
        ));
        self.save_ticket(t, now_ms)?;
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
                reviewer_cwd: Some(dir),
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
        self.save_ticket(t, now_ms)?;
        let subject = stage.subject.clone().unwrap_or_default();
        match view.state {
            RunState::Starting | RunState::AwaitingFeedback | RunState::AwaitingResponse => Ok(()),
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
            if t.active() && t.attempts.iter().any(Attempt::is_open) {
                running += 1;
            }
            pending += u32::try_from(t.pending_decisions().len()).unwrap_or(u32::MAX);
        }
        let mut pipeline: Option<Pipeline> = None;
        for t in &mut tickets {
            if !t.active() {
                continue;
            }
            let p = match self.pipeline_of(t) {
                Ok(p) => p,
                Err(e) => {
                    self.park(t, &format!("pipeline copy unreadable: {e}"), now_ms)?;
                    continue;
                }
            };
            let has_open = t.attempts.iter().any(Attempt::is_open);
            // Watching what runs is free; starting something new takes a
            // slot and is refused while too much waits on the user.
            let may_start =
                has_open || (running < p.policy.slots && pending < p.policy.waiting_on_me);
            if !may_start {
                continue;
            }
            let attempts_before = t.attempts.len();
            if let Err(e) = self.step(t, &mut ps, &p, now_ms) {
                log::error!("ticket {}: {e}", t.id);
            }
            if !has_open
                && t.attempts.len() > attempts_before
                && t.attempts.iter().any(Attempt::is_open)
            {
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
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "json")
                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
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
        let mut t = self.load_ticket(ticket)?;
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
        let path = self.data.ticket_file(ticket);
        t.updated_ms = now_ms;
        self.data.with_lock(|| write_json(&path, &t))?;
        Ok(d)
    }
}

impl Runner {
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
fn definition_of(reviewer_name: &str, review: crate::pipeline::Review) -> wire::Definition {
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

/// Whether a failed attempt's rerun was authorised.
fn may_rerun(t: &Ticket, failed: &Attempt) -> bool {
    t.decisions.iter().any(|d| {
        d.name == "rerun"
            && d.attempt.as_ref() == Some(&(failed.stage.clone(), failed.n))
            && matches!(&d.state, DecisionState::Answered { answer, .. } if answer == "rerun")
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
        .set("project.root", p.project.root.display().to_string())
        .set(
            "lanes",
            t.lanes
                .iter()
                .map(|l| l.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        );
    if let Some(l) = lane.and_then(|name| t.lanes.iter().find(|x| x.name == name)) {
        vars.set("lane", l.name.clone())
            .set("branch", l.branch.clone())
            .set("worktree", l.worktree.display().to_string());
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
