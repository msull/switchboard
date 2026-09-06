//! Reading an agent's transcript into the shape the session view draws:
//! one turn per human prompt, the tool activity in between, and the
//! final response. The adapter (`adapters::transcript`) knows the file
//! format; the UI only sees this.

use std::time::SystemTime;

use crate::core::ResumeHandle;

/// A whole conversation, in the order it happened.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Conversation {
    /// The name the user gave the session (`/rename`), if any.
    pub title: Option<String>,
    pub model: Option<String>,
    /// Agent program version.
    pub version: Option<String>,
    pub branch: Option<String>,
    pub start: Option<SystemTime>,
    pub end: Option<SystemTime>,
    pub turns: Vec<Turn>,
    pub usage: Usage,
}

/// One human prompt and everything the agent did in response.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Turn {
    /// 1-based turn number.
    pub n: usize,
    pub at: Option<SystemTime>,
    /// Time of the last assistant message in the turn.
    pub end: Option<SystemTime>,
    pub user: String,
    pub activity: Vec<Activity>,
    /// The agent's last text in the turn: its answer.
    pub final_text: String,
    pub assistant_msgs: usize,
    pub tools: usize,
    pub thinking: usize,
    pub errors: usize,
    pub permission_mode: Option<String>,
}

/// One line in a turn's activity list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    pub kind: ActivityKind,
    pub line: String,
    pub at: Option<SystemTime>,
    /// The tool call was rejected or failed.
    pub error: bool,
    /// Full input and result, for tool calls only.
    pub detail: Option<ToolDetail>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    Tool,
    /// Text the agent wrote before its final answer.
    Text,
    /// Hook output, notifications, slash-command echoes.
    System,
}

/// What a tool was asked and what it returned; both capped so a huge
/// file read does not bloat the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDetail {
    pub name: String,
    /// Pretty-printed JSON input, at most `DETAIL_CAP` characters.
    pub input: String,
    /// Result text, at most `DETAIL_CAP` characters.
    pub result: String,
}

/// Character cap for [`ToolDetail`] input and result.
pub const DETAIL_CAP: usize = 4000;

/// Token counts summed over every assistant message.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_create: u64,
}

pub trait TranscriptReader: Send + Sync {
    /// Parse the whole transcript behind `handle`.
    fn read(&self, handle: &ResumeHandle) -> Result<Conversation, String>;
    /// The transcript file's modification time, so callers can skip
    /// re-reading an unchanged file. `None` when there is no file.
    fn modified(&self, handle: &ResumeHandle) -> Option<SystemTime>;
}
