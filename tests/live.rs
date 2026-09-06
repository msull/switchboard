//! Tests that run the real agents and Ghostty. All `#[ignore]`d; each
//! costs a few cents or opens a window. Run one with
//!
//! ```sh
//! cargo test --test live -- --ignored claude_live --nocapture
//! cargo test --test live -- --ignored codex_live --nocapture
//! cargo test --test live -- --ignored ghostty_live --nocapture
//! ```
//!
//! `claude_live` runs, from a temp cwd with a temp data dir:
//!
//! ```sh
//! SWITCHBOARD_RECORD_ID=<uuid> SWITCHBOARD_DATA_DIR=<tmp> \
//!   claude -p "reply with pong" --max-turns 1 --model haiku \
//!   --session-id <uuid> --settings <tmp>/claude-hooks.json
//! ```
//!
//! and asserts the hook log carries `SessionStart`, `PromptSubmitted`,
//! `Stopped`, `SessionEnded` with the record and session ids, and that
//! the computed transcript path is the one Claude actually wrote.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime};

use switchboard::adapters::agents::Agents;
use switchboard::adapters::ghostty::MacOpener;
use switchboard::adapters::hooks::{HookLog, WakeSocket, write_hook_settings};
use switchboard::core::{AgentKind, RecordId, ResumeHandle};
use switchboard::ports::agent::AgentLauncher;
use switchboard::ports::events::{EventKind, EventSource};
use switchboard::ports::opener::Opener;

/// A nested Claude session inherits `CLAUDE*` variables that turn off
/// transcript saving in the child (spike 03), so scrub them.
fn scrub_claude_env(cmd: &mut Command) {
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("CLAUDE") {
            cmd.env_remove(k);
        }
    }
}

#[test]
#[ignore = "runs claude with --model haiku; costs a fraction of a cent"]
fn claude_live() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    let cwd = tmp.path().join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    let hook_bin = Path::new(env!("CARGO_BIN_EXE_switchboard-hook"));
    let settings = write_hook_settings(&data_dir, hook_bin).unwrap();
    let socket = WakeSocket::bind(&data_dir).unwrap();

    let agents = Agents::detect(&data_dir);
    assert!(agents.available(AgentKind::ClaudeCode), "claude not found");
    let record = RecordId::new();
    let launch = agents
        .prepare_launch(AgentKind::ClaudeCode, record, "switchboard-live", &cwd)
        .unwrap();
    let Some(ResumeHandle::ClaudeCode { session_id, .. }) = launch.resume.clone() else {
        panic!("no Claude handle");
    };
    assert_eq!(launch.argv[6], settings.to_string_lossy());

    // The real launch is interactive; `-p` with a one-turn prompt is the
    // cheap stand-in. Same argv otherwise.
    let mut cmd = Command::new(&launch.argv[0]);
    cmd.args([
        "-p",
        "reply with pong",
        "--max-turns",
        "1",
        "--model",
        "haiku",
    ])
    .args(&launch.argv[1..])
    .current_dir(&cwd)
    .envs(launch.env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    scrub_claude_env(&mut cmd);
    println!("running: {cmd:?}");
    let out = cmd.output().expect("run claude");
    println!("stdout: {}", String::from_utf8_lossy(&out.stdout));
    println!("stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "claude exited {}", out.status);

    let mut log = HookLog::new(&data_dir);
    let events = log.poll();
    for e in &events {
        println!(
            "event: {:?} record={:?} session={:?}",
            e.kind, e.record_id, e.provider_session_id
        );
    }
    let kinds: Vec<&EventKind> = events.iter().map(|e| &e.kind).collect();
    assert!(kinds.contains(&&EventKind::SessionStart));
    assert!(kinds.contains(&&EventKind::PromptSubmitted));
    assert!(kinds.iter().any(|k| matches!(k, EventKind::Stopped { .. })));
    assert!(
        kinds
            .iter()
            .any(|k| matches!(k, EventKind::SessionEnded { .. }))
    );
    for e in &events {
        assert_eq!(e.record_id, Some(record));
        assert_eq!(
            e.provider_session_id.as_deref(),
            Some(session_id.to_string().as_str())
        );
    }
    assert!(socket.take_woken(), "hook helper did not poke the socket");
    log.checkpoint();

    let handle = launch.resume.unwrap();
    let computed = agents.transcript_path(&handle).unwrap();
    let reported = events[0]
        .transcript_path
        .clone()
        .expect("transcript_path in event");
    println!("computed transcript: {}", computed.display());
    println!("reported transcript: {}", reported.display());
    assert_eq!(computed, reported);
    assert!(agents.transcript_exists(&handle));

    let resume = agents
        .prepare_resume(&handle, record, "switchboard-live", &cwd)
        .unwrap();
    println!("resume argv: {:?}", resume.argv);
}

#[test]
#[ignore = "runs codex exec once; uses the Codex account"]
fn codex_live() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    let cwd = tmp.path().join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    let agents = Agents::detect(&data_dir);
    assert!(agents.available(AgentKind::Codex), "codex not found");
    let record = RecordId::new();
    let launch = agents
        .prepare_launch(AgentKind::Codex, record, "switchboard-live", &cwd)
        .unwrap();
    assert!(launch.resume.is_none());

    let since = SystemTime::now() - Duration::from_secs(1);
    let mut cmd = Command::new(&launch.argv[0]);
    cmd.args([
        "exec",
        "--skip-git-repo-check",
        "-s",
        "read-only",
        "reply with the single word pong",
    ])
    .current_dir(&cwd)
    .envs(launch.env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    println!("running: {cmd:?}");
    let out = cmd.output().expect("run codex");
    println!("stdout: {}", String::from_utf8_lossy(&out.stdout));
    println!("stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "codex exited {}", out.status);

    let handle = agents
        .discover(AgentKind::Codex, &cwd, since)
        .unwrap()
        .expect("a rollout for this cwd");
    println!("discovered: {handle:?}");
    assert!(agents.transcript_exists(&handle));
    let resume = agents
        .prepare_resume(&handle, record, "switchboard-live", &cwd)
        .unwrap();
    println!("resume argv: {:?}", resume.argv);
    assert_eq!(resume.argv[1], "resume");
}

#[test]
#[ignore = "opens a Ghostty window for two seconds and raises it"]
fn ghostty_live() {
    let opener = MacOpener::detect();
    assert!(opener.ghostty.is_some(), "Ghostty not found");
    let title = format!("switchboard-test-{}", std::process::id());
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(opener.raise_terminal(&title), Ok(false));
    opener
        .open_terminal(&title, &["sleep".into(), "2".into()], tmp.path())
        .unwrap();
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(opener.raise_terminal(&title), Ok(true));
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        opener.raise_terminal(&title),
        Ok(false),
        "window should be gone"
    );
}
