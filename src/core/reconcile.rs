//! Startup reconcile and host polls. Records plus the host's live pane
//! list go in; card states come out, and the only launches are trusted
//! `autostart` services whose pane is gone. Agents are never resumed
//! here: a resume costs money, so it waits for a click.

use crate::core::action::{AppCore, Clock, Effect, FlightKind, Out, View};
use crate::core::model::{AgentKind, Launch, RecordId, SavedView, SessionKind, SessionRecord};
use crate::ports::host::{HostId, HostStatus, Liveness, SpawnSpec};
use crate::ports::store::{Loaded, StoreError};

/// An agent without hooks whose pane has printed nothing for this long is
/// shown idle: it is waiting at its prompt, or for the user.
pub const QUIET_AFTER: std::time::Duration = std::time::Duration::from_secs(20);

/// Injected into every pane so hooks and shells can report the record
/// they belong to without relying on cwd.
pub const RECORD_ID_ENV: &str = "SWITCHBOARD_RECORD_ID";

impl AppCore {
    pub(super) fn store_loaded(&mut self, result: Result<Loaded, StoreError>, out: &mut Out) {
        match result {
            Ok(loaded) => {
                self.workspaces = loaded.workspaces;
                self.settings = loaded.settings;
                self.views = loaded.views;
                self.restore_view();
                // Definition files are read before the first host poll, so
                // the reconcile already knows which entries are approved.
                for w in &self.workspaces {
                    out.push(super::definitions::read_config(
                        w.project.id,
                        w.project.root.clone(),
                    ));
                }
                for notice in loaded.notices {
                    if notice == StoreError::Locked {
                        self.read_only = true;
                    }
                    self.error(notice.to_string());
                }
            }
            Err(StoreError::Locked) => {
                self.read_only = true;
                self.error(StoreError::Locked.to_string());
            }
            Err(e) => self.error(format!("could not load workspaces: {e}")),
        }
        self.store_loaded = true;
        self.reconciled = false;
    }

    /// Reopen the screen the last run ended on, if what it showed still
    /// exists. Only the view moves: nothing is launched or resumed.
    fn restore_view(&mut self) {
        let view = match self.settings.last_view {
            SavedView::Board(id) if self.workspace(id).is_some() => View::Board(id),
            SavedView::Session(id) if self.session(id).is_some() => View::Session(id),
            SavedView::WorkingSet => match self.views.sets.first() {
                Some(set) => View::WorkingSet(set.id),
                None => return,
            },
            SavedView::Set(id) if self.working_set(id).is_some() => View::WorkingSet(id),
            SavedView::Workflow(id) if self.workflow(id).is_some() => View::Workflow(id),
            _ => return,
        };
        self.view_stack.push(view);
    }

    /// Replaces the host snapshot. The first poll after the store loaded
    /// is the reconcile, which also brings autostart services back.
    pub(super) fn host_listed(&mut self, statuses: Vec<HostStatus>, now: Clock, out: &mut Out) {
        self.host = statuses;
        self.quiet = self
            .workspaces
            .iter()
            .flat_map(|w| &w.sessions)
            .filter(|s| s.kind == SessionKind::Agent(AgentKind::Codex))
            .filter(|s| {
                self.host_status(s.id).is_some_and(|h| {
                    h.last_activity
                        .and_then(|t| now.wall.duration_since(t).ok())
                        .is_some_and(|quiet| quiet >= QUIET_AFTER)
                })
            })
            .map(|s| s.id)
            .collect();
        self.record_exit_codes(out);
        if self.store_loaded && !self.reconciled {
            self.reconciled = true;
            self.autostart_services(now, out);
        }
    }

    /// Writes exit codes the host reports back into the records, so a
    /// month-old card can still say how its process ended.
    fn record_exit_codes(&mut self, out: &mut Out) {
        let exited: Vec<_> = self
            .workspaces
            .iter()
            .flat_map(|w| &w.sessions)
            .filter_map(|s| match self.host_status(s.id) {
                Some(HostStatus {
                    liveness: Liveness::Exited { code },
                    ..
                }) if s.last_exit != *code => Some((s.id, *code)),
                _ => None,
            })
            .collect();
        for (id, code) in exited {
            self.edit_session(id, out, |s| s.last_exit = code);
        }
    }

    fn autostart_services(&mut self, now: Clock, out: &mut Out) {
        let cold: Vec<_> = self
            .workspaces
            .iter()
            .flat_map(|w| &w.sessions)
            .filter(|s| s.kind == SessionKind::Service && s.effective_autostart())
            .filter(|s| self.host_status(s.id).is_none() && !self.is_in_flight(s.id))
            .map(|s| s.id)
            .collect();
        for id in cold {
            self.spawn_record(id, FlightKind::Launch, now, out);
        }
    }

    /// Emits `Spawn` for a non-agent record (or an agent started without
    /// a composed launch) and marks it in flight.
    pub(super) fn spawn_record(
        &mut self,
        id: RecordId,
        kind: FlightKind,
        now: Clock,
        out: &mut Out,
    ) {
        if let Some(record) = self.session(id) {
            let spec = spawn_spec(record);
            self.start_flight(id, kind, now);
            out.push(Effect::Spawn { id, spec });
        }
    }
}

/// The host spec for a record's own `Launch`. Scrollback is left for the
/// app, which knows the data directory.
#[must_use]
pub fn spawn_spec(record: &SessionRecord) -> SpawnSpec {
    let command = match &record.launch {
        Launch::Shell => None,
        Launch::Argv(argv) => Some(argv.clone()),
        Launch::Command { command, shell } => {
            Some(vec![shell.clone(), "-lc".into(), command.clone()])
        }
    };
    SpawnSpec {
        id: HostId(record.id.host_name()),
        cwd: record.cwd.clone(),
        command,
        env: vec![(RECORD_ID_ENV.into(), record.id.0.to_string())],
        scrollback: None,
    }
}

/// `env` with the record id set exactly once (an agent launcher may have
/// added it already).
#[must_use]
pub fn env_with_record_id(mut env: Vec<(String, String)>, id: RecordId) -> Vec<(String, String)> {
    env.retain(|(k, _)| k != RECORD_ID_ENV);
    env.push((RECORD_ID_ENV.into(), id.0.to_string()));
    env
}
