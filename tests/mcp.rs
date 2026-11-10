//! `switchboard-mcp` against the app's real remote port: the TLS
//! listener on a loopback port with fakes behind it, as `tests/remote.rs`
//! builds it, and the MCP server's library driven over a pipe pair, as
//! an agent session's client drives the binary. The app answers on this
//! thread, so every wait for an MCP reply pumps it.

// Tests assert emptiness with `assert!` throughout, and pass `json!`
// arguments by value as they read.
#![allow(clippy::assert_is_empty, clippy::needless_pass_by_value)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{Value, json};
use switchboard::SwitchboardApp;
use switchboard::adapters::fakes::{
    self, FakeDispatch, FakeHost, FakeOperations, FakeRemoteStore, FakeSecrets, FakeTranscripts,
};
use switchboard::app::Services;
use switchboard::core::{AppAction, RecordId};
use switchboard::ports::control::OpLine;
use switchboard::ports::dispatch::{ProjectView, Status, SupervisorView, TicketView};
use switchboard::ports::host::{HostId, HostStatus, Liveness};
use switchboard::ports::remote::Scope;
use switchboard::ports::transcript::{Conversation, Turn};
use switchboard_control::{Body, Client, Launch, RecordKind, Reply, Request, SessionKind};
use switchboard_mcp::{Server, config, pair};

/// The app, its port, its fakes, and an MCP server over pipes.
struct Rig {
    app: SwitchboardApp,
    addr: std::net::SocketAddr,
    control: PathBuf,
    remote: FakeRemoteStore,
    host: FakeHost,
    transcripts: FakeTranscripts,
    operations: FakeOperations,
    asks: Arc<Mutex<Vec<dispatch_control::AskView>>>,
    server: Arc<Server>,
    input: std::io::PipeWriter,
    output: Receiver<Value>,
    /// Every line the server wrote, for the token check.
    seen: Vec<String>,
    next: u64,
    dir: tempfile::TempDir,
    _sock: tempfile::TempDir,
}

fn status() -> Status {
    Status {
        data_dir: "/dispatch".into(),
        worktrees: "/wt".into(),
        projects: vec![ProjectView {
            name: "Orchard".into(),
            queue: vec!["t1".into()],
            slots: 1,
            supervisor: Some(SupervisorView {
                session: Some("sup".into()),
                ..SupervisorView::default()
            }),
            ..ProjectView::default()
        }],
        tickets: vec![TicketView {
            id: "t1".into(),
            project: "Orchard".into(),
            kind: "github".into(),
            number: Some(7),
            title: "One file per entry".into(),
            state: "active".into(),
            stages: vec!["investigate".into()],
            updated_ms: 5,
            ..TicketView::default()
        }],
        ..Status::default()
    }
}

fn rig() -> Rig {
    let host = FakeHost::default();
    let remote = FakeRemoteStore::default();
    let transcripts = FakeTranscripts::default();
    let operations = FakeOperations::default();
    let dispatch = FakeDispatch {
        status: Some(status()),
        briefs: [(
            "Orchard".to_owned(),
            dispatch_control::BriefView {
                project: "Orchard".into(),
                supervisor: "supervisor: session sup".into(),
                ..Default::default()
            },
        )]
        .into_iter()
        .collect(),
        ..FakeDispatch::default()
    };
    let asks = Arc::clone(&dispatch.asks);
    let services = Services {
        secrets: Arc::new(FakeSecrets::default()),
        host: Arc::new(host.clone()),
        remote: Box::new(remote.clone()),
        operations: Box::new(operations.clone()),
        transcripts: Arc::new(transcripts.clone()),
        dispatch: Some(Box::new(dispatch)),
        ..fakes::services()
    };
    let mut app = SwitchboardApp::with_services(services);
    app.ui_state.prompt_boxes.native = false;
    app.start();
    let sock = tempfile::Builder::new()
        .prefix("sbm")
        .tempdir_in("/tmp")
        .unwrap();
    let control = app.listen_at(sock.path(), || {}).expect("control socket");
    app.set_remote_port_override(Some(0));
    let addr = app.listen_remote(|| {}).expect("remote port");
    app.dispatch(AppAction::DispatchStatus(Some(status())));
    let dir = tempfile::tempdir().unwrap();
    let (reader, input) = std::io::pipe().unwrap();
    let (out_reader, out_writer) = std::io::pipe().unwrap();
    let server = Server::new(dir.path().join("mcp"), Box::new(out_writer));
    let running = Arc::clone(&server);
    std::thread::spawn(move || running.run(BufReader::new(reader)));
    let (tx, output) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(out_reader).lines() {
            let Ok(line) = line else { break };
            let _ = tx.send(serde_json::from_str(&line).unwrap_or(Value::String(line)));
        }
    });
    Rig {
        app,
        addr,
        control,
        remote,
        host,
        transcripts,
        operations,
        asks,
        server,
        input,
        output,
        seen: Vec::new(),
        next: 1,
        dir,
        _sock: sock,
    }
}

impl Rig {
    fn mcp_dir(&self) -> PathBuf {
        self.dir.path().join("mcp")
    }

    /// Send one request; its id.
    fn send(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next;
        self.next += 1;
        let line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(self.input, "{line}").unwrap();
        self.input.flush().unwrap();
        id
    }

    /// The reply to `id`, pumping the app and running `each` on every
    /// pass until it comes.
    fn reply_with(&mut self, id: u64, mut each: impl FnMut(&mut Self)) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.app.serve_pending();
            each(self);
            match self.output.try_recv() {
                Ok(v) => {
                    self.seen.push(v.to_string());
                    if v["id"] == id {
                        return v;
                    }
                }
                Err(TryRecvError::Disconnected) => panic!("the server stopped"),
                Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(2)),
            }
            assert!(Instant::now() < deadline, "no reply to {id}");
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.send(method, params);
        self.reply_with(id, |_| {})
    }

    /// A tool's text and whether it is an error.
    fn tool(&mut self, name: &str, args: Value) -> (String, bool) {
        let v = self.request("tools/call", json!({ "name": name, "arguments": args }));
        result_of(&v)
    }

    /// Open a pairing window for an agent with `scope` and pair the
    /// MCP's config with it as `name`; the lines `pair` printed.
    fn pair(&mut self, name: &str, scope: Scope) -> Vec<String> {
        self.app.dispatch(AppAction::OpenPairing {
            code: "ABCD2345".into(),
            scope,
            agent: true,
        });
        let spki = self.remote.state().identity.clone().unwrap().spki_sha256;
        let (dir, address, name) = (self.mcp_dir(), self.addr.to_string(), name.to_owned());
        let worker = std::thread::spawn(move || {
            pair::pair(
                &dir,
                &pair::Pairing {
                    name: &name,
                    address: &address,
                    spki_sha256: &spki,
                    code: "abcd2345",
                    device: "laptop",
                },
            )
        });
        while !worker.is_finished() {
            self.app.serve_pending();
            std::thread::sleep(Duration::from_millis(2));
        }
        worker.join().unwrap().expect("paired")
    }

    fn token(&self) -> String {
        config::load(&self.mcp_dir()).unwrap().hosts[0]
            .token
            .clone()
    }

    fn control(&mut self, request: Request) -> Reply {
        let path = self.control.clone();
        let client = std::thread::spawn(move || {
            Client::connect(&path)
                .expect("connect")
                .call(&request)
                .expect("reply")
        });
        while !client.is_finished() {
            self.app.serve_pending();
            std::thread::sleep(Duration::from_millis(2));
        }
        client.join().unwrap()
    }

    /// A space and a project rooted at `/tmp`; the project's id.
    fn project(&mut self) -> String {
        let reply = self.control(Request::new("sp", Body::SpaceNew { name: "D".into() }));
        let space = made(&reply, RecordKind::Space);
        let reply = self.control(Request::new(
            "pj",
            Body::ProjectAdd {
                space,
                name: "web".into(),
                root: PathBuf::from("/tmp"),
            },
        ));
        made(&reply, RecordKind::Project)
    }

    fn session(&mut self, project: &str, name: &str, kind: SessionKind, launch: Launch) -> String {
        let reply = self.control(Request::new(
            format!("se-{name}"),
            Body::SessionNew {
                project: project.to_owned(),
                name: name.into(),
                session_kind: kind,
                cwd: PathBuf::from("/tmp"),
                launch,
                prompt: None,
                notes: String::new(),
                env: std::collections::BTreeMap::new(),
                env_sets: Vec::new(),
                replaces: None,
            },
        ));
        let id = made(&reply, RecordKind::Session);
        self.app.poll_now();
        id
    }

    fn exited(&mut self, id: RecordId, code: i32) {
        let pane = HostId(id.host_name());
        let mut host = self.host.state();
        host.statuses.retain(|s| s.id != pane);
        host.statuses.push(HostStatus {
            id: pane,
            liveness: Liveness::Exited { code: Some(code) },
            cwd: None,
            last_activity: None,
            title: None,
        });
    }

    fn write_log(&self, id: RecordId, text: &str) {
        let log = self
            .app
            .core()
            .session(id)
            .unwrap()
            .runs
            .last()
            .unwrap()
            .log
            .clone();
        let dir =
            switchboard::adapters::scrollback::scrollback_dir(&fakes::services().store.data_dir());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(log), text).unwrap();
    }
}

fn made(reply: &Reply, kind: RecordKind) -> String {
    reply
        .made()
        .iter()
        .find(|m| m.kind == kind)
        .unwrap_or_else(|| panic!("no {kind:?} in {reply:?}"))
        .id
        .clone()
}

fn result_of(v: &Value) -> (String, bool) {
    let r = &v["result"];
    (
        r["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        r["isError"].as_bool().unwrap_or(false),
    )
}

fn json_of(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

#[test]
fn both_handshakes_are_answered_and_the_tools_are_listed() {
    let mut r = rig();
    let init = r.request("initialize", json!({ "protocolVersion": "2025-06-18" }));
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "switchboard-mcp");
    let discover = r.request("server/discover", json!({}));
    assert!(
        discover["result"]["capabilities"]["tools"].is_object(),
        "{discover}"
    );
    assert_eq!(r.request("ping", json!({}))["result"], json!({}));
    let tools = r.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for want in [
        "hosts",
        "projects",
        "sessions",
        "session",
        "conversation",
        "last_response",
        "screen",
        "waiting",
        "tickets",
        "ticket",
        "ticket_events",
        "ticket_file",
        "brief",
        "run_output",
        "wait",
        "new_session",
        "run",
        "decide",
        "close_ticket",
        "ask_supervisor",
        "dispatch",
        "resume",
        "add_project",
    ] {
        assert!(names.contains(&want), "{want} is missing from {names:?}");
    }
    assert!(
        !names.iter().any(|n| *n == "send" || *n == "shell"),
        "{names:?}"
    );
    let unknown = r.request("tools/call", json!({ "name": "send", "arguments": {} }));
    assert_eq!(unknown["error"]["code"], -32602, "{unknown}");
    let (text, error) = r.tool("projects", json!({}));
    assert!(error, "{text}");
    assert!(text.contains("no host is paired"), "{text}");
    let none = r.request("no/such", json!({}));
    assert_eq!(none["error"]["code"], -32601);
}

#[test]
fn a_paired_host_is_read_through_every_read_tool_and_its_token_stays_in_the_file() {
    let mut r = rig();
    let lines = r.pair("box", Scope::Control);
    let project = r.project();
    let agent = r.session(&project, "explain", SessionKind::Claude, Launch::Shell);
    let dir = r.mcp_dir();
    let mode = |p: &Path| {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    };
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(&dir.join(config::HOSTS_FILE)), 0o600);
    let token = r.token();
    let mut printed = lines;
    printed.extend(pair::hosts(&dir).unwrap());
    let (text, error) = r.tool("hosts", json!({}));
    assert!(!error, "{text}");
    assert_eq!(json_of(&text)["hosts"][0]["name"], "box");
    for (name, args) in [
        ("projects", json!({})),
        ("sessions", json!({ "project": "web" })),
        ("session", json!({ "session": agent })),
        ("waiting", json!({})),
        ("tickets", json!({})),
        ("ticket", json!({ "ticket": "t1" })),
        ("brief", json!({ "project": "Orchard", "host": "box" })),
    ] {
        let (text, error) = r.tool(name, args);
        assert!(!error, "{name}: {text}");
        printed.push(text);
    }
    let (text, _) = r.tool("sessions", json!({ "project": "web" }));
    assert_eq!(json_of(&text)["sessions"][0]["name"], "explain");
    let (text, _) = r.tool("brief", json!({ "project": "Orchard" }));
    assert_eq!(
        json_of(&text)["brief"]["supervisor"],
        "supervisor: session sup"
    );
    for line in printed.iter().chain(&r.seen) {
        assert!(!line.contains(&token), "the token leaked: {line}");
    }
}

#[test]
fn a_rotated_host_key_marks_the_host_and_every_call_says_to_pair_again() {
    let mut r = rig();
    r.pair("box", Scope::Control);
    let dir = r.mcp_dir();
    let mut hosts = config::load(&dir).unwrap();
    hosts.hosts[0].spki_sha256 = "00".repeat(32);
    config::save(&dir, &hosts).unwrap();
    let (text, error) = r.tool("projects", json!({}));
    assert!(error && text.contains("needs pairing again"), "{text}");
    assert!(config::load(&dir).unwrap().hosts[0].needs_pairing);
    let (text, error) = r.tool("tickets", json!({}));
    assert!(error && text.contains("switchboard-mcp pair box"), "{text}");
}

#[test]
fn an_old_host_without_the_owner_channel_is_named_too_old() {
    let mut r = rig();
    r.pair("box", Scope::Control);
    r.server.info.lock().unwrap().insert(
        "box".into(),
        json!({ "capabilities": ["read", "control"], "version": "0.13.0" }),
    );
    let (text, error) = r.tool("brief", json!({ "project": "Orchard" }));
    assert!(error, "{text}");
    assert!(
        text.starts_with("host too old: box runs Switchboard 0.13.0"),
        "{text}"
    );
}

#[test]
fn wait_returns_on_a_session_or_a_ticket_and_times_out_and_shares_one_socket() {
    let mut r = rig();
    r.pair("box", Scope::Control);
    let project = r.project();
    let agent = r.session(&project, "explain", SessionKind::Claude, Launch::Shell);
    // Two waits at once, one on the session and one on the ticket.
    let on_session = r.send(
        "tools/call",
        json!({ "name": "wait", "arguments": { "session": agent, "timeout": 20 } }),
    );
    let on_ticket = r.send(
        "tools/call",
        json!({ "name": "wait", "arguments": { "ticket": "t1", "timeout": 20 } }),
    );
    let started = Instant::now();
    let mut noted = false;
    let reply = r.reply_with(on_session, |r| {
        if !noted && started.elapsed() > Duration::from_millis(800) {
            noted = true;
            r.control(Request::new(
                "notes",
                Body::SessionNotes {
                    session: agent.clone(),
                    text: "moved".into(),
                },
            ));
        }
    });
    let (text, error) = result_of(&reply);
    assert!(!error, "{text}");
    assert_eq!(json_of(&text)["changed"], true, "{text}");
    let mut moved = false;
    let reply = r.reply_with(on_ticket, |r| {
        if !moved && started.elapsed() > Duration::from_millis(1_200) {
            moved = true;
            let mut next = status();
            next.tickets[0].updated_ms = 9;
            next.tickets[0].state = "parking".into();
            r.app.dispatch(AppAction::DispatchStatus(Some(next)));
        }
    });
    let (text, error) = result_of(&reply);
    assert!(!error, "{text}");
    let v = json_of(&text);
    assert_eq!(
        (v["changed"].clone(), v["reason"].clone()),
        (json!(true), json!("ticket"))
    );
    let (text, error) = r.tool("wait", json!({ "session": agent, "timeout": 1 }));
    assert!(!error, "{text}");
    assert_eq!(json_of(&text), json!({ "changed": false }));
    assert_eq!(r.server.hub.opened(), 1, "one socket for every wait");
}

#[test]
fn run_returns_an_approved_commands_exit_and_output_and_refuses_an_unapproved_one() {
    let mut r = rig();
    r.pair("box", Scope::Control);
    let project = r.project();
    let build = r.session(
        &project,
        "build",
        SessionKind::Command,
        Launch::Command {
            command: "make".into(),
            shell: "sh".into(),
        },
    );
    let id = RecordId(uuid::Uuid::parse_str(&build).unwrap());
    // The launch at creation ran and ended.
    r.exited(id, 0);
    r.app.poll_now();
    let call = r.send(
        "tools/call",
        json!({ "name": "run", "arguments": { "project": "web", "entry": "build", "timeout": 20 } }),
    );
    let mut finished = false;
    let reply = r.reply_with(call, |r| {
        let runs = r.app.core().session(id).unwrap().runs.len();
        if !finished && runs == 2 {
            finished = true;
            r.write_log(id, "compiling\nall done\n");
            r.exited(id, 3);
            r.app.poll_now();
        }
    });
    let (text, error) = result_of(&reply);
    assert!(!error, "{text}");
    let v = json_of(&text);
    assert_eq!(v["run"], 2, "{text}");
    assert_eq!(v["exit"], 3, "{text}");
    assert_eq!(v["output"], "compiling\nall done", "{text}");
    let (text, error) = r.tool("run_output", json!({ "session": build }));
    assert!(!error && text.contains("all done"), "{text}");
    assert_eq!(
        r.server.hub.opened(),
        1,
        "the run's wait opened the one socket"
    );

    // An entry of the project file nobody approved.
    let pid = switchboard::core::ProjectId(uuid::Uuid::parse_str(&project).unwrap());
    r.app.dispatch(AppAction::ProjectConfigRead {
        project: pid,
        result: Ok(Some(switchboard::ports::project_config::ProjectConfig {
            entries: vec![switchboard::ports::project_config::DefinedEntry {
                name: "lint".into(),
                kind: switchboard::core::SessionKind::Command,
                command: "make lint".into(),
                cwd: None,
                env: Vec::new(),
                autostart: false,
                outputs: Vec::new(),
            }],
            warnings: vec![],
            shell: "/bin/sh".into(),
            show: vec![],
        })),
    });
    let (text, error) = r.tool("run", json!({ "project": "web", "entry": "lint" }));
    assert!(error, "{text}");
    assert!(text.starts_with("not_approved"), "{text}");
}

#[test]
fn resume_without_confirm_sends_nothing_and_with_it_sends_once() {
    let mut r = rig();
    r.pair("box", Scope::Control);
    let project = r.project();
    let agent = r.session(&project, "explain", SessionKind::Claude, Launch::Shell);
    let resumes = |r: &Rig| {
        r.operations
            .lines
            .lock()
            .unwrap()
            .iter()
            .filter(|l| matches!(l, OpLine::Requested { kind, .. } if kind == "session.resume"))
            .count()
    };
    let (text, error) = r.tool("resume", json!({ "session": agent, "confirm": false }));
    assert!(error && text.contains("confirm"), "{text}");
    let (text, error) = r.tool("resume", json!({ "session": agent }));
    assert!(error, "{text}");
    assert_eq!(resumes(&r), 0, "nothing was sent");
    r.tool("resume", json!({ "session": agent, "confirm": true }));
    assert_eq!(resumes(&r), 1);
}

#[test]
fn ask_supervisor_waits_for_the_supervisors_answer() {
    let mut r = rig();
    r.pair("box", Scope::Control);
    let project = r.project();
    let sup = r.session(&project, "supervisor", SessionKind::Claude, Launch::Shell);
    let sup_id = RecordId(uuid::Uuid::parse_str(&sup).unwrap());
    r.app.dispatch(AppAction::ReturnToSession(sup_id));
    r.app.poll_now();
    let call = r.send(
        "tools/call",
        json!({ "name": "ask_supervisor", "arguments": {
            "project": "Orchard", "text": "Where does t1 stand?", "wait": true, "timeout": 20,
        } }),
    );
    let mut typed = false;
    let reply = r.reply_with(call, |r| {
        let id = {
            let asks = r.asks.lock().unwrap();
            match asks.first() {
                Some(a) if !typed => a.id.clone(),
                _ => return,
            }
        };
        typed = true;
        {
            let mut asks = r.asks.lock().unwrap();
            asks[0].state = "delivered".into();
            asks[0].session = Some(sup.clone());
        }
        r.transcripts.set(Some(Conversation {
            turns: vec![Turn {
                n: 1,
                user: format!("Owner message {id} (from mcp:laptop):\nAn orchestrating session, not the owner, asks; answer in your next response.\nWhere does t1 stand?"),
                final_text: "t1 is investigating".into(),
                end: Some(SystemTime::now()),
                ..Turn::default()
            }],
            ..Conversation::default()
        }));
    });
    let (text, error) = result_of(&reply);
    assert!(!error, "{text}");
    let v = json_of(&text);
    assert_eq!(v["state"], "answered", "{text}");
    assert_eq!(v["reply"], "t1 is investigating");
    let asks = r.asks.lock().unwrap();
    assert_eq!(asks.len(), 1);
    assert_eq!(asks[0].by, "mcp:laptop");
}
