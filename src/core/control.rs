//! The control port's side of the core: what another process may ask
//! for, run quietly (no view changes, no terminal windows), with the
//! operation id stamped on every record made so the asker can find them
//! again. The wire format lives in the `switchboard-control` crate; the
//! `TryFrom<wire::Body>` at the end of this file translates it to
//! `ControlAction`, and the read models answer in its shapes.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::SystemTime;

use switchboard_control as wire;

use super::action::{AppAction, Out, RULE_COLUMNS, TYPED_HOLD};
use super::env::{SecretScope, valid_set_name, valid_var_name};
use super::{
    Activity, AgentKind, AppCore, Ask, AskKind, AwsMethod, Clock, ENV_SETUP_LOCKED, Effect, EnvSet,
    EnvVar, GridRect, Launch, PinTarget, PinnedItem, ProjectId, RecordId, SessionKind, SetId,
    SetRule, Space, SpaceId, WorkflowDefinition, WorkflowId, WorkingSet, grid,
};
use crate::ports::host::Liveness;

/// One command from the control port. Payloads are the core's own ids;
/// the app parses the wire's strings into them.
#[derive(Debug, Clone, PartialEq)]
pub enum ControlAction {
    AddProject {
        space: SpaceId,
        name: String,
        root: PathBuf,
    },
    RemoveProject(ProjectId),
    NewSession {
        project: ProjectId,
        name: String,
        kind: SessionKind,
        cwd: PathBuf,
        launch: Launch,
        /// An agent's first prompt, on its command line.
        prompt: Option<String>,
        notes: String,
        /// Variables for every spawn of the session (`SessionRecord::env`).
        env: BTreeMap<String, String>,
        /// Environment sets granted to the session.
        env_sets: Vec<String>,
    },
    /// A Claude Code session cloned from `source`'s whole transcript,
    /// launched with `prompt`.
    CloneSession {
        source: RecordId,
        name: String,
        prompt: String,
        notes: String,
        /// The clone's own environment sets, never the source's.
        env_sets: Vec<String>,
    },
    SendInput {
        id: RecordId,
        text: String,
    },
    /// `SendInput` only when `AppCore::ready_for_prompt` allows it,
    /// checked and typed in one step; otherwise refused with the reason.
    Prompt {
        id: RecordId,
        text: String,
    },
    Kill(RecordId),
    /// Resume an agent's conversation without opening a terminal; never
    /// a fresh launch.
    Resume(RecordId),
    Remove(RecordId),
    SetNotes {
        id: RecordId,
        text: String,
    },
    /// Why the session waits on the user, or `None` to clear it.
    SetWaiting {
        id: RecordId,
        reason: Option<String>,
    },
    /// The session's own question to the owner, or `None` to clear it.
    /// Taken only with the token its latest spawn was given.
    Ask {
        id: RecordId,
        token: String,
        message: Option<String>,
        /// `"confirm"`, `"choice"` or `"text"`; `None` for a plain ask.
        kind: Option<String>,
        /// A choice ask's options, in order.
        choices: Vec<String>,
    },
    /// Answer Claude's folder trust question in the pane with yes.
    TrustFolder(RecordId),
    /// The project's directory moved.
    SetProjectRoot {
        id: ProjectId,
        root: PathBuf,
    },
    MoveSession {
        id: RecordId,
        project: ProjectId,
    },
    RenameProject {
        id: ProjectId,
        name: String,
    },
    NewSpace {
        name: String,
    },
    NewSet {
        space: SpaceId,
        name: String,
    },
    /// The set's cards become exactly this list, or nothing changes.
    SyncSet {
        set: SetId,
        items: Vec<PinnedItem>,
    },
    /// Add or replace the definition of that name.
    InstallDefinition(WorkflowDefinition),
    StartWorkflow {
        source: RecordId,
        plan: PathBuf,
        definition: String,
        reviewer_cwd: Option<PathBuf>,
        reviewer_args: Vec<String>,
    },
    PauseWorkflow(WorkflowId),
    ContinueWorkflow(WorkflowId),
    /// The owner's objection: round `round` of the run carries `text`.
    ObjectWorkflow {
        run: WorkflowId,
        round: u32,
        text: String,
    },
    FinalizeWorkflow(WorkflowId),
    RemoveWorkflow(WorkflowId),
    /// Stop, start or restart the Dispatch runner the app runs.
    DispatchRunner(wire::RunnerVerb),
    /// Create the environment set when missing, replace each named
    /// variable, and replace `aws` when given.
    EnvSetUpsert {
        name: String,
        vars: Vec<EnvVar>,
        aws: Option<AwsMethod>,
    },
    /// Store a set's secret value, adding the variable when missing.
    EnvSecretStore {
        set: String,
        name: String,
        value: String,
    },
    /// Grant a set to a target, or take it back.
    EnvGrant {
        target: GrantTarget,
        set: String,
        remove: bool,
    },
}

/// Who an `env.grant` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantTarget {
    Project(ProjectId),
    Session(RecordId),
    /// The Dispatch runner's record (`Settings::dispatch_runner`), whose
    /// grants serve every gate that runs with `env`.
    Runner,
}

/// What one control command did: the records it made and the error it
/// hit, if any. Taken by the app to write the reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlOutcome {
    pub op: String,
    pub made: Vec<wire::Made>,
    pub error: Option<String>,
}

fn made(kind: wire::RecordKind, id: uuid::Uuid) -> wire::Made {
    wire::Made {
        kind,
        id: id.to_string(),
    }
}

impl AppCore {
    /// Apply one control command. Notices it would have posted become
    /// the outcome's error instead: the window stays quiet and the
    /// asker gets the reason.
    pub(super) fn control(&mut self, op: String, action: ControlAction, now: Clock, out: &mut Out) {
        self.quiet_op = Some(op.clone());
        let notices_before = self.notices.len();
        let creation = matches!(
            action,
            ControlAction::AddProject { .. }
                | ControlAction::NewSession { .. }
                | ControlAction::CloneSession { .. }
                | ControlAction::NewSpace { .. }
                | ControlAction::NewSet { .. }
                | ControlAction::StartWorkflow { .. }
        );
        let kind = control_kind(&action);
        let made = self.apply_control(action, now, out);
        self.quiet_op = None;
        let error = self
            .notices
            .drain(notices_before..)
            .filter(|n| n.is_error)
            .map(|n| n.text)
            .reduce(|a, b| format!("{a}; {b}"));
        if creation && error.is_none() && !made.is_empty() {
            out.push(Effect::LogOperation {
                op: op.clone(),
                kind,
                ids: made.iter().map(|m| m.id.clone()).collect(),
            });
        }
        self.control_outcomes
            .push(ControlOutcome { op, made, error });
    }

    /// The outcome of `op`, once; `None` if no such command ran.
    pub fn take_control_outcome(&mut self, op: &str) -> Option<ControlOutcome> {
        let pos = self.control_outcomes.iter().position(|o| o.op == op)?;
        Some(self.control_outcomes.remove(pos))
    }

    #[allow(clippy::too_many_lines)] // one arm per command
    fn apply_control(
        &mut self,
        action: ControlAction,
        now: Clock,
        out: &mut Out,
    ) -> Vec<wire::Made> {
        use wire::RecordKind as K;
        match action {
            ControlAction::AddProject { space, name, root } => {
                if space.is_global() {
                    self.error("the global workspace holds no projects");
                    return Vec::new();
                }
                if self.space(space).is_none() {
                    self.error("no such space");
                    return Vec::new();
                }
                let id = self.add_project(name, root, space, now, out);
                vec![made(K::Project, id.0)]
            }
            ControlAction::RemoveProject(id) => {
                self.remove_project(id, now, out);
                Vec::new()
            }
            ControlAction::NewSession {
                project,
                name,
                kind,
                cwd,
                launch,
                prompt,
                notes,
                env,
                env_sets,
            } => {
                if let Some(text) = self.host_unavailable("start", &name) {
                    self.error(text);
                    return Vec::new();
                }
                let Some(id) = self.add_record(project, name, kind, cwd, launch, now, out) else {
                    return Vec::new();
                };
                self.edit_session(id, out, |s| {
                    s.notes = notes;
                    s.env = env.into_iter().collect();
                    s.env_sets = env_sets;
                });
                if let Some(prompt) = prompt.filter(|_| matches!(kind, SessionKind::Agent(_))) {
                    self.first_prompts.push((id, prompt));
                }
                self.launch_fresh(id, now, out);
                vec![made(K::Session, id.0)]
            }
            ControlAction::CloneSession {
                source,
                name,
                prompt,
                notes,
                env_sets,
            } => {
                if let Some(text) = self.host_unavailable("start", &name) {
                    self.error(text);
                    return Vec::new();
                }
                self.clone_into(source, name, prompt, notes, env_sets, now, out)
                    .map_or_else(Vec::new, |id| vec![made(K::Session, id.0)])
            }
            ControlAction::SendInput { id, text } => {
                // Nothing reaches a pane that is gone, so no prompt
                // would consume the mark.
                if self.session(id).is_some_and(|s| s.asking.is_some())
                    && self.is_running(id)
                    && !self.relayed.contains(&id)
                {
                    self.relayed.push(id);
                }
                self.session_action(AppAction::SendInput { id, text }, now, out);
                Vec::new()
            }
            ControlAction::Prompt { id, text } => {
                if let Err(why) = self.ready_for_prompt(id, now.wall) {
                    self.error(why);
                    return Vec::new();
                }
                // The pane reads as between turns until Claude Code
                // reports the prompt, so the mark refuses a second one in
                // that window.
                self.mark_typed(id, now.wall, true);
                self.apply_control(ControlAction::SendInput { id, text }, now, out)
            }
            ControlAction::Kill(id) => {
                self.session_action(AppAction::KillSession(id), now, out);
                Vec::new()
            }
            ControlAction::Resume(id) => {
                self.control_resume(id, now, out);
                Vec::new()
            }
            ControlAction::Remove(id) => {
                self.session_action(AppAction::RemoveSession(id), now, out);
                Vec::new()
            }
            ControlAction::SetNotes { id, text } => {
                self.session_action(AppAction::SetSessionNotes(id, text), now, out);
                Vec::new()
            }
            ControlAction::SetWaiting { id, reason } => {
                if self.session(id).is_none() {
                    self.error("no such session");
                }
                self.edit_session(id, out, |s| s.waiting_on = reason);
                Vec::new()
            }
            ControlAction::Ask {
                id,
                token,
                message,
                kind,
                choices,
            } => {
                let ask = message.map(|text| (text, kind, choices));
                self.session_ask(id, &token, ask, now, out);
                Vec::new()
            }
            ControlAction::TrustFolder(id) => {
                self.trust_folder(id, out);
                Vec::new()
            }
            ControlAction::MoveSession { id, project } => {
                self.move_session(id, project, out);
                Vec::new()
            }
            ControlAction::RenameProject { id, name } => {
                if self.workspace(id).is_none() {
                    self.error("no such project");
                }
                self.edit_project(id, out, |p| p.name = name);
                Vec::new()
            }
            ControlAction::SetProjectRoot { id, root } => {
                if self.workspace(id).is_none() {
                    self.error("no such project");
                }
                self.edit_project(id, out, |p| p.root = root);
                Vec::new()
            }
            ControlAction::NewSpace { name } => {
                let name = name.trim().to_owned();
                if name.is_empty() {
                    self.error("a space needs a name");
                    return Vec::new();
                }
                let space = Space {
                    id: SpaceId::new(),
                    name,
                    op: self.quiet_op.clone(),
                };
                let id = space.id;
                self.update_views(out, |v| v.spaces.push(space));
                vec![made(K::Space, id.0)]
            }
            ControlAction::NewSet { space, name } => {
                // A set in the global space may hold cards from any space.
                if !space.is_global() && self.space(space).is_none() {
                    self.error("no such space");
                    return Vec::new();
                }
                let set = WorkingSet {
                    id: SetId::new(),
                    name,
                    items: Vec::new(),
                    space,
                    op: self.quiet_op.clone(),
                    rule: None,
                    dismissed: Vec::new(),
                    running_only: false,
                    card_scale: WorkingSet::default_card_scale(),
                };
                let id = set.id;
                self.update_views(out, |v| v.sets.push(set));
                vec![made(K::Set, id.0)]
            }
            ControlAction::SyncSet { set, items } => {
                self.sync_set(set, items, out);
                Vec::new()
            }
            ControlAction::InstallDefinition(def) => {
                if def.name.trim().is_empty() {
                    self.error("a definition needs a name");
                    return Vec::new();
                }
                self.update_settings(out, |s| {
                    s.workflows.retain(|d| d.name != def.name);
                    s.workflows.push(def);
                });
                Vec::new()
            }
            ControlAction::StartWorkflow {
                source,
                plan,
                definition,
                reviewer_cwd,
                reviewer_args,
            } => {
                let before: Vec<WorkflowId> = self.workflows().map(|r| r.id).collect();
                self.start_workflow(
                    source,
                    &plan,
                    &definition,
                    reviewer_cwd,
                    reviewer_args,
                    now,
                    out,
                );
                let Some(run) = self.workflows().find(|r| !before.contains(&r.id)).cloned() else {
                    return Vec::new();
                };
                vec![made(K::Run, run.id.0), made(K::Session, run.reviewer.0)]
            }
            ControlAction::PauseWorkflow(id) => {
                self.workflow_action(AppAction::PauseWorkflow(id), now, out);
                Vec::new()
            }
            ControlAction::ContinueWorkflow(id) => {
                self.workflow_action(AppAction::ContinueWorkflow(id), now, out);
                Vec::new()
            }
            ControlAction::ObjectWorkflow { run, round, text } => {
                self.workflow_action(AppAction::ObjectWorkflow { run, round, text }, now, out);
                Vec::new()
            }
            ControlAction::FinalizeWorkflow(id) => {
                self.workflow_action(AppAction::FinalizeWorkflow(id), now, out);
                Vec::new()
            }
            ControlAction::RemoveWorkflow(id) => {
                self.workflow_action(AppAction::RemoveWorkflow(id), now, out);
                Vec::new()
            }
            ControlAction::DispatchRunner(verb) => {
                self.control_runner(verb, now, out);
                Vec::new()
            }
            ControlAction::EnvSetUpsert { name, vars, aws } => {
                if self.env_setup_open(now) {
                    self.env_set_upsert(&name, vars, aws, out);
                }
                Vec::new()
            }
            ControlAction::EnvSecretStore { set, name, value } => {
                if self.env_setup_open(now) {
                    self.env_secret_store(&set, name, value, out);
                }
                Vec::new()
            }
            ControlAction::EnvGrant {
                target,
                set,
                remove,
            } => {
                if self.env_setup_open(now) {
                    self.env_grant(target, &set, remove, out);
                }
                Vec::new()
            }
        }
    }

    /// Whether `switchboard-env`'s setup commands are accepted now; a
    /// refusal is the command's error. Every later refusal is an error
    /// too, so nothing is half-applied.
    fn env_setup_open(&mut self, now: Clock) -> bool {
        let open = self.env_setup_until.is_some_and(|t| now.mono < t);
        if !open {
            self.error(ENV_SETUP_LOCKED);
        }
        open
    }

    fn env_set_upsert(
        &mut self,
        name: &str,
        vars: Vec<EnvVar>,
        aws: Option<AwsMethod>,
        out: &mut Out,
    ) {
        if !valid_set_name(name) {
            return self.error(format!("{name:?} is not a set name ([a-z0-9][a-z0-9-]*)"));
        }
        if let Some(bad) = vars.iter().find(|v| !valid_var_name(&v.name)) {
            return self.error(format!(
                "{:?} is not a variable name ([A-Z_][A-Z0-9_]*)",
                bad.name
            ));
        }
        if vars.iter().any(|v| v.secret && !v.value.is_empty()) {
            return self.error("a secret's value goes through `env.secret.store`");
        }
        self.update_settings(out, |s| {
            let set = set_entry(&mut s.env_sets, name);
            for var in vars {
                put_var(set, var);
            }
            if aws.is_some() {
                set.aws = aws;
            }
        });
    }

    fn env_secret_store(&mut self, set: &str, name: String, value: String, out: &mut Out) {
        if !self.settings.env_sets.iter().any(|s| s.name == set) {
            return self.error(format!("no environment set named {set}"));
        }
        if !valid_var_name(&name) {
            return self.error(format!(
                "{name:?} is not a variable name ([A-Z_][A-Z0-9_]*)"
            ));
        }
        if value.is_empty() {
            return self.error("a secret needs a value");
        }
        let account = SecretScope::Set(set.to_owned()).account(&name);
        self.update_settings(out, |s| {
            put_var(
                set_entry(&mut s.env_sets, set),
                EnvVar {
                    name,
                    value: String::new(),
                    secret: true,
                },
            );
        });
        out.push(Effect::StoreSecret { account, value });
    }

    fn env_grant(&mut self, target: GrantTarget, set: &str, remove: bool, out: &mut Out) {
        if !valid_set_name(set) {
            return self.error(format!("{set:?} is not a set name ([a-z0-9][a-z0-9-]*)"));
        }
        // A revoke may name a set that is gone; a grant may not.
        if !remove && !self.settings.env_sets.iter().any(|s| s.name == set) {
            return self.error(format!("no environment set named {set}"));
        }
        let edit = |sets: &mut Vec<String>| {
            if remove {
                sets.retain(|s| s != set);
            } else if !sets.iter().any(|s| s == set) {
                sets.push(set.to_owned());
            }
        };
        let session = match target {
            GrantTarget::Project(id) => {
                if self.workspace(id).is_none() {
                    return self.error("no such project");
                }
                return self.edit_project(id, out, |p| edit(&mut p.env_sets));
            }
            GrantTarget::Session(id) => id,
            GrantTarget::Runner => match self.settings.dispatch_runner {
                Some(id) => id,
                None => return self.error("there is no Dispatch runner to grant to"),
            },
        };
        if self.session(session).is_none() {
            return self.error("no such session");
        }
        self.edit_session(session, out, |s| edit(&mut s.env_sets));
    }

    /// The runner's buttons, asked for over the port. A stop with
    /// nothing to stop succeeds, since that is already the state asked
    /// for; a restart kills without turning autostart off, so the record
    /// says the runner comes back however the app ends.
    fn control_runner(&mut self, verb: wire::RunnerVerb, now: Clock, out: &mut Out) {
        use super::dispatch::{RUNNER_STOP_OUTSIDE, RunnerStanding};
        use wire::RunnerVerb;
        let standing = self.runner_standing();
        match (verb, standing) {
            (RunnerVerb::Stop, RunnerStanding::Outside) => self.error(RUNNER_STOP_OUTSIDE),
            (
                RunnerVerb::Stop,
                RunnerStanding::Up { .. } | RunnerStanding::Starting | RunnerStanding::StartQueued,
            ) => self.runner_stop(now, out),
            (RunnerVerb::Stop, RunnerStanding::Stopping) => {}
            (RunnerVerb::Stop, RunnerStanding::Stopped | RunnerStanding::Gone) => {
                if let Some(id) = self
                    .runner()
                    .filter(|id| self.session(*id).is_some_and(|s| s.autostart))
                {
                    self.edit_session(id, out, |s| s.autostart = false);
                }
            }
            (
                RunnerVerb::Restart,
                RunnerStanding::Up { .. } | RunnerStanding::Starting | RunnerStanding::StartQueued,
            ) => {
                self.runner_kill(now, out);
                self.runner_start(now, out);
            }
            (RunnerVerb::Start | RunnerVerb::Restart, _) => self.runner_start(now, out),
        }
    }

    /// Replace a set's cards wholesale. Every target must be in the set's
    /// space and no two may overlap; otherwise nothing changes and the
    /// reason is an error.
    fn sync_set(&mut self, id: SetId, items: Vec<PinnedItem>, out: &mut Out) {
        let Some(set) = self.working_set(id) else {
            self.error("no such working set");
            return;
        };
        if set.rule.is_some() {
            self.error("the set is chosen by a rule");
            return;
        }
        let space = set.space;
        let items: Vec<PinnedItem> = items
            .into_iter()
            .map(|i| PinnedItem {
                target: i.target,
                rect: grid::clamp(i.rect),
            })
            .collect();
        for (n, item) in items.iter().enumerate() {
            if !self.target_in(&item.target, space) {
                self.error(format!("card {n} is not in the set's space"));
                return;
            }
            if items[..n].iter().any(|other| other.target == item.target) {
                self.error(format!("card {n} appears twice"));
                return;
            }
            if items[..n]
                .iter()
                .any(|other| other.rect.overlaps(item.rect))
            {
                self.error(format!("card {n} overlaps an earlier card"));
                return;
            }
        }
        self.update_set(out, id, |s| s.items = items);
    }

    /// Records that the session's input box may hold text now: a
    /// `session.prompt` being submitted when `prompt`, else the owner's
    /// keys.
    pub(super) fn mark_typed(&mut self, id: RecordId, at: SystemTime, prompt: bool) {
        self.typed.retain(|(r, ..)| *r != id);
        self.typed.push((id, at, prompt));
    }

    /// Whether a line typed into the session now would arrive as its
    /// next prompt and nothing else, or why not. The pane's own verdict
    /// is `between_turns`, the rule an owner's queued answer is sent by;
    /// in front of it go what that rule cannot see: the session's kind
    /// (only Claude Code reports turns), the trust question, keys the
    /// owner typed, and the owner's own answer, which goes first.
    pub fn ready_for_prompt(&self, id: RecordId, now: SystemTime) -> Result<(), String> {
        let Some(s) = self.session(id) else {
            return Err("no such session".to_owned());
        };
        if s.kind != SessionKind::Agent(AgentKind::ClaudeCode) {
            return Err("not a Claude Code session".to_owned());
        }
        if self.at_trust_prompt(id) {
            return Err("at a prompt (trust)".to_owned());
        }
        // A clock that went backwards reads as just typed.
        let typing = self.typed.iter().find(|(r, at, _)| {
            *r == id && now.duration_since(*at).map_or(true, |d| d < TYPED_HOLD)
        });
        if let Some((.., prompt)) = typing {
            return Err(if *prompt {
                "a prompt is being submitted"
            } else {
                "the owner is typing"
            }
            .to_owned());
        }
        if s.asking.as_ref().is_some_and(|a| a.answer.is_some()) {
            return Err("an answer waits".to_owned());
        }
        if self.between_turns(id) {
            return Ok(());
        }
        if !self.is_running(id) {
            return Err("not running".to_owned());
        }
        Err(match (s.activity, &s.activity_reason) {
            (Activity::WaitingOnYou, Some(why)) => format!("at a prompt ({why})"),
            (Activity::WaitingOnYou, None) => "at a prompt".to_owned(),
            (Activity::Working, _) => "busy".to_owned(),
            (Activity::Unknown, _) => "no turn reported yet".to_owned(),
            (Activity::Idle | Activity::Ended, _) => "ended".to_owned(),
        })
    }

    // --- read models, in the wire's shapes

    /// One session as the port reports it; `None` for an unknown id.
    #[must_use]
    pub fn session_view(&self, id: RecordId, now: SystemTime) -> Option<wire::SessionView> {
        let s = self.session(id)?;
        let status = self.host_status(id);
        let liveness = match status.map(|h| &h.liveness) {
            Some(Liveness::Running { .. }) => wire::Liveness::Running,
            Some(Liveness::Exited { code }) => wire::Liveness::Exited { code: *code },
            Some(Liveness::Missing) | None => wire::Liveness::Missing,
        };
        let quiet_secs = match liveness {
            wire::Liveness::Running => status
                .and_then(|h| h.last_activity)
                .and_then(|t| now.duration_since(t).ok())
                .map(|d| d.as_secs()),
            _ => None,
        };
        let card = self.card_state(id);
        Some(wire::SessionView {
            id: id.0.to_string(),
            project: s.project.0.to_string(),
            name: s.name.clone(),
            kind: match s.kind {
                SessionKind::Agent(AgentKind::ClaudeCode) => wire::SessionKind::Claude,
                SessionKind::Agent(AgentKind::Codex) => wire::SessionKind::Codex,
                SessionKind::Shell => wire::SessionKind::Shell,
                SessionKind::Command => wire::SessionKind::Command,
                SessionKind::Service => wire::SessionKind::Service,
            },
            cwd: s.cwd.clone(),
            notes: s.notes.clone(),
            liveness,
            card: card.label(),
            last_exit: s.last_exit,
            last_stop_at_ms: s.last_stop_at.and_then(epoch_ms),
            quiet_secs,
            waiting: card == super::CardState::WaitingOnYou,
            waiting_reason: self.waiting_reason(id),
            trust_question: self.at_trust_prompt(id),
            prompt_refusal: self.ready_for_prompt(id, now).err(),
            resume_id: s.resume.as_ref().map(super::ResumeHandle::provider_id),
            op: s.op.clone(),
        })
    }

    #[must_use]
    pub fn project_views(&self, space: Option<SpaceId>) -> Vec<wire::ProjectView> {
        self.workspaces
            .iter()
            .map(|w| &w.project)
            .filter(|p| space.is_none_or(|s| p.space == s))
            .map(|p| wire::ProjectView {
                id: p.id.0.to_string(),
                name: p.name.clone(),
                root: p.root.clone(),
                space: p.space.0.to_string(),
                op: p.op.clone(),
            })
            .collect()
    }

    /// The spaces, then the global space marked as a view, so every set
    /// `set.new` can make is reachable by walking this list.
    #[must_use]
    pub fn space_views(&self) -> Vec<wire::SpaceView> {
        self.views
            .spaces
            .iter()
            .map(|s| wire::SpaceView {
                id: s.id.0.to_string(),
                name: s.name.clone(),
                op: s.op.clone(),
                view: false,
            })
            .chain(std::iter::once(wire::SpaceView {
                id: SpaceId::GLOBAL.0.to_string(),
                name: SpaceId::GLOBAL_NAME.to_owned(),
                op: None,
                view: true,
            }))
            .collect()
    }

    #[must_use]
    pub fn set_views(&self, space: SpaceId) -> Vec<wire::SetView> {
        self.views
            .sets
            .iter()
            .filter(|s| s.space == space)
            .map(|s| wire::SetView {
                id: s.id.0.to_string(),
                name: s.name.clone(),
                space: s.space.0.to_string(),
                items: self
                    .set_cards(s, RULE_COLUMNS)
                    .iter()
                    .map(pin_view)
                    .collect(),
                op: s.op.clone(),
                rule: s.rule.map(|r| match r {
                    SetRule::Recent { hours } => wire::SetRule::Recent { hours },
                }),
            })
            .collect()
    }

    #[must_use]
    pub fn run_view(&self, id: WorkflowId) -> Option<wire::RunView> {
        let r = self.workflow(id)?;
        Some(wire::RunView {
            id: r.id.0.to_string(),
            project: r.project.0.to_string(),
            definition: r.definition.clone(),
            state: match &r.state {
                super::RunState::Starting => wire::RunState::Starting,
                super::RunState::AwaitingFeedback => wire::RunState::AwaitingFeedback,
                super::RunState::AwaitingResponse => wire::RunState::AwaitingResponse,
                super::RunState::Converged => wire::RunState::Converged,
                super::RunState::AtCap => wire::RunState::AtCap,
                super::RunState::Paused(reason) => wire::RunState::Paused {
                    reason: reason.clone(),
                    failed: false,
                },
                super::RunState::Failed(reason) => wire::RunState::Paused {
                    reason: reason.clone(),
                    failed: true,
                },
                super::RunState::Finalized => wire::RunState::Finalized,
                super::RunState::HandedOff => wire::RunState::HandedOff,
            },
            round: r.rounds.last().map_or(0, |round| round.n),
            cap: r.cap,
            source: r.source.0.to_string(),
            plan: r.plan.clone(),
            reviewer: r.reviewer.0.to_string(),
            planner: r.planner.map(|p| p.0.to_string()),
            op: r.op.clone(),
        })
    }

    /// Every session whose card reads as waiting on the user.
    #[must_use]
    pub fn waiting_views(&self, now: SystemTime) -> Vec<wire::SessionView> {
        self.workspaces
            .iter()
            .flat_map(|w| &w.sessions)
            .filter(|s| self.card_state(s.id) == super::CardState::WaitingOnYou)
            .filter_map(|s| self.session_view(s.id, now))
            .collect()
    }

    /// Every record still present that `op` made, with its state.
    #[must_use]
    pub fn records_with_op(&self, op: &str, now: SystemTime) -> Vec<wire::Found> {
        let has = |o: &Option<String>| o.as_deref() == Some(op);
        let mut found = Vec::new();
        for s in &self.views.spaces {
            if has(&s.op) {
                found.push(found_record(wire::RecordKind::Space, s.id.0));
            }
        }
        for s in &self.views.sets {
            if has(&s.op) {
                found.push(found_record(wire::RecordKind::Set, s.id.0));
            }
        }
        for w in &self.workspaces {
            if has(&w.project.op) {
                found.push(found_record(wire::RecordKind::Project, w.project.id.0));
            }
            for s in &w.sessions {
                if has(&s.op) {
                    let mut f = found_record(wire::RecordKind::Session, s.id.0);
                    f.session = self.session_view(s.id, now);
                    found.push(f);
                }
            }
            for r in &w.workflows {
                if has(&r.op) {
                    let mut f = found_record(wire::RecordKind::Run, r.id.0);
                    f.run = self.run_view(r.id);
                    found.push(f);
                }
            }
        }
        found
    }

    /// Records whose launch was written down but never reported back: a
    /// restart happened between the two.
    #[must_use]
    pub fn interrupted_ops(&self) -> Vec<String> {
        self.workspaces
            .iter()
            .flat_map(|w| &w.sessions)
            .filter(|s| s.pending_launch && !self.in_flight.iter().any(|f| f.id == s.id))
            .filter_map(|s| s.op.clone())
            .collect()
    }
}

/// The set called `name`, made empty when there is none.
fn set_entry<'a>(sets: &'a mut Vec<EnvSet>, name: &str) -> &'a mut EnvSet {
    let pos = sets.iter().position(|s| s.name == name).unwrap_or_else(|| {
        sets.push(EnvSet {
            name: name.to_owned(),
            ..EnvSet::default()
        });
        sets.len() - 1
    });
    &mut sets[pos]
}

/// Replace or add one variable. A secret keeps no value here, so turning
/// a plain variable secret blanks what `settings.json` held for it.
fn put_var(set: &mut EnvSet, mut var: EnvVar) {
    if var.secret {
        var.value.clear();
    }
    match set.vars.iter_mut().find(|v| v.name == var.name) {
        Some(existing) => *existing = var,
        None => set.vars.push(var),
    }
}

fn found_record(kind: wire::RecordKind, id: uuid::Uuid) -> wire::Found {
    wire::Found {
        kind,
        id: id.to_string(),
        removed: false,
        session: None,
        run: None,
    }
}

fn pin_view(item: &PinnedItem) -> wire::Pin {
    wire::Pin {
        target: match &item.target {
            PinTarget::Session(id) => wire::PinTarget::Session {
                session: id.0.to_string(),
            },
            PinTarget::File(project, path) => wire::PinTarget::File {
                project: project.0.to_string(),
                path: path.clone(),
            },
        },
        rect: rect_view(item.rect),
    }
}

fn rect_view(r: GridRect) -> wire::Rect {
    wire::Rect {
        x: r.x,
        y: r.y,
        w: r.w,
        h: r.h,
    }
}

impl AppCore {
    /// Mark or clear a session's own question, for the holder of its
    /// launch token only, as `env.resolve` checks it. Never touches
    /// `waiting_on`, which is Dispatch's. Only a Claude Code agent may
    /// ask for an answer, since only its `Stop` hook says when to send it.
    /// `ask` is the wire's message, kind and choices; `None` clears.
    fn session_ask(
        &mut self,
        id: RecordId,
        token: &str,
        ask: Option<(String, Option<String>, Vec<String>)>,
        now: Clock,
        out: &mut Out,
    ) {
        let Some(record) = self.session(id) else {
            self.error("no such session");
            return;
        };
        let Some(hash) = &record.token_hash else {
            self.error("this session was launched before tokens; restart it");
            return;
        };
        if token.is_empty() || super::token_hash(token) != *hash {
            self.error("token does not match");
            return;
        }
        let claude = record.kind == SessionKind::Agent(AgentKind::ClaudeCode);
        let asking = match ask {
            None => None,
            Some((text, kind, choices)) => {
                let Some(message) = ask_message(&text) else {
                    self.error("an ask needs a message");
                    return;
                };
                let kind = match parse_ask_kind(kind.as_deref(), &choices) {
                    Ok(kind) => kind,
                    Err(why) => {
                        self.error(why);
                        return;
                    }
                };
                if kind != AskKind::Note && !claude {
                    self.error(
                        "only a Claude Code session can ask for an answer; ask without --confirm, --choice or --text",
                    );
                    return;
                }
                Some(Ask {
                    message,
                    at: now.wall,
                    kind,
                    answer: None,
                })
            }
        };
        self.edit_session(id, out, |s| s.asking = asking);
    }
}

/// An ask's message as the record keeps it: the first line, trimmed,
/// cut at `ASK_MAX_CHARS` characters with "…" after. `None` when nothing
/// is left, so a blank ask is refused rather than read as a clear.
pub(crate) fn ask_message(text: &str) -> Option<String> {
    cut_line(text, super::ASK_MAX_CHARS)
}

/// An ask's kind from the wire's name and options. Each option is cut
/// like a message, to `ASK_CHOICE_MAX_CHARS`; a blank or repeated one
/// is refused, as is a choice of fewer than two or more than
/// `ASK_CHOICES_MAX`.
pub(crate) fn parse_ask_kind(kind: Option<&str>, choices: &[String]) -> Result<AskKind, String> {
    if kind != Some("choice") && !choices.is_empty() {
        return Err("choices go only with a choice ask".into());
    }
    match kind {
        None => Ok(AskKind::Note),
        Some("confirm") => Ok(AskKind::Confirm),
        Some("text") => Ok(AskKind::Text),
        Some("choice") => {
            if choices.len() < 2 {
                return Err("a choice ask needs at least two choices".into());
            }
            if choices.len() > super::ASK_CHOICES_MAX {
                return Err(format!(
                    "a choice ask takes at most {} choices",
                    super::ASK_CHOICES_MAX
                ));
            }
            let mut kept: Vec<String> = Vec::with_capacity(choices.len());
            for choice in choices {
                let Some(option) = cut_line(choice, super::ASK_CHOICE_MAX_CHARS) else {
                    return Err("a choice may not be blank".into());
                };
                if kept.contains(&option) {
                    return Err("each choice must differ".into());
                }
                kept.push(option);
            }
            Ok(AskKind::Choice(kept))
        }
        Some(_) => Err("unknown ask kind; use confirm, choice or text".into()),
    }
}

/// The first line of `text`, trimmed, cut at `max` characters with "…"
/// after; `None` when nothing is left.
fn cut_line(text: &str, max: usize) -> Option<String> {
    let line = text.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        return None;
    }
    match line.char_indices().nth(max) {
        Some((cut, _)) => Some(format!("{}…", line[..cut].trim_end())),
        None => Some(line.to_owned()),
    }
}

fn epoch_ms(t: SystemTime) -> Option<u64> {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The command's name for the operations log.
fn control_kind(action: &ControlAction) -> String {
    match action {
        ControlAction::AddProject { .. } => "project.add",
        ControlAction::RemoveProject(_) => "project.remove",
        ControlAction::NewSession { .. } => "session.new",
        ControlAction::CloneSession { .. } => "session.clone",
        ControlAction::SendInput { .. } => "session.send",
        ControlAction::Prompt { .. } => "session.prompt",
        ControlAction::Kill(_) => "session.kill",
        ControlAction::Resume(_) => "session.resume",
        ControlAction::Remove(_) => "session.remove",
        ControlAction::SetNotes { .. } => "session.notes",
        ControlAction::SetWaiting { .. } => "session.waiting",
        ControlAction::Ask { .. } => "session.ask",
        ControlAction::TrustFolder(_) => "session.trust",
        ControlAction::MoveSession { .. } => "session.move",
        ControlAction::RenameProject { .. } => "project.rename",
        ControlAction::SetProjectRoot { .. } => "project.root",
        ControlAction::NewSpace { .. } => "space.new",
        ControlAction::NewSet { .. } => "set.new",
        ControlAction::SyncSet { .. } => "set.sync",
        ControlAction::InstallDefinition(_) => "workflow.definitions.install",
        ControlAction::StartWorkflow { .. } => "workflow.start",
        ControlAction::PauseWorkflow(_) => "workflow.pause",
        ControlAction::ContinueWorkflow(_) => "workflow.continue",
        ControlAction::ObjectWorkflow { .. } => "workflow.object",
        ControlAction::FinalizeWorkflow(_) => "workflow.finalize",
        ControlAction::RemoveWorkflow(_) => "workflow.remove",
        ControlAction::DispatchRunner(verb) => {
            return format!("dispatch.runner.{}", verb.word());
        }
        ControlAction::EnvSetUpsert { .. } => "env.set.upsert",
        ControlAction::EnvSecretStore { .. } => "env.secret.store",
        ControlAction::EnvGrant { .. } => "env.grant",
    }
    .to_owned()
}

// --- from the wire

fn parse_id(kind: &str, text: &str) -> Result<uuid::Uuid, String> {
    uuid::Uuid::parse_str(text).map_err(|_| format!("{kind} id {text:?} is not a uuid"))
}

fn session_kind(kind: wire::SessionKind) -> SessionKind {
    match kind {
        wire::SessionKind::Claude => SessionKind::Agent(AgentKind::ClaudeCode),
        wire::SessionKind::Codex => SessionKind::Agent(AgentKind::Codex),
        wire::SessionKind::Shell => SessionKind::Shell,
        wire::SessionKind::Command => SessionKind::Command,
        wire::SessionKind::Service => SessionKind::Service,
    }
}

fn launch(launch: wire::Launch) -> Launch {
    match launch {
        wire::Launch::Shell => Launch::Shell,
        wire::Launch::Argv(argv) => Launch::Argv(argv),
        wire::Launch::Command { command, shell } => Launch::Command { command, shell },
    }
}

fn pin(pin: wire::Pin) -> Result<PinnedItem, String> {
    let target = match pin.target {
        wire::PinTarget::Session { session } => {
            PinTarget::Session(RecordId(parse_id("session", &session)?))
        }
        wire::PinTarget::File { project, path } => {
            PinTarget::File(ProjectId(parse_id("project", &project)?), path)
        }
    };
    Ok(PinnedItem {
        target,
        rect: GridRect {
            x: pin.rect.x,
            y: pin.rect.y,
            w: pin.rect.w,
            h: pin.rect.h,
        },
    })
}

fn aws_method(method: wire::AwsMethod) -> AwsMethod {
    match method {
        wire::AwsMethod::Vault { profile } => AwsMethod::Vault { profile },
        wire::AwsMethod::Sso { profile } => AwsMethod::Sso { profile },
        wire::AwsMethod::Static => AwsMethod::Static,
    }
}

/// The core's AWS method as the wire carries it.
#[must_use]
pub fn aws_view(method: &AwsMethod) -> wire::AwsMethod {
    match method {
        AwsMethod::Vault { profile } => wire::AwsMethod::Vault {
            profile: profile.clone(),
        },
        AwsMethod::Sso { profile } => wire::AwsMethod::Sso {
            profile: profile.clone(),
        },
        AwsMethod::Static => wire::AwsMethod::Static,
    }
}

fn definition(d: wire::Definition) -> WorkflowDefinition {
    WorkflowDefinition {
        name: d.name,
        reviewer: match d.reviewer {
            wire::AgentKind::Claude => AgentKind::ClaudeCode,
            wire::AgentKind::Codex => AgentKind::Codex,
        },
        review_first: d.review_first,
        review_round: d.review_round,
        respond: d.respond,
        respond_to_user: d.respond_to_user,
        handoff: d.handoff,
        no_feedback: d.no_feedback,
        cap: d.cap,
    }
}

impl TryFrom<wire::Body> for ControlAction {
    type Error = String;

    /// A command's payload with its ids parsed. A query is an error: it
    /// is answered from the read models, never dispatched.
    #[allow(clippy::too_many_lines)] // one arm per command
    fn try_from(body: wire::Body) -> Result<Self, String> {
        let session = |s: &str| parse_id("session", s).map(RecordId);
        let project = |s: &str| parse_id("project", s).map(ProjectId);
        let space = |s: &str| parse_id("space", s).map(SpaceId);
        let run = |s: &str| parse_id("run", s).map(WorkflowId);
        Ok(match body {
            wire::Body::ProjectAdd {
                space: sp,
                name,
                root,
            } => Self::AddProject {
                space: space(&sp)?,
                name,
                root,
            },
            wire::Body::ProjectRemove { project: p } => Self::RemoveProject(project(&p)?),
            wire::Body::SessionNew {
                project: p,
                name,
                session_kind: k,
                cwd,
                launch: l,
                prompt,
                notes,
                env,
                env_sets,
            } => Self::NewSession {
                project: project(&p)?,
                name,
                kind: session_kind(k),
                cwd,
                launch: launch(l),
                prompt,
                notes,
                env,
                env_sets,
            },
            wire::Body::SessionClone {
                source: s,
                name,
                prompt,
                notes,
                env_sets,
            } => Self::CloneSession {
                source: session(&s)?,
                name,
                prompt,
                notes,
                env_sets,
            },
            wire::Body::SessionSend { session: s, text } => Self::SendInput {
                id: session(&s)?,
                text,
            },
            wire::Body::SessionPrompt { session: s, text } => Self::Prompt {
                id: session(&s)?,
                text,
            },
            wire::Body::SessionKill { session: s } => Self::Kill(session(&s)?),
            wire::Body::SessionResume { session: s } => Self::Resume(session(&s)?),
            wire::Body::SessionRemove { session: s } => Self::Remove(session(&s)?),
            wire::Body::SessionNotes { session: s, text } => Self::SetNotes {
                id: session(&s)?,
                text,
            },
            wire::Body::SessionWaiting {
                session: s,
                on,
                reason,
            } => Self::SetWaiting {
                id: session(&s)?,
                reason: on.then_some(reason),
            },
            wire::Body::SessionAsk {
                session: s,
                token,
                message,
                ask_kind,
                choices,
            } => Self::Ask {
                id: session(&s)?,
                token,
                message,
                kind: ask_kind,
                choices,
            },
            wire::Body::SessionTrust { session: s } => Self::TrustFolder(session(&s)?),
            wire::Body::SessionMove {
                session: s,
                project: p,
            } => Self::MoveSession {
                id: session(&s)?,
                project: project(&p)?,
            },
            wire::Body::ProjectRename { project: p, name } => Self::RenameProject {
                id: project(&p)?,
                name,
            },
            wire::Body::ProjectRoot { project: p, root } => Self::SetProjectRoot {
                id: project(&p)?,
                root,
            },
            wire::Body::SpaceNew { name } => Self::NewSpace { name },
            wire::Body::SetNew { space: sp, name } => Self::NewSet {
                space: space(&sp)?,
                name,
            },
            wire::Body::SetSync { set, items } => Self::SyncSet {
                set: SetId(parse_id("set", &set)?),
                items: items.into_iter().map(pin).collect::<Result<_, _>>()?,
            },
            wire::Body::DefinitionInstall { definition: d } => {
                Self::InstallDefinition(definition(d))
            }
            wire::Body::WorkflowStart {
                source,
                plan,
                definition,
                reviewer_cwd,
                reviewer_args,
            } => Self::StartWorkflow {
                source: session(&source)?,
                plan,
                definition,
                reviewer_cwd,
                reviewer_args,
            },
            wire::Body::WorkflowPause { run: r } => Self::PauseWorkflow(run(&r)?),
            wire::Body::WorkflowContinue { run: r } => Self::ContinueWorkflow(run(&r)?),
            wire::Body::WorkflowObject {
                run: r,
                round,
                text,
            } => Self::ObjectWorkflow {
                run: run(&r)?,
                round,
                text,
            },
            wire::Body::WorkflowFinalize { run: r } => Self::FinalizeWorkflow(run(&r)?),
            wire::Body::WorkflowRemove { run: r } => Self::RemoveWorkflow(run(&r)?),
            wire::Body::DispatchRunner { action } => Self::DispatchRunner(action),
            wire::Body::EnvSetUpsert { name, vars, aws } => Self::EnvSetUpsert {
                name,
                vars: vars
                    .into_iter()
                    .map(|v| EnvVar {
                        name: v.name,
                        value: v.value,
                        secret: v.secret,
                    })
                    .collect(),
                aws: aws.map(aws_method),
            },
            wire::Body::EnvSecretStore { set, name, value } => {
                Self::EnvSecretStore { set, name, value }
            }
            wire::Body::EnvGrant {
                project: p,
                session: s,
                runner,
                set,
                remove,
            } => Self::EnvGrant {
                target: match (p, s, runner) {
                    (Some(p), None, false) => GrantTarget::Project(project(&p)?),
                    (None, Some(s), false) => GrantTarget::Session(session(&s)?),
                    (None, None, true) => GrantTarget::Runner,
                    _ => {
                        return Err(
                            "a grant names exactly one of a project, a session or the runner"
                                .into(),
                        );
                    }
                },
                set,
                remove,
            },
            other => return Err(format!("{} is a query, not a command", other.kind())),
        })
    }
}
