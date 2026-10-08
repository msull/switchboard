//! Hook events -> record activity. Correlation never uses cwd: the
//! record id injected into the pane wins, then the provider's session
//! id. Delivery is append-first and replayed after a crash, so an event
//! no newer than the record's last applied one is ignored rather than
//! applied out of order.

use std::time::SystemTime;

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
        // Read before this event replaces it: it says whether a turn was
        // open when the prompt came. A `SessionStart` sets `Working`
        // without opening one.
        let turn_open = record.activity == Activity::Working && !self.started.contains(&id);
        let pending = record
            .asking
            .as_ref()
            .filter(|a| a.at < event.at)
            .and_then(|a| a.answer.clone());
        let name = record.name.clone();
        if let Some(new) = Self::rebound_session(id, &event, record.resume.as_ref()) {
            self.edit_session(id, out, |s| {
                s.resume = Some(ResumeHandle::ClaudeCode {
                    session_id: new,
                    transcript: event.transcript_path.clone(),
                });
                // The cut conversation is gone from the process too.
                s.discard = None;
            });
            self.info_about(
                id,
                format!("{name} started a new conversation; the old one stays on disk"),
                now,
            );
        }
        if matches!(event.kind, EventKind::SessionEnded { .. }) {
            self.carry_dismissals(id, event.at, out);
        }
        // A hook ran, so Claude is past its own prompts.
        self.prompted.retain(|r| *r != id);
        // An event that leaves the activity alone leaves the `Working`
        // the `SessionStart` set, so the mark must stay with it.
        if change.is_some() {
            self.started.retain(|r| *r != id);
        }
        if event.kind == EventKind::SessionStart && !turn_open {
            self.started.push(id);
        }
        // Only the owner answers a question: a notification or a
        // reminder arrives as `PromptInjected`, a prompt typed while a
        // turn is open is a reminder the helper could not tell apart, and
        // a tool's `session.send` is marked when it is sent.
        let typed = event.kind == EventKind::PromptSubmitted;
        let relayed = typed && self.relayed.contains(&id);
        if relayed {
            self.relayed.retain(|r| *r != id);
        }
        let answers = typed && !relayed && !turn_open;
        // The typed reply wins over an answer left on the card; sending
        // that after it would be a second, stale reply.
        let dropped = pending.filter(|_| answers);
        let stopped = matches!(event.kind, EventKind::Stopped { .. });
        self.edit_session(id, out, |s| {
            s.last_event_at = Some(event.at);
            s.last_seen = s.last_seen.max(event.at);
            if matches!(event.kind, EventKind::Stopped { .. }) {
                s.last_stop_at = Some(event.at);
            }
            if let Some((activity, reason)) = change {
                s.activity = activity;
                s.activity_reason = reason;
            }
            // A prompt older than the ask started the turn that asked it.
            if answers && s.asking.as_ref().is_some_and(|a| a.at < event.at) {
                s.asking = None;
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
        if let Some(answer) = dropped {
            self.info_about(
                id,
                format!("{name}: your prompt replaced the answer \"{answer}\", which was not sent"),
                now,
            );
        }
        if stopped {
            self.deliver_answer(id, out);
        }
    }

    /// An end is not activity: a session killed after it was dismissed
    /// sends its end later than the stamp, which would bring it straight
    /// back. Dismissals in force just before the end move up to it.
    fn carry_dismissals(&mut self, id: RecordId, at: SystemTime, out: &mut Out) {
        let Some(before) = self.last_active(id) else {
            return;
        };
        self.update_views(out, |v| {
            for d in v.sets.iter_mut().flat_map(|s| &mut s.dismissed) {
                if d.record == id && before <= d.at {
                    d.at = d.at.max(at);
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
        | EventKind::PromptInjected
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
            // An MCP tool's dialog closing hands back to the turn it
            // interrupted; only a `Stop` ends that turn.
            "quota_auto_resume_fired" | "elicitation_complete" | "elicitation_response" => {
                Some((Activity::Working, None))
            }
            "idle_prompt" => Some((Activity::Idle, None)),
            // Anything else, `agent_completed` among them, says nothing
            // about the main turn: a background agent can finish while
            // that turn runs on.
            _ => None,
        },
    }
}
