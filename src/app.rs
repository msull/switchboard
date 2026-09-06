//! Orchestration: owns [`AppCore`] and the adapters. Maps effects to
//! adapter calls and feeds results back as actions; polls the host and
//! the event log on a timer; keeps the UI's captions fresh. Rendering
//! lives in [`crate::ui`].

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use crate::adapters::hooks::WakeSocket;
use crate::core::{AgentKind, AppAction, AppCore, Clock, Effect, RecordId, SessionKind, View};
use crate::ports::agent::AgentLauncher;
use crate::ports::events::EventSource;
use crate::ports::host::{HostId, Liveness, ProcessHost};
use crate::ports::opener::Opener;
use crate::ports::store::{Store, StoreError};
use crate::ports::transcript::TranscriptReader;
use crate::ui::UiState;

/// How often the host is listed and the event log read.
const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// How often card captions and the session snapshot are refreshed.
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
    /// The hook helper's wake-up socket; `None` in tests.
    pub wake: Option<WakeSocket>,
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
    last_caption: Option<Instant>,
    discoveries: Vec<Discovery>,
    /// Transient state owned by the UI (dialog drafts, embedded terminals).
    /// Nothing in here is persisted or read by the core.
    pub ui_state: UiState,
    /// When set, every dispatched action is also appended to `dispatched`.
    /// UI tests use this to assert what a click did.
    pub record_actions: bool,
    pub dispatched: Vec<AppAction>,
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
            last_caption: None,
            discoveries: Vec::new(),
            ui_state: UiState::default(),
            record_actions: false,
            dispatched: Vec::new(),
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

    /// Dispatch without recording: effect results are consequences, not
    /// what the UI asked for.
    fn dispatch_inner(&mut self, action: AppAction) {
        let now = self.clock();
        let effects = self.core.dispatch(action, now);
        for effect in effects {
            if let Some(result) = self.run_effect(effect) {
                self.dispatch_inner(result);
            }
        }
    }

    /// The persistence effects; only a workspace save reports back.
    fn run_store_effect(&self, effect: Effect) -> Option<AppAction> {
        let store = &self.services.store;
        match effect {
            Effect::SaveSettings(settings) => {
                store
                    .save_settings(&settings)
                    .unwrap_or_else(|e| log::error!("save settings failed: {e}"));
                None
            }
            Effect::Save(ws) => {
                let id = ws.project.id;
                let result = store.save(&ws);
                if let Err(e) = &result {
                    log::error!("save failed: {e}");
                }
                Some(AppAction::SaveFinished(id, result))
            }
            Effect::Delete(id) => {
                if let Err(e) = store.delete(id) {
                    log::error!("delete failed: {e}");
                }
                None
            }
            _ => None,
        }
    }

    /// Performs one effect; returns the action reporting its result, or
    /// `None` when the result arrives later (discovery) or has no report.
    fn run_effect(&mut self, effect: Effect) -> Option<AppAction> {
        let s = &self.services;
        match effect {
            Effect::SaveSettings(_) | Effect::Save(_) | Effect::Delete(_) => {
                self.run_store_effect(effect)
            }
            Effect::PrepareLaunch {
                id,
                kind,
                name,
                cwd,
            } => Some(AppAction::LaunchPrepared {
                id,
                result: s.agents.prepare_launch(kind, id, &name, &cwd),
            }),
            Effect::PrepareResume {
                id,
                handle,
                name,
                cwd,
            } => Some(AppAction::LaunchPrepared {
                id,
                result: s.agents.prepare_resume(&handle, id, &name, &cwd),
            }),
            Effect::CheckTranscript { id, handle } => Some(AppAction::TranscriptChecked {
                id,
                exists: s.agents.transcript_exists(&handle),
            }),
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
                spec.scrollback = Some(self.scrollback_path(&spec.id));
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
            Effect::SendInput { host, text } => {
                self.send_input(&host, &text);
                None
            }
            Effect::Kill(host) => {
                if let Err(e) = s.host.kill(&host) {
                    log::warn!("kill {} failed: {e}", host.0);
                }
                None
            }
            Effect::Forget(host) => {
                let path = self.scrollback_path(&host);
                if let Err(e) = std::fs::remove_file(&path)
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    log::warn!("remove {}: {e}", path.display());
                }
                None
            }
            Effect::OpenPath(path) => {
                if let Err(e) = s.opener.open_default(&path) {
                    log::warn!("open failed: {e}");
                }
                None
            }
            Effect::OpenInEditor { editor, path } => {
                if let Err(e) = s.opener.open_editor(&editor, &path) {
                    log::warn!("open in editor failed: {e}");
                }
                None
            }
            Effect::Reveal(path) => {
                if let Err(e) = s.opener.reveal(&path) {
                    log::warn!("reveal failed: {e}");
                }
                None
            }
        }
    }

    /// Type `text` into the pane, then Enter as a separate write after a
    /// pause: a newline inside the same chunk reads as a pasted line break
    /// to a TUI, not as submit.
    fn send_input(&self, host: &HostId, text: &str) {
        let r = self
            .services
            .host
            .write(host, text.as_bytes())
            .and_then(|()| {
                std::thread::sleep(Duration::from_millis(150));
                self.services.host.write(host, b"\r")
            });
        if let Err(e) = r {
            log::warn!("send input to {} failed: {e}", host.0);
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
        let path = self.scrollback_path(host);
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

    /// Where the host pipes a session's raw output.
    fn scrollback_path(&self, host: &HostId) -> PathBuf {
        self.services
            .store
            .data_dir()
            .join("scrollback")
            .join(format!("{}.vt", host.0))
    }

    /// Captions for cards on screen and the snapshot for the open session.
    fn refresh_captions(&mut self) {
        let view = self.core.view();
        if let View::Session(id) = view {
            self.refresh_conversation(id);
        }
        let ids: Vec<RecordId> = match view {
            View::Switchboard => self
                .core
                .all_sessions_sorted()
                .iter()
                .map(|s| s.id)
                .collect(),
            View::Board(p) => self.core.sessions_sorted(p).iter().map(|s| s.id).collect(),
            View::Session(id) => vec![id],
            View::Document(..) => Vec::new(),
        };
        for id in ids {
            let running = self
                .core
                .host_status(id)
                .is_some_and(|h| !matches!(h.liveness, Liveness::Missing));
            let host = HostId(id.host_name());
            if !running {
                self.cold_scrollback(id, &host, matches!(view, View::Session(sid) if sid == id));
                continue;
            }
            let wants_snapshot = matches!(view, View::Session(sid) if sid == id)
                && self
                    .core
                    .session(id)
                    .is_some_and(|s| matches!(s.kind, SessionKind::Agent(_)));
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
        self.refresh_captions();
    }

    fn pump(&mut self) {
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
    }
}

impl eframe::App for SwitchboardApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.dispatch(AppAction::Tick);
        if self.last_poll.is_some() {
            // Only after `start`: tests never poll.
            self.pump();
            ui.ctx().request_repaint_after(POLL_INTERVAL);
        }
        crate::ui::draw(self, ui);
    }
}
