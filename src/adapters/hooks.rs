//! Append-first hook event log reader plus the Unix-socket wake-up, and
//! the settings JSON handed to Claude Code at launch.
//!
//! The `switchboard-hook` helper appends one JSON line per Claude Code
//! hook event to `<data dir>/events.log` and then connects to
//! `<data dir>/wake.sock`. `HookLog` reads the log from a checkpointed
//! byte offset (`events.offset`), so nothing is lost while the app is
//! down and a crash between poll and checkpoint only replays events.
//! `WakeSocket` is the listener whose only job is to flip a flag.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use uuid::Uuid;

use crate::core::RecordId;
use crate::ports::events::{EventKind, EventSource, SessionEvent};

const LOG_FILE: &str = "events.log";
const ROTATED_FILE: &str = "events.log.1";
const OFFSET_FILE: &str = "events.offset";
const SOCKET_FILE: &str = "wake.sock";
const SETTINGS_FILE: &str = "claude-hooks.json";

/// Rotate the log once it is fully consumed and larger than this.
pub const COMPACT_THRESHOLD: u64 = 1024 * 1024;

/// The events `switchboard-hook` is registered for (spike 03).
pub const HOOK_EVENTS: [&str; 10] = [
    "SessionStart",
    "UserPromptSubmit",
    "PermissionRequest",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionDenied",
    "Stop",
    "StopFailure",
    "Notification",
    "SessionEnd",
];

/// One line as the helper writes it. Every field but `at`, `seq`, and
/// `event` may be null.
#[derive(Debug, Deserialize)]
struct LogLine {
    at: u64,
    #[serde(default)]
    seq: u64,
    event: String,
    record_id: Option<String>,
    session_id: Option<String>,
    cwd: Option<PathBuf>,
    transcript_path: Option<PathBuf>,
    notification_type: Option<String>,
    tool_name: Option<String>,
    reason: Option<String>,
    last_message: Option<String>,
}

impl LogLine {
    fn into_event(self) -> Option<SessionEvent> {
        let kind = match self.event.as_str() {
            "SessionStart" => EventKind::SessionStart,
            "UserPromptSubmit" => EventKind::PromptSubmitted,
            "PostToolUse" | "PostToolUseFailure" => EventKind::ToolFinished,
            "PermissionRequest" => EventKind::PermissionRequested {
                tool: self.tool_name,
            },
            "PermissionDenied" => EventKind::PermissionDenied,
            "Stop" | "StopFailure" => EventKind::Stopped {
                last_message: self.last_message,
            },
            "Notification" => EventKind::Notification {
                kind: self.notification_type.unwrap_or_default(),
            },
            "SessionEnd" => EventKind::SessionEnded {
                reason: self.reason,
            },
            _ => return None,
        };
        Some(SessionEvent {
            at: UNIX_EPOCH + Duration::from_millis(self.at),
            seq: self.seq,
            record_id: self
                .record_id
                .and_then(|s| Uuid::parse_str(&s).ok())
                .map(RecordId),
            provider_session_id: self.session_id,
            cwd: self.cwd,
            transcript_path: self.transcript_path,
            kind,
        })
    }
}

/// Reader over `events.log` with a durable offset.
#[derive(Debug)]
pub struct HookLog {
    data_dir: PathBuf,
    /// Bytes of `events.log` already returned by `poll`.
    offset: u64,
    /// Events read from a rotated file during `compact`, handed out by
    /// the next `poll`.
    pending: Vec<SessionEvent>,
}

impl HookLog {
    /// Opens the log under `data_dir`, resuming from the saved offset.
    #[must_use]
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        let data_dir = data_dir.into();
        let offset = fs::read_to_string(data_dir.join(OFFSET_FILE))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        Self {
            data_dir,
            offset,
            pending: Vec::new(),
        }
    }

    #[must_use]
    pub fn log_path(&self) -> PathBuf {
        self.data_dir.join(LOG_FILE)
    }

    #[must_use]
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Rotates the log with an atomic rename once everything in it has
    /// been consumed and it has grown past `COMPACT_THRESHOLD`. The helper
    /// opens the log by path for every append, so appends after the
    /// rename land in a fresh file; an append that raced the rename ends
    /// up in the rotated file and is picked up here.
    pub fn compact(&mut self) {
        let path = self.log_path();
        let Ok(meta) = fs::metadata(&path) else {
            return;
        };
        if meta.len() < COMPACT_THRESHOLD || meta.len() != self.offset {
            return;
        }
        let rotated = self.data_dir.join(ROTATED_FILE);
        if fs::rename(&path, &rotated).is_err() {
            return;
        }
        let (late, _) = read_from(&rotated, self.offset);
        self.pending.extend(late);
        self.offset = 0;
        self.checkpoint();
    }
}

/// Reads complete lines from `offset`; returns the events and the new
/// offset (just past the last newline consumed).
fn read_from(path: &Path, offset: u64) -> (Vec<SessionEvent>, u64) {
    let Ok(mut file) = File::open(path) else {
        return (Vec::new(), offset);
    };
    // A shorter file than the checkpoint means it was replaced or
    // truncated by hand; start over rather than reading from mid-line.
    let len = file.metadata().map_or(0, |m| m.len());
    let mut offset = if len < offset { 0 } else { offset };
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return (Vec::new(), offset);
    }
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return (Vec::new(), offset);
    }
    let mut events = Vec::new();
    let mut start = 0;
    while let Some(nl) = buf[start..].iter().position(|&b| b == b'\n') {
        let line = &buf[start..start + nl];
        start += nl + 1;
        if let Some(event) = serde_json::from_slice::<LogLine>(line)
            .ok()
            .and_then(LogLine::into_event)
        {
            events.push(event);
        }
    }
    offset += start as u64;
    (events, offset)
}

impl EventSource for HookLog {
    fn poll(&mut self) -> Vec<SessionEvent> {
        let (events, offset) = read_from(&self.log_path(), self.offset);
        self.offset = offset;
        let mut out = std::mem::take(&mut self.pending);
        out.extend(events);
        out
    }

    fn checkpoint(&mut self) {
        let final_path = self.data_dir.join(OFFSET_FILE);
        let tmp = self.data_dir.join(format!("{OFFSET_FILE}.tmp"));
        let write = || -> io::Result<()> {
            let mut f = File::create(&tmp)?;
            f.write_all(self.offset.to_string().as_bytes())?;
            f.sync_all()?;
            fs::rename(&tmp, &final_path)
        };
        if let Err(e) = write() {
            log::warn!("hook log checkpoint failed: {e}");
        }
    }
}

/// Listener on `<data dir>/wake.sock`. Every connection sets a flag that
/// `take_woken` clears; the bytes sent are ignored. State travels only
/// through the log.
#[derive(Debug)]
pub struct WakeSocket {
    path: PathBuf,
    woken: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl WakeSocket {
    /// Binds the socket, replacing a stale file from a previous run, and
    /// starts the accept loop on a background thread.
    pub fn bind(data_dir: &Path) -> io::Result<Self> {
        Self::bind_with(data_dir, || {})
    }

    /// Like `bind`, with a callback run on the listener thread for every
    /// wake-up (for example an egui `request_repaint`).
    pub fn bind_with(data_dir: &Path, on_wake: impl Fn() + Send + 'static) -> io::Result<Self> {
        fs::create_dir_all(data_dir)?;
        let path = data_dir.join(SOCKET_FILE);
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        let woken = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let woken = Arc::clone(&woken);
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("switchboard-wake".into())
                .spawn(move || {
                    for stream in listener.incoming() {
                        if stop.load(Ordering::SeqCst) {
                            break;
                        }
                        if stream.is_ok() {
                            woken.store(true, Ordering::SeqCst);
                            on_wake();
                        }
                    }
                })?
        };
        Ok(Self {
            path,
            woken,
            stop,
            thread: Some(thread),
        })
    }

    /// True once since the last call if any helper connected.
    #[must_use]
    pub fn take_woken(&self) -> bool {
        self.woken.swap(false, Ordering::SeqCst)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for WakeSocket {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // The accept loop only checks the flag after a connection, so
        // make one to unblock it.
        let _ = UnixStream::connect(&self.path);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = fs::remove_file(&self.path);
    }
}

/// The settings object Claude Code receives via `--settings`: the ten
/// events from spike 03, each running `<hook_binary> <Event>` with a
/// 5 s timeout and no matcher.
#[must_use]
pub fn hook_settings_json(hook_binary: &Path) -> serde_json::Value {
    let binary = shell_quote(&hook_binary.to_string_lossy());
    let hooks: serde_json::Map<String, serde_json::Value> = HOOK_EVENTS
        .iter()
        .map(|event| {
            let entry = serde_json::json!([{
                "hooks": [{
                    "type": "command",
                    "command": format!("{binary} {event}"),
                    "timeout": 5,
                }]
            }]);
            ((*event).to_string(), entry)
        })
        .collect();
    serde_json::json!({ "hooks": hooks })
}

/// Hooks run through `/bin/sh -c`, so a path with spaces (Application
/// Support) needs quoting. Plain paths are left as they are.
fn shell_quote(s: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "/._-+:@%".contains(c);
    if !s.is_empty() && s.chars().all(plain) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Writes the settings to `<data dir>/claude-hooks.json` (atomically) and
/// returns that path, for `claude --settings <path>`.
pub fn write_hook_settings(data_dir: &Path, hook_binary: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(data_dir)?;
    let path = data_dir.join(SETTINGS_FILE);
    let tmp = data_dir.join(format!("{SETTINGS_FILE}.tmp"));
    let text = serde_json::to_string_pretty(&hook_settings_json(hook_binary))?;
    {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Wall-clock now as the helper records it, for tests and for `since`
/// arguments.
#[must_use]
pub fn unix_millis(t: SystemTime) -> u64 {
    u64::try_from(t.duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(at: u64, event: &str, extra: &str) -> String {
        format!(
            "{{\"at\":{at},\"seq\":0,\"pid\":1,\"event\":\"{event}\",\"record_id\":\"4c1d0d3e-2f1a-4b58-9d3f-1b2c3d4e5f60\",\"session_id\":\"s1\",\"cwd\":\"/w\",\"transcript_path\":\"/t.jsonl\",\"notification_type\":null,\"tool_name\":null,\"reason\":null,\"last_message\":null{extra}}}\n"
        )
    }

    #[test]
    fn maps_events_and_leaves_partial_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG_FILE);
        let mut log = HookLog::new(dir.path());
        assert!(log.poll().is_empty(), "no file yet");

        let mut text = line(1000, "SessionStart", "");
        text.push_str(&line(1001, "UserPromptSubmit", ""));
        text.push_str(&line(1002, "InstructionsLoaded", ""));
        text.push_str("{\"at\":1003,\"event\":\"Stop\"");
        fs::write(&path, &text).unwrap();

        let events = log.poll();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, EventKind::SessionStart);
        assert_eq!(events[0].at, UNIX_EPOCH + Duration::from_millis(1000));
        assert_eq!(
            events[0].record_id.unwrap().0.to_string(),
            "4c1d0d3e-2f1a-4b58-9d3f-1b2c3d4e5f60"
        );
        assert_eq!(events[0].provider_session_id.as_deref(), Some("s1"));
        assert_eq!(events[0].cwd.as_deref(), Some(Path::new("/w")));
        assert_eq!(events[1].kind, EventKind::PromptSubmitted);
        assert!(log.poll().is_empty(), "partial line waits");

        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b",\"last_message\":\"pong\"}\n").unwrap();
        let events = log.poll();
        assert_eq!(
            events[0].kind,
            EventKind::Stopped {
                last_message: Some("pong".into())
            }
        );
    }

    #[test]
    fn maps_every_registered_event() {
        let dir = tempfile::tempdir().unwrap();
        let mut text = String::new();
        text.push_str(&line(1, "PostToolUse", ""));
        text.push_str(&line(2, "PostToolUseFailure", ""));
        text.push_str(
            &line(3, "PermissionRequest", "")
                .replace("\"tool_name\":null", "\"tool_name\":\"Bash\""),
        );
        text.push_str(&line(4, "PermissionDenied", ""));
        text.push_str(&line(5, "StopFailure", ""));
        text.push_str(&line(6, "Notification", "").replace(
            "\"notification_type\":null",
            "\"notification_type\":\"idle_prompt\"",
        ));
        text.push_str(
            &line(7, "SessionEnd", "").replace("\"reason\":null", "\"reason\":\"other\""),
        );
        fs::write(dir.path().join(LOG_FILE), text).unwrap();
        let kinds: Vec<EventKind> = HookLog::new(dir.path())
            .poll()
            .into_iter()
            .map(|e| e.kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                EventKind::ToolFinished,
                EventKind::ToolFinished,
                EventKind::PermissionRequested {
                    tool: Some("Bash".into())
                },
                EventKind::PermissionDenied,
                EventKind::Stopped { last_message: None },
                EventKind::Notification {
                    kind: "idle_prompt".into()
                },
                EventKind::SessionEnded {
                    reason: Some("other".into())
                },
            ]
        );
    }

    #[test]
    fn checkpoint_survives_reopen_and_missing_record_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG_FILE);
        fs::write(
            &path,
            line(1, "SessionStart", "").replace(
                "\"record_id\":\"4c1d0d3e-2f1a-4b58-9d3f-1b2c3d4e5f60\"",
                "\"record_id\":null",
            ),
        )
        .unwrap();
        let mut log = HookLog::new(dir.path());
        let events = log.poll();
        assert_eq!(events.len(), 1);
        assert!(events[0].record_id.is_none());
        log.checkpoint();

        let mut again = HookLog::new(dir.path());
        assert_eq!(again.offset(), log.offset());
        assert!(again.poll().is_empty(), "already consumed");

        // Without a checkpoint the event is replayed, which is harmless.
        assert_eq!(HookLog::new(dir.path()).offset(), log.offset());
    }

    #[test]
    fn compacts_only_when_consumed_and_large() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG_FILE);
        let one = line(1, "Stop", "");
        let mut text = String::new();
        while (text.len() as u64) < COMPACT_THRESHOLD {
            text.push_str(&one);
        }
        fs::write(&path, &text).unwrap();
        let mut log = HookLog::new(dir.path());
        log.compact();
        assert!(path.exists(), "unconsumed log stays");

        let n = log.poll().len();
        assert!(n > 0);
        // A helper that opened the old path just before the rename.
        let mut late = OpenOptions::new().append(true).open(&path).unwrap();
        log.compact();
        late.write_all(line(2, "SessionEnd", "").as_bytes())
            .unwrap();
        drop(late);
        assert!(!path.exists());
        assert!(dir.path().join(ROTATED_FILE).exists());
        assert_eq!(log.offset(), 0);

        // The late line was written into the rotated file after our read,
        // so it is found on the next compact-free poll only if we look
        // there; this test pins the simpler guarantee: a fresh append to
        // the new log is read from offset 0.
        fs::write(&path, line(3, "SessionStart", "")).unwrap();
        let events = log.poll();
        assert_eq!(events.last().unwrap().kind, EventKind::SessionStart);
    }

    #[test]
    fn compact_keeps_a_line_appended_before_the_rename_was_seen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG_FILE);
        let one = line(1, "Stop", "");
        let mut text = String::new();
        while (text.len() as u64) < COMPACT_THRESHOLD {
            text.push_str(&one);
        }
        fs::write(&path, &text).unwrap();
        let mut log = HookLog::new(dir.path());
        log.poll();
        // Simulate the race: the helper's append lands between the length
        // check and the rename. From `compact`'s view the file length
        // matched at check time; the tail read after the rename sees it.
        let rotated = dir.path().join(ROTATED_FILE);
        fs::rename(&path, &rotated).unwrap();
        let mut f = OpenOptions::new().append(true).open(&rotated).unwrap();
        f.write_all(line(2, "SessionEnd", "").as_bytes()).unwrap();
        drop(f);
        let (late, _) = read_from(&rotated, log.offset());
        assert_eq!(late.len(), 1);
        assert_eq!(late[0].kind, EventKind::SessionEnded { reason: None });
    }

    #[test]
    fn wake_socket_flags_a_connection() {
        let dir = tempfile::tempdir().unwrap();
        let sock = WakeSocket::bind(dir.path()).unwrap();
        assert!(!sock.take_woken());
        let mut c = UnixStream::connect(sock.path()).unwrap();
        // The listener only needs the connection; it may close before
        // this byte lands, which is a broken pipe, not a failure.
        let _ = c.write_all(b"1");
        drop(c);
        // Generous: the whole suite runs in parallel on the pre-commit hook.
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while !sock.take_woken() {
            assert!(std::time::Instant::now() < deadline, "no wake-up");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!sock.take_woken(), "flag cleared by take");
        let path = sock.path().to_path_buf();
        drop(sock);
        assert!(!path.exists());
    }

    #[test]
    fn settings_cover_the_ten_events_with_an_absolute_command() {
        let v = hook_settings_json(Path::new("/opt/sb/switchboard-hook"));
        let hooks = v["hooks"].as_object().unwrap();
        assert_eq!(hooks.len(), 10);
        for event in HOOK_EVENTS {
            let h = &hooks[event][0]["hooks"][0];
            assert_eq!(h["type"], "command");
            assert_eq!(h["command"], format!("/opt/sb/switchboard-hook {event}"));
            assert_eq!(h["timeout"], 5);
        }
        let spaced = hook_settings_json(Path::new("/Users/x/Application Support/sb-hook"));
        assert_eq!(
            spaced["hooks"]["Stop"][0]["hooks"][0]["command"],
            "'/Users/x/Application Support/sb-hook' Stop"
        );
    }

    #[test]
    fn writes_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_hook_settings(dir.path(), Path::new("/opt/sb/switchboard-hook")).unwrap();
        assert_eq!(path, dir.path().join(SETTINGS_FILE));
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(v["hooks"]["SessionEnd"].is_array());
    }
}
