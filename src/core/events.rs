//! Hook events -> record activity. Correlation never uses cwd: the
//! record id injected into the pane wins, then the provider's session
//! id. Delivery is append-first and replayed after a crash, so an event
//! no newer than the record's last applied one is ignored rather than
//! applied out of order.

use crate::core::action::{AppCore, Out};
use crate::core::model::{Activity, RecordId, ResumeHandle};
use crate::ports::events::{EventKind, SessionEvent};

impl AppCore {
    pub(super) fn apply_events(&mut self, events: Vec<SessionEvent>, out: &mut Out) {
        for event in events {
            if let Some(id) = self.match_event(&event) {
                self.apply_event(id, event, out);
            }
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

    fn apply_event(&mut self, id: RecordId, event: SessionEvent, out: &mut Out) {
        let Some(record) = self.session(id) else {
            return;
        };
        if record.last_event_at.is_some_and(|last| event.at <= last) {
            return;
        }
        let activity = activity_for(&event.kind);
        self.edit_session(id, out, |s| {
            s.last_event_at = Some(event.at);
            s.last_seen = s.last_seen.max(event.at);
            if let Some(a) = activity {
                s.activity = a;
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

/// What an event says the session is doing; `None` for notifications
/// the card does not care about.
fn activity_for(kind: &EventKind) -> Option<Activity> {
    Some(match kind {
        EventKind::SessionStart
        | EventKind::PromptSubmitted
        | EventKind::ToolFinished
        | EventKind::PermissionDenied => Activity::Working,
        EventKind::PermissionRequested { .. } => Activity::WaitingOnYou,
        EventKind::Stopped { .. } => Activity::Idle,
        EventKind::SessionEnded { .. } => Activity::Ended,
        EventKind::Notification { kind } if kind.contains("permission") => Activity::WaitingOnYou,
        EventKind::Notification { kind } if kind.contains("idle") => Activity::Idle,
        EventKind::Notification { .. } => return None,
    })
}
