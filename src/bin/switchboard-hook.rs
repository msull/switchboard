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

use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Payload field to log field, in output order.
const COPIED: [(&str, &str); 7] = [
    ("session_id", "session_id"),
    ("cwd", "cwd"),
    ("transcript_path", "transcript_path"),
    ("notification_type", "notification_type"),
    ("tool_name", "tool_name"),
    ("reason", "reason"),
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

    let line = build_line(&event, &fields);
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

fn build_line(event: &str, fields: &[(String, String)]) -> String {
    let get = |key: &str| {
        fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    let at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
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
    out.push_str("}\n");
    out
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
        ];
        let line = build_line("Stop", &fields);
        assert!(line.ends_with("}\n"));
        assert!(line.contains("\"event\":\"Stop\""));
        assert!(line.contains("\"cwd\":null"));
        assert!(line.contains("\"reason\":\"say \\\"hi\\\"\\n\""));
        assert!(line.contains(&format!("\"last_message\":\"{}\"", "x".repeat(200))));
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
    fn tolerates_garbage() {
        assert!(top_level_strings("").is_empty());
        assert!(top_level_strings("{\"a\":\"unterminated").is_empty());
        assert!(top_level_strings("}}}\"\"\"").is_empty());
    }
}
