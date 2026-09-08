//! Milestone 1 gate tests (design.md, "Milestones"): one test per gate
//! item, driving `SwitchboardApp` through `dispatch` and `poll_now` with
//! the real adapters the item needs and no egui at all.
//!
//! Real: `JsonStore` on a temp data dir, `TmuxHost` on a private
//! `switchboard-test-gate-<pid>-<n>` socket (killed by a drop guard),
//! `HookLog` on the same data dir, `Agents::detect` with hook settings
//! pointing at the freshly built `switchboard-hook`. Fake: `FakeOpener`
//! stands in for Ghostty so "attach" is observable without windows.
//!
//! Tests that run a real agent are `#[ignore]`d; run one with
//!
//! ```sh
//! cargo test --test gate -- --ignored claude_sessions_map_to_their_cards --nocapture
//! cargo test --test gate -- --ignored hook_events_while_down_apply_in_order --nocapture
//! cargo test --test gate -- --ignored codex_launches_bind_distinct_ids --nocapture
//! ```
//!
//! Interactive `claude` shows a trust dialog for a directory it has not
//! seen, and accepting writes `~/.claude.json`, so the Claude tests use a
//! throwaway subdirectory of `$HOME/code_repos`, which is already trusted
//! (spike 03). They add `"model": "haiku"` to the `--settings` file the
//! app hands Claude, so a turn costs a fraction of a cent. Interactive
//! `codex` blocks on an "update available" dialog and then a trust dialog
//! before writing its rollout; the Codex test answers both by writing
//! into its own panes. Trusting the throwaway directory leaves a
//! `[projects."<tmp>"]` entry in `~/.codex/config.toml`.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};

use switchboard::SwitchboardApp;
use switchboard::adapters::agents::Agents;
use switchboard::adapters::fakes::{FakeOpener, FakeSecrets};
use switchboard::adapters::hooks::{HookLog, unix_millis, write_hook_settings};
use switchboard::adapters::store::JsonStore;
use switchboard::adapters::tmux::TmuxHost;
use switchboard::adapters::transcript::ClaudeTranscripts;
use switchboard::app::Services;
use switchboard::core::{
    Activity, AgentKind, AppAction, AppCore, CardLayout, CardState, Clock, Effect, Launch, Project,
    ProjectEnv, ProjectId, RecordId, ResumeHandle, SessionKind, SessionRecord, Workspace,
};
use switchboard::ports::agent::{AgentLaunch, AgentLauncher};
use switchboard::ports::events::{EventKind, EventSource, SessionEvent};
use switchboard::ports::host::{HostId, HostStatus, Liveness, ProcessHost};
use switchboard::ports::store::{Loaded, Store};
use uuid::Uuid;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A temp data dir plus a private tmux server that dies with the guard.
struct Gate {
    tmp: tempfile::TempDir,
    data_dir: PathBuf,
    socket: String,
    tmux_conf: PathBuf,
    host: TmuxHost,
    opener: FakeOpener,
    hook_bin: PathBuf,
}

impl Drop for Gate {
    fn drop(&mut self) {
        let _ = self.host.kill_server();
    }
}

impl Gate {
    /// `None` when tmux is unusable here, so CI without tmux stays green.
    fn new() -> Option<Self> {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let socket = format!("switchboard-test-gate-{}-{n}", std::process::id());
        let tmp = tempfile::tempdir().expect("temp dir");
        let data_dir = tmp.path().join("data");
        let tmux_conf = data_dir.join("tmux.conf");
        TmuxHost::write_default_config(&tmux_conf).expect("tmux config");
        let host = TmuxHost::new(&socket, Some(tmux_conf.clone()));
        if let Err(e) = host.probe() {
            eprintln!("skipping gate test: {e}");
            return None;
        }
        if let Err(e) = host.smoke_test() {
            eprintln!("skipping gate test: cannot start a session here: {e}");
            let _ = host.kill_server();
            return None;
        }
        let hook_bin = PathBuf::from(env!("CARGO_BIN_EXE_switchboard-hook"));
        write_hook_settings(&data_dir, &hook_bin).expect("hook settings");
        Some(Self {
            tmp,
            data_dir,
            socket,
            tmux_conf,
            host,
            opener: FakeOpener::default(),
            hook_bin,
        })
    }

    /// An app on this data dir and server, as `main.rs` would build it.
    fn app(&self) -> SwitchboardApp {
        SwitchboardApp::with_services(Services {
            store: Box::new(JsonStore::new(self.data_dir.clone())),
            host: Box::new(self.host.clone()),
            events: Box::new(HookLog::new(&self.data_dir)),
            agents: Box::new(Agents::detect(&self.data_dir)),
            opener: Box::new(self.opener.clone()),
            transcripts: Box::new(ClaudeTranscripts),
            secrets: Box::new(FakeSecrets::default()),
            wake: None,
        })
    }

    fn started(&self) -> SwitchboardApp {
        let mut app = self.app();
        app.start();
        app
    }

    /// A locked store on the data dir, for seeding records before start.
    /// Drop it before starting the app or the app comes up read-only.
    fn store(&self) -> JsonStore {
        let mut store = JsonStore::new(self.data_dir.clone());
        assert_eq!(store.lock(), Ok(true));
        store
    }

    /// A fresh directory under the temp root, canonicalized so it matches
    /// what tmux reports as the pane's path.
    fn work_dir(&self, name: &str) -> PathBuf {
        let dir = self.tmp.path().join(name);
        fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn list(&self) -> Vec<HostStatus> {
        self.host.list().expect("tmux list")
    }

    fn snapshot(&self, id: RecordId) -> String {
        self.host
            .snapshot(&HostId(id.host_name()), Some(50))
            .unwrap_or_default()
    }

    /// Type `text` and press return in the record's own pane.
    fn type_line(&self, id: RecordId, text: &str) {
        let host = HostId(id.host_name());
        self.host.write(&host, text.as_bytes()).expect("send-keys");
        // Claude Code's input box needs a beat between text and return.
        std::thread::sleep(Duration::from_millis(300));
        self.host.write(&host, b"\r").expect("send-keys");
    }

    /// Sessions with a client attached, straight from tmux.
    fn attached_sessions(&self) -> Vec<String> {
        let out = Command::new("tmux")
            .args(["-L", &self.socket, "-f"])
            .arg(&self.tmux_conf)
            .args(["list-clients", "-F", "#{session_name}"])
            .output()
            .expect("tmux list-clients");
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn events_log(&self) -> PathBuf {
        self.data_dir.join("events.log")
    }

    fn events_offset(&self) -> u64 {
        fs::read_to_string(self.data_dir.join("events.offset"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    }
}

/// Polls the app every `step` until `ready`, failing after `timeout`.
fn wait_until(
    app: &mut SwitchboardApp,
    what: &str,
    timeout: Duration,
    step: Duration,
    mut ready: impl FnMut(&SwitchboardApp) -> bool,
) {
    let deadline = Instant::now() + timeout;
    loop {
        app.poll_now();
        if ready(app) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(step);
    }
}

const QUICK: Duration = Duration::from_millis(100);
const SLOW: Duration = Duration::from_millis(500);

fn add_project(app: &mut SwitchboardApp, root: &Path) -> ProjectId {
    app.dispatch(AppAction::AddProject {
        name: "gate".into(),
        root: root.to_path_buf(),
    });
    app.core()
        .workspaces()
        .last()
        .expect("project added")
        .project
        .id
}

fn new_session(
    app: &mut SwitchboardApp,
    project: ProjectId,
    name: &str,
    kind: SessionKind,
    cwd: &Path,
) -> RecordId {
    app.dispatch(AppAction::NewSession {
        project,
        name: name.into(),
        kind,
        cwd: cwd.to_path_buf(),
        launch: Launch::Shell,
    });
    app.core()
        .workspace(project)
        .and_then(|w| w.sessions.iter().find(|s| s.name == name))
        .expect("session record")
        .id
}

fn is_running(app: &SwitchboardApp, id: RecordId) -> bool {
    app.core()
        .host_status(id)
        .is_some_and(|h| matches!(h.liveness, Liveness::Running { .. }))
}

fn claude_session_id(app: &SwitchboardApp, id: RecordId) -> Uuid {
    match &app.core().session(id).expect("record").resume {
        Some(ResumeHandle::ClaudeCode { session_id, .. }) => *session_id,
        other => panic!("expected a Claude handle, got {other:?}"),
    }
}

fn project(root: &Path) -> Project {
    let now = SystemTime::now();
    Project {
        id: ProjectId::new(),
        name: "seeded".into(),
        root: root.to_path_buf(),
        tags: Vec::new(),
        notes: String::new(),
        pinned: Vec::new(),
        env: ProjectEnv::default(),
        created: now,
        last_active: now,
    }
}

fn record(project: ProjectId, name: &str, kind: SessionKind, cwd: &Path) -> SessionRecord {
    let now = SystemTime::now();
    SessionRecord {
        id: RecordId::new(),
        project,
        name: name.into(),
        kind,
        cwd: cwd.to_path_buf(),
        launch: Launch::Shell,
        env_profile: None,
        created: now,
        last_seen: now,
        notes: String::new(),
        resume: None,
        autostart: false,
        layout: CardLayout::default(),
        activity: Activity::Unknown,
        last_event_at: None,
        last_exit: None,
        not_resumable: false,
        scrollback: None,
    }
}

// ---- real Claude Code helpers (ignored tests only) ----

/// A throwaway project root under the already trusted `$HOME/code_repos`,
/// removed on drop. See the module docs for why not a temp dir.
struct TrustedRoot(PathBuf);

impl TrustedRoot {
    fn new() -> Self {
        let home = std::env::var_os("HOME").expect("HOME");
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = PathBuf::from(home)
            .join("code_repos")
            .join(format!(".switchboard-gate-{}-{n}", std::process::id()));
        fs::create_dir_all(&dir).expect("trusted root");
        Self(dir.canonicalize().unwrap())
    }
}

impl Drop for TrustedRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Adds `"model": "haiku"` to the settings file Claude is launched with.
fn cheap_claude_settings(data_dir: &Path) {
    let path = data_dir.join("claude-hooks.json");
    let mut v: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    v["model"] = serde_json::json!("haiku");
    fs::write(&path, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
}

fn has_event(events: &[SessionEvent], id: RecordId, pred: impl Fn(&EventKind) -> bool) -> bool {
    events
        .iter()
        .any(|e| e.record_id == Some(id) && pred(&e.kind))
}

/// Waits for each Claude session to start, gives it a one-turn prompt,
/// and waits for its `Stop`. Returns every event the log carried, read
/// with the test's own reader so the app's checkpoint does not hide them.
fn run_claude_turn(
    gate: &Gate,
    app: &mut SwitchboardApp,
    reader: &mut HookLog,
    ids: &[RecordId],
) -> Vec<SessionEvent> {
    let mut seen = Vec::new();
    wait_until(
        app,
        "SessionStart for every Claude session",
        Duration::from_secs(60),
        SLOW,
        |_| {
            seen.extend(reader.poll());
            ids.iter()
                .all(|&id| has_event(&seen, id, |k| *k == EventKind::SessionStart))
        },
    );
    for &id in ids {
        assert_eq!(
            app.core().card_state(id),
            CardState::Working,
            "an agent that just started is busy until a hook says otherwise"
        );
        gate.type_line(id, "reply with pong");
    }
    wait_until(
        app,
        "Stop for every Claude session",
        Duration::from_secs(90),
        SLOW,
        |_| {
            seen.extend(reader.poll());
            ids.iter()
                .all(|&id| has_event(&seen, id, |k| matches!(k, EventKind::Stopped { .. })))
        },
    );
    for &id in ids {
        assert_eq!(app.core().card_state(id), CardState::Idle);
        let kinds: Vec<&EventKind> = seen
            .iter()
            .filter(|e| e.record_id == Some(id))
            .map(|e| &e.kind)
            .collect();
        println!("{}: {kinds:?}", app.core().session(id).unwrap().name);
    }
    seen
}

// ---- gate item 1: two Claude Code sessions in the same cwd ----

/// Proves: two Claude sessions launched together in one directory get
/// distinct ids, each hook event lands on the record whose pane emitted
/// it (matched by the injected record id, never by cwd), the id in every
/// event equals the resume handle the core assigned before spawn, and the
/// cards move Working -> Idle independently.
#[test]
#[ignore = "runs two real claude sessions; a few cents"]
fn claude_sessions_map_to_their_cards() {
    let Some(gate) = Gate::new() else { return };
    cheap_claude_settings(&gate.data_dir);
    let root = TrustedRoot::new();
    let mut reader = HookLog::new(&gate.data_dir);
    let mut app = gate.started();
    let project = add_project(&mut app, &root.0);
    let kind = SessionKind::Agent(AgentKind::ClaudeCode);
    let a = new_session(&mut app, project, "gate-a", kind, &root.0);
    let b = new_session(&mut app, project, "gate-b", kind, &root.0);

    // PrepareLaunch -> Spawn -> Attach all ran synchronously.
    let (sid_a, sid_b) = (claude_session_id(&app, a), claude_session_id(&app, b));
    assert_ne!(sid_a, sid_b);
    assert_eq!(gate.list().len(), 2);
    let titles: Vec<String> = gate
        .opener
        .state()
        .terminals
        .iter()
        .map(|t| t.0.clone())
        .collect();
    assert_eq!(titles, vec![a.host_name(), b.host_name()]);

    let events = run_claude_turn(&gate, &mut app, &mut reader, &[a, b]);
    for (id, sid) in [(a, sid_a), (b, sid_b)] {
        let mine: Vec<&SessionEvent> = events.iter().filter(|e| e.record_id == Some(id)).collect();
        assert!(mine.len() >= 3, "{id:?} saw {mine:?}");
        for e in mine {
            assert_eq!(
                e.provider_session_id.as_deref(),
                Some(sid.to_string().as_str()),
                "event {e:?} carries the wrong session id for {id:?}"
            );
            assert_eq!(e.cwd.as_deref(), Some(root.0.as_path()));
        }
    }
    assert!(events.iter().all(|e| e.record_id.is_some()));

    app.dispatch(AppAction::KillSession(a));
    app.dispatch(AppAction::KillSession(b));
    wait_until(
        &mut app,
        "panes gone",
        Duration::from_secs(5),
        QUICK,
        |_| gate.list().is_empty(),
    );
    assert_eq!(app.core().card_state(a), CardState::NotRunning);
}

// ---- gate item 2: closing the terminal window detaches ----

/// Proves the host-level property behind "closing the Ghostty window":
/// a terminal is only a tmux client, so killing it (here a `script`-
/// wrapped `attach-session`, exactly the argv Ghostty would run) leaves
/// the session and its process untouched.
#[test]
fn killing_the_attached_client_keeps_the_session() {
    let Some(gate) = Gate::new() else { return };
    let work = gate.work_dir("work");
    let mut app = gate.started();
    let project = add_project(&mut app, &work);
    let id = new_session(&mut app, project, "shell", SessionKind::Shell, &work);
    wait_until(
        &mut app,
        "shell running",
        Duration::from_secs(5),
        QUICK,
        |app| is_running(app, id),
    );
    let pid_before = match &app.core().host_status(id).unwrap().liveness {
        Liveness::Running { pid, .. } => *pid,
        other => panic!("{other:?}"),
    };

    let argv = gate.host.attach_command(&HostId(id.host_name()));
    let mut client = Command::new("script")
        .args(["-q", "/dev/null"])
        .args(&argv)
        .env("TERM", "xterm-256color")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("script + tmux attach");
    let deadline = Instant::now() + Duration::from_secs(5);
    while gate.attached_sessions() != [id.host_name()] {
        assert!(Instant::now() < deadline, "client never attached");
        std::thread::sleep(QUICK);
    }
    std::thread::sleep(Duration::from_secs(2));
    client.kill().unwrap();
    client.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !gate.attached_sessions().is_empty() {
        assert!(Instant::now() < deadline, "client never detached");
        std::thread::sleep(QUICK);
    }

    app.poll_now();
    assert_eq!(app.core().card_state(id), CardState::Idle);
    assert!(matches!(
        app.core().host_status(id).unwrap().liveness,
        Liveness::Running { pid, .. } if pid == pid_before
    ));
    assert_eq!(gate.list().len(), 1);
}

// ---- gate item 3: repeated "return" never duplicates ----

/// Proves, on the real host: however many times a running record is
/// returned to, there is one tmux session for it; the opener is asked to
/// raise first and opens a window only when none exists.
#[test]
fn repeated_return_opens_at_most_one_window_and_one_session() {
    let Some(gate) = Gate::new() else { return };
    let work = gate.work_dir("work");
    let mut app = gate.started();
    let project = add_project(&mut app, &work);
    let id = new_session(&mut app, project, "shell", SessionKind::Shell, &work);
    let title = id.host_name();

    // Before the first poll the core has no host status for the new
    // record, but a successful spawn leaves a placeholder "running" status
    // so a return in that window attaches instead of spawning again.
    app.dispatch(AppAction::ReturnToSession(id));
    {
        let s = gate.opener.state();
        assert_eq!(s.raised, vec![title.clone()]);
        assert_eq!(s.terminals.len(), 1);
        assert_eq!(s.terminals[0].0, title);
        assert_eq!(
            s.terminals[0].1,
            gate.host.attach_command(&HostId(title.clone()))
        );
    }
    // The window now exists: further returns only raise it, before and
    // after the first real host poll.
    gate.opener.state().existing.push(title.clone());
    for _ in 0..2 {
        app.dispatch(AppAction::ReturnToSession(id));
    }
    assert_eq!(gate.list().len(), 1);

    wait_until(
        &mut app,
        "shell running",
        Duration::from_secs(5),
        QUICK,
        |app| is_running(app, id),
    );
    for _ in 0..2 {
        app.dispatch(AppAction::ReturnToSession(id));
    }
    {
        let s = gate.opener.state();
        assert_eq!(s.terminals.len(), 1);
        assert_eq!(s.raised.len(), 5);
    }
    assert_eq!(gate.list().len(), 1);
    assert_eq!(
        app.core().notices().iter().filter(|n| n.is_error).count(),
        0,
        "{:?}",
        app.core().notices()
    );
}

/// Proves, at the core: returns while an agent launch is in flight
/// (before `LaunchPrepared`, before `Spawned`) emit nothing, so one
/// record produces exactly one `Spawn`.
#[test]
fn return_while_in_flight_spawns_once() {
    /// Dispatches at `ms` and counts the `Spawn`s in the result.
    fn step(core: &mut AppCore, spawns: &mut usize, action: AppAction, ms: u64) -> Vec<Effect> {
        let effects = core.dispatch(action, Clock::at(ms));
        *spawns += effects
            .iter()
            .filter(|e| matches!(e, Effect::Spawn { .. }))
            .count();
        effects
    }
    let mut core = AppCore::new();
    let mut spawns = 0;
    step(
        &mut core,
        &mut spawns,
        AppAction::StoreLoaded(Ok(Loaded::default())),
        0,
    );
    step(&mut core, &mut spawns, AppAction::HostListed(Vec::new()), 1);
    step(
        &mut core,
        &mut spawns,
        AppAction::AddProject {
            name: "p".into(),
            root: "/work".into(),
        },
        2,
    );
    let project = core.workspaces()[0].project.id;
    let effects = step(
        &mut core,
        &mut spawns,
        AppAction::NewSession {
            project,
            name: "agent".into(),
            kind: SessionKind::Agent(AgentKind::ClaudeCode),
            cwd: "/work".into(),
            launch: Launch::Shell,
        },
        3,
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::PrepareLaunch { .. }))
    );
    let id = core.workspaces()[0].sessions[0].id;

    assert!(step(&mut core, &mut spawns, AppAction::ReturnToSession(id), 4).is_empty());
    assert!(step(&mut core, &mut spawns, AppAction::ReturnToSession(id), 5).is_empty());
    let launch = AgentLaunch {
        argv: vec!["claude".into()],
        env: Vec::new(),
        resume: Some(ResumeHandle::ClaudeCode {
            session_id: Uuid::new_v4(),
            transcript: None,
        }),
    };
    step(
        &mut core,
        &mut spawns,
        AppAction::LaunchPrepared {
            id,
            result: Ok(launch),
        },
        6,
    );
    assert_eq!(spawns, 1);
    assert!(step(&mut core, &mut spawns, AppAction::ReturnToSession(id), 7).is_empty());
    let effects = step(
        &mut core,
        &mut spawns,
        AppAction::Spawned { id, result: Ok(()) },
        8,
    );
    assert!(effects.iter().any(|e| matches!(e, Effect::Attach { .. })));
    step(
        &mut core,
        &mut spawns,
        AppAction::HostListed(vec![HostStatus {
            id: HostId(id.host_name()),
            liveness: Liveness::Running {
                pid: 1,
                command: "claude".into(),
            },
            cwd: None,
            last_activity: None,
            title: None,
        }]),
        9,
    );
    let effects = step(&mut core, &mut spawns, AppAction::ReturnToSession(id), 10);
    assert!(effects.iter().any(|e| matches!(e, Effect::Attach { .. })));
    assert_eq!(spawns, 1);
}

// ---- gate item 4: restart preserves live processes and state ----

/// Proves: dropping the app without any shutdown (a crash) leaves the
/// tmux session alive; a new app on the same data dir loads the records,
/// reconciles the warm pane as running, still reads its history, and
/// "return" attaches instead of spawning.
#[test]
fn restart_reattaches_to_live_sessions() {
    let Some(gate) = Gate::new() else { return };
    let work = gate.work_dir("work");
    let marker = format!("GATE_MARKER_{}", Uuid::new_v4().simple());
    let (project, id) = {
        let mut app = gate.started();
        let project = add_project(&mut app, &work);
        let id = new_session(&mut app, project, "shell", SessionKind::Shell, &work);
        wait_until(
            &mut app,
            "shell running",
            Duration::from_secs(5),
            QUICK,
            |app| is_running(app, id),
        );
        // Quoting splits the typed line so only the output matches.
        let (head, tail) = marker.split_at(8);
        gate.type_line(id, &format!("echo {head}\"\"{tail}"));
        wait_until(
            &mut app,
            "marker output",
            Duration::from_secs(5),
            QUICK,
            |_| gate.snapshot(id).contains(&marker),
        );
        (project, id)
        // `app` drops here: no shutdown, the store lock is released with it.
    };

    let mut app = gate.started();
    assert_eq!(app.core().workspaces().len(), 1);
    let ws = app.core().workspace(project).expect("workspace reloaded");
    assert_eq!(ws.project.root, work);
    assert_eq!(ws.sessions.len(), 1);
    assert_eq!(ws.sessions[0].id, id);
    assert_eq!(app.core().card_state(id), CardState::Idle);
    assert!(gate.snapshot(id).contains(&marker));

    app.dispatch(AppAction::ReturnToSession(id));
    app.poll_now();
    let s = gate.opener.state();
    assert_eq!(s.terminals.len(), 1);
    assert_eq!(s.terminals[0].0, id.host_name());
    assert_eq!(gate.list().len(), 1);
    assert!(app.core().notices().iter().all(|n| !n.is_error));
}

// ---- gate item 5: deleted transcript shows "not resumable" ----

/// Proves: returning to an agent record whose transcript is gone marks
/// the card not resumable and says so, and nothing is launched.
#[test]
fn missing_transcript_is_not_resumable() {
    let Some(gate) = Gate::new() else { return };
    let work = gate.work_dir("work");
    let mut ws = Workspace::new(project(&work));
    let mut agent = record(
        ws.project.id,
        "claude",
        SessionKind::Agent(AgentKind::ClaudeCode),
        &work,
    );
    agent.resume = Some(ResumeHandle::ClaudeCode {
        session_id: Uuid::new_v4(),
        transcript: Some(gate.data_dir.join("gone/transcript.jsonl")),
    });
    let id = agent.id;
    ws.sessions.push(agent);
    gate.store().save(&ws).unwrap();

    let mut app = gate.started();
    assert_eq!(app.core().card_state(id), CardState::NotRunning);
    app.dispatch(AppAction::ReturnToSession(id));
    app.poll_now();
    assert_eq!(app.core().card_state(id), CardState::NotResumable);
    assert!(app.core().session(id).unwrap().not_resumable);
    assert!(
        app.core()
            .notices()
            .iter()
            .any(|n| n.text.contains("not resumable")),
        "{:?}",
        app.core().notices()
    );
    assert!(gate.list().is_empty(), "nothing spawned");
    assert!(gate.opener.state().terminals.is_empty());
    assert!(!app.core().is_in_flight(id));

    // The state is durable: a restart still shows it.
    drop(app);
    let app = gate.started();
    assert_eq!(app.core().card_state(id), CardState::NotResumable);
}

// ---- gate item 6: two Codex launches in the same cwd ----

/// Proves: two Codex records created at the same moment in one directory
/// are launched one after the other, each is bound to its own rollout id,
/// the ids differ, and each rollout's `session_meta.cwd` is the launch
/// directory. Codex's startup dialogs are answered in the test's panes;
/// see the module docs.
#[test]
#[ignore = "runs two real codex sessions"]
fn codex_launches_bind_distinct_ids() {
    let Some(gate) = Gate::new() else { return };
    let work = gate.work_dir("codex-work");
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(&work)
            .status()
            .is_ok_and(|s| s.success())
    );
    let mut app = gate.started();
    let project = add_project(&mut app, &work);
    let kind = SessionKind::Agent(AgentKind::Codex);
    let a = new_session(&mut app, project, "codex-a", kind, &work);
    let b = new_session(&mut app, project, "codex-b", kind, &work);
    assert!(app.core().is_in_flight(a));
    assert!(app.core().queued_codex(b), "second launch waits its turn");
    assert_eq!(gate.list().len(), 1);

    let handle = |app: &SwitchboardApp, id: RecordId| match &app.core().session(id)?.resume {
        Some(ResumeHandle::Codex {
            rollout_id,
            transcript,
        }) => Some((rollout_id.clone(), transcript.clone())),
        _ => None,
    };
    let mut answered: HashSet<(RecordId, &str)> = HashSet::new();
    wait_until(
        &mut app,
        "both Codex ids",
        Duration::from_secs(120),
        SLOW,
        |app| {
            for id in [a, b] {
                if !is_running(app, id) {
                    continue;
                }
                let screen = gate.snapshot(id);
                for (needle, answer) in [("Update available", "2"), ("Do you trust", "1")] {
                    if screen.contains(needle) && answered.insert((id, needle)) {
                        gate.type_line(id, answer);
                    }
                }
            }
            handle(app, a).is_some() && handle(app, b).is_some()
        },
    );
    let (id_a, path_a) = handle(&app, a).unwrap();
    let (id_b, path_b) = handle(&app, b).unwrap();
    println!("codex-a: {id_a} {path_a:?}\ncodex-b: {id_b} {path_b:?}");
    assert_ne!(id_a, id_b);
    for (rollout_id, path) in [(&id_a, path_a), (&id_b, path_b)] {
        let path = path.expect("discovered rollout path");
        let first = fs::read_to_string(&path)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .to_owned();
        let meta: serde_json::Value = serde_json::from_str(&first).unwrap();
        assert_eq!(meta["type"], "session_meta");
        assert_eq!(meta["payload"]["cwd"], work.to_string_lossy().as_ref());
        assert!(
            path.to_string_lossy()
                .ends_with(&format!("-{rollout_id}.jsonl"))
        );
    }
    assert_eq!(gate.list().len(), 2);
    assert!(app.core().notices().iter().all(|n| !n.is_error));

    let agents = Agents::detect(&gate.data_dir);
    let resume = agents
        .prepare_resume(
            &app.core().session(a).unwrap().resume.clone().unwrap(),
            a,
            "codex-a",
            &work,
        )
        .unwrap();
    assert_eq!(resume.argv[1..], ["resume".to_string(), id_a]);

    app.dispatch(AppAction::KillSession(a));
    app.dispatch(AppAction::KillSession(b));
    wait_until(
        &mut app,
        "panes gone",
        Duration::from_secs(5),
        QUICK,
        |_| gate.list().is_empty(),
    );
}

// ---- gate item 7: corrupt record recovers from .bak ----

/// Proves: a truncated record file falls back to its `.bak` on startup,
/// the workspace is fully loaded, and the bottom bar gets an error notice
/// that says it was recovered.
#[test]
fn corrupt_record_recovers_from_backup_with_notice() {
    let Some(gate) = Gate::new() else { return };
    let work = gate.work_dir("work");
    let mut ws = Workspace::new(project(&work));
    ws.sessions
        .push(record(ws.project.id, "shell", SessionKind::Shell, &work));
    {
        let store = gate.store();
        store.save(&ws).unwrap();
        store.save(&ws).unwrap();
    }
    let path = gate
        .data_dir
        .join("projects")
        .join(format!("{}.json", ws.project.id.0));
    let full = fs::read(&path).unwrap();
    fs::write(&path, &full[..full.len() / 2]).unwrap();

    let app = gate.started();
    assert_eq!(app.core().workspaces(), std::slice::from_ref(&ws));
    let notice = app
        .core()
        .notices()
        .iter()
        .find(|n| n.text.contains("recovered"))
        .unwrap_or_else(|| panic!("no recovery notice in {:?}", app.core().notices()));
    assert!(notice.is_error);
    assert!(notice.text.contains(&*path.to_string_lossy()));
    assert!(!app.core().read_only());
}

// ---- gate item 8: hostile project-local config is ignored ----

/// Proves: a `<root>/.switchboard/project.json` with an autostart
/// service is never read, on first add or on a later reconcile. Nothing
/// under a project root is parsed as configuration in Milestone 1.
#[test]
fn hostile_project_config_is_ignored() {
    let Some(gate) = Gate::new() else { return };
    let root = gate.work_dir("hostile");
    let marker = gate.tmp.path().join("pwned");
    let hostile = Workspace {
        schema_version: 1,
        project: project(&root),
        sessions: vec![SessionRecord {
            kind: SessionKind::Service,
            launch: Launch::Command {
                command: format!("touch {}", marker.display()),
                shell: "/bin/sh".into(),
            },
            autostart: true,
            ..record(ProjectId::new(), "evil", SessionKind::Service, &root)
        }],
    };
    let dir = root.join(".switchboard");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("project.json"),
        serde_json::to_vec_pretty(&hostile).unwrap(),
    )
    .unwrap();

    let mut app = gate.started();
    let project = add_project(&mut app, &root);
    for _ in 0..5 {
        app.poll_now();
        std::thread::sleep(QUICK);
    }
    assert!(app.core().workspace(project).unwrap().sessions.is_empty());
    assert!(!marker.exists());
    assert!(gate.list().is_empty());

    // The reconcile on a restart sees only the private store.
    drop(app);
    let mut app = gate.started();
    for _ in 0..5 {
        app.poll_now();
        std::thread::sleep(QUICK);
    }
    assert!(app.core().workspace(project).unwrap().sessions.is_empty());
    assert!(!marker.exists());
    assert!(gate.list().is_empty());
}

// ---- gate item 9: hook events while the app was down ----

/// Proves: events appended to the log while no app was running are
/// applied on the next start (a real `PermissionRequest` from the hook
/// helper moves the card to waiting), an event older than the record's
/// last applied one is ignored even though it sits later in the log, and
/// the checkpoint advances past both.
#[test]
#[ignore = "runs one real claude session; a fraction of a cent"]
fn hook_events_while_down_apply_in_order() {
    let Some(gate) = Gate::new() else { return };
    cheap_claude_settings(&gate.data_dir);
    let root = TrustedRoot::new();
    let mut reader = HookLog::new(&gate.data_dir);
    let (id, sid, last_event_at) = {
        let mut app = gate.started();
        let project = add_project(&mut app, &root.0);
        let id = new_session(
            &mut app,
            project,
            "gate",
            SessionKind::Agent(AgentKind::ClaudeCode),
            &root.0,
        );
        run_claude_turn(&gate, &mut app, &mut reader, &[id]);
        let record = app.core().session(id).unwrap();
        assert_eq!(record.activity, Activity::Idle);
        let last = record.last_event_at.expect("Stop applied");
        (id, claude_session_id(&app, id), last)
    };
    assert_eq!(
        gate.events_offset(),
        fs::metadata(gate.events_log()).unwrap().len()
    );

    // Claude is still at its prompt in tmux. The helper stamps "now", so
    // this is the newer event.
    let payload = serde_json::json!({
        "session_id": sid.to_string(),
        "cwd": root.0,
        "hook_event_name": "PermissionRequest",
        "tool_name": "Bash",
    });
    let mut helper = Command::new(&gate.hook_bin)
        .arg("PermissionRequest")
        .env("SWITCHBOARD_RECORD_ID", id.0.to_string())
        .env("SWITCHBOARD_DATA_DIR", &gate.data_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    helper
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    assert!(helper.wait().unwrap().success());
    // A stale Stop, later in the log but stamped before the last applied
    // event, as a replay after a crash could produce.
    let stale = format!(
        "{{\"at\":{},\"seq\":0,\"pid\":1,\"event\":\"Stop\",\"record_id\":\"{}\",\"session_id\":\"{sid}\",\"cwd\":null,\"transcript_path\":null,\"notification_type\":null,\"tool_name\":null,\"reason\":null,\"last_message\":\"stale\"}}\n",
        unix_millis(last_event_at) - 1000,
        id.0
    );
    fs::OpenOptions::new()
        .append(true)
        .open(gate.events_log())
        .unwrap()
        .write_all(stale.as_bytes())
        .unwrap();

    let mut app = gate.started();
    let record = app.core().session(id).unwrap();
    assert_eq!(record.activity, Activity::WaitingOnYou);
    assert!(record.last_event_at.unwrap() > last_event_at);
    assert_eq!(app.core().card_state(id), CardState::WaitingOnYou);
    assert_eq!(app.core().waiting_count(), 1);
    assert_eq!(
        gate.events_offset(),
        fs::metadata(gate.events_log()).unwrap().len()
    );

    app.dispatch(AppAction::KillSession(id));
    wait_until(&mut app, "pane gone", Duration::from_secs(5), QUICK, |_| {
        gate.list().is_empty()
    });
}

// ---- gate item 10: autostart service ----

/// Proves: on startup the reconcile brings back a trusted autostart
/// service and nothing else; an agent record in the same store is left
/// for a click.
#[test]
fn autostart_service_starts_and_agent_does_not() {
    let Some(gate) = Gate::new() else { return };
    let work = gate.work_dir("work");
    let mut ws = Workspace::new(project(&work));
    let service = SessionRecord {
        launch: Launch::Command {
            command: "sleep 30".into(),
            shell: "/bin/sh".into(),
        },
        autostart: true,
        ..record(ws.project.id, "svc", SessionKind::Service, &work)
    };
    let mut agent = record(
        ws.project.id,
        "claude",
        SessionKind::Agent(AgentKind::ClaudeCode),
        &work,
    );
    let transcript = gate.data_dir.join("transcript.jsonl");
    fs::write(&transcript, "{}\n").unwrap();
    agent.resume = Some(ResumeHandle::ClaudeCode {
        session_id: Uuid::new_v4(),
        transcript: Some(transcript),
    });
    let (svc, cl) = (service.id, agent.id);
    ws.sessions = vec![service, agent];
    gate.store().save(&ws).unwrap();

    let mut app = gate.started();
    wait_until(
        &mut app,
        "service running",
        Duration::from_secs(5),
        QUICK,
        |app| is_running(app, svc),
    );
    assert_eq!(app.core().card_state(svc), CardState::Idle);
    let listed = gate.list();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id.0, svc.host_name());
    assert_eq!(app.core().card_state(cl), CardState::NotRunning);
    assert!(!app.core().is_in_flight(cl));
    assert!(gate.opener.state().terminals.is_empty());

    // A second start finds the pane warm and does not start it again.
    drop(app);
    let mut app = gate.started();
    app.poll_now();
    assert_eq!(app.core().card_state(svc), CardState::Idle);
    assert_eq!(gate.list().len(), 1);
    assert!(app.core().notices().iter().all(|n| !n.is_error));
}
