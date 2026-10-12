//! What the app read from outside the core and keeps between frames,
//! with the version it was read at: today the parsed transcripts. The
//! views read the cache through `DrawCtx`, and a remote port reads the
//! same one, so a transcript is parsed once per change for everyone who
//! shows it. Nothing here is persisted or read by the core.

use std::collections::HashMap;
use std::time::SystemTime;

use crate::core::{RecordId, ResumeHandle};
use crate::ports::transcript::Conversation;

/// One parsed transcript and where it came from.
struct Entry {
    /// The handle the transcript was read through. A record rebound to
    /// another handle (`/clear`, a discard) needs a fresh read. `None`
    /// is a conversation with no file behind it (tests, the script),
    /// which no handle makes stale.
    handle: Option<ResumeHandle>,
    /// The file's modification time at the read; `None` when the reader
    /// has no file time, which then matches `None` on the next check.
    modified: Option<SystemTime>,
    conversation: Conversation,
}

/// Parsed transcripts per agent session, each with the handle and file
/// time it was read at, and why a session has none.
#[derive(Default)]
pub struct ConversationCache {
    entries: HashMap<RecordId, Entry>,
    errors: HashMap<RecordId, String>,
}

impl ConversationCache {
    /// The session's conversation, if one was read.
    #[must_use]
    pub fn get(&self, id: RecordId) -> Option<&Conversation> {
        self.entries.get(&id).map(|e| &e.conversation)
    }

    /// The conversation for editing in place: tests shape a turn after
    /// seeding it.
    pub fn get_mut(&mut self, id: RecordId) -> Option<&mut Conversation> {
        self.entries.get_mut(&id).map(|e| &mut e.conversation)
    }

    /// Why the session has no conversation (unsupported agent, no file).
    #[must_use]
    pub fn error(&self, id: RecordId) -> Option<&str> {
        self.errors.get(&id).map(String::as_str)
    }

    #[must_use]
    pub fn contains(&self, id: RecordId) -> bool {
        self.entries.contains_key(&id)
    }

    /// Whether what is held for `id` was read through `handle` at
    /// `modified`, so no read is needed.
    #[must_use]
    pub fn is_current(
        &self,
        id: RecordId,
        handle: &ResumeHandle,
        modified: Option<SystemTime>,
    ) -> bool {
        self.entries.get(&id).is_some_and(|e| {
            e.handle.as_ref().is_none_or(|h| h == handle) && e.modified == modified
        })
    }

    /// Keep a conversation read through `handle` at `modified`; clears
    /// any error for the session.
    pub fn insert(
        &mut self,
        id: RecordId,
        handle: ResumeHandle,
        modified: Option<SystemTime>,
        conversation: Conversation,
    ) {
        self.errors.remove(&id);
        self.entries.insert(
            id,
            Entry {
                handle: Some(handle),
                modified,
                conversation,
            },
        );
    }

    /// Keep a conversation with no file behind it, for tests and the
    /// script: it stays until removed.
    pub fn seed(&mut self, id: RecordId, conversation: Conversation) {
        self.errors.remove(&id);
        self.entries.insert(
            id,
            Entry {
                handle: None,
                modified: None,
                conversation,
            },
        );
    }

    /// Record that the session's transcript could not be read; drops
    /// whatever was held.
    pub fn fail(&mut self, id: RecordId, reason: String) {
        self.entries.remove(&id);
        self.errors.insert(id, reason);
    }

    /// Forget the session's conversation so the next refresh reads it.
    pub fn remove(&mut self, id: RecordId) {
        self.entries.remove(&id);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(s: &str) -> ResumeHandle {
        ResumeHandle::Codex {
            rollout_id: s.into(),
            transcript: None,
        }
    }

    #[test]
    fn a_read_is_current_for_its_handle_and_time() {
        let id = RecordId::new();
        let mut cache = ConversationCache::default();
        let at = Some(SystemTime::UNIX_EPOCH);
        cache.insert(id, handle("a"), at, Conversation::default());
        assert!(cache.is_current(id, &handle("a"), at));
        assert!(!cache.is_current(id, &handle("b"), at), "rebound");
        assert!(!cache.is_current(id, &handle("a"), None), "file changed");
        assert!(!cache.is_current(RecordId::new(), &handle("a"), at));
    }

    #[test]
    fn a_seeded_conversation_is_current_for_any_handle() {
        let id = RecordId::new();
        let mut cache = ConversationCache::default();
        cache.seed(id, Conversation::default());
        assert!(cache.is_current(id, &handle("a"), None));
        assert!(!cache.is_current(id, &handle("a"), Some(SystemTime::UNIX_EPOCH)));
    }

    #[test]
    fn a_failure_replaces_the_conversation_and_a_read_clears_it() {
        let id = RecordId::new();
        let mut cache = ConversationCache::default();
        cache.seed(id, Conversation::default());
        cache.fail(id, "no file".into());
        assert!(!cache.contains(id));
        assert_eq!(cache.error(id), Some("no file"));
        cache.insert(id, handle("a"), None, Conversation::default());
        assert!(cache.contains(id));
        assert_eq!(cache.error(id), None);
    }
}
