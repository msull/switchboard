//! What the app read from outside the core and keeps between frames,
//! with the version it was read at: the parsed transcripts, and the
//! content the remote port serves through one worker thread (a pane's
//! screen today; conversations, run output and files follow). The
//! views read the transcript cache through `DrawCtx`, and the remote
//! port reads the same one, so a transcript is parsed once per change
//! for everyone who shows it. Nothing here is persisted or read by the
//! core.
//!
//! The worker does the read, the mask, the version and the
//! serialization off the frame, one job at a time, and reports what it
//! observed; the frame keeps the versions, assigns revisions and emits
//! `changed`. The same worker polls every watched key every
//! [`WATCH_POLL`], so a watched pane moves for a device while the host
//! shows something else or is hidden.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use crate::adapters::remote::RemoteReply;
use crate::core::{RecordId, ResumeHandle};
use crate::ports::host::{HostId, ProcessHost};
use crate::ports::transcript::Conversation;

/// How often the worker reads every watched key.
pub const WATCH_POLL: Duration = Duration::from_secs(2);
/// A key with no subscriber and no read for this long leaves the set.
pub const WATCH_IDLE: Duration = Duration::from_secs(60);
/// The most keys the set holds; past it the least recently read goes.
pub const WATCH_CAP: usize = 128;
/// A screen's text after masking is cut from the top to this.
pub const SCREEN_BYTES_CAP: usize = 256 * 1024;
/// How many lines above the asked-for ones a screen read takes and
/// masks with them, so a value wrapped across the window's first line
/// is still found whole.
pub const SCREEN_SLACK_LINES: usize = 8;

/// The most lines a screen read returns.
pub const SCREEN_LINES_CAP: usize = 200;
/// How many jobs may wait for the worker.
const JOBS: usize = 64;

/// One piece of content a device can read and watch.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ContentKey {
    Screen(RecordId),
}

impl ContentKey {
    /// The `kind` on the wire.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Screen(_) => "screen",
        }
    }

    /// The `id` on the wire.
    #[must_use]
    pub fn id(&self) -> String {
        match self {
            Self::Screen(id) => id.0.to_string(),
        }
    }

    /// The key a `watch` frame names, if this host serves the kind.
    #[must_use]
    pub fn parse(kind: &str, id: &str) -> Option<Self> {
        match kind {
            "screen" => uuid::Uuid::parse_str(id)
                .ok()
                .map(|u| Self::Screen(RecordId(u))),
            _ => None,
        }
    }

    #[must_use]
    pub fn record(&self) -> RecordId {
        match self {
            Self::Screen(id) => *id,
        }
    }
}

/// The secret values a text leaving the port is masked of, and the
/// fingerprint that names this policy in every version. The values are
/// in memory only: never logged, never stored.
#[derive(Debug, Clone, Default)]
pub struct Policy {
    pub secrets: Vec<(String, String)>,
    pub fingerprint: u64,
}

impl Policy {
    #[must_use]
    pub fn new(mut secrets: Vec<(String, String)>) -> Self {
        secrets.sort();
        secrets.dedup();
        let mut h = DefaultHasher::new();
        secrets.hash(&mut h);
        Self {
            fingerprint: h.finish(),
            secrets,
        }
    }
}

/// One read for the worker: what to read, how to mask it, and where
/// the reply goes (nowhere for a poll).
#[derive(Clone)]
pub struct ContentJob {
    pub key: ContentKey,
    pub pane: HostId,
    pub lines: usize,
    pub policy: Policy,
    pub reply: Option<SyncSender<RemoteReply>>,
}

/// What the worker saw for a job: the source version and the masked
/// body, or why it could not read.
pub struct Observed {
    pub key: ContentKey,
    pub policy: u64,
    pub result: Result<(u64, serde_json::Value), String>,
    pub reply: Option<SyncSender<RemoteReply>>,
}

/// The version the frame holds per key: what it last served and the
/// revision it assigned when that changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentVersion {
    pub source: u64,
    pub policy: u64,
    pub revision: u64,
}

/// A watched key: who asked, and when it was last read if a read put
/// it here. A read keeps it for [`WATCH_IDLE`]; a subscriber keeps it
/// while it holds the watch.
#[derive(Debug, Clone)]
pub struct Watch {
    pub last_read: Option<Instant>,
    pub subscribers: std::collections::HashSet<u64>,
}

/// What goes down the worker's channel: a read, or a word that the
/// poll interval moved.
enum Work {
    Job(ContentJob),
    Nudge,
}

/// The one thread that reads content for the port.
pub struct ContentWorker {
    jobs: SyncSender<Work>,
    watch: Arc<Mutex<Vec<ContentJob>>>,
    poll: Arc<Mutex<Duration>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ContentWorker {
    /// Start the worker over `host`; every observation goes to
    /// `observed` and `wake` runs after each so the frame picks it up.
    #[must_use]
    pub fn spawn(
        host: Arc<dyn ProcessHost>,
        observed: Sender<Observed>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let (jobs, inbox) = mpsc::sync_channel(JOBS);
        let watch = Arc::new(Mutex::new(Vec::new()));
        let poll = Arc::new(Mutex::new(WATCH_POLL));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (watch, poll, stop) = (Arc::clone(&watch), Arc::clone(&poll), Arc::clone(&stop));
            std::thread::Builder::new()
                .name("switchboard-content".into())
                .spawn(move || {
                    let every = || *poll.lock().expect("poll");
                    let mut next_poll = Instant::now() + every();
                    while !stop.load(Ordering::SeqCst) {
                        let wait = next_poll.saturating_duration_since(Instant::now());
                        match inbox.recv_timeout(wait) {
                            Ok(Work::Job(job)) => {
                                let out = observe(&*host, &job);
                                if observed.send(out).is_err() {
                                    return;
                                }
                                wake();
                            }
                            Ok(Work::Nudge) => next_poll = Instant::now() + every(),
                            Err(mpsc::RecvTimeoutError::Disconnected) => return,
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                let jobs = watch.lock().map(|w| w.clone()).unwrap_or_default();
                                for job in jobs {
                                    if observed.send(observe(&*host, &job)).is_err() {
                                        return;
                                    }
                                }
                                wake();
                                next_poll = Instant::now() + every();
                            }
                        }
                    }
                })
                .ok()
        };
        Self {
            jobs,
            watch,
            poll,
            stop,
            thread,
        }
    }

    /// Queue one read; back when the worker is too far behind.
    pub fn submit(&self, job: ContentJob) -> Result<(), ContentJob> {
        self.jobs.try_send(Work::Job(job)).map_err(|e| match e {
            mpsc::TrySendError::Full(Work::Job(j))
            | mpsc::TrySendError::Disconnected(Work::Job(j)) => j,
            mpsc::TrySendError::Full(Work::Nudge)
            | mpsc::TrySendError::Disconnected(Work::Nudge) => {
                unreachable!("a job was sent")
            }
        })
    }

    /// Replace what the poll reads.
    pub fn set_watch(&self, jobs: Vec<ContentJob>) {
        if let Ok(mut w) = self.watch.lock() {
            *w = jobs;
        }
    }

    /// How often the poll runs, from now; tests shorten it.
    pub fn set_poll(&self, every: Duration) {
        if let Ok(mut p) = self.poll.lock() {
            *p = every;
        }
        let _ = self.jobs.try_send(Work::Nudge);
    }
}

impl Drop for ContentWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // An empty job wakes the loop so it sees the flag.
        if let Some(t) = self.thread.take() {
            drop(std::mem::replace(&mut self.jobs, mpsc::sync_channel(1).0));
            let _ = t.join();
        }
    }
}

/// One job on the worker: a pane's whole window with slack, versioned
/// by that text, masked, the asked-for lines kept, cut to the byte
/// cap. The read is always the full window so a short read and the
/// poll see one version.
fn observe(host: &dyn ProcessHost, job: &ContentJob) -> Observed {
    let result = match &job.key {
        ContentKey::Screen(_) => host
            .snapshot(&job.pane, Some(SCREEN_LINES_CAP + SCREEN_SLACK_LINES))
            .map_err(|e| format!("snapshot: {e}"))
            .map(|text| {
                let mut h = DefaultHasher::new();
                text.hash(&mut h);
                let source = h.finish();
                let redacted = redact(&text, &job.policy.secrets);
                let kept: Vec<&str> = redacted.trim_end().lines().collect();
                let mut text = kept[kept.len().saturating_sub(job.lines)..].join("\n");
                let mut truncated = false;
                if text.len() > SCREEN_BYTES_CAP {
                    let mut cut = text.len() - SCREEN_BYTES_CAP;
                    while !text.is_char_boundary(cut) {
                        cut += 1;
                    }
                    text = text[cut..].to_owned();
                    truncated = true;
                }
                (
                    source,
                    serde_json::json!({ "text": text, "truncated": truncated }),
                )
            }),
    };
    Observed {
        key: job.key.clone(),
        policy: job.policy.fingerprint,
        result,
        reply: job.reply.clone(),
    }
}

/// `text` with every occurrence of each secret's value replaced by
/// `<NAME>`. Longer values go first, so a value that holds another is
/// replaced whole rather than leaving a fragment around the shorter one.
/// A pane's capture breaks a line longer than the pane where it wraps,
/// so a value is also found with line breaks inside it.
/// The capture also trims each row's trailing spaces, so a space in the
/// value that fell at a wrap may be gone.
#[must_use]
pub fn redact(text: &str, secrets: &[(String, String)]) -> String {
    let mut by_length: Vec<&(String, String)> =
        secrets.iter().filter(|(_, v)| !v.is_empty()).collect();
    by_length.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
    let mut out = text.to_owned();
    for (name, value) in by_length {
        let placeholder = format!("<{name}>");
        let mut from = 0;
        while let Some((start, end)) = find_across_breaks(&out, value, from) {
            out.replace_range(start..end, &placeholder);
            from = start + placeholder.len();
        }
    }
    out
}

/// The byte range of the first occurrence of `value` in `text` at or
/// after `from`, where `text` may break a line anywhere inside it and
/// some of a run of spaces in `value` at such a break may be missing:
/// the row before the break lost its trailing ones, and the row after
/// holds the rest. Byte matching stays on character boundaries: a
/// value's first byte is never a UTF-8 continuation byte, and a line
/// break never sits inside one.
fn find_across_breaks(text: &str, value: &str, from: usize) -> Option<(usize, usize)> {
    let hay = text.as_bytes();
    let needle = value.as_bytes();
    let is_break = |b: u8| b == b'\n' || b == b'\r';
    let spaces = |bytes: &[u8], at: usize| {
        bytes
            .get(at..)
            .map_or(0, |rest| rest.iter().take_while(|&&b| b == b' ').count())
    };
    (from..hay.len()).find_map(|start| {
        let mut i = start;
        let mut k = 0;
        while let Some(&want) = needle.get(k) {
            let at_break = hay.get(i).copied().is_some_and(is_break);
            if k > 0 && !is_break(want) && at_break {
                while hay.get(i).copied().is_some_and(is_break) {
                    i += 1;
                }
                if want == b' ' {
                    let kept = spaces(hay, i);
                    let run = spaces(needle, k);
                    if kept > run {
                        return None;
                    }
                    i += kept;
                    k += run;
                    continue;
                }
            }
            if hay.get(i) != Some(&want) {
                return None;
            }
            i += 1;
            k += 1;
        }
        Some((start, i))
    })
}

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
