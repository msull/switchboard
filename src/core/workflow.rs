//! Workflow runs: the plan review loop as a state machine over records.
//! A run launches its reviewer, clones its planner, and then only ever
//! waits for a file, prompts an agent, or stops. Nothing here retries.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::core::action::{AppAction, AppCore, Clock, Effect, Out, View};
use crate::core::model::{
    Activity, AgentKind, CardLayout, HandoffMode, Launch, RecordId, ResumeHandle, Round, RunState,
    SessionKind, SessionRecord, Verdict, WorkflowDefinition, WorkflowId, WorkflowRun,
};
use crate::core::sessions::next_order;
use crate::ports::host::Liveness;
use crate::ports::round_files::{FileStamp, Probed};

/// Probes in a row that must find the file unchanged before it counts
/// as written. The poll is once a second.
pub const SETTLE_PROBES: u8 = 3;
/// A waiting round's agent that has printed nothing for this long is
/// most likely at an approval prompt: the run is marked stalled and the
/// user told once.
pub const STALL_AFTER: Duration = Duration::from_secs(120);
/// How long a Claude Code agent must have read idle after its Stop,
/// with its round file still missing, before the run fails. Claude Code
/// can end a turn with background work that writes later, so a Stop
/// alone is not enough. Dispatch's `STOP_IDLE_POLLS` is the same wait
/// for a stage agent.
pub const STOP_GRACE: Duration = Duration::from_secs(30);

/// How long a run's awaited file has looked the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probe {
    pub run: WorkflowId,
    pub stamp: FileStamp,
    pub streak: u8,
}

/// Where a round's copies go: `<data dir>/workflows/<run>/round-<n>/`.
#[must_use]
pub fn snapshot_dir(data_dir: &Path, run: WorkflowId, n: u32) -> PathBuf {
    data_dir
        .join("workflows")
        .join(run.0.to_string())
        .join(format!("round-{n}"))
}

fn round_count(run: &WorkflowRun) -> u32 {
    u32::try_from(run.rounds.len()).unwrap_or(u32::MAX)
}

/// Round file names beside the plan: `<stem>.feedback-<n>.md` and
/// `<stem>.response-<n>.md`.
#[must_use]
pub fn round_paths(plan: &Path, n: u32) -> (PathBuf, PathBuf) {
    let stem = plan_stem(plan);
    let dir = plan.parent().unwrap_or_else(|| Path::new("/"));
    (
        dir.join(format!("{stem}.feedback-{n}.md")),
        dir.join(format!("{stem}.response-{n}.md")),
    )
}

/// The plan's file name without its extension, which names the round
/// files and the reviewer.
fn plan_stem(plan: &Path) -> &str {
    plan.file_stem().and_then(|s| s.to_str()).unwrap_or("plan")
}

/// Round `n` with nothing in it yet, its files beside the plan.
fn fresh_round(plan: &Path, n: u32) -> Round {
    let (feedback, response) = round_paths(plan, n);
    Round {
        n,
        feedback,
        response,
        verdict: None,
        user_feedback: None,
        responded: false,
        snapshot: false,
        feedback_asked: None,
        response_asked: None,
    }
}

impl Round {
    /// When the agent `state` waits on was last asked for this round's
    /// file.
    fn asked(&self, state: &RunState) -> Option<SystemTime> {
        match state {
            RunState::AwaitingFeedback => self.feedback_asked,
            RunState::AwaitingResponse => self.response_asked,
            _ => None,
        }
    }

    /// Record that the agent `state` waits on was asked at `at`.
    fn set_asked(&mut self, state: &RunState, at: SystemTime) {
        match state {
            RunState::AwaitingFeedback => self.feedback_asked = Some(at),
            RunState::AwaitingResponse => self.response_asked = Some(at),
            _ => {}
        }
    }
}

/// When the run's awaited agent was asked for the current round's file.
fn asked_at(run: &WorkflowRun) -> Option<SystemTime> {
    run.current()?.asked(&run.state)
}

/// The planner's prompt for `round`: the user's own feedback is sent in
/// it, the reviewer's is a file to read.
fn respond_prompt(def: &WorkflowDefinition, round: &Round, plan: &Path, cap: u32) -> String {
    match &round.user_feedback {
        Some(text) => def
            .render(&def.respond_to_user, round, plan, cap)
            .replace("{text}", text),
        None => def.render(&def.respond, round, plan, cap),
    }
}

/// The word a round goes by in the rounds list: who is at work on it,
/// or what came of it.
#[must_use]
pub fn round_status(run: &WorkflowRun, round: &Round) -> &'static str {
    let at_work = run.current().map(|r| r.n) == Some(round.n) && run.awaiting().is_some();
    match (round.user_feedback.is_some(), round.verdict, &run.state) {
        (_, _, RunState::AwaitingResponse) if at_work => "answering",
        (true, _, _) => "your feedback",
        (false, Some(Verdict::Nothing), _) => "nothing further",
        (false, Some(Verdict::Changes), _) if round.responded => "answered",
        (false, Some(Verdict::Changes), _) => "changes asked",
        (false, None, RunState::Starting) => "starting",
        (false, None, _) => "reviewing",
    }
}

impl AppCore {
    /// The workflows' transitions, split out of `dispatch` for length.
    pub(super) fn workflow_action(&mut self, action: AppAction, now: Clock, out: &mut Out) {
        match action {
            AppAction::StartWorkflow {
                source,
                plan,
                definition,
            } => self.start_workflow(source, &plan, &definition, None, Vec::new(), now, out),
            AppAction::ShowWorkflow(id) => {
                if self.workflow(id).is_some() {
                    self.show(View::Workflow(id), now, out);
                }
            }
            AppAction::PauseWorkflow(id) => self.pause_workflow(id, "paused by you", now, out),
            AppAction::ContinueWorkflow(id) => self.continue_workflow(id, now, out),
            AppAction::RaiseWorkflowCap { run, cap } => self.raise_cap(run, cap, now, out),
            AppAction::FinalizeWorkflow(id) => {
                if self.workflow(id).is_some_and(|r| !r.state.waiting()) {
                    self.edit_run(id, now, out, |r| r.state = RunState::Finalized);
                }
            }
            AppAction::CleanUpWorkflow(id) => self.clean_up(id, out),
            AppAction::HandOffWorkflow { run, mode } => self.hand_off(run, mode, now, out),
            AppAction::UserFeedback { run, text } => self.user_feedback(run, &text, now, out),
            AppAction::RemoveWorkflow(id) => self.remove_workflow(id, out),
            AppAction::SetWorkflowRoundCap(cap) => {
                self.update_settings(out, |s| s.workflow_round_cap = cap.max(1));
            }
            AppAction::SetWorkflowDefinitions(defs) => {
                self.update_settings(out, |s| s.workflows = defs);
            }
            AppAction::WorkflowCloned { run, result } => {
                self.workflow_cloned(run, result, now, out);
            }
            AppAction::RoundFileProbed { run, path, found } => {
                self.round_file_probed(run, &path, found, now, out);
            }
            AppAction::RoundSnapshotted { run, n, result } => match result {
                Ok(()) => self.edit_run(run, now, out, |r| {
                    if let Some(round) = r.rounds.iter_mut().find(|x| x.n == n) {
                        round.snapshot = true;
                    }
                }),
                Err(e) => self.error(format!("could not snapshot round {n}: {e}")),
            },
            AppAction::RoundFilesRemoved { run, result } => match result {
                Ok(()) => {
                    self.edit_run(run, now, out, |r| r.cleaned = true);
                    self.info("round files deleted", now);
                }
                Err(e) => self.error(format!("could not delete the round files: {e}")),
            },
            _ => unreachable!("not a workflow action"),
        }
    }

    // --- read model

    /// Every run of every project.
    pub fn workflows(&self) -> impl Iterator<Item = &WorkflowRun> {
        self.workspaces.iter().flat_map(|w| &w.workflows)
    }

    #[must_use]
    pub fn workflow(&self, id: WorkflowId) -> Option<&WorkflowRun> {
        self.workflows().find(|r| r.id == id)
    }

    /// The definition a run uses, falling back to the built-in when the
    /// user's copy is gone.
    #[must_use]
    pub fn definition_of(&self, run: &WorkflowRun) -> WorkflowDefinition {
        self.settings.workflow(&run.definition).unwrap_or_default()
    }

    fn workflow_mut(&mut self, id: WorkflowId) -> Option<&mut WorkflowRun> {
        self.workspaces
            .iter_mut()
            .flat_map(|w| &mut w.workflows)
            .find(|r| r.id == id)
    }

    fn edit_run(
        &mut self,
        id: WorkflowId,
        now: Clock,
        out: &mut Out,
        edit: impl FnOnce(&mut WorkflowRun),
    ) {
        if let Some(run) = self.workflow_mut(id) {
            let project = run.project;
            edit(run);
            run.updated = now.wall;
            out.touch(project);
        }
    }

    // --- start

    /// `reviewer_cwd` is where the reviewer runs when it is not the
    /// source's directory: Codex writes only inside its own, and a
    /// review whose files live elsewhere names that place.
    /// `reviewer_args` are extra flags for the reviewer's command line.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_workflow(
        &mut self,
        source: RecordId,
        plan: &Path,
        definition: &str,
        reviewer_cwd: Option<PathBuf>,
        reviewer_args: Vec<String>,
        now: Clock,
        out: &mut Out,
    ) {
        let Some(def) = self.settings.workflow(definition) else {
            self.error(format!("no workflow definition called {definition}"));
            return;
        };
        let Some(record) = self.session(source).cloned() else {
            return;
        };
        let (
            SessionKind::Agent(AgentKind::ClaudeCode),
            Some(handle @ ResumeHandle::ClaudeCode { .. }),
        ) = (&record.kind, &record.resume)
        else {
            self.error(format!(
                "{} cannot be the planner: only a Claude Code session with a transcript can be cloned",
                record.name
            ));
            return;
        };
        let handle = handle.clone();
        if !plan.is_absolute() {
            self.error("the plan path must be absolute");
            return;
        }
        if let Some(text) = self.host_unavailable("start", "a review") {
            self.error(text);
            return;
        }
        let stem = plan_stem(plan).to_owned();
        let Some(reviewer) = self.add_record(
            record.project,
            format!("{stem} review"),
            SessionKind::Agent(def.reviewer),
            reviewer_cwd.unwrap_or_else(|| record.cwd.clone()),
            if reviewer_args.is_empty() {
                Launch::Shell
            } else {
                Launch::Argv(reviewer_args)
            },
            now,
            out,
        ) else {
            return;
        };
        let cap = def.cap.unwrap_or(self.settings.workflow_round_cap).max(1);
        let round = Round {
            feedback_asked: Some(now.wall),
            ..fresh_round(plan, 1)
        };
        let id = WorkflowId::new();
        let run = WorkflowRun {
            id,
            project: record.project,
            definition: def.name.clone(),
            source,
            plan: plan.to_path_buf(),
            planner: None,
            reviewer,
            rounds: vec![round.clone()],
            state: RunState::Starting,
            cap,
            cleaned: false,
            created: now.wall,
            updated: now.wall,
            op: self.quiet_op.clone(),
        };
        let prompt = def.render(&def.review_first, &round, plan, cap);
        if let Some(w) = self
            .workspaces
            .iter_mut()
            .find(|w| w.project.id == record.project)
        {
            w.workflows.push(run);
        }
        self.first_prompts.push((reviewer, prompt));
        self.launch_fresh(reviewer, now, out);
        out.push(Effect::CloneAllTranscript { run: id, handle });
        self.show(View::Workflow(id), now, out);
    }

    fn workflow_cloned(
        &mut self,
        id: WorkflowId,
        result: Result<ResumeHandle, String>,
        now: Clock,
        out: &mut Out,
    ) {
        let Some(run) = self.workflow(id).cloned() else {
            return;
        };
        let handle = match result {
            Ok(h) => h,
            Err(e) => {
                self.pause_workflow(
                    id,
                    &format!("could not clone the planning session: {e}"),
                    now,
                    out,
                );
                return;
            }
        };
        let Some(source) = self.session(run.source).cloned() else {
            return;
        };
        let Some(workspace) = self
            .workspaces
            .iter_mut()
            .find(|w| w.project.id == run.project)
        else {
            return;
        };
        let order = next_order(workspace);
        let planner = RecordId::new();
        workspace.sessions.push(SessionRecord {
            id: planner,
            name: format!("{} planner", source.name),
            created: now.wall,
            last_seen: now.wall,
            resume: Some(handle),
            autostart: false,
            layout: CardLayout { order, group: None },
            activity: Activity::Unknown,
            activity_reason: None,
            last_event_at: None,
            last_exit: None,
            not_resumable: false,
            scrollback: None,
            source: None,
            approved_hash: None,
            discard: None,
            runs: Vec::new(),
            outputs: Vec::new(),
            // The clone belongs to the run's operation, not the source's.
            op: run.op.clone(),
            waiting_on: None,
            pending_launch: false,
            last_stop_at: None,
            env: Vec::new(),
            // A planner is no Dispatch stage and is granted nothing.
            env_sets: Vec::new(),
            token_hash: None,
            ..source
        });
        self.edit_run(id, now, out, |r| {
            r.planner = Some(planner);
            if r.state == RunState::Starting {
                r.state = RunState::AwaitingFeedback;
            }
        });
    }

    // --- waiting

    /// The agents of stalled runs: quiet mid-round, most likely at a
    /// prompt of their own.
    pub(super) fn stalled_agents(&self) -> impl Iterator<Item = RecordId> + '_ {
        self.stalled
            .iter()
            .filter_map(|id| self.workflow(*id).and_then(WorkflowRun::awaiting))
    }

    /// Whether the run's agent has been quiet for [`STALL_AFTER`] while
    /// the round waits on it.
    #[must_use]
    pub fn stalled(&self, id: WorkflowId) -> bool {
        self.stalled.contains(&id)
    }

    /// Once a second: probe each waiting run's file, and notice an
    /// awaited agent whose pane has exited without writing it.
    pub(super) fn workflow_tick(&mut self, now: Clock, out: &mut Out) {
        let waiting: Vec<_> = self
            .workflows()
            .filter(|r| r.state.waiting())
            .filter_map(|r| Some((r.id, r.awaited_file()?.clone(), r.awaiting()?)))
            .collect();
        self.stalled
            .retain(|id| waiting.iter().any(|(run, _, _)| run == id));
        for (run, path, agent) in waiting {
            self.watch_for_stall(run, agent, &path, now);
            let exited = self
                .host_status(agent)
                .is_some_and(|h| matches!(h.liveness, Liveness::Exited { .. }));
            if exited {
                let name = self.session_name(agent);
                let file = path.display().to_string();
                self.fail_workflow(
                    run,
                    &format!("{name} exited before writing {file}"),
                    now,
                    out,
                );
                continue;
            }
            out.push(Effect::ProbeRoundFile { run, path });
        }
    }

    /// An agent that has printed nothing for [`STALL_AFTER`] while its
    /// file is awaited is most likely at an approval prompt Switchboard
    /// cannot see (Codex has no hooks). Say so once; a pane that moves
    /// again clears the mark, so a later stall is noticed again.
    fn watch_for_stall(&mut self, run: WorkflowId, agent: RecordId, path: &Path, now: Clock) {
        let quiet = self
            .running_status(agent)
            .and_then(|h| h.last_activity)
            .and_then(|t| now.wall.duration_since(t).ok());
        let stalled = quiet.is_some_and(|q| q >= STALL_AFTER);
        let marked = self.stalled.contains(&run);
        if stalled && !marked {
            self.stalled.push(run);
            let name = self.session_name(agent);
            let file = path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mins = quiet.map_or(0, |q| q.as_secs() / 60);
            self.info_about(
                agent,
                format!(
                    "{name} has been quiet for {mins} min while the review waits on {file}; \
                     it may be waiting on an approval"
                ),
                now,
            );
        } else if !stalled && marked {
            self.stalled.retain(|id| *id != run);
        }
    }

    fn round_file_probed(
        &mut self,
        id: WorkflowId,
        path: &Path,
        found: Option<Probed>,
        now: Clock,
        out: &mut Out,
    ) {
        let Some(run) = self.workflow(id) else {
            return;
        };
        if !run.state.waiting() || run.awaited_file().map(PathBuf::as_path) != Some(path) {
            return;
        }
        let Some(found) = found else {
            self.probes.retain(|p| p.run != id);
            self.fail_if_stopped(id, path, now, out);
            return;
        };
        let streak = match self.probes.iter_mut().find(|p| p.run == id) {
            Some(p) if p.stamp == found.stamp => {
                p.streak = p.streak.saturating_add(1);
                p.streak
            }
            Some(p) => {
                p.stamp = found.stamp;
                p.streak = 1;
                1
            }
            None => {
                self.probes.push(Probe {
                    run: id,
                    stamp: found.stamp,
                    streak: 1,
                });
                1
            }
        };
        if streak >= SETTLE_PROBES {
            self.probes.retain(|p| p.run != id);
            self.settled(id, &found.first_line, now, out);
        }
    }

    /// The awaited file is missing: fail the run when its agent stopped
    /// after it was asked and has read idle for [`STOP_GRACE`]. A round
    /// with no asked time (saved before it was kept) never fails here,
    /// since a recorded stop may be the previous round's.
    fn fail_if_stopped(&mut self, id: WorkflowId, path: &Path, now: Clock, out: &mut Out) {
        let Some(run) = self.workflow(id) else {
            return;
        };
        let (Some(agent), Some(asked)) = (run.awaiting(), asked_at(run)) else {
            return;
        };
        let Some(stop) = self.stopped_since(agent, asked) else {
            return;
        };
        if now
            .wall
            .duration_since(stop)
            .is_ok_and(|idle| idle >= STOP_GRACE)
        {
            let name = self.session_name(agent);
            let file = path.display().to_string();
            self.fail_workflow(
                id,
                &format!("{name} stopped without writing {file}"),
                now,
                out,
            );
        }
    }

    /// The agent's last Stop, when it came strictly after `asked` (one
    /// at or before is the previous turn's) and its card reads idle. A
    /// queued prompt reads `Working` and a permission prompt or a
    /// question `WaitingOnYou`, neither of which is done. A Codex card
    /// never reads idle (it has no hooks), so it never matches.
    fn stopped_since(&self, agent: RecordId, asked: SystemTime) -> Option<SystemTime> {
        let session = self.session(agent)?;
        let stop = session.last_stop_at?;
        (stop > asked && session.activity == Activity::Idle).then_some(stop)
    }

    /// The awaited file stopped changing: record what it says and move
    /// the run along.
    fn settled(&mut self, id: WorkflowId, first_line: &str, now: Clock, out: &mut Out) {
        let Some(run) = self.workflow(id).cloned() else {
            return;
        };
        let def = self.definition_of(&run);
        let Some(round) = run.current().cloned() else {
            return;
        };
        match run.state {
            RunState::AwaitingFeedback => {
                let verdict = if first_line.trim() == def.no_feedback.trim() {
                    Verdict::Nothing
                } else {
                    Verdict::Changes
                };
                self.edit_run(id, now, out, |r| {
                    if let Some(x) = r.rounds.last_mut() {
                        x.verdict = Some(verdict);
                    }
                    r.state = match verdict {
                        Verdict::Nothing => RunState::Converged,
                        Verdict::Changes => RunState::AwaitingResponse,
                    };
                });
                Self::snapshot(&run, &round, out);
                if verdict == Verdict::Changes {
                    let prompt = def.render(&def.respond, &round, &run.plan, run.cap);
                    self.prompt_planner(id, &prompt, now, out);
                }
            }
            RunState::AwaitingResponse => {
                self.edit_run(id, now, out, |r| {
                    if let Some(x) = r.rounds.last_mut() {
                        x.responded = true;
                    }
                });
                Self::snapshot(&run, &round, out);
                if round.user_feedback.is_some() {
                    self.edit_run(id, now, out, |r| r.state = RunState::Converged);
                } else if round.n >= run.cap {
                    self.edit_run(id, now, out, |r| r.state = RunState::AtCap);
                } else {
                    self.next_review_round(id, now, out);
                }
            }
            _ => {}
        }
    }

    fn snapshot(run: &WorkflowRun, round: &Round, out: &mut Out) {
        out.push(Effect::SnapshotRound {
            run: run.id,
            n: round.n,
            files: vec![
                run.plan.clone(),
                round.feedback.clone(),
                round.response.clone(),
            ],
            note: round.user_feedback.clone(),
        });
    }

    /// Open round n+1 and ask the reviewer again.
    fn next_review_round(&mut self, id: WorkflowId, now: Clock, out: &mut Out) {
        let Some(run) = self.workflow(id).cloned() else {
            return;
        };
        let def = self.definition_of(&run);
        let n = round_count(&run) + 1;
        let round = Round {
            feedback_asked: Some(now.wall),
            ..fresh_round(&run.plan, n)
        };
        // `{response}` in the reviewer's round prompt is the previous
        // round's, the one it is asked to read; `{feedback}` is the new
        // file to write.
        let template = match run.current() {
            Some(prev) => def
                .review_round
                .replace("{response}", &prev.response.display().to_string()),
            None => def.review_round.clone(),
        };
        let prompt = def.render(&template, &round, &run.plan, run.cap.max(n));
        self.edit_run(id, now, out, |r| {
            r.rounds.push(round);
            r.cap = r.cap.max(n);
            r.state = RunState::AwaitingFeedback;
        });
        self.prompt_agent(run.reviewer, &prompt, now, out);
    }

    fn prompt_planner(&mut self, id: WorkflowId, prompt: &str, now: Clock, out: &mut Out) {
        let Some(planner) = self.workflow(id).and_then(|r| r.planner) else {
            self.pause_workflow(id, "the planner clone does not exist yet", now, out);
            return;
        };
        self.edit_run(id, now, out, |r| {
            if let Some(round) = r.rounds.last_mut() {
                round.response_asked = Some(now.wall);
            }
        });
        self.prompt_agent(planner, prompt, now, out);
    }

    /// Send text into a running agent, or make it the first prompt of
    /// a launch when the pane is gone.
    fn prompt_agent(&mut self, agent: RecordId, prompt: &str, now: Clock, out: &mut Out) {
        if let Some(host) = self.running_host(agent) {
            out.push(Effect::SendInput {
                host,
                text: prompt.to_owned(),
            });
        } else {
            self.first_prompts.retain(|(r, _)| *r != agent);
            self.first_prompts.push((agent, prompt.to_owned()));
            self.return_to_session(agent, now, out);
        }
    }

    // --- user controls

    fn pause_workflow(&mut self, id: WorkflowId, reason: &str, now: Clock, out: &mut Out) {
        self.halt_workflow(id, RunState::Paused(reason.to_owned()), now, out);
    }

    /// Like a pause, but the awaited agent let the round down rather
    /// than the user stopping it; Dispatch reruns a failed review.
    fn fail_workflow(&mut self, id: WorkflowId, reason: &str, now: Clock, out: &mut Out) {
        self.halt_workflow(id, RunState::Failed(reason.to_owned()), now, out);
    }

    /// Stop waiting: no more probes, and the run rests in `state`.
    fn halt_workflow(&mut self, id: WorkflowId, state: RunState, now: Clock, out: &mut Out) {
        if self.workflow(id).is_none() {
            return;
        }
        self.probes.retain(|p| p.run != id);
        self.edit_run(id, now, out, |r| r.state = state);
    }

    /// From `Paused` or `Failed`, take the interrupted step again: wait
    /// for the same file, and re-prompt the agent if its pane is gone
    /// or it stopped since it was asked (for a round saved without an
    /// asked time, any recorded stop with its card idle). A running
    /// agent that has not stopped is still working, and the probe tells
    /// when it is done. From `AtCap` or `Converged`, one more round.
    fn continue_workflow(&mut self, id: WorkflowId, now: Clock, out: &mut Out) {
        let Some(run) = self.workflow(id).cloned() else {
            return;
        };
        let def = self.definition_of(&run);
        match run.state {
            RunState::Paused(_) | RunState::Failed(_) => {
                let Some(round) = run.current().cloned() else {
                    return;
                };
                if run.planner.is_none() {
                    let Some(handle) = self.session(run.source).and_then(|s| s.resume.clone())
                    else {
                        return;
                    };
                    self.edit_run(id, now, out, |r| r.state = RunState::Starting);
                    out.push(Effect::CloneAllTranscript { run: id, handle });
                    return;
                }
                let (state, agent, prompt) = match round.verdict {
                    None => {
                        let template = if round.n == 1 {
                            &def.review_first
                        } else {
                            &def.review_round
                        };
                        (
                            RunState::AwaitingFeedback,
                            run.reviewer,
                            def.render(template, &round, &run.plan, run.cap),
                        )
                    }
                    Some(Verdict::Changes) if !round.responded => (
                        RunState::AwaitingResponse,
                        run.planner.unwrap_or(run.reviewer),
                        respond_prompt(&def, &round, &run.plan, run.cap),
                    ),
                    Some(_) => {
                        self.next_review_round(id, now, out);
                        return;
                    }
                };
                let asked = round.asked(&state).unwrap_or(SystemTime::UNIX_EPOCH);
                let reprompt =
                    !self.is_running(agent) || self.stopped_since(agent, asked).is_some();
                self.edit_run(id, now, out, |r| {
                    if reprompt && let Some(round) = r.rounds.last_mut() {
                        round.set_asked(&state, now.wall);
                    }
                    r.state = state;
                });
                if reprompt {
                    self.prompt_agent(agent, &prompt, now, out);
                }
            }
            RunState::AtCap | RunState::Converged => self.next_review_round(id, now, out),
            _ => {}
        }
    }

    fn raise_cap(&mut self, id: WorkflowId, cap: u32, now: Clock, out: &mut Out) {
        let Some(run) = self.workflow(id).cloned() else {
            return;
        };
        let cap = cap.max(1);
        self.edit_run(id, now, out, |r| r.cap = cap);
        if run.state == RunState::AtCap && cap > round_count(&run) {
            self.next_review_round(id, now, out);
        }
    }

    fn clean_up(&mut self, id: WorkflowId, out: &mut Out) {
        let Some(run) = self.workflow(id) else {
            return;
        };
        if run.state.waiting() {
            self.error("pause the review before deleting its files");
            return;
        }
        out.push(Effect::RemoveRoundFiles {
            run: id,
            files: run.round_files(),
        });
    }

    fn hand_off(&mut self, id: WorkflowId, mode: HandoffMode, now: Clock, out: &mut Out) {
        let Some(run) = self.workflow(id).cloned() else {
            return;
        };
        if run.state.waiting() {
            self.error("the review is still running");
            return;
        }
        let def = self.definition_of(&run);
        let Some(round) = run.current().cloned() else {
            return;
        };
        let prompt = def.render(&def.handoff, &round, &run.plan, run.cap);
        let Some(source) = self.session(run.source).cloned() else {
            self.error("the planning session no longer exists");
            return;
        };
        // The prompt is never sent: it is primed as the message box's
        // draft of the session it goes to, so the user reads and edits
        // it before pressing send. Only `/compact` goes straight in.
        let shown = match mode {
            HandoffMode::AsIs => {
                self.return_to_session(run.source, now, out);
                run.source
            }
            HandoffMode::Compact => {
                let Some(host) = self.running_host(run.source) else {
                    self.error(format!(
                        "{} is not running; return to it first, or hand off as is",
                        source.name
                    ));
                    return;
                };
                out.push(Effect::SendInput {
                    host,
                    text: "/compact".into(),
                });
                run.source
            }
            HandoffMode::Fresh => {
                let Some(fresh) = self.add_record(
                    run.project,
                    format!("{} implement", source.name),
                    source.kind,
                    source.cwd.clone(),
                    Launch::Shell,
                    now,
                    out,
                ) else {
                    return;
                };
                self.launch_fresh(fresh, now, out);
                fresh
            }
        };
        self.primed.push((shown, prompt));
        self.edit_run(id, now, out, |r| r.state = RunState::HandedOff);
        self.show(View::Session(shown), now, out);
    }

    fn user_feedback(&mut self, id: WorkflowId, text: &str, now: Clock, out: &mut Out) {
        let Some(run) = self.workflow(id).cloned() else {
            return;
        };
        if run.state.waiting() || run.planner.is_none() {
            self.error("the review must be stopped, with a planner, before your own round");
            return;
        }
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let def = self.definition_of(&run);
        let n = round_count(&run) + 1;
        let round = Round {
            verdict: Some(Verdict::Changes),
            user_feedback: Some(text.to_owned()),
            ..fresh_round(&run.plan, n)
        };
        let prompt = respond_prompt(&def, &round, &run.plan, run.cap);
        self.edit_run(id, now, out, |r| {
            r.rounds.push(round);
            r.state = RunState::AwaitingResponse;
        });
        self.prompt_planner(id, &prompt, now, out);
    }

    fn remove_workflow(&mut self, id: WorkflowId, out: &mut Out) {
        let Some(run) = self.workflow(id).cloned() else {
            return;
        };
        if run.state.waiting() {
            self.error("pause the review before removing it");
            return;
        }
        self.probes.retain(|p| p.run != id);
        if let Some(w) = self
            .workspaces
            .iter_mut()
            .find(|w| w.project.id == run.project)
        {
            w.workflows.retain(|r| r.id != id);
            out.touch(run.project);
        }
        if self.view() == View::Workflow(id) {
            self.view_stack.pop();
        }
    }
}
