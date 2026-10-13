//! Orchestration: owns [`AppCore`] and the adapters. Maps effects to
//! adapter calls and feeds results back as actions; polls the host and
//! the event log on a timer; keeps the UI's captions fresh. Rendering
//! lives in [`crate::ui`].

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use switchboard_control as wire;

use crate::adapters::control::{ControlSocket, Incoming};
use crate::adapters::hooks::WakeSocket;
use crate::adapters::remote::{
    self, RemoteIncoming, RemotePort, RemoteReply, RemoteRequest, SubscriberFrame, SubscriberQueue,
};
use crate::adapters::scrollback::scrollback_dir;
use crate::content::{
    ACTIVITY_BYTES_CAP, ContentJob, ContentKey, ContentVersion, ContentView, ContentWorker,
    ConversationCache, Observed, PAGE_TURNS_CAP, PAGE_TURNS_DEFAULT, PageFrom, Policy,
    RUN_LINES_CAP, SCREEN_LINES_CAP, SCREEN_SLACK_LINES, Source, TURN_BYTES_CAP, WATCH_CAP,
    WATCH_IDLE, Watch, redact,
};
use crate::core::revision::Fingerprints;
use crate::core::{
    Activity, AgentKind, AppAction, AppCore, Clock, Composer, ControlAction, ControlOutcome,
    DISPATCH_POLL, Effect, ProjectId, RECORD_TOKEN_ENV, RecordId, Resolved, ResumeHandle,
    SessionKind, SessionRecord, SetsResolved, SpaceId, View, WorkflowId, aws_view, file_refs,
    resolve_sets, token_hash,
};
use crate::ports::agent::AgentLauncher;
use crate::ports::artifacts::ArtifactFinder;
use crate::ports::control::{OpLine, Operations};
use crate::ports::controller::Controller;
use crate::ports::dispatch::{Body, DispatchPort, Reply, Status};
use crate::ports::events::EventSource;
use crate::ports::host::{HostId, ProcessHost, SpawnSpec};
use crate::ports::opener::Opener;
use crate::ports::project_config::ProjectConfigReader;
use crate::ports::remote::{Advertisement, Advertiser, Listen, PairedDevice, RemoteStore, Scope};
use crate::ports::round_files::RoundFiles;
use crate::ports::secrets::SecretStore;
use crate::ports::store::{Store, StoreError};
use crate::ports::transcript::TranscriptReader;
use crate::ui::UiState;

/// How often the host is listed and the event log read.
const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// The line Claude Code prints in its folder trust question, and how
/// far back in a pane to look for it.
const TRUST_PROMPT_MARK: &str = "trust this folder";
const TRUST_PROMPT_LINES: usize = 20;

/// How often definition files are checked for a change.
const CONFIG_INTERVAL: Duration = Duration::from_secs(5);
/// How often card captions and the session snapshot are refreshed.
const CAPTION_INTERVAL: Duration = Duration::from_secs(2);
/// How long a Codex id discovery keeps looking. Codex writes its rollout
/// file on the first prompt, not at launch, so this is generous; a
/// discovery also ends as soon as the pane is gone.
const DISCOVERY_TIMEOUT: Duration = Duration::from_hours(24);

pub struct Services {
    pub store: Box<dyn Store>,
    /// An `Arc`: the content worker reads panes through the same host.
    pub host: Arc<dyn ProcessHost>,
    pub events: Box<dyn EventSource>,
    pub agents: Box<dyn AgentLauncher>,
    pub opener: Box<dyn Opener>,
    pub transcripts: Arc<dyn TranscriptReader>,
    pub secrets: Box<dyn SecretStore>,
    pub project_config: Box<dyn ProjectConfigReader>,
    pub round_files: Box<dyn RoundFiles>,
    /// A branch's commits and files over its base, for the ticket page.
    /// An `Arc` rather than a `Box`: the page hands a clone to the thread
    /// that reads git while `Services` stays borrowed by the frame.
    pub changes: std::sync::Arc<dyn crate::ports::changes::BranchChanges>,
    pub artifacts: Box<dyn ArtifactFinder>,
    /// The hand controller (a nunchuk over serial); a fake in tests.
    pub controller: Box<dyn Controller>,
    /// The control port's operations log.
    pub operations: Box<dyn Operations>,
    /// The remote port's identity and device files.
    pub remote: Box<dyn RemoteStore>,
    /// Bonjour, for the remote port while it is on the local network.
    pub advertiser: Box<dyn Advertiser>,
    /// Dispatch's port: tickets as views, decisions answered. The app
    /// takes it into a worker of its own when it starts, so it is
    /// `None` after `SwitchboardApp::with_services`; `None` before it
    /// means the app runs without Dispatch.
    pub dispatch: Option<Box<dyn DispatchPort>>,
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
    /// The Dispatch port on a thread of its own when it may block.
    dispatch_port: DispatchWorker,
    /// The waiting count last put on the Dock badge and sent to the
    /// controller.
    badge: Option<usize>,
    last_caption: Option<Instant>,
    discoveries: Vec<Discovery>,
    /// Definition file mtimes as last seen, per project, so the poll
    /// re-reads only a changed file.
    config_seen: HashMap<ProjectId, Option<SystemTime>>,
    last_config: Option<Instant>,
    /// Parsed transcripts with the version each was read at, for the
    /// views and a remote port alike.
    pub conversations: ConversationCache,
    /// Transient state owned by the UI (dialog drafts, embedded terminals).
    /// Nothing in here is persisted or read by the core.
    pub ui_state: UiState,
    /// When set, every dispatched action is also appended to `dispatched`.
    /// UI tests use this to assert what a click did.
    pub record_actions: bool,
    pub dispatched: Vec<AppAction>,
    /// The control port, once `listen` bound it.
    control: Option<ControlSocket>,
    /// The remote port's listener while it is bound, and how to wake
    /// the frame when a request arrives (kept so a change of address
    /// can rebind). `remote_bound` is what the listener was bound with.
    remote: Option<RemotePort>,
    remote_wake: Option<Arc<dyn Fn() + Send + Sync>>,
    remote_bound: Option<Listen>,
    /// `SWITCHBOARD_REMOTE_PORT`: a port in place of the identity's, so
    /// two instances can run on one machine; `0` picks a free one.
    remote_port_override: Option<u16>,
    /// This run's epoch and the revision counter every remote reply
    /// carries; the counter moves once per pass whose read-model diff
    /// found a change.
    epoch: uuid::Uuid,
    revision: u64,
    /// The read models as the last diff pass saw them, and whether a
    /// dispatch since then makes another pass worth taking.
    fingerprints: Option<Fingerprints>,
    dirty: bool,
    /// The devices subscribed to events, each with its connection's
    /// queue; a revocation drops the device's entry and so its socket.
    subscribers: Vec<Subscriber>,
    /// The content worker, started with the port, and what it reports.
    content: Option<ContentWorker>,
    observed: Option<mpsc::Receiver<Observed>>,
    /// The version last served per content key, and the keys devices
    /// watch or read lately, with who asked and when each was read.
    versions: std::collections::HashMap<ContentKey, ContentVersion>,
    watches: std::collections::HashMap<ContentKey, Watch>,
    watch_dirty: bool,
    last_watch_sweep: Option<Instant>,
    /// Each session's redaction policy as last resolved; cleared by any
    /// save or secret change, since either may have moved it.
    policies: std::collections::HashMap<RecordId, Policy>,
    /// File diffs being read on threads of their own.
    diff_reads: DiffReads,
}

/// One WebSocket subscriber as the frame holds it.
struct Subscriber {
    id: u64,
    queue: SubscriberQueue,
    device: uuid::Uuid,
}

/// How a remote request was answered: now, or by the content worker
/// through the reply channel it was handed.
enum Served {
    Now(RemoteReply),
    Deferred,
}

/// A file diff's answer from its thread.
struct DiffDone {
    ticket: String,
    lane: String,
    path: String,
    /// The range it was read for.
    range: crate::core::diff::DiffRange,
    result: Result<crate::ports::changes::FileDiff, String>,
}

/// The threads reading file diffs: each sends one answer, and `out`
/// counts the ones not yet drained.
struct DiffReads {
    tx: std::sync::mpsc::Sender<DiffDone>,
    rx: std::sync::mpsc::Receiver<DiffDone>,
    out: usize,
}

impl Default for DiffReads {
    fn default() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        Self { tx, rx, out: 0 }
    }
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
    pub fn with_services(mut services: Services) -> Self {
        // `take` moves the boxed port out and leaves `None`, so the
        // worker owns the only handle to it.
        let dispatch_port = DispatchWorker::new(services.dispatch.take());
        Self {
            core: AppCore::new(),
            services,
            dispatch_port,
            started: Instant::now(),
            last_poll: None,
            last_dispatch: None,
            badge: None,
            last_caption: None,
            discoveries: Vec::new(),
            config_seen: HashMap::new(),
            last_config: None,
            conversations: ConversationCache::default(),
            ui_state: UiState::default(),
            record_actions: false,
            dispatched: Vec::new(),
            control: None,
            remote: None,
            remote_wake: None,
            remote_bound: None,
            remote_port_override: None,
            epoch: uuid::Uuid::new_v4(),
            revision: 0,
            fingerprints: None,
            dirty: false,
            subscribers: Vec::new(),
            content: None,
            observed: None,
            versions: std::collections::HashMap::new(),
            watches: std::collections::HashMap::new(),
            watch_dirty: false,
            last_watch_sweep: None,
            policies: std::collections::HashMap::new(),
            diff_reads: DiffReads::default(),
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
        self.load_identity();
        self.dispatch(AppAction::DispatchConfigured {
            command: self.dispatch_port.command.clone(),
            data_dir: self.dispatch_port.data_dir.clone(),
            switchboard_data_dir: self.services.store.data_dir(),
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
            .filter(|r| self.core.is_running(r.id))
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

    /// See [`AppCore::seed`]; tests only.
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
        self.diff_pass();
        self.sweep_watches();
    }

    /// Tell the core how wide the working-set view is. Not recorded: it
    /// is a measurement of the window, not something the user asked for.
    pub(crate) fn set_view_columns(&mut self, columns: u32) {
        self.dispatch_inner(AppAction::ViewColumns(columns));
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
        self.dirty = true;
        if let AppAction::RevokeDevice(device) = &action {
            // Dropping the queue is the close: the connection thread
            // sees the channel end and closes the socket.
            let gone: Vec<u64> = self
                .subscribers
                .iter()
                .filter(|s| s.device == *device)
                .map(|s| s.id)
                .collect();
            self.subscribers.retain(|s| s.device != *device);
            for id in gone {
                self.forget_subscriber(id);
            }
        }
        // A discard puts a cut copy behind a new handle; the cache is
        // dropped at once so the view never shows the old file first.
        if let AppAction::TranscriptDiscarded { id, .. } | AppAction::UndoDiscard(id) = &action {
            self.conversations.remove(*id);
            file_refs::keep_only(&mut self.ui_state.file_links, *id, None);
        }
        let handles_before = self.resume_handles(&action);
        let effects = self.core.dispatch(action, now);
        for (id, before) in handles_before {
            if self.core.session(id).map(|s| s.resume.clone()) != Some(before) {
                self.conversations.remove(id);
                file_refs::keep_only(&mut self.ui_state.file_links, id, None);
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

    /// Type a message from one of the owner's boxes and settle that box's
    /// draft by the outcome. The box keeps its draft until the pane has
    /// the text, so a dead session or a failed write leaves it to resend.
    fn send_message(
        &mut self,
        id: RecordId,
        host: &HostId,
        text: String,
        from: Composer,
    ) -> Option<AppAction> {
        let result = self.services.host.write_line(host, &text);
        match (from, result.is_ok()) {
            // Only while the draft is still this message: the owner may
            // have typed on since it was sent.
            (Composer::Line, true) => {
                if self.ui_state.input_drafts.get(&id) == Some(&text) {
                    self.ui_state.input_drafts.remove(&id);
                }
            }
            (Composer::Line, false) | (Composer::Editor, true) => {}
            // The editor emptied itself when it queued the text, so a
            // failed write hands the text back for its next draw.
            (Composer::Editor, false) => {
                self.ui_state.primed.insert(id, text);
            }
        }
        failed(result, || format!("send a message to {}", host.0))
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
            _ => unreachable!("not a store effect"),
        }
    }

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
            Effect::CloneTranscriptInto {
                target,
                handle,
                prompt,
            } => AppAction::TranscriptClonedInto {
                target,
                prompt,
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

    /// Performs one effect; returns the action reporting its result, or
    /// `None` when the result arrives later (discovery) or has no report.
    fn run_effect(&mut self, effect: Effect) -> Option<AppAction> {
        let s = &self.services;
        match effect {
            Effect::SaveSettings(_)
            | Effect::SaveViews(_)
            | Effect::Save(_)
            | Effect::Delete(_)
            | Effect::StoreSecret { .. }
            | Effect::DeleteSecret(_) => {
                // Any of these may have moved a session's secret set.
                self.policies.clear();
                self.watch_dirty = true;
                self.run_store_effect(effect)
            }
            Effect::SaveIdentity(_) | Effect::SaveDevices(_) => self.run_remote_effect(effect),
            Effect::PrepareLaunch { .. }
            | Effect::PrepareResume { .. }
            | Effect::CheckTranscript { .. }
            | Effect::CloneTranscript { .. }
            | Effect::CloneAllTranscript { .. }
            | Effect::CloneTranscriptInto { .. }
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
            Effect::Spawn { id, spec } => Some(self.spawn(id, spec)),
            Effect::Attach {
                id,
                host,
                title,
                cwd,
            } => Some(AppAction::Attached {
                id,
                result: self.attach(&host, &title, &cwd),
            }),
            Effect::SendInput { .. }
            | Effect::SendMessage { .. }
            | Effect::SendAnswer { .. }
            | Effect::SendKeys { .. } => self.run_pane_write(effect),
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
                self.dispatch_port
                    .call(body)
                    .map(|(body, result)| AppAction::DispatchReplied {
                        body,
                        result: result.map_err(|e| e.to_string()),
                    })
            }
            Effect::ReadFileDiff { .. } => self.read_file_diff(effect),
            // Windows are the UI's; `logic` raises the one asked for.
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

    /// The writes into a pane, kept out of `run_effect` for its length.
    fn run_pane_write(&mut self, effect: Effect) -> Option<AppAction> {
        let host_port = &self.services.host;
        match effect {
            Effect::SendInput { host, text } => failed(host_port.write_line(&host, &text), || {
                format!("send input to {}", host.0)
            }),
            Effect::SendMessage {
                id,
                host,
                text,
                from,
            } => self.send_message(id, &host, text, from),
            Effect::SendAnswer { host, text } => failed(host_port.write_line(&host, &text), || {
                format!("send an answer to {}", host.0)
            }),
            Effect::SendKeys { host, bytes } => failed(host_port.write(&host, &bytes), || {
                format!("send keys to {}", host.0)
            }),
            _ => unreachable!("not a pane write"),
        }
    }

    /// The effects on run logs and artifacts, kept out of `run_effect`
    /// for length.
    fn run_log_effect(&mut self, effect: Effect) -> Option<AppAction> {
        match effect {
            Effect::Forget(host) => {
                // The record's own log and every run's (`<host>-r<n>.vt`).
                let dir = scrollback_dir(&self.services.store.data_dir());
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
                remove_quietly(&scrollback_dir(&self.services.store.data_dir()).join(name));
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
        if let Some(last) = caption_of(&text) {
            self.ui_state.captions.insert(id, last);
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
            let alive = self.core.is_running(d.id);
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
        if self.conversations.is_current(id, &handle, modified) {
            return;
        }
        match self.services.transcripts.read(&handle) {
            Ok(conversation) => {
                file_refs::keep_only(&mut self.ui_state.file_links, id, Some(&conversation));
                self.conversations
                    .insert(id, handle, modified, conversation);
            }
            Err(e) => {
                file_refs::keep_only(&mut self.ui_state.file_links, id, None);
                self.conversations.fail(id, e);
            }
        }
    }

    /// Start a record's pane with its environment and a fresh launch
    /// token, whose hash is on disk before the pane exists: a surviving
    /// pane still resolves after a restart, and a dead pane's token
    /// stops working at the next spawn.
    fn spawn(&mut self, id: RecordId, mut spec: SpawnSpec) -> AppAction {
        spec.scrollback = Some(self.log_path(id, &spec.id));
        spec.env = self.spawn_env(id, spec.env);
        // Two v4 uuids: 244 random bits without another crate.
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        self.dispatch_inner(AppAction::RecordTokenIssued {
            id,
            hash: token_hash(&token),
        });
        // Only the fresh token: never one a launcher or a respawn carried.
        spec.env.retain(|(k, _)| k != RECORD_TOKEN_ENV);
        spec.env.push((RECORD_TOKEN_ENV.to_owned(), token));
        let result = self.services.host.spawn(&spec).map_err(|e| e.to_string());
        if let Err(e) = &result {
            log::error!("spawn {} failed: {e}", spec.id.0);
        }
        AppAction::Spawned { id, result }
    }

    /// The project's environment, then the variables an outside
    /// launcher put on the record, then the launcher's own, so a later
    /// value wins. The record's come back on every spawn, so a resumed
    /// session keeps them.
    fn spawn_env(&self, id: RecordId, own: Vec<(String, String)>) -> Vec<(String, String)> {
        let data_dir = (
            "SWITCHBOARD_DATA_DIR".to_owned(),
            self.services
                .store
                .data_dir()
                .to_string_lossy()
                .into_owned(),
        );
        let Some(record) = self.core.session(id) else {
            let mut env = vec![data_dir];
            env.extend(own);
            return env;
        };
        let resolved = resolve_project_env(&self.core, &self.services, record.project);
        let missing = resolved.missing();
        if !missing.is_empty() {
            log::warn!("secrets without a stored value: {}", missing.join(", "));
        }
        // Every kind gets the data dir, so `switchboard-env` in a shell
        // asks the app that launched it; an agent's own says the same.
        let mut env = vec![data_dir];
        env.extend(resolved.pairs());
        log::debug!(
            "injecting {:?} into {}",
            env.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            id.host_name()
        );
        env.extend(record.env.iter().cloned());
        env.extend(own);
        env
    }

    /// Where the host pipes a session's raw output.
    fn scrollback_path(&self, host: &HostId) -> PathBuf {
        scrollback_dir(&self.services.store.data_dir()).join(format!("{}.vt", host.0))
    }

    /// Where a record's output goes and is read from: its latest run's
    /// log when it has runs, else the one file named after the host.
    fn log_path(&self, id: RecordId, host: &HostId) -> PathBuf {
        match self.core.session(id).and_then(|s| s.runs.last()) {
            Some(run) => scrollback_dir(&self.services.store.data_dir()).join(&run.log),
            None => self.scrollback_path(host),
        }
    }

    /// Claude Code asks whether to trust a folder before any hook can
    /// run, so a pane that has reported nothing is read for that
    /// question and the core told when it appears or goes away.
    fn check_trust_prompts(&mut self) {
        let candidates: Vec<RecordId> = self
            .core
            .all_sessions_sorted()
            .iter()
            .filter(|s| matches!(s.kind, SessionKind::Agent(AgentKind::ClaudeCode)))
            .filter(|s| s.activity == Activity::Unknown && self.core.is_running(s.id))
            .map(|s| s.id)
            .collect();
        for id in candidates {
            let host = HostId(id.host_name());
            let Ok(text) = self.services.host.snapshot(&host, Some(TRUST_PROMPT_LINES)) else {
                continue;
            };
            let seen = text.contains(TRUST_PROMPT_MARK);
            if seen != self.core.at_trust_prompt(id) {
                self.dispatch(AppAction::PromptSeen { id, seen });
            }
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
            let running = self.core.is_running(id);
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
                if let Some(last) = caption_of(&text) {
                    self.ui_state.captions.insert(id, last);
                }
            }
        }
    }

    /// One full poll right now, ignoring the timers: events, host list,
    /// Codex discoveries, captions. For tests and the startup script,
    /// which drive the app without a frame loop.
    pub fn poll_now(&mut self) {
        self.last_poll = Some(Instant::now());
        self.last_caption = Some(Instant::now());
        self.poll_events();
        self.poll_host();
        self.poll_discoveries();
        self.poll_configs();
        self.refresh_captions();
        self.check_trust_prompts();
    }

    fn pump(&mut self) {
        self.serve_pending();
        // Every `logic` call, not every poll: a button press should land
        // in the pass its wake-up requested.
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
            self.check_trust_prompts();
        }
        if self
            .last_config
            .is_none_or(|t| t.elapsed() >= CONFIG_INTERVAL)
        {
            self.last_config = Some(Instant::now());
            self.poll_configs();
        }
        self.drain_dispatch();
        if self
            .last_dispatch
            .is_none_or(|t| t.elapsed() >= DISPATCH_POLL)
        {
            self.last_dispatch = Some(Instant::now());
            self.poll_dispatch();
        }
        self.diff_pass();
    }

    /// Ask Dispatch's port for its status. No runner is a quick error
    /// (no socket, or nobody listening), reported to the core as such.
    /// Whether the answer came back at once (from a port that does not
    /// block).
    fn poll_dispatch(&mut self) -> bool {
        let Some((_, result)) = self.dispatch_port.poll_status() else {
            return false;
        };
        self.dispatch(AppAction::DispatchStatus(status_of(result)));
        true
    }

    /// Ask Dispatch for its status and wait up to `timeout` for the
    /// answer, for a script line that needs the tickets before the
    /// first frame (a frame never waits: it hands the poll to the port's
    /// thread and reads the answer on a later frame). Whether a status
    /// arrived.
    pub fn await_dispatch_status(&mut self, timeout: Duration) -> bool {
        self.last_dispatch = Some(Instant::now());
        if self.poll_dispatch() {
            return true;
        }
        let deadline = Instant::now() + timeout;
        loop {
            // Other calls' replies may finish first; each is delivered
            // as it would be on a frame.
            let left = deadline.saturating_duration_since(Instant::now());
            let done = self.dispatch_port.wait(left);
            let timed_out = done.is_empty();
            let answered = done.iter().any(|(body, _)| matches!(body, Body::Status));
            self.deliver_dispatch(done);
            if answered || timed_out {
                return answered;
            }
        }
    }

    /// Replies the port's thread finished since the last frame, each
    /// one an action.
    fn drain_dispatch(&mut self) {
        let done = self.dispatch_port.drain();
        self.deliver_dispatch(done);
    }

    /// Read one file's diff on a thread of its own; `drain_diffs` takes
    /// the answer.
    fn read_file_diff(&mut self, effect: Effect) -> Option<AppAction> {
        let Effect::ReadFileDiff {
            ticket,
            lane,
            path,
            old_path,
            range,
        } = effect
        else {
            unreachable!("not a diff read")
        };
        // An `Arc` clone is a second handle to the same reader, which the
        // thread owns while it runs.
        let reader = std::sync::Arc::clone(&self.services.changes);
        let tx = self.diff_reads.tx.clone();
        self.diff_reads.out += 1;
        std::thread::spawn(move || {
            // A panic still sends an answer, or the core would wait on
            // this read for the life of the app. `AssertUnwindSafe` says
            // the borrowed values are fine to use after a panic: the
            // closure only reads `range` and `path`, and the reader goes
            // with the thread.
            let read = std::panic::AssertUnwindSafe(|| {
                reader.diff(
                    &range.dir,
                    &range.base,
                    &range.head,
                    &path,
                    old_path.as_deref(),
                )
            });
            let result = std::panic::catch_unwind(read)
                .unwrap_or_else(|_| Err("the diff read panicked".to_owned()));
            let _ = tx.send(DiffDone {
                ticket,
                lane,
                path,
                range,
                result,
            });
        });
        None
    }

    /// File diffs whose threads answered since the last frame, each one
    /// an action.
    fn drain_diffs(&mut self) {
        let done: Vec<DiffDone> = self.diff_reads.rx.try_iter().collect();
        for DiffDone {
            ticket,
            lane,
            path,
            range,
            result,
        } in done
        {
            self.diff_reads.out = self.diff_reads.out.saturating_sub(1);
            self.dispatch(AppAction::FileDiffRead {
                ticket,
                lane,
                path,
                range,
                result,
            });
        }
    }

    fn deliver_dispatch(&mut self, done: Vec<DispatchDone>) {
        for (body, result) in done {
            if matches!(body, Body::Status) {
                self.dispatch(AppAction::DispatchStatus(status_of(result)));
            } else {
                self.dispatch(AppAction::DispatchReplied {
                    body,
                    result: result.map_err(|e| e.to_string()),
                });
            }
        }
    }
}

impl eframe::App for SwitchboardApp {
    /// eframe 0.36 skips `ui` while the root window is occluded or
    /// minimized and calls only this, so anything that must keep ticking
    /// (the control port, hook wakes, polls, the tick's own re-arm, the
    /// window raises they ask for) lives here rather than in `ui`.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.dispatch(AppAction::Tick);
        self.drain_diffs();
        if self.diff_reads.out > 0 {
            // The read threads have no context to wake the frame with.
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        if self.last_poll.is_some() {
            // Only after `start`: UI tests never call it, so they never
            // poll or badge.
            self.pump();
            ctx.request_repaint_after(POLL_INTERVAL);
            let waiting = self.core.waiting_count();
            if self.badge != Some(waiting) {
                crate::adapters::dock::set_waiting_badge(waiting);
                self.services.controller.set_waiting(waiting);
                self.badge = Some(waiting);
            }
        }
        crate::ui::popout::raise(
            ctx,
            &self.core.settings().popouts,
            std::mem::take(&mut self.ui_state.focus_windows),
        );
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
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
        if let Some((frame, _)) = self.ui_state.dispatch_frame.take()
            && self
                .core
                .settings()
                .dispatch_window
                .as_ref()
                .is_some_and(|w| w.frame.as_ref() != Some(&frame))
        {
            self.dispatch(AppAction::DispatchWindowMoved(frame));
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

    /// Answer every request that has arrived. Called from `logic`, which
    /// runs before every frame and while the window is hidden; a test
    /// calls it while its client waits. The remote port is drained at
    /// most `DRAIN_PER_PASS` requests per pass, in arrival order.
    pub fn serve_pending(&mut self) {
        self.diff_pass();
        let incoming: Vec<Incoming> = self
            .control
            .as_mut()
            .map(ControlSocket::poll)
            .unwrap_or_default();
        for Incoming { request, reply } in incoming {
            let answer = self.serve(&request);
            let _ = reply.send(answer);
        }
        let incoming: Vec<RemoteIncoming> = self
            .remote
            .as_mut()
            .map(|port| port.poll(remote::DRAIN_PER_PASS))
            .unwrap_or_default();
        for RemoteIncoming {
            request,
            reply,
            subscriber,
        } in incoming
        {
            if let Served::Now(answer) = self.serve_remote(&request, subscriber, &reply) {
                let _ = reply.try_send(answer);
            }
        }
        let frames: Vec<SubscriberFrame> = self
            .remote
            .as_mut()
            .map(RemotePort::poll_frames)
            .unwrap_or_default();
        for frame in frames {
            self.subscriber_frame(&frame);
        }
        let observed: Vec<Observed> = self
            .observed
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for observation in observed {
            self.observe(observation);
        }
        self.sweep_watches();
    }

    // --- content: the worker, the versions, the watch set

    /// The content worker's report on one read: a moved source or
    /// policy takes a new revision and emits `changed`; the reply, if
    /// one waits, is stamped with the key's revision, so two reads of
    /// unchanged content carry the same one. A key whose record is gone
    /// changes nothing; the reply still carries what was read.
    fn observe(&mut self, observation: Observed) {
        let Observed {
            key,
            policy,
            result,
            reply,
        } = observation;
        match result {
            Ok((source, body)) => {
                if self.core.session(key.record()).is_some() {
                    let moved = self
                        .versions
                        .get(&key)
                        .is_none_or(|v| v.source != source || v.policy != policy);
                    if moved {
                        self.revision += 1;
                        self.versions.insert(
                            key.clone(),
                            ContentVersion {
                                source,
                                policy,
                                revision: self.revision,
                            },
                        );
                        let text =
                            self.event_json("changed", Some((key.kind(), &key.id())), now_ms());
                        self.broadcast(&text);
                    }
                }
                if let Some(reply) = reply {
                    let revision = self
                        .versions
                        .get(&key)
                        .map_or(self.revision, |v| v.revision);
                    let out = self.envelope_at(revision, &body);
                    let out = if out.body.len() > BODY_CAP {
                        log::warn!("remote: {} {} is over the body cap", key.kind(), key.id());
                        RemoteReply {
                            status: 500,
                            body: self.error_json("too_large", "the reply is over the body cap"),
                            authenticated: true,
                        }
                    } else {
                        out
                    };
                    let _ = reply.try_send(out);
                }
            }
            Err(reason) => {
                if let Some(reply) = reply {
                    log::info!("remote: {} {} refused: failed", key.kind(), key.id());
                    let _ = reply.try_send(RemoteReply {
                        status: 400,
                        body: self.error_json("failed", &reason),
                        authenticated: true,
                    });
                }
            }
        }
    }

    /// A frame from a subscribed device: `watch` puts the key in the
    /// set and is answered `watching` once it is there; `unwatch` takes
    /// the device off it; the socket's end takes it off every key. A
    /// watch is never refused.
    fn subscriber_frame(&mut self, frame: &SubscriberFrame) {
        let Some(text) = &frame.text else {
            self.subscribers.retain(|s| s.id != frame.subscriber);
            self.forget_subscriber(frame.subscriber);
            return;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
            return;
        };
        let kind = value["resource"]["kind"].as_str().unwrap_or("");
        let id = value["resource"]["id"].as_str().unwrap_or("");
        let Some(key) = ContentKey::parse(kind, id) else {
            log::info!("remote: a frame named {kind:?}, which this host does not serve");
            return;
        };
        match value["type"].as_str() {
            Some("watch") => {
                self.watches
                    .entry(key.clone())
                    .or_insert_with(|| Watch {
                        last_read: None,
                        subscribers: std::collections::HashSet::new(),
                    })
                    .subscribers
                    .insert(frame.subscriber);
                self.watch_dirty = true;
                let text = self.event_json("watching", Some((key.kind(), &key.id())), now_ms());
                if let Some(s) = self.subscribers.iter().find(|s| s.id == frame.subscriber) {
                    let _ = s.queue.tx.try_send(text);
                }
            }
            Some("unwatch") => {
                if let Some(w) = self.watches.get_mut(&key) {
                    w.subscribers.remove(&frame.subscriber);
                    self.watch_dirty = true;
                }
            }
            _ => {}
        }
    }

    /// A subscriber is gone: its watches go with it.
    fn forget_subscriber(&mut self, id: u64) {
        for w in self.watches.values_mut() {
            if w.subscribers.remove(&id) {
                self.watch_dirty = true;
            }
        }
    }

    /// A content read of `key` keeps it in the set for a minute.
    fn touch_watch(&mut self, key: &ContentKey) {
        let now = Instant::now();
        if let Some(w) = self.watches.get_mut(key) {
            w.last_read = Some(now);
        } else {
            self.watches.insert(
                key.clone(),
                Watch {
                    last_read: Some(now),
                    subscribers: std::collections::HashSet::new(),
                },
            );
            self.watch_dirty = true;
        }
    }

    /// Keep the watch set within its bounds and hand the worker what
    /// to poll: a key with no subscriber and no read for a minute goes,
    /// past the cap the least recently read goes, and only a running
    /// pane is polled. Taken when the set or a policy moved, and every
    /// few seconds for the idle rule.
    fn sweep_watches(&mut self) {
        let due = self
            .last_watch_sweep
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(5));
        if !self.watch_dirty && !due {
            return;
        }
        self.watch_dirty = false;
        self.last_watch_sweep = Some(Instant::now());
        let now = Instant::now();
        self.watches.retain(|_, w| {
            !w.subscribers.is_empty() || w.last_read.is_some_and(|t| now - t < WATCH_IDLE)
        });
        while self.watches.len() > WATCH_CAP {
            let Some(oldest) = self
                .watches
                .iter()
                .min_by_key(|(_, w)| w.last_read)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            self.watches.remove(&oldest);
        }
        let keys: Vec<ContentKey> = self.watches.keys().cloned().collect();
        let mut jobs = Vec::new();
        for key in keys {
            let view = match key {
                ContentKey::Screen(_) => ContentView::Screen {
                    lines: SCREEN_LINES_CAP,
                },
                ContentKey::Conversation(_) => ContentView::LastResponse,
                ContentKey::RunOutput(..) => ContentView::Tail {
                    lines: RUN_LINES_CAP,
                },
            };
            if let Ok(job) = self.content_job(&key, view, None) {
                jobs.push(job);
            }
        }
        if let Some(content) = &self.content {
            content.set_watch(jobs);
        }
    }

    /// The job that reads `key`: its source resolved from the record
    /// (a running pane, a Claude Code transcript, a run's log) and the
    /// session's policy. The error is a refusal's code and reason.
    fn content_job(
        &mut self,
        key: &ContentKey,
        view: ContentView,
        reply: Option<mpsc::SyncSender<RemoteReply>>,
    ) -> Result<ContentJob, (u16, &'static str, String)> {
        let id = key.record();
        let Some(record) = self.core.session(id) else {
            return Err((404, "not_found", "no such session".into()));
        };
        let source = match key {
            ContentKey::Screen(_) => {
                if !self.core.is_running(id) {
                    return Err((400, "failed", "not running".into()));
                }
                Source::Pane(HostId(id.host_name()))
            }
            ContentKey::Conversation(_) => {
                if record.kind != SessionKind::Agent(AgentKind::ClaudeCode) {
                    return Err((
                        400,
                        "no_conversation",
                        "only a Claude Code session has a conversation".into(),
                    ));
                }
                match &record.resume {
                    Some(handle @ ResumeHandle::ClaudeCode { .. }) => {
                        Source::Transcript(handle.clone())
                    }
                    _ => {
                        return Err((
                            400,
                            "no_conversation",
                            "the session has not started a conversation yet".into(),
                        ));
                    }
                }
            }
            ContentKey::RunOutput(_, n) => {
                let Some(run) = record.runs.iter().find(|r| r.n == *n) else {
                    return Err((
                        404,
                        "no_run",
                        "no such run: it was never made, or its log was pruned".into(),
                    ));
                };
                Source::Log(scrollback_dir(&self.services.store.data_dir()).join(&run.log))
            }
        };
        let policy = self.policy_for(id).map_err(|reason| {
            log::warn!(
                "remote: {} for {}: policy: {reason}",
                key.kind(),
                id.host_name()
            );
            (
                503,
                "policy",
                "the secret set could not be resolved; nothing is sent unredacted".into(),
            )
        })?;
        Ok(ContentJob {
            key: key.clone(),
            source,
            view,
            policy,
            reply,
        })
    }

    /// A session's redaction policy: the project's secret variables and
    /// its granted sets' secrets, with the fingerprint that names it. A
    /// secret store that fails is an error, never an unmasked text.
    fn resolve_policy(&self, id: RecordId) -> Result<Policy, String> {
        let Some(record) = self.core.session(id) else {
            return Err("no such session".into());
        };
        let mut secrets: Vec<(String, String)> =
            resolve_project_env(&self.core, &self.services, record.project)
                .vars
                .into_iter()
                .filter(|v| v.secret)
                .filter_map(|v| v.value.filter(|x| !x.is_empty()).map(|x| (v.name, x)))
                .collect();
        // A child of `switchboard-env exec` may print a set's secret too.
        let resolved = self.resolve_record_sets(record)?;
        let secret_names = self.set_secret_names();
        secrets.extend(
            resolved
                .pairs
                .into_iter()
                .filter(|(name, value)| !value.is_empty() && secret_names.contains(name)),
        );
        Ok(Policy::new(secrets))
    }

    /// `resolve_policy`, cached until a save or a secret change.
    fn policy_for(&mut self, id: RecordId) -> Result<Policy, String> {
        if let Some(p) = self.policies.get(&id) {
            return Ok(p.clone());
        }
        let policy = self.resolve_policy(id)?;
        self.policies.insert(id, policy.clone());
        Ok(policy)
    }

    /// The content reads by path: the screen, the conversation page or
    /// the last response, and a run's output, each a key and a view
    /// for the worker.
    fn remote_content_route(
        &mut self,
        request: &RemoteRequest,
        segments: &[&str],
        reply: &mpsc::SyncSender<RemoteReply>,
    ) -> Served {
        let id = match parse_uuid("session", segments[1]) {
            Ok(id) => RecordId(id),
            Err(reason) => return Served::Now(self.refuse(request, 400, "failed", &reason, true)),
        };
        let (key, view) = match segments {
            [_, _, "screen"] => (
                ContentKey::Screen(id),
                ContentView::Screen {
                    lines: Self::lines_param(request, 40, SCREEN_LINES_CAP),
                },
            ),
            [_, _, "last-response"] => (ContentKey::Conversation(id), ContentView::LastResponse),
            [_, _, "conversation", n] => match n.parse::<usize>() {
                Ok(n) if n >= 1 => (ContentKey::Conversation(id), ContentView::Turn { n }),
                _ => {
                    return Served::Now(self.refuse(
                        request,
                        400,
                        "bad_request",
                        "the turn is a number",
                        true,
                    ));
                }
            },
            [_, _, "conversation"] => {
                let from = match (request.param("from"), request.param("before")) {
                    (None, None) => PageFrom::Turn(1),
                    (Some("last"), None) => PageFrom::Last,
                    (Some(n), None) => match n.parse::<usize>() {
                        Ok(n) if n >= 1 => PageFrom::Turn(n),
                        _ => {
                            return Served::Now(self.refuse(
                                request,
                                400,
                                "bad_request",
                                "from is a turn number or last",
                                true,
                            ));
                        }
                    },
                    (None, Some(n)) => match n.parse::<usize>() {
                        Ok(n) if n >= 1 => PageFrom::Before(n),
                        _ => {
                            return Served::Now(self.refuse(
                                request,
                                400,
                                "bad_request",
                                "before is a turn number",
                                true,
                            ));
                        }
                    },
                    (Some(_), Some(_)) => {
                        return Served::Now(self.refuse(
                            request,
                            400,
                            "bad_request",
                            "from and before do not go together",
                            true,
                        ));
                    }
                };
                let limit = request
                    .param("limit")
                    .and_then(|n| n.parse::<usize>().ok())
                    .unwrap_or(PAGE_TURNS_DEFAULT)
                    .clamp(1, PAGE_TURNS_CAP);
                (
                    ContentKey::Conversation(id),
                    ContentView::Page { from, limit },
                )
            }
            [_, _, "runs", n, "output"] => {
                let Ok(n) = n.parse::<u32>() else {
                    return Served::Now(self.refuse(
                        request,
                        400,
                        "bad_request",
                        "the run is a number",
                        true,
                    ));
                };
                (
                    ContentKey::RunOutput(id, n),
                    ContentView::Tail {
                        lines: Self::lines_param(request, 200, RUN_LINES_CAP),
                    },
                )
            }
            _ => {
                return Served::Now(self.refuse(
                    request,
                    404,
                    "not_found",
                    "no such endpoint",
                    true,
                ));
            }
        };
        self.remote_content(request, &key, view, reply)
    }

    /// A device's content read: the frame checks the record and
    /// resolves the policy, the worker reads and masks, and the reply
    /// goes out on the connection's channel once the frame has stamped
    /// it.
    fn remote_content(
        &mut self,
        request: &RemoteRequest,
        key: &ContentKey,
        view: ContentView,
        reply: &mpsc::SyncSender<RemoteReply>,
    ) -> Served {
        let job = match self.content_job(key, view, Some(reply.clone())) {
            Ok(job) => job,
            Err((status, code, reason)) => {
                return Served::Now(self.refuse(request, status, code, &reason, true));
            }
        };
        let Some(content) = &self.content else {
            return Served::Now(self.refuse(request, 503, "busy", "no content worker", true));
        };
        if !content.submit(job) {
            return Served::Now(self.refuse(
                request,
                503,
                "busy",
                "the content worker is behind; try again",
                true,
            ));
        }
        self.touch_watch(key);
        Served::Deferred
    }

    /// The `lines` a read asks for, within `cap`.
    fn lines_param(request: &RemoteRequest, default: usize, cap: usize) -> usize {
        request
            .param("lines")
            .and_then(|n| n.parse::<usize>().ok())
            .unwrap_or(default)
            .clamp(1, cap)
    }

    /// A command's or service's runs as the record holds them: the
    /// numbers, times, exits and the artifacts' names, never paths.
    fn remote_runs(&self, request: &RemoteRequest, session: &str) -> RemoteReply {
        let record = parse_uuid("session", session)
            .ok()
            .and_then(|id| self.core.session(RecordId(id)));
        let Some(record) = record else {
            return self.refuse(request, 404, "not_found", "no such session", true);
        };
        let runs: Vec<serde_json::Value> = record
            .runs
            .iter()
            .map(|r| {
                serde_json::json!({
                    "n": r.n,
                    "started_ms": epoch_millis(r.started),
                    "ended_ms": r.ended.map(epoch_millis),
                    "exit": r.exit,
                    "artifacts": r
                        .artifacts
                        .iter()
                        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        let body = serde_json::json!({ "runs": runs });
        self.envelope(200, &body, true)
    }

    /// How often the worker polls the watched keys; tests shorten it.
    pub fn set_watch_poll(&self, every: Duration) {
        if let Some(content) = &self.content {
            content.set_poll(every);
        }
    }

    /// The keys in the watch set now.
    #[must_use]
    pub fn watched_keys(&self) -> Vec<ContentKey> {
        self.watches.keys().cloned().collect()
    }

    /// The revision a content key was last served at.
    #[must_use]
    pub fn content_revision(&self, key: &ContentKey) -> Option<u64> {
        self.versions.get(key).map(|v| v.revision)
    }

    // --- the revision and its events

    /// Fingerprint every read model and compare with the last pass:
    /// one `changed` or `removed` per resource that moved, the
    /// revision bumped once if anything did. Taken only when a
    /// dispatch happened since the last pass, and only while the port
    /// is bound, since nothing else reads the revision.
    fn diff_pass(&mut self) {
        if !self.dirty || self.remote.is_none() {
            return;
        }
        self.dirty = false;
        let next = Fingerprints::of(&self.core, SystemTime::now());
        let Some(previous) = self.fingerprints.replace(next) else {
            return;
        };
        let changes = previous.changes(self.fingerprints.as_ref().unwrap_or(&previous));
        if changes.is_empty() {
            return;
        }
        self.revision += 1;
        let at_ms = epoch_millis(SystemTime::now());
        for change in changes {
            let kind = if change.removed { "removed" } else { "changed" };
            let text = self.event_json(
                kind,
                Some((change.resource.kind.word(), &change.resource.id)),
                at_ms,
            );
            self.broadcast(&text);
        }
    }

    /// One event frame as the subscribers get it.
    fn event_json(&self, kind: &str, resource: Option<(&str, &str)>, at_ms: u64) -> String {
        let server_id = self.core.identity().map(|i| i.server_id.to_string());
        let mut value = serde_json::json!({
            "server_id": server_id,
            "epoch": self.epoch,
            "revision": self.revision,
            "type": kind,
            "at_ms": at_ms,
        });
        if let Some((kind, id)) = resource {
            value["resource"] = serde_json::json!({ "kind": kind, "id": id });
        }
        value.to_string()
    }

    /// Hand one frame to every subscriber. A full queue is a subscriber
    /// that fell `SUBSCRIBER_QUEUE` events behind: it is dropped with
    /// its overflow flag set, never trimmed, so its connection sends
    /// `resync_required` and closes.
    fn broadcast(&mut self, text: &str) {
        self.subscribers
            .retain(|s| match s.queue.tx.try_send(text.to_owned()) {
                Ok(()) => true,
                Err(mpsc::TrySendError::Full(_)) => {
                    s.queue.overflow.store(true, Ordering::SeqCst);
                    log::warn!("remote: a subscriber fell behind and is dropped");
                    false
                }
                Err(mpsc::TrySendError::Disconnected(_)) => false,
            });
    }

    /// Accept a device's subscription: `hello` with the revision as it
    /// stands goes into the queue first, so a snapshot the device takes
    /// after it can be reconciled against everything that follows.
    fn remote_subscribe(
        &mut self,
        request: &RemoteRequest,
        device: uuid::Uuid,
        subscriber: Option<SubscriberQueue>,
    ) -> RemoteReply {
        let Some(queue) = subscriber.filter(|_| request.upgrade.is_some()) else {
            return self.refuse(
                request,
                400,
                "upgrade_required",
                "events are a WebSocket; upgrade the connection",
                true,
            );
        };
        let hello = self.event_json("hello", None, epoch_millis(SystemTime::now()));
        if queue.tx.try_send(hello).is_err() {
            return self.refuse(
                request,
                503,
                "busy",
                "the subscription could not start",
                true,
            );
        }
        self.subscribers.push(Subscriber {
            id: queue.id,
            queue,
            device,
        });
        RemoteReply {
            status: remote::SUBSCRIBED,
            body: Vec::new(),
            authenticated: true,
        }
    }

    /// How many devices are subscribed to events.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.subscribers.len()
    }

    /// The revision every reply carries now.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    // --- the remote port

    /// The identity at start: loaded from `identity.json` with its key
    /// from the secret store, or made now. A certificate whose key is
    /// gone (a Keychain reset, an unsigned build) is remade over a new
    /// key, which invalidates every pairing, so `devices.json` is
    /// cleared and the owner is told. A secret store that errors is
    /// not a missing key: the identity is kept and the port refuses to
    /// listen rather than throw the pairings away.
    fn load_identity(&mut self) {
        let loaded = match self.services.remote.load_identity() {
            Ok(loaded) => loaded,
            Err(e) => {
                log::error!("identity: {e}");
                self.dispatch_inner(AppAction::Failed(format!("read the host identity: {e}")));
                return;
            }
        };
        let mut devices = match self.services.remote.load_devices() {
            Ok(devices) => devices,
            Err(e) => {
                log::error!("paired devices: {e}");
                self.dispatch_inner(AppAction::Failed(format!("read the paired devices: {e}")));
                Vec::new()
            }
        };
        let (identity, rotated) = match loaded {
            Some(identity) => match self.services.secrets.get(&identity.key_account()) {
                Ok(Some(key))
                    if remote::spki_sha256_of_key(&key).ok().as_deref()
                        == Some(identity.spki_sha256.as_str()) =>
                {
                    (identity, false)
                }
                Ok(_) if self.core.read_only() => (identity, false),
                Ok(_) => {
                    log::warn!("the remote key is missing; making a new identity");
                    match self.make_identity(
                        Some(identity.server_id),
                        &identity.server_name,
                        identity.listen.clone(),
                    ) {
                        Some(fresh) => {
                            devices.clear();
                            if let Err(e) = self.services.remote.save_devices(&devices) {
                                log::error!("devices.json: {e}");
                            }
                            (fresh, true)
                        }
                        None => (identity, false),
                    }
                }
                Err(e) => {
                    log::error!("the remote key could not be read: {e}");
                    self.dispatch_inner(AppAction::Failed(format!(
                        "read the remote port's key: {e}; the port is not listening"
                    )));
                    (identity, false)
                }
            },
            None if self.core.read_only() => return,
            None => {
                let name = remote::default_server_name();
                match self.make_identity(None, &name, Listen::default()) {
                    Some(fresh) => {
                        if !devices.is_empty() {
                            devices.clear();
                            if let Err(e) = self.services.remote.save_devices(&devices) {
                                log::error!("devices.json: {e}");
                            }
                        }
                        (fresh, false)
                    }
                    None => return,
                }
            }
        };
        self.dispatch_inner(AppAction::IdentityLoaded {
            identity,
            devices,
            rotated,
        });
    }

    /// A new certificate and key, the key stored first so a crash
    /// between the two leaves an unused key rather than a certificate
    /// without one. `None` when either store refused, with a notice.
    fn make_identity(
        &mut self,
        server_id: Option<uuid::Uuid>,
        name: &str,
        listen: Listen,
    ) -> Option<crate::ports::remote::Identity> {
        let made = remote::make_identity(server_id, name, listen).and_then(|(identity, key)| {
            self.services
                .secrets
                .set(&identity.key_account(), &key)
                .map_err(|e| format!("store the key: {e}"))?;
            self.services
                .remote
                .save_identity(&identity)
                .map_err(|e| format!("write identity.json: {e}"))?;
            Ok(identity)
        });
        match made {
            Ok(identity) => {
                log::info!("remote identity {} made", identity.server_id);
                Some(identity)
            }
            Err(e) => {
                log::error!("remote identity: {e}");
                self.dispatch_inner(AppAction::Failed(format!("make the host identity: {e}")));
                None
            }
        }
    }

    /// The identity and device files; a changed address rebinds the port.
    fn run_remote_effect(&mut self, effect: Effect) -> Option<AppAction> {
        match effect {
            Effect::SaveIdentity(identity) => {
                let saved = self.services.remote.save_identity(&identity);
                if self.remote_wake.is_some()
                    && self.remote_bound.as_ref() != Some(&identity.listen)
                {
                    match self.bind_remote() {
                        Ok(addr) => log::info!("remote port at {addr}"),
                        Err(e) => {
                            return failed(Err::<(), _>(e), || "bind the remote port");
                        }
                    }
                } else {
                    // A rename alone: the record carries the name.
                    self.advertise_remote();
                }
                failed(saved, || "save the host identity")
            }
            Effect::SaveDevices(devices) => failed(
                self.services.remote.save_devices(&devices),
                || "save the paired devices",
            ),
            _ => None,
        }
    }

    /// A port in place of the identity's (`SWITCHBOARD_REMOTE_PORT`);
    /// `0` picks a free one, which is what tests use.
    pub fn set_remote_port_override(&mut self, port: Option<u16>) {
        self.remote_port_override = port;
    }

    /// Bind the remote port as the identity says: loopback until the
    /// owner allows the local network. `wake` runs on a connection
    /// thread whenever a request is queued. Refused for a read-only
    /// instance and for one whose key is unavailable.
    pub fn listen_remote(
        &mut self,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> io::Result<SocketAddr> {
        self.remote_wake = Some(Arc::new(wake));
        self.bind_remote()
    }

    fn bind_remote(&mut self) -> io::Result<SocketAddr> {
        self.remote = None;
        self.remote_bound = None;
        self.services.advertiser.withdraw();
        if self.core.read_only() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "another instance holds the data directory; not listening",
            ));
        }
        let identity = self
            .core
            .identity()
            .cloned()
            .ok_or_else(|| io::Error::other("no host identity"))?;
        let key = self
            .services
            .secrets
            .get(&identity.key_account())
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::other("the remote key is missing"))?;
        let tls = remote::tls_config(&identity, &key).map_err(io::Error::other)?;
        let wake = self
            .remote_wake
            .clone()
            .ok_or_else(|| io::Error::other("no wake"))?;
        let ip = if identity.listen.lan {
            IpAddr::from([0, 0, 0, 0])
        } else {
            IpAddr::from([127, 0, 0, 1])
        };
        let port = self.remote_port_override.unwrap_or(identity.listen.port);
        let waker = Arc::clone(&wake);
        let bound = RemotePort::bind(SocketAddr::new(ip, port), tls, move || waker())?;
        if self.content.is_none() {
            let (tx, rx) = mpsc::channel();
            self.content = Some(ContentWorker::spawn(
                Arc::clone(&self.services.host),
                Arc::clone(&self.services.transcripts),
                tx,
                wake,
            ));
            self.observed = Some(rx);
        }
        let addr = bound.addr();
        self.remote = Some(bound);
        self.remote_bound = Some(identity.listen);
        self.advertise_remote();
        Ok(addr)
    }

    /// Advertise the port over Bonjour while it is bound to the local
    /// network, under the identity's current name; withdrawn otherwise.
    /// A refusal from the responder is logged and leaves the port
    /// reachable by address.
    fn advertise_remote(&mut self) {
        let Some(identity) = self.core.identity() else {
            return;
        };
        let on_lan = self.remote_bound.as_ref().is_some_and(|l| l.lan);
        match self.remote_addr() {
            Some(addr) if on_lan => {
                let ad = Advertisement::new(identity.server_id, &identity.server_name, addr.port());
                if let Err(e) = self.services.advertiser.advertise(&ad) {
                    log::warn!("bonjour: {e}");
                }
            }
            _ => self.services.advertiser.withdraw(),
        }
    }

    /// Where the remote port is bound, while it is.
    #[must_use]
    pub fn remote_addr(&self) -> Option<SocketAddr> {
        self.remote.as_ref().map(RemotePort::addr)
    }

    /// One HTTP request from a device, answered on the frame: `/info`
    /// and `/pair` to anyone, the allowlisted reads to a paired device,
    /// everything else refused before it is translated to the wire.
    fn serve_remote(
        &mut self,
        request: &RemoteRequest,
        subscriber: Option<SubscriberQueue>,
        reply: &mpsc::SyncSender<RemoteReply>,
    ) -> Served {
        let Some(rest) = request.path.strip_prefix("/api/v1/") else {
            return Served::Now(self.refuse(request, 404, "not_found", "no such endpoint", false));
        };
        let segments: Vec<&str> = rest.split('/').collect();
        match (request.method.as_str(), segments.as_slice()) {
            ("GET", ["info"]) => return Served::Now(self.remote_info()),
            ("POST", ["pair"]) => return Served::Now(self.remote_pair(request)),
            _ => {}
        }
        let Some(token) = &request.bearer else {
            return Served::Now(self.refuse(
                request,
                401,
                "unauthorized",
                "pair this device first",
                false,
            ));
        };
        let Some(device) = self
            .core
            .device_by_token(&remote::token_sha256(token))
            .cloned()
        else {
            return Served::Now(self.refuse(
                request,
                401,
                "unauthorized",
                "the token is not known; pair again",
                false,
            ));
        };
        self.dispatch_inner(AppAction::DeviceSeen(device.id));
        if let ("GET", ["events"]) = (request.method.as_str(), segments.as_slice()) {
            return Served::Now(self.remote_subscribe(request, device.id, subscriber));
        }
        let notes_cap = if matches!(segments.as_slice(), ["sessions", _]) {
            NOTES_CAP_ALONE
        } else {
            STRING_CAP
        };
        let body = match (request.method.as_str(), segments.as_slice()) {
            ("GET", ["projects"]) => wire::Body::Projects {
                space: request.param("space").map(str::to_owned),
            },
            ("GET", ["spaces"]) => wire::Body::Spaces,
            ("GET", ["spaces", space, "sets"]) => wire::Body::Sets {
                space: (*space).to_owned(),
            },
            ("GET", ["sets", set]) => return Served::Now(self.remote_set(request, set)),
            ("GET", ["projects", project, "sessions"]) => wire::Body::Sessions {
                project: (*project).to_owned(),
            },
            ("GET", ["sessions", session]) => wire::Body::Session {
                session: (*session).to_owned(),
            },
            ("GET", ["waiting"]) => wire::Body::Waiting,
            ("GET", ["ops", op]) => wire::Body::OpStatus {
                operation: (*op).to_owned(),
            },
            ("GET", ["sessions", session, "runs"]) => {
                return Served::Now(self.remote_runs(request, session));
            }
            ("POST", ["sessions", session, verb @ ("send" | "resume" | "trust")]) => {
                return Served::Now(self.remote_command(request, &device, session, verb));
            }
            (
                "GET",
                ["sessions", _, "screen" | "conversation" | "last-response"]
                | ["sessions", _, "conversation", _]
                | ["sessions", _, "runs", _, "output"],
            ) => {
                return self.remote_content_route(request, &segments, reply);
            }
            _ => {
                return Served::Now(self.refuse(
                    request,
                    404,
                    "not_found",
                    "not available on this port",
                    true,
                ));
            }
        };
        Served::Now(self.remote_reply(request, &self.answer(&body), notes_cap))
    }

    /// A command under `control` scope. The body's `op` is the operation
    /// id the device made, which is the idempotency key: the command
    /// goes through `serve` as the control socket's do, so a send or a
    /// resume is logged before it runs and a repeat of the op is
    /// answered from the log without running again (§4.5).
    fn remote_command(
        &mut self,
        request: &RemoteRequest,
        device: &PairedDevice,
        session: &str,
        verb: &str,
    ) -> RemoteReply {
        if device.scope != Scope::Control {
            return self.refuse(
                request,
                403,
                "scope",
                "this device reads only; raise its scope on the host",
                true,
            );
        }
        let Ok(body) = serde_json::from_slice::<serde_json::Value>(&request.body) else {
            return self.refuse(request, 400, "bad_request", "the body is not JSON", true);
        };
        let field = |name: &str| body.get(name).and_then(|v| v.as_str()).map(str::trim);
        let op = field("op").unwrap_or_default();
        if op.is_empty() || op.len() > OP_CAP {
            return self.refuse(
                request,
                400,
                "bad_request",
                "op is the operation id, 1 to 64 characters",
                true,
            );
        }
        let session = session.to_owned();
        let body = match verb {
            "send" => {
                let text = field("text").unwrap_or_default();
                if text.is_empty() || text.len() > SEND_BYTES_CAP {
                    return self.refuse(
                        request,
                        400,
                        "bad_request",
                        "text is the message, 1 byte to 16 KiB",
                        true,
                    );
                }
                wire::Body::SessionSend {
                    session,
                    text: text.to_owned(),
                }
            }
            "resume" => wire::Body::SessionResume { session },
            _ => wire::Body::SessionTrust { session },
        };
        let reply = self.serve(&wire::Request::new(op, body));
        if let wire::Reply::Failed { reason } = &reply
            && reason.starts_with("interrupted")
        {
            return self.refuse(request, 409, "interrupted", reason, true);
        }
        self.remote_reply(request, &reply, STRING_CAP)
    }

    /// Who this host is, to anyone: nothing in it is a secret and
    /// nothing in it is trusted until the key is pinned.
    fn remote_info(&self) -> RemoteReply {
        let Some(identity) = self.core.identity() else {
            return RemoteReply {
                status: 503,
                body: self.error_json("no_identity", "the host has no identity yet"),
                authenticated: false,
            };
        };
        let body = serde_json::json!({
            "server_id": identity.server_id,
            "server_name": identity.server_name,
            "protocol": REMOTE_PROTOCOL,
            "capabilities": ["read", "control"],
            "spki_sha256": identity.spki_sha256,
            "limits": {
                "body_bytes": BODY_CAP,
                "string_bytes": STRING_CAP,
                "notes_bytes": NOTES_CAP_ALONE,
                "screen_lines": SCREEN_LINES_CAP,
                "run_lines": RUN_LINES_CAP,
                "conversation_turns": PAGE_TURNS_CAP,
                "turn_bytes": TURN_BYTES_CAP,
                "activity_bytes": ACTIVITY_BYTES_CAP,
                "send_bytes": SEND_BYTES_CAP,
                "op_chars": OP_CAP,
            },
            "pairing_open": self.core.pairing_at(self.clock()).is_some(),
        });
        self.envelope(200, &body, false)
    }

    /// The pairing handshake: inside the window, with the host's own
    /// key hash (so a request relayed through a substituted
    /// certificate fails even with the code) and the code. A match
    /// makes a token the device keeps and the host stores only the hash
    /// of; a mismatch counts against the window.
    fn remote_pair(&mut self, request: &RemoteRequest) -> RemoteReply {
        let now = self.clock();
        let Some(window) = self.core.pairing_at(now).cloned() else {
            return self.refuse(
                request,
                403,
                "pairing_closed",
                "pairing is not open on the host",
                false,
            );
        };
        let Some(identity) = self.core.identity().cloned() else {
            return self.refuse(
                request,
                503,
                "no_identity",
                "the host has no identity",
                false,
            );
        };
        let Ok(body) = serde_json::from_slice::<serde_json::Value>(&request.body) else {
            return self.refuse(request, 400, "bad_request", "the body is not JSON", false);
        };
        let field = |name: &str| {
            body.get(name)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_owned()
        };
        if field("spki_sha256") != identity.spki_sha256 {
            self.dispatch_inner(AppAction::PairingFailed);
            return self.refuse(
                request,
                403,
                "wrong_key",
                "this is not the host whose code was scanned",
                false,
            );
        }
        if field("code").to_ascii_uppercase() != window.code {
            self.dispatch_inner(AppAction::PairingFailed);
            return self.refuse(request, 403, "wrong_code", "the code does not match", false);
        }
        let name = field("device_name");
        let name = if name.is_empty() {
            "Device"
        } else {
            name.as_str()
        };
        let token = remote::bearer_token();
        let device = self
            .core
            .new_device(name, remote::token_sha256(&token), now.wall);
        let reply = serde_json::json!({
            "token": token,
            "scope": device.scope,
            "server_id": identity.server_id,
            "device_id": device.id,
        });
        log::info!(
            "remote: paired {:?} with {} scope",
            device.name,
            device.scope.word()
        );
        self.dispatch_inner(AppAction::Paired(device));
        self.envelope(200, &reply, false)
    }

    /// One working set with its pins, found across every space.
    fn remote_set(&self, request: &RemoteRequest, set: &str) -> RemoteReply {
        let found = parse_uuid("set", set).ok().and_then(|id| {
            self.core
                .space_views()
                .iter()
                .filter_map(|space| uuid::Uuid::parse_str(&space.id).ok())
                .flat_map(|space| self.core.set_views(SpaceId(space)))
                .find(|view| view.id == id.to_string())
        });
        match found {
            Some(set) => {
                self.remote_reply(request, &wire::Reply::Sets { sets: vec![set] }, STRING_CAP)
            }
            None => self.refuse(request, 404, "not_found", "no such set", true),
        }
    }

    /// A wire reply as the HTTP body: the tag dropped, every string
    /// bounded with the cut ones named, the whole under the body cap.
    fn remote_reply(
        &self,
        request: &RemoteRequest,
        reply: &wire::Reply,
        notes_cap: usize,
    ) -> RemoteReply {
        if let wire::Reply::Failed { reason } = reply {
            let (status, code) = if reason.starts_with("no such") {
                (404, "not_found")
            } else {
                (400, "failed")
            };
            return self.refuse(request, status, code, reason, true);
        }
        let mut value = serde_json::to_value(reply).unwrap_or_default();
        if let serde_json::Value::Object(map) = &mut value {
            map.remove("reply");
        }
        bound_strings(&mut value, notes_cap);
        let out = self.envelope(200, &value, true);
        if out.body.len() > BODY_CAP {
            log::warn!(
                "remote: {} {} is {} bytes, over the cap",
                request.method,
                request.path,
                out.body.len()
            );
            return self.refuse(
                request,
                500,
                "too_large",
                "the reply is over the body cap",
                true,
            );
        }
        out
    }

    /// A content reply at the revision its key holds.
    fn envelope_at(&self, revision: u64, body: &serde_json::Value) -> RemoteReply {
        let text = serde_json::json!({
            "epoch": self.epoch,
            "revision": revision,
            "body": body,
        });
        RemoteReply {
            status: 200,
            body: text.to_string().into_bytes(),
            authenticated: true,
        }
    }

    fn envelope(&self, status: u16, body: &serde_json::Value, authenticated: bool) -> RemoteReply {
        let text = serde_json::json!({
            "epoch": self.epoch,
            "revision": self.revision,
            "body": body,
        });
        RemoteReply {
            status,
            body: text.to_string().into_bytes(),
            authenticated,
        }
    }

    fn error_json(&self, code: &str, message: &str) -> Vec<u8> {
        serde_json::json!({
            "epoch": self.epoch,
            "revision": self.revision,
            "error": { "code": code, "message": message },
        })
        .to_string()
        .into_bytes()
    }

    /// A refusal, logged by method, path and code only.
    fn refuse(
        &self,
        request: &RemoteRequest,
        status: u16,
        code: &str,
        message: &str,
        authenticated: bool,
    ) -> RemoteReply {
        log::info!(
            "remote: {} {} refused: {code}",
            request.method,
            request.path
        );
        RemoteReply {
            status,
            body: self.error_json(code, message),
            authenticated,
        }
    }

    /// One request, one reply. A query reads; a command runs through the
    /// core under its operation id, and its reply is logged so a repeat
    /// of the same id is answered from the log without running again.
    fn serve(&mut self, request: &wire::Request) -> wire::Reply {
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
        if request.body.class() == wire::Class::NonReplayable {
            // A repeat could spend money or type twice, so the request
            // line goes down before anything runs: an op with a request
            // line and no reply was started and is never run again.
            if self.requested(op) {
                return wire::Reply::failed(
                    "interrupted: the host started this operation and its outcome is unknown",
                );
            }
            let line = OpLine::Requested {
                op: op.clone(),
                kind: request.body.kind(),
                ids: Vec::new(),
                at: SystemTime::now(),
            };
            if let Err(e) = self.services.operations.append(&line) {
                log::error!("operations log: {e}");
                return wire::Reply::failed(format!("the operations log refused the request: {e}"));
            }
        }
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

    /// Whether the log holds a request line for `op`.
    fn requested(&self, op: &str) -> bool {
        self.services
            .operations
            .find(op)
            .iter()
            .any(|l| matches!(l, OpLine::Requested { .. }))
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
            wire::Body::SessionScreen { session, lines } => match parse_uuid("session", session) {
                Ok(id) => self.screen(RecordId(id), *lines),
                Err(reason) => wire::Reply::failed(reason),
            },
            wire::Body::EnvResolve { session, token } => match parse_uuid("session", session) {
                Ok(id) => self.env_resolve(RecordId(id), token),
                Err(reason) => wire::Reply::failed(reason),
            },
            wire::Body::EnvSets => wire::Reply::EnvSets {
                sets: core.env_set_views(),
            },
            _ => wire::Reply::failed("not a query"),
        }
    }

    /// The variables of a session's granted sets, for `switchboard-env`,
    /// answered only to the holder of the session's launch token. Never
    /// logged: queries reach no operations line, and the log lines here
    /// carry names only.
    fn env_resolve(&self, id: RecordId, token: &str) -> wire::Reply {
        let Some(record) = self.core.session(id) else {
            return wire::Reply::failed("no such session");
        };
        let Some(hash) = &record.token_hash else {
            return wire::Reply::failed("this session was launched before tokens; restart it");
        };
        if token.is_empty() || token_hash(token) != *hash {
            return wire::Reply::failed("token does not match");
        }
        match self.resolve_record_sets(record) {
            Ok(resolved) => {
                log::info!(
                    "env.resolve for {}: {} variables",
                    id.host_name(),
                    resolved.pairs.len()
                );
                wire::Reply::Env {
                    pairs: resolved.pairs,
                    aws: resolved.aws.as_ref().map(aws_view),
                    missing: resolved.missing,
                }
            }
            Err(reason) => wire::Reply::failed(reason),
        }
    }

    /// What the record's and its project's grants resolve to now: values
    /// are never cached, so a grant or a secret changed since launch is
    /// what the next `exec` gets.
    fn resolve_record_sets(&self, record: &SessionRecord) -> Result<SetsResolved, String> {
        let project = self
            .core
            .workspace(record.project)
            .map(|w| w.project.env_sets.as_slice())
            .unwrap_or_default();
        let lookup = |account: &str| self.services.secrets.get(account).ok().flatten();
        resolve_sets(
            &self.core.settings().env_sets,
            project,
            &record.env_sets,
            &lookup,
        )
    }

    /// The last lines of a running session's pane for `session.screen`,
    /// with the values of its project's secrets replaced by their names:
    /// the reply becomes another program's output, where a secret must
    /// never appear.
    fn screen(&self, id: RecordId, lines: Option<u32>) -> wire::Reply {
        if !self.core.is_running(id) {
            return wire::Reply::failed("not running");
        }
        let n = lines.unwrap_or(40).clamp(1, 200) as usize;
        // Lines above the ones returned, redacted with them, so a value
        // wrapped across the window's first line is still found whole.
        let text = match self
            .services
            .host
            .snapshot(&HostId(id.host_name()), Some(n + SCREEN_SLACK_LINES))
        {
            Ok(text) => text,
            Err(e) => return wire::Reply::failed(format!("snapshot: {e}")),
        };
        let policy = match self.resolve_policy(id) {
            Ok(policy) => policy,
            Err(reason) => {
                return wire::Reply::failed(format!(
                    "the secret set could not be resolved ({reason}); nothing is sent unredacted"
                ));
            }
        };
        let redacted = redact(&text, &policy.secrets);
        if redacted != text {
            log::debug!("session.screen for {}: secrets redacted", id.host_name());
        }
        let kept: Vec<&str> = redacted.trim_end().lines().collect();
        wire::Reply::Screen {
            text: kept[kept.len().saturating_sub(n)..].join("\n"),
        }
    }

    /// The names of every set variable marked secret.
    fn set_secret_names(&self) -> Vec<String> {
        self.core
            .settings()
            .env_sets
            .iter()
            .flat_map(|s| s.vars.iter().filter(|v| v.secret).map(|v| v.name.clone()))
            .collect()
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
        let replied = self.replied(op);
        let requested = self
            .services
            .operations
            .find(op)
            .iter()
            .any(|l| matches!(l, OpLine::Requested { .. }));
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

fn now_ms() -> u64 {
    epoch_millis(SystemTime::now())
}

/// Milliseconds since the epoch, the wire's time; zero before it.
fn epoch_millis(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// A card's caption: the last line of a pane with something on it.
fn caption_of(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_owned())
}

/// A status reply as the core takes it: a status, or `None` for no
/// runner (an error, or an answer that is not a status).
fn status_of(result: io::Result<Reply>) -> Option<Status> {
    match result {
        Ok(Reply::Status(status)) => Some(status),
        Ok(other) => {
            log::warn!("dispatch status answered {other:?}");
            None
        }
        Err(e) => {
            log::debug!("dispatch status: {e}");
            None
        }
    }
}

type DispatchDone = (Body, io::Result<Reply>);

/// The Dispatch port, called from a thread of its own when it may
/// block: a runner holds its writer lock while a ticket's step fetches
/// or launches, and a frame must not wait for that. Calls are answered
/// in order through `drain`. Only the real adapter blocks; a port that
/// says it cannot (`may_block`) is called in place, so a test with a
/// fake sees its reply on the same frame.
struct DispatchWorker {
    command: PathBuf,
    data_dir: PathBuf,
    inline: Option<Box<dyn DispatchPort>>,
    jobs: Option<std::sync::mpsc::Sender<Body>>,
    done: Option<std::sync::mpsc::Receiver<DispatchDone>>,
    /// A status poll is on the thread; the next waits for its answer
    /// rather than queueing behind a slow call twice.
    status_in_flight: bool,
}

impl DispatchWorker {
    fn new(port: Option<Box<dyn DispatchPort>>) -> Self {
        let Some(mut port) = port else {
            return Self {
                command: PathBuf::new(),
                data_dir: PathBuf::new(),
                inline: None,
                jobs: None,
                done: None,
                status_in_flight: false,
            };
        };
        let command = port.command();
        let data_dir = port.data_dir();
        if !port.may_block() {
            return Self {
                command,
                data_dir,
                inline: Some(port),
                jobs: None,
                done: None,
                status_in_flight: false,
            };
        }
        let (jobs, job_rx) = std::sync::mpsc::channel::<Body>();
        let (done_tx, done) = std::sync::mpsc::channel::<DispatchDone>();
        let spawned = std::thread::Builder::new()
            .name("dispatch-port".into())
            .spawn(move || {
                for body in job_rx {
                    let result = port.call(&body);
                    if done_tx.send((body, result)).is_err() {
                        break;
                    }
                }
            });
        if let Err(e) = spawned {
            log::error!("dispatch port thread: {e}");
        }
        Self {
            command,
            data_dir,
            inline: None,
            jobs: Some(jobs),
            done: Some(done),
            status_in_flight: false,
        }
    }

    /// Send one call. In place, the reply comes back now; on the
    /// thread, it comes through `drain` and this is `None`.
    fn call(&mut self, body: Body) -> Option<DispatchDone> {
        if let Some(port) = &mut self.inline {
            let result = port.call(&body);
            return Some((body, result));
        }
        let why = match &self.jobs {
            Some(jobs) if jobs.send(body.clone()).is_ok() => return None,
            Some(_) => "the dispatch port thread is gone",
            None => "the app runs without a Dispatch port",
        };
        Some((body, Err(io::Error::other(why))))
    }

    /// Ask for the status, unless the last ask is still on the thread.
    fn poll_status(&mut self) -> Option<DispatchDone> {
        if self.jobs.is_some() {
            if self.status_in_flight {
                return None;
            }
            self.status_in_flight = true;
        }
        self.call(Body::Status)
    }

    fn drain(&mut self) -> Vec<DispatchDone> {
        let Some(done) = &self.done else {
            return Vec::new();
        };
        let finished: Vec<DispatchDone> = done.try_iter().collect();
        self.finished(finished)
    }

    /// The next reply the thread finishes, waiting up to `timeout` for
    /// it; nothing when it does not come in time.
    fn wait(&mut self, timeout: Duration) -> Vec<DispatchDone> {
        let Some(done) = &self.done else {
            return Vec::new();
        };
        let finished: Vec<DispatchDone> = done.recv_timeout(timeout).into_iter().collect();
        self.finished(finished)
    }

    fn finished(&mut self, finished: Vec<DispatchDone>) -> Vec<DispatchDone> {
        if finished
            .iter()
            .any(|(body, _)| matches!(body, Body::Status))
        {
            self.status_in_flight = false;
        }
        finished
    }
}

/// The remote port's protocol version, in `/info`.
const REMOTE_PROTOCOL: u32 = 1;
/// The remote port's caps: a reply's bytes, a string in a view, and a
/// session's notes when the session is read alone.
const BODY_CAP: usize = 1024 * 1024;
const STRING_CAP: usize = 4 * 1024;
/// The most a `send` carries, and the longest operation id a device
/// may make.
const SEND_BYTES_CAP: usize = 16 * 1024;
const OP_CAP: usize = 64;
const NOTES_CAP_ALONE: usize = 64 * 1024;

/// Cut every string in `value` to its cap, `notes` to `notes_cap` and
/// the rest to `STRING_CAP`, naming each cut field in a `truncated`
/// list on its object, so no field leaves unbounded and a client can
/// tell a short value from a cut one.
fn bound_strings(value: &mut serde_json::Value, notes_cap: usize) {
    match value {
        serde_json::Value::Object(map) => {
            let mut cut = Vec::new();
            for (name, field) in map.iter_mut() {
                match field {
                    serde_json::Value::String(s) => {
                        let cap = if name == "notes" {
                            notes_cap
                        } else {
                            STRING_CAP
                        };
                        if s.len() > cap {
                            let mut end = cap;
                            while !s.is_char_boundary(end) {
                                end -= 1;
                            }
                            s.truncate(end);
                            cut.push(name.clone());
                        }
                    }
                    other => bound_strings(other, notes_cap),
                }
            }
            if !cut.is_empty() {
                map.insert("truncated".into(), serde_json::json!(cut));
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                bound_strings(item, notes_cap);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::time::SystemTime;

    use super::SwitchboardApp;
    use crate::adapters::fakes;
    use crate::content::redact;
    use crate::core::model::{Project, ProjectEnv, ProjectId, SessionKind, SpaceId, Workspace};
    use crate::core::{AppAction, ControlAction, Launch};
    use crate::core::{RECORD_TOKEN_ENV, token_hash};
    use crate::ports::host::SpawnSpec;

    /// Every spawn, a resume as much as the first, carries the
    /// variables on the record, under the launcher's own.
    #[test]
    fn a_spawn_carries_the_records_variables_under_the_launchers_own() {
        let mut app = SwitchboardApp::with_services(fakes::services());
        let project = Project {
            id: ProjectId::new(),
            name: "p".into(),
            root: PathBuf::from("/work/p"),
            tags: Vec::new(),
            notes: String::new(),
            pinned: Vec::new(),
            env: ProjectEnv::default(),
            shown: Vec::new(),
            created: SystemTime::UNIX_EPOCH,
            last_active: SystemTime::UNIX_EPOCH,
            space: SpaceId::DEFAULT,
            op: None,
            env_sets: Vec::new(),
        };
        let pid = project.id;
        app.core_mut_for_seeding()
            .seed(vec![Workspace::new(project)], Vec::new());
        app.dispatch(AppAction::Control {
            op: "op-1".into(),
            action: ControlAction::NewSession {
                project: pid,
                name: "tester".into(),
                kind: SessionKind::Shell,
                cwd: "/work/p".into(),
                launch: Launch::Shell,
                prompt: None,
                notes: String::new(),
                env: BTreeMap::from([
                    ("DISPATCH_INPUT_PERSONAS".into(), "/d/personas.md".into()),
                    ("SHARED".into(), "record".into()),
                ]),
                env_sets: Vec::new(),
                replaces: None,
            },
        });
        let id = app.core.workspace(pid).unwrap().sessions[0].id;
        let env = app.spawn_env(id, vec![("SHARED".into(), "launcher".into())]);
        let at = |key: &str, value: &str| {
            env.iter()
                .position(|(k, v)| k == key && v == value)
                .unwrap_or_else(|| panic!("{key}={value} in {env:?}"))
        };
        at("DISPATCH_INPUT_PERSONAS", "/d/personas.md");
        assert!(at("SHARED", "record") < at("SHARED", "launcher"), "{env:?}");
    }

    /// Each spawn carries a fresh token whose hash is what the record
    /// keeps, and the data dir whatever the kind; a second spawn replaces
    /// both, so the first pane's token stops resolving.
    #[test]
    fn a_spawn_carries_a_fresh_token_whose_hash_is_on_the_record() {
        let host = fakes::FakeHost::default();
        let mut services = fakes::services();
        services.host = std::sync::Arc::new(host.clone());
        let mut app = SwitchboardApp::with_services(services);
        let project = Project {
            id: ProjectId::new(),
            name: "p".into(),
            root: PathBuf::from("/work/p"),
            tags: Vec::new(),
            notes: String::new(),
            pinned: Vec::new(),
            env: ProjectEnv::default(),
            env_sets: Vec::new(),
            shown: Vec::new(),
            created: SystemTime::UNIX_EPOCH,
            last_active: SystemTime::UNIX_EPOCH,
            space: SpaceId::DEFAULT,
            op: None,
        };
        let pid = project.id;
        app.core_mut_for_seeding()
            .seed(vec![Workspace::new(project)], Vec::new());
        app.dispatch(AppAction::Control {
            op: "op-1".into(),
            action: ControlAction::NewSession {
                project: pid,
                name: "shell".into(),
                kind: SessionKind::Shell,
                cwd: "/work/p".into(),
                launch: Launch::Shell,
                prompt: None,
                notes: String::new(),
                env: BTreeMap::new(),
                env_sets: Vec::new(),
                replaces: None,
            },
        });
        let id = app.core.workspace(pid).unwrap().sessions[0].id;
        let token_of = |spec: &SpawnSpec| {
            spec.env
                .iter()
                .find(|(k, _)| k == RECORD_TOKEN_ENV)
                .map(|(_, v)| v.clone())
                .expect("a token")
        };
        let first = host.state().spawned.last().cloned().expect("a spawn");
        let token = token_of(&first);
        assert_eq!(token.len(), 64);
        assert!(
            first
                .env
                .iter()
                .any(|(k, v)| k == "SWITCHBOARD_DATA_DIR" && !v.is_empty()),
            "{:?}",
            first.env
        );
        let hash = app.core.session(id).unwrap().token_hash.clone();
        assert_eq!(hash, Some(token_hash(&token)));
        let action = app.spawn(id, first.clone());
        assert!(matches!(action, AppAction::Spawned { result: Ok(()), .. }));
        let second = token_of(host.state().spawned.last().unwrap());
        assert_ne!(second, token);
        let hash = app.core.session(id).unwrap().token_hash.clone();
        assert_eq!(hash, Some(token_hash(&second)));
    }

    #[test]
    fn a_value_inside_another_is_replaced_whole_in_both() {
        let secrets = vec![
            ("SHORT".to_owned(), "abc123".to_owned()),
            ("LONG".to_owned(), "xx-abc123-yy".to_owned()),
        ];
        let text = "token xx-abc123-yy and abc123 and abc";
        assert_eq!(redact(text, &secrets), "token <LONG> and <SHORT> and abc");
        assert_eq!(redact("nothing here", &secrets), "nothing here");
    }

    #[test]
    fn a_value_wrapped_across_lines_is_replaced() {
        let secrets = vec![("API_TOKEN".to_owned(), "tok-0123456789".to_owned())];
        let text = "$ echo $API_TOKEN\nAPI_TOKEN=tok-012\n3456789\r\nnext tok-0123456789";
        assert_eq!(
            redact(text, &secrets),
            "$ echo $API_TOKEN\nAPI_TOKEN=<API_TOKEN>\r\nnext <API_TOKEN>"
        );
        let secrets = vec![("PASS".to_owned(), "correct horse battery staple".to_owned())];
        let text = "PASS=correct horse\nbattery staple and correct horse \nbattery staple";
        assert_eq!(redact(text, &secrets), "PASS=<PASS> and <PASS>");
        let secrets = vec![("PASS".to_owned(), "pass  word".to_owned())];
        let text = "a pass\nword b pass\n word c pass\n  word d pass\n   word";
        assert_eq!(
            redact(text, &secrets),
            "a <PASS> b <PASS> c <PASS> d pass\n   word"
        );
    }
}
