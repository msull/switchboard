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
    /// Usage of the last assistant message: what the model currently
    /// holds in context, as opposed to the running total in `usage`.
    pub last_usage: Option<Usage>,
}

impl Conversation {
    /// Tokens in the model's context right now: the last message's input
    /// plus what it read from and wrote to the cache. `None` before the
    /// first assistant message.
    #[must_use]
    pub fn context_tokens(&self) -> Option<u64> {
        self.last_usage
            .map(|u| u.input + u.cache_read + u.cache_create)
    }
}

/// The context window for a model id, in tokens. Approximate: it is a
/// guide for the "how full is this session" figure, and Claude Code
/// compacts well before the window is reached.
#[must_use]
pub fn context_window(model: &str) -> u64 {
    const MILLION: [&str; 8] = [
        "claude-fable-",
        "claude-mythos-",
        "claude-opus-5",
        "claude-opus-4-6",
        "claude-opus-4-7",
        "claude-opus-4-8",
        "claude-sonnet-5",
        "claude-sonnet-4-6",
    ];
    if model.contains("[1m]") || MILLION.iter().any(|m| model.starts_with(m)) {
        1_000_000
    } else {
        200_000
    }
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
    /// The whole message, for text the agent wrote before its final
    /// answer; `line` is its excerpt.
    pub text: Option<String>,
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
    /// Copy the conversation up to (not including) the `before`th human
    /// prompt into a new provider session, beside the original, and
    /// return its handle. The original is never touched. This is the one
    /// place Switchboard writes into a provider's session directory.
    fn clone_before(&self, handle: &ResumeHandle, before: usize) -> Result<ResumeHandle, String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_window_table() {
        assert_eq!(context_window("claude-haiku-4-5-20251001"), 200_000);
        assert_eq!(context_window("claude-fable-5-1"), 1_000_000);
        assert_eq!(context_window("claude-sonnet-4-5[1m]"), 1_000_000);
        assert_eq!(context_window("claude-opus-4-1"), 200_000);
    }

    #[test]
    fn context_tokens_come_from_the_last_message() {
        let mut c = Conversation::default();
        assert_eq!(c.context_tokens(), None);
        c.last_usage = Some(Usage {
            input: 1,
            output: 99,
            cache_read: 2,
            cache_create: 3,
        });
        assert_eq!(c.context_tokens(), Some(6));
    }
}
