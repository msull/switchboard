//! Orchestration: owns [`AppCore`] and the adapters. Maps effects to
//! adapter calls and feeds results back as actions; polls the host and
//! the event log on a timer; keeps the UI's captions fresh. Rendering
//! lives in [`crate::ui`].

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use base64::Engine as _;
use switchboard_control as wire;

use crate::adapters::control::{ControlSocket, Incoming};
use crate::adapters::hooks::WakeSocket;
use crate::adapters::remote::{
    self, RemoteIncoming, RemotePort, RemoteReply, RemoteRequest, SubscriberFrame, SubscriberQueue,
};
use crate::adapters::scrollback::scrollback_dir;
use crate::cadence::{self, Cadence, Step};
use crate::content::{
    self, ACTIVITY_BYTES_CAP, ContentJob, ContentKey, ContentVersion, ContentView, ContentWorker,
    ConversationCache, FILE_BYTES_CAP, Observed, PAGE_TURNS_CAP, PAGE_TURNS_DEFAULT, PageFrom,
    Policy, PolicySpec, RUN_LINES_CAP, SCREEN_LINES_CAP, SCREEN_SLACK_LINES, Source,
    TURN_BYTES_CAP, TicketPart, WATCH_CAP, WATCH_IDLE, Watch, redact,
};
use crate::core::model::PinTarget;
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
use crate::ui::{UiState, prompt_box};

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
    /// An `Arc`: the content worker resolves policies through it.
    pub secrets: Arc<dyn SecretStore>,
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
    /// Where the app's own updates come from. An `Arc`: the check and
    /// the download run on threads of their own.
    pub updates: Arc<dyn crate::ports::updates::UpdateSource>,
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

/// Whether the root window is drawn: eframe skips `ui` while it is
/// occluded or minimized and calls only `logic`.
#[derive(Default)]
struct Shown {
    /// Whether `ui` ran since the last `logic`.
    ui_ran: bool,
    /// Whether the last `logic` saw the window visible.
    last_visible: bool,
}

pub struct SwitchboardApp {
    core: AppCore,
    services: Services,
    started: Instant,
    /// When the tick and polls last ran; `None` until `start`, so UI
    /// tests, which never call it, never poll or badge.
    cadence: Option<Cadence>,
    shown: Shown,
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
    /// The remote port's refusals since the last summary line. In a
    /// `RefCell` because `refuse` takes `&self` at every call site;
    /// the frame is one thread and `count_refusal` and
    /// `flush_refusals` are the only borrowers, each for one call.
    refusals: RefCell<RefusalSummary>,
    /// This run's epoch and the revision counter every remote reply
    /// carries; the counter moves once per pass whose read-model diff
    /// found a change.
    epoch: uuid::Uuid,
    revision: u64,
    /// The read models as the last diff pass saw them, and whether a
    /// dispatch since then makes another pass worth taking.
    fingerprints: Option<Fingerprints>,
    dirty: bool,
    /// A device's decide or resume on a ticket, answered when Dispatch
    /// answers the call it became (the port's thread brings the reply
    /// on a later frame); one too old is dropped, the connection
    /// having given up by then.
    ticket_replies: Vec<TicketReply>,
    /// Each session's draft as last broadcast, so a device holding a
    /// draft hears when the editor under it moves.
    draft_marks: HashMap<RecordId, u64>,
    last_draft_pass: Option<Instant>,
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
    /// Counts the saves and secret changes, which may have moved a
    /// policy: part of every policy spec, so the worker resolves again.
    policy_epoch: u64,
    /// File diffs being read on threads of their own.
    diff_reads: DiffReads,
    /// What the update threads answer, each one an action for the
    /// next frame.
    update_results: UpdateResults,
    /// The staged bundle whose swap helper is waiting for this process
    /// to exit; the next frame closes the window.
    pending_update: Option<PathBuf>,
}

struct UpdateResults {
    tx: std::sync::mpsc::Sender<AppAction>,
    rx: std::sync::mpsc::Receiver<AppAction>,
}

impl Default for UpdateResults {
    fn default() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        Self { tx, rx }
    }
}

/// One WebSocket subscriber as the frame holds it.
struct Subscriber {
    id: u64,
    queue: SubscriberQueue,
    device: uuid::Uuid,
    /// Subscribed with `scope=all`: told about Dispatch's worker
    /// records too, which a plain subscriber never hears of.
    all: bool,
}

/// The records Dispatch made for its own use, as `scope=all` alone
/// lists them: a ticket's project with everything in it, and a worker
/// session in any project (`wire::is_dispatch_worker_op`).
#[derive(Default)]
struct WorkerRecords {
    projects: HashSet<String>,
    sessions: HashSet<String>,
}

impl WorkerRecords {
    /// Whether a frame about `resource` is for `scope=all` only.
    fn covers(&self, resource: &crate::core::revision::Resource) -> bool {
        use crate::core::revision::ResourceKind as K;
        match resource.kind {
            K::Project | K::Sessions => self.projects.contains(&resource.id),
            K::Session | K::Runs => self.sessions.contains(&resource.id),
            K::Space | K::Set | K::Projects | K::Spaces | K::Sets | K::Waiting | K::Tickets => {
                false
            }
        }
    }
}

/// Who a control request came from, decided by the channel it arrived
/// on and never by anything in it: a line on `control.sock` is a tool's,
/// even one from a pane holding its launch token, and a request the
/// remote port authenticated is the owner's.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
    Tool,
    Owner,
}

/// How a remote request was answered: now, or later through the reply
/// channel it was handed, by the content worker or by Dispatch's
/// answer to a ticket command.
enum Served {
    Now(RemoteReply),
    Deferred,
}

/// A ticket command a device sent, waiting on Dispatch's answer: the
/// call it became, and where the answer goes.
struct TicketReply {
    call: Body,
    reply: mpsc::SyncSender<RemoteReply>,
    asked: Instant,
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

/// The remote port's refusals counted per cause (the refusal code, or
/// `rate_limited`) and logged as one line a minute per cause, so a
/// host on the network that keeps knocking writes a line a minute, not
/// one per knock. Times are the frame clock's monotonic ones.
#[derive(Debug, Default)]
struct RefusalSummary {
    counts: BTreeMap<String, u64>,
    since: Option<Duration>,
}

impl RefusalSummary {
    const EVERY: Duration = Duration::from_mins(1);

    fn count(&mut self, cause: &str, n: u64, now: Duration) {
        if n == 0 {
            return;
        }
        *self.counts.entry(cause.to_owned()).or_insert(0) += n;
        self.since.get_or_insert(now);
    }

    /// One line per cause once a minute has passed since the first
    /// count, and a fresh start; nothing before then.
    fn flush(&mut self, now: Duration) -> Vec<String> {
        if self
            .since
            .is_none_or(|since| now.saturating_sub(since) < Self::EVERY)
        {
            return Vec::new();
        }
        self.since = None;
        std::mem::take(&mut self.counts)
            .into_iter()
            .map(|(cause, n)| format!("remote: {n} refused {cause} in the last minute"))
            .collect()
    }
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
            cadence: None,
            shown: Shown::default(),
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
            refusals: RefCell::new(RefusalSummary::default()),
            epoch: uuid::Uuid::new_v4(),
            revision: 0,
            fingerprints: None,
            dirty: false,
            ticket_replies: Vec::new(),
            draft_marks: HashMap::new(),
            last_draft_pass: None,
            subscribers: Vec::new(),
            content: None,
            observed: None,
            versions: std::collections::HashMap::new(),
            watches: std::collections::HashMap::new(),
            watch_dirty: false,
            last_watch_sweep: None,
            policy_epoch: 0,
            diff_reads: DiffReads::default(),
            update_results: UpdateResults::default(),
            pending_update: None,
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
        self.cadence = Some(Cadence::started(Instant::now()));
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
        let replied = match &action {
            AppAction::DispatchReplied { body, result } if !self.ticket_replies.is_empty() => {
                Some((body.clone(), result.clone()))
            }
            _ => None,
        };
        let effects = self.core.dispatch(action, now);
        if let Some((body, result)) = &replied {
            self.answer_ticket_reply(body, result);
        }
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
    // One arm per effect: the table is long because the contract is.
    #[allow(clippy::too_many_lines)]
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
                self.policy_epoch += 1;
                self.watch_dirty = true;
                self.run_store_effect(effect)
            }
            Effect::SaveIdentity(_) | Effect::SaveDevices(_) => self.run_remote_effect(effect),
            Effect::CheckForUpdate | Effect::DownloadUpdate(_) | Effect::InstallUpdate(_) => {
                self.run_update_effect(effect)
            }
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
        self.cadence = Some(self.cadence.unwrap_or_default().host_polled(Instant::now()));
        self.last_caption = Some(Instant::now());
        self.poll_events();
        self.poll_host();
        self.poll_discoveries();
        self.poll_configs();
        self.refresh_captions();
        self.check_trust_prompts();
    }

    fn pump(&mut self, step: &Step) {
        self.serve_pending();
        // Every `logic` call, not every poll: a button press should land
        // in the pass its wake-up requested.
        for event in self.services.controller.poll() {
            self.dispatch(AppAction::Controller(event));
        }
        // Before the early return so a hidden pass still collects the
        // Dispatch reply instead of leaving it for the next 5 s tick.
        self.drain_dispatch();
        if step.events {
            self.poll_events();
        }
        if step.host_poll {
            self.poll_host();
            self.poll_discoveries();
        }
        if !step.tick {
            return;
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

    /// The update's steps: the check and the download on threads of
    /// their own, answering through `update_results`; the install on
    /// the frame, since it only arranges the swap.
    fn run_update_effect(&mut self, effect: Effect) -> Option<AppAction> {
        let source = Arc::clone(&self.services.updates);
        let tx = self.update_results.tx.clone();
        match effect {
            Effect::CheckForUpdate => {
                std::thread::spawn(move || {
                    let _ = tx.send(AppAction::UpdateChecked(source.latest()));
                });
                None
            }
            Effect::DownloadUpdate(release) => {
                std::thread::spawn(move || {
                    let _ = tx.send(AppAction::UpdateDownloaded(source.download(&release)));
                });
                None
            }
            Effect::InstallUpdate(staged) => match source.install(&staged) {
                Ok(()) => {
                    self.pending_update = Some(staged);
                    None
                }
                Err(reason) => Some(AppAction::UpdateInstallFailed(reason)),
            },
            _ => unreachable!("not an update effect"),
        }
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
        // `take` swaps in `false` and returns the old value, so the flag
        // means "since the previous `logic`".
        let visible = std::mem::take(&mut self.shown.ui_ran);
        self.shown.last_visible = visible;
        let step = self.cadence.map(|prev| {
            let woken = self
                .services
                .wake
                .as_ref()
                .is_some_and(WakeSocket::take_woken);
            let (next, step) = cadence::decide(prev, Instant::now(), visible, woken);
            self.cadence = Some(next);
            step
        });
        if step.is_none_or(|s| s.tick) {
            self.dispatch(AppAction::Tick);
        }
        self.flush_refusals();
        self.drain_diffs();
        let answered: Vec<AppAction> = self.update_results.rx.try_iter().collect();
        for action in answered {
            self.dispatch(action);
        }
        if self.pending_update.is_some() {
            // The helper waits for this process to go, then swaps the
            // bundles and relaunches; a normal close saves the windows.
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.diff_reads.out > 0 {
            // The read threads have no context to wake the frame with.
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        if let Some(step) = step {
            self.pump(&step);
            ctx.request_repaint_after(step.next_in);
            let waiting = self.core.waiting_count();
            // A hidden pass that neither ticks nor lists the host has no new
            // waiting count; a trailing host poll changes it, so it counts.
            if (step.tick || step.events || step.host_poll) && self.badge != Some(waiting) {
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
        self.shown.ui_ran = true;
        if self.cadence.is_some() && !self.shown.last_visible {
            // The `logic` before this frame thought the window hidden and
            // may have skipped the polls; one more pass refreshes the
            // cards at once instead of on the next tick.
            ui.ctx().request_repaint();
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

/// The directory a device names for a session, absolute or relative
/// to the project's root: `None` unless it is the root or a directory
/// under it, with no step back out, so a device never opens a shell
/// elsewhere on the disk.
fn project_dir(root: &Path, text: &str) -> Option<PathBuf> {
    use std::path::Component;
    let given = Path::new(text);
    let dir = if given.is_absolute() {
        given.to_path_buf()
    } else {
        root.join(given)
    };
    let plain = dir.components().all(|c| {
        matches!(
            c,
            Component::Normal(_) | Component::RootDir | Component::Prefix(_)
        )
    });
    (plain && dir.starts_with(root) && dir.is_dir()).then_some(dir)
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
        self.draft_pass();
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
        self.ticket_replies
            .retain(|r| r.asked.elapsed() < remote::ANSWER_TIMEOUT);
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
            // Ticket text has no record to version: the reply carries
            // the port's current revision, as every other JSON read does.
            Ok((_, body)) if matches!(key, ContentKey::TicketText(..)) => {
                if let Some(reply) = reply {
                    // Named per read: an event's `text` in the detail is
                    // a short string, a file's `text` is the file.
                    let longs: &[(&str, usize)] = match &key {
                        ContentKey::TicketText(_, TicketPart::Detail) => {
                            &[("notes", LONG_TEXT_CAP), ("body", LONG_TEXT_CAP)]
                        }
                        ContentKey::TicketText(_, TicketPart::File(_)) => &[("text", FILE_CAP)],
                        _ => &[],
                    };
                    let label = format!("{} {}", key.kind(), key.id());
                    let _ = reply.try_send(self.bounded_reply(&label, body, longs));
                }
            }
            Ok((source, body)) => {
                if self.content_present(&key) {
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
                        self.broadcast(&text, false);
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
                        RemoteReply::new(
                            500,
                            self.error_json("too_large", "the reply is over the body cap"),
                            true,
                        )
                    } else {
                        out
                    };
                    let _ = reply.try_send(out);
                }
            }
            Err(error) => {
                if let Some(reply) = reply {
                    let (status, code, reason) = error.refusal();
                    log::debug!("remote: {} {} refused: {code}", key.kind(), key.id());
                    self.count_refusal(code);
                    let _ = reply.try_send(RemoteReply::new(
                        status,
                        self.error_json(code, &reason),
                        true,
                    ));
                }
            }
        }
    }

    /// Whether what a key reads is still on the host: its session, or
    /// for a file its project.
    fn content_present(&self, key: &ContentKey) -> bool {
        match key {
            ContentKey::File(project, _) => self.core.workspace(*project).is_some(),
            _ => key
                .record()
                .is_some_and(|id| self.core.session(id).is_some()),
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
                ContentKey::File(..) => ContentView::Whole,
                // Ticket text is read once per request, never watched.
                ContentKey::TicketText(..) => continue,
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
    /// (a running pane, a Claude Code transcript, a run's log) or the
    /// project (a pinned file), and the spec of the policy to mask it
    /// with, which the worker resolves. The error is a refusal's code
    /// and reason.
    fn content_job(
        &mut self,
        key: &ContentKey,
        view: ContentView,
        reply: Option<mpsc::SyncSender<RemoteReply>>,
    ) -> Result<ContentJob, (u16, &'static str, String)> {
        if let ContentKey::File(project, path) = key {
            let Some(workspace) = self.core.workspace(*project) else {
                return Err((404, "not_found", "no such project".into()));
            };
            let pinned = self.core.working_sets().iter().any(|set| {
                set.items
                    .iter()
                    .any(|item| item.target == PinTarget::File(*project, path.clone()))
            });
            if !pinned {
                return Err((404, "not_pinned", "no working set pins this file".into()));
            }
            return Ok(ContentJob {
                key: key.clone(),
                source: Source::File {
                    root: workspace.project.root.clone(),
                    relative: path.clone(),
                },
                view,
                policies: vec![self.policy_spec(*project, None)],
                reply,
            });
        }
        let Some(id) = key.record() else {
            return Err((404, "not_found", "no such session".into()));
        };
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
            // A file is handled above, and ticket text has no record.
            ContentKey::File(..) | ContentKey::TicketText(..) => unreachable!("no record"),
        };
        Ok(ContentJob {
            key: key.clone(),
            source,
            view,
            policies: vec![self.policy_spec(record.project, Some(record))],
            reply,
        })
    }

    /// The policies ticket text is masked with: one per project and
    /// stored environment set, not only the sets granted now, so text a
    /// child printed under a grant stays masked once the grant is taken
    /// back or its session is gone. Each set resolves alone, because
    /// resolving several as one overlay lets a later set's variable
    /// replace an earlier one's of the same name and leave that value
    /// unmasked. No ticket names the project it runs in, so every
    /// project is masked, which over-masks rather than leaks. The set
    /// loses its AWS method, which masks nothing and would make two
    /// methods an error.
    fn ticket_policy_specs(&self) -> Vec<PolicySpec> {
        let mut projects: Vec<PolicySpec> = self
            .core
            .workspaces()
            .iter()
            .map(|w| self.policy_spec(w.project.id, None))
            .collect();
        if projects.is_empty() {
            projects.push(self.policy_spec(ProjectId(uuid::Uuid::nil()), None));
        }
        let mut specs = Vec::new();
        for mut spec in projects {
            let sets = std::mem::take(&mut spec.sets);
            spec.project_sets = Vec::new();
            // The project's own and the global variables, with no set,
            // so a store with no sets still has a policy to resolve.
            specs.push(spec.clone());
            for mut set in sets {
                set.aws = None;
                let mut one = spec.clone();
                one.project_sets = vec![set.name.clone()];
                one.sets = vec![set];
                specs.push(one);
            }
        }
        specs
    }

    /// Send ticket text to the worker to be masked before it is
    /// serialized; the answer goes to `reply` from `observe`. False
    /// when the worker is missing or behind.
    fn submit_ticket_text(
        &self,
        ticket: Option<&str>,
        part: TicketPart,
        value: serde_json::Value,
        reply: &mpsc::SyncSender<RemoteReply>,
    ) -> bool {
        let Some(content) = &self.content else {
            return false;
        };
        content.submit(ContentJob {
            key: ContentKey::TicketText(ticket.map(str::to_owned), part),
            source: Source::Json(value),
            view: ContentView::Whole,
            policies: self.ticket_policy_specs(),
            reply: Some(reply.clone()),
        })
    }

    /// [`Self::submit_ticket_text`] for a route: deferred, or refused
    /// `busy`.
    fn serve_ticket_text(
        &self,
        request: &RemoteRequest,
        ticket: Option<&str>,
        part: TicketPart,
        value: serde_json::Value,
        reply: &mpsc::SyncSender<RemoteReply>,
    ) -> Served {
        if self.submit_ticket_text(ticket, part, value, reply) {
            Served::Deferred
        } else {
            Served::Now(self.refuse(
                request,
                503,
                "busy",
                "the content worker is behind; try again",
                true,
            ))
        }
    }

    /// What names a policy, from the core: the project's environment
    /// and the sets it and the session may use, with the epoch. The
    /// values are read on the worker.
    fn policy_spec(&self, project: ProjectId, record: Option<&SessionRecord>) -> PolicySpec {
        let workspace = self.core.workspace(project);
        PolicySpec {
            epoch: self.policy_epoch,
            project,
            root: workspace
                .map(|w| w.project.root.clone())
                .unwrap_or_default(),
            global: self.core.settings().env.clone(),
            project_env: workspace.map(|w| w.project.env.clone()).unwrap_or_default(),
            sets: self.core.settings().env_sets.clone(),
            project_sets: workspace
                .map(|w| w.project.env_sets.clone())
                .unwrap_or_default(),
            record_sets: record.map(|r| r.env_sets.clone()).unwrap_or_default(),
        }
    }

    /// A session's redaction policy on the frame, for the control
    /// socket's own screen read.
    fn resolve_policy(&self, id: RecordId) -> Result<Policy, String> {
        let Some(record) = self.core.session(id) else {
            return Err("no such session".into());
        };
        content::resolve_policy(
            &self.policy_spec(record.project, Some(record)),
            &*self.services.secrets,
        )
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
        if let ["projects", project, "files", path @ ..] = segments {
            return self.remote_file_route(request, project, path, reply);
        }
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
                let from = match Self::page_from(request) {
                    Ok(from) => from,
                    Err(reason) => {
                        return Served::Now(self.refuse(request, 400, "bad_request", reason, true));
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

    /// Where a page is anchored, from `from` (a turn number or `last`)
    /// or `before` (a turn number), which do not go together.
    fn page_from(request: &RemoteRequest) -> Result<PageFrom, &'static str> {
        match (request.param("from"), request.param("before")) {
            (None, None) => Ok(PageFrom::Turn(1)),
            (Some("last"), None) => Ok(PageFrom::Last),
            (Some(n), None) => match n.parse::<usize>() {
                Ok(n) if n >= 1 => Ok(PageFrom::Turn(n)),
                _ => Err("from is a turn number or last"),
            },
            (None, Some(n)) => match n.parse::<usize>() {
                Ok(n) if n >= 1 => Ok(PageFrom::Before(n)),
                _ => Err("before is a turn number"),
            },
            (Some(_), Some(_)) => Err("from and before do not go together"),
        }
    }

    /// A pinned file by its project and its path under the root; the
    /// path is checked here before the walk checks it again.
    fn remote_file_route(
        &mut self,
        request: &RemoteRequest,
        project: &str,
        path: &[&str],
        reply: &mpsc::SyncSender<RemoteReply>,
    ) -> Served {
        let project = match parse_uuid("project", project) {
            Ok(id) => ProjectId(id),
            Err(reason) => {
                return Served::Now(self.refuse(request, 400, "failed", &reason, true));
            }
        };
        let relative = PathBuf::from(path.join("/"));
        let clean = !path.is_empty()
            && relative
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)));
        if !clean {
            return Served::Now(self.refuse(
                request,
                400,
                "bad_request",
                "the path is relative to the project root, with no . or .. component",
                true,
            ));
        }
        self.remote_content(
            request,
            &ContentKey::File(project, relative),
            ContentView::Whole,
            reply,
        )
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

    /// Dispatch's tickets as the core lists them (§4.7), masked.
    fn remote_tickets(
        &self,
        request: &RemoteRequest,
        reply: &mpsc::SyncSender<RemoteReply>,
    ) -> Served {
        let value = serde_json::to_value(self.core.ticket_listing()).unwrap_or_default();
        self.serve_ticket_text(request, None, TicketPart::List, value, reply)
    }

    /// One ticket with its body, attempts and lanes, masked.
    fn remote_ticket(
        &self,
        request: &RemoteRequest,
        ticket: &str,
        reply: &mpsc::SyncSender<RemoteReply>,
    ) -> Served {
        let Some(detail) = self.core.ticket_detail(ticket) else {
            return Served::Now(self.refuse(request, 404, "not_found", "no such ticket", true));
        };
        let value = serde_json::to_value(detail).unwrap_or_default();
        self.serve_ticket_text(request, Some(ticket), TicketPart::Detail, value, reply)
    }

    /// A decide, a resume or a close on a ticket under `control` scope, checked
    /// against the last status here (the decision pending, the answer
    /// among its options, a note where one is needed) and then the
    /// Mac's own action; Dispatch's answer reaches the device through
    /// `answer_ticket_reply`. A decision already answered as asked is
    /// answered again from the record, so a repeat after a lost reply
    /// costs nothing.
    fn remote_ticket_command(
        &mut self,
        request: &RemoteRequest,
        device: &PairedDevice,
        ticket: &str,
        verb: &str,
        reply: &mpsc::SyncSender<RemoteReply>,
    ) -> Served {
        if device.scope != Scope::Control {
            return Served::Now(self.refuse(
                request,
                403,
                "scope",
                "this device reads only; raise its scope on the host",
                true,
            ));
        }
        let Some(t) = self.core.ticket(ticket).cloned() else {
            return Served::Now(self.refuse(request, 404, "not_found", "no such ticket", true));
        };
        if !self.core.dispatch_state().connected {
            return Served::Now(self.refuse(
                request,
                503,
                "dispatch_off",
                "Dispatch is not running on the host",
                true,
            ));
        }
        let call = if verb == "decide" {
            match self.ticket_decide_call(request, &t, reply) {
                Ok(call) => call,
                Err(served) => return served,
            }
        } else if verb == "close" {
            if !t.closable {
                return Served::Now(self.refuse(
                    request,
                    409,
                    "not_closable",
                    &format!("the ticket is {}; it cannot be closed now", t.state),
                    true,
                ));
            }
            Body::Close {
                ticket: t.id.clone(),
                reason: None,
            }
        } else if t.state == "parked" {
            Body::Resume {
                ticket: t.id.clone(),
            }
        } else {
            return Served::Now(self.refuse(
                request,
                409,
                "not_parked",
                &format!("the ticket is {}, not parked", t.state),
                true,
            ));
        };
        let action = match &call {
            Body::Decide {
                ticket,
                decision,
                answer,
                note,
            } => AppAction::DispatchDecide {
                ticket: ticket.clone(),
                decision: decision.clone(),
                answer: answer.clone(),
                note: note.clone(),
            },
            Body::Close { .. } => AppAction::DispatchClose(t.id.clone()),
            _ => AppAction::DispatchResume(t.id.clone()),
        };
        self.ticket_replies.push(TicketReply {
            call,
            reply: reply.clone(),
            asked: Instant::now(),
        });
        self.dispatch_inner(action);
        Served::Deferred
    }

    /// The `decide` call a request's body asks for, or the refusal.
    fn ticket_decide_call(
        &self,
        request: &RemoteRequest,
        t: &crate::ports::dispatch::TicketView,
        reply: &mpsc::SyncSender<RemoteReply>,
    ) -> Result<Body, Served> {
        let bad = |message: &str| {
            Err(Served::Now(self.refuse(
                request,
                400,
                "bad_request",
                message,
                true,
            )))
        };
        let Ok(body) = serde_json::from_slice::<serde_json::Value>(&request.body) else {
            return bad("the body is not JSON");
        };
        let field = |name: &str| body.get(name).and_then(|v| v.as_str()).map(str::trim);
        let (Some(decision), Some(answer)) = (field("decision"), field("answer")) else {
            return bad("decision (its id) and answer are required");
        };
        if answer.is_empty() || answer.len() > STRING_CAP {
            return bad("answer is the option chosen, or the options joined by commas");
        }
        let note = field("note").filter(|n| !n.is_empty()).map(str::to_owned);
        if note.as_ref().is_some_and(|n| n.len() > LONG_TEXT_CAP) {
            return bad("the note is over 64 KiB");
        }
        let Some(d) = t.decisions.iter().find(|d| d.id == decision) else {
            return Err(Served::Now(self.refuse(
                request,
                404,
                "not_found",
                "no such decision on the ticket",
                true,
            )));
        };
        if d.state != "pending" {
            if d.answer.as_deref() == Some(answer) {
                let value = serde_json::json!({
                    "decision": crate::core::tickets::TicketDecision::from(d),
                });
                return Err(self.serve_ticket_text(
                    request,
                    Some(&t.id),
                    TicketPart::Detail,
                    value,
                    reply,
                ));
            }
            return Err(Served::Now(self.refuse(
                request,
                409,
                "answered",
                &format!("the decision is {}", d.state),
                true,
            )));
        }
        let chosen: Vec<&str> = if d.multiple {
            answer.split(',').map(str::trim).collect()
        } else {
            vec![answer]
        };
        if chosen.iter().any(|c| !d.options.iter().any(|o| o == c)) {
            return bad("the answer is not among the decision's options");
        }
        if note.is_none() && chosen.iter().any(|c| d.needs_note.iter().any(|n| n == c)) {
            return bad("this answer needs a note");
        }
        Ok(Body::Decide {
            ticket: t.id.clone(),
            decision: d.id.clone(),
            answer: chosen.join(","),
            note,
        })
    }

    /// Dispatch's answer to a call a device asked for, sent on to it.
    fn answer_ticket_reply(&mut self, body: &Body, result: &Result<Reply, String>) {
        let same = |a: &Body| match (a, body) {
            (
                Body::Decide {
                    ticket, decision, ..
                },
                Body::Decide {
                    ticket: t,
                    decision: d,
                    ..
                },
            ) => ticket == t && decision == d,
            (Body::Resume { ticket }, Body::Resume { ticket: t })
            | (Body::Close { ticket, .. }, Body::Close { ticket: t, .. })
            | (Body::Events { ticket, .. }, Body::Events { ticket: t, .. }) => ticket == t,
            (Body::Artifact { path, .. }, Body::Artifact { path: p, .. }) => path == p,
            _ => false,
        };
        // Every device that asked for the same thing gets the one answer.
        let (pending, kept): (Vec<TicketReply>, Vec<TicketReply>) =
            std::mem::take(&mut self.ticket_replies)
                .into_iter()
                .partition(|r| same(&r.call));
        self.ticket_replies = kept;
        if pending.is_empty() {
            return;
        }
        // Ticket text goes to the worker to be masked, which answers
        // each device; anything else is answered here.
        let text = match (body, result) {
            (_, Ok(Reply::Decided(d))) => Some((
                TicketPart::Detail,
                serde_json::json!({
                    "decision": crate::core::tickets::TicketDecision::from(d),
                }),
            )),
            (_, Ok(Reply::Ticket(t))) => Some((
                TicketPart::Detail,
                serde_json::json!({ "ticket": self.core.ticket_summary(t) }),
            )),
            (Body::Artifact { path, .. }, Ok(Reply::Artifact { text })) => {
                let key = crate::core::tickets::file_key(path);
                Some((
                    TicketPart::File(key.clone()),
                    serde_json::json!({ "key": key, "text": text }),
                ))
            }
            // The core has taken the events in by now, or marked them
            // unavailable from this runner; either way the cache answers.
            (Body::Events { ticket, .. }, Ok(Reply::Events(_) | Reply::Failed { .. })) => Some((
                TicketPart::Events,
                serde_json::to_value(self.core.ticket_events(ticket)).unwrap_or_default(),
            )),
            _ => None,
        };
        if let Some((part, value)) = text {
            let ticket = body_ticket(body);
            for r in pending {
                if !self.submit_ticket_text(ticket, part.clone(), value.clone(), &r.reply) {
                    let _ = r.reply.try_send(RemoteReply::new(
                        503,
                        self.error_json("busy", "the content worker is behind; try again"),
                        true,
                    ));
                }
            }
            return;
        }
        // Dispatch refuses a file over the bound before reading it; a
        // device hears that as the worker's own refusal would read.
        let answer = match (body, result) {
            (Body::Artifact { .. }, Ok(Reply::Failed { reason }))
                if let Some(len) = dispatch_control::too_large_len(reason) =>
            {
                let (status, code, reason) = content::ContentError::TooLarge(len).refusal();
                self.count_refusal(code);
                RemoteReply::new(status, self.error_json(code, &reason), true)
            }
            (_, Ok(Reply::Failed { reason })) => {
                let (status, code) = if reason.starts_with("no such") {
                    (404, "not_found")
                } else {
                    (400, "failed")
                };
                RemoteReply::new(status, self.error_json(code, reason), true)
            }
            (_, Ok(_)) => RemoteReply::new(
                500,
                self.error_json("failed", "Dispatch answered with something else"),
                true,
            ),
            (_, Err(e)) => RemoteReply::new(
                503,
                self.error_json("dispatch_off", &format!("Dispatch did not answer: {e}")),
                true,
            ),
        };
        for r in pending {
            let _ = r.reply.try_send(answer.clone());
        }
    }

    /// A ticket's events, from the Mac's cache while the ticket has not
    /// changed since they were read, else read through Dispatch's port
    /// first; the reply waits for that read. Without a runner the cache
    /// answers as it stands, and empty-handed it is `dispatch_off`.
    fn remote_ticket_events(
        &mut self,
        request: &RemoteRequest,
        ticket: &str,
        reply: &mpsc::SyncSender<RemoteReply>,
    ) -> Served {
        let Some(t) = self.core.ticket(ticket) else {
            return Served::Now(self.refuse(request, 404, "not_found", "no such ticket", true));
        };
        let updated_ms = t.updated_ms;
        let state = self.core.dispatch_state();
        let cached = state.events.get(ticket);
        let fresh = cached
            .is_some_and(|e| e.unavailable || (!e.in_flight && e.asked_at == Some(updated_ms)));
        if fresh || !state.connected {
            if cached.is_none() {
                return Served::Now(self.refuse(
                    request,
                    503,
                    "dispatch_off",
                    "Dispatch is not running on the host",
                    true,
                ));
            }
            let value = serde_json::to_value(self.core.ticket_events(ticket)).unwrap_or_default();
            return self.serve_ticket_text(request, Some(ticket), TicketPart::Events, value, reply);
        }
        self.ticket_replies.push(TicketReply {
            call: Body::Events {
                ticket: ticket.to_owned(),
                since: 0,
            },
            reply: reply.clone(),
            asked: Instant::now(),
        });
        self.dispatch_inner(AppAction::DispatchReadEvents {
            ticket: ticket.to_owned(),
            updated_ms,
        });
        Served::Deferred
    }

    /// One file of a ticket's attempt by its key (`TicketFile`): an
    /// image straight from the disk, text through the Mac's own read of
    /// Dispatch's port, from its cache while the ticket has not changed
    /// since. Dispatch refuses a secret artifact and a file outside the
    /// ticket's directory, and that refusal is the device's answer.
    fn remote_ticket_file(
        &mut self,
        request: &RemoteRequest,
        ticket: &str,
        key: &str,
        reply: &mpsc::SyncSender<RemoteReply>,
    ) -> Served {
        let Some(t) = self.core.ticket(ticket) else {
            return Served::Now(self.refuse(request, 404, "not_found", "no such ticket", true));
        };
        let updated_ms = t.updated_ms;
        let Some((file, path)) = self.core.ticket_file(ticket, key) else {
            return Served::Now(self.refuse(request, 404, "not_found", "no such file", true));
        };
        if file.image {
            // No mask reads pixels; the attempt's `env` tells the
            // device it ran with grants, and the owner accepted that
            // when granting the set, as with a tool result.
            return Served::Now(self.remote_ticket_image(request, &file, &path));
        }
        if !file.inline {
            return Served::Now(self.refuse(
                request,
                415,
                "not_text",
                "not a text file; open it on the host",
                true,
            ));
        }
        if self.core.artifact_current(&path, Some(updated_ms)) {
            let text = self
                .core
                .dispatch_state()
                .artifacts
                .get(&path)
                .cloned()
                .unwrap_or_default();
            let value = serde_json::json!({ "key": file.key, "text": text });
            let part = TicketPart::File(file.key);
            return self.serve_ticket_text(request, Some(ticket), part, value, reply);
        }
        if !self.core.dispatch_state().connected {
            return Served::Now(self.refuse(
                request,
                503,
                "dispatch_off",
                "Dispatch is not running on the host",
                true,
            ));
        }
        self.ticket_replies.push(TicketReply {
            call: Body::Artifact {
                ticket: ticket.to_owned(),
                path: path.clone(),
            },
            reply: reply.clone(),
            asked: Instant::now(),
        });
        self.dispatch_inner(AppAction::DispatchReadArtifact {
            ticket: ticket.to_owned(),
            path,
        });
        Served::Deferred
    }

    /// An evidence image as base64 with its media type, read from the
    /// disk here: the file is one the runner recorded under the ticket,
    /// and the app opens the same path for the owner on the Mac. The
    /// base64 is a long field up to the body cap, and the length is
    /// checked again after the read because the file can grow between
    /// the stat and the read.
    fn remote_ticket_image(
        &self,
        request: &RemoteRequest,
        file: &crate::core::tickets::TicketFile,
        path: &Path,
    ) -> RemoteReply {
        let too_large = || {
            self.refuse(
                request,
                413,
                "too_large",
                "the image is over what the port serves; open it on the host",
                true,
            )
        };
        let len = match std::fs::metadata(path) {
            Ok(m) => m.len(),
            Err(_) => return self.refuse(request, 404, "not_found", "the file is gone", true),
        };
        if len > IMAGE_CAP {
            return too_large();
        }
        let Ok(bytes) = std::fs::read(path) else {
            return self.refuse(request, 404, "not_found", "the file is gone", true);
        };
        if bytes.len() as u64 > IMAGE_CAP {
            return too_large();
        }
        let value = serde_json::json!({
            "key": file.key,
            "media": crate::core::tickets::image_media(path).unwrap_or("application/octet-stream"),
            "data": base64::engine::general_purpose::STANDARD.encode(bytes),
        });
        self.remote_json(request, value, &[("data", BODY_CAP)])
    }

    /// A JSON body for a device: strings bounded, the fields in `longs`
    /// to their own caps, the envelope put round it, and a body over the
    /// cap refused rather than cut.
    fn remote_json(
        &self,
        request: &RemoteRequest,
        value: serde_json::Value,
        longs: &[(&str, usize)],
    ) -> RemoteReply {
        self.bounded_reply(
            &format!("{} {}", request.method, request.path),
            value,
            longs,
        )
    }

    /// The half of [`Self::remote_json`] after any mask: strings
    /// bounded, the envelope, and a body over the cap refused; `label`
    /// names the read in the log.
    fn bounded_reply(
        &self,
        label: &str,
        mut value: serde_json::Value,
        longs: &[(&str, usize)],
    ) -> RemoteReply {
        bound_strings(&mut value, longs);
        let out = self.envelope(200, &value, true);
        if out.body.len() > BODY_CAP {
            log::warn!("remote: {label} is {} bytes, over the cap", out.body.len());
            self.count_refusal("too_large");
            return RemoteReply::new(
                500,
                self.error_json("too_large", "the reply is over the port's body cap"),
                true,
            );
        }
        out
    }

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
        let workers = if self.subscribers.iter().all(|s| s.all) {
            WorkerRecords::default()
        } else {
            self.worker_records()
        };
        for change in changes {
            let kind = if change.removed { "removed" } else { "changed" };
            let text = self.event_json(
                kind,
                Some((change.resource.kind.word(), &change.resource.id)),
                at_ms,
            );
            self.broadcast(&text, workers.covers(&change.resource));
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

    /// The worker records as they stand, for the frames that stay
    /// with `scope=all` subscribers.
    fn worker_records(&self) -> WorkerRecords {
        let mut out = WorkerRecords::default();
        for p in self.core.project_views(None) {
            let Ok(id) = uuid::Uuid::parse_str(&p.id) else {
                continue;
            };
            let Some(w) = self.core.workspace(ProjectId(id)) else {
                continue;
            };
            let whole = p.op.as_deref().is_some_and(wire::is_dispatch_worker_op);
            if whole {
                out.projects.insert(p.id.clone());
            }
            for s in &w.sessions {
                if whole || s.op.as_deref().is_some_and(wire::is_dispatch_worker_op) {
                    out.sessions.insert(s.id.0.to_string());
                }
            }
        }
        out
    }

    /// Drops Dispatch's worker records from a listing, for a request
    /// without `scope=all`.
    fn hide_workers(reply: &mut wire::Reply) {
        let shown = |op: &Option<String>| !op.as_deref().is_some_and(wire::is_dispatch_worker_op);
        match reply {
            wire::Reply::Projects { projects } => projects.retain(|p| shown(&p.op)),
            wire::Reply::Sessions { sessions } | wire::Reply::Waiting { sessions } => {
                sessions.retain(|s| shown(&s.op));
            }
            _ => {}
        }
    }

    /// Hand one frame to every subscriber, or with `all_only` to the
    /// `scope=all` ones. A full queue is a subscriber that fell
    /// `SUBSCRIBER_QUEUE` events behind: it is dropped with its
    /// overflow flag set, never trimmed, so its connection sends
    /// `resync_required` and closes.
    fn broadcast(&mut self, text: &str, all_only: bool) {
        self.subscribers.retain(|s| {
            if all_only && !s.all {
                return true;
            }
            match s.queue.tx.try_send(text.to_owned()) {
                Ok(()) => true,
                Err(mpsc::TrySendError::Full(_)) => {
                    s.queue.overflow.store(true, Ordering::SeqCst);
                    log::warn!("remote: a subscriber fell behind and is dropped");
                    false
                }
                Err(mpsc::TrySendError::Disconnected(_)) => false,
            }
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
            all: request.param("scope") == Some("all"),
        });
        RemoteReply::new(remote::SUBSCRIBED, Vec::new(), true)
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
                Arc::clone(&self.services.secrets),
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
    // The route table alone is about the lint's hundred lines, one line
    // per short route. It reads best whole: splitting it to get under
    // would scatter the arms that decide what a device may reach.
    #[allow(clippy::too_many_lines)]
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
        let device = match self.authenticate(request) {
            Ok(device) => device,
            Err(refused) => return Served::Now(refused),
        };
        if let ("GET", ["events"]) = (request.method.as_str(), segments.as_slice()) {
            return Served::Now(self.remote_subscribe(request, device.id, subscriber));
        }
        let notes_cap = if matches!(segments.as_slice(), ["sessions", _]) {
            LONG_TEXT_CAP
        } else {
            STRING_CAP
        };
        let body = match (request.method.as_str(), segments.as_slice()) {
            ("GET", ["projects"]) => wire::Body::Projects {
                space: request.param("space").map(str::to_owned),
            },
            ("GET", ["me"]) => return Served::Now(self.remote_me(&device)),
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
            ("POST", ["projects", project, "sessions"]) => {
                return Served::Now(self.remote_new_session(request, &device, project));
            }
            ("GET" | "POST", ["sessions", session, "draft"]) => {
                return Served::Now(self.remote_draft(request, &device, session));
            }
            ("POST", ["sessions", session, "dictation"]) => {
                return Served::Now(self.remote_dictation(request, &device, session));
            }
            ("GET", ["dispatch", "tickets"]) => return self.remote_tickets(request, reply),
            ("GET", ["dispatch", "tickets", ticket]) => {
                return self.remote_ticket(request, ticket, reply);
            }
            (
                "POST",
                [
                    "dispatch",
                    "tickets",
                    ticket,
                    verb @ ("decide" | "resume" | "close"),
                ],
            ) => {
                return self.remote_ticket_command(request, &device, ticket, verb, reply);
            }
            ("GET", ["dispatch", "tickets", ticket, "events"]) => {
                return self.remote_ticket_events(request, ticket, reply);
            }
            ("GET", ["dispatch", "tickets", ticket, "files", key]) => {
                return self.remote_ticket_file(request, ticket, key, reply);
            }
            (
                "GET",
                ["sessions", _, "screen" | "conversation" | "last-response"]
                | ["sessions", _, "conversation", _]
                | ["sessions", _, "runs", _, "output"]
                | ["projects", _, "files", _, ..],
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
        let mut reply = self.answer(&body);
        if request.param("scope") != Some("all") {
            Self::hide_workers(&mut reply);
        }
        Served::Now(self.remote_reply(request, &reply, notes_cap))
    }

    /// The paired device behind a request's bearer token, or else its
    /// cookie named for this host, seen now; or the refusal, which says
    /// nothing of the host. A cookie rides any request the browser
    /// makes to this address, so a cookie-only request that changes
    /// something (anything but a `GET`, or an upgrade) must also come
    /// from the page this port served: its `Origin` names the `Host` it
    /// was sent to. A native client sends a bearer and no `Origin`.
    fn authenticate(&mut self, request: &RemoteRequest) -> Result<PairedDevice, RemoteReply> {
        let cookie_name = self
            .core
            .identity()
            .map(|i| remote_cookie_name(i.server_id));
        let cookie = cookie_name.as_deref().and_then(|name| request.cookie(name));
        let (token, by_cookie) = match (&request.bearer, cookie) {
            (Some(bearer), _) => (bearer.as_str(), false),
            (None, Some(cookie)) => (cookie, true),
            (None, None) => {
                return Err(self.refuse(
                    request,
                    401,
                    "unauthorized",
                    "pair this device first",
                    false,
                ));
            }
        };
        let Some(device) = self
            .core
            .device_by_token(&remote::token_sha256(token))
            .cloned()
        else {
            return Err(self.refuse(
                request,
                401,
                "unauthorized",
                "the token is not known; pair again",
                false,
            ));
        };
        if by_cookie
            && (request.method != "GET" || request.upgrade.is_some())
            && !same_origin(request)
        {
            return Err(self.refuse(
                request,
                403,
                "origin",
                "a browser's request must come from this host's page",
                false,
            ));
        }
        self.dispatch_inner(AppAction::DeviceSeen(device.id));
        Ok(device)
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
        let reply = self.serve_from(&wire::Request::new(op, body), Origin::Owner);
        if let wire::Reply::Failed { reason } = &reply
            && reason.starts_with("interrupted")
        {
            return self.refuse(request, 409, "interrupted", reason, true);
        }
        self.remote_reply(request, &reply, STRING_CAP)
    }

    /// A new session in a project under `control` scope, through
    /// `session.new` as the CLI and Dispatch make theirs, the body's
    /// `op` the key as for `remote_command`. A device opens an agent or
    /// a shell, named, in the project's root or a directory under it,
    /// never elsewhere on the disk; an agent's `prompt` becomes its
    /// first turn, so the session is already working when its window
    /// opens.
    fn remote_new_session(
        &mut self,
        request: &RemoteRequest,
        device: &PairedDevice,
        project: &str,
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
        let root = parse_uuid("project", project)
            .ok()
            .and_then(|id| self.core.workspace(ProjectId(id)))
            .map(|w| w.project.root.clone());
        let Some(root) = root else {
            return self.refuse(request, 404, "not_found", "no such project", true);
        };
        let name = field("name").unwrap_or_default();
        if name.is_empty() || name.len() > NAME_CAP {
            return self.refuse(
                request,
                400,
                "bad_request",
                "name is the session's name, 1 to 128 bytes",
                true,
            );
        }
        let session_kind = match field("kind") {
            Some("claude") => wire::SessionKind::Claude,
            Some("codex") => wire::SessionKind::Codex,
            Some("shell") => wire::SessionKind::Shell,
            _ => {
                return self.refuse(
                    request,
                    400,
                    "bad_request",
                    "kind is claude, codex or shell",
                    true,
                );
            }
        };
        let cwd = match field("cwd").filter(|c| !c.is_empty()) {
            None => root,
            Some(text) => match project_dir(&root, text) {
                Some(dir) => dir,
                None => {
                    return self.refuse(
                        request,
                        400,
                        "bad_request",
                        "cwd is a directory of the project, by a path under its root",
                        true,
                    );
                }
            },
        };
        let prompt = field("prompt").filter(|p| !p.is_empty()).map(str::to_owned);
        if prompt.as_ref().is_some_and(|p| p.len() > SEND_BYTES_CAP) {
            return self.refuse(
                request,
                400,
                "bad_request",
                "prompt is the first message, up to 16 KiB",
                true,
            );
        }
        let body = wire::Body::SessionNew {
            project: project.to_owned(),
            name: name.to_owned(),
            session_kind,
            cwd,
            launch: wire::Launch::Shell,
            prompt,
            notes: String::new(),
            env: std::collections::BTreeMap::new(),
            env_sets: Vec::new(),
            replaces: None,
        };
        let reply = self.serve(&wire::Request::new(op, body));
        if let wire::Reply::Failed { reason } = &reply
            && reason.starts_with("interrupted")
        {
            return self.refuse(request, 409, "interrupted", reason, true);
        }
        self.remote_reply(request, &reply, STRING_CAP)
    }

    /// The session whose draft a device asks for: an agent session,
    /// since only those have a Prompt Box editor.
    fn draft_record(
        &self,
        request: &RemoteRequest,
        session: &str,
    ) -> Result<SessionRecord, RemoteReply> {
        let record = parse_uuid("session", session)
            .ok()
            .and_then(|id| self.core.session(RecordId(id)));
        let Some(record) = record else {
            return Err(self.refuse(request, 404, "not_found", "no such session", true));
        };
        if !matches!(record.kind, SessionKind::Agent(_)) {
            return Err(self.refuse(
                request,
                400,
                "no_draft",
                "only an agent session has a prompt",
                true,
            ));
        }
        Ok(record.clone())
    }

    /// The draft as the device reads it, from the session's editor,
    /// made now if the session has not been opened on the host.
    fn draft_reply(&mut self, record: &SessionRecord) -> RemoteReply {
        let editor =
            prompt_box::ensure_editor(&self.core, &self.services, &mut self.ui_state, record);
        let draft = serde_json::to_value(prompt_box::draft_view(editor)).unwrap_or_default();
        self.envelope(200, &serde_json::json!({ "draft": draft }), true)
    }

    /// A session's prompt, its Prompt Box editor on the host (§4.5):
    /// read by any paired device, and under the control scope edited
    /// with the editor's own tools. `send` takes the draft's text to
    /// the pane through `serve` with the device's op as the key, as
    /// the plain send does, and clears the editor when it went.
    fn remote_draft(
        &mut self,
        request: &RemoteRequest,
        device: &PairedDevice,
        session: &str,
    ) -> RemoteReply {
        let record = match self.draft_record(request, session) {
            Ok(record) => record,
            Err(reply) => return reply,
        };
        if request.method == "GET" {
            return self.draft_reply(&record);
        }
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
        let field = |name: &str| body.get(name).and_then(|v| v.as_str());
        let action = field("action").unwrap_or_default();
        let text = field("text");
        if text.is_some_and(|t| t.len() > SEND_BYTES_CAP) {
            return self.refuse(request, 400, "bad_request", "text is at most 16 KiB", true);
        }
        if action == "send" {
            let op = field("op").map(str::trim).unwrap_or_default();
            if op.is_empty() || op.len() > OP_CAP {
                return self.refuse(
                    request,
                    400,
                    "bad_request",
                    "op is the operation id, 1 to 64 characters",
                    true,
                );
            }
            let editor =
                prompt_box::ensure_editor(&self.core, &self.services, &mut self.ui_state, &record);
            let text = prompt_box::draft_view(editor).text.trim().to_owned();
            if text.is_empty() {
                // A repeat of a send that went: the draft was cleared
                // by it, and the op's log line is the answer.
                let status = self.answer(&wire::Body::OpStatus {
                    operation: op.to_owned(),
                });
                if let wire::Reply::OpStatus {
                    status: wire::OpStatus::Done { reply },
                } = status
                {
                    return self.remote_reply(request, &reply, STRING_CAP);
                }
                return self.refuse(request, 400, "failed", "the draft is empty", true);
            }
            let reply = self.serve_from(
                &wire::Request::new(
                    op,
                    wire::Body::SessionSend {
                        session: session.to_owned(),
                        text,
                    },
                ),
                Origin::Owner,
            );
            match &reply {
                wire::Reply::Failed { reason } if reason.starts_with("interrupted") => {
                    return self.refuse(request, 409, "interrupted", reason, true);
                }
                wire::Reply::Failed { .. } => {
                    return self.remote_reply(request, &reply, STRING_CAP);
                }
                _ => {}
            }
            let editor =
                prompt_box::ensure_editor(&self.core, &self.services, &mut self.ui_state, &record);
            editor.dispatch(promptbox::core::AppAction::ClearPrompt);
            return self.draft_reply(&record);
        }
        let Some(action) = prompt_box::DraftAction::parse(action, text) else {
            return self.refuse(
                request,
                400,
                "bad_request",
                "action is replace (with text), undo, redo, clear, clean_up, delete_sentence, delete_paragraph or send (with op)",
                true,
            );
        };
        let editor =
            prompt_box::ensure_editor(&self.core, &self.services, &mut self.ui_state, &record);
        if let Err(reason) = action.apply(editor) {
            return self.refuse(request, 400, "failed", &reason, true);
        }
        self.draft_reply(&record)
    }

    /// A device's speech events into a session's editor (§4.5), as the
    /// host's own recognizer feeds it: the device recognizes, the
    /// editor keeps the document. The reply is the draft as it stands,
    /// so the device need not wait for the event.
    fn remote_dictation(
        &mut self,
        request: &RemoteRequest,
        device: &PairedDevice,
        session: &str,
    ) -> RemoteReply {
        let record = match self.draft_record(request, session) {
            Ok(record) => record,
            Err(reply) => return reply,
        };
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
        let Some(recognizer) = body.get("session").and_then(serde_json::Value::as_u64) else {
            return self.refuse(
                request,
                400,
                "bad_request",
                "session is the device's recognizer session, a number",
                true,
            );
        };
        let events = body.get("events").cloned().unwrap_or_default();
        let Ok(events) = serde_json::from_value::<Vec<prompt_box::DictationEvent>>(events) else {
            return self.refuse(
                request,
                400,
                "bad_request",
                "events is a list of {sequence, kind, utterance, revision, text}",
                true,
            );
        };
        if events.len() > DICTATION_EVENTS_CAP
            || events.iter().any(|e| e.text.len() > SEND_BYTES_CAP)
        {
            return self.refuse(
                request,
                400,
                "bad_request",
                "at most 256 events of at most 16 KiB each",
                true,
            );
        }
        let editor =
            prompt_box::ensure_editor(&self.core, &self.services, &mut self.ui_state, &record);
        prompt_box::apply_dictation(editor, recognizer, &events);
        self.draft_reply(&record)
    }

    /// Every draft a device could hold, fingerprinted against the
    /// last pass: one `changed {draft}` per editor that moved, at most
    /// a few times a second while dictation revises it. Only while
    /// someone is subscribed, since nothing else reads the events.
    fn draft_pass(&mut self) {
        if self.remote.is_none() || self.subscribers.is_empty() {
            return;
        }
        if self
            .last_draft_pass
            .is_some_and(|t| t.elapsed() < DRAFT_PASS_EVERY)
        {
            return;
        }
        self.last_draft_pass = Some(Instant::now());
        let editors = &self.ui_state.prompt_boxes.editors;
        self.draft_marks.retain(|id, _| editors.contains_key(id));
        let mut moved = Vec::new();
        for (id, editor) in editors {
            let mark = prompt_box::draft_mark(editor);
            if self.draft_marks.get(id) != Some(&mark) {
                self.draft_marks.insert(*id, mark);
                moved.push(*id);
            }
        }
        if moved.is_empty() {
            return;
        }
        self.revision += 1;
        let at_ms = epoch_millis(SystemTime::now());
        for id in moved {
            let text = self.event_json("changed", Some(("draft", &id.0.to_string())), at_ms);
            self.broadcast(&text, false);
        }
    }

    /// Who this host is, to anyone: nothing in it is a secret and
    /// nothing in it is trusted until the key is pinned.
    fn remote_info(&self) -> RemoteReply {
        let Some(identity) = self.core.identity() else {
            return RemoteReply::new(
                503,
                self.error_json("no_identity", "the host has no identity yet"),
                false,
            );
        };
        let body = serde_json::json!({
            "server_id": identity.server_id,
            "server_name": identity.server_name,
            "protocol": REMOTE_PROTOCOL,
            "capabilities": ["read", "control", "draft", "scope", "tickets", "evidence", "create"],
            "spki_sha256": identity.spki_sha256,
            "version": self.core.version(),
            "limits": {
                "body_bytes": BODY_CAP,
                "string_bytes": STRING_CAP,
                "notes_bytes": LONG_TEXT_CAP,
                "screen_lines": SCREEN_LINES_CAP,
                "run_lines": RUN_LINES_CAP,
                "conversation_turns": PAGE_TURNS_CAP,
                "turn_bytes": TURN_BYTES_CAP,
                "activity_bytes": ACTIVITY_BYTES_CAP,
                "send_bytes": SEND_BYTES_CAP,
                "op_chars": OP_CAP,
                "file_bytes": FILE_BYTES_CAP,
            },
            "pairing_open": self.core.pairing_at(self.clock()).is_some(),
        });
        self.envelope(200, &body, false)
    }

    /// The pairing handshake: inside the window, with the host's own
    /// key hash (so a request relayed through a substituted
    /// certificate fails even with the code) and the code. A match
    /// makes a token the device keeps and the host stores only the hash
    /// of; a mismatch counts against the address it came from, and an
    /// address with three is refused for the rest of the window.
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
        if request.peer.is_some_and(|ip| window.is_locked(ip)) {
            return self.refuse(
                request,
                403,
                "pairing_locked",
                "too many wrong codes from this address; ask the owner to reopen pairing",
                false,
            );
        }
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
        // A sender with no key never saw the QR; it is refused without
        // spending an attempt, so an empty knock cannot lock anyone out.
        if field("spki_sha256").is_empty() {
            return self.refuse(request, 400, "bad_request", "the body has no key", false);
        }
        if field("spki_sha256") != identity.spki_sha256 {
            self.pairing_failed(request.peer);
            return self.refuse(
                request,
                403,
                "wrong_key",
                "this is not the host whose code was scanned",
                false,
            );
        }
        if field("code").to_ascii_uppercase() != window.code {
            self.pairing_failed(request.peer);
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
        // A browser asks for its token as a cookie its script cannot
        // read; the body then leaves the token out, so the page never
        // holds it.
        let cookie = field("transport") == "cookie";
        let mut reply = serde_json::json!({
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
        if cookie {
            let mut out = self.envelope(200, &reply, false);
            out.headers.push((
                "Set-Cookie",
                format!(
                    "{}={token}; Max-Age={COOKIE_MAX_AGE_SECS}; Path=/; Secure; HttpOnly; SameSite=Strict",
                    remote_cookie_name(identity.server_id)
                ),
            ));
            out
        } else {
            reply["token"] = serde_json::Value::String(token);
            self.envelope(200, &reply, false)
        }
    }

    /// The device asking, as the host knows it: what a browser cannot
    /// otherwise learn, since its credential is a cookie it cannot read.
    fn remote_me(&self, device: &PairedDevice) -> RemoteReply {
        let identity = self.core.identity();
        let body = serde_json::json!({
            "device_id": device.id,
            "name": device.name,
            "scope": device.scope,
            "server_id": identity.map(|i| i.server_id),
            "server_name": identity.map(|i| i.server_name.as_str()),
        });
        self.envelope(200, &body, true)
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
        self.remote_json(request, value, &[("notes", notes_cap)])
    }

    /// A content reply at the revision its key holds.
    fn envelope_at(&self, revision: u64, body: &serde_json::Value) -> RemoteReply {
        let text = serde_json::json!({
            "epoch": self.epoch,
            "revision": revision,
            "body": body,
        });
        RemoteReply::new(200, text.to_string().into_bytes(), true)
    }

    fn envelope(&self, status: u16, body: &serde_json::Value, authenticated: bool) -> RemoteReply {
        let text = serde_json::json!({
            "epoch": self.epoch,
            "revision": self.revision,
            "body": body,
        });
        RemoteReply::new(status, text.to_string().into_bytes(), authenticated)
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

    /// A refusal, logged by method, path and code only, at debug; the
    /// info line is the minute's count per code (`flush_refusals`).
    fn refuse(
        &self,
        request: &RemoteRequest,
        status: u16,
        code: &str,
        message: &str,
        authenticated: bool,
    ) -> RemoteReply {
        log::debug!(
            "remote: {} {} refused: {code}",
            request.method,
            request.path
        );
        self.count_refusal(code);
        RemoteReply::new(status, self.error_json(code, message), authenticated)
    }

    fn count_refusal(&self, code: &str) {
        let now = self.clock().mono;
        self.refusals.borrow_mut().count(code, 1, now);
    }

    /// Takes the listener's 429 count and, once a minute, logs the
    /// refusals per cause.
    fn flush_refusals(&self) {
        let now = self.clock().mono;
        let mut refusals = self.refusals.borrow_mut();
        if let Some(remote) = &self.remote {
            refusals.count("rate_limited", remote.take_rate_limited(), now);
        }
        for line in refusals.flush(now) {
            log::info!("{line}");
        }
    }

    /// Counts a failed `/pair` against its source. The core's notice
    /// is what tells the owner about a lockout; the info line is for
    /// the log, once per source per window. A request with no peer
    /// exists only when built by hand, so it counts without locking.
    fn pairing_failed(&mut self, peer: Option<IpAddr>) {
        let Some(ip) = peer else {
            self.dispatch_inner(AppAction::PairingFailed);
            return;
        };
        self.dispatch_inner(AppAction::PairingFailedFrom(ip));
        if self.core.pairing().is_some_and(|p| p.is_locked(ip)) {
            log::info!("remote: pairing locked out {ip}");
        }
    }

    /// One request from a tool on `control.sock`, one reply.
    fn serve(&mut self, request: &wire::Request) -> wire::Reply {
        self.serve_from(request, Origin::Tool)
    }

    /// One request, one reply. A query reads; a command runs through the
    /// core under its operation id, and its reply is logged so a repeat
    /// of the same id is answered from the log without running again.
    /// The owner's `session.send` becomes `OwnerSend`, which answers a
    /// standing ask; a tool's stays `SendInput`, which leaves it.
    fn serve_from(&mut self, request: &wire::Request, origin: Origin) -> wire::Reply {
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
            Ok(ControlAction::SendInput { id, text }) if origin == Origin::Owner => {
                ControlAction::OwnerSend { id, text }
            }
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
/// The longest name a device gives a session it opens.
const NAME_CAP: usize = 128;
/// The most speech events one dictation request carries.
const DICTATION_EVENTS_CAP: usize = 256;
/// How often the drafts are fingerprinted for `changed` events.
const DRAFT_PASS_EVERY: Duration = Duration::from_millis(150);
/// The most of a session's notes, or a ticket's notes and body, read
/// alone.
const LONG_TEXT_CAP: usize = 64 * 1024;
/// The most of a ticket's file a device reads as text: a plan or a
/// review's findings run long, and a device shows them whole.
const FILE_CAP: usize = 256 * 1024;
/// The most of an evidence image the port serves, as base64 inside the
/// body cap; a screenshot is well under it.
const IMAGE_CAP: u64 = 700 * 1024;
/// How long a browser keeps its pairing cookie: 400 days, the most a
/// browser honours.
const COOKIE_MAX_AGE_SECS: u64 = 400 * 24 * 60 * 60;

/// The cookie a browser paired with this host keeps its token in.
/// Cookies ignore ports, so two hosts on one address share a jar; the
/// name carries the server id so neither overwrites the other's, and
/// the id survives a key rotation. `__Host-` has the browser refuse it
/// unless it is `Secure`, `Path=/` and bound to this host alone.
fn remote_cookie_name(server_id: uuid::Uuid) -> String {
    format!("__Host-sb-{server_id}")
}

/// Whether a browser's `Origin` is the page this port served: `https`
/// and the `Host` the request was sent to. Without either header it is
/// not.
fn same_origin(request: &RemoteRequest) -> bool {
    match (&request.origin, &request.host) {
        (Some(origin), Some(host)) => origin.strip_prefix("https://") == Some(host.as_str()),
        _ => false,
    }
}

/// The ticket a call to Dispatch is about.
fn body_ticket(body: &Body) -> Option<&str> {
    match body {
        Body::Decide { ticket, .. }
        | Body::Resume { ticket }
        | Body::Close { ticket, .. }
        | Body::Events { ticket, .. }
        | Body::Artifact { ticket, .. } => Some(ticket),
        _ => None,
    }
}

/// Cut every string in `value` to its cap, a field named in `longs` to
/// the cap beside its name, at any depth, and the rest to `STRING_CAP`,
/// naming each cut field in a `truncated` list on its object, so no
/// field leaves unbounded and a client can tell a short value from a
/// cut one. The table is per read because one name means different
/// things in different replies: a file's `text` is the file, an event's
/// is a line.
fn bound_strings(value: &mut serde_json::Value, longs: &[(&str, usize)]) {
    match value {
        serde_json::Value::Object(map) => {
            let mut cut = Vec::new();
            for (name, field) in map.iter_mut() {
                match field {
                    serde_json::Value::String(s) => {
                        let cap = longs
                            .iter()
                            .find(|(long, _)| long == name)
                            .map_or(STRING_CAP, |&(_, cap)| cap);
                        if s.len() > cap {
                            let mut end = cap;
                            while !s.is_char_boundary(end) {
                                end -= 1;
                            }
                            s.truncate(end);
                            cut.push(name.clone());
                        }
                    }
                    other => bound_strings(other, longs),
                }
            }
            if !cut.is_empty() {
                map.insert("truncated".into(), serde_json::json!(cut));
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                bound_strings(item, longs);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    use super::{RefusalSummary, SwitchboardApp};
    use crate::adapters::fakes;
    use crate::content::redact;
    use crate::core::model::{
        EnvSet, EnvVar, Project, ProjectEnv, ProjectId, SessionKind, SpaceId, Workspace,
    };
    use crate::core::{AppAction, ControlAction, Launch};
    use crate::core::{RECORD_TOKEN_ENV, token_hash};
    use crate::ports::host::SpawnSpec;

    /// A ticket read alone keeps its notes and body to the long cap,
    /// while a `text` in the same reply, an event's, stays short.
    #[test]
    fn bound_strings_gives_the_long_cap_only_to_the_fields_named() {
        let long = "n".repeat(super::LONG_TEXT_CAP);
        let mut value = serde_json::json!({
            "notes": long.clone(),
            "body": long.clone(),
            "events": [{ "text": long.clone() }],
        });
        super::bound_strings(
            &mut value,
            &[
                ("notes", super::LONG_TEXT_CAP),
                ("body", super::LONG_TEXT_CAP),
            ],
        );
        assert_eq!(value["notes"], long.as_str());
        assert_eq!(value["body"], long.as_str());
        assert!(value["truncated"].is_null(), "{}", value["truncated"]);
        let event = &value["events"][0];
        assert_eq!(event["text"].as_str().unwrap().len(), super::STRING_CAP);
        assert_eq!(event["truncated"], serde_json::json!(["text"]));
    }

    /// With no table, as the ticket list is bounded, every string is
    /// cut to the short cap.
    #[test]
    fn bound_strings_with_no_table_cuts_every_string_short() {
        let mut value = serde_json::json!({ "body": "b".repeat(20 * 1024) });
        super::bound_strings(&mut value, &[]);
        assert_eq!(value["body"].as_str().unwrap().len(), super::STRING_CAP);
        assert_eq!(value["truncated"], serde_json::json!(["body"]));
    }

    /// Refusals are counted per cause and come out as one line each
    /// once a minute has passed since the first, then start again.
    #[test]
    fn refusals_are_summed_per_cause_and_logged_once_a_minute() {
        let at = Duration::from_secs;
        let mut summary = RefusalSummary::default();
        assert!(summary.flush(at(600)).is_empty(), "nothing counted");
        summary.count("wrong_code", 1, at(10));
        summary.count("rate_limited", 0, at(11));
        summary.count("wrong_code", 1, at(20));
        summary.count("rate_limited", 4, at(30));
        assert!(summary.flush(at(69)).is_empty(), "not a minute yet");
        assert_eq!(
            summary.flush(at(70)),
            vec![
                "remote: 4 refused rate_limited in the last minute",
                "remote: 2 refused wrong_code in the last minute",
            ]
        );
        assert!(summary.flush(at(200)).is_empty(), "reset after a flush");
        summary.count("unauthorized", 1, at(300));
        assert!(summary.flush(at(359)).is_empty(), "the minute starts again");
        assert_eq!(summary.flush(at(360)).len(), 1);
    }

    /// Two sets that name the same secret each keep their own value
    /// in the policy ticket text is masked with, not only the last.
    #[test]
    fn ticket_policies_mask_each_sets_value_of_a_shared_name() {
        let services = fakes::services();
        let secrets = services.secrets.clone();
        let mut app = SwitchboardApp::with_services(services);
        let set = |name: &str, secret: bool| EnvSet {
            name: name.into(),
            vars: vec![EnvVar {
                name: "API_KEY".into(),
                value: String::new(),
                secret,
            }],
            aws: None,
        };
        // `plain` shares the name with a secret and holds no value.
        let settings = crate::core::model::Settings {
            env_sets: vec![set("staging", true), set("prod", true), set("plain", false)],
            ..crate::core::model::Settings::default()
        };
        app.core_mut_for_seeding().dispatch(
            AppAction::StoreLoaded(Ok(crate::ports::store::Loaded {
                workspaces: Vec::new(),
                notices: Vec::new(),
                settings,
                views: crate::core::model::Views::default(),
            })),
            crate::core::Clock::at(0),
        );
        for (name, value) in [("staging", "s-1"), ("prod", "p-2")] {
            secrets
                .set(
                    &crate::core::SecretScope::Set(name.into()).account("API_KEY"),
                    value,
                )
                .unwrap();
        }
        let mut masked: Vec<String> = app
            .ticket_policy_specs()
            .iter()
            .flat_map(|spec| {
                crate::content::resolve_policy(spec, &*secrets)
                    .unwrap()
                    .secrets
            })
            .map(|(_, value)| value)
            .collect();
        masked.sort();
        masked.dedup();
        assert_eq!(masked, ["p-2", "s-1"]);
    }

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
