//! A confined command gate on this Mac's real sandbox: the gate writes
//! its tree and the lane's `writable` path, then outside both; that
//! write is refused, the attempt fails, and `checks.log` names the
//! refused path. The tree and the writable path sit under `$HOME`, so
//! neither write is let through by the temp directory's own grant. `#[ignore]`d: it runs
//! `sandbox-exec` and reads the unified log, so it is macOS only and
//! touches the system rather than memory. Run it with:
//!
//! ```sh
//! cargo test --locked -p dispatch --test sandbox -- --ignored --nocapture
//! ```
//!
//! No agent runs: the stage's agent is the in-memory Switchboard's,
//! stopped by hand, and the gate after it runs through the real `GitCli`
//! in a throwaway repository the project works in place.

#![cfg(target_os = "macos")]

#[allow(dead_code)]
mod support;

use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use dispatch::epoch_ms;
use dispatch::git::GitCli;
use dispatch::scheduler::Runner;
use dispatch::store::DataDir;
use dispatch::ticket::{AttemptState, SourceSnapshot};
use support::{FakeSwitchboard, SharedPort};

const PROJECT: &str = "Sandbox";

fn now_ms() -> u64 {
    epoch_ms(SystemTime::now())
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

// One run, start to finish, read in order.
#[test]
#[ignore = "runs sandbox-exec and reads the unified log"]
#[allow(clippy::too_many_lines)]
fn a_confined_gate_that_writes_outside_the_tree_fails_and_names_the_path() {
    let tmp = tempfile::tempdir().unwrap();
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap();
    let in_home = |prefix: &str| {
        tempfile::Builder::new()
            .prefix(prefix)
            .tempdir_in(&home)
            .unwrap()
    };
    let checkout = in_home(".dispatch-sandbox-tree-");
    let cache = in_home(".dispatch-sandbox-cache-");
    let root = checkout.path().to_path_buf();
    git(&root, &["init", "-q"]);
    std::fs::write(root.join("README.md"), "throwaway\n").unwrap();
    git(&root, &["add", "README.md"]);
    git(
        &root,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-qm",
            "init",
        ],
    );
    let probe = home.join(format!(".dispatch-sandbox-probe-{}", std::process::id()));

    let data = DataDir::new(tmp.path().join("dz"));
    std::fs::create_dir_all(data.root.join("pipelines")).unwrap();
    let pipeline = format!(
        r#"
version = 1

[project]
name = "Sandbox"
root = "{root}"
space = "Dispatch · Sandbox"

[source]
kind = "github"
repo = "msull/switchboard"
label = "dispatch"

[[lanes]]
name = "repo"
path = "."
writable = ["{cache}"]

[operators.worker]
kind = "claude"

[[stages]]
name = "work"
operator = "worker"
context = "root"
writes = ["notes"]
prompt = "Write {{notes}}."
gate = {{ kind = "command", argv = ["sh", "-c", "touch tree-ok \"{cache}/cache-ok\" && touch \"{probe}\""], in = "root" }}

[policy]
confine = true
"#,
        root = root.display(),
        cache = cache.path().display(),
        probe = probe.display(),
    );
    std::fs::write(data.pipeline(PROJECT), &pipeline).unwrap();

    let sb = Arc::new(Mutex::new(FakeSwitchboard::new()));
    let mut runner = Runner::new(
        data.clone(),
        Box::new(SharedPort(Arc::clone(&sb))),
        Box::new(GitCli::default()),
    );
    let id = runner
        .take(
            PROJECT,
            &pipeline,
            SourceSnapshot {
                kind: "github".into(),
                identity: "msull/switchboard#0".into(),
                number: Some(0),
                title: "sandbox check".into(),
                body: String::new(),
                url: None,
                labels: Vec::new(),
                taken_at_ms: now_ms(),
                taken_by: None,
                pull_requests: Vec::new(),
            },
            now_ms(),
        )
        .unwrap()
        .id;

    let deadline = Instant::now() + Duration::from_secs(60);
    let mut stopped = false;
    let attempt = loop {
        runner.step_project(PROJECT, now_ms()).unwrap();
        let t = runner.load_ticket(&id).unwrap();
        let a = t.attempts_of("work").last().cloned();
        if let Some(a) = &a
            && !stopped
            && let Some(session) = &a.session
        {
            std::fs::write(&a.artifacts["notes"], "done\n").unwrap();
            sb.lock().unwrap().stop(session, now_ms());
            stopped = true;
        }
        if let Some(a) = a.filter(|a| matches!(a.state, AttemptState::Failed { .. })) {
            break a;
        }
        assert!(t.active(), "{t:#?}");
        assert!(Instant::now() < deadline, "timed out: {t:#?}");
        std::thread::sleep(Duration::from_millis(500));
    };
    let created = probe.exists();
    let _ = std::fs::remove_file(&probe);
    assert!(!created, "the write outside the tree was refused");
    let log = std::fs::read_to_string(&attempt.artifacts["checks"]).unwrap();
    println!("{log}");
    assert!(log.starts_with("dispatch: confined; writable "), "{log}");
    assert!(
        log.contains(&format!("deny(1) file-write-create {}", probe.display())),
        "{log}"
    );
    assert!(root.join("tree-ok").exists(), "the tree is writable");
    assert!(
        cache.path().join("cache-ok").exists(),
        "a root gate gets the lane's writable"
    );
}
