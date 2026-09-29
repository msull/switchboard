//! Orchestration: owns [`AppCore`] and the adapters. Maps effects to
//! adapter calls and feeds results back as actions; polls the host and
//! the event log on a timer; keeps the UI's captions fresh. Rendering
//! lives in [`crate::ui`].

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use switchboard_control as wire;

use crate::adapters::control::{ControlSocket, Incoming};
use crate::adapters::hooks::WakeSocket;
use crate::core::{
    AgentKind, AppAction, AppCore, Clock, ControlAction, ControlOutcome, Effect, ProjectId,
    RecordId, Resolved, ResumeHandle, SessionKind, SpaceId, View, WorkflowId,
};
use crate::ports::agent::AgentLauncher;
use crate::ports::artifacts::ArtifactFinder;
use crate::ports::control::{OpLine, Operations};
use crate::ports::controller::Controller;
use crate::ports::dispatch::DispatchPort;
use crate::ports::events::EventSource;
use crate::ports::host::{HostId, Liveness, ProcessHost};
use crate::ports::opener::Opener;
use crate::ports::project_config::ProjectConfigReader;
use crate::ports::round_files::RoundFiles;
use crate::ports::secrets::SecretStore;
use crate::ports::store::{Store, StoreError};
use crate::ports::transcript::TranscriptReader;
use crate::ui::UiState;

/// How often the host is listed and the event log read.
const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// How often card captions and the session snapshot are refreshed.
/// How often definition files are checked for a change.
const CONFIG_INTERVAL: Duration = Duration::from_secs(5);
/// How often Dispatch is asked for its status.
const DISPATCH_INTERVAL: Duration = Duration::from_secs(2);
const CAPTION_INTERVAL: Duration = Duration::from_secs(2);
/// How long a Codex id discovery keeps looking. Codex writes its rollout
/// file on the first prompt, not at launch, so this is generous; a
/// discovery also ends as soon as the pane is gone.
const DISCOVERY_TIMEOUT: Duration = Duration::from_hours(24);

pub struct Services {
    pub store: Box<dyn Store>,
    pub host: Box<dyn ProcessHost>,
    pub events: Box<dyn EventSource>,
    pub agents: Box<dyn AgentLauncher>,
    pub opener: Box<dyn Opener>,
    pub transcripts: Box<dyn TranscriptReader>,
    pub secrets: Box<dyn SecretStore>,
    pub project_config: Box<dyn ProjectConfigReader>,
    pub round_files: Box<dyn RoundFiles>,
    pub artifacts: Box<dyn ArtifactFinder>,
    /// The hand controller (a nunchuk over serial); a fake in tests.
    pub controller: Box<dyn Controller>,
    /// The control port's operations log.
    pub operations: Box<dyn Operations>,
    /// Dispatch's port: tickets as views, decisions answered.
    pub dispatch: Box<dyn DispatchPort>,
    /// The hook helper's wake-up socket; `None` in tests.
    pub wake: Option<WakeSocket>,
}

/// The environment sessions of `pid` get: global variables, the
/// project's `.env` files when it opted in, then its own variables;
/// secrets read from the store. Also what `.env.example` asks for.
#[must_use]
pub fn resolve_project_env(core: &AppCore, services: &Services, pid: ProjectId) -> Resolved {
    let Some(project) = core.workspace(pid).map(|w| &w.project) else {
        return Resolved::default();
    };
    let mut dotenv = Vec::new();
    if project.env.load_dotenv {
        for file in project.env.files() {
            match std::fs::read_to_string(project.root.join(&file)) {
                Ok(text) => dotenv.push((file, crate::adapters::dotenv::parse(&text))),
                Err(e) => log::warn!("{file} in {}: {e}", project.root.display()),
            }
        }
    }
    let example = std::fs::read_to_string(project.root.join(".env.example"))
        .map(|t| crate::adapters::dotenv::names(&t))
        .unwrap_or_default();
    let lookup = |account: &str| services.secrets.get(account).ok().flatten();
    crate::core::env::resolve(
        &core.settings().env,
        pid,
        &project.env,
        &dotenv,
        &example,
        &lookup,
    )
}

/// A Codex launch waiting for its rollout file.
struct Discovery {
    id: RecordId,
    kind: AgentKind,
    cwd: PathBuf,
    since: SystemTime,
    deadline: Instant,
}

pub struct SwitchboardApp {
    core: AppCore,
    services: Services,
    started: Instant,
    last_poll: Option<Instant>,
    /// When Dispatch was last asked for its status.
    last_dispatch: Option<Instant>,
    /// The waiting count last put on the Dock badge and sent to the
    /// controller.
    badge: Option<usize>,
    last_caption: Option<Instant>,
    discoveries: Vec<Discovery>,
    /// Definition file mtimes as last seen, per project, so the poll
    /// re-reads only a changed file.
    config_seen: HashMap<ProjectId, Option<SystemTime>>,
    last_config: Option<Instant>,
    /// Transient state owned by the UI (dialog drafts, embedded terminals).
    /// Nothing in here is persisted or read by the core.
    pub ui_state: UiState,
    /// When set, every dispatched action is also appended to `dispatched`.
    /// UI tests use this to assert what a click did.
    pub record_actions: bool,
    pub dispatched: Vec<AppAction>,
    /// The control port, once `listen` bound it.
    control: Option<ControlSocket>,
}

/// Turn an adapter failure into the notice the user sees; the log keeps
/// the same line. `what` names the attempt, e.g. "kill s-1".
fn failed<E: std::fmt::Display, W: Into<String>>(
    result: Result<(), E>,
    what: impl FnOnce() -> W,
) -> Option<AppAction> {
    let e = result.err()?;
    let text = format!("{} failed: {e}", what().into());
    log::warn!("{text}");
    Some(AppAction::Failed(text))
}

impl SwitchboardApp {
    /// Creates the app with the given adapters. Real ones are assembled in
    /// `main.rs`, which then calls [`Self::start`]; tests pass fakes and
    /// seed the core instead.
    #[must_use]
    pub fn with_services(services: Services) -> Self {
        Self {
            core: AppCore::new(),
            services,
            started: Instant::now(),
            last_poll: None,
            last_dispatch: None,
            badge: None,
            last_caption: None,
            discoveries: Vec::new(),
            config_seen: HashMap::new(),
            last_config: None,
            ui_state: UiState::default(),
            record_actions: false,
            dispatched: Vec::new(),
            control: None,
        }
    }

    /// Startup: probe the host, take the store lock, load records, then
    /// list the host so the core reconciles. Never launches an agent.
    pub fn start(&mut self) {
        if let Err(e) = self.services.host.probe() {
            log::warn!("host unavailable: {e}");
            self.dispatch(AppAction::HostUnavailable(Some(e)));
        }
        let loaded = match self.services.store.lock() {
            Ok(true) => self.services.store.load_all(),
            Ok(false) => Err(StoreError::Locked),
            Err(e) => Err(e),
        };
        self.dispatch(AppAction::StoreLoaded(loaded));
        self.dispatch(AppAction::DispatchConfigured {
            command: self.services.dispatch.command(),
            data_dir: self.services.dispatch.data_dir(),
        });
        self.last_poll = Some(Instant::now());
        self.poll_events();
        self.poll_host();
        self.rearm_discoveries();
    }

    /// A running Codex pane whose record has no resume handle yet (the
    /// app was restarted, or the id appeared late) keeps being looked for.
    fn rearm_discoveries(&mut self) {
        let pending: Vec<(RecordId, PathBuf, SystemTime)> = self
            .core
            .workspaces()
            .iter()
            .flat_map(|w| &w.sessions)
            .filter(|r| {
                matches!(r.kind, SessionKind::Agent(AgentKind::Codex)) && r.resume.is_none()
            })
            .filter(|r| {
                self.core
                    .host_status(r.id)
                    .is_some_and(|h| matches!(h.liveness, Liveness::Running { .. }))
            })
            .map(|r| (r.id, r.cwd.clone(), r.created))
            .collect();
        for (id, cwd, since) in pending {
            log::info!("re-arming Codex id discovery for {}", id.host_name());
            self.discoveries.push(Discovery {
                id,
                kind: AgentKind::Codex,
                cwd,
                since,
                deadline: Instant::now() + DISCOVERY_TIMEOUT,
            });
        }
    }

    #[must_use]
    pub fn core(&self) -> &AppCore {
        &self.core
    }

    /// See [`AppCore::seed`]; tests and the demo launcher only.
    pub fn core_mut_for_seeding(&mut self) -> &mut AppCore {
        &mut self.core
    }

    #[must_use]
    pub fn services(&self) -> &Services {
        &self.services
    }

    fn clock(&self) -> Clock {
        Clock {
            mono: self.started.elapsed(),
            wall: SystemTime::now(),
        }
    }

    /// The single entry point for every user, worker, or timer action.
    pub fn dispatch(&mut self, action: AppAction) {
        if self.record_actions {
            self.dispatched.push(action.clone());
        }
        self.dispatch_inner(action);
    }

    /// The resume handles of the records an `Events` action names, so a
    /// handle swapped by an event (`/clear`) drops the cached
    /// conversation the way a discard does.
    fn resume_handles(&self, action: &AppAction) -> Vec<(RecordId, Option<ResumeHandle>)> {
        let AppAction::Events(events) = action else {
            return Vec::new();
        };
        events
            .iter()
            .filter_map(|e| e.record_id)
            .filter_map(|id| self.core.session(id).map(|s| (id, s.resume.clone())))
            .collect()
    }

    /// Dispatch without recording: effect results are consequences, not
    /// what the UI asked for.
    fn dispatch_inner(&mut self, action: AppAction) {
        let now = self.clock();
        // The conversation cache is keyed on the file's modification
        // time; a swap of the handle behind it needs a fresh read.
        if let AppAction::TranscriptDiscarded { id, .. } | AppAction::UndoDiscard(id) = &action {
            self.ui_state.conversations.remove(id);
        }
        let handles_before = self.resume_handles(&action);
        let effects = self.core.dispatch(action, now);
        for (id, before) in handles_before {
            if self.core.session(id).map(|s| s.resume.clone()) != Some(before) {
                self.ui_state.conversations.remove(&id);
            }
        }
        let prompt_box = self.core.settings().prompt_box;
        for (id, text) in self.core.take_primed() {
            if prompt_box {
                self.ui_state.primed.insert(id, text);
            } else {
                self.ui_state.input_drafts.insert(id, text);
            }
        }
        self.ui_state.requests.extend(self.core.take_ui_requests());
        for effect in effects {
            if let Some(result) = self.run_effect(effect) {
                self.dispatch_inner(result);
            }
        }
    }

    /// Type a message into a pane. The message box keeps its draft until
    /// the pane has the text, so a dead session or a failed write leaves
    /// it there to resend.
    fn send_input(&mut self, host: &HostId, text: &str) -> Option<AppAction> {
        let result = self.services.host.write_line(host, text);
        if result.is_ok() {
            let sent = self
                .core
                .workspaces()
                .iter()
                .flat_map(|w| &w.sessions)
                .find(|r| r.id.host_name() == host.0)
                .map(|r| r.id);
            if let Some(id) = sent {
                self.ui_state.input_drafts.remove(&id);
            }
        }
        failed(result, || format!("send input to {}", host.0))
    }

    /// The persistence effects; only a workspace save reports back.
    fn run_store_effect(&self, effect: Effect) -> Option<AppAction> {
        let store = &self.services.store;
        match effect {
            Effect::SaveSettings(settings) => {
                failed(store.save_settings(&settings), || "save settings")
            }
            Effect::SaveViews(views) => failed(store.save_views(&views), || "save views"),
            Effect::Save(ws) => {
                let id = ws.project.id;
                let result = store.save(&ws);
                if let Err(e) = &result {
                    log::error!("save failed: {e}");
                }
                Some(AppAction::SaveFinished(id, result))
            }
            Effect::Delete(id) => failed(store.delete(id), || "delete project"),
            Effect::StoreSecret { account, value } => {
                failed(self.services.secrets.set(&account, &value), || {
                    format!("store secret {account}")
                })
            }
            Effect::DeleteSecret(account) => failed(self.services.secrets.delete(&account), || {
                format!("delete secret {account}")
            }),
            Effect::PrepareLaunch { .. }
            | Effect::PrepareResume { .. }
            | Effect::CheckTranscript { .. }
            | Effect::CloneTranscript { .. }
            | Effect::CloneAllTranscript { .. }
            | Effect::DiscardTranscript { .. }
            | Effect::Discover { .. }
            | Effect::Spawn { .. }
            | Effect::Attach { .. }
            | Effect::Kill(_)
            | Effect::SendInput { .. }
            | Effect::SendKeys { .. }
            | Effect::ReadProjectConfig { .. }
            | Effect::WriteProjectConfig { .. }
            | Effect::ProbeRoundFile { .. }
            | Effect::SnapshotRound { .. }
            | Effect::RemoveRoundFiles { .. }
            | Effect::FindArtifacts { .. }
            | Effect::RemoveLog(_)
            | Effect::LogOperation { .. }
            | Effect::DispatchCall(_)
            | Effect::OpenPath(_)
            | Effect::Forget(_)
            | Effect::OpenInEditor { .. }
            | Effect::FocusWindow(_)
            | Effect::Reveal(_) => unreachable!("not a store effect"),
        }
    }

    /// Performs one effect; returns the action reporting its result, or
    /// `None` when the result arrives later (discovery) or has no report.
    /// The effects answered by the agent launcher and the transcript
    /// reader, kept out of `run_effect` for length.
    fn run_agent_effect(&self, effect: Effect) -> AppAction {
        let s = &self.services;
        match effect {
            Effect::PrepareLaunch {
                id,
                kind,
                name,
                cwd,
            } => AppAction::LaunchPrepared {
                id,
                result: s.agents.prepare_launch(kind, id, &name, &cwd),
            },
            Effect::PrepareResume {
                id,
                handle,
                name,
                cwd,
            } => AppAction::LaunchPrepared {
                id,
                result: s.agents.prepare_resume(&handle, id, &name, &cwd),
            },
            Effect::CheckTranscript { id, handle } => AppAction::TranscriptChecked {
                id,
                exists: s.agents.transcript_exists(&handle),
            },
            Effect::CloneTranscript {
                source,
                handle,
                before,
                prompt,
            } => AppAction::TranscriptCloned {
                source,
                prompt,
                result: s.transcripts.clone_before(&handle, before),
            },
            Effect::CloneAllTranscript { run, handle } => AppAction::WorkflowCloned {
                run,
                result: s.transcripts.clone_all(&handle),
            },
            Effect::DiscardTranscript {
                id,
                handle,
                before,
                prompt,
            } => AppAction::TranscriptDiscarded {
                id,
                before,
                prompt,
                result: s.transcripts.clone_before(&handle, before),
            },
            _ => unreachable!("not an agent effect"),
        }
    }

    fn run_effect(&mut self, effect: Effect) -> Option<AppAction> {
        let s = &self.services;
        match effect {
            Effect::SaveSettings(_)
            | Effect::SaveViews(_)
            | Effect::Save(_)
            | Effect::Delete(_)
            | Effect::StoreSecret { .. }
            | Effect::DeleteSecret(_) => self.run_store_effect(effect),
            Effect::PrepareLaunch { .. }
            | Effect::PrepareResume { .. }
            | Effect::CheckTranscript { .. }
            | Effect::CloneTranscript { .. }
            | Effect::CloneAllTranscript { .. }
            | Effect::DiscardTranscript { .. } => Some(self.run_agent_effect(effect)),
            Effect::ProbeRoundFile { .. }
            | Effect::SnapshotRound { .. }
            | Effect::RemoveRoundFiles { .. } => Some(self.run_round_files(effect)),
            Effect::Discover {
                id,
                kind,
                cwd,
                since,
            } => {
                self.discoveries.push(Discovery {
                    id,
                    kind,
                    cwd,
                    since,
                    deadline: Instant::now() + DISCOVERY_TIMEOUT,
                });
                None
            }
            Effect::Spawn { id, mut spec } => {
                spec.scrollback = Some(self.log_path(id, &spec.id));
                spec.env = self.spawn_env(id, spec.env);
                let result = s.host.spawn(&spec).map_err(|e| e.to_string());
                if let Err(e) = &result {
                    log::error!("spawn {} failed: {e}", spec.id.0);
                }
                Some(AppAction::Spawned { id, result })
            }
            Effect::Attach {
                id,
                host,
                title,
                cwd,
            } => Some(AppAction::Attached {
                id,
                result: self.attach(&host, &title, &cwd),
            }),
            Effect::SendInput { host, text } => self.send_input(&host, &text),
            Effect::SendKeys { host, bytes } => failed(s.host.write(&host, &bytes), || {
                format!("send keys to {}", host.0)
            }),
            Effect::ReadProjectConfig { project, root } => {
                self.config_seen
                    .insert(project, s.project_config.modified(&root));
                Some(AppAction::ProjectConfigRead {
                    project,
                    result: s.project_config.read(&root),
                })
            }
            Effect::WriteProjectConfig {
                project,
                root,
                text,
            } => Some(AppAction::ProjectConfigWritten {
                project,
                result: s.project_config.write_text(&root, &text),
            }),
            Effect::Kill(host) => failed(s.host.kill(&host), || format!("kill {}", host.0)),
            Effect::Forget(_)
            | Effect::RemoveLog(_)
            | Effect::LogOperation { .. }
            | Effect::FindArtifacts { .. } => self.run_log_effect(effect),
            Effect::OpenPath(path) => failed(s.opener.open_default(&path), || {
                format!("open {}", path.display())
            }),
            Effect::DispatchCall(body) => {
                let result = self
                    .services
                    .dispatch
                    .call(&body)
                    .map_err(|e| e.to_string());
                Some(AppAction::DispatchReplied { body, result })
            }
            // Windows are the UI's; it raises the one asked for next frame.
            Effect::FocusWindow(id) => {
                self.ui_state.focus_windows.push(id);
                None
            }
            Effect::OpenInEditor { editor, path } => {
                failed(s.opener.open_editor(&editor, &path), || {
                    format!("open {} in {editor}", path.display())
                })
            }
            Effect::Reveal(path) => failed(s.opener.reveal(&path), || {
                format!("reveal {}", path.display())
            }),
        }
    }

    /// The effects on run logs and artifacts, kept out of `run_effect`
    /// for length.
    fn run_log_effect(&mut self, effect: Effect) -> Option<AppAction> {
        match effect {
            Effect::Forget(host) => {
                // The record's own log and every run's (`<host>-r<n>.vt`).
                let dir = self.services.store.data_dir().join("scrollback");
                let prefix = format!("{}-r", host.0);
                let mut paths = vec![self.scrollback_path(&host)];
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    paths.extend(entries.flatten().map(|e| e.path()).filter(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.starts_with(&prefix))
                    }));
                }
                for path in paths {
                    remove_quietly(&path);
                }
                None
            }
            Effect::RemoveLog(name) => {
                remove_quietly(&self.services.store.data_dir().join("scrollback").join(name));
                None
            }
            Effect::LogOperation { op, kind, ids } => {
                let line = OpLine::Requested {
                    op,
                    kind,
                    ids,
                    at: SystemTime::now(),
                };
                failed(self.services.operations.append(&line), || "operations log")
            }
            Effect::FindArtifacts {
                id,
                n,
                cwd,
                patterns,
                since,
            } => Some(AppAction::ArtifactsFound {
                id,
                n,
                paths: self.services.artifacts.find(&cwd, &patterns, since),
            }),
            _ => unreachable!("not a log effect"),
        }
    }

    /// The effects answered by the round files adapter. Snapshots go
    /// under `<data dir>/workflows/<run>/round-<n>/`.
    fn run_round_files(&self, effect: Effect) -> AppAction {
        let s = &self.services;
        match effect {
            Effect::ProbeRoundFile { run, path } => AppAction::RoundFileProbed {
                run,
                found: s.round_files.probe(&path),
                path,
            },
            Effect::SnapshotRound {
                run,
                n,
                files,
                note,
            } => {
                let dir = crate::core::snapshot_dir(&s.store.data_dir(), run, n);
                AppAction::RoundSnapshotted {
                    run,
                    n,
                    result: s.round_files.snapshot(&files, &dir, note.as_deref()),
                }
            }
            Effect::RemoveRoundFiles { run, files } => AppAction::RoundFilesRemoved {
                run,
                result: s.round_files.remove(&files),
            },
            _ => unreachable!("not a round files effect"),
        }
    }

    /// Raise the terminal window for this session if one exists,
    /// otherwise open a new one attached to the host session.
    fn attach(&self, host: &HostId, title: &str, cwd: &std::path::Path) -> Result<(), String> {
        let s = &self.services;
        if s.opener.raise_terminal(title)? {
            Ok(())
        } else {
            s.opener
                .open_terminal(title, &s.host.attach_command(host), cwd)
        }
    }

    /// A pane that is gone still has its output on disk: the open
    /// session shows the tail of it, cards get a caption from it once.
    fn cold_scrollback(&mut self, id: RecordId, host: &HostId, on_screen: bool) {
        if !on_screen && self.ui_state.captions.contains_key(&id) {
            return;
        }
        let path = self.log_path(id, host);
        let Ok(text) = crate::adapters::scrollback::tail_text(&path, 200) else {
            return;
        };
        if let Some(last) = text.lines().rev().find(|l| !l.trim().is_empty()) {
            self.ui_state.captions.insert(id, last.trim().to_owned());
        }
        if on_screen {
            self.ui_state.snapshots.insert(id, text);
        }
    }

    fn poll_host(&mut self) {
        match self.services.host.list() {
            Ok(list) => {
                log::debug!("host lists {} pane(s)", list.len());
                self.dispatch(AppAction::HostListed(list));
            }
            Err(e) => log::warn!("host list failed: {e}"),
        }
    }

    /// Re-read a project's definition file when its mtime moved. One
    /// `stat` per project, so it runs on the poll tick, not a thread.
    fn poll_configs(&mut self) {
        let roots: Vec<(ProjectId, PathBuf)> = self
            .core
            .workspaces()
            .iter()
            .map(|w| (w.project.id, w.project.root.clone()))
            .collect();
        for (project, root) in roots {
            let now = self.services.project_config.modified(&root);
            if self.config_seen.get(&project) != Some(&now) {
                self.config_seen.insert(project, now);
                self.dispatch(AppAction::ProjectConfigRead {
                    project,
                    result: self.services.project_config.read(&root),
                });
            }
        }
    }

    fn poll_events(&mut self) {
        let events = self.services.events.poll();
        if !events.is_empty() {
            self.dispatch(AppAction::Events(events));
            self.services.events.checkpoint();
        }
    }

    /// Pending Codex discoveries: cheap file scans, so run on the poll tick.
    fn poll_discoveries(&mut self) {
        let mut finished = Vec::new();
        for (i, d) in self.discoveries.iter().enumerate() {
            let alive = self
                .core
                .host_status(d.id)
                .is_some_and(|h| matches!(h.liveness, Liveness::Running { .. }));
            match self.services.agents.discover(d.kind, &d.cwd, d.since) {
                Ok(None) if alive && Instant::now() < d.deadline => {}
                Ok(None) => finished.push((i, Ok(None))),
                other => finished.push((i, other)),
            }
        }
        for (i, result) in finished.into_iter().rev() {
            let d = self.discoveries.remove(i);
            self.dispatch(AppAction::Discovered { id: d.id, result });
        }
    }

    /// The conversation shown in the session view, re-read only when the
    /// transcript file changed. Reading is synchronous: transcripts are
    /// usually well under a megabyte and parse in a few milliseconds,
    /// while a long session of several megabytes costs tens of
    /// milliseconds once per change, which one frame absorbs.
    fn refresh_conversation(&mut self, id: RecordId) {
        let Some(handle) = self.core.session(id).and_then(|s| {
            matches!(s.kind, SessionKind::Agent(_))
                .then(|| s.resume.clone())
                .flatten()
        }) else {
            return;
        };
        let modified = self.services.transcripts.modified(&handle);
        let cached = self.ui_state.conversations.get(&id).map(|(m, _)| *m);
        if cached == Some(modified) {
            return;
        }
        match self.services.transcripts.read(&handle) {
            Ok(conversation) => {
                self.ui_state.conversation_errors.remove(&id);
                self.ui_state
                    .conversations
                    .insert(id, (modified, conversation));
            }
            Err(e) => {
                self.ui_state.conversations.remove(&id);
                self.ui_state.conversation_errors.insert(id, e);
            }
        }
    }

    /// The project's environment under the launcher's own variables, so
    /// an agent-specific value still wins.
    fn spawn_env(&self, id: RecordId, own: Vec<(String, String)>) -> Vec<(String, String)> {
        let Some(pid) = self.core.session(id).map(|s| s.project) else {
            return own;
        };
        let resolved = resolve_project_env(&self.core, &self.services, pid);
        let missing = resolved.missing();
        if !missing.is_empty() {
            log::warn!("secrets without a stored value: {}", missing.join(", "));
        }
        let mut env = resolved.pairs();
        log::debug!(
            "injecting {:?} into {}",
            env.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            id.host_name()
        );
        env.extend(own);
        env
    }

    /// Where the host pipes a session's raw output.
    fn scrollback_path(&self, host: &HostId) -> PathBuf {
        self.services
            .store
            .data_dir()
            .join("scrollback")
            .join(format!("{}.vt", host.0))
    }

    /// Where a record's output goes and is read from: its latest run's
    /// log when it has runs, else the one file named after the host.
    fn log_path(&self, id: RecordId, host: &HostId) -> PathBuf {
        match self.core.session(id).and_then(|s| s.runs.last()) {
            Some(run) => self
                .services
                .store
                .data_dir()
                .join("scrollback")
                .join(&run.log),
            None => self.scrollback_path(host),
        }
    }

    /// Captions for cards on screen and the snapshot for the open session.
    /// Agent cards excerpt the transcript rather than the pane, so their
    /// conversations are kept fresh too (a stat each; a read on change).
    fn refresh_captions(&mut self) {
        let view = self.core.view();
        if let View::Session(id) = view {
            self.refresh_conversation(id);
        }
        let mut ids: Vec<RecordId> = match view {
            View::Switchboard => self
                .core
                .all_sessions_sorted()
                .iter()
                .map(|s| s.id)
                .collect(),
            View::Board(p) => self.core.sessions_sorted(p).iter().map(|s| s.id).collect(),
            View::Session(id) => vec![id],
            View::Document(..) | View::Workflow(_) | View::Dispatch | View::Ticket(_) => Vec::new(),
            View::WorkingSet(set) => self.core.working_set_sessions(set),
        };
        // The Run tab shows every command's and service's output, so
        // those get a snapshot too while it is on screen.
        let run_project = match view {
            View::Board(p) => Some(p),
            View::Session(id) if self.core.settings().files_open => {
                self.core.session(id).map(|s| s.project)
            }
            View::Session(_)
            | View::Switchboard
            | View::Document(..)
            | View::WorkingSet(_)
            | View::Workflow(_)
            | View::Dispatch
            | View::Ticket(_) => None,
        }
        .filter(|_| self.core.settings().side_tab == crate::core::SideTab::Run);
        let run_set: Vec<RecordId> = run_project
            .map(|p| self.core.run_entries(p).iter().map(|s| s.id).collect())
            .unwrap_or_default();
        // The run bar under a session header shows each entry's last
        // line on hover, so those need captions even with the side closed.
        let bar_set: Vec<RecordId> = match view {
            View::Session(id) => self
                .core
                .session(id)
                .map(|s| {
                    self.core
                        .run_entries(s.project)
                        .iter()
                        .map(|s| s.id)
                        .collect()
                })
                .unwrap_or_default(),
            View::Board(_)
            | View::Switchboard
            | View::Document(..)
            | View::WorkingSet(_)
            | View::Dispatch
            | View::Ticket(_)
            | View::Workflow(_) => Vec::new(),
        };
        for id in run_set.iter().chain(&bar_set) {
            if !ids.contains(id) {
                ids.push(*id);
            }
        }
        // Working-set shell cards show the pane's tail, so they need the
        // snapshot too while the set is on screen.
        let set_ids: Vec<RecordId> = match view {
            View::WorkingSet(set) => self.core.working_set_sessions(set),
            _ => Vec::new(),
        };
        for id in ids {
            let on_screen = matches!(view, View::Session(sid) if sid == id);
            if !on_screen {
                self.refresh_conversation(id);
            }
            let wants_snapshot = on_screen || run_set.contains(&id) || set_ids.contains(&id);
            // A pane that exited still exists (`remain-on-exit`), but its
            // output on disk is complete, so read that like a gone pane.
            let running = self
                .core
                .host_status(id)
                .is_some_and(|h| matches!(h.liveness, Liveness::Running { .. }));
            let host = HostId(id.host_name());
            if !running {
                self.cold_scrollback(id, &host, wants_snapshot);
                continue;
            }
            let lines = if wants_snapshot { Some(60) } else { Some(3) };
            if let Ok(text) = self.services.host.snapshot(&host, lines) {
                if wants_snapshot {
                    self.ui_state.snapshots.insert(id, text.clone());
                }
                if let Some(last) = text.lines().rev().find(|l| !l.trim().is_empty()) {
                    self.ui_state.captions.insert(id, last.trim().to_owned());
                }
            }
        }
    }

    /// One full poll right now, ignoring the timers: events, host list,
    /// Codex discoveries, captions. For tests and the launcher, which
    /// drive the app without a frame loop.
    pub fn poll_now(&mut self) {
        self.last_poll = Some(Instant::now());
        self.last_caption = Some(Instant::now());
        self.poll_events();
        self.poll_host();
        self.poll_discoveries();
        self.poll_configs();
        self.refresh_captions();
    }

    fn pump(&mut self) {
        self.serve_pending();
        // Every frame, not every poll: a button press should land in
        // the frame its wake-up requested.
        for event in self.services.controller.poll() {
            self.dispatch(AppAction::Controller(event));
        }
        let woken = self
            .services
            .wake
            .as_ref()
            .is_some_and(WakeSocket::take_woken);
        let due = self.last_poll.is_none_or(|t| t.elapsed() >= POLL_INTERVAL);
        if woken || due {
            self.last_poll = Some(Instant::now());
            self.poll_events();
            self.poll_host();
            self.poll_discoveries();
        }
        if self
            .last_caption
            .is_none_or(|t| t.elapsed() >= CAPTION_INTERVAL)
        {
            self.last_caption = Some(Instant::now());
            self.refresh_captions();
        }
        if self
            .last_config
            .is_none_or(|t| t.elapsed() >= CONFIG_INTERVAL)
        {
            self.last_config = Some(Instant::now());
            self.poll_configs();
        }
        if self
            .last_dispatch
            .is_none_or(|t| t.elapsed() >= DISPATCH_INTERVAL)
        {
            self.last_dispatch = Some(Instant::now());
            self.poll_dispatch();
        }
    }

    /// Ask Dispatch's port for its status. No runner is a quick error
    /// (no socket, or nobody listening), reported to the core as such.
    fn poll_dispatch(&mut self) {
        let status = match self
            .services
            .dispatch
            .call(&crate::ports::dispatch::Body::Status)
        {
            Ok(crate::ports::dispatch::Reply::Status(status)) => Some(status),
            Ok(other) => {
                log::warn!("dispatch status answered {other:?}");
                None
            }
            Err(e) => {
                log::debug!("dispatch status: {e}");
                None
            }
        };
        self.dispatch(AppAction::DispatchStatus(status));
    }
}

impl eframe::App for SwitchboardApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.dispatch(AppAction::Tick);
        if self.last_poll.is_some() {
            // Only after `start`: tests never poll, and never badge.
            self.pump();
            ui.ctx().request_repaint_after(POLL_INTERVAL);
            let waiting = self.core.waiting_count();
            if self.badge != Some(waiting) {
                crate::adapters::dock::set_waiting_badge(waiting);
                self.services.controller.set_waiting(waiting);
                self.badge = Some(waiting);
            }
        }
        crate::ui::draw(self, ui);
    }

    /// Quitting saves where the windows are, even ones that moved in the
    /// last moment and had not held still long enough to be saved yet.
    fn on_exit(&mut self) {
        if let Some((frame, _)) = self.ui_state.main_frame.take()
            && self.core.settings().main_window.as_ref() != Some(&frame)
        {
            self.dispatch(AppAction::MainWindowMoved(frame));
        }
        let popouts = std::mem::take(&mut self.ui_state.popout_frames);
        for (id, (frame, _)) in popouts {
            let saved = self
                .core
                .settings()
                .popouts
                .iter()
                .find(|p| p.session == id);
            if saved.is_some_and(|p| p.frame.as_ref() != Some(&frame)) {
                self.dispatch(AppAction::PopoutMoved(id, frame));
            }
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        crate::ui::zoom::before_main_pass(&self.ui_state, ctx, raw_input);
    }

    /// Fully transparent: the rail, side, and central panels paint their
    /// own opaque fills, and the caption overlay viewport needs a
    /// transparent backbuffer, which eframe enables from the root's.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }
}

/// Delete a file that may already be gone.
fn remove_quietly(path: &std::path::Path) {
    if let Err(e) = std::fs::remove_file(path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("remove {}: {e}", path.display());
    }
}

// --- the control port

/// The record kind each id of a creation's log line names.
fn logged_kind(command: &str, position: usize) -> wire::RecordKind {
    match command {
        "project.add" => wire::RecordKind::Project,
        "space.new" => wire::RecordKind::Space,
        "set.new" => wire::RecordKind::Set,
        "workflow.start" if position == 0 => wire::RecordKind::Run,
        _ => wire::RecordKind::Session,
    }
}

fn parse_uuid(kind: &str, text: &str) -> Result<uuid::Uuid, String> {
    uuid::Uuid::parse_str(text).map_err(|_| format!("{kind} id {text:?} is not a uuid"))
}

impl SwitchboardApp {
    /// Bind the control socket in the data directory. Refused for a
    /// read-only instance: the one holding the store lock owns the port.
    pub fn listen(&mut self, wake: impl Fn() + Send + Sync + 'static) -> io::Result<PathBuf> {
        let dir = self.services.store.data_dir();
        self.listen_at(&dir, wake)
    }

    /// `listen` with the directory chosen; tests keep each socket apart.
    pub fn listen_at(
        &mut self,
        dir: &std::path::Path,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> io::Result<PathBuf> {
        if self.core.read_only() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "another instance holds the data directory; not listening",
            ));
        }
        let socket = ControlSocket::bind(dir, wake)?;
        let path = socket.path().to_path_buf();
        self.control = Some(socket);
        Ok(path)
    }

    /// Answer every request that has arrived. Called once per frame; a
    /// test calls it while its client waits.
    pub fn serve_pending(&mut self) {
        let incoming: Vec<Incoming> = self
            .control
            .as_mut()
            .map(ControlSocket::poll)
            .unwrap_or_default();
        for Incoming { request, reply } in incoming {
            let answer = self.serve(&request);
            let _ = reply.send(answer);
        }
    }

    /// One request, one reply. A query reads; a command runs through the
    /// core under its operation id, and its reply is logged so a repeat
    /// of the same id is answered from the log without running again.
    pub fn serve(&mut self, request: &wire::Request) -> wire::Reply {
        if !request.body.is_command() {
            return self.answer(&request.body);
        }
        let op = &request.op;
        if op.trim().is_empty() {
            return wire::Reply::failed("a command needs an op");
        }
        if let Some(reply) = self.replied(op) {
            return reply;
        }
        let action = match ControlAction::try_from(request.body.clone()) {
            Ok(action) => action,
            Err(reason) => return wire::Reply::failed(reason),
        };
        self.dispatch(AppAction::Control {
            op: op.clone(),
            action,
        });
        let reply = match self.core.take_control_outcome(op) {
            None => wire::Reply::failed("the command produced no outcome"),
            Some(ControlOutcome {
                error: Some(reason),
                ..
            }) => wire::Reply::failed(reason),
            Some(ControlOutcome { made, .. }) => {
                let launched = made.iter().any(|m| {
                    m.kind == wire::RecordKind::Session
                        && uuid::Uuid::parse_str(&m.id)
                            .is_ok_and(|id| self.core.host_status(RecordId(id)).is_some())
                });
                if launched {
                    wire::Reply::Launched { made }
                } else {
                    wire::Reply::Persisted { made }
                }
            }
        };
        let line = OpLine::Replied {
            op: op.clone(),
            reply: serde_json::to_string(&reply).unwrap_or_default(),
            at: SystemTime::now(),
        };
        if let Err(e) = self.services.operations.append(&line) {
            log::error!("operations log: {e}");
        }
        reply
    }

    /// The reply already given to `op`, if the log has one.
    fn replied(&self, op: &str) -> Option<wire::Reply> {
        self.services
            .operations
            .find(op)
            .into_iter()
            .find_map(|line| match line {
                OpLine::Replied { reply, .. } => wire::Reply::parse(&reply).ok(),
                OpLine::Requested { .. } => None,
            })
    }

    fn answer(&self, body: &wire::Body) -> wire::Reply {
        let now = SystemTime::now();
        let core = &self.core;
        match body {
            wire::Body::Projects { space } => {
                let space = match space {
                    Some(s) => match parse_uuid("space", s) {
                        Ok(id) => Some(SpaceId(id)),
                        Err(reason) => return wire::Reply::failed(reason),
                    },
                    None => None,
                };
                wire::Reply::Projects {
                    projects: core.project_views(space),
                }
            }
            wire::Body::Spaces => wire::Reply::Spaces {
                spaces: core.space_views(),
            },
            wire::Body::Sets { space } => match parse_uuid("space", space) {
                Ok(id) => wire::Reply::Sets {
                    sets: core.set_views(SpaceId(id)),
                },
                Err(reason) => wire::Reply::failed(reason),
            },
            wire::Body::Sessions { project } => match parse_uuid("project", project) {
                Ok(id) => match core.workspace(ProjectId(id)) {
                    Some(w) => wire::Reply::Sessions {
                        sessions: w
                            .sessions
                            .iter()
                            .filter_map(|s| core.session_view(s.id, now))
                            .collect(),
                    },
                    None => wire::Reply::failed("no such project"),
                },
                Err(reason) => wire::Reply::failed(reason),
            },
            wire::Body::Session { session } => match parse_uuid("session", session) {
                Ok(id) => match core.session_view(RecordId(id), now) {
                    Some(session) => wire::Reply::Session { session },
                    None => wire::Reply::failed("no such session"),
                },
                Err(reason) => wire::Reply::failed(reason),
            },
            wire::Body::Waiting => wire::Reply::Waiting {
                sessions: core.waiting_views(now),
            },
            wire::Body::Workflow { run } => match parse_uuid("run", run) {
                Ok(id) => match core.run_view(WorkflowId(id)) {
                    Some(run) => wire::Reply::Workflow { run },
                    None => wire::Reply::failed("no such run"),
                },
                Err(reason) => wire::Reply::failed(reason),
            },
            wire::Body::Workflows { project } => match parse_uuid("project", project) {
                Ok(id) => match core.workspace(ProjectId(id)) {
                    Some(w) => wire::Reply::Workflows {
                        runs: w
                            .workflows
                            .iter()
                            .filter_map(|r| core.run_view(r.id))
                            .collect(),
                    },
                    None => wire::Reply::failed("no such project"),
                },
                Err(reason) => wire::Reply::failed(reason),
            },
            wire::Body::Find { operation } => wire::Reply::Found {
                records: self.find_op(operation, now),
            },
            wire::Body::OpStatus { operation } => wire::Reply::OpStatus {
                status: self.op_status(operation),
            },
            _ => wire::Reply::failed("not a query"),
        }
    }

    /// Every record `op` made: the ones still present with their state,
    /// and the ones the log names that are gone, marked removed. The log
    /// is read first, so a record removed in the window is still
    /// reported as made.
    fn find_op(&self, op: &str, now: SystemTime) -> Vec<wire::Found> {
        let mut found = self.core.records_with_op(op, now);
        for line in self.services.operations.find(op) {
            let OpLine::Requested { kind, ids, .. } = line else {
                continue;
            };
            for (n, id) in ids.iter().enumerate() {
                if !found.iter().any(|f| &f.id == id) {
                    found.push(wire::Found {
                        kind: logged_kind(&kind, n),
                        id: id.clone(),
                        removed: true,
                        session: None,
                        run: None,
                    });
                }
            }
        }
        found
    }

    /// Requests are answered in the same frame they run, so a request
    /// line with no reply line can only mean the app died between the
    /// two; so can a record still marked as launching.
    fn op_status(&self, op: &str) -> wire::OpStatus {
        let lines = self.services.operations.find(op);
        let replied = lines.iter().find_map(|l| match l {
            OpLine::Replied { reply, .. } => wire::Reply::parse(reply).ok(),
            OpLine::Requested { .. } => None,
        });
        let requested = lines.iter().any(|l| matches!(l, OpLine::Requested { .. }));
        let interrupted = self.core.interrupted_ops().iter().any(|o| o == op);
        if interrupted {
            return wire::OpStatus::Interrupted;
        }
        match replied {
            Some(reply) => wire::OpStatus::Done {
                reply: Box::new(reply),
            },
            None if requested => wire::OpStatus::Interrupted,
            None => wire::OpStatus::Unknown,
        }
    }
}
