//! The first slice against a real Switchboard and a real Claude Code.
//! `#[ignore]`d: it opens the Switchboard window and runs one haiku turn.
//! Build both binaries first, then:
//!
//! ```sh
//! cargo build --locked --workspace
//! cargo test --locked -p dispatch --test live -- --ignored --nocapture
//! ```
//!
//! It starts the debug `switchboard` beside the `dispatch` binary on a
//! temp data directory and a `switchboard-test-<pid>-dispatch` tmux
//! socket, takes a one-stage pipeline whose root is a throwaway git
//! repository under `$HOME/code_repos`, and steps the scheduler until
//! the investigator has written its notes and been killed. The root is
//! a plain directory under `$HOME/code_repos`, which is trusted by
//! Claude Code and covers plain subdirectories but not a fresh git
//! repository (a repository is its own workspace and gets the trust
//! dialog), so the stage here cuts no lane. The notes go into Dispatch's
//! directory under an allow rule, so no permission prompt. The window is
//! closed by killing the child by pid; the tmux server is killed by
//! socket name.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use dispatch::epoch_ms;
use dispatch::git::GitCli;
use dispatch::port::SocketPort;
use dispatch::scheduler::Runner;
use dispatch::store::DataDir;
use dispatch::ticket::{AttemptState, SourceSnapshot, TicketState};
use switchboard_control::{Body, Client, Reply, Request, SOCKET_FILE};

const PROJECT: &str = "Live";

/// A Switchboard on its own data directory and tmux socket, gone with
/// the guard.
struct App {
    child: Child,
    socket: String,
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = Command::new("tmux")
            .args(["-L", &self.socket, "kill-server"])
            .status();
    }
}

/// A repository under `$HOME/code_repos`, removed with the guard.
struct Repo(PathBuf);

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn now_ms() -> u64 {
    epoch_ms(SystemTime::now())
}

// One run, start to finish, read in order.
#[test]
#[ignore = "opens the Switchboard window and runs claude with --model haiku"]
#[allow(clippy::too_many_lines)]
fn an_issue_is_investigated_by_a_real_agent() {
    let tmp = tempfile::tempdir().unwrap();
    let sb_dir = tmp.path().join("sb");
    let data = DataDir::new(tmp.path().join("dz"));
    let home = std::env::var_os("HOME").unwrap();
    // A direct child of the trusted directory: one level deeper and
    // Claude Code asks again.
    let repo = Repo(
        Path::new(&home)
            .join("code_repos")
            .join(format!("switchboard-dispatch-live-{}", std::process::id())),
    );
    std::fs::create_dir_all(&repo.0).unwrap();
    std::fs::write(repo.0.join("README.md"), "throwaway\n").unwrap();

    let socket = format!("switchboard-test-{}-dispatch", std::process::id());
    let bin = Path::new(env!("CARGO_BIN_EXE_dispatch")).with_file_name("switchboard");
    assert!(bin.exists(), "build the workspace first: {}", bin.display());
    let mut cmd = Command::new(&bin);
    cmd.env("SWITCHBOARD_DATA_DIR", &sb_dir)
        .env("SWITCHBOARD_TMUX_SOCKET", &socket)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // A nested Claude inherits `CLAUDE*` variables that turn off its
    // transcript; the app's children must not.
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("CLAUDE") {
            cmd.env_remove(k);
        }
    }
    let app = App {
        child: cmd.spawn().unwrap(),
        socket,
    };
    let control = sb_dir.join(SOCKET_FILE);
    let started = Instant::now();
    while !control.exists() {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "no control socket"
        );
        std::thread::sleep(Duration::from_millis(200));
    }

    std::fs::create_dir_all(data.root.join("pipelines")).unwrap();
    let pipeline = format!(
        r#"
version = 1

[project]
name = "Live"
root = "{root}"                 # in place: a plain directory, no clone, no branch
space = "Dispatch · Live"

[source]
kind = "github"
repo = "msull/switchboard"
label = "dispatch"

[[lanes]]
name = "repo"
path = "."

[operators.investigator]
kind = "claude"
args = ["--model", "haiku"]
guidance = "Do exactly what the prompt says, ask nothing, then stop."

[[stages]]
name = "investigate"
operator = "investigator"
context = "root"
writes = ["notes"]
prompt = "Write the single word pong to the file {{notes}} and stop."
"#,
        root = repo.0.display(),
    );
    std::fs::write(data.pipeline(PROJECT), &pipeline).unwrap();

    let mut runner = Runner::new(
        data.clone(),
        Box::new(SocketPort::new(&sb_dir)),
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
                title: "live check".into(),
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
    runner.recover(now_ms()).unwrap();

    let deadline = Instant::now() + Duration::from_secs(240);
    let ticket = loop {
        runner.step_project(PROJECT, now_ms()).unwrap();
        let t = runner.load_ticket(&id).unwrap();
        if !t.active() {
            break t;
        }
        assert!(Instant::now() < deadline, "timed out: {t:#?}");
        std::thread::sleep(Duration::from_secs(1));
    };
    assert!(
        matches!(&ticket.state, TicketState::Closed { .. }),
        "{ticket:#?}"
    );
    let attempt = ticket.attempts_of("investigate").last().unwrap();
    assert_eq!(attempt.state, AttemptState::Complete, "{ticket:#?}");
    let notes = std::fs::read_to_string(&attempt.artifacts["notes"]).unwrap();
    assert!(notes.to_lowercase().contains("pong"), "{notes}");

    // The investigator was killed, and the operations log has a reply
    // for every request Dispatch made (and the request line too for the
    // ones that made records).
    let mut client = Client::connect(&control).unwrap();
    let session = attempt.session.clone().unwrap();
    let reply = client
        .call(&Request::new(
            "q-live",
            Body::Session {
                session: session.clone(),
            },
        ))
        .unwrap();
    let Reply::Session { session: view } = reply else {
        panic!("{reply:?}")
    };
    assert!(view.op.is_some());
    assert!(
        !matches!(view.liveness, switchboard_control::Liveness::Running),
        "{view:?}"
    );
    let log = std::fs::read_to_string(sb_dir.join("operations.log")).unwrap();
    for op in &ticket.ledger {
        let expected = if op.class == "creation" { 2 } else { 1 };
        assert_eq!(
            log.matches(&op.op).count(),
            expected,
            "{}: {}",
            op.op,
            op.kind
        );
    }
    drop(app);
}
