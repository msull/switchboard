//! Hook helper called by Claude Code. Reads the hook JSON from stdin,
//! appends one line to the durable event log, pokes the app's socket as a
//! wake-up, and always exits 0. std only: it must start in well under a
//! millisecond and never break the agent.
//!
//! Usage (from the settings JSON Switchboard passes with `--settings`):
//! `switchboard-hook <EventName>`. Environment: `SWITCHBOARD_RECORD_ID`
//! (optional; injected into the pane by Switchboard) and
//! `SWITCHBOARD_DATA_DIR` (defaults to the app's Application Support dir).
//!
//! Never prints to stdout: anything a hook prints is fed back to Claude.

// Tests assert emptiness with `assert!` throughout; the rest of the
// crate is held to the lint.
#![cfg_attr(test, allow(clippy::assert_is_empty))]

use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Payload field to log field, in output order.
const COPIED: [(&str, &str); 8] = [
    ("session_id", "session_id"),
    ("cwd", "cwd"),
    ("transcript_path", "transcript_path"),
    ("notification_type", "notification_type"),
    ("tool_name", "tool_name"),
    ("reason", "reason"),
    ("error", "error"),
    ("last_assistant_message", "last_message"),
];

const LAST_MESSAGE_CHARS: usize = 200;

fn main() {
    let event = std::env::args().nth(1).unwrap_or_default();
    let mut raw = Vec::new();
    // A truncated or unreadable stdin still produces a line: the event
    // name alone is useful, and the agent must never see a failure.
    let _ = std::io::stdin().read_to_end(&mut raw);
    let payload = String::from_utf8_lossy(&raw);
    let fields = top_level_strings(&payload);

    let pending = (event == "Stop").then(|| pending_lists(&payload));
    let line = build_line(&event, &fields, pending);
    let data_dir = data_dir();
    let _ = create_private_dir(&data_dir);
    if let Err(e) = append_line(&data_dir.join("events.log"), &line) {
        eprintln!("switchboard-hook: cannot append to events.log: {e}");
    }
    wake(&data_dir.join("wake.sock"));
    std::process::exit(0);
}

fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("SWITCHBOARD_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
    home.join("Library/Application Support/Switchboard")
}

/// `pending` is `pending_lists`' result, written only for a `Stop`.
fn build_line(
    event: &str,
    fields: &[(String, String)],
    pending: Option<(Option<String>, Option<String>)>,
) -> String {
    let get = |key: &str| {
        fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    let at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    // `core::RECORD_ID_ENV`; spelled out because this binary is std-only.
    let record_id = std::env::var("SWITCHBOARD_RECORD_ID").ok();

    let mut out = String::with_capacity(512);
    let _ = write!(
        out,
        "{{\"at\":{at},\"seq\":0,\"pid\":{},\"event\":",
        std::process::id()
    );
    push_json_string(&mut out, event);
    out.push_str(",\"record_id\":");
    push_opt(&mut out, record_id.as_deref());
    for (source, target) in COPIED {
        out.push_str(",\"");
        out.push_str(target);
        out.push_str("\":");
        let value = get(source).map(|v| {
            if source == "last_assistant_message" {
                v.chars().take(LAST_MESSAGE_CHARS).collect::<String>()
            } else {
                v.to_string()
            }
        });
        push_opt(&mut out, value.as_deref());
    }
    // Only the verdict is written: the prompt is the owner's text and
    // never reaches the log.
    if event == "UserPromptSubmit" {
        let injected = get("prompt").is_some_and(is_injected);
        let _ = write!(out, ",\"injected\":{injected}");
    }
    if let Some((tasks, crons)) = pending {
        out.push_str(",\"tasks\":");
        out.push_str(tasks.as_deref().unwrap_or("null"));
        out.push_str(",\"crons\":");
        out.push_str(crons.as_deref().unwrap_or("null"));
    }
    out.push_str("}\n");
    out
}

/// The `Stop` payload's `background_tasks` and `session_crons`, re-emitted
/// as JSON arrays that keep only a task's `type`, `status` and
/// `agent_type` and a cron's `schedule` and `recurring` (see
/// `spikes/20-stop-pending`). A task's `description` and `command` and a
/// cron's `prompt` are the owner's text and never reach the log. `None`
/// for a list that is absent or not an array of objects.
fn pending_lists(json: &str) -> (Option<String>, Option<String>) {
    let Some(Json::Obj(top)) = parse_value(json, &mut 0, 0) else {
        return (None, None);
    };
    let list = |key: &str, keep: &[&str]| -> Option<String> {
        let Some((_, Json::Arr(items))) = top.iter().find(|(k, _)| k == key) else {
            return None;
        };
        let mut out = String::from("[");
        for (n, item) in items.iter().enumerate() {
            let Json::Obj(fields) = item else { return None };
            if n > 0 {
                out.push(',');
            }
            out.push('{');
            for (m, name) in keep.iter().enumerate() {
                if m > 0 {
                    out.push(',');
                }
                push_json_string(&mut out, name);
                out.push(':');
                match fields.iter().find(|(k, _)| k == name).map(|(_, v)| v) {
                    Some(Json::Str(v)) => push_json_string(&mut out, v),
                    Some(Json::Bool(v)) => {
                        let _ = write!(out, "{v}");
                    }
                    _ => out.push_str("null"),
                }
            }
            out.push('}');
        }
        out.push(']');
        Some(out)
    };
    (
        list("background_tasks", &["type", "status", "agent_type"]),
        list("session_crons", &["schedule", "recurring"]),
    )
}

/// Just enough of a JSON value for `pending_lists`: numbers and `null`
/// are kept only as `Other`.
enum Json {
    Str(String),
    Bool(bool),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
    Other,
}

fn skip_ws(bytes: &[u8], i: &mut usize) {
    while bytes.get(*i).is_some_and(u8::is_ascii_whitespace) {
        *i += 1;
    }
}

/// Parses one value at `*i` and leaves `*i` just past it; `None` on
/// malformed or truncated input, or nesting deeper than `MAX_DEPTH`, so
/// a strange payload can never overflow the helper's stack.
fn parse_value(json: &str, i: &mut usize, depth: usize) -> Option<Json> {
    const MAX_DEPTH: usize = 64;
    if depth > MAX_DEPTH {
        return None;
    }
    let bytes = json.as_bytes();
    skip_ws(bytes, i);
    match *bytes.get(*i)? {
        b'"' => {
            let (s, next) = parse_string(json, *i + 1)?;
            *i = next;
            Some(Json::Str(s))
        }
        b'[' => {
            *i += 1;
            let mut items = Vec::new();
            skip_ws(bytes, i);
            if bytes.get(*i) == Some(&b']') {
                *i += 1;
                return Some(Json::Arr(items));
            }
            loop {
                items.push(parse_value(json, i, depth + 1)?);
                skip_ws(bytes, i);
                match *bytes.get(*i)? {
                    b',' => *i += 1,
                    b']' => {
                        *i += 1;
                        return Some(Json::Arr(items));
                    }
                    _ => return None,
                }
            }
        }
        b'{' => {
            *i += 1;
            let mut fields = Vec::new();
            skip_ws(bytes, i);
            if bytes.get(*i) == Some(&b'}') {
                *i += 1;
                return Some(Json::Obj(fields));
            }
            loop {
                let Json::Str(key) = parse_value(json, i, depth + 1)? else {
                    return None;
                };
                skip_ws(bytes, i);
                if bytes.get(*i) != Some(&b':') {
                    return None;
                }
                *i += 1;
                fields.push((key, parse_value(json, i, depth + 1)?));
                skip_ws(bytes, i);
                match *bytes.get(*i)? {
                    b',' => *i += 1,
                    b'}' => {
                        *i += 1;
                        return Some(Json::Obj(fields));
                    }
                    _ => return None,
                }
            }
        }
        _ => {
            let start = *i;
            while bytes
                .get(*i)
                .is_some_and(|b| !matches!(b, b',' | b']' | b'}') && !b.is_ascii_whitespace())
            {
                *i += 1;
            }
            match &bytes[start..*i] {
                b"true" => Some(Json::Bool(true)),
                b"false" => Some(Json::Bool(false)),
                b"" => None,
                _ => Some(Json::Other),
            }
        }
    }
}

/// A turn Claude Code started on its own: a background task's
/// notification or a harness reminder, which arrive as prompts but are
/// not the owner typing (see `spikes/17-prompt-origin`).
fn is_injected(prompt: &str) -> bool {
    const TAGS: [&str; 2] = ["<task-notification>", "<system-reminder>"];
    let prompt = prompt.trim_start();
    TAGS.iter().any(|tag| prompt.starts_with(tag))
}

fn push_opt(out: &mut String, value: Option<&str>) {
    match value {
        Some(v) => push_json_string(out, v),
        None => out.push_str("null"),
    }
}

fn push_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// One `write_all` of the whole line on an `O_APPEND` descriptor, so
/// concurrent helpers never interleave within a line.
fn append_line(path: &std::path::Path, line: &str) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    // The helper may run before the app has ever created the log, so it
    // must create it as private as the store would (0600 in a 0700 dir).
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(line.as_bytes())
}

/// `create_dir_all` with owner-only permissions on the leaf directory.
fn create_private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Best effort: connect, write one byte, leave. Nobody listening is the
/// normal case when the app is closed, so every error is ignored.
fn wake(path: &std::path::Path) {
    if let Ok(mut stream) = UnixStream::connect(path) {
        let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
        let _ = stream.write_all(b"1");
    }
}

/// Collects `"key": "value"` pairs at the top level of a JSON object.
/// Nested objects and arrays are skipped, so a `"cwd"` inside
/// `tool_input` cannot shadow the real one. Not a validator: on malformed
/// input it returns whatever it found before the problem.
fn top_level_strings(json: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = json.as_bytes();
    let mut i = 0;
    let mut depth = 0usize;
    // The last significant byte outside a string: tells a key from a value.
    let mut prev = b' ';
    let mut key: Option<String> = None;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'"' => {
                let Some((s, next)) = parse_string(json, i + 1) else {
                    break;
                };
                if depth == 1 {
                    if prev == b':' {
                        if let Some(k) = key.take() {
                            out.push((k, s));
                        }
                    } else if matches!(prev, b'{' | b',') {
                        key = Some(s);
                    }
                }
                prev = b'"';
                i = next;
                continue;
            }
            b'{' | b'[' => {
                if depth == 1 {
                    key = None;
                }
                depth += 1;
                prev = b;
            }
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
                prev = b;
            }
            b' ' | b'\t' | b'\n' | b'\r' => {}
            _ => prev = b,
        }
        i += 1;
    }
    out
}

/// Parses the body of a JSON string starting just after the opening
/// quote; returns the value and the index just past the closing quote,
/// or `None` when the input ends first.
fn parse_string(json: &str, start: usize) -> Option<(String, usize)> {
    let bytes = json.as_bytes();
    let mut out = String::new();
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return Some((out, i + 1)),
            b'\\' => {
                i += 1;
                let Some(&e) = bytes.get(i) else { break };
                match e {
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'b' => out.push('\u{8}'),
                    b'f' => out.push('\u{c}'),
                    b'u' => {
                        let (c, next) = parse_unicode_escape(bytes, i + 1);
                        out.push(c);
                        i = next;
                        continue;
                    }
                    other => out.push(other as char),
                }
                i += 1;
            }
            _ => {
                // Copy one whole UTF-8 character so multi-byte text survives.
                let ch = json[i..].chars().next().unwrap_or('\u{fffd}');
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    None
}

/// `\uXXXX`, with a following `\uXXXX` combined when the first is a high
/// surrogate. Returns the character and the index just past the escape.
fn parse_unicode_escape(bytes: &[u8], start: usize) -> (char, usize) {
    let hex4 = |at: usize| -> Option<u32> {
        let s = bytes.get(at..at + 4)?;
        u32::from_str_radix(std::str::from_utf8(s).ok()?, 16).ok()
    };
    let Some(first) = hex4(start) else {
        return ('\u{fffd}', start);
    };
    let mut end = start + 4;
    let code = if (0xD800..0xDC00).contains(&first) {
        match (bytes.get(end), bytes.get(end + 1), hex4(end + 2)) {
            (Some(b'\\'), Some(b'u'), Some(low)) if (0xDC00..0xE000).contains(&low) => {
                end += 6;
                0x10000 + ((first - 0xD800) << 10) + (low - 0xDC00)
            }
            _ => 0xFFFD,
        }
    } else {
        first
    };
    (char::from_u32(code).unwrap_or('\u{fffd}'), end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_top_level_strings_only() {
        let json = r#"{"session_id":"abc","cwd":"/x/y","tool_input":{"cwd":"nested","command":"echo \"cwd\": 1"},"n":5,"list":["cwd"],"reason":"other"}"#;
        let fields = top_level_strings(json);
        assert_eq!(
            fields,
            vec![
                ("session_id".to_string(), "abc".to_string()),
                ("cwd".to_string(), "/x/y".to_string()),
                ("reason".to_string(), "other".to_string()),
            ]
        );
    }

    #[test]
    fn unescapes_strings() {
        let json = r#"{"m":"a\"b\\c\nd\u00e9\ud83d\ude00 é"}"#;
        let fields = top_level_strings(json);
        assert_eq!(fields[0].1, "a\"b\\c\ndé😀 é");
    }

    #[test]
    fn builds_a_line_with_nulls_and_escapes() {
        let fields = vec![
            ("session_id".to_string(), "s1".to_string()),
            ("last_assistant_message".to_string(), "x".repeat(300)),
            ("reason".to_string(), "say \"hi\"\n".to_string()),
            ("error".to_string(), "rate_limit".to_string()),
        ];
        let line = build_line("Stop", &fields, None);
        assert!(line.ends_with("}\n"));
        assert!(line.contains("\"event\":\"Stop\""));
        assert!(line.contains("\"cwd\":null"));
        assert!(line.contains("\"error\":\"rate_limit\""));
        assert!(line.contains("\"reason\":\"say \\\"hi\\\"\\n\""));
        assert!(line.contains(&format!("\"last_message\":\"{}\"", "x".repeat(200))));
    }

    #[test]
    fn flags_a_prompt_the_owner_did_not_type() {
        let line = |prompt: &str| {
            build_line(
                "UserPromptSubmit",
                &[("prompt".to_string(), prompt.to_string())],
                None,
            )
        };
        let typed = line("fix the secret-sauce bug");
        assert!(typed.contains("\"injected\":false"));
        assert!(!typed.contains("secret-sauce"));
        let task = line("<task-notification>\n<task-id>b1</task-id> secret-sauce");
        assert!(task.contains("\"injected\":true"));
        assert!(!task.contains("secret-sauce"));
        assert!(line("  <system-reminder>a file changed").contains("\"injected\":true"));
        assert!(
            build_line("UserPromptSubmit", &[], None).contains("\"injected\":false"),
            "a payload without a prompt reads as typed"
        );
        let other = build_line(
            "Stop",
            &[("prompt".to_string(), "<system-reminder>".into())],
            None,
        );
        assert!(!other.contains("injected"));
    }

    #[cfg(unix)]
    #[test]
    fn creates_the_log_and_its_directory_private() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = std::env::temp_dir().join(format!("switchboard-hook-{}", std::process::id()));
        let dir = tmp.join("data");
        create_private_dir(&dir).unwrap();
        let log = dir.join("events.log");
        append_line(&log, "x\n").unwrap();
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&log), 0o600);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn spike_03s_empty_lists_carry_through() {
        let json = r#"{"hook_event_name":"Stop","stop_hook_active":false,"background_tasks":[],"session_crons":[]}"#;
        let line = build_line("Stop", &[], Some(pending_lists(json)));
        assert!(line.contains(r#""tasks":[],"crons":[]}"#), "{line}");
    }

    #[test]
    fn keeps_only_kinds_from_spike_20s_stop() {
        let json = r#"{"hook_event_name":"Stop","last_assistant_message":"ok",
            "background_tasks":[
              {"id":"a06","type":"subagent","status":"running","description":"secret-sauce","agent_type":"general-purpose"},
              {"id":"b1t","type":"shell","status":"running","description":"secret-sauce","command":"sleep 90 # secret-sauce"}],
            "session_crons":[{"id":"dd0","schedule":"24 22 * * *","recurring":false,"prompt":"say secret-sauce"}]}"#;
        let (tasks, crons) = pending_lists(json);
        assert_eq!(
            tasks.as_deref(),
            Some(
                r#"[{"type":"subagent","status":"running","agent_type":"general-purpose"},{"type":"shell","status":"running","agent_type":null}]"#
            )
        );
        assert_eq!(
            crons.as_deref(),
            Some(r#"[{"schedule":"24 22 * * *","recurring":false}]"#)
        );
        let line = build_line("Stop", &top_level_strings(json), Some((tasks, crons)));
        assert!(!line.contains("secret-sauce"), "{line}");
        assert!(!line.contains("sleep 90"), "{line}");
    }

    #[test]
    fn ignores_nested_lists_and_malformed_ones() {
        let nested = r#"{"tool_input":{"background_tasks":[{"type":"shell"}]},"session_crons":[]}"#;
        assert_eq!(pending_lists(nested), (None, Some("[]".to_string())));
        let not_objects = r#"{"background_tasks":["shell"],"session_crons":{"schedule":"x"}}"#;
        assert_eq!(pending_lists(not_objects), (None, None));
        let truncated = r#"{"background_tasks":[{"type":"shell""#;
        assert_eq!(pending_lists(truncated), (None, None));
        let line = build_line("Stop", &[], Some(pending_lists(truncated)));
        assert!(line.contains(r#""tasks":null,"crons":null}"#), "{line}");
        assert!(!build_line("SessionEnd", &[], None).contains("tasks"));
        let deep = format!(
            r#"{{"x":{}1{},"session_crons":[]}}"#,
            "[".repeat(10_000),
            "]".repeat(10_000)
        );
        assert_eq!(pending_lists(&deep), (None, None));
    }

    #[test]
    fn tolerates_garbage() {
        assert!(top_level_strings("").is_empty());
        assert!(top_level_strings("{\"a\":\"unterminated").is_empty());
        assert!(top_level_strings("}}}\"\"\"").is_empty());
    }
}
