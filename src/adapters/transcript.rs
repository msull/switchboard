//! Claude Code transcript reader. The transcript is JSONL, one record
//! per line: human prompts and tool results as `user` records, the
//! model's messages as `assistant` records, hook and notification
//! output as `system` records, plus bookkeeping types (`custom-title`,
//! `file-history-snapshot`, ...) that are skipped. Sidechain records
//! belong to subagents and are summarized by their `Agent` tool call.
//!
//! Every rule here mirrors the user's own viewer, so both show the same
//! turns. Unparsable lines are skipped: the file is appended live, so
//! the last line may be half-written.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use crate::core::ResumeHandle;
use crate::ports::transcript::{
    Activity, ActivityKind, Conversation, DETAIL_CAP, ToolDetail, TranscriptReader, Turn, Usage,
};

/// Reads Claude Code transcripts from the path in the resume handle.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClaudeTranscripts;

impl TranscriptReader for ClaudeTranscripts {
    fn read(&self, handle: &ResumeHandle) -> Result<Conversation, String> {
        match handle {
            ResumeHandle::Codex { .. } => Err("Codex transcripts are not supported yet".into()),
            ResumeHandle::ClaudeCode {
                transcript: None, ..
            } => Err("no transcript path for this session".into()),
            ResumeHandle::ClaudeCode {
                transcript: Some(path),
                ..
            } => {
                let text = std::fs::read_to_string(path)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                Ok(parse(&text))
            }
        }
    }

    fn modified(&self, handle: &ResumeHandle) -> Option<SystemTime> {
        match handle {
            ResumeHandle::ClaudeCode {
                transcript: Some(path),
                ..
            } => std::fs::metadata(path).ok()?.modified().ok(),
            ResumeHandle::ClaudeCode { .. } | ResumeHandle::Codex { .. } => None,
        }
    }

    fn clone_before(&self, handle: &ResumeHandle, before: usize) -> Result<ResumeHandle, String> {
        write_fork(handle, Some(before))
    }

    fn clone_all(&self, handle: &ResumeHandle) -> Result<ResumeHandle, String> {
        write_fork(handle, None)
    }
}

/// The copy behind both clones: up to a prompt, or (`None`) the whole
/// conversation.
fn write_fork(handle: &ResumeHandle, before: Option<usize>) -> Result<ResumeHandle, String> {
    {
        let ResumeHandle::ClaudeCode {
            session_id,
            transcript: Some(path),
        } = handle
        else {
            return Err("only Claude Code sessions with a transcript can be cloned".into());
        };
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let new_id = uuid::Uuid::new_v4();
        let (old_id, new) = (session_id.to_string(), new_id.to_string());
        let forked = match before {
            Some(before) => fork(&text, before, &old_id, &new)
                .ok_or_else(|| format!("the transcript has no prompt #{before}"))?,
            None => relabel(&text, &old_id, &new),
        };
        let dir = path
            .parent()
            .ok_or_else(|| format!("{}: no parent directory", path.display()))?;
        let dst = dir.join(format!("{new_id}.jsonl"));
        write_private(&dst, &forked).map_err(|e| format!("{}: {e}", dst.display()))?;
        Ok(ResumeHandle::ClaudeCode {
            session_id: new_id,
            transcript: Some(dst),
        })
    }
}

/// Every record, re-labelled; half-written last lines dropped as in
/// [`fork`].
fn relabel(text: &str, old_id: &str, new_id: &str) -> String {
    let mut out = String::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        if serde_json::from_str::<Value>(line).is_err() {
            continue;
        }
        out.push_str(&line.replace(old_id, new_id));
        out.push('\n');
    }
    out
}

/// The transcript's records before the `before`th typed prompt (the
/// turn numbering of [`parse`]), re-labelled with `new_id` wherever the
/// old session id appears. `None` when the prompt does not exist, so a
/// stale turn number never clones the whole conversation. The cut is a
/// prefix, so the `parentUuid` chain needs no repair.
#[must_use]
pub fn fork(text: &str, before: usize, old_id: &str, new_id: &str) -> Option<String> {
    let mut out = String::new();
    let mut seen = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        // A half-written last line is dropped, as `parse` drops it.
        let Ok(r) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if is_human_prompt(&r) {
            let content = r
                .pointer("/message/content")
                .map_or_else(String::new, text_of);
            if !systemish(&content) {
                seen += 1;
                if seen == before {
                    return Some(out);
                }
            }
        }
        out.push_str(&line.replace(old_id, new_id));
        out.push('\n');
    }
    None
}

/// The file a shell command writes with `>`/`>>` or `tee`, as agents
/// often do with a heredoc (`cat > plan.md <<'EOF'`). A `cd` earlier in
/// the command line resolves a relative target; otherwise it stays
/// relative for the caller's directory. `None` for a command that
/// redirects nowhere, or only to `/dev/null`.
#[must_use]
pub fn shell_write_target(command: &str) -> Option<PathBuf> {
    let first_line = command.lines().next().unwrap_or("");
    let tokens: Vec<&str> = first_line.split_whitespace().collect();
    let mut dir: Option<PathBuf> = None;
    let mut target: Option<&str> = None;
    let mut i = 0;
    while i < tokens.len() && target.is_none() {
        let tok = tokens[i];
        let starts_command = i == 0 || matches!(tokens[i - 1], "&&" | ";" | "||" | "|");
        if starts_command
            && tok == "cd"
            && let Some(d) = tokens.get(i + 1)
        {
            dir = Some(PathBuf::from(unquote(d)));
            i += 2;
            continue;
        }
        if tok == ">" || tok == ">>" {
            target = tokens.get(i + 1).copied();
        } else if let Some(rest) = tok.strip_prefix(">>").or_else(|| tok.strip_prefix('>'))
            && !rest.is_empty()
            && !matches!(tok.chars().next(), Some('2' | '&'))
        {
            target = Some(rest);
        } else if starts_command && tok == "tee" {
            target = tokens[i + 1..]
                .iter()
                .find(|t| !t.starts_with('-'))
                .copied();
        }
        i += 1;
    }
    let target = unquote(target?);
    if target.is_empty() || target == "/dev/null" || target.starts_with('&') {
        return None;
    }
    let path = PathBuf::from(target);
    Some(match dir {
        Some(d) if !path.is_absolute() => d.join(path),
        _ => path,
    })
}

fn unquote(s: &str) -> &str {
    s.trim_matches(|c| c == '\'' || c == '"')
}

/// Write a transcript the way the provider does: readable by the owner
/// only, and complete before it carries the final name.
fn write_private(dst: &Path, text: &str) -> std::io::Result<()> {
    let tmp = dst.with_extension("jsonl.tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, dst)
}

/// Prompts that start with one of these tags are notifications or
/// slash-command echoes, not something the user typed: they attach to
/// the current turn as activity instead of opening a new one.
const SYSTEMISH: [&str; 6] = [
    "<task-notification",
    "<system-reminder",
    "<local-command",
    "<command-name",
    "<bash-input",
    "<bash-stdout",
];

/// What a tool call came back with, indexed by `tool_use_id`.
struct ToolResult {
    error: bool,
    text: String,
}

/// Parse a whole transcript. Public so tests can feed it text directly.
#[must_use]
pub fn parse(text: &str) -> Conversation {
    let recs: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    let results = index_tool_results(&recs);

    let mut conv = Conversation::default();
    let mut usage = Usage::default();
    let mut last_usage = None;
    let mut cwd_seen = false;
    for r in &recs {
        let kind = str_field(r, "type");
        // Each `/rename` appends one; the latest is the current name.
        if kind == Some("custom-title")
            && let Some(title) = str_field(r, "customTitle")
        {
            conv.title = Some(title.to_owned());
        }
        if r.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let at = str_field(r, "timestamp").and_then(parse_time);
        if at.is_some() {
            conv.start = conv.start.or(at);
            conv.end = at;
        }
        // First value wins for the metadata fields, as in the viewer.
        // `cwd` is read for parity but the record already knows its own.
        if conv.version.is_none() {
            conv.version = str_field(r, "version").map(str::to_owned);
        }
        if conv.branch.is_none() {
            conv.branch = str_field(r, "gitBranch").map(str::to_owned);
        }
        if !cwd_seen && str_field(r, "cwd").is_some() {
            cwd_seen = true;
        }

        if is_human_prompt(r) {
            let content = r
                .pointer("/message/content")
                .map_or_else(String::new, text_of);
            if systemish(&content) {
                if let Some(cur) = conv.turns.last_mut() {
                    cur.activity
                        .push(activity(ActivityKind::System, short(&content, 120), at));
                }
                continue;
            }
            conv.turns.push(Turn {
                n: conv.turns.len() + 1,
                at,
                end: at,
                user: content,
                permission_mode: str_field(r, "permissionMode").map(str::to_owned),
                ..Turn::default()
            });
            continue;
        }

        let Some(cur) = conv.turns.last_mut() else {
            continue;
        };
        match kind {
            Some("assistant") => {
                assistant_record(
                    cur,
                    &mut conv.model,
                    &mut usage,
                    &mut last_usage,
                    r,
                    &results,
                    at,
                );
            }
            Some("system") if str_field(r, "subtype") != Some("bridge_status") => {
                if let Some(c) = str_field(r, "content").filter(|c| !c.trim().is_empty()) {
                    cur.activity
                        .push(activity(ActivityKind::System, short(c, 120), at));
                }
            }
            _ => {}
        }
    }
    conv.usage = usage;
    conv.last_usage = last_usage;

    // The final response is also the last text activity: show it once.
    // (The viewer only drops it when it is the very last line; a hook
    // or notification line after it would leave the duplicate.)
    for turn in &mut conv.turns {
        if let Some(i) = turn
            .activity
            .iter()
            .rposition(|a| a.kind == ActivityKind::Text)
        {
            turn.activity.remove(i);
        }
    }
    conv
}

/// Tool results arrive as later `user` records, but they are shown on
/// the tool call that produced them, so index them first.
fn index_tool_results(recs: &[Value]) -> HashMap<String, ToolResult> {
    let mut results: HashMap<String, ToolResult> = HashMap::new();
    for r in recs {
        if str_field(r, "type") != Some("user") {
            continue;
        }
        let Some(blocks) = r.pointer("/message/content").and_then(Value::as_array) else {
            continue;
        };
        for b in blocks {
            if str_field(b, "type") != Some("tool_result") {
                continue;
            }
            let Some(id) = str_field(b, "tool_use_id") else {
                continue;
            };
            let content = b.get("content").map_or_else(String::new, text_of);
            results.insert(
                id.to_owned(),
                ToolResult {
                    error: b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                    text: cap(&content, DETAIL_CAP),
                },
            );
        }
    }
    results
}

/// One assistant message: token usage, the model name, and its blocks
/// (thinking, text, tool calls) applied to the current turn.
fn assistant_record(
    cur: &mut Turn,
    model: &mut Option<String>,
    usage: &mut Usage,
    last_usage: &mut Option<Usage>,
    r: &Value,
    results: &HashMap<String, ToolResult>,
    at: Option<SystemTime>,
) {
    let msg = r.get("message").cloned().unwrap_or(Value::Null);
    if let Some(u) = msg.get("usage") {
        let this = Usage {
            input: num(u, "input_tokens"),
            output: num(u, "output_tokens"),
            cache_read: num(u, "cache_read_input_tokens"),
            cache_create: num(u, "cache_creation_input_tokens"),
        };
        usage.input += this.input;
        usage.output += this.output;
        usage.cache_read += this.cache_read;
        usage.cache_create += this.cache_create;
        // A multi-block turn is several records with the same usage, so
        // the last one seen is the current context.
        *last_usage = Some(this);
    }
    if model.is_none() {
        *model = str_field(&msg, "model").map(str::to_owned);
    }
    cur.assistant_msgs += 1;
    cur.end = at.or(cur.end);
    let blocks = match msg.get("content") {
        Some(Value::String(s)) => vec![serde_json::json!({"type": "text", "text": s})],
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    };
    let mut texts = Vec::new();
    // Where the message's text goes in the activity list: before any
    // tool call that followed it in the same message.
    let mut text_at = None;
    for b in &blocks {
        match str_field(b, "type") {
            Some("thinking") => cur.thinking += 1,
            Some("text") => {
                if let Some(t) = str_field(b, "text").filter(|t| !t.trim().is_empty()) {
                    text_at.get_or_insert(cur.activity.len());
                    texts.push(t.to_owned());
                }
            }
            Some("tool_use") => {
                cur.tools += 1;
                let outcome = str_field(b, "id").and_then(|id| results.get(id));
                let error = outcome.is_some_and(|o| o.error);
                if error {
                    cur.errors += 1;
                }
                let name = str_field(b, "name").unwrap_or("?");
                let input = b.get("input").map_or_else(String::new, |i| {
                    serde_json::to_string_pretty(i).unwrap_or_default()
                });
                let path = b.get("input").and_then(|i| {
                    if name == "Bash" {
                        i.get("command")
                            .and_then(Value::as_str)
                            .and_then(shell_write_target)
                    } else {
                        i.get("file_path")
                            .or_else(|| i.get("notebook_path"))
                            .and_then(Value::as_str)
                            .map(PathBuf::from)
                    }
                });
                cur.activity.push(Activity {
                    kind: ActivityKind::Tool,
                    line: tool_line(b),
                    at,
                    error,
                    detail: Some(ToolDetail {
                        name: name.to_owned(),
                        input: cap(&input, DETAIL_CAP),
                        result: outcome.map(|o| o.text.clone()).unwrap_or_default(),
                        path,
                    }),
                    text: None,
                });
            }
            _ => {}
        }
    }
    if !texts.is_empty() {
        let joined = texts.join("\n\n");
        // Last text wins: it is the turn's final response.
        let mut a = activity(ActivityKind::Text, short(&joined, 140), at);
        a.text = Some(joined.clone());
        let at_index = text_at.unwrap_or(cur.activity.len());
        cur.activity.insert(at_index, a);
        cur.final_text = joined;
    }
}

fn activity(kind: ActivityKind, line: String, at: Option<SystemTime>) -> Activity {
    Activity {
        kind,
        line,
        at,
        error: false,
        detail: None,
        text: None,
    }
}

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn num(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// A message's content, as a string or a list of blocks, flattened to
/// the text blocks joined by newlines.
fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| str_field(b, "type") == Some("text"))
            .filter_map(|b| str_field(b, "text"))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Whitespace collapsed to single spaces and cut to `n` characters
/// with an ellipsis.
fn short(s: &str, n: usize) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= n {
        collapsed
    } else {
        let mut out: String = collapsed.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// Cut to `n` characters, noting the cut.
fn cap(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_owned()
    } else {
        let mut out: String = s.chars().take(n).collect();
        out.push_str("\n… (truncated)");
        out
    }
}

/// `2026-09-06T07:28:41.672Z` and other RFC 3339 forms.
fn parse_time(s: &str) -> Option<SystemTime> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(SystemTime::from)
}

/// A `user` record the person typed: not a subagent's, not injected
/// context, not a tool result.
fn is_human_prompt(r: &Value) -> bool {
    if str_field(r, "type") != Some("user") {
        return false;
    }
    let flag = |k: &str| r.get(k).and_then(Value::as_bool) == Some(true);
    if flag("isSidechain") || flag("isMeta") || flag("isCompactSummary") {
        return false;
    }
    match r.pointer("/message/content") {
        Some(Value::String(_)) => true,
        Some(Value::Array(blocks)) => {
            let has = |t: &str| blocks.iter().any(|b| str_field(b, "type") == Some(t));
            has("text") && !has("tool_result")
        }
        _ => false,
    }
}

fn systemish(text: &str) -> bool {
    let t = text.trim_start();
    SYSTEMISH.iter().any(|tag| t.starts_with(tag))
}

/// One-line description of a `tool_use` block, per tool.
fn tool_line(block: &Value) -> String {
    let name = str_field(block, "name").unwrap_or("?");
    let Some(inp) = block.get("input").and_then(Value::as_object) else {
        return name.to_owned();
    };
    let s = |k: &str| inp.get(k).and_then(Value::as_str).unwrap_or("");
    let first_of = |keys: &[&str]| {
        keys.iter()
            .map(|k| s(k))
            .find(|v| !v.is_empty())
            .unwrap_or("")
            .to_owned()
    };
    match name {
        "Bash" => format!(
            "Bash: {}",
            short(&first_of(&["description", "command"]), 100)
        ),
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
            format!("{name}: {}", first_of(&["file_path", "notebook_path"]))
        }
        "Grep" | "Glob" => {
            let path = s("path");
            let suffix = if path.is_empty() {
                String::new()
            } else {
                format!(" in {path}")
            };
            format!("{name}: {}{suffix}", s("pattern"))
        }
        "Agent" | "Task" => {
            let sub = inp
                .get("subagent_type")
                .and_then(Value::as_str)
                .unwrap_or("general");
            format!(
                "Agent[{sub}]: {}",
                short(&first_of(&["description", "prompt"]), 90)
            )
        }
        "Skill" => format!("Skill: /{} {}", s("skill"), short(s("args"), 60)),
        "AskUserQuestion" => {
            let q = inp
                .get("questions")
                .and_then(Value::as_array)
                .and_then(|qs| qs.first())
                .and_then(|q| str_field(q, "question"))
                .unwrap_or("");
            format!("AskUserQuestion: {}", short(q, 90))
        }
        "WebFetch" | "WebSearch" => format!("{name}: {}", short(&first_of(&["url", "query"]), 90)),
        "Artifact" => {
            let action = inp
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("publish");
            format!("Artifact[{action}]: {}", first_of(&["file_path", "url"]))
        }
        "TodoWrite" => {
            let n = inp
                .get("todos")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            format!("TodoWrite: {n} items")
        }
        _ => inp
            .values()
            .find_map(|v| v.as_str().filter(|s| !s.is_empty()))
            .map_or_else(|| name.to_owned(), |v| format!("{name}: {}", short(v, 90))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::UNIX_EPOCH;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude-small.jsonl")
    }

    fn conversation() -> Conversation {
        parse(&std::fs::read_to_string(fixture()).unwrap())
    }

    #[test]
    fn the_latest_rename_is_the_title() {
        let mut text = std::fs::read_to_string(fixture()).unwrap();
        text.push_str(
            "{\"type\":\"custom-title\",\"customTitle\":\"renamed-later\",\"sessionId\":\"x\"}\n",
        );
        assert_eq!(parse(&text).title.as_deref(), Some("renamed-later"));
    }

    #[test]
    fn turns_follow_human_prompts() {
        let c = conversation();
        // Three typed prompts plus the "[Request interrupted]" text the
        // client writes as a user message, which the viewer counts too.
        assert_eq!(c.turns.len(), 4);
        assert_eq!(c.turns[0].user, "reply with the single word pong");
        assert_eq!(c.turns[0].final_text, "pong");
        assert_eq!(c.turns[0].assistant_msgs, 1);
        assert!(
            c.turns[0].activity.is_empty() || c.turns[0].activity[0].kind != ActivityKind::Text
        );
        assert_eq!(c.title.as_deref(), Some("explain-repo"));
        assert_eq!(c.branch.as_deref(), Some("main"));
        assert_eq!(c.version.as_deref(), Some("2.1.263"));
    }

    #[test]
    fn fork_keeps_the_turns_before_the_cut_under_the_new_id() {
        let text = std::fs::read_to_string(fixture()).unwrap();
        let old = &conversation_id(&text);
        let forked = fork(&text, 2, old, "new-id").unwrap();
        let c = parse(&forked);
        assert_eq!(c.turns.len(), 1);
        assert_eq!(c.turns[0].user, "reply with the single word pong");
        assert_eq!(c.turns[0].final_text, "pong");
        assert!(!forked.contains(old), "the old id is gone");
        assert!(forked.contains("\"sessionId\":\"new-id\""));
        // Before the first prompt: only the preamble. Past the end: nothing.
        assert_eq!(parse(&fork(&text, 1, old, "n").unwrap()).turns.len(), 0);
        assert!(fork(&text, 99, old, "n").is_none());
    }

    fn conversation_id(text: &str) -> String {
        text.lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .find_map(|r| str_field(&r, "sessionId").map(str::to_owned))
            .unwrap()
    }

    #[test]
    fn shell_write_targets() {
        let t = |c: &str| shell_write_target(c).map(|p| p.display().to_string());
        assert_eq!(
            t("cd /w/delta && cat > plans/a.md <<'EOF'\n# hi\nEOF"),
            Some("/w/delta/plans/a.md".into())
        );
        assert_eq!(t("cat >plans/a.md <<EOF"), Some("plans/a.md".into()));
        assert_eq!(t("echo x >> \"/abs/b.md\""), Some("/abs/b.md".into()));
        assert_eq!(t("printf x | tee -a notes.md"), Some("notes.md".into()));
        assert_eq!(t("cargo test 2>/dev/null"), None);
        assert_eq!(t("cargo test 2>&1 | head"), None);
        assert_eq!(t("ls -la"), None);
    }

    #[test]
    fn clone_before_writes_a_private_sibling_file() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.jsonl");
        std::fs::copy(fixture(), &src).unwrap();
        let text = std::fs::read_to_string(&src).unwrap();
        let handle = ResumeHandle::ClaudeCode {
            session_id: conversation_id(&text).parse().unwrap(),
            transcript: Some(src.clone()),
        };
        let cloned = ClaudeTranscripts.clone_before(&handle, 3).unwrap();
        let ResumeHandle::ClaudeCode {
            session_id,
            transcript: Some(path),
        } = &cloned
        else {
            panic!("{cloned:?}")
        };
        assert_eq!(path.parent(), src.parent());
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            format!("{session_id}.jsonl")
        );
        assert_eq!(ClaudeTranscripts.read(&cloned).unwrap().turns.len(), 2);
        assert_eq!(
            std::fs::read_to_string(&src).unwrap(),
            text,
            "source untouched"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(ClaudeTranscripts.clone_before(&handle, 99).is_err());
        let whole = ClaudeTranscripts.clone_all(&handle).unwrap();
        assert_eq!(
            ClaudeTranscripts.read(&whole).unwrap().turns.len(),
            ClaudeTranscripts.read(&handle).unwrap().turns.len()
        );
        assert_ne!(whole.provider_id(), handle.provider_id());
    }

    #[test]
    fn rejected_tool_is_an_error_activity() {
        let c = conversation();
        let t = &c.turns[1];
        assert_eq!(t.errors, 1);
        let tool = t
            .activity
            .iter()
            .find(|a| a.kind == ActivityKind::Tool)
            .unwrap();
        assert!(tool.error);
        assert_eq!(tool.line, "AskUserQuestion: Do you prefer red or blue?");
        assert!(tool.detail.as_ref().unwrap().result.contains("rejected"));
        assert!(t.final_text.is_empty(), "interrupted turn has no answer");
    }

    #[test]
    fn bash_tool_carries_input_and_result() {
        let c = conversation();
        let t = c.turns.last().unwrap();
        let tool = t
            .activity
            .iter()
            .find(|a| a.kind == ActivityKind::Tool)
            .unwrap();
        assert_eq!(tool.line, "Bash: Read crate name from Cargo.toml");
        let d = tool.detail.as_ref().unwrap();
        assert_eq!(d.name, "Bash");
        assert!(d.input.contains("Cargo.toml"));
        assert!(d.result.contains("switchboard"));
        assert!(t.final_text.contains("switchboard"));
        // The final text is not repeated as a text activity.
        assert!(t.activity.iter().all(|a| a.kind != ActivityKind::Text));
    }

    #[test]
    fn meta_and_usage_are_summed() {
        let c = conversation();
        assert!(c.usage.output > 0);
        assert!(c.usage.cache_read > 0);
        assert_eq!(c.model.as_deref(), Some("claude-fable-5-1"));
        assert!(c.start.unwrap() > UNIX_EPOCH);
        assert!(c.end.unwrap() > c.start.unwrap());
    }

    #[test]
    fn last_usage_is_the_final_assistant_message() {
        let c = conversation();
        let ctx = c.context_tokens().expect("fixture has assistant messages");
        assert!(ctx > 0);
        // One message never exceeds the sum over all of them.
        assert!(ctx <= c.usage.input + c.usage.cache_read + c.usage.cache_create);

        let text = concat!(
            r#"{"type":"user","message":{"role":"user","content":"hi"},"timestamp":"2026-01-01T00:00:00Z"}"#,
            "\n",
            r#"{"type":"assistant","message":{"role":"assistant","model":"m","content":"a","usage":{"input_tokens":10,"output_tokens":1,"cache_read_input_tokens":100,"cache_creation_input_tokens":5}},"timestamp":"2026-01-01T00:00:01Z"}"#,
            "\n",
            r#"{"type":"assistant","message":{"role":"assistant","model":"m","content":"b","usage":{"input_tokens":20,"output_tokens":2,"cache_read_input_tokens":200,"cache_creation_input_tokens":7}},"timestamp":"2026-01-01T00:00:02Z"}"#,
        );
        let c = parse(text);
        assert_eq!(c.usage.input, 30);
        assert_eq!(c.context_tokens(), Some(20 + 200 + 7));
    }

    #[test]
    fn text_before_the_answer_keeps_its_whole_message_in_order() {
        let long = "x".repeat(300);
        let text = format!(
            concat!(
                r#"{{"type":"user","message":{{"role":"user","content":"hi"}}}}"#,
                "\n",
                r#"{{"type":"assistant","message":{{"model":"m","content":[{{"type":"text","text":"{long}"}},{{"type":"tool_use","id":"t1","name":"Read","input":{{"file_path":"/a"}}}}]}}}}"#,
                "\n",
                r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t1","content":"ok"}}]}}}}"#,
                "\n",
                r#"{{"type":"assistant","message":{{"model":"m","content":[{{"type":"text","text":"done"}}]}}}}"#,
            ),
            long = long
        );
        let c = parse(&text);
        let t = &c.turns[0];
        assert_eq!(t.final_text, "done");
        // The text comes before the tool call it was written with, as an
        // excerpt on the line and the whole message beside it.
        assert_eq!(t.activity.len(), 2);
        assert_eq!(t.activity[0].kind, ActivityKind::Text);
        assert!(t.activity[0].line.len() < long.len());
        assert_eq!(t.activity[0].text.as_deref(), Some(long.as_str()));
        assert_eq!(t.activity[1].kind, ActivityKind::Tool);
        assert_eq!(
            t.activity[1].detail.as_ref().and_then(|d| d.path.clone()),
            Some(PathBuf::from("/a"))
        );
    }

    #[test]
    fn systemish_prompts_attach_to_the_current_turn() {
        let text = concat!(
            r#"{"type":"user","message":{"role":"user","content":"hi"},"timestamp":"2026-01-01T00:00:00Z"}"#,
            "\n",
            r#"{"type":"user","message":{"role":"user","content":"<system-reminder>ignore</system-reminder>"}}"#,
            "\n",
            r#"{"type":"user","isSidechain":true,"message":{"role":"user","content":"subagent"}}"#,
            "\n",
            "not json\n",
            r#"{"type":"assistant","message":{"model":"m","content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]}}"#,
        );
        let c = parse(text);
        assert_eq!(c.turns.len(), 1);
        assert_eq!(c.turns[0].activity.len(), 1);
        assert_eq!(c.turns[0].activity[0].kind, ActivityKind::System);
        assert_eq!(c.turns[0].final_text, "a\n\nb");
    }

    #[test]
    fn tool_lines_per_tool() {
        let line = |v: Value| tool_line(&v);
        assert_eq!(
            line(serde_json::json!({"name":"Read","input":{"file_path":"/a.rs"}})),
            "Read: /a.rs"
        );
        assert_eq!(
            line(serde_json::json!({"name":"Grep","input":{"pattern":"fn","path":"src"}})),
            "Grep: fn in src"
        );
        assert_eq!(
            line(serde_json::json!({"name":"TodoWrite","input":{"todos":[{},{}]}})),
            "TodoWrite: 2 items"
        );
        assert_eq!(
            line(serde_json::json!({"name":"Custom","input":{"n":1,"q":"hello"}})),
            "Custom: hello"
        );
        assert_eq!(line(serde_json::json!({"name":"Bare"})), "Bare");
    }

    #[test]
    fn codex_is_not_supported() {
        let r = ClaudeTranscripts.read(&ResumeHandle::Codex {
            rollout_id: "x".into(),
            transcript: None,
        });
        assert!(r.unwrap_err().contains("Codex"));
        let handle = ResumeHandle::ClaudeCode {
            session_id: uuid::Uuid::nil(),
            transcript: Some(fixture()),
        };
        assert!(ClaudeTranscripts.modified(&handle).is_some());
        assert_eq!(ClaudeTranscripts.read(&handle).unwrap().turns.len(), 4);
    }
}
