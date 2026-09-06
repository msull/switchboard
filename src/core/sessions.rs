//! Launching, returning to, and resuming sessions. "Return" is
//! idempotent: a record with a flight in progress ignores further
//! returns, so two clicks never make two processes for one record.

use std::path::PathBuf;

use crate::core::action::{AppCore, Clock, Effect, Flight, FlightKind, Out};
use crate::core::model::{
    Activity, AgentKind, CardLayout, Launch, ProjectId, RecordId, ResumeHandle, SessionKind,
    SessionRecord,
};
use crate::core::reconcile::env_with_record_id;
use crate::ports::agent::AgentLaunch;
use crate::ports::host::{HostId, HostStatus, Liveness, SpawnSpec};

impl AppCore {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new_session(
        &mut self,
        project: ProjectId,
        name: String,
        kind: SessionKind,
        cwd: PathBuf,
        launch: Launch,
        now: Clock,
        out: &mut Out,
    ) {
        if let Some(reason) = self.host_error.clone() {
            self.error(format!("cannot start {name}: {reason}"));
            return;
        }
        let Some(workspace) = self.workspaces.iter_mut().find(|w| w.project.id == project) else {
            self.error("cannot start a session: unknown project");
            return;
        };
        let order = workspace
            .sessions
            .iter()
            .map(|s| s.layout.order + 1)
            .max()
            .unwrap_or(0);
        let id = RecordId::new();
        workspace.sessions.push(SessionRecord {
            id,
            project,
            name,
            kind,
            cwd,
            launch,
            env_profile: None,
            created: now.wall,
            last_seen: now.wall,
            notes: String::new(),
            resume: None,
            autostart: false,
            layout: CardLayout { order, group: None },
            activity: Activity::Unknown,
            last_event_at: None,
            last_exit: None,
            not_resumable: false,
            scrollback: None,
        });
        out.touch(project);
        self.launch_fresh(id, now, out);
    }

    /// Starts a record from scratch: agents get a composed launch (Codex
    /// waits its turn), everything else spawns its own `Launch`.
    fn launch_fresh(&mut self, id: RecordId, now: Clock, out: &mut Out) {
        let Some(record) = self.session(id) else {
            return;
        };
        match record.kind {
            SessionKind::Agent(AgentKind::Codex) if self.codex_busy() => {
                self.start_flight(id, FlightKind::Launch, now);
                self.codex_queue.push(id);
            }
            SessionKind::Agent(kind) => {
                let (name, cwd) = (record.name.clone(), record.cwd.clone());
                self.start_flight(id, FlightKind::Launch, now);
                out.push(Effect::PrepareLaunch {
                    id,
                    kind,
                    name,
                    cwd,
                });
            }
            SessionKind::Command | SessionKind::Service | SessionKind::Shell => {
                self.spawn_record(id, FlightKind::Launch, now, out);
            }
        }
    }

    /// Codex ids are discovered from the rollout file a launch creates,
    /// so only one Codex launch may be unbound at a time.
    fn codex_busy(&self) -> bool {
        self.codex_pending.is_some()
            || self.in_flight.iter().any(|f| {
                f.kind == FlightKind::Launch
                    && !self.codex_queue.contains(&f.id)
                    && self
                        .session(f.id)
                        .is_some_and(|s| s.kind == SessionKind::Agent(AgentKind::Codex))
            })
    }

    /// Lets the next queued Codex record launch once the previous one is
    /// bound (or has failed).
    fn advance_codex_queue(&mut self, now: Clock, out: &mut Out) {
        if self.codex_busy() || self.codex_queue.is_empty() {
            return;
        }
        let id = self.codex_queue.remove(0);
        let Some(record) = self.session(id) else {
            return;
        };
        out.push(Effect::PrepareLaunch {
            id,
            kind: AgentKind::Codex,
            name: record.name.clone(),
            cwd: record.cwd.clone(),
        });
        // Discovery looks for rollouts newer than the launch, which is now,
        // not when the record was queued.
        self.start_flight(id, FlightKind::Launch, now);
    }

    pub(super) fn start_flight(&mut self, id: RecordId, kind: FlightKind, now: Clock) {
        self.in_flight.retain(|f| f.id != id);
        self.in_flight.push(Flight {
            id,
            kind,
            started: now.wall,
        });
    }

    fn end_flight(&mut self, id: RecordId) -> Option<Flight> {
        let pos = self.in_flight.iter().position(|f| f.id == id)?;
        Some(self.in_flight.remove(pos))
    }

    fn flight(&self, id: RecordId) -> Option<Flight> {
        self.in_flight.iter().copied().find(|f| f.id == id)
    }

    pub(super) fn return_to_session(&mut self, id: RecordId, now: Clock, out: &mut Out) {
        if self.is_in_flight(id) {
            return;
        }
        let Some(record) = self.session(id) else {
            return;
        };
        if let Some(reason) = self.host_error.clone() {
            let name = record.name.clone();
            self.error(format!("cannot return to {name}: {reason}"));
            return;
        }
        match self.host_status(id).map(|h| &h.liveness) {
            // Warm, or dead but the pane is kept: attaching shows it either way.
            Some(Liveness::Running { .. } | Liveness::Exited { .. }) => {
                out.push(attach(record));
            }
            Some(Liveness::Missing) | None => match (record.kind, &record.resume) {
                (SessionKind::Agent(_), Some(handle)) if !record.not_resumable => {
                    let handle = handle.clone();
                    self.start_flight(id, FlightKind::Preflight, now);
                    out.push(Effect::CheckTranscript { id, handle });
                }
                _ => {
                    // No conversation to resume: a fresh agent in the same
                    // directory, or a shell/command/service run again.
                    if record.not_resumable {
                        self.edit_session(id, out, |s| s.not_resumable = false);
                    }
                    self.launch_fresh(id, now, out);
                }
            },
        }
    }

    pub(super) fn transcript_checked(
        &mut self,
        id: RecordId,
        exists: bool,
        now: Clock,
        out: &mut Out,
    ) {
        if self.flight(id).map(|f| f.kind) != Some(FlightKind::Preflight) {
            return;
        }
        let Some(record) = self.session(id) else {
            self.end_flight(id);
            return;
        };
        let (name, cwd) = (record.name.clone(), record.cwd.clone());
        match record.resume.clone() {
            Some(handle) if exists => {
                self.start_flight(id, FlightKind::Resume, now);
                out.push(Effect::PrepareResume {
                    id,
                    handle,
                    name,
                    cwd,
                });
            }
            _ => {
                self.end_flight(id);
                self.mark_not_resumable(id, now, out);
            }
        }
    }

    fn mark_not_resumable(&mut self, id: RecordId, now: Clock, out: &mut Out) {
        self.edit_session(id, out, |s| s.not_resumable = true);
        let name = self.session_name(id);
        self.info(
            format!("{name} is not resumable; start a fresh session"),
            now,
        );
    }

    pub(super) fn launch_prepared(
        &mut self,
        id: RecordId,
        result: Result<AgentLaunch, String>,
        now: Clock,
        out: &mut Out,
    ) {
        if !self.is_in_flight(id) {
            return;
        }
        match result {
            Ok(launch) => {
                let Some(record) = self.session_mut(id) else {
                    self.end_flight(id);
                    return;
                };
                if let Some(handle) = launch.resume {
                    record.resume = Some(handle);
                    let project = record.project;
                    out.touch(project);
                }
                let spec = SpawnSpec {
                    id: HostId(id.host_name()),
                    cwd: record.cwd.clone(),
                    command: Some(launch.argv),
                    env: env_with_record_id(launch.env, id),
                    scrollback: None,
                };
                out.push(Effect::Spawn { id, spec });
            }
            Err(e) => {
                let name = self.session_name(id);
                self.error(format!("could not prepare {name}: {e}"));
                self.end_flight(id);
                self.advance_codex_queue(now, out);
            }
        }
    }

    pub(super) fn spawned(
        &mut self,
        id: RecordId,
        result: Result<(), String>,
        now: Clock,
        out: &mut Out,
    ) {
        let Some(flight) = self.flight(id) else {
            return;
        };
        match result {
            Ok(()) => {
                self.edit_session(id, out, |s| {
                    s.last_seen = now.wall;
                    s.activity = Activity::Unknown;
                    s.last_exit = None;
                });
                // Until the next host poll, treat the session as running so
                // a "return" in that window attaches instead of spawning
                // again. The poll replaces this placeholder.
                let host_id = HostId(id.host_name());
                if !self.host.iter().any(|h| h.id == host_id) {
                    self.host.push(HostStatus {
                        id: host_id,
                        liveness: Liveness::Running {
                            pid: 0,
                            command: String::new(),
                        },
                        cwd: None,
                        last_activity: Some(now.wall),
                        title: None,
                    });
                }
                let Some(record) = self.session(id) else {
                    self.end_flight(id);
                    return;
                };
                let needs_discovery =
                    record.kind == SessionKind::Agent(AgentKind::Codex) && record.resume.is_none();
                if let SessionKind::Agent(kind) = record.kind {
                    out.push(attach(record));
                    if needs_discovery {
                        out.push(Effect::Discover {
                            id,
                            kind,
                            cwd: record.cwd.clone(),
                            since: flight.started,
                        });
                        self.codex_pending = Some(id);
                        return;
                    }
                }
                self.end_flight(id);
                self.advance_codex_queue(now, out);
            }
            Err(e) => {
                let name = self.session_name(id);
                self.error(format!("could not start {name}: {e}"));
                self.end_flight(id);
                if flight.kind == FlightKind::Resume {
                    self.edit_session(id, out, |s| s.not_resumable = true);
                }
                self.advance_codex_queue(now, out);
            }
        }
    }

    pub(super) fn discovered(
        &mut self,
        id: RecordId,
        result: Result<Option<ResumeHandle>, String>,
        now: Clock,
        out: &mut Out,
    ) {
        if self.codex_pending == Some(id) {
            self.codex_pending = None;
        }
        self.end_flight(id);
        match result {
            // One rollout belongs to one record; binding it twice would
            // resume the wrong conversation, so refuse and say so.
            Ok(Some(handle)) if self.bound_elsewhere(id, &handle) => {
                let name = self.session_name(id);
                self.error(format!(
                    "Codex session id unknown for {name}: the only new rollout belongs to another card"
                ));
            }
            Ok(Some(handle)) => self.edit_session(id, out, |s| s.resume = Some(handle)),
            Ok(None) => {
                let name = self.session_name(id);
                self.error(format!("Codex session id unknown for {name}"));
            }
            Err(e) => {
                let name = self.session_name(id);
                self.error(format!("could not discover Codex session for {name}: {e}"));
            }
        }
        self.advance_codex_queue(now, out);
    }
}

impl AppCore {
    /// Another record already resumes with this handle's provider id.
    fn bound_elsewhere(&self, id: RecordId, handle: &ResumeHandle) -> bool {
        let provider = handle.provider_id();
        self.workspaces.iter().flat_map(|w| &w.sessions).any(|s| {
            s.id != id
                && s.resume
                    .as_ref()
                    .is_some_and(|h| h.provider_id() == provider)
        })
    }
}

fn attach(record: &SessionRecord) -> Effect {
    Effect::Attach {
        id: record.id,
        host: HostId(record.id.host_name()),
        title: record.id.host_name(),
        cwd: record.cwd.clone(),
    }
}
