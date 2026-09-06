//! Session events: what hooks (and later, stream parsers) tell us about
//! a session, independent of where the signal came from.

use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::core::RecordId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    SessionStart,
    PromptSubmitted,
    ToolFinished,
    PermissionRequested { tool: Option<String> },
    PermissionDenied,
    Stopped { last_message: Option<String> },
    Notification { kind: String },
    SessionEnded { reason: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEvent {
    pub at: SystemTime,
    /// Per-helper-process sequence, for stable ordering within one instant.
    pub seq: u64,
    /// From `SWITCHBOARD_RECORD_ID` in the pane's environment, when present.
    pub record_id: Option<RecordId>,
    /// The provider's session id (Claude Code `session_id`).
    pub provider_session_id: Option<String>,
    pub cwd: Option<PathBuf>,
    pub transcript_path: Option<PathBuf>,
    pub kind: EventKind,
}

/// Append-first log reader. The socket is only a wake-up; `poll` reads
/// from the checkpointed offset and returns new events in order.
pub trait EventSource: Send {
    fn poll(&mut self) -> Vec<SessionEvent>;
    /// Persist the consumed offset; called after the core applied them.
    fn checkpoint(&mut self);
}
