//! Launching, returning to, and resuming sessions. "Return" is
//! idempotent: a record with a flight in progress ignores further
//! returns, so two clicks never make two processes for one record.

use std::path::PathBuf;

use crate::core::action::{AppCore, Clock, Effect, Flight, FlightKind, Out, View};
use crate::core::model::{
    Activity, AgentKind, CardLayout, Discarded, Launch, ProjectId, RecordId, ResumeHandle,
    SessionKind, SessionRecord,
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
            activity_reason: None,
            last_event_at: None,
            last_exit: None,
            not_resumable: false,
            scrollback: None,
            source: None,
            approved_hash: None,
            discard: None,
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
        // A defined record runs only the exact definition the user saw and
        // approved; the file may have been edited since.
        if !record.runnable() {
            let name = record.name.clone();
            self.error(format!(
                "{name} comes from .switchboard/project.json and is not approved; approve it in the Run tab"
            ));
            return;
        }
        // A fresh agent is a new conversation: a handle left from the old
        // one would stop Codex discovery from binding the new rollout.
        if matches!(record.kind, SessionKind::Agent(_)) && record.resume.is_some() {
            self.edit_session(id, out, |s| s.resume = None);
        }
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
    pub(super) fn advance_codex_queue(&mut self, now: Clock, out: &mut Out) {
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
        let Some(record) = self.session(id).cloned() else {
            return;
        };
        let record = &record;
        if let Some(reason) = self.host_error.clone() {
            let name = record.name.clone();
            self.error(format!("cannot return to {name}: {reason}"));
            return;
        }
        let liveness = self.host_status(id).map(|h| h.liveness.clone());
        if let Some(Liveness::Exited { .. }) = liveness {
            // A dead pane kept by the host is cold: clear it so the
            // resume or relaunch below can reuse the name.
            let host = HostId(id.host_name());
            out.push(Effect::Kill(host.clone()));
            self.host.retain(|h| h.id != host);
        }
        match liveness {
            Some(Liveness::Running { .. }) => {
                out.push(attach(record));
            }
            Some(Liveness::Exited { .. } | Liveness::Missing) | None => {
                match (record.kind, &record.resume) {
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
                }
            }
        }
    }

    /// Kill the pane if there is one, then launch the record fresh.
    /// Agents are not restarted this way (a restart would lose the
    /// conversation); for them it is a plain return.
    pub(super) fn restart_session(&mut self, id: RecordId, now: Clock, out: &mut Out) {
        if self.is_in_flight(id) {
            return;
        }
        let Some(record) = self.session(id) else {
            return;
        };
        if matches!(record.kind, SessionKind::Agent(_)) {
            self.return_to_session(id, now, out);
            return;
        }
        if let Some(reason) = self.host_error.clone() {
            let name = record.name.clone();
            self.error(format!("cannot restart {name}: {reason}"));
            return;
        }
        let host = HostId(id.host_name());
        if self.host_status(id).is_some() {
            out.push(Effect::Kill(host.clone()));
            self.host.retain(|h| h.id != host);
        }
        self.launch_fresh(id, now, out);
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

    /// Ask the shell for a provider-side copy of the conversation up to
    /// turn `before`. Only Claude Code sessions with a transcript can be
    /// cloned; anything else is a notice, not a record.
    pub(super) fn clone_session(
        &mut self,
        id: RecordId,
        before: usize,
        prompt: String,
        out: &mut Out,
    ) {
        if let Some(handle) = self.forkable(id, before, "clone") {
            out.push(Effect::CloneTranscript {
                source: id,
                handle,
                before,
                prompt,
            });
        }
    }

    /// The handle a copy up to `before` can be made from, or a notice
    /// saying why not (`verb` names the operation in it).
    fn forkable(&mut self, id: RecordId, before: usize, verb: &str) -> Option<ResumeHandle> {
        let record = self.session(id)?;
        let name = record.name.clone();
        match (&record.kind, record.resume.clone()) {
            (SessionKind::Agent(AgentKind::ClaudeCode), Some(handle))
                if handle.transcript().is_some() && before >= 1 =>
            {
                Some(handle)
            }
            (SessionKind::Agent(AgentKind::ClaudeCode), _) => {
                self.error(format!("cannot {verb} {name}: it has no transcript yet"));
                None
            }
            _ => {
                self.error(format!(
                    "cannot {verb} {name}: only Claude Code sessions can be {verb}d"
                ));
                None
            }
        }
    }

    /// The in-place counterpart of `clone_session`: the same copy, to
    /// become this record's own conversation.
    pub(super) fn discard_to(
        &mut self,
        id: RecordId,
        before: usize,
        prompt: String,
        out: &mut Out,
    ) {
        if let Some(handle) = self.forkable(id, before, "discard") {
            out.push(Effect::DiscardTranscript {
                id,
                handle,
                before,
                prompt,
            });
        }
    }

    /// The copy exists: the record resumes through it from now on and
    /// remembers the handle it replaced. A running agent is on the old
    /// conversation, so it is stopped; the session is cold until the user
    /// returns to it, with the chosen prompt primed.
    pub(super) fn transcript_discarded(
        &mut self,
        id: RecordId,
        before: usize,
        prompt: String,
        result: Result<ResumeHandle, String>,
        now: Clock,
        out: &mut Out,
    ) {
        let Some(record) = self.session(id).cloned() else {
            return;
        };
        let handle = match result {
            Ok(handle) => handle,
            Err(e) => {
                self.error(format!("could not discard in {}: {e}", record.name));
                return;
            }
        };
        let Some(previous) = record.resume else {
            return;
        };
        self.stop_if_running(id, out);
        self.edit_session(id, out, |s| {
            s.resume = Some(handle);
            s.not_resumable = false;
            s.discard = Some(Discarded {
                previous,
                before,
                prompt: prompt.clone(),
            });
        });
        self.primed.push((id, prompt));
        self.info(
            format!(
                "{}: conversation cut back to before turn {before}; Undo discard puts it back",
                record.name
            ),
            now,
        );
    }

    /// Back to the handle the discard replaced. Its file was never
    /// touched, so nothing is copied; the copy stays on disk unused.
    pub(super) fn undo_discard(&mut self, id: RecordId, now: Clock, out: &mut Out) {
        let Some(record) = self.session(id).cloned() else {
            return;
        };
        let Some(discarded) = record.discard else {
            self.error(format!("{}: nothing to undo", record.name));
            return;
        };
        self.stop_if_running(id, out);
        self.edit_session(id, out, |s| {
            s.resume = Some(discarded.previous);
            s.not_resumable = false;
            s.discard = None;
        });
        self.info(format!("{}: discard undone", record.name), now);
    }

    fn stop_if_running(&mut self, id: RecordId, out: &mut Out) {
        if let Some(status) = self.host_status(id) {
            out.push(Effect::Kill(status.id.clone()));
        }
    }

    /// The copy exists: a new cold record beside the source, resumable
    /// through the new handle, shown with the chosen prompt primed.
    pub(super) fn transcript_cloned(
        &mut self,
        source: RecordId,
        prompt: String,
        result: Result<ResumeHandle, String>,
        now: Clock,
        out: &mut Out,
    ) {
        let Some(record) = self.session(source).cloned() else {
            return;
        };
        let handle = match result {
            Ok(handle) => handle,
            Err(e) => {
                self.error(format!("could not clone {}: {e}", record.name));
                return;
            }
        };
        let Some(workspace) = self
            .workspaces
            .iter_mut()
            .find(|w| w.project.id == record.project)
        else {
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
            name: format!("{} clone", record.name),
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
            ..record
        });
        out.touch(record.project);
        self.primed.push((id, prompt));
        self.show(View::Session(id), now, out);
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
                    s.activity_reason = None;
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
                    // The window is the user's to open: most of the time
                    // the session is driven from here, so it stays closed
                    // unless asked for.
                    if self.settings.open_terminal_on_launch {
                        out.push(attach(record));
                    }
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
