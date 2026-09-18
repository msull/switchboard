//! Hook events -> record activity. Correlation never uses cwd: the
//! record id injected into the pane wins, then the provider's session
//! id. Delivery is append-first and replayed after a crash, so an event
//! no newer than the record's last applied one is ignored rather than
//! applied out of order.

use uuid::Uuid;

use crate::core::action::{AppCore, Clock, Out};
use crate::core::model::{Activity, RecordId, ResumeHandle};
use crate::ports::events::{EventKind, SessionEvent};

impl AppCore {
    pub(super) fn apply_events(&mut self, events: Vec<SessionEvent>, now: Clock, out: &mut Out) {
        for event in events {
            if let Some(id) = self.match_event(&event) {
                self.apply_event(id, event, now, out);
            }
        }
    }

    /// The new Claude Code session id an event carries for a pane that
    /// Switchboard started, when it differs from the record's: `/clear`
    /// keeps the process and starts a fresh conversation under a new
    /// id, and the record must follow it or keep reading (and resuming)
    /// the old one. Only an event tied to the pane by record id counts;
    /// a provider id alone could be another process in the same cwd.
    fn rebound_session(
        record_id: RecordId,
        event: &SessionEvent,
        resume: Option<&ResumeHandle>,
    ) -> Option<Uuid> {
        if event.record_id != Some(record_id) {
            return None;
        }
        let new = Uuid::parse_str(event.provider_session_id.as_deref()?).ok()?;
        match resume {
            Some(ResumeHandle::ClaudeCode { session_id, .. }) if *session_id != new => Some(new),
            _ => None,
        }
    }

    fn match_event(&self, event: &SessionEvent) -> Option<RecordId> {
        if let Some(id) = event.record_id
            && self.session(id).is_some()
        {
            return Some(id);
        }
        let provider = event.provider_session_id.as_deref()?;
        self.workspaces
            .iter()
            .flat_map(|w| &w.sessions)
            .find(|s| {
                s.resume
                    .as_ref()
                    .is_some_and(|h| h.provider_id() == provider)
            })
            .map(|s| s.id)
    }

    fn apply_event(&mut self, id: RecordId, event: SessionEvent, now: Clock, out: &mut Out) {
        let Some(record) = self.session(id) else {
            return;
        };
        if record.last_event_at.is_some_and(|last| event.at <= last) {
            return;
        }
        let change = interpret(&event.kind);
        if let Some(new) = Self::rebound_session(id, &event, record.resume.as_ref()) {
            let name = record.name.clone();
            self.edit_session(id, out, |s| {
                s.resume = Some(ResumeHandle::ClaudeCode {
                    session_id: new,
                    transcript: event.transcript_path.clone(),
                });
                // The cut conversation is gone from the process too.
                s.discard = None;
            });
            self.info(
                format!("{name} started a new conversation; the old one stays on disk"),
                now,
            );
        }
        self.edit_session(id, out, |s| {
            s.last_event_at = Some(event.at);
            s.last_seen = s.last_seen.max(event.at);
            if let Some((activity, reason)) = change {
                s.activity = activity;
                s.activity_reason = reason;
            }
            if event.kind == EventKind::SessionStart
                && let Some(path) = event.transcript_path
                && let Some(handle) = &mut s.resume
                && handle.transcript().is_none()
            {
                match handle {
                    ResumeHandle::ClaudeCode { transcript, .. }
                    | ResumeHandle::Codex { transcript, .. } => *transcript = Some(path),
                }
            }
        });
    }
}

/// What an event says the session is doing, with a few words of why
/// when it is waiting; `None` for notifications the card does not care
/// about. Notification kinds are the ones Claude Code sends (see
/// `spikes/03-session-state`); an unknown kind changes nothing.
fn interpret(kind: &EventKind) -> Option<(Activity, Option<String>)> {
    let waiting = |why: &str| Some((Activity::WaitingOnYou, Some(why.to_owned())));
    match kind {
        EventKind::SessionStart
        | EventKind::PromptSubmitted
        | EventKind::ToolFinished
        | EventKind::PermissionDenied => Some((Activity::Working, None)),
        EventKind::PermissionRequested { tool } => match tool.as_deref() {
            Some("AskUserQuestion") => waiting("question"),
            Some(tool) => waiting(&format!("permission for {tool}")),
            None => waiting("permission"),
        },
        EventKind::StopFailed { reason } => {
            let why = reason
                .as_deref()
                .map_or_else(|| "failed".to_owned(), |r| r.replace('_', " "));
            waiting(&why)
        }
        EventKind::Stopped { .. } => Some((Activity::Idle, None)),
        EventKind::SessionEnded { .. } => Some((Activity::Ended, None)),
        EventKind::Notification { kind } => match kind.as_str() {
            "permission_prompt" => waiting("permission"),
            "elicitation_dialog" | "elicitation_url_dialog" | "agent_needs_input" => {
                waiting("input requested")
            }
            "quota_auto_resume_stale" | "quota_auto_resume_disabled" => waiting("quota"),
            "quota_auto_resume_fired" => Some((Activity::Working, None)),
            "idle_prompt" | "agent_completed" | "elicitation_complete" | "elicitation_response" => {
                Some((Activity::Idle, None))
            }
            _ => None,
        },
    }
}
