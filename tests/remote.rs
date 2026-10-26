//! The remote port end to end inside one process: the real TLS listener
//! on a loopback port, a pinned client on a thread, the app with fakes
//! answering on its "frame". What a device relies on is checked here:
//! `/info` to anyone and nothing else, pairing inside the window with
//! the host's own key, a token that reads every view, revocation and
//! scope from the dialog's side, the allowlist, the bounds, and a
//! screen with the project's secret values named rather than shown.

// Tests assert emptiness with `assert!` throughout.
#![allow(clippy::assert_is_empty)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};
use switchboard::SwitchboardApp;
use switchboard::adapters::fakes::{
    self, FakeAdvertiser, FakeDispatch, FakeHost, FakeOperations, FakeRemoteStore, FakeSecrets,
    FakeTranscripts, MemoryStore,
};
use switchboard::adapters::remote::{
    self, MAX_UNAUTHENTICATED, MAX_UNAUTHENTICATED_PER_SOURCE, pinned_client,
};
use switchboard::app::Services;
use switchboard::content::ContentKey;
use switchboard::core::{AppAction, Ask, RecordId};
use switchboard::ports::events::{EventKind, SessionEvent};
use switchboard::ports::host::{HostId, HostStatus, Liveness as HostLiveness};
use switchboard::ports::remote::{Listen, Scope};
use switchboard::ports::transcript::{Conversation, Turn};
use switchboard_control::{
    Body, Client, Launch, Made, Pin, PinTarget, RecordKind, Rect, Reply, Request, SessionKind,
};
use tungstenite::client::IntoClientRequest as _;

struct Port {
    app: SwitchboardApp,
    addr: SocketAddr,
    client: Arc<ClientConfig>,
    control: PathBuf,
    host: FakeHost,
    secrets: FakeSecrets,
    remote: FakeRemoteStore,
    advertiser: FakeAdvertiser,
    transcripts: FakeTranscripts,
    _dir: tempfile::TempDir,
}

/// A started app with the remote port on a free loopback port and the
/// control socket beside it (the tests make records through the latter).
fn port() -> Port {
    port_with(
        FakeHost::default(),
        FakeSecrets::default(),
        FakeRemoteStore::default(),
    )
}

fn port_with(host: FakeHost, secrets: FakeSecrets, remote: FakeRemoteStore) -> Port {
    port_built(host, secrets, remote, FakeDispatch::default())
}

/// A port whose Dispatch runner answers `status`, which the app is
/// handed as its last poll.
fn port_dispatching(status: switchboard::ports::dispatch::Status) -> Port {
    use switchboard::ports::dispatch::EventView;
    let mut port = port_built(
        FakeHost::default(),
        FakeSecrets::default(),
        FakeRemoteStore::default(),
        FakeDispatch {
            status: Some(status.clone()),
            artifacts: [(NOTES_PATH.into(), "Found it: one file each.\n".to_owned())]
                .into_iter()
                .collect(),
            events: vec![
                EventView {
                    seq: 1,
                    at_ms: 3,
                    stage: "investigate".into(),
                    kind: "stage".into(),
                    text: "queued → investigate".into(),
                    ..EventView::default()
                },
                EventView {
                    seq: 2,
                    at_ms: 3,
                    stage: "investigate".into(),
                    kind: "attempt-started".into(),
                    text: "root #1 agent".into(),
                    attempt: Some(("investigate".into(), 1)),
                    ..EventView::default()
                },
            ],
            ..FakeDispatch::default()
        },
    );
    port.app.dispatch(AppAction::DispatchStatus(Some(status)));
    port
}

fn port_built(
    host: FakeHost,
    secrets: FakeSecrets,
    remote: FakeRemoteStore,
    dispatch: FakeDispatch,
) -> Port {
    let advertiser = FakeAdvertiser::default();
    let transcripts = FakeTranscripts::default();
    let services = Services {
        secrets: std::sync::Arc::new(secrets.clone()),
        host: Arc::new(host.clone()),
        remote: Box::new(remote.clone()),
        advertiser: Box::new(advertiser.clone()),
        operations: Box::new(FakeOperations::default()),
        transcripts: Arc::new(transcripts.clone()),
        dispatch: Some(Box::new(dispatch)),
        ..fakes::services()
    };
    let mut app = SwitchboardApp::with_services(services);
    // The editors a draft is read from save through a fake dialog.
    app.ui_state.prompt_boxes.native = false;
    app.start();
    let dir = tempfile::Builder::new()
        .prefix("sbr")
        .tempdir_in("/tmp")
        .expect("temp dir");
    let control = app.listen_at(dir.path(), || {}).expect("control socket");
    app.set_remote_port_override(Some(0));
    let addr = app.listen_remote(|| {}).expect("remote port");
    let spki = remote
        .state()
        .identity
        .as_ref()
        .unwrap()
        .spki_sha256
        .clone();
    Port {
        app,
        addr,
        client: pinned_client(&spki).unwrap(),
        control,
        host,
        secrets,
        remote,
        advertiser,
        transcripts,
        _dir: dir,
    }
}

/// A conversation of `n` turns; the last answer names a token.
fn conversation(n: usize) -> Conversation {
    let mut c = Conversation {
        title: Some("explain-repo".into()),
        model: Some("claude-fable-5-1".into()),
        branch: Some("main".into()),
        ..Conversation::default()
    };
    for i in 1..=n {
        c.turns.push(Turn {
            n: i,
            user: format!("prompt {i}"),
            final_text: if i == n {
                "the token is tok-5e3cr3t".into()
            } else {
                format!("answer {i}")
            },
            assistant_msgs: 1,
            activity: vec![switchboard::ports::transcript::Activity {
                kind: switchboard::ports::transcript::ActivityKind::Tool,
                line: "Read README.md".into(),
                at: None,
                error: false,
                detail: Some(switchboard::ports::transcript::ToolDetail {
                    name: "Read".into(),
                    input: r#"{"file_path": "README.md"}"#.into(),
                    result: "# Switchboard\n".repeat(i * 40),
                    path: None,
                }),
                text: None,
            }],
            tools: 1,
            ..Turn::default()
        });
    }
    c
}

/// Launch the session so the fake agent launcher binds a Claude Code
/// handle to it (the transcript fake serves every handle).
fn launched(port: &mut Port, session: &str) -> RecordId {
    let id = RecordId(uuid::Uuid::parse_str(session).unwrap());
    port.app.dispatch(AppAction::ReturnToSession(id));
    port.app.poll_now();
    id
}

/// A command session with its first run opened, and the run's log
/// written with `text`.
fn command_with_run(port: &mut Port, project: &str, text: &str) -> (String, RecordId) {
    let reply = control(
        port,
        Request::new(
            "cmd",
            Body::SessionNew {
                project: project.to_owned(),
                name: "build".into(),
                session_kind: SessionKind::Command,
                cwd: PathBuf::from("/tmp"),
                launch: Launch::Command {
                    command: "make".into(),
                    shell: "sh".into(),
                },
                prompt: None,
                notes: String::new(),
                env: std::collections::BTreeMap::new(),
                env_sets: Vec::new(),
                replaces: None,
            },
        ),
    );
    let session = made_id(&reply, RecordKind::Session);
    let id = launched(port, &session);
    let run = port
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
    std::fs::write(dir.join(run), text).unwrap();
    (session, id)
}

/// One HTTP request over a fresh pinned TLS connection: the status and
/// the body.
fn http(
    addr: SocketAddr,
    config: &Arc<ClientConfig>,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    body: &str,
) -> std::io::Result<(u16, String)> {
    let tcp = TcpStream::connect(addr)?;
    tcp.set_read_timeout(Some(Duration::from_secs(10)))?;
    let conn = ClientConnection::new(
        Arc::clone(config),
        ServerName::try_from("localhost").unwrap(),
    )
    .map_err(std::io::Error::other)?;
    let mut s = StreamOwned::new(conn, tcp);
    while s.conn.is_handshaking() {
        s.conn.complete_io(&mut s.sock)?;
    }
    let auth = bearer
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{auth}Content-Length: {}\r\n\r\n{body}",
        body.len()
    )?;
    s.flush()?;
    let mut reader = BufReader::new(s);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let status: u16 = line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, line.clone()))?;
    let mut length = 0usize;
    loop {
        let mut h = String::new();
        reader.read_line(&mut h)?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some(v) = h.strip_prefix("Content-Length:") {
            length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut out = vec![0; length];
    reader.read_exact(&mut out)?;
    Ok((status, String::from_utf8_lossy(&out).into_owned()))
}

/// A request from a client thread, served by the app on this thread.
fn call(
    port: &mut Port,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    body: &str,
) -> (u16, serde_json::Value) {
    let addr = port.addr;
    let config = Arc::clone(&port.client);
    let (method, path, bearer, body) = (
        method.to_owned(),
        path.to_owned(),
        bearer.map(str::to_owned),
        body.to_owned(),
    );
    let client =
        std::thread::spawn(move || http(addr, &config, &method, &path, bearer.as_deref(), &body));
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !client.is_finished() {
        assert!(std::time::Instant::now() < deadline, "no reply");
        port.app.serve_pending();
        std::thread::sleep(Duration::from_millis(2));
    }
    let (status, text) = client.join().unwrap().expect("http");
    let value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"));
    (status, value)
}

fn get(port: &mut Port, path: &str, bearer: Option<&str>) -> (u16, serde_json::Value) {
    call(port, "GET", path, bearer, "")
}

/// The error code of a refusal.
fn code(value: &serde_json::Value) -> &str {
    value["error"]["code"].as_str().unwrap_or("")
}

/// One call on the control socket, for making records.
fn control(port: &mut Port, request: Request) -> Reply {
    let path = port.control.clone();
    let client = std::thread::spawn(move || {
        let mut client = Client::connect(&path).expect("connect");
        client.call(&request).expect("reply")
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !client.is_finished() {
        assert!(std::time::Instant::now() < deadline, "no reply");
        port.app.serve_pending();
        std::thread::sleep(Duration::from_millis(2));
    }
    client.join().unwrap()
}

fn made_id(reply: &Reply, kind: RecordKind) -> String {
    reply
        .made()
        .iter()
        .find(|m: &&Made| m.kind == kind)
        .unwrap_or_else(|| panic!("no {kind:?} in {reply:?}"))
        .id
        .clone()
}

/// A space, a project in it, and a session in that, with `notes`.
fn seed(port: &mut Port, notes: &str) -> (String, String, String) {
    let reply = control(
        port,
        Request::new("sp", Body::SpaceNew { name: "D".into() }),
    );
    let space = made_id(&reply, RecordKind::Space);
    let reply = control(
        port,
        Request::new(
            "pj",
            Body::ProjectAdd {
                space: space.clone(),
                name: "#1".into(),
                root: PathBuf::from("/tmp"),
            },
        ),
    );
    let project = made_id(&reply, RecordKind::Project);
    let reply = control(
        port,
        Request::new(
            "se",
            Body::SessionNew {
                project: project.clone(),
                name: "investigator".into(),
                session_kind: SessionKind::Claude,
                cwd: PathBuf::from("/tmp"),
                launch: Launch::Shell,
                prompt: None,
                notes: notes.into(),
                env: std::collections::BTreeMap::new(),
                env_sets: Vec::new(),
                replaces: None,
            },
        ),
    );
    let session = made_id(&reply, RecordKind::Session);
    // The host lists no pane for it: a session that is not running.
    port.app.poll_now();
    (space, project, session)
}

/// Open the pairing window with `code` and pair as `name`: the token.
fn pair(port: &mut Port, code: &str, scope: Scope, name: &str) -> String {
    port.app.dispatch(AppAction::OpenPairing {
        code: code.into(),
        scope,
    });
    let spki = port
        .remote
        .state()
        .identity
        .as_ref()
        .unwrap()
        .spki_sha256
        .clone();
    let body = serde_json::json!({ "code": code, "device_name": name, "spki_sha256": spki });
    let (status, reply) = call(port, "POST", "/api/v1/pair", None, &body.to_string());
    assert_eq!(status, 200, "{reply}");
    reply["body"]["token"].as_str().unwrap().to_owned()
}

#[test]
fn info_is_served_to_anyone_and_every_other_read_is_refused_unpaired() {
    let mut port = port();
    let (status, info) = get(&mut port, "/api/v1/info", None);
    assert_eq!(status, 200, "{info}");
    let identity = port.remote.state().identity.clone().unwrap();
    let body = &info["body"];
    assert_eq!(body["server_id"], identity.server_id.to_string());
    assert_eq!(body["server_name"], identity.server_name);
    assert_eq!(body["spki_sha256"], identity.spki_sha256);
    assert_eq!(body["protocol"], 1);
    assert_eq!(body["pairing_open"], false);
    assert_eq!(body["limits"]["screen_lines"], 200);
    assert!(info["epoch"].is_string());
    assert_eq!(info["revision"], 0);
    assert!(
        !info.to_string().contains("remote-key"),
        "no key material in /info"
    );

    for path in [
        "/api/v1/projects",
        "/api/v1/spaces",
        "/api/v1/waiting",
        "/api/v1/sessions/x",
    ] {
        let (status, reply) = get(&mut port, path, None);
        assert_eq!(
            (status, code(&reply)),
            (401, "unauthorized"),
            "{path}: {reply}"
        );
        let (status, reply) = get(&mut port, path, Some("not-a-token"));
        assert_eq!(
            (status, code(&reply)),
            (401, "unauthorized"),
            "{path}: {reply}"
        );
    }
    let (status, reply) = get(&mut port, "/other", None);
    assert_eq!((status, code(&reply)), (404, "not_found"), "{reply}");
    let (status, reply) = call(
        &mut port,
        "POST",
        "/api/v1/pair",
        None,
        r#"{"code":"AAAAAAAA","spki_sha256":"x"}"#,
    );
    assert_eq!((status, code(&reply)), (403, "pairing_closed"), "{reply}");
}

#[test]
fn a_pairing_inside_the_window_gives_a_token_that_reads_every_view() {
    let mut port = port();
    let (space, project, session) = seed(&mut port, "ticket 1");
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    assert_eq!(token.len(), 43);
    assert!(port.app.core().pairing().is_none(), "the code is spent");
    {
        let saved = port.remote.state();
        assert_eq!(saved.devices.len(), 1);
        assert_eq!(saved.devices[0].name, "Vision Pro");
        assert_eq!(saved.devices[0].scope, Scope::Read);
        assert_eq!(saved.devices[0].token_sha256, remote::token_sha256(&token));
        let text = serde_json::to_string(&saved.devices).unwrap();
        assert!(!text.contains(&token), "the token itself is never stored");
    }
    let t = Some(token.as_str());
    let (status, reply) = get(&mut port, "/api/v1/projects", t);
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["projects"][0]["id"], project);
    assert!(reply["epoch"].is_string());
    let (status, reply) = get(&mut port, &format!("/api/v1/projects?space={space}"), t);
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["projects"].as_array().unwrap().len(), 1);
    let (status, reply) = get(&mut port, "/api/v1/spaces", t);
    assert_eq!(status, 200, "{reply}");
    assert!(
        reply["body"]["spaces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == space)
    );
    let (status, reply) = get(&mut port, &format!("/api/v1/spaces/{space}/sets"), t);
    assert_eq!(status, 200, "{reply}");
    assert!(reply["body"]["sets"].is_array());
    let (status, reply) = get(
        &mut port,
        &format!("/api/v1/projects/{project}/sessions"),
        t,
    );
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["sessions"][0]["id"], session);
    assert_eq!(reply["body"]["sessions"][0]["notes"], "ticket 1");
    let (status, reply) = get(&mut port, &format!("/api/v1/sessions/{session}"), t);
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["session"]["name"], "investigator");
    let (status, reply) = get(&mut port, "/api/v1/waiting", t);
    assert_eq!(status, 200, "{reply}");
    assert!(reply["body"]["sessions"].is_array());
    let (status, reply) = get(&mut port, &format!("/api/v1/sessions/{session}/screen"), t);
    assert_eq!(
        (status, code(&reply)),
        (400, "failed"),
        "not running: {reply}"
    );
    let (status, reply) = get(&mut port, "/api/v1/sessions/not-a-uuid", t);
    assert_eq!((status, code(&reply)), (400, "failed"), "{reply}");
    let (status, reply) = get(
        &mut port,
        &format!("/api/v1/sessions/{}", uuid::Uuid::nil()),
        t,
    );
    assert_eq!((status, code(&reply)), (404, "not_found"), "{reply}");
    let (status, reply) = get(&mut port, "/api/v1/info", t);
    assert_eq!(status, 200, "{reply}");
    assert!(
        port.app.core().devices()[0].last_seen.is_some(),
        "an authenticated request marks the device seen"
    );
}

/// A working set made through the control port is read whole with its
/// pins through `/sets/{id}`, found across the spaces.
#[test]
fn a_set_is_read_by_its_id_across_spaces() {
    let mut port = port();
    let (space, _, session) = seed(&mut port, "");
    let reply = control(
        &mut port,
        Request::new(
            "st",
            Body::SetNew {
                space: space.clone(),
                name: "Deploy".into(),
            },
        ),
    );
    let set = made_id(&reply, RecordKind::Set);
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Pad");
    let (status, reply) = get(&mut port, &format!("/api/v1/sets/{set}"), Some(&token));
    assert_eq!(status, 200, "{reply}");
    let view = &reply["body"]["sets"][0];
    assert_eq!(view["id"], set);
    assert_eq!(view["name"], "Deploy");
    assert!(view["items"].is_array(), "{view}");
    assert_ne!(session, "");
    let (status, reply) = get(
        &mut port,
        &format!("/api/v1/sets/{}", uuid::Uuid::nil()),
        Some(&token),
    );
    assert_eq!((status, code(&reply)), (404, "not_found"), "{reply}");
}

#[test]
fn a_pairing_with_another_hosts_key_or_the_wrong_code_counts_and_three_lock_the_address_out() {
    let mut port = port();
    port.app.dispatch(AppAction::OpenPairing {
        code: "ABCD2345".into(),
        scope: Scope::Read,
    });
    let spki = port
        .remote
        .state()
        .identity
        .as_ref()
        .unwrap()
        .spki_sha256
        .clone();
    let (status, info) = get(&mut port, "/api/v1/info", None);
    assert_eq!(status, 200);
    assert_eq!(info["body"]["pairing_open"], true);
    // The right code through a substituted certificate: refused.
    let relayed = serde_json::json!({ "code": "ABCD2345", "device_name": "x", "spki_sha256": "ff".repeat(32) });
    let (status, reply) = call(
        &mut port,
        "POST",
        "/api/v1/pair",
        None,
        &relayed.to_string(),
    );
    assert_eq!((status, code(&reply)), (403, "wrong_key"), "{reply}");
    assert_eq!(port.app.core().pairing().unwrap().failures, 1);
    let wrong = serde_json::json!({ "code": "ZZZZZZZZ", "device_name": "x", "spki_sha256": spki });
    let (status, reply) = call(&mut port, "POST", "/api/v1/pair", None, &wrong.to_string());
    assert_eq!((status, code(&reply)), (403, "wrong_code"), "{reply}");
    let (status, reply) = call(&mut port, "POST", "/api/v1/pair", None, "not json");
    assert_eq!((status, code(&reply)), (400, "bad_request"), "{reply}");
    assert_eq!(
        port.app.core().pairing().unwrap().failures,
        2,
        "a bad body is not an attempt"
    );
    let (status, reply) = call(&mut port, "POST", "/api/v1/pair", None, &wrong.to_string());
    assert_eq!((status, code(&reply)), (403, "wrong_code"), "{reply}");
    let window = port
        .app
        .core()
        .pairing()
        .expect("three failures leave the window open")
        .clone();
    let locked: Vec<_> = window.locked().map(|s| (s.ip, s.failures)).collect();
    assert_eq!(locked, vec![(IpAddr::from([127, 0, 0, 1]), 3)]);
    // The right code, from the locked address.
    let right = serde_json::json!({ "code": "abcd2345", "device_name": "x", "spki_sha256": spki });
    let (status, reply) = call(&mut port, "POST", "/api/v1/pair", None, &right.to_string());
    assert_eq!((status, code(&reply)), (403, "pairing_locked"), "{reply}");
    assert!(port.remote.state().devices.is_empty());
    assert!(port.app.core().devices().is_empty());
    let (status, info) = get(&mut port, "/api/v1/info", None);
    assert_eq!(status, 200);
    assert_eq!(info["body"]["pairing_open"], true, "still open for others");
}

/// A body with no key never saw the QR: it is refused without
/// spending an attempt, so empty knocks cannot lock an address out.
#[test]
fn a_pair_body_with_no_key_is_refused_and_not_counted() {
    let mut port = port();
    port.app.dispatch(AppAction::OpenPairing {
        code: "ABCD2345".into(),
        scope: Scope::Read,
    });
    for body in [
        serde_json::json!({}),
        serde_json::json!({ "code": "ZZZZZZZZ", "spki_sha256": "  " }),
        serde_json::json!({ "code": "ZZZZZZZZ", "spki_sha256": 7 }),
    ] {
        let (status, reply) = call(&mut port, "POST", "/api/v1/pair", None, &body.to_string());
        assert_eq!((status, code(&reply)), (400, "bad_request"), "{reply}");
    }
    let window = port.app.core().pairing().unwrap();
    assert_eq!(window.failures, 0);
    assert!(window.sources.is_empty());
}

/// Reopening pairing clears a lockout: the owner's own mistyped codes
/// cost them a reopen, no more.
#[test]
fn reopening_pairing_lets_a_locked_address_pair() {
    let mut port = port();
    let open = |port: &mut Port| {
        port.app.dispatch(AppAction::OpenPairing {
            code: "ABCD2345".into(),
            scope: Scope::Read,
        });
    };
    open(&mut port);
    let spki = port
        .remote
        .state()
        .identity
        .as_ref()
        .unwrap()
        .spki_sha256
        .clone();
    let wrong = serde_json::json!({ "code": "ZZZZZZZZ", "device_name": "x", "spki_sha256": spki });
    for _ in 0..3 {
        let (status, reply) = call(&mut port, "POST", "/api/v1/pair", None, &wrong.to_string());
        assert_eq!((status, code(&reply)), (403, "wrong_code"), "{reply}");
    }
    let right = serde_json::json!({ "code": "ABCD2345", "device_name": "x", "spki_sha256": spki });
    let (status, reply) = call(&mut port, "POST", "/api/v1/pair", None, &right.to_string());
    assert_eq!((status, code(&reply)), (403, "pairing_locked"), "{reply}");
    open(&mut port);
    let (status, reply) = call(&mut port, "POST", "/api/v1/pair", None, &right.to_string());
    assert_eq!(status, 200, "{reply}");
    assert_eq!(port.app.core().devices().len(), 1);
}

#[test]
fn a_consumed_code_is_refused_and_the_window_grants_its_scope() {
    let mut port = port();
    let token = pair(&mut port, "ABCD2345", Scope::Control, "Pad");
    assert_eq!(port.app.core().devices()[0].scope, Scope::Control);
    let spki = port
        .remote
        .state()
        .identity
        .as_ref()
        .unwrap()
        .spki_sha256
        .clone();
    let again = serde_json::json!({ "code": "ABCD2345", "device_name": "y", "spki_sha256": spki });
    let (status, reply) = call(&mut port, "POST", "/api/v1/pair", None, &again.to_string());
    assert_eq!((status, code(&reply)), (403, "pairing_closed"), "{reply}");
    assert_eq!(port.app.core().devices().len(), 1);
    // The code is matched case-insensitively: it is typed by hand.
    let token2 = pair(&mut port, "wxyz6789", Scope::Read, "Phone");
    assert_ne!(token, token2);
    assert_eq!(port.app.core().devices().len(), 2);
}

#[test]
fn revoke_and_a_scope_change_take_effect_on_the_next_request() {
    let mut port = port();
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Pad");
    let id = port.app.core().devices()[0].id;
    let (status, _) = get(&mut port, "/api/v1/spaces", Some(&token));
    assert_eq!(status, 200);
    port.app.dispatch(AppAction::SetDeviceScope {
        device: id,
        scope: Scope::Control,
    });
    assert_eq!(
        port.remote.state().devices[0].scope,
        Scope::Control,
        "saved"
    );
    port.app.dispatch(AppAction::RevokeDevice(id));
    assert!(port.remote.state().devices.is_empty(), "saved");
    let (status, reply) = get(&mut port, "/api/v1/spaces", Some(&token));
    assert_eq!((status, code(&reply)), (401, "unauthorized"), "{reply}");
}

/// Nothing a device sends changes the host in this milestone, and what
/// is not on the table is refused by name, before translation.
#[test]
fn the_allowlist_refuses_everything_else_with_no_side_effect() {
    let mut port = port();
    let (_, project, session) = seed(&mut port, "");
    let token = pair(&mut port, "ABCD2345", Scope::Control, "Pad");
    let t = Some(token.as_str());
    for (method, path) in [
        ("POST", format!("/api/v1/sessions/{session}/kill")),
        ("POST", format!("/api/v1/sessions/{session}/notes")),
        ("GET", format!("/api/v1/sessions/{session}/env")),
        ("GET", "/api/v1/env/resolve".into()),
        ("POST", "/api/v1/projects".into()),
        ("DELETE", format!("/api/v1/sessions/{session}")),
        ("GET", "/api/v2/projects".into()),
    ] {
        let (status, reply) = call(&mut port, method, &path, t, r#"{"text":"hi"}"#);
        assert_eq!(
            (status, code(&reply)),
            (404, "not_found"),
            "{method} {path}: {reply}"
        );
    }
    // A file no set pins is refused by name, with nothing read.
    let (status, reply) = get(
        &mut port,
        &format!("/api/v1/projects/{project}/files/README.md"),
        t,
    );
    assert_eq!((status, code(&reply)), (404, "not_pinned"), "{reply}");
    assert!(
        port.host.state().written.is_empty(),
        "nothing reached a pane"
    );
    assert_eq!(
        port.app
            .core()
            .session(RecordId(uuid::Uuid::parse_str(&session).unwrap()))
            .unwrap()
            .notes,
        ""
    );
}

/// Make the fake host list `session` as a running Claude pane showing
/// `screen`.
fn running(port: &mut Port, session: &str, screen: &str) {
    let record = RecordId(uuid::Uuid::parse_str(session).unwrap());
    let pane = HostId(record.host_name());
    port.host.state().statuses.push(HostStatus {
        id: pane.clone(),
        liveness: HostLiveness::Running {
            pid: 7,
            command: "claude".into(),
        },
        cwd: None,
        last_activity: None,
        title: None,
    });
    port.host.state().snapshots.insert(pane, screen.into());
    port.app.poll_now();
}

/// A session's draft is its Prompt Box editor on the host (§4.5): a
/// device's speech events land in it as the host's recognizer's do,
/// utterance by utterance, the editor's tools work on it, and `send`
/// takes its text to the pane under the device's op and clears it.
/// Any paired device reads it; only the control scope changes it.
#[test]
#[allow(clippy::too_many_lines)]
fn a_draft_is_the_sessions_editor_fed_by_dictation_and_worked_by_its_tools() {
    let mut port = port();
    let (_, project, session) = seed(&mut port, "");
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let t = Some(token.as_str());
    let draft = format!("/api/v1/sessions/{session}/draft");
    let dictation = format!("/api/v1/sessions/{session}/dictation");

    let (status, reply) = get(&mut port, &draft, t);
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["draft"]["text"], "");
    assert_eq!(reply["body"]["draft"]["status"], "idle");
    assert_eq!(reply["body"]["draft"]["can_undo"], false);
    assert_eq!(reply["body"]["draft"]["ai"], false);
    let (status, reply) = call(
        &mut port,
        "POST",
        &dictation,
        t,
        r#"{"session":7,"events":[]}"#,
    );
    assert_eq!((status, code(&reply)), (403, "scope"), "{reply}");
    let (status, reply) = call(&mut port, "POST", &draft, t, r#"{"action":"undo"}"#);
    assert_eq!((status, code(&reply)), (403, "scope"), "{reply}");

    let id = port.app.core().devices()[0].id;
    port.app.dispatch(AppAction::SetDeviceScope {
        device: id,
        scope: Scope::Control,
    });
    let s = subscribe(&mut port, &token);
    let _hello = next_frame(&mut port, &s, 5);

    // An utterance revises in place and commits on its final; the
    // next one goes after it, spaced by the editor.
    let (status, reply) = call(
        &mut port,
        "POST",
        &dictation,
        t,
        r#"{"session":7,"events":[
            {"sequence":1,"kind":"started"},
            {"sequence":2,"kind":"voice_started","utterance":1},
            {"sequence":3,"kind":"partial","utterance":1,"revision":1,"text":"fix the"},
            {"sequence":4,"kind":"partial","utterance":1,"revision":2,"text":"fix the test"},
            {"sequence":5,"kind":"final","utterance":1,"text":"Fix the test."}]}"#,
    );
    assert_eq!(status, 200, "{reply}");
    let view = &reply["body"]["draft"];
    assert_eq!(view["text"], "Fix the test.");
    assert_eq!(view["status"], "listening");
    assert_eq!(view["live"], serde_json::Value::Null);
    assert_eq!(view["can_undo"], true);
    let (_, reply) = call(
        &mut port,
        "POST",
        &dictation,
        t,
        r#"{"session":7,"events":[
            {"sequence":6,"kind":"voice_started","utterance":2},
            {"sequence":7,"kind":"partial","utterance":2,"revision":1,"text":"and run"}]}"#,
    );
    let view = &reply["body"]["draft"];
    assert_eq!(view["text"], "Fix the test. and run");
    assert_eq!(view["live"], serde_json::json!([13, 21]));
    let (_, reply) = call(
        &mut port,
        "POST",
        &dictation,
        t,
        r#"{"session":7,"events":[
            {"sequence":8,"kind":"final","utterance":2,"text":"and run it"},
            {"sequence":9,"kind":"stopped"}]}"#,
    );
    let view = &reply["body"]["draft"];
    assert_eq!(view["text"], "Fix the test. and run it");
    assert_eq!(view["status"], "idle");
    let seen = resources(&events(&mut port, &s));
    assert!(
        seen.contains(&("changed".into(), "draft".into(), session.clone())),
        "{seen:?}"
    );

    // The editor's tools: undo takes the last utterance back, replace
    // is what a device typed, the AI pass needs a key.
    let (_, reply) = call(&mut port, "POST", &draft, t, r#"{"action":"undo"}"#);
    assert_eq!(reply["body"]["draft"]["text"], "Fix the test.");
    assert_eq!(reply["body"]["draft"]["can_redo"], true);
    let (_, reply) = call(
        &mut port,
        "POST",
        &draft,
        t,
        r#"{"action":"replace","text":"ship it"}"#,
    );
    assert_eq!(reply["body"]["draft"]["text"], "ship it");
    let (status, reply) = call(&mut port, "POST", &draft, t, r#"{"action":"clean_up"}"#);
    assert_eq!((status, code(&reply)), (400, "failed"), "{reply}");
    let (status, reply) = call(&mut port, "POST", &draft, t, r#"{"action":"shout"}"#);
    assert_eq!((status, code(&reply)), (400, "bad_request"), "{reply}");
    let (status, reply) = call(&mut port, "POST", &draft, t, r#"{"action":"send"}"#);
    assert_eq!((status, code(&reply)), (400, "bad_request"), "{reply}");

    // Send goes to the pane under the op, once, and clears the draft;
    // a session that is not running refuses and keeps it.
    let (status, reply) = call(
        &mut port,
        "POST",
        &draft,
        t,
        r#"{"action":"send","op":"d1"}"#,
    );
    assert_eq!((status, code(&reply)), (400, "failed"), "{reply}");
    assert!(port.host.state().written.is_empty());
    let (_, reply) = get(&mut port, &draft, t);
    assert_eq!(reply["body"]["draft"]["text"], "ship it");
    running(&mut port, &session, "");
    let (status, reply) = call(
        &mut port,
        "POST",
        &draft,
        t,
        r#"{"action":"send","op":"d2"}"#,
    );
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["draft"]["text"], "");
    assert_eq!(port.host.state().written.len(), 1);
    let (status, _) = call(
        &mut port,
        "POST",
        &draft,
        t,
        r#"{"action":"send","op":"d2"}"#,
    );
    assert_eq!(status, 200);
    assert_eq!(port.host.state().written.len(), 1, "the op is the key");

    // Only an agent session has a prompt.
    let (_, shell) = command_with_run(&mut port, &project, "echo");
    let (status, reply) = get(&mut port, &format!("/api/v1/sessions/{}/draft", shell.0), t);
    assert_eq!((status, code(&reply)), (400, "no_draft"), "{reply}");
}

/// A project with a root of its own on disk, for the file reads.
fn project_with_root(port: &mut Port, space: &str, root: &std::path::Path) -> String {
    let reply = control(
        port,
        Request::new(
            "pj2",
            Body::ProjectAdd {
                space: space.to_owned(),
                name: "#2".into(),
                root: root.to_path_buf(),
            },
        ),
    );
    made_id(&reply, RecordKind::Project)
}

/// A control device opens a session in a project: an agent with its
/// first prompt on its command line, or a shell, in the root or a
/// directory under it; the op is the key, so a repeat opens nothing
/// more. A read device, a directory outside the project or missing,
/// another kind, no name and an unknown project are refused by name.
#[test]
fn a_control_device_opens_a_session_in_a_project_and_nowhere_else() {
    use switchboard::core::{AgentKind, SessionKind as Kind};
    let mut port = port();
    let (space, _, _) = seed(&mut port, "");
    let root = tempfile::Builder::new()
        .prefix("sbnew")
        .tempdir_in("/tmp")
        .unwrap();
    std::fs::create_dir(root.path().join("sub")).unwrap();
    let project = project_with_root(&mut port, &space, root.path());
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let t = Some(token.as_str());
    let path = format!("/api/v1/projects/{project}/sessions");
    let agent = r#"{"op":"ns1","name":"explorer","kind":"claude","prompt":"Look around."}"#;
    let (status, reply) = call(&mut port, "POST", &path, t, agent);
    assert_eq!((status, code(&reply)), (403, "scope"), "{reply}");
    let id = port.app.core().devices()[0].id;
    port.app.dispatch(AppAction::SetDeviceScope {
        device: id,
        scope: Scope::Control,
    });

    let spawned = port.host.state().spawned.len();
    let (status, reply) = call(&mut port, "POST", &path, t, agent);
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["made"][0]["kind"], "session");
    let made = |reply: &serde_json::Value| {
        let id = reply["body"]["made"][0]["id"].as_str().unwrap();
        RecordId(uuid::Uuid::parse_str(id).unwrap())
    };
    let record = port.app.core().session(made(&reply)).unwrap();
    assert_eq!(record.name, "explorer");
    assert_eq!(record.kind, Kind::Agent(AgentKind::ClaudeCode));
    assert_eq!(record.cwd, root.path(), "the project's root by default");
    let spawns = port.host.state().spawned.clone();
    assert_eq!(spawns.len(), spawned + 1, "launched");
    let argv = spawns.last().unwrap().command.clone().unwrap_or_default();
    assert_eq!(argv.last().map(String::as_str), Some("Look around."));
    let (status, _) = call(&mut port, "POST", &path, t, agent);
    assert_eq!(status, 200);
    assert_eq!(
        port.host.state().spawned.len(),
        spawned + 1,
        "a repeat of the op opens nothing more"
    );

    let shell = r#"{"op":"ns2","name":"sub shell","kind":"shell","cwd":"sub"}"#;
    let (status, reply) = call(&mut port, "POST", &path, t, shell);
    assert_eq!(status, 200, "{reply}");
    let record = port.app.core().session(made(&reply)).unwrap();
    assert_eq!(record.kind, Kind::Shell);
    assert_eq!(record.cwd, root.path().join("sub"));
    assert_eq!(port.host.state().spawned.len(), spawned + 2);

    for body in [
        r#"{"op":"ns3","name":"x","kind":"shell","cwd":"../"}"#,
        r#"{"op":"ns4","name":"x","kind":"shell","cwd":"/etc"}"#,
        r#"{"op":"ns5","name":"x","kind":"shell","cwd":"nowhere"}"#,
        r#"{"op":"ns6","name":"x","kind":"service"}"#,
        r#"{"op":"ns7","kind":"shell"}"#,
        r#"{"name":"x","kind":"shell"}"#,
        "nope",
    ] {
        let (status, reply) = call(&mut port, "POST", &path, t, body);
        assert_eq!(
            (status, code(&reply)),
            (400, "bad_request"),
            "{body}: {reply}"
        );
    }
    let (status, reply) = call(
        &mut port,
        "POST",
        "/api/v1/projects/00000000-0000-0000-0000-000000000000/sessions",
        t,
        shell,
    );
    assert_eq!((status, code(&reply)), (404, "not_found"), "{reply}");
    assert_eq!(
        port.host.state().spawned.len(),
        spawned + 2,
        "nothing else ran"
    );
}

/// A working set in `space` pinning `paths` of `project`.
fn pin_files(port: &mut Port, space: &str, project: &str, paths: &[&str]) -> String {
    let reply = control(
        port,
        Request::new(
            "st",
            Body::SetNew {
                space: space.to_owned(),
                name: "Desk".into(),
            },
        ),
    );
    let set = made_id(&reply, RecordKind::Set);
    let items = paths
        .iter()
        .enumerate()
        .map(|(i, path)| Pin {
            target: PinTarget::File {
                project: project.to_owned(),
                path: PathBuf::from(path),
            },
            rect: Rect {
                x: 0,
                y: u32::try_from(i).unwrap() * 2,
                w: 2,
                h: 2,
            },
        })
        .collect();
    let reply = control(
        port,
        Request::new(
            "ss",
            Body::SetSync {
                set: set.clone(),
                items,
            },
        ),
    );
    assert!(
        !matches!(reply, Reply::Failed { .. }),
        "set.sync: {reply:?}"
    );
    set
}

/// `file.read` (§4.3): a file some working set pins is read whole,
/// masked of the project's secrets, with its size and modification
/// time; a dotenv file is refused though pinned, a file over the
/// limit is refused with its size, and a watched file moves on the
/// poll when it is edited. The policy is resolved on the worker.
#[test]
#[allow(clippy::too_many_lines)]
fn a_pinned_file_is_read_masked_and_watched_and_only_when_pinned() {
    let mut port = port();
    let (space, _, _) = seed(&mut port, "");
    let dir = tempfile::Builder::new()
        .prefix("sbfile")
        .tempdir_in("/tmp")
        .unwrap();
    std::fs::create_dir(dir.path().join("docs")).unwrap();
    let plan = "# Plan\nuse tok-5e3cr3t here\n";
    std::fs::write(dir.path().join("docs/plan.md"), plan).unwrap();
    std::fs::write(dir.path().join(".env"), "API_TOKEN=tok-5e3cr3t\n").unwrap();
    std::fs::write(dir.path().join("docs/big.txt"), "x".repeat(800 * 1024)).unwrap();
    let project = project_with_root(&mut port, &space, dir.path());
    let pid = switchboard::core::ProjectId(uuid::Uuid::parse_str(&project).unwrap());
    port.app.dispatch(AppAction::SetProjectEnv(
        pid,
        switchboard::core::ProjectEnv {
            vars: vec![switchboard::core::EnvVar {
                name: "API_TOKEN".into(),
                value: String::new(),
                secret: true,
            }],
            ..Default::default()
        },
    ));
    port.secrets.state().insert(
        switchboard::core::SecretScope::Project(pid).account("API_TOKEN"),
        "tok-5e3cr3t".into(),
    );
    port.app.set_watch_poll(Duration::from_millis(50));
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let t = Some(token.as_str());
    let path = format!("/api/v1/projects/{project}/files/docs/plan.md");

    let (status, reply) = get(&mut port, &path, t);
    assert_eq!((status, code(&reply)), (404, "not_pinned"), "{reply}");
    pin_files(
        &mut port,
        &space,
        &project,
        &["docs/plan.md", ".env", "docs/big.txt", "docs/missing.md"],
    );
    let (status, reply) = get(&mut port, &path, t);
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["text"], "# Plan\nuse <API_TOKEN> here\n");
    assert_eq!(reply["body"]["path"], "docs/plan.md");
    assert_eq!(reply["body"]["size"], plan.len());
    assert!(reply["body"]["modified_ms"].is_u64(), "{reply}");
    assert!(!reply.to_string().contains("tok-5e3cr3t"), "{reply}");
    let first = reply["revision"].as_u64().unwrap();
    let (_, again) = get(&mut port, &path, t);
    assert_eq!(
        again["revision"], first,
        "an unchanged file keeps its revision"
    );

    let (status, reply) = get(
        &mut port,
        &format!("/api/v1/projects/{project}/files/.env"),
        t,
    );
    assert_eq!((status, code(&reply)), (400, "failed"), "{reply}");
    assert!(!reply.to_string().contains("tok-5e3cr3t"), "{reply}");
    let (status, reply) = get(
        &mut port,
        &format!("/api/v1/projects/{project}/files/docs/big.txt"),
        t,
    );
    assert_eq!((status, code(&reply)), (413, "too_large"), "{reply}");
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("819200"),
        "{reply}"
    );
    let (status, reply) = get(
        &mut port,
        &format!("/api/v1/projects/{project}/files/docs/missing.md"),
        t,
    );
    assert_eq!((status, code(&reply)), (400, "failed"), "{reply}");
    let (status, reply) = get(
        &mut port,
        &format!("/api/v1/projects/{project}/files/docs/../plan.md"),
        t,
    );
    assert_eq!((status, code(&reply)), (400, "bad_request"), "{reply}");
    let (status, reply) = get(&mut port, &format!("/api/v1/projects/{project}/files"), t);
    assert_eq!(status, 404, "{reply}");

    // Watched: an edit moves it on the poll, with a new revision.
    let sub = subscribe(&mut port, &token);
    let id = format!("{project}/docs/plan.md");
    sub.send("watch", "file", &id);
    let seen = resources(&events(&mut port, &sub));
    assert!(
        seen.contains(&("watching".into(), "file".into(), id.clone())),
        "{seen:?}"
    );
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(dir.path().join("docs/plan.md"), "# Plan\nrevised\n").unwrap();
    let got = events(&mut port, &sub);
    let seen = resources(&got);
    assert!(
        seen.contains(&("changed".into(), "file".into(), id.clone())),
        "{seen:?}"
    );
    let (_, reply) = get(&mut port, &path, t);
    assert_eq!(reply["body"]["text"], "# Plan\nrevised\n");
    assert!(reply["revision"].as_u64().unwrap() > first);

    // A secret store that fails refuses rather than answers unmasked:
    // the policy is read again after any save, and the store is asked.
    port.secrets.set_failing(true);
    port.app.dispatch(AppAction::SetProjectEnv(
        pid,
        switchboard::core::ProjectEnv {
            vars: vec![switchboard::core::EnvVar {
                name: "API_TOKEN".into(),
                value: String::new(),
                secret: true,
            }],
            ..Default::default()
        },
    ));
    let (status, reply) = get(&mut port, &path, t);
    assert_eq!((status, code(&reply)), (503, "policy"), "{reply}");
}

/// The control scope (§4.5): `send`, `resume` and `trust` go through
/// the control socket's `serve`, so the device's op is the idempotency
/// key and a repeat is answered from the log without reaching the pane
/// again; a read device is refused by name and nothing runs.
#[test]
fn the_control_scope_sends_with_the_op_as_the_key_and_a_read_device_is_refused() {
    let mut port = port();
    let (_, _, session) = seed(&mut port, "");
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let t = Some(token.as_str());
    let send = format!("/api/v1/sessions/{session}/send");
    let hello = r#"{"op":"tx1","text":"hello"}"#;
    let (status, reply) = call(&mut port, "POST", &send, t, hello);
    assert_eq!((status, code(&reply)), (403, "scope"), "{reply}");
    assert!(port.host.state().written.is_empty());

    let id = port.app.core().devices()[0].id;
    port.app.dispatch(AppAction::SetDeviceScope {
        device: id,
        scope: Scope::Control,
    });
    // A resume on the not-running session launches it once; the op is
    // the key, so a repeat does not spend a second turn.
    let spawned = port.host.state().spawned.len();
    let resume = format!("/api/v1/sessions/{session}/resume");
    let (status, reply) = call(&mut port, "POST", &resume, t, r#"{"op":"rs0"}"#);
    assert_eq!(status, 200, "{reply}");
    assert_eq!(port.host.state().spawned.len(), spawned + 1, "launched");
    let (status, _) = call(&mut port, "POST", &resume, t, r#"{"op":"rs0"}"#);
    assert_eq!(status, 200);
    assert_eq!(port.host.state().spawned.len(), spawned + 1, "once");

    running(&mut port, &session, "\u{276f} ");
    let (status, reply) = call(&mut port, "POST", &send, t, hello);
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["made"], serde_json::json!([]));
    let written = port.host.state().written.clone();
    assert_eq!(written.len(), 1, "{written:?}");
    assert_eq!(written[0].1, "hello");
    let (status, again) = call(&mut port, "POST", &send, t, hello);
    assert_eq!(status, 200, "{again}");
    assert_eq!(
        port.host.state().written.len(),
        1,
        "a repeat never types twice"
    );
    let (_, done) = get(&mut port, "/api/v1/ops/tx1", t);
    assert_eq!(done["body"]["status"]["status"], "done");

    for body in [
        r#"{"text":"hi"}"#,
        r#"{"op":"tx3"}"#,
        r#"{"op":"","text":"x"}"#,
        "nope",
    ] {
        let (status, reply) = call(&mut port, "POST", &send, t, body);
        assert_eq!(
            (status, code(&reply)),
            (400, "bad_request"),
            "{body}: {reply}"
        );
    }
    assert_eq!(port.host.state().written.len(), 1);

    // A resume while running is accepted and launches nothing; trust
    // with no question asked is refused by name; an unknown session is
    // not found.
    let (status, reply) = call(&mut port, "POST", &resume, t, r#"{"op":"rs1"}"#);
    assert_eq!(status, 200, "{reply}");
    assert_eq!(port.host.state().spawned.len(), spawned + 1);
    let trust = format!("/api/v1/sessions/{session}/trust");
    let (status, reply) = call(&mut port, "POST", &trust, t, r#"{"op":"tr1"}"#);
    assert_eq!((status, code(&reply)), (400, "failed"), "{reply}");
    assert_eq!(port.host.state().written.len(), 1);
    let (status, reply) = call(
        &mut port,
        "POST",
        "/api/v1/sessions/00000000-0000-0000-0000-000000000000/send",
        t,
        r#"{"op":"tx9","text":"x"}"#,
    );
    assert_eq!((status, code(&reply)), (400, "failed"), "{reply}");

    // Lowered again, the next command is refused.
    port.app.dispatch(AppAction::SetDeviceScope {
        device: id,
        scope: Scope::Read,
    });
    let (status, reply) = call(
        &mut port,
        "POST",
        &send,
        t,
        r#"{"op":"tx4","text":"hello"}"#,
    );
    assert_eq!((status, code(&reply)), (403, "scope"), "{reply}");
}

/// Puts a standing ask on `id` as `session.ask` would; the test holds
/// no launch token to call it with.
fn seed_ask(port: &mut Port, id: RecordId) {
    let ask = Ask::note("merge now?", SystemTime::now());
    port.app.core_mut_for_seeding().seed_asking(id, Some(ask));
}

/// One hook event for `id`, as the helper would log it.
fn hook(port: &mut Port, id: RecordId, kind: EventKind) {
    port.app.dispatch(AppAction::Events(vec![SessionEvent {
        at: SystemTime::now(),
        seq: 0,
        record_id: Some(id),
        provider_session_id: None,
        cwd: None,
        transcript_path: None,
        kind,
    }]));
}

fn stopped() -> EventKind {
    EventKind::Stopped {
        last_message: None,
        pending: None,
    }
}

/// The owner's reply from a paired device, by `/send` or by the
/// draft's Send, answers a standing ask as one typed in the app does.
/// The same `session.send` on the control socket is a tool's, even
/// from the pane itself, and leaves the ask standing.
#[test]
fn a_devices_send_answers_the_ask_and_a_socket_send_does_not() {
    let mut port = port();
    let (_, _, session) = seed(&mut port, "");
    let id = RecordId(uuid::Uuid::parse_str(&session).unwrap());
    let token = pair(&mut port, "ABCD2345", Scope::Control, "Vision Pro");
    let t = Some(token.as_str());
    running(&mut port, &session, "\u{276f} ");
    let asking = |port: &Port| port.app.core().session(id).unwrap().asking.is_some();

    seed_ask(&mut port, id);
    hook(&mut port, id, stopped());
    let send = format!("/api/v1/sessions/{session}/send");
    let body = r#"{"op":"tx1","text":"merge it"}"#;
    let (status, reply) = call(&mut port, "POST", &send, t, body);
    assert_eq!(status, 200, "{reply}");
    hook(&mut port, id, EventKind::PromptSubmitted);
    assert!(!asking(&port), "the device's send answered");
    let (status, _) = call(&mut port, "POST", &send, t, body);
    assert_eq!(status, 200);
    assert_eq!(
        port.host.state().written.len(),
        1,
        "a repeat never types twice"
    );

    seed_ask(&mut port, id);
    hook(&mut port, id, stopped());
    let draft = format!("/api/v1/sessions/{session}/draft");
    call(
        &mut port,
        "POST",
        &draft,
        t,
        r#"{"action":"replace","text":"merge it"}"#,
    );
    let (status, reply) = call(
        &mut port,
        "POST",
        &draft,
        t,
        r#"{"action":"send","op":"d1"}"#,
    );
    assert_eq!(status, 200, "{reply}");
    hook(&mut port, id, EventKind::PromptSubmitted);
    assert!(!asking(&port), "the draft's Send answered");
    let (status, _) = call(
        &mut port,
        "POST",
        &draft,
        t,
        r#"{"action":"send","op":"d1"}"#,
    );
    assert_eq!(status, 200);
    assert_eq!(
        port.host.state().written.len(),
        2,
        "a repeat never types twice"
    );

    seed_ask(&mut port, id);
    hook(&mut port, id, stopped());
    let reply = control(
        &mut port,
        Request::new(
            "s1",
            Body::SessionSend {
                session: session.clone(),
                text: "commit your work".into(),
            },
        ),
    );
    assert!(!matches!(reply, Reply::Failed { .. }), "{reply:?}");
    assert_eq!(port.host.state().written.len(), 3);
    hook(&mut port, id, EventKind::PromptSubmitted);
    assert!(asking(&port), "a tool's send is not the owner's answer");
}

/// A pane at Claude Code's folder question: the session view says so,
/// `trust` answers it, and the answer reaches the pane once.
#[test]
fn trust_answers_the_folder_question_the_session_view_carries() {
    let mut port = port();
    let (_, _, session) = seed(&mut port, "");
    let token = pair(&mut port, "ABCD2345", Scope::Control, "Vision Pro");
    let t = Some(token.as_str());
    running(
        &mut port,
        &session,
        "Quick safety check: Is this a project you created or one you trust?\n\
         \u{276f} No, exit\n  Yes, I trust this folder\n",
    );
    let (_, view) = get(&mut port, &format!("/api/v1/sessions/{session}"), t);
    assert_eq!(view["body"]["session"]["trust_question"], true, "{view}");
    let trust = format!("/api/v1/sessions/{session}/trust");
    let (status, reply) = call(&mut port, "POST", &trust, t, r#"{"op":"tr1"}"#);
    assert_eq!(status, 200, "{reply}");
    let keys = port.host.state().keys.len();
    assert!(keys >= 1, "the answer reached the pane");
    let (status, _) = call(&mut port, "POST", &trust, t, r#"{"op":"tr1"}"#);
    assert_eq!(status, 200);
    assert_eq!(port.host.state().keys.len(), keys, "once");
}

#[test]
fn a_screen_is_read_with_the_projects_secret_values_named() {
    let mut port = port();
    let (_, project, session) = seed(&mut port, "");
    let pid = switchboard::core::ProjectId(uuid::Uuid::parse_str(&project).unwrap());
    port.app.dispatch(AppAction::SetProjectEnv(
        pid,
        switchboard::core::ProjectEnv {
            vars: vec![switchboard::core::EnvVar {
                name: "API_TOKEN".into(),
                value: String::new(),
                secret: true,
            }],
            ..Default::default()
        },
    ));
    port.secrets.state().insert(
        switchboard::core::SecretScope::Project(pid).account("API_TOKEN"),
        "tok-5e3cr3t".into(),
    );
    let record = RecordId(uuid::Uuid::parse_str(&session).unwrap());
    let pane = HostId(record.host_name());
    port.host.state().statuses.push(HostStatus {
        id: pane.clone(),
        liveness: HostLiveness::Running {
            pid: 7,
            command: "claude".into(),
        },
        cwd: None,
        last_activity: None,
        title: None,
    });
    port.host.state().snapshots.insert(
        pane,
        "one\ntwo\n$ env | grep TOKEN\nAPI_TOKEN=tok-5e3cr3t\n\n\n".into(),
    );
    port.app.poll_now();
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Pad");
    let (status, reply) = get(
        &mut port,
        &format!("/api/v1/sessions/{session}/screen?lines=2"),
        Some(&token),
    );
    assert_eq!(status, 200, "{reply}");
    assert_eq!(
        reply["body"]["text"],
        "$ env | grep TOKEN\nAPI_TOKEN=<API_TOKEN>"
    );
    let (status, reply) = get(
        &mut port,
        &format!("/api/v1/sessions/{session}/screen"),
        Some(&token),
    );
    assert_eq!(status, 200);
    let text = reply["body"]["text"].as_str().unwrap();
    assert!(text.starts_with("one\ntwo"), "{text}");
    assert!(!reply.to_string().contains("tok-5e3cr3t"), "{reply}");
}

/// Every string in a collection is bounded and the cut ones are named;
/// a session read alone carries its notes whole.
#[test]
fn strings_are_bounded_in_a_collection_and_whole_alone() {
    let mut port = port();
    let notes = "n".repeat(10_000);
    let (_, project, session) = seed(&mut port, &notes);
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Pad");
    let (status, reply) = get(
        &mut port,
        &format!("/api/v1/projects/{project}/sessions"),
        Some(&token),
    );
    assert_eq!(status, 200);
    let view = &reply["body"]["sessions"][0];
    assert_eq!(view["notes"].as_str().unwrap().len(), 4096);
    assert_eq!(view["truncated"], serde_json::json!(["notes"]));
    let (status, reply) = get(
        &mut port,
        &format!("/api/v1/sessions/{session}"),
        Some(&token),
    );
    assert_eq!(status, 200);
    let view = &reply["body"]["session"];
    assert_eq!(view["notes"].as_str().unwrap().len(), 10_000);
    assert!(view.get("truncated").is_none(), "{view}");
}

#[test]
fn a_wrong_pin_fails_at_the_handshake_before_any_request() {
    let mut port = port();
    let other = pinned_client(&"00".repeat(32)).unwrap();
    let result = http(port.addr, &other, "GET", "/api/v1/info", None, "");
    assert!(result.is_err(), "{result:?}");
    // The right pin is still served.
    let (status, _) = get(&mut port, "/api/v1/info", None);
    assert_eq!(status, 200);
}

#[test]
fn pair_is_rate_limited_per_source() {
    let mut port = port();
    let mut statuses = Vec::new();
    for _ in 0..7 {
        let (status, _) = call(&mut port, "POST", "/api/v1/pair", None, "{}");
        statuses.push(status);
    }
    assert_eq!(statuses, vec![403, 403, 403, 403, 403, 403, 429]);
    let (status, reply) = get(&mut port, "/api/v1/info", None);
    assert_eq!(status, 200, "info has its own limit: {reply}");
}

/// A certificate whose key is gone is remade over a new key under the
/// same server id, every pairing is dropped, and the owner is told; a
/// key that is present keeps the identity across restarts.
#[test]
fn a_missing_key_at_start_remakes_the_identity_and_clears_the_devices() {
    let (old, _key_never_stored) = remote::make_identity(None, "Old", Listen::default()).unwrap();
    let store = FakeRemoteStore::default();
    {
        let mut state = store.state();
        state.identity = Some(old.clone());
        state
            .devices
            .push(switchboard::ports::remote::PairedDevice {
                id: uuid::Uuid::new_v4(),
                name: "Stale".into(),
                scope: Scope::Read,
                token_sha256: "ab".repeat(32),
                created: std::time::SystemTime::UNIX_EPOCH,
                last_seen: None,
            });
    }
    let secrets = FakeSecrets::default();
    let mut port = port_with(FakeHost::default(), secrets.clone(), store.clone());
    let fresh = store.state().identity.clone().unwrap();
    assert_eq!(fresh.server_id, old.server_id, "the same installation");
    // What a browser's page compares after a 401: the id it stored
    // with another key.
    let (_, info) = get(&mut port, "/api/v1/info", None);
    assert_eq!(info["body"]["server_id"], old.server_id.to_string());
    assert_ne!(info["body"]["spki_sha256"], old.spki_sha256);
    assert_eq!(fresh.server_name, "Old");
    assert_ne!(fresh.spki_sha256, old.spki_sha256, "a new key");
    assert!(store.state().devices.is_empty());
    assert!(port.app.core().identity_rotated());
    assert!(port.app.core().devices().is_empty());
    assert!(
        secrets.state().contains_key(&fresh.key_account()),
        "the new key is stored"
    );
    drop(port);

    // The key is there now: a restart keeps everything.
    let again = port_with(FakeHost::default(), secrets, store.clone());
    assert_eq!(
        store.state().identity.as_ref().unwrap().spki_sha256,
        fresh.spki_sha256
    );
    assert!(!again.app.core().identity_rotated());
}

#[test]
fn two_instances_have_distinct_identities_and_a_read_only_one_does_not_listen() {
    let mut a = port();
    let mut b = port();
    let (_, ia) = get(&mut a, "/api/v1/info", None);
    let (_, ib) = get(&mut b, "/api/v1/info", None);
    assert_ne!(ia["body"]["server_id"], ib["body"]["server_id"]);
    assert_ne!(ia["body"]["spki_sha256"], ib["body"]["spki_sha256"]);
    assert_ne!(ia["epoch"], ib["epoch"]);
    // B's pin does not open A.
    let result = http(a.addr, &b.client, "GET", "/api/v1/info", None, "");
    assert!(result.is_err());

    let services = Services {
        store: Box::new(MemoryStore {
            lock_result: Some(false),
            ..MemoryStore::default()
        }),
        ..fakes::services()
    };
    let mut app = SwitchboardApp::with_services(services);
    // The editors a draft is read from save through a fake dialog.
    app.ui_state.prompt_boxes.native = false;
    app.start();
    app.set_remote_port_override(Some(0));
    assert!(app.listen_remote(|| {}).is_err());
    assert!(app.remote_addr().is_none());
}

/// A changed address rebinds the port; a changed name does not.
#[test]
fn a_change_of_address_rebinds_the_port() {
    let mut port = port();
    let before = port.app.remote_addr().unwrap();
    port.app.dispatch(AppAction::SetServerName("Desk".into()));
    assert_eq!(
        port.app.remote_addr(),
        Some(before),
        "a rename keeps the socket"
    );
    assert_eq!(
        port.remote.state().identity.as_ref().unwrap().server_name,
        "Desk"
    );
    port.app.dispatch(AppAction::SetRemoteListen {
        lan: false,
        port: 43199,
    });
    let after = port.app.remote_addr().unwrap();
    assert_ne!(after, before, "rebound");
    assert!(after.ip().is_loopback());
    port.addr = after;
    let (status, _) = get(&mut port, "/api/v1/info", None);
    assert_eq!(status, 200, "served on the new socket");
}

/// Bonjour follows the listen preference: nothing on loopback, the
/// record while the port is on the local network, under the current
/// name, and withdrawn when the owner turns the network off.
#[test]
fn bonjour_advertises_only_while_the_port_is_on_the_lan() {
    let mut port = port();
    assert_eq!(
        port.advertiser.current(),
        None,
        "loopback is not advertised"
    );
    // The core refuses port 0; the test's override still picks a free one.
    port.app.dispatch(AppAction::SetRemoteListen {
        lan: true,
        port: 43187,
    });
    let addr = port.app.remote_addr().unwrap();
    assert!(addr.ip().is_unspecified(), "bound to every interface");
    let ad = port.advertiser.current().expect("advertised on the LAN");
    let identity = port.remote.state().identity.clone().unwrap();
    assert_eq!(ad.port, addr.port());
    assert_eq!(ad.server_id, identity.server_id);
    assert_eq!(ad.server_name, identity.server_name);
    assert!(
        ad.instance.starts_with(&identity.server_name),
        "{:?} carries the display name",
        ad.instance
    );
    assert!(
        ad.instance
            .contains(&identity.server_id.simple().to_string()[..6]),
        "{:?} carries a short id suffix",
        ad.instance
    );
    let txt = ad.txt();
    assert!(txt.contains(&("v".to_owned(), "1".to_owned())));
    assert!(txt.contains(&("id".to_owned(), identity.server_id.to_string())));
    assert!(
        !txt.iter().any(|(_, v)| v == &identity.spki_sha256),
        "no key material in the TXT records"
    );

    port.app.dispatch(AppAction::SetServerName("Desk".into()));
    let renamed = port.advertiser.current().expect("still advertised");
    assert_eq!(renamed.server_name, "Desk");
    assert!(renamed.instance.starts_with("Desk ("));
    assert_eq!(
        port.app.remote_addr(),
        Some(addr),
        "a rename re-advertises without rebinding"
    );

    port.app.dispatch(AppAction::SetRemoteListen {
        lan: false,
        port: 43187,
    });
    assert_eq!(port.advertiser.current(), None, "withdrawn off the LAN");
    assert!(port.app.remote_addr().unwrap().ip().is_loopback());
}

/// The Swift client's fixtures: one reply of each shape the port sends,
/// as `client/Fixtures/*.json`. The committed files must still decode
/// as the wire types here (the `reply` tag put back), and the Swift
/// tests decode the same files, so a shape change on either side fails
/// a test. `SWITCHBOARD_WRITE_FIXTURES=1` rewrites them from a seeded
/// port.
/// Every fixture from a seeded port, written to `dir`.
#[allow(clippy::too_many_lines)] // one entry per fixture
fn write_fixtures(dir: &std::path::Path) {
    let mut port = port_dispatching(tickets_status());
    let mut scripted = conversation(2);
    scripted.turns[1].final_text = "pong".into();
    port.transcripts.set(Some(scripted));
    let (space, project, session) = seed(&mut port, "ticket 1\nnotes with two lines");
    launched(&mut port, &session);
    let (command, _) = command_with_run(&mut port, &project, "$ make\nbuilding\n");
    let record = RecordId(uuid::Uuid::parse_str(&session).unwrap());
    let pane = HostId(record.host_name());
    port.host.state().statuses.push(HostStatus {
        id: pane.clone(),
        liveness: HostLiveness::Running {
            pid: 7,
            command: "claude".into(),
        },
        cwd: None,
        last_activity: None,
        title: None,
    });
    port.host
        .state()
        .snapshots
        .insert(pane, "$ claude\n> fix the test\n\n".into());
    port.app.poll_now();
    control(
        &mut port,
        Request::new(
            "st",
            Body::SetNew {
                space: space.clone(),
                name: "Desk".into(),
            },
        ),
    );
    // A fixed name: the fake's default is this machine's hostname.
    port.app
        .dispatch(AppAction::SetServerName("Desk Mac".into()));
    let identity = port.remote.state().identity.clone().unwrap();
    let (_, info) = get(&mut port, "/api/v1/info", None);
    port.app.dispatch(AppAction::OpenPairing {
        code: "ABCD2345".into(),
        scope: Scope::Read,
    });
    let body = serde_json::json!({
        "code": "ABCD2345", "device_name": "Vision Pro", "spki_sha256": identity.spki_sha256
    });
    let (_, paired) = call(&mut port, "POST", "/api/v1/pair", None, &body.to_string());
    let token = paired["body"]["token"].as_str().unwrap().to_owned();
    let t = Some(token.as_str());
    let mut files = vec![
        ("info", info),
        ("pair", paired),
        ("projects", get(&mut port, "/api/v1/projects", t).1),
        ("spaces", get(&mut port, "/api/v1/spaces", t).1),
        (
            "sets",
            get(&mut port, &format!("/api/v1/spaces/{space}/sets"), t).1,
        ),
        (
            "sessions",
            get(
                &mut port,
                &format!("/api/v1/projects/{project}/sessions"),
                t,
            )
            .1,
        ),
        (
            "session",
            get(&mut port, &format!("/api/v1/sessions/{session}"), t).1,
        ),
        ("waiting", get(&mut port, "/api/v1/waiting", t).1),
        (
            "screen",
            get(&mut port, &format!("/api/v1/sessions/{session}/screen"), t).1,
        ),
        ("error", get(&mut port, "/api/v1/projects", None).1),
        (
            "pin",
            serde_json::json!({
                "certificate": identity.certificate,
                "spki_sha256": identity.spki_sha256,
            }),
        ),
    ];
    // The command fixtures need the control scope.
    let device = port.app.core().devices()[0].id;
    port.app.dispatch(AppAction::SetDeviceScope {
        device,
        scope: Scope::Control,
    });
    files.extend(content_fixtures(&mut port, &session, &command, &token));
    files.push((
        "created",
        call(
            &mut port,
            "POST",
            &format!("/api/v1/projects/{project}/sessions"),
            t,
            r#"{"op":"fixture-new","name":"explorer","kind":"claude","prompt":"Look around."}"#,
        )
        .1,
    ));
    files.push(("file", file_fixture(&mut port, &space, &token)));
    for (name, value) in &mut files {
        if *name == "pair" {
            // A fixture never carries a real token.
            value["body"]["token"] = serde_json::json!("A".repeat(43));
        }
        let text = serde_json::to_string_pretty(value).unwrap();
        std::fs::write(dir.join(format!("{name}.json")), text + "\n").unwrap();
    }
}

/// The fixture ticket's notes, read by the key its listing gave.
fn ticket_file_fixture(port: &mut Port, t: Option<&str>) -> serde_json::Value {
    let (_, ticket) = get(port, "/api/v1/dispatch/tickets/t1", t);
    let key = ticket["body"]["attempts"][0]["files"][0]["key"]
        .as_str()
        .unwrap()
        .to_owned();
    get(port, &format!("/api/v1/dispatch/tickets/t1/files/{key}"), t).1
}

/// A pinned file of a project with a root on disk, read.
fn file_fixture(port: &mut Port, space: &str, token: &str) -> serde_json::Value {
    let root = tempfile::Builder::new()
        .prefix("sbfix")
        .tempdir_in("/tmp")
        .unwrap();
    std::fs::create_dir(root.path().join("docs")).unwrap();
    std::fs::write(
        root.path().join("docs/plan.md"),
        "# Plan\n\n1. Fix the test.\n",
    )
    .unwrap();
    let pinned = project_with_root(port, space, root.path());
    pin_files(port, space, &pinned, &["docs/plan.md"]);
    get(
        port,
        &format!("/api/v1/projects/{pinned}/files/docs/plan.md"),
        Some(token),
    )
    .1
}

/// The content replies: a page, the last response, the runs, a tail.
fn content_fixtures(
    port: &mut Port,
    session: &str,
    command: &str,
    token: &str,
) -> Vec<(&'static str, serde_json::Value)> {
    let t = Some(token);
    vec![
        (
            "conversation",
            get(
                port,
                &format!("/api/v1/sessions/{session}/conversation?limit=1"),
                t,
            )
            .1,
        ),
        (
            "turn",
            get(
                port,
                &format!("/api/v1/sessions/{session}/conversation/1"),
                t,
            )
            .1,
        ),
        (
            "last_response",
            get(
                port,
                &format!("/api/v1/sessions/{session}/last-response"),
                t,
            )
            .1,
        ),
        (
            "runs",
            get(port, &format!("/api/v1/sessions/{command}/runs"), t).1,
        ),
        (
            "run_output",
            get(
                port,
                &format!("/api/v1/sessions/{command}/runs/1/output"),
                t,
            )
            .1,
        ),
        (
            "command",
            call(
                port,
                "POST",
                &format!("/api/v1/sessions/{session}/send"),
                t,
                r#"{"op":"fixture-send","text":"ping"}"#,
            )
            .1,
        ),
        ("tickets", get(port, "/api/v1/dispatch/tickets", t).1),
        ("ticket", get(port, "/api/v1/dispatch/tickets/t1", t).1),
        (
            "decided",
            call(
                port,
                "POST",
                "/api/v1/dispatch/tickets/t1/decide",
                t,
                r#"{"decision":"d1","answer":"frontend"}"#,
            )
            .1,
        ),
        ("ticket_events", get(port, "/api/v1/dispatch/tickets/t1/events", t).1),
        ("ticket_file", ticket_file_fixture(port, t)),
        (
            "closed",
            call(port, "POST", "/api/v1/dispatch/tickets/t3/close", t, "{}").1,
        ),
        ("op_status", get(port, "/api/v1/ops/fixture-send", t).1),
        (
            "draft",
            call(
                port,
                "POST",
                &format!("/api/v1/sessions/{session}/dictation"),
                t,
                r#"{"session":7,"events":[{"sequence":1,"kind":"started"},{"sequence":2,"kind":"voice_started","utterance":1},{"sequence":3,"kind":"final","utterance":1,"text":"Fix the test."},{"sequence":4,"kind":"partial","utterance":2,"revision":1,"text":"and run"}]}"#,
            )
            .1,
        ),
    ]
}

#[test]
fn the_client_fixtures_decode_on_both_sides() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("client/Fixtures");
    if std::env::var_os("SWITCHBOARD_WRITE_FIXTURES").is_some() {
        write_fixtures(&dir);
    }
    let read = |name: &str| -> serde_json::Value {
        let path = dir.join(format!("{name}.json"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path:?}: {e}"))
    };
    for (name, tag) in [
        ("projects", "projects"),
        ("spaces", "spaces"),
        ("sets", "sets"),
        ("sessions", "sessions"),
        ("session", "session"),
        ("waiting", "waiting"),
        ("screen", "screen"),
    ] {
        let fixture = read(name);
        assert!(fixture["epoch"].is_string(), "{name}: epoch");
        assert!(fixture["revision"].is_u64(), "{name}: revision");
        let mut body = fixture["body"].clone();
        body["reply"] = serde_json::json!(tag);
        let reply: Reply = serde_json::from_value(body)
            .unwrap_or_else(|e| panic!("{name}.json no longer decodes as the wire type: {e}"));
        assert!(
            !matches!(reply, Reply::Failed { .. }),
            "{name}: a failure fixture"
        );
    }
    // The content shapes have no wire type on the control side; they
    // are checked by field.
    let page = read("conversation");
    assert_eq!(page["body"]["next"], 2);
    assert_eq!(page["body"]["turns"][0]["n"], 1);
    let last = read("last_response");
    assert_eq!(last["body"]["n"], 2);
    assert!(last["body"]["text"].is_string());
    let runs = read("runs");
    assert_eq!(runs["body"]["runs"][0]["n"], 1);
    let output = read("run_output");
    assert_eq!(output["body"]["n"], 1);
    assert!(output["body"]["text"].is_string());
    let file = read("file");
    assert_eq!(file["body"]["path"], "docs/plan.md");
    assert!(file["body"]["text"].is_string() && file["body"]["size"].is_u64());
    let tickets = read("tickets");
    assert_eq!(tickets["body"]["connected"], true);
    assert_eq!(tickets["body"]["tickets"][0]["id"], "t1");
    assert_eq!(
        tickets["body"]["tickets"][0]["decisions"][0]["name"],
        "lanes"
    );
    let ticket = read("ticket");
    assert!(ticket["body"]["body"].is_string());
    assert_eq!(ticket["body"]["attempts"][0]["n"], 1);
    let decided = read("decided");
    assert_eq!(decided["body"]["decision"]["id"], "d1");
    let events = read("ticket_events");
    assert_eq!(events["body"]["events"][0]["kind"], "stage");
    let file = read("ticket_file");
    assert!(file["body"]["text"].is_string());
    let closed = read("closed");
    assert_eq!(closed["body"]["ticket"]["state"], "closing");
    let created = read("created");
    assert_eq!(created["body"]["made"][0]["kind"], "session");
    let draft = read("draft");
    assert_eq!(draft["body"]["draft"]["text"], "Fix the test. and run");
    assert_eq!(draft["body"]["draft"]["status"], "listening");
    let info = read("info");
    assert_eq!(info["body"]["protocol"], 1);
    assert!(info["body"]["spki_sha256"].is_string());
    let pair = read("pair");
    assert_eq!(pair["body"]["token"].as_str().map(str::len), Some(43));
    assert_eq!(pair["body"]["scope"], "read");
    let error = read("error");
    assert_eq!(code(&error), "unauthorized");
    let pin = read("pin");
    let der = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        pin["certificate"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(
        remote::spki_sha256_of_cert(&der).unwrap(),
        pin["spki_sha256"].as_str().unwrap(),
        "the pin fixture's hash is the certificate's"
    );
}

// --- events

/// A device's event subscription: a thread holding the WebSocket over
/// the pinned connection, each text frame handed over `frames`, the
/// close as an `Err`. While `paused` the thread reads nothing, which is
/// how a test makes a subscriber fall behind.
/// What the test sends down its subscriber's socket.
enum Out {
    Text(String),
    Close,
}

struct Subscriber {
    frames: std::sync::mpsc::Receiver<Result<String, String>>,
    outbox: std::sync::mpsc::Sender<Out>,
    paused: Arc<std::sync::atomic::AtomicBool>,
}

impl Subscriber {
    /// A `watch` or `unwatch` frame for a screen.
    fn send(&self, kind: &str, resource_kind: &str, id: &str) {
        let text = serde_json::json!({
            "type": kind,
            "resource": { "kind": resource_kind, "id": id },
        })
        .to_string();
        self.outbox.send(Out::Text(text)).expect("socket thread");
    }

    /// Close the socket from the device's side.
    fn close(&self) {
        let _ = self.outbox.send(Out::Close);
    }
}

fn subscribe(port: &mut Port, token: &str) -> Subscriber {
    subscribe_at(port, token, "/api/v1/events")
}

/// A subscription at `path`, for its query (`?scope=all`).
fn subscribe_at(port: &mut Port, token: &str, path: &str) -> Subscriber {
    let bearer = vec![("Authorization".to_owned(), format!("Bearer {token}"))];
    subscribe_with(port, bearer, path).expect("subscribed")
}

/// A subscription at `path` with these handshake headers, or why the
/// upgrade did not give `hello`.
fn subscribe_with(
    port: &mut Port,
    headers: Vec<(String, String)>,
    path: &str,
) -> Result<Subscriber, String> {
    let addr = port.addr;
    let url = format!("wss://localhost{path}");
    let config = Arc::clone(&port.client);
    let (tx, frames) = std::sync::mpsc::channel();
    let (outbox, out_rx) = std::sync::mpsc::channel::<Out>();
    let paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hold = Arc::clone(&paused);
    std::thread::spawn(move || {
        let tcp = TcpStream::connect(addr).expect("connect");
        let conn = ClientConnection::new(config, ServerName::try_from("localhost").unwrap())
            .expect("client");
        let mut s = StreamOwned::new(conn, tcp);
        while s.conn.is_handshaking() {
            s.conn.complete_io(&mut s.sock).expect("handshake");
        }
        let mut request = url.into_client_request().unwrap();
        for (name, value) in headers {
            let name = tungstenite::http::HeaderName::from_bytes(name.as_bytes()).unwrap();
            request.headers_mut().insert(name, value.parse().unwrap());
        }
        let (mut ws, _) = match tungstenite::client(request, s) {
            Ok(ok) => ok,
            Err(e) => {
                let _ = tx.send(Err(format!("upgrade refused: {e}")));
                return;
            }
        };
        // A short read timeout, so the outbox is looked at between
        // frames; the library keeps a part-read frame across it.
        ws.get_mut()
            .sock
            .set_read_timeout(Some(Duration::from_millis(20)))
            .expect("timeout");
        loop {
            while hold.load(std::sync::atomic::Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            for out in out_rx.try_iter() {
                match out {
                    Out::Text(text) => ws.send(tungstenite::Message::text(text)).expect("send"),
                    Out::Close => {
                        let _ = ws.close(None);
                        let _ = ws.flush();
                        return;
                    }
                }
            }
            match ws.read() {
                Ok(tungstenite::Message::Text(text)) => {
                    if tx.send(Ok(text.to_string())).is_err() {
                        return;
                    }
                }
                Ok(tungstenite::Message::Close(_)) => {
                    let _ = tx.send(Err("closed".into()));
                    return;
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => {
                    let _ = tx.send(Err(format!("closed: {e}")));
                    return;
                }
            }
        }
    });
    let subscriber = Subscriber {
        frames,
        outbox,
        paused,
    };
    let hello = next_frame(port, &subscriber, 5).ok_or("no frame")??;
    let hello: serde_json::Value = serde_json::from_str(&hello).unwrap();
    assert_eq!(hello["type"], "hello", "{hello}");
    Ok(subscriber)
}

/// The next frame within `secs`, the app serving meanwhile.
fn next_frame(port: &mut Port, s: &Subscriber, secs: u64) -> Option<Result<String, String>> {
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    loop {
        port.app.serve_pending();
        match s.frames.try_recv() {
            Ok(frame) => return Some(frame),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        if std::time::Instant::now() > deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Every event frame that arrives before 300 ms of silence, parsed.
fn events(port: &mut Port, s: &Subscriber) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let mut quiet = std::time::Instant::now();
    loop {
        port.app.serve_pending();
        match s.frames.try_recv() {
            Ok(Ok(text)) => {
                out.push(serde_json::from_str(&text).unwrap());
                quiet = std::time::Instant::now();
                continue;
            }
            Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        if quiet.elapsed() > Duration::from_millis(300) {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    out
}

/// `(type, kind, id)` of each event, for comparing.
fn resources(events: &[serde_json::Value]) -> Vec<(String, String, String)> {
    events
        .iter()
        .map(|e| {
            (
                e["type"].as_str().unwrap_or("").to_owned(),
                e["resource"]["kind"].as_str().unwrap_or("").to_owned(),
                e["resource"]["id"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect()
}

#[test]
fn a_subscriber_gets_hello_then_changed_and_removed_as_the_state_moves() {
    let mut port = port();
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let sub = subscribe(&mut port, &token);
    assert_eq!(port.app.subscriber_count(), 1);
    let before = port.app.revision();
    let (space, project, session) = seed(&mut port, "ticket 1");
    let got = events(&mut port, &sub);
    let moved = resources(&got);
    for expected in [
        ("changed", "space", space.as_str()),
        ("changed", "spaces", ""),
        ("changed", "project", project.as_str()),
        ("changed", "projects", ""),
        ("changed", "session", session.as_str()),
        ("changed", "sessions", project.as_str()),
    ] {
        let expected = (
            expected.0.to_owned(),
            expected.1.to_owned(),
            expected.2.to_owned(),
        );
        assert!(moved.contains(&expected), "{expected:?} in {moved:?}");
    }
    let identity = port.remote.state().identity.clone().unwrap();
    for event in &got {
        assert_eq!(event["server_id"], identity.server_id.to_string());
        assert!(event["revision"].as_u64().unwrap() > before, "{event}");
        assert!(event["at_ms"].as_u64().unwrap() > 0);
        assert!(event["epoch"].is_string());
    }
    assert_eq!(port.app.revision(), got.last().unwrap()["revision"]);

    let record = RecordId(uuid::Uuid::parse_str(&session).unwrap());
    port.app.dispatch(AppAction::RemoveSession(record));
    let moved = resources(&events(&mut port, &sub));
    assert!(
        moved.contains(&("removed".into(), "session".into(), session.clone())),
        "{moved:?}"
    );
    assert!(
        moved.contains(&("changed".into(), "sessions".into(), project.clone())),
        "{moved:?}"
    );
}

/// Dispatch's worker records, a ticket's project and a worker session
/// with an op of Dispatch's `<ticket>-<nonce>` form, are the ticket
/// view's and not the browser's: a listing leaves them out and a plain
/// subscriber never hears of them, while `scope=all` gets everything.
/// A supervisor's records and the owner's own are listed as ever, and
/// a worker record is still read by its id.
/// A project added over the control port under `op`.
fn project_under_op(port: &mut Port, op: &str, space: &str, name: &str) -> String {
    let reply = control(
        port,
        Request::new(
            op,
            Body::ProjectAdd {
                space: space.to_owned(),
                name: name.into(),
                root: PathBuf::from("/tmp"),
            },
        ),
    );
    made_id(&reply, RecordKind::Project)
}

/// A session made over the control port under `op`.
fn session_under_op(port: &mut Port, op: &str, project: &str, name: &str) -> String {
    let reply = control(
        port,
        Request::new(
            op,
            Body::SessionNew {
                project: project.to_owned(),
                name: name.into(),
                session_kind: SessionKind::Claude,
                cwd: PathBuf::from("/tmp"),
                launch: Launch::Shell,
                prompt: None,
                notes: String::new(),
                env: std::collections::BTreeMap::new(),
                env_sets: Vec::new(),
                replaces: None,
            },
        ),
    );
    made_id(&reply, RecordKind::Session)
}

/// The ids of a listing at `path`, under `key`.
fn listed(port: &mut Port, path: &str, token: &str, key: &str) -> Vec<String> {
    let (status, reply) = get(port, path, Some(token));
    assert_eq!(status, 200, "{reply}");
    reply["body"][key]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["id"].as_str().unwrap().to_owned())
        .collect()
}

/// Dispatch's last status as a device reads it: a lanes decision open
/// on `t1`, one answered and one needing a note on `t2`, and `t3`
/// parked; `Orchard` has a supervisor session.
#[allow(clippy::too_many_lines)] // one status whole, read as the runner would answer it
/// The notes of the fixture ticket's first attempt, which the fake
/// Dispatch answers a read of.
const NOTES_PATH: &str = "/dispatch/tickets/t1/investigate/1/root/notes.md";
/// A screenshot in the fixture ticket's evidence, written to the disk
/// so the port reads it as the real runner's would be.
const SHOT_PATH: &str = "/tmp/switchboard-remote-tests/shot.png";
const SHOT_BYTES: &[u8] = b"\x89PNG\r\n\x1a\nnot really a picture";

#[allow(clippy::too_many_lines)]
fn tickets_status() -> switchboard::ports::dispatch::Status {
    use switchboard::ports::dispatch::{
        AttemptView, ChecksView, DecisionView, EvidenceView, LaneView, ProjectView, Status,
        SupervisorView, TicketView,
    };
    std::fs::create_dir_all("/tmp/switchboard-remote-tests").unwrap();
    std::fs::write(SHOT_PATH, SHOT_BYTES).unwrap();
    let evidence = |rel: &str, path: &str, bytes: u64| EvidenceView {
        path: path.into(),
        rel: rel.into(),
        bytes,
        modified_ms: 4,
    };
    let decision = |id: &str, ticket: &str, name: &str, options: &[&str]| DecisionView {
        id: id.into(),
        ticket: ticket.into(),
        stage: "plan".into(),
        name: name.into(),
        question: format!("{name}?"),
        options: options.iter().map(|o| (*o).to_owned()).collect(),
        state: "pending".into(),
        made_ms: 10,
        ..DecisionView::default()
    };
    Status {
        data_dir: "/dispatch".into(),
        worktrees: "/wt".into(),
        projects: vec![ProjectView {
            name: "Orchard".into(),
            queue: vec!["t1".into(), "t2".into()],
            slots: 2,
            waiting_on_me: 1,
            running: 1,
            pending: 1,
            supervisor: Some(SupervisorView {
                session: Some("sup-session-id".into()),
                ..SupervisorView::default()
            }),
            ..ProjectView::default()
        }],
        tickets: vec![
            TicketView {
                id: "t2".into(),
                project: "Orchard".into(),
                kind: "github".into(),
                number: Some(9),
                title: "Roster import".into(),
                state: "active".into(),
                stages: vec!["investigate".into(), "plan".into()],
                stage: 1,
                decisions: vec![
                    DecisionView {
                        state: "answered".into(),
                        answer: Some("merge".into()),
                        ..decision("d2", "t2", "merge", &["merge", "park"])
                    },
                    DecisionView {
                        needs_note: vec!["park".into()],
                        recommendation: Some("rerun".into()),
                        ..decision("d3", "t2", "rerun", &["rerun", "park"])
                    },
                ],
                created_ms: 2,
                updated_ms: 20,
                ..TicketView::default()
            },
            TicketView {
                id: "t1".into(),
                project: "Orchard".into(),
                kind: "github".into(),
                number: Some(104),
                title: "One file per entry".into(),
                body: "Each entry in its own file.".into(),
                url: Some("https://example.invalid/issues/104".into()),
                state: "active".into(),
                stages: vec!["investigate".into(), "plan".into()],
                stage: 1,
                decisions: vec![DecisionView {
                    multiple: true,
                    recommendation: Some("frontend".into()),
                    ..decision("d1", "t1", "lanes", &["backend", "frontend"])
                }],
                lanes: vec![LaneView {
                    name: "frontend".into(),
                    branch: "t1-frontend".into(),
                    chosen: true,
                    ..LaneView::default()
                }],
                attempts: vec![AttemptView {
                    stage: "investigate".into(),
                    n: 1,
                    context: "root".into(),
                    kind: "agent".into(),
                    state: "complete".into(),
                    session: Some("worker-session-id".into()),
                    artifacts: vec![
                        ("notes".into(), NOTES_PATH.into()),
                        (
                            "token".into(),
                            "/dispatch/tickets/t1/investigate/1/root/token".into(),
                        ),
                    ],
                    secret: vec!["token".into()],
                    checks: Some(ChecksView {
                        head: "abc1234".into(),
                        exit: Some(0),
                        log: "/dispatch/tickets/t1/investigate/1/root/checks.log".into(),
                        env: false,
                    }),
                    evidence: vec![
                        evidence(
                            "probe.log",
                            "/dispatch/tickets/t1/investigate/1/root/evidence/probe.log",
                            12,
                        ),
                        evidence("shot.png", SHOT_PATH, SHOT_BYTES.len() as u64),
                    ],
                    started_ms: 3,
                    ended_ms: Some(4),
                    ..AttemptView::default()
                }],
                current_session: Some("worker-session-id".into()),
                created_ms: 1,
                updated_ms: 30,
                ..TicketView::default()
            },
            TicketView {
                id: "t3".into(),
                project: "Orchard".into(),
                kind: "manual".into(),
                title: "Parked one".into(),
                state: "parked".into(),
                reason: Some("owner's call".into()),
                stages: vec!["investigate".into()],
                closable: true,
                created_ms: 3,
                updated_ms: 40,
                ..TicketView::default()
            },
        ],
    }
}

#[test]
fn tickets_are_listed_with_their_standing_and_read_alone() {
    let mut port = port_dispatching(tickets_status());
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let t = Some(token.as_str());
    let (status, reply) = get(&mut port, "/api/v1/dispatch/tickets", t);
    assert_eq!(status, 200, "{reply}");
    let body = &reply["body"];
    assert_eq!(body["connected"], true);
    assert_eq!(body["projects"][0]["supervisor_session"], "sup-session-id");
    let ids: Vec<&str> = body["tickets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        ["t1", "t2", "t3"],
        "oldest first, not the runner's order"
    );
    let t1 = &body["tickets"][0];
    assert_eq!(t1["standing"], "1 waiting on you");
    assert_eq!(
        (t1["waits"].as_bool(), t1["urgent"].as_bool()),
        (Some(true), Some(true))
    );
    assert_eq!(t1["stage"], "plan");
    assert_eq!(t1["decisions"][0]["question"], "lanes?");
    assert!(t1.get("body").is_none(), "the list leaves the body out");
    let t3 = &body["tickets"][2];
    assert_eq!(t3["standing"], "parked: owner's call");

    let (status, reply) = get(&mut port, "/api/v1/dispatch/tickets/t1", t);
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["body"], "Each entry in its own file.");
    assert_eq!(reply["body"]["attempts"][0]["session"], "worker-session-id");
    assert_eq!(reply["body"]["lanes"][0]["branch"], "t1-frontend");
    let (status, reply) = get(&mut port, "/api/v1/dispatch/tickets/nope", t);
    assert_eq!((status, code(&reply)), (404, "not_found"));
}

#[test]
fn a_decision_is_answered_under_control_and_checked_against_the_record() {
    let mut port = port_dispatching(tickets_status());
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let t = Some(token.as_str());
    let decide = "/api/v1/dispatch/tickets/t1/decide";
    let answer = r#"{"decision":"d1","answer":"frontend"}"#;
    let (status, reply) = call(&mut port, "POST", decide, t, answer);
    assert_eq!((status, code(&reply)), (403, "scope"), "{reply}");
    let id = port.app.core().devices()[0].id;
    port.app.dispatch(AppAction::SetDeviceScope {
        device: id,
        scope: Scope::Control,
    });
    let heard = subscribe(&mut port, &token);
    let (status, reply) = call(&mut port, "POST", decide, t, answer);
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["decision"]["id"], "d1");

    // Not among the options; a note where the option needs one; a
    // decision answered as asked is answered again from the record,
    // and otherwise refused.
    let (status, reply) = call(
        &mut port,
        "POST",
        decide,
        t,
        r#"{"decision":"d1","answer":"ios"}"#,
    );
    assert_eq!((status, code(&reply)), (400, "bad_request"), "{reply}");
    let decide2 = "/api/v1/dispatch/tickets/t2/decide";
    let park = r#"{"decision":"d3","answer":"park"}"#;
    let (status, reply) = call(&mut port, "POST", decide2, t, park);
    assert_eq!((status, code(&reply)), (400, "bad_request"), "{reply}");
    let noted = r#"{"decision":"d3","answer":"park","note":"later"}"#;
    let (status, reply) = call(&mut port, "POST", decide2, t, noted);
    assert_eq!(status, 200, "{reply}");
    let again = r#"{"decision":"d2","answer":"merge"}"#;
    let (status, reply) = call(&mut port, "POST", decide2, t, again);
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["decision"]["state"], "answered");
    let other = r#"{"decision":"d2","answer":"park"}"#;
    let (status, reply) = call(&mut port, "POST", decide2, t, other);
    assert_eq!((status, code(&reply)), (409, "answered"), "{reply}");

    // The next status answers the lanes question: the listing moved.
    let mut next = tickets_status();
    next.tickets[1].decisions[0].state = "answered".into();
    next.tickets[1].decisions[0].answer = Some("frontend".into());
    port.app.dispatch(AppAction::DispatchStatus(Some(next)));
    let kinds = resources(&events(&mut port, &heard));
    assert!(
        kinds
            .iter()
            .any(|(t, k, _)| t == "changed" && k == "tickets"),
        "{kinds:?}"
    );
    let (_, reply) = get(&mut port, "/api/v1/dispatch/tickets", t);
    let standing = reply["body"]["tickets"][0]["standing"].as_str().unwrap();
    assert!(standing.starts_with("held:"), "{standing}");
}

#[test]
fn a_tickets_files_are_listed_by_key_and_read_through_the_port() {
    use base64::Engine as _;
    let mut port = port_dispatching(tickets_status());
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let t = Some(token.as_str());
    let (status, reply) = get(&mut port, "/api/v1/dispatch/tickets/t1", t);
    assert_eq!(status, 200, "{reply}");
    let attempt = &reply["body"]["attempts"][0];
    assert_eq!(attempt["checks"]["exit"], 0);
    let files = attempt["files"].as_array().unwrap();
    let names: Vec<&str> = files.iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        ["notes", "token", "checks", "probe.log", "shot.png"],
        "artifacts, the checks log, then the evidence"
    );
    let kinds: Vec<&str> = files.iter().map(|f| f["kind"].as_str().unwrap()).collect();
    assert_eq!(
        kinds,
        ["artifact", "secret", "checks", "evidence", "evidence"]
    );
    assert_eq!(files[1]["inline"], false, "a secret artifact is never read");
    assert_eq!(files[4]["image"], true);
    assert_eq!(files[3]["bytes"], 12);
    let key = |i: usize| files[i]["key"].as_str().unwrap().to_owned();
    assert!(
        files.iter().all(|f| f.get("path").is_none()),
        "paths stay on the Mac"
    );

    // Text comes through the Mac's own read of Dispatch's port.
    let read = format!("/api/v1/dispatch/tickets/t1/files/{}", key(0));
    let (status, reply) = get(&mut port, &read, t);
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["key"], key(0));
    assert_eq!(reply["body"]["text"], "Found it: one file each.\n");
    // Again, from the cache: the ticket has not changed.
    let (status, reply) = get(&mut port, &read, t);
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["text"], "Found it: one file each.\n");

    // The fake has no text for the checks log: the runner's refusal.
    let read = format!("/api/v1/dispatch/tickets/t1/files/{}", key(2));
    let (status, reply) = get(&mut port, &read, t);
    assert_eq!((status, code(&reply)), (404, "not_found"), "{reply}");
    let read = format!("/api/v1/dispatch/tickets/t1/files/{}", key(1));
    let (status, reply) = get(&mut port, &read, t);
    assert_eq!((status, code(&reply)), (415, "not_text"), "{reply}");

    // An image comes from the disk, as base64 with its media type.
    let read = format!("/api/v1/dispatch/tickets/t1/files/{}", key(4));
    let (status, reply) = get(&mut port, &read, t);
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["media"], "image/png");
    let data = reply["body"]["data"].as_str().unwrap();
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap(),
        SHOT_BYTES
    );
    let (status, reply) = get(&mut port, "/api/v1/dispatch/tickets/t1/files/nope", t);
    assert_eq!((status, code(&reply)), (404, "not_found"), "{reply}");
}

#[test]
fn a_tickets_events_are_read_and_a_closable_ticket_is_closed() {
    let mut port = port_dispatching(tickets_status());
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let t = Some(token.as_str());
    let events = "/api/v1/dispatch/tickets/t1/events";
    let (status, reply) = get(&mut port, events, t);
    assert_eq!(status, 200, "{reply}");
    let listed = reply["body"]["events"].as_array().unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[1]["text"], "root #1 agent");
    assert_eq!(listed[1]["attempt"], "investigate/1");
    assert_eq!(reply["body"]["last"], 2);
    assert_eq!(reply["body"]["unavailable"], false);
    // Again, from the cache.
    let (status, reply) = get(&mut port, events, t);
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["events"].as_array().unwrap().len(), 2);
    let (status, reply) = get(&mut port, "/api/v1/dispatch/tickets/nope/events", t);
    assert_eq!((status, code(&reply)), (404, "not_found"), "{reply}");

    // A close needs the control scope and a closable ticket.
    let close = "/api/v1/dispatch/tickets/t3/close";
    let (status, reply) = call(&mut port, "POST", close, t, "{}");
    assert_eq!((status, code(&reply)), (403, "scope"), "{reply}");
    let id = port.app.core().devices()[0].id;
    port.app.dispatch(AppAction::SetDeviceScope {
        device: id,
        scope: Scope::Control,
    });
    let (status, reply) = call(&mut port, "POST", close, t, "{}");
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["ticket"]["id"], "t3");
    assert_eq!(reply["body"]["ticket"]["state"], "closing");
    let (status, reply) = call(
        &mut port,
        "POST",
        "/api/v1/dispatch/tickets/t1/close",
        t,
        "{}",
    );
    assert_eq!((status, code(&reply)), (409, "not_closable"), "{reply}");
}

#[test]
fn a_parked_ticket_is_resumed_and_without_a_runner_the_tickets_still_read() {
    let mut port = port_dispatching(tickets_status());
    let token = pair(&mut port, "ABCD2345", Scope::Control, "Vision Pro");
    let t = Some(token.as_str());
    let (status, reply) = call(
        &mut port,
        "POST",
        "/api/v1/dispatch/tickets/t3/resume",
        t,
        "{}",
    );
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["ticket"]["id"], "t3");
    let (status, reply) = call(
        &mut port,
        "POST",
        "/api/v1/dispatch/tickets/t1/resume",
        t,
        "{}",
    );
    assert_eq!((status, code(&reply)), (409, "not_parked"), "{reply}");

    port.app.dispatch(AppAction::DispatchStatus(None));
    let (status, reply) = get(&mut port, "/api/v1/dispatch/tickets", t);
    assert_eq!(status, 200);
    assert_eq!(reply["body"]["connected"], false);
    assert_eq!(
        reply["body"]["tickets"].as_array().unwrap().len(),
        3,
        "the last seen"
    );
    let (status, reply) = call(
        &mut port,
        "POST",
        "/api/v1/dispatch/tickets/t3/resume",
        t,
        "{}",
    );
    assert_eq!((status, code(&reply)), (503, "dispatch_off"), "{reply}");
}

/// The ids of the `session` frames a subscriber got.
fn session_frames(port: &mut Port, s: &Subscriber) -> Vec<String> {
    resources(&events(port, s))
        .into_iter()
        .filter(|(_, kind, _)| kind == "session")
        .map(|(_, _, id)| id)
        .collect()
}

#[test]
fn dispatchs_worker_records_are_listed_and_announced_only_for_scope_all() {
    let mut port = port();
    let (space, project, session) = seed(&mut port, "");
    let ticket = project_under_op(&mut port, "2fac53cf-1a0b9b3f", &space, "#7");
    let planner = session_under_op(&mut port, "2fac53cf-cfb35312", &ticket, "planner");
    let sup_project = project_under_op(&mut port, "sup-Switchboard-710de0ce", &space, "Sup · S");
    let supervisor = session_under_op(&mut port, "sup-Switchboard-62f47554", &sup_project, "Sup");
    let reviewer = session_under_op(&mut port, "2fac53cf-c6c61153", &project, "reviewer");
    port.app.poll_now();
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let shown = listed(&mut port, "/api/v1/projects", &token, "projects");
    assert!(
        shown.contains(&project) && shown.contains(&sup_project),
        "{shown:?}"
    );
    assert!(!shown.contains(&ticket), "the ticket's project is left out");
    let all = listed(&mut port, "/api/v1/projects?scope=all", &token, "projects");
    assert!(all.contains(&ticket), "scope=all lists it");
    let path = format!("/api/v1/projects/{project}/sessions");
    let shown = listed(&mut port, &path, &token, "sessions");
    assert!(
        shown.contains(&session) && !shown.contains(&reviewer),
        "{shown:?}"
    );
    let all = listed(&mut port, &format!("{path}?scope=all"), &token, "sessions");
    assert!(all.contains(&reviewer), "scope=all lists the worker");
    let path = format!("/api/v1/projects/{sup_project}/sessions");
    let shown = listed(&mut port, &path, &token, "sessions");
    assert!(
        shown.contains(&supervisor),
        "a supervisor's session is the owner's"
    );
    let (status, _) = get(
        &mut port,
        &format!("/api/v1/sessions/{reviewer}"),
        Some(&token),
    );
    assert_eq!(status, 200, "a worker record is still read by its id");

    let plain = subscribe(&mut port, &token);
    let all = subscribe_at(&mut port, &token, "/api/v1/events?scope=all");
    for (id, name) in [
        (&reviewer, "reviewer 2"),
        (&planner, "planner 2"),
        (&supervisor, "Supervisor 2"),
        (&session, "investigator 2"),
    ] {
        let record = RecordId(uuid::Uuid::parse_str(id).unwrap());
        port.app
            .dispatch(AppAction::RenameSession(record, name.into()));
    }
    let heard = resources(&events(&mut port, &plain));
    assert!(
        !heard
            .iter()
            .any(|(_, kind, id)| kind == "sessions" && id == &ticket),
        "no frame about the ticket project's collection: {heard:?}"
    );
    let plain_ids: Vec<String> = heard
        .into_iter()
        .filter(|(_, kind, _)| kind == "session")
        .map(|(_, _, id)| id)
        .collect();
    assert!(
        plain_ids.contains(&session) && plain_ids.contains(&supervisor),
        "{plain_ids:?}"
    );
    assert!(
        !plain_ids.contains(&reviewer) && !plain_ids.contains(&planner),
        "{plain_ids:?}"
    );
    let all_ids = session_frames(&mut port, &all);
    assert!(
        all_ids.contains(&reviewer) && all_ids.contains(&planner),
        "{all_ids:?}"
    );
}

/// What the effect list would miss: a set rename (`SaveViews`), a trust
/// question (`PromptSeen`, no save) and a Codex pane passing its quiet
/// interval (`HostListed`, no save) each emit `changed`; ten ticks over
/// a running pane with nothing else moving emit nothing, though
/// `quiet_secs` moved on each.
#[test]
fn a_rename_a_trust_question_and_a_quiet_pane_emit_but_ten_quiet_passes_do_not() {
    let mut port = port();
    let (space, _project, session) = seed(&mut port, "");
    control(
        &mut port,
        Request::new(
            "st",
            Body::SetNew {
                space: space.clone(),
                name: "Desk".into(),
            },
        ),
    );
    let set = port
        .app
        .core()
        .set_views(switchboard::core::SpaceId(
            uuid::Uuid::parse_str(&space).unwrap(),
        ))
        .into_iter()
        .find(|s| s.name == "Desk")
        .unwrap();
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Pad");
    let sub = subscribe(&mut port, &token);
    port.app.dispatch(AppAction::RenameWorkingSet {
        set: switchboard::core::SetId(uuid::Uuid::parse_str(&set.id).unwrap()),
        name: "Bench".into(),
    });
    assert_eq!(
        resources(&events(&mut port, &sub)),
        vec![("changed".into(), "set".into(), set.id.clone())]
    );

    let record = RecordId(uuid::Uuid::parse_str(&session).unwrap());
    let pane = HostId(record.host_name());
    port.host.state().statuses.push(HostStatus {
        id: pane.clone(),
        liveness: HostLiveness::Running {
            pid: 7,
            command: "claude".into(),
        },
        cwd: None,
        last_activity: Some(std::time::SystemTime::now()),
        title: None,
    });
    port.app.poll_now();
    let _ = events(&mut port, &sub);
    port.app.dispatch(AppAction::PromptSeen {
        id: record,
        seen: true,
    });
    let moved = resources(&events(&mut port, &sub));
    assert!(
        moved.contains(&("changed".into(), "session".into(), session.clone())),
        "{moved:?}"
    );
    assert!(
        moved.contains(&("changed".into(), "waiting".into(), String::new())),
        "{moved:?}"
    );
    let revision = port.app.revision();
    for _ in 0..10 {
        port.app.dispatch(AppAction::Tick);
        port.app.serve_pending();
    }
    assert_eq!(events(&mut port, &sub), Vec::<serde_json::Value>::new());
    assert_eq!(port.app.revision(), revision, "nothing moved");
}

/// A mutation after `hello`: its event and a snapshot taken after it
/// carry the same revision, so the client reconciles the two as one.
#[test]
fn a_mutation_after_hello_is_seen_at_the_revision_the_snapshot_carries() {
    let mut port = port();
    let (_, _, session) = seed(&mut port, "before");
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Pad");
    let sub = subscribe(&mut port, &token);
    let hello_revision = port.app.revision();
    let record = RecordId(uuid::Uuid::parse_str(&session).unwrap());
    port.app
        .dispatch(AppAction::SetSessionNotes(record, "after".into()));
    let got = events(&mut port, &sub);
    let changed = got
        .iter()
        .find(|e| e["resource"]["kind"] == "session")
        .expect("the session changed");
    assert_eq!(changed["revision"], hello_revision + 1);
    let (status, reply) = get(
        &mut port,
        &format!("/api/v1/sessions/{session}"),
        Some(&token),
    );
    assert_eq!(status, 200);
    assert_eq!(reply["body"]["session"]["notes"], "after");
    assert_eq!(reply["revision"], changed["revision"]);
}

/// A subscriber that stops reading is dropped once it falls a queue
/// behind: it gets `resync_required` and the close, and a subscriber
/// that keeps up sees every event meanwhile. The device's socket
/// buffers hold a good deal before the host's send blocks, so the
/// burst here is a hundred and fifty sessions flipping between listed
/// and not, as many times as it takes.
#[test]
fn an_overflowed_subscriber_is_told_to_resync_and_closed_while_another_keeps_up() {
    let mut port = port();
    let (_, project, _) = seed(&mut port, "");
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Pad");
    let slow = subscribe(&mut port, &token);
    let fast = subscribe(&mut port, &token);
    slow.paused.store(true, std::sync::atomic::Ordering::SeqCst);
    for n in 1..150 {
        port.app.dispatch(AppAction::Control {
            op: format!("se{n}"),
            action: switchboard::core::ControlAction::try_from(Body::SessionNew {
                project: project.clone(),
                name: format!("s{n}"),
                session_kind: SessionKind::Shell,
                cwd: PathBuf::from("/tmp"),
                launch: Launch::Shell,
                prompt: None,
                notes: String::new(),
                env: std::collections::BTreeMap::new(),
                env_sets: Vec::new(),
                replaces: None,
            })
            .unwrap(),
        });
    }
    let all: Vec<HostStatus> = port.app.core().workspaces()[0]
        .sessions
        .iter()
        .map(|s| HostStatus {
            id: HostId(s.id.host_name()),
            liveness: HostLiveness::Running {
                pid: 7,
                command: "sh".into(),
            },
            cwd: None,
            last_activity: None,
            title: None,
        })
        .collect();
    let mut rounds = 0;
    while port.app.subscriber_count() == 2 && rounds < 200 {
        port.host.state().statuses = if rounds % 2 == 0 {
            all.clone()
        } else {
            Vec::new()
        };
        port.app.poll_now();
        port.app.serve_pending();
        rounds += 1;
    }
    assert_eq!(port.app.subscriber_count(), 1, "the slow one is dropped");
    slow.paused
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let mut last_text = None;
    let mut closed = false;
    while let Some(frame) = next_frame(&mut port, &slow, 15) {
        let Ok(text) = frame else {
            closed = true;
            break;
        };
        last_text = Some(text);
    }
    let last: serde_json::Value = serde_json::from_str(&last_text.expect("frames")).unwrap();
    assert_eq!(last["type"], "resync_required", "{last}");
    assert!(closed, "the socket was closed after it");
    let seen_by_fast = events(&mut port, &fast);
    assert!(
        seen_by_fast.len() > 300,
        "{} events reached the fast one",
        seen_by_fast.len()
    );
    assert_eq!(
        port.app.subscriber_count(),
        1,
        "the fast one is still subscribed"
    );
}

/// Revoke in the dialog closes the device's socket at once, with no
/// `resync_required`: there is nothing to come back for.
#[test]
fn revoke_closes_the_devices_socket() {
    let mut port = port();
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Pad");
    let sub = subscribe(&mut port, &token);
    let device = port.app.core().devices()[0].id;
    port.app.dispatch(AppAction::RevokeDevice(device));
    assert_eq!(port.app.subscriber_count(), 0);
    match next_frame(&mut port, &sub, 5) {
        Some(Err(reason)) => assert!(reason.starts_with("closed"), "{reason}"),
        other => panic!("expected the close, got {other:?}"),
    }
    let (status, _) = get(&mut port, "/api/v1/waiting", Some(&token));
    assert_eq!(status, 401, "the token is gone too");
}

/// `/ops/{op}` on the port: a creation's status is `done` with its
/// reply, an op never seen is `unknown`; and `/events` without an
/// upgrade is refused by name.
#[test]
fn ops_status_is_read_on_the_port_and_events_needs_an_upgrade() {
    let mut port = port();
    seed(&mut port, "");
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Pad");
    let (status, reply) = get(&mut port, "/api/v1/ops/se", Some(&token));
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["status"]["status"], "done");
    assert_eq!(reply["body"]["status"]["reply"]["reply"], "launched");
    let (_, reply) = get(&mut port, "/api/v1/ops/never", Some(&token));
    assert_eq!(reply["body"]["status"]["status"], "unknown");
    let (status, reply) = get(&mut port, "/api/v1/events", Some(&token));
    assert_eq!((status, code(&reply)), (400, "upgrade_required"), "{reply}");
    let (status, _) = get(&mut port, "/api/v1/events", None);
    assert_eq!(status, 401);
}

/// The diff pass over three hundred sessions, timed and printed (run
/// with `--nocapture`); the spec's budget is 1 ms of frame time per
/// pass.
#[test]
fn the_fingerprint_pass_over_three_hundred_sessions_is_measured() {
    let mut port = port();
    let (_, project, _) = seed(&mut port, "ticket");
    for n in 1..300 {
        port.app.dispatch(AppAction::Control {
            op: format!("se{n}"),
            action: switchboard::core::ControlAction::try_from(Body::SessionNew {
                project: project.clone(),
                name: format!("s{n}"),
                session_kind: SessionKind::Shell,
                cwd: PathBuf::from("/tmp"),
                launch: Launch::Shell,
                prompt: None,
                notes: "n".repeat(500),
                env: std::collections::BTreeMap::new(),
                env_sets: Vec::new(),
                replaces: None,
            })
            .unwrap(),
        });
    }
    assert_eq!(port.app.core().workspaces()[0].sessions.len(), 300);
    let now = std::time::SystemTime::now();
    let t0 = std::time::Instant::now();
    let runs = 20;
    let mut fp = None;
    for _ in 0..runs {
        fp = Some(switchboard::core::revision::Fingerprints::of(
            port.app.core(),
            now,
        ));
    }
    let per_pass = t0.elapsed() / runs;
    let fp = fp.unwrap();
    let t1 = std::time::Instant::now();
    let changes = fp.changes(&fp);
    let compare = t1.elapsed();
    assert!(changes.is_empty());
    println!(
        "fingerprint pass over {} resources: {per_pass:?}; compare: {compare:?}",
        fp.len()
    );
}

/// Put `session`'s pane on the fake host, running, showing `text`.
fn pane_running(port: &mut Port, session: &str, text: &str) -> RecordId {
    let record = RecordId(uuid::Uuid::parse_str(session).unwrap());
    let pane = HostId(record.host_name());
    {
        let mut state = port.host.state();
        state.statuses.retain(|s| s.id != pane);
        state.statuses.push(HostStatus {
            id: pane.clone(),
            liveness: HostLiveness::Running {
                pid: 7,
                command: "claude".into(),
            },
            cwd: None,
            last_activity: None,
            title: None,
        });
        state.snapshots.insert(pane, text.into());
    }
    port.app.poll_now();
    record
}

/// Serve until `check` holds or `secs` pass.
fn settle(port: &mut Port, secs: u64, check: impl Fn(&SwitchboardApp) -> bool) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    loop {
        port.app.serve_pending();
        if check(&port.app) {
            return true;
        }
        if std::time::Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A screen read by a paired device: the revision and the text.
fn screen(port: &mut Port, session: &str, token: &str, lines: Option<usize>) -> (u64, String) {
    let query = lines.map(|n| format!("?lines={n}")).unwrap_or_default();
    let (status, reply) = get(
        port,
        &format!("/api/v1/sessions/{session}/screen{query}"),
        Some(token),
    );
    assert_eq!(status, 200, "{reply}");
    (
        reply["revision"].as_u64().unwrap(),
        reply["body"]["text"].as_str().unwrap().to_owned(),
    )
}

/// A `watch` is answered `watching` and enters the key; the poll then
/// observes the pane and emits `changed` for the screen; two reads of
/// the same text carry one revision, however many lines each asks for,
/// and the poll says nothing while the pane holds still; a pane that
/// moves is a new revision, which the next read carries.
#[test]
fn a_watched_screen_moves_on_poll_and_an_unchanged_read_keeps_its_revision() {
    let mut port = port();
    let (_, _, session) = seed(&mut port, "");
    let record = pane_running(&mut port, &session, "one\ntwo\n");
    port.app.set_watch_poll(Duration::from_millis(50));
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let sub = subscribe(&mut port, &token);
    sub.send("watch", "screen", &session);
    let got = events(&mut port, &sub);
    let moved = resources(&got);
    assert_eq!(
        moved.first(),
        Some(&("watching".into(), "screen".into(), session.clone())),
        "{moved:?}"
    );
    assert!(
        moved.contains(&("changed".into(), "screen".into(), session.clone())),
        "{moved:?}"
    );
    assert_eq!(port.app.watched_keys(), vec![ContentKey::Screen(record)]);
    let first = port
        .app
        .content_revision(&ContentKey::Screen(record))
        .unwrap();
    assert_eq!(got.last().unwrap()["revision"], first);

    let (r1, text) = screen(&mut port, &session, &token, None);
    assert_eq!((r1, text.as_str()), (first, "one\ntwo"));
    let (r2, text) = screen(&mut port, &session, &token, Some(1));
    assert_eq!((r2, text.as_str()), (first, "two"));
    assert!(events(&mut port, &sub).is_empty(), "nothing moved");

    port.host
        .state()
        .snapshots
        .insert(HostId(record.host_name()), "one\ntwo\nthree\n".into());
    let got = events(&mut port, &sub);
    assert_eq!(
        resources(&got),
        vec![("changed".into(), "screen".into(), session.clone())]
    );
    let next = got[0]["revision"].as_u64().unwrap();
    assert!(next > first);
    let (r3, text) = screen(&mut port, &session, &token, None);
    assert_eq!((r3, text.as_str()), (next, "one\ntwo\nthree"));
}

/// A secret added to the project after the watch moves the screen
/// though the pane did not: the policy is in the version, and the next
/// read carries the value's name instead of the value.
#[test]
fn a_policy_change_moves_a_watched_screen_though_the_pane_did_not() {
    let mut port = port();
    let (_, project, session) = seed(&mut port, "");
    pane_running(&mut port, &session, "$ env\nAPI_TOKEN=tok-5e3cr3t\n");
    port.app.set_watch_poll(Duration::from_millis(50));
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let sub = subscribe(&mut port, &token);
    sub.send("watch", "screen", &session);
    let got = events(&mut port, &sub);
    assert_eq!(got.len(), 2, "{got:?}");
    let (first, text) = screen(&mut port, &session, &token, None);
    assert!(
        text.contains("tok-5e3cr3t"),
        "no secret declared yet: {text}"
    );

    let pid = switchboard::core::ProjectId(uuid::Uuid::parse_str(&project).unwrap());
    port.secrets.state().insert(
        switchboard::core::SecretScope::Project(pid).account("API_TOKEN"),
        "tok-5e3cr3t".into(),
    );
    port.app.dispatch(AppAction::SetProjectEnv(
        pid,
        switchboard::core::ProjectEnv {
            vars: vec![switchboard::core::EnvVar {
                name: "API_TOKEN".into(),
                value: String::new(),
                secret: true,
            }],
            ..Default::default()
        },
    ));
    let got = events(&mut port, &sub);
    let moved = resources(&got);
    assert!(
        moved.contains(&("changed".into(), "screen".into(), session.clone())),
        "{moved:?}"
    );
    let (next, text) = screen(&mut port, &session, &token, None);
    assert!(next > first);
    assert_eq!(text, "$ env\nAPI_TOKEN=<API_TOKEN>");
}

/// Past the cap the least recently read key goes, silently; a read of
/// an evicted key works and puts it back, at another's expense.
#[test]
fn the_watch_set_is_capped_and_a_read_of_an_evicted_key_puts_it_back() {
    let mut port = port();
    let (_, project, _) = seed(&mut port, "");
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Pad");
    let mut sessions = Vec::new();
    for n in 0..(switchboard::content::WATCH_CAP + 2) {
        // An op id of its own each: a repeated id is replayed.
        let reply = control(
            &mut port,
            Request::new(
                format!("se{n}"),
                Body::SessionNew {
                    project: project.clone(),
                    name: format!("s{n}"),
                    session_kind: SessionKind::Claude,
                    cwd: PathBuf::from("/tmp"),
                    launch: Launch::Shell,
                    prompt: None,
                    notes: String::new(),
                    env: std::collections::BTreeMap::new(),
                    env_sets: Vec::new(),
                    replaces: None,
                },
            ),
        );
        let session = made_id(&reply, RecordKind::Session);
        let record = pane_running(&mut port, &session, &format!("pane {n}\n"));
        sessions.push((session, ContentKey::Screen(record)));
    }
    for (session, _) in &sessions {
        screen(&mut port, session, &token, Some(1));
    }
    let watched = port.app.watched_keys();
    assert_eq!(watched.len(), switchboard::content::WATCH_CAP);
    assert!(!watched.contains(&sessions[0].1), "the first read went");
    assert!(!watched.contains(&sessions[1].1), "and the second");
    assert!(watched.contains(&sessions[2].1));

    let (_, text) = screen(&mut port, &sessions[0].0, &token, Some(1));
    assert_eq!(text, "pane 0");
    let watched = port.app.watched_keys();
    assert_eq!(watched.len(), switchboard::content::WATCH_CAP);
    assert!(watched.contains(&sessions[0].1), "back in");
    assert!(!watched.contains(&sessions[2].1), "the oldest went for it");
}

/// A key stays while any subscriber holds it: one device's `unwatch`
/// leaves the other's watch, a closed socket drops its watches, and a
/// revocation drops the device's at once. A watch of a pane that is
/// not running is answered `watching` too; it is read when it runs.
#[test]
fn unwatch_a_closed_socket_and_a_revocation_each_drop_their_watches() {
    let mut port = port();
    let (_, _, session) = seed(&mut port, "");
    let record = RecordId(uuid::Uuid::parse_str(&session).unwrap());
    let key = ContentKey::Screen(record);
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Pad");
    let a = subscribe(&mut port, &token);
    let b = subscribe(&mut port, &token);
    a.send("watch", "screen", &session);
    b.send("watch", "screen", &session);
    assert_eq!(resources(&events(&mut port, &a)).len(), 1, "watching");
    assert_eq!(resources(&events(&mut port, &b)).len(), 1, "watching");
    assert_eq!(port.app.watched_keys(), vec![key.clone()]);

    a.send("unwatch", "screen", &session);
    assert!(events(&mut port, &a).is_empty());
    assert_eq!(
        port.app.watched_keys(),
        vec![key.clone()],
        "b still holds it"
    );

    b.close();
    assert!(
        settle(&mut port, 5, |app| app.watched_keys().is_empty()),
        "b's close dropped the watch"
    );
    assert_eq!(port.app.subscriber_count(), 1);

    a.send("watch", "screen", &session);
    assert!(settle(&mut port, 5, |app| !app.watched_keys().is_empty()));
    let device = port.app.core().devices()[0].id;
    port.app.dispatch(AppAction::RevokeDevice(device));
    assert!(
        port.app.watched_keys().is_empty(),
        "revoked with its watches"
    );
    assert_eq!(port.app.subscriber_count(), 0);
}

/// Strangers on one address that connect and say nothing take that
/// address's share of the unauthenticated pool and no more: the next is
/// closed at once, a paired subscriber keeps receiving meanwhile, and
/// the slots are free once they hang up. (Each would also be closed at
/// the handshake deadline, five seconds on; the test is done sooner.)
#[test]
fn stalled_strangers_fill_their_own_pool_and_never_a_paired_clients() {
    let mut port = port();
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Pad");
    let sub = subscribe(&mut port, &token);
    let strangers: Vec<TcpStream> = (0..MAX_UNAUTHENTICATED_PER_SOURCE)
        .map(|_| TcpStream::connect(port.addr).expect("connect"))
        .collect();
    // The listener has handed each to a thread before it accepts more.
    std::thread::sleep(Duration::from_millis(100));
    let mut next = TcpStream::connect(port.addr).expect("connect");
    next.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut buf = [0u8; 1];
    let closed = matches!(next.read(&mut buf), Ok(0) | Err(_));
    assert!(
        closed,
        "the next stranger from the address was closed at once"
    );
    // A page load is a stranger too, and gets no slot past the share.
    assert!(
        http_head(port.addr, &port.client, "GET", "/", &[], "").is_err(),
        "a page request past the share fails at accept"
    );

    let (_, _, session) = seed(&mut port, "");
    let moved = resources(&events(&mut port, &sub));
    assert!(
        moved.contains(&("changed".into(), "session".into(), session.clone())),
        "the paired subscriber kept receiving: {moved:?}"
    );

    drop(strangers);
    std::thread::sleep(Duration::from_millis(100));
    let (status, _) = get(&mut port, "/api/v1/waiting", Some(&token));
    assert_eq!(status, 200, "the slots are free again");
}

/// Two strangers stalled on an address leave room in its share for a
/// paired device's fresh connection, which starts unauthenticated. A
/// third would make that connection the fourth from the address, which
/// is refused by design; that strangers on one address never shut out
/// a device on another is the `Admission` unit test, since a second
/// loopback address needs root on macOS.
#[test]
fn a_fresh_paired_connection_finds_room_beside_stalled_strangers() {
    let mut port = port();
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Pad");
    let strangers: Vec<TcpStream> = (0..MAX_UNAUTHENTICATED_PER_SOURCE - 1)
        .map(|_| TcpStream::connect(port.addr).expect("connect"))
        .collect();
    std::thread::sleep(Duration::from_millis(100));
    let (status, reply) = get(&mut port, "/api/v1/waiting", Some(&token));
    assert_eq!(status, 200, "{reply}");
    drop(strangers);
}

/// Requests in turn over one pinned connection, each written once the
/// one before is answered, served by the app on this thread: the
/// answers, and whether the host then ended the connection.
fn on_one_connection(port: &mut Port, requests: &[&str]) -> (Vec<Answer>, bool) {
    let addr = port.addr;
    let config = Arc::clone(&port.client);
    let requests: Vec<String> = requests.iter().map(|r| (*r).to_owned()).collect();
    let client = std::thread::spawn(move || -> std::io::Result<(Vec<Answer>, bool)> {
        let mut reader = BufReader::new(stranger_tls(addr, &config));
        reader
            .get_ref()
            .sock
            .set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut answers = Vec::new();
        for request in requests {
            reader.get_mut().write_all(request.as_bytes())?;
            reader.get_mut().flush()?;
            answers.push(read_answer(&mut reader)?);
        }
        reader
            .get_ref()
            .sock
            .set_read_timeout(Some(Duration::from_millis(300)))?;
        let mut buf = [0u8; 1];
        let ended = match reader.read(&mut buf) {
            Ok(n) => n == 0,
            Err(e) => !matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ),
        };
        Ok((answers, ended))
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !client.is_finished() {
        assert!(std::time::Instant::now() < deadline, "no reply");
        port.app.serve_pending();
        std::thread::sleep(Duration::from_millis(2));
    }
    client.join().unwrap().expect("http")
}

/// A stranger gets one answer per connection, whatever it is, and is
/// told so; a paired device keeps its connection for the next request.
#[test]
fn every_reply_to_a_stranger_closes_its_connection_and_a_paired_one_stays_open() {
    let mut port = port();
    // Pairing first spends one of `/pair`'s six a minute.
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Pad");
    let info = "GET /api/v1/info HTTP/1.1\r\nHost: localhost\r\n\r\n";
    let pair_request =
        "POST /api/v1/pair HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\n\r\n{}";
    let bad_bearer =
        "GET /api/v1/waiting HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer nope\r\n\r\n";
    for (request, status) in [(info, 200), (pair_request, 403), (bad_bearer, 401)] {
        let (answers, ended) = on_one_connection(&mut port, &[request]);
        assert_eq!(answers[0].0, status, "{request}: {}", answers[0].2);
        assert_eq!(
            header(&answers[0].1, "connection"),
            Some("close"),
            "{request}"
        );
        assert!(ended, "{request}: the host closed after its reply");
    }
    // `/pair` has been asked twice above; four more fill its window.
    for _ in 0..4 {
        let (status, _) = call(&mut port, "POST", "/api/v1/pair", None, "{}");
        assert_eq!(status, 403);
    }
    let (answers, ended) = on_one_connection(&mut port, &[pair_request]);
    assert_eq!(answers[0].0, 429);
    assert_eq!(header(&answers[0].1, "connection"), Some("close"));
    assert!(ended, "the 429 closed too");

    let waiting = format!(
        "GET /api/v1/waiting HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\n\r\n"
    );
    let (answers, ended) = on_one_connection(&mut port, &[&waiting, &waiting]);
    for (status, head, body) in &answers {
        assert_eq!(*status, 200, "{body}");
        assert_eq!(header(head, "connection"), Some("keep-alive"));
    }
    assert!(!ended, "a paired connection stays open");
}

/// A pinned TLS connection that has said nothing yet, for a test that
/// writes raw bytes.
fn stranger_tls(
    addr: SocketAddr,
    config: &Arc<ClientConfig>,
) -> StreamOwned<ClientConnection, TcpStream> {
    let tcp = TcpStream::connect(addr).expect("connect");
    let conn = ClientConnection::new(
        Arc::clone(config),
        ServerName::try_from("localhost").unwrap(),
    )
    .expect("client");
    let mut s = StreamOwned::new(conn, tcp);
    while s.conn.is_handshaking() {
        s.conn.complete_io(&mut s.sock).expect("handshake");
    }
    s
}

/// A head that never ends its line is refused once it passes the head
/// bound, well before the read timeout, and gets no reply.
#[test]
fn a_stranger_streaming_a_head_with_no_newline_is_closed_early_with_no_reply() {
    let port = port();
    let mut s = stranger_tls(port.addr, &port.client);
    let started = std::time::Instant::now();
    let chunk = [b'a'; 4 * 1024];
    for _ in 0..16 {
        // The port may close while this is still writing.
        if s.write_all(&chunk).and_then(|()| s.flush()).is_err() {
            break;
        }
    }
    s.sock
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut buf = [0u8; 64];
    let read = s.read(&mut buf);
    let elapsed = started.elapsed();
    match read {
        Ok(n) => assert_eq!(n, 0, "no reply: {:?}", &buf[..n]),
        Err(e) => assert!(
            !matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ),
            "the port was still reading: {e}"
        ),
    }
    assert!(elapsed < Duration::from_secs(2), "closed after {elapsed:?}");
}

/// A paired device's frame past the inbound cap ends its subscription.
#[test]
fn a_paired_device_sending_an_oversize_frame_is_disconnected() {
    let mut port = port();
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Pad");
    let sub = subscribe(&mut port, &token);
    assert_eq!(port.app.subscriber_count(), 1);
    sub.outbox
        .send(Out::Text("a".repeat(20 * 1024)))
        .expect("socket thread");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while port.app.subscriber_count() > 0 && std::time::Instant::now() < deadline {
        port.app.serve_pending();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(port.app.subscriber_count(), 0, "the subscription ended");
}

// --- conversations and runs

/// A Claude Code session's conversation pages by turn with `next` for
/// the rest, `from=last` is its tail, the last response is the last
/// turn's answer at the conversation's revision, and both are masked;
/// a shell has no conversation.
#[test]
fn a_page_carries_the_head_of_a_tool_result_and_the_turn_alone_carries_it_whole() {
    let mut port = port();
    port.transcripts.set(Some(conversation(3)));
    let (_, _, session) = seed(&mut port, "");
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let t = Some(token.as_str());
    launched(&mut port, &session);
    let (_, page) = get(
        &mut port,
        &format!("/api/v1/sessions/{session}/conversation?from=3"),
        t,
    );
    let activity = &page["body"]["turns"][0]["activity"][0];
    assert_eq!(activity["more"], true, "{activity}");
    assert_eq!(activity["detail"]["result"].as_str().unwrap().len(), 512);
    let (status, whole) = get(
        &mut port,
        &format!("/api/v1/sessions/{session}/conversation/3"),
        t,
    );
    assert_eq!(status, 200, "{whole}");
    assert_eq!(whole["body"]["total"], 3);
    assert_eq!(whole["body"]["turn"]["n"], 3);
    let activity = &whole["body"]["turn"]["activity"][0];
    assert!(activity.get("more").is_none());
    assert_eq!(
        activity["detail"]["result"].as_str().unwrap().len(),
        "# Switchboard\n".len() * 120
    );
    assert_eq!(whole["revision"], page["revision"]);
    let (_, missing) = get(
        &mut port,
        &format!("/api/v1/sessions/{session}/conversation/9"),
        t,
    );
    assert_eq!(missing["body"]["turn"], serde_json::Value::Null);
    let (status, _) = get(
        &mut port,
        &format!("/api/v1/sessions/{session}/conversation/x"),
        t,
    );
    assert_eq!(status, 400);
}

#[test]
fn a_conversation_page_anchored_before_a_held_turn_ends_there() {
    let mut port = port();
    port.transcripts.set(Some(conversation(3)));
    let (_, _, session) = seed(&mut port, "");
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let t = Some(token.as_str());
    launched(&mut port, &session);
    let (_, before) = get(
        &mut port,
        &format!("/api/v1/sessions/{session}/conversation?before=3&limit=1"),
        t,
    );
    assert_eq!(before["body"]["from"], 2, "the page ending before turn 3");
    assert_eq!(before["body"]["next"], 3);
    assert_eq!(before["body"]["turns"][0]["n"], 2);
    let (status, _) = get(
        &mut port,
        &format!("/api/v1/sessions/{session}/conversation?from=1&before=3"),
        t,
    );
    assert_eq!(status, 400, "one anchor per page");
}

#[test]
fn a_conversation_pages_by_turn_and_its_last_response_shares_the_revision() {
    let mut port = port();
    port.transcripts.set(Some(conversation(3)));
    let (_, project, session) = seed(&mut port, "");
    let pid = switchboard::core::ProjectId(uuid::Uuid::parse_str(&project).unwrap());
    port.secrets.state().insert(
        switchboard::core::SecretScope::Project(pid).account("API_TOKEN"),
        "tok-5e3cr3t".into(),
    );
    port.app.dispatch(AppAction::SetProjectEnv(
        pid,
        switchboard::core::ProjectEnv {
            vars: vec![switchboard::core::EnvVar {
                name: "API_TOKEN".into(),
                value: String::new(),
                secret: true,
            }],
            ..Default::default()
        },
    ));
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let t = Some(token.as_str());

    launched(&mut port, &session);
    let (status, page) = get(
        &mut port,
        &format!("/api/v1/sessions/{session}/conversation?limit=2"),
        t,
    );
    assert_eq!(status, 200, "{page}");
    let body = &page["body"];
    assert_eq!(body["title"], "explain-repo");
    assert_eq!(body["total"], 3);
    assert_eq!(body["from"], 1);
    assert_eq!(body["next"], 3);
    let turns = body["turns"].as_array().unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0]["n"], 1);
    assert_eq!(turns[0]["user"], "prompt 1");
    assert_eq!(turns[1]["final_text"], "answer 2");
    let revision = page["revision"].as_u64().unwrap();

    let (_, rest) = get(
        &mut port,
        &format!("/api/v1/sessions/{session}/conversation?from=3"),
        t,
    );
    assert_eq!(rest["body"]["next"], serde_json::Value::Null);
    assert_eq!(
        rest["body"]["turns"][0]["final_text"],
        "the token is <API_TOKEN>"
    );
    assert_eq!(rest["revision"], revision, "one conversation, one revision");
    let (_, last_page) = get(
        &mut port,
        &format!("/api/v1/sessions/{session}/conversation?from=last&limit=1"),
        t,
    );
    assert_eq!(last_page["body"]["from"], 3);
    assert_eq!(last_page["body"]["turns"][0]["n"], 3, "{last_page}");

    let (status, last) = get(
        &mut port,
        &format!("/api/v1/sessions/{session}/last-response"),
        t,
    );
    assert_eq!(status, 200, "{last}");
    assert_eq!(last["body"]["n"], 3);
    assert_eq!(last["body"]["total"], 3);
    assert_eq!(last["body"]["text"], "the token is <API_TOKEN>");
    assert_eq!(
        last["revision"], revision,
        "a projection of the conversation"
    );
    assert!(last["body"]["resume_id"].is_string());

    let reply = control(
        &mut port,
        Request::new(
            "sh",
            Body::SessionNew {
                project: project.clone(),
                name: "shell".into(),
                session_kind: SessionKind::Shell,
                cwd: PathBuf::from("/tmp"),
                launch: Launch::Shell,
                prompt: None,
                notes: String::new(),
                env: std::collections::BTreeMap::new(),
                env_sets: Vec::new(),
                replaces: None,
            },
        ),
    );
    let shell = made_id(&reply, RecordKind::Session);
    for route in ["last-response", "conversation?from=last&limit=20"] {
        let (status, reply) = get(&mut port, &format!("/api/v1/sessions/{shell}/{route}"), t);
        assert_eq!(status, 400, "{route}");
        assert_eq!(code(&reply), "no_conversation", "{route}");
    }
}

/// A watched conversation moves when the transcript grows, with one
/// `changed {conversation}` the last response follows, and nothing is
/// said while it holds still.
#[test]
fn a_watched_conversation_moves_when_the_transcript_grows() {
    let mut port = port();
    port.transcripts.set(Some(conversation(2)));
    let (_, _, session) = seed(&mut port, "");
    let record = launched(&mut port, &session);
    port.app.set_watch_poll(Duration::from_millis(50));
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let sub = subscribe(&mut port, &token);
    sub.send("watch", "conversation", &session);
    let got = events(&mut port, &sub);
    let moved = resources(&got);
    assert_eq!(
        moved.first(),
        Some(&("watching".into(), "conversation".into(), session.clone())),
        "{moved:?}"
    );
    assert!(
        moved.contains(&("changed".into(), "conversation".into(), session.clone())),
        "{moved:?}"
    );
    assert_eq!(
        port.app.watched_keys(),
        vec![ContentKey::Conversation(record)]
    );
    let first = port
        .app
        .content_revision(&ContentKey::Conversation(record))
        .unwrap();
    let (_, last) = get(
        &mut port,
        &format!("/api/v1/sessions/{session}/last-response"),
        Some(&token),
    );
    assert_eq!(
        (
            last["revision"].as_u64().unwrap(),
            last["body"]["n"].as_u64().unwrap()
        ),
        (first, 2)
    );
    assert!(events(&mut port, &sub).is_empty(), "nothing moved");

    port.transcripts.set(Some(conversation(3)));
    let got = events(&mut port, &sub);
    assert_eq!(
        resources(&got),
        vec![("changed".into(), "conversation".into(), session.clone())]
    );
    let next = got[0]["revision"].as_u64().unwrap();
    assert!(next > first);
    let (_, last) = get(
        &mut port,
        &format!("/api/v1/sessions/{session}/last-response"),
        Some(&token),
    );
    assert_eq!(
        (
            last["revision"].as_u64().unwrap(),
            last["body"]["n"].as_u64().unwrap()
        ),
        (next, 3)
    );
}

/// A command's runs are listed from the record, a run's output is the
/// tail of its log with the project's secret masked, a run that never
/// was is refused, and a watched run output moves when the log grows.
#[test]
fn a_runs_output_is_the_logs_masked_tail_and_moves_when_the_log_grows() {
    let mut port = port();
    let (_, project, _) = seed(&mut port, "");
    let pid = switchboard::core::ProjectId(uuid::Uuid::parse_str(&project).unwrap());
    port.secrets.state().insert(
        switchboard::core::SecretScope::Project(pid).account("API_TOKEN"),
        "tok-5e3cr3t".into(),
    );
    port.app.dispatch(AppAction::SetProjectEnv(
        pid,
        switchboard::core::ProjectEnv {
            vars: vec![switchboard::core::EnvVar {
                name: "API_TOKEN".into(),
                value: String::new(),
                secret: true,
            }],
            ..Default::default()
        },
    ));
    let (session, record) = command_with_run(
        &mut port,
        &project,
        "$ make\nAPI_TOKEN=tok-5e3cr3t\nbuilding\n",
    );
    port.app.set_watch_poll(Duration::from_millis(50));
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Pad");
    let t = Some(token.as_str());

    let (status, runs) = get(&mut port, &format!("/api/v1/sessions/{session}/runs"), t);
    assert_eq!(status, 200, "{runs}");
    let list = runs["body"]["runs"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["n"], 1);
    assert!(list[0]["started_ms"].is_u64());
    // The fake host lists no pane, so the reconcile closed the run.
    assert!(list[0]["ended_ms"].is_u64());
    assert_eq!(list[0]["exit"], serde_json::Value::Null);

    let (status, out) = get(
        &mut port,
        &format!("/api/v1/sessions/{session}/runs/1/output?lines=2"),
        t,
    );
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["body"]["n"], 1);
    assert_eq!(out["body"]["text"], "API_TOKEN=<API_TOKEN>\nbuilding");
    let first = out["revision"].as_u64().unwrap();
    let (status, missing) = get(
        &mut port,
        &format!("/api/v1/sessions/{session}/runs/2/output"),
        t,
    );
    assert_eq!(status, 404);
    assert_eq!(code(&missing), "no_run");

    let sub = subscribe(&mut port, &token);
    let key = ContentKey::RunOutput(record, 1);
    sub.send("watch", "run_output", &key.id());
    let got = resources(&events(&mut port, &sub));
    assert_eq!(
        got.first(),
        Some(&("watching".into(), "run_output".into(), key.id())),
        "{got:?}"
    );
    assert!(events(&mut port, &sub).is_empty(), "the log held still");
    let log = port.app.core().session(record).unwrap().runs[0].log.clone();
    let dir =
        switchboard::adapters::scrollback::scrollback_dir(&fakes::services().store.data_dir());
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join(log))
        .unwrap();
    writeln!(file, "done").unwrap();
    let got = events(&mut port, &sub);
    assert_eq!(
        resources(&got),
        vec![("changed".into(), "run_output".into(), key.id())]
    );
    assert!(got[0]["revision"].as_u64().unwrap() > first);
    let (_, out) = get(
        &mut port,
        &format!("/api/v1/sessions/{session}/runs/1/output?lines=1"),
        t,
    );
    assert_eq!(out["body"]["text"], "done");
}

// --- the web page

/// A reply's status, its headers with lowercased names, and its body.
type Answer = (u16, Vec<(String, String)>, String);

/// One request over a fresh pinned connection with extra header lines:
/// the status, the reply's headers with lowercased names, and the body.
fn http_head(
    addr: SocketAddr,
    config: &Arc<ClientConfig>,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> std::io::Result<Answer> {
    let tcp = TcpStream::connect(addr)?;
    tcp.set_read_timeout(Some(Duration::from_secs(10)))?;
    let conn = ClientConnection::new(
        Arc::clone(config),
        ServerName::try_from("localhost").unwrap(),
    )
    .map_err(std::io::Error::other)?;
    let mut s = StreamOwned::new(conn, tcp);
    while s.conn.is_handshaking() {
        s.conn.complete_io(&mut s.sock)?;
    }
    let mut extra = String::new();
    for (n, v) in headers {
        for part in [n, ": ", v, "\r\n"] {
            extra.push_str(part);
        }
    }
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{extra}Content-Length: {}\r\n\r\n{body}",
        body.len()
    )?;
    s.flush()?;
    read_answer(&mut BufReader::new(s))
}

/// One reply off a connection: the status, the headers with lowercased
/// names, and the body.
fn read_answer<R: Read>(reader: &mut BufReader<R>) -> std::io::Result<Answer> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let status: u16 = line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, line.clone()))?;
    let mut head = Vec::new();
    loop {
        let mut h = String::new();
        reader.read_line(&mut h)?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((n, v)) = h.split_once(':') {
            head.push((n.trim().to_ascii_lowercase(), v.trim().to_owned()));
        }
    }
    let length = header(&head, "content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut out = vec![0; length];
    reader.read_exact(&mut out)?;
    Ok((status, head, String::from_utf8_lossy(&out).into_owned()))
}

/// `http_head` from a client thread, served by the app on this thread.
fn call_head(
    port: &mut Port,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (u16, Vec<(String, String)>, String) {
    let addr = port.addr;
    let config = Arc::clone(&port.client);
    let headers: Vec<(String, String)> = headers
        .iter()
        .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
        .collect();
    let (method, path, body) = (method.to_owned(), path.to_owned(), body.to_owned());
    let client = std::thread::spawn(move || {
        let headers: Vec<(&str, &str)> = headers
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_str()))
            .collect();
        http_head(addr, &config, &method, &path, &headers, &body)
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !client.is_finished() {
        assert!(std::time::Instant::now() < deadline, "no reply");
        port.app.serve_pending();
        std::thread::sleep(Duration::from_millis(2));
    }
    client.join().unwrap().expect("http")
}

fn header<'a>(head: &'a [(String, String)], name: &str) -> Option<&'a str> {
    head.iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

/// Open the pairing window with `code` and pair as a browser does,
/// asking for the cookie: the `Set-Cookie` value and the reply body.
fn pair_cookie(port: &mut Port, code: &str, scope: Scope) -> (String, serde_json::Value) {
    port.app.dispatch(AppAction::OpenPairing {
        code: code.into(),
        scope,
    });
    let spki = port
        .remote
        .state()
        .identity
        .as_ref()
        .unwrap()
        .spki_sha256
        .clone();
    let body = serde_json::json!({
        "code": code,
        "device_name": "Safari on iPhone",
        "spki_sha256": spki,
        "transport": "cookie",
    });
    let (status, head, text) = call_head(port, "POST", "/api/v1/pair", &[], &body.to_string());
    assert_eq!(status, 200, "{text}");
    let cookie = header(&head, "set-cookie")
        .expect("a cookie was set")
        .to_owned();
    (cookie, json(&text))
}

/// `name=value` from a `Set-Cookie` value, as a browser sends it back.
fn cookie_pair(set_cookie: &str) -> String {
    set_cookie.split(';').next().unwrap().trim().to_owned()
}

const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// The page's three files are answered to anyone, with the headers
/// that hold them to themselves, and the connection closes after each;
/// anything else outside the API is the JSON 404 as before, and a
/// query on a page path is never echoed.
#[test]
fn the_page_is_served_to_anyone_with_its_headers() {
    let mut port = port();
    for (path, kind) in [
        ("/", "text/html"),
        ("/app.js", "text/javascript"),
        ("/app.css", "text/css"),
    ] {
        let (status, head, body) = call_head(&mut port, "GET", path, &[], "");
        assert_eq!(status, 200, "{path}");
        assert!(
            header(&head, "content-type").is_some_and(|v| v.starts_with(kind)),
            "{path}: {head:?}"
        );
        assert_eq!(header(&head, "content-security-policy"), Some(CSP));
        assert_eq!(header(&head, "cache-control"), Some("no-store"));
        assert_eq!(header(&head, "x-content-type-options"), Some("nosniff"));
        assert_eq!(header(&head, "referrer-policy"), Some("no-referrer"));
        assert_eq!(header(&head, "connection"), Some("close"));
        let page = switchboard::adapters::web::page("GET", path).unwrap();
        assert_eq!(body, page.body);
    }
    let (status, _, body) = call_head(&mut port, "GET", "/?code=QUERY2345", &[], "");
    assert_eq!(status, 200);
    assert!(!body.contains("QUERY2345"), "a query is not echoed");

    for (method, path) in [("GET", "/favicon.ico"), ("POST", "/"), ("GET", "/api")] {
        let (status, head, body) = call_head(&mut port, method, path, &[], "");
        assert_eq!(status, 404, "{method} {path}");
        assert_eq!(code(&json(&body)), "not_found");
        assert_eq!(header(&head, "content-type"), Some("application/json"));
    }
    // Every JSON reply tells a browser neither to keep it nor to read
    // it as anything else.
    let (_, head, _) = call_head(&mut port, "GET", "/api/v1/info", &[], "");
    assert_eq!(header(&head, "cache-control"), Some("no-store"));
    assert_eq!(header(&head, "x-content-type-options"), Some("nosniff"));
    // A stranger's reply ends its connection.
    assert_eq!(header(&head, "connection"), Some("close"));
}

/// Every route in the page's table is one the host serves: for a
/// paired device none answers `not_found`. And no `/api/v1/` string in
/// the script is outside the table, so the check covers them all.
#[test]
fn the_page_calls_only_routes_the_host_serves() {
    let mut port = port();
    let (_, project, session) = seed(&mut port, "");
    let token = pair(&mut port, "ABCD2345", Scope::Control, "Pad");
    let script = include_str!("../web/app.js");
    let mut routes = Vec::new();
    for method in ["GET", "POST"] {
        let quoted = format!("'{method} /api/v1/");
        for (at, _) in script.match_indices(&quoted) {
            let entry = &script[at + 1..];
            let entry = &entry[..entry.find('\'').unwrap()];
            let (_, path) = entry.split_once(' ').unwrap();
            routes.push((method.to_owned(), path.to_owned()));
        }
    }
    assert_eq!(
        routes.len(),
        script.matches("/api/v1/").count(),
        "every API path in app.js is in its table"
    );
    assert!(routes.len() >= 10, "{routes:?}");
    for (method, path) in routes {
        let id = if path.starts_with("/api/v1/projects/") {
            &project
        } else {
            &session
        };
        let path = path.replace("{id}", id);
        assert!(
            !path.contains('{'),
            "{path}: a placeholder this test does not fill"
        );
        let (status, reply) = call(&mut port, &method, &path, Some(&token), "{}");
        assert_ne!(
            code(&reply),
            "not_found",
            "{method} {path}: {status} {reply}"
        );
    }
}

/// A browser pairs as the command line does, `/info` then `/pair`, but
/// asks for a cookie: it gets one named for the host that its script
/// cannot read, and a body with no token. The cookie alone reads `/me`.
/// A native pairing is as it was.
#[test]
fn a_browser_pairs_into_a_cookie_named_for_the_host() {
    let mut port = port();
    let (status, info) = get(&mut port, "/api/v1/info", None);
    assert_eq!(status, 200);
    let (set_cookie, reply) = pair_cookie(&mut port, "ABCD2345", Scope::Read);
    let server_id = reply["body"]["server_id"].as_str().unwrap().to_owned();
    assert_eq!(info["body"]["server_id"], server_id);
    assert!(reply["body"].get("token").is_none(), "{reply}");
    assert_eq!(reply["body"]["scope"], "read");
    let attributes: Vec<&str> = set_cookie.split(';').map(str::trim).collect();
    assert!(attributes[0].starts_with(&format!("__Host-sb-{server_id}=")));
    for attribute in ["HttpOnly", "Secure", "SameSite=Strict", "Path=/"] {
        assert!(attributes.contains(&attribute), "{set_cookie}");
    }
    assert!(!set_cookie.contains("Domain"), "{set_cookie}");
    assert_eq!(port.app.core().devices()[0].name, "Safari on iPhone");

    let cookie = cookie_pair(&set_cookie);
    let (status, _, text) = call_head(&mut port, "GET", "/api/v1/me", &[("Cookie", &cookie)], "");
    assert_eq!(status, 200, "{text}");
    let me = json(&text);
    assert_eq!(me["body"]["device_id"], reply["body"]["device_id"]);
    assert_eq!(me["body"]["name"], "Safari on iPhone");
    assert_eq!(me["body"]["scope"], "read");
    assert_eq!(me["body"]["server_id"], server_id);
    assert_eq!(me["body"]["server_name"], info["body"]["server_name"]);
    // Among other cookies, it is still found by its exact name.
    let (status, _, _) = call_head(
        &mut port,
        "GET",
        "/api/v1/waiting",
        &[("Cookie", &format!("theme=dark; {cookie}; other=1"))],
        "",
    );
    assert_eq!(status, 200);

    // A native client's pairing is unchanged: the token in the body.
    port.app.dispatch(AppAction::OpenPairing {
        code: "WXYZ6789".into(),
        scope: Scope::Read,
    });
    let spki = info["body"]["spki_sha256"].as_str().unwrap();
    let body = serde_json::json!({ "code": "WXYZ6789", "device_name": "Pad", "spki_sha256": spki });
    let (status, head, text) = call_head(&mut port, "POST", "/api/v1/pair", &[], &body.to_string());
    assert_eq!(status, 200);
    assert!(header(&head, "set-cookie").is_none());
    assert_eq!(
        json(&text)["body"]["token"].as_str().map(str::len),
        Some(43)
    );
}

/// Two hosts on one address share a browser's cookie jar, since
/// cookies ignore ports. Each names its cookie for itself, reads only
/// its own, and a request carrying both is each host's own device; a
/// cookie under the bare prefix authenticates nothing.
#[test]
fn two_hosts_on_one_address_each_read_only_their_own_cookie() {
    let mut a = port();
    let mut b = port();
    let (set_a, reply_a) = pair_cookie(&mut a, "ABCD2345", Scope::Read);
    let (set_b, reply_b) = pair_cookie(&mut b, "ABCD2345", Scope::Read);
    let (cookie_a, cookie_b) = (cookie_pair(&set_a), cookie_pair(&set_b));
    let name = |c: &str| c.split('=').next().unwrap().to_owned();
    assert_ne!(name(&cookie_a), name(&cookie_b));
    let both = format!("{cookie_a}; {cookie_b}");
    for (port, reply) in [(&mut a, &reply_a), (&mut b, &reply_b)] {
        let (status, _, text) = call_head(port, "GET", "/api/v1/me", &[("Cookie", &both)], "");
        assert_eq!(status, 200, "{text}");
        assert_eq!(json(&text)["body"]["device_id"], reply["body"]["device_id"]);
    }
    let (status, _, text) = call_head(&mut a, "GET", "/api/v1/me", &[("Cookie", &cookie_b)], "");
    assert_eq!((status, code(&json(&text))), (401, "unauthorized"));
    let token_a = cookie_a.split_once('=').unwrap().1;
    let bare = format!("__Host-sb={token_a}");
    let (status, _, _) = call_head(&mut a, "GET", "/api/v1/me", &[("Cookie", &bare)], "");
    assert_eq!(status, 401, "only the host's own name is read");
}

/// A cookie rides every request a browser makes to the address, so a
/// cookie-only request that changes something must come from the page
/// this port served; a bearer request needs no `Origin`.
#[test]
fn a_cookie_write_or_upgrade_needs_the_pages_origin() {
    let mut port = port();
    let (_, _, session) = seed(&mut port, "");
    let (set_cookie, _) = pair_cookie(&mut port, "ABCD2345", Scope::Control);
    let cookie = cookie_pair(&set_cookie);
    let path = format!("/api/v1/sessions/{session}/send");
    let body = r#"{"op":"web-1","text":"hello"}"#;
    for origin in [None, Some("https://evil.example"), Some("http://localhost")] {
        let mut headers = vec![("Cookie", cookie.as_str())];
        if let Some(origin) = origin {
            headers.push(("Origin", origin));
        }
        let (status, _, text) = call_head(&mut port, "POST", &path, &headers, body);
        assert_eq!((status, code(&json(&text))), (403, "origin"), "{origin:?}");
    }
    let headers = [("Cookie", cookie.as_str()), ("Origin", "https://localhost")];
    let (status, _, text) = call_head(&mut port, "POST", &path, &headers, body);
    assert_ne!(code(&json(&text)), "origin", "{status} {text}");
    assert_ne!(status, 401);
    // A read by cookie needs none.
    let (status, _, _) = call_head(
        &mut port,
        "GET",
        "/api/v1/waiting",
        &[("Cookie", &cookie)],
        "",
    );
    assert_eq!(status, 200);

    let token = pair(&mut port, "WXYZ6789", Scope::Control, "Pad");
    let (status, reply) = call(
        &mut port,
        "POST",
        &path,
        Some(&token),
        r#"{"op":"native-1","text":"hi"}"#,
    );
    assert_ne!(code(&reply), "origin", "{status} {reply}");

    let with = |origin: &str| {
        vec![
            ("Cookie".to_owned(), cookie.clone()),
            ("Origin".to_owned(), origin.to_owned()),
        ]
    };
    assert!(
        subscribe_with(&mut port, with("https://evil.example"), "/api/v1/events").is_err(),
        "a foreign page's upgrade is refused"
    );
    assert!(
        subscribe_with(&mut port, with("https://localhost"), "/api/v1/events").is_ok(),
        "the page's own upgrade gets hello"
    );
}

/// `/me` says the device's scope as the dialog sets it; after a revoke
/// the cookie is a plain 401 while `/info` still names the same host
/// and key, which is how the page tells "removed" from "changed".
#[test]
fn me_follows_the_dialog_and_a_revoked_cookie_leaves_info_unchanged() {
    let mut port = port();
    let (_, before) = get(&mut port, "/api/v1/info", None);
    let (set_cookie, _) = pair_cookie(&mut port, "ABCD2345", Scope::Read);
    let cookie = cookie_pair(&set_cookie);
    let me = |port: &mut Port| {
        let (status, _, text) = call_head(port, "GET", "/api/v1/me", &[("Cookie", &cookie)], "");
        (status, json(&text))
    };
    assert_eq!(me(&mut port).1["body"]["scope"], "read");
    let id = port.app.core().devices()[0].id;
    port.app.dispatch(AppAction::SetDeviceScope {
        device: id,
        scope: Scope::Control,
    });
    assert_eq!(me(&mut port).1["body"]["scope"], "control");
    port.app.dispatch(AppAction::RevokeDevice(id));
    let (status, reply) = me(&mut port);
    assert_eq!((status, code(&reply)), (401, "unauthorized"));
    let (_, after) = get(&mut port, "/api/v1/info", None);
    assert_eq!(after["body"]["server_id"], before["body"]["server_id"]);
    assert_eq!(after["body"]["spki_sha256"], before["body"]["spki_sha256"]);
}

/// A page load closes its connection, so it frees its stranger's slot
/// at once rather than at a deadline: four loads from one address,
/// more than its share, whose clients keep their sockets still leave
/// room for a fifth.
#[test]
fn a_page_load_frees_its_slot_at_once() {
    let port = port();
    let mut held = Vec::new();
    for _ in 0..MAX_UNAUTHENTICATED {
        let tcp = TcpStream::connect(port.addr).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let conn = ClientConnection::new(
            Arc::clone(&port.client),
            ServerName::try_from("localhost").unwrap(),
        )
        .unwrap();
        let mut s = StreamOwned::new(conn, tcp);
        write!(s, "GET /app.css HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        s.flush().unwrap();
        let mut all = Vec::new();
        s.read_to_end(&mut all)
            .expect("the host closed after the page");
        assert!(String::from_utf8_lossy(&all).starts_with("HTTP/1.1 200"));
        held.push(s);
    }
    std::thread::sleep(Duration::from_millis(50));
    let (status, _, _) = http_head(port.addr, &port.client, "GET", "/", &[], "").unwrap();
    assert_eq!(status, 200, "the fifth load found a slot");
}

/// The page has a window of its own: the 121st file in a minute is
/// 429, and `/info` and `/pair` keep their own windows.
#[test]
fn page_requests_have_their_own_rate_window() {
    let mut port = port();
    for i in 0..120 {
        let (status, _, _) =
            http_head(port.addr, &port.client, "GET", "/app.css", &[], "").unwrap();
        assert_eq!(status, 200, "request {i}");
    }
    let (status, _, body) = http_head(port.addr, &port.client, "GET", "/", &[], "").unwrap();
    assert_eq!((status, code(&json(&body))), (429, "rate_limited"));
    let (status, _) = get(&mut port, "/api/v1/info", None);
    assert_eq!(status, 200);
    let (status, reply) = call(&mut port, "POST", "/api/v1/pair", None, "{}");
    assert_eq!((status, code(&reply)), (403, "pairing_closed"));
}

/// The page in headless Chromium against this host: a browser that
/// refuses the cookie, pairing from the invite, the list and a session
/// at phone width, a reply kept across a poll, one polling chain across
/// tab switches, a send typed once, and the identity-changed and
/// removed notices. `tests/browser/page.mjs` is the browser half; this
/// side pumps the host and does each step's part when the script asks.
/// Not run by `cargo test`: it needs node and a Chromium download.
///
/// ```sh
/// cd tests/browser && npm ci && npx playwright install chromium
/// cargo test --locked --test remote -- --ignored the_page_works_in_a_real_browser --nocapture
/// ```
#[test]
#[ignore = "drives headless Chromium; see the doc comment"]
fn the_page_works_in_a_real_browser() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/browser");
    assert!(
        dir.join("node_modules").is_dir(),
        "the browser test needs its driver installed:\n  \
         cd tests/browser && npm ci && npx playwright install chromium\n  \
         cargo test --locked --test remote -- --ignored the_page_works_in_a_real_browser --nocapture"
    );
    let mut port = port();
    port.transcripts.set(Some(conversation(3)));
    let (_, _, session) = seed(&mut port, "");
    let record = launched(&mut port, &session);
    running(&mut port, &session, "\u{276f} ");
    // A session that has reported no turn refuses a reply.
    hook(&mut port, record, stopped());

    let mut child = Browser(
        std::process::Command::new("node")
            .arg("page.mjs")
            .current_dir(&dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("node"),
    );
    let mut stdin = child.0.stdin.take().unwrap();
    let stdout = child.0.stdout.take().unwrap();
    let mut stderr = child.0.stderr.take().unwrap();
    let (tx, lines) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });

    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    let mut done = false;
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "the browser ran out of time"
        );
        port.app.serve_pending();
        let line = match lines.recv_timeout(Duration::from_millis(2)) {
            Ok(line) => line,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let Some(step) = line.strip_prefix("STEP ") else {
            if line == "DONE" {
                done = true;
            }
            println!("{line}");
            continue;
        };
        let answer = browser_step(&mut port, step, record);
        writeln!(stdin, "{answer}").unwrap();
        stdin.flush().unwrap();
    }
    let status = child.0.wait().unwrap();
    let errors = errors.join().unwrap();
    assert!(status.success() && done, "{status}: {errors}");
}

/// The node child, killed if the test panics first.
struct Browser(std::process::Child);

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The host's part of one step of `tests/browser/page.mjs`: its answer.
fn browser_step(port: &mut Port, step: &str, record: RecordId) -> String {
    let invite = |port: &mut Port, code: &str| {
        port.app.dispatch(AppAction::OpenPairing {
            code: code.into(),
            scope: Scope::Control,
        });
        let identity = port.remote.state().identity.clone().unwrap();
        let window = port.app.core().pairing().unwrap().clone();
        let url = switchboard::core::remote::invite_url(&identity, &window, None, port.addr.port());
        format!("invite {url}")
    };
    let revoke_all = |port: &mut Port| {
        let ids: Vec<_> = port.app.core().devices().iter().map(|d| d.id).collect();
        for id in ids {
            port.app.dispatch(AppAction::RevokeDevice(id));
        }
    };
    match step {
        "refused-cookie" => invite(port, "WXYZ2345"),
        "pair" => {
            // The device the refusing browser paired as holds no cookie.
            assert_eq!(port.app.core().devices().len(), 1, "the refused pairing");
            revoke_all(port);
            invite(port, "ABCD2345")
        }
        "poll-keeps-reply" => {
            port.app
                .dispatch(AppAction::RenameSession(record, "investigator 2".into()));
            port.transcripts.set(Some(conversation(4)));
            port.app.poll_now();
            "go".into()
        }
        "sent" => {
            let written = port.host.state().written.clone();
            let sent = written.iter().filter(|(_, text)| text == "second").count();
            assert_eq!(sent, 1, "{written:?}");
            "go".into()
        }
        "changed-and-removed" => {
            revoke_all(port);
            "go".into()
        }
        _ => "go".into(),
    }
}

/// A set's secret, with a quote in it so a mask after serialization
/// would miss it.
const SET_VALUE: &str = "dk-\"q7Zp\"-9041";
const CHECKS_PATH: &str = "/dispatch/tickets/t1/investigate/1/root/checks.log";
const PROBE_PATH: &str = "/dispatch/tickets/t1/investigate/1/root/evidence/probe.log";

/// A port whose fixture ticket carries `SET_VALUE` in its body, its
/// checks log, its evidence text and its events, with the `deploy` set
/// holding it as `DEPLOY_KEY` granted to the attempt's session (a
/// record made here, named in the status the app is handed). The
/// runner lists the checks log among the artifacts too, as real
/// Dispatch does. Returns the port and the session.
fn port_with_a_granted_ticket(big: bool) -> (Port, String) {
    if big {
        port_with_a_checks_log_of(800 * 1024)
    } else {
        port_with_ticket_text(
            format!("+ deploy --key {SET_VALUE}\nok\n"),
            format!("Deploy with {SET_VALUE}."),
        )
    }
}

/// The granted ticket with a checks log of `len` bytes.
fn port_with_a_checks_log_of(len: usize) -> (Port, String) {
    port_with_ticket_text("x".repeat(len), format!("Deploy with {SET_VALUE}."))
}

/// The granted ticket with `checks` as its checks log and `body` as
/// its body.
fn port_with_ticket_text(checks: String, body: String) -> (Port, String) {
    use switchboard::ports::dispatch::EventView;
    let secrets = FakeSecrets::default();
    let mut port = port_built(
        FakeHost::default(),
        secrets,
        FakeRemoteStore::default(),
        FakeDispatch {
            status: Some(tickets_status()),
            artifacts: [
                (NOTES_PATH.into(), "Found it: one file each.\n".to_owned()),
                (CHECKS_PATH.into(), checks),
                (PROBE_PATH.into(), format!("probe saw {SET_VALUE}\n")),
            ]
            .into_iter()
            .collect(),
            events: vec![EventView {
                seq: 1,
                at_ms: 3,
                stage: "investigate".into(),
                kind: "stage".into(),
                text: format!("ran with {SET_VALUE}"),
                ..EventView::default()
            }],
            ..FakeDispatch::default()
        },
    );
    let (_, project, _) = seed(&mut port, "");
    port.app.dispatch(AppAction::SetEnvSetup { open: true });
    for (op, body) in [
        (
            "up",
            Body::EnvSetUpsert {
                name: "deploy".into(),
                vars: Vec::new(),
                aws: None,
            },
        ),
        (
            "sec",
            Body::EnvSecretStore {
                set: "deploy".into(),
                name: "DEPLOY_KEY".into(),
                value: SET_VALUE.into(),
            },
        ),
    ] {
        let reply = control(&mut port, Request::new(op, body));
        assert!(matches!(reply, Reply::Persisted { .. }), "{op}: {reply:?}");
    }
    let reply = control(
        &mut port,
        Request::new(
            "se",
            Body::SessionNew {
                project,
                name: "implementer".into(),
                session_kind: SessionKind::Shell,
                cwd: PathBuf::from("/tmp"),
                launch: Launch::Shell,
                prompt: None,
                notes: String::new(),
                env: std::collections::BTreeMap::new(),
                env_sets: Vec::new(),
                replaces: None,
            },
        ),
    );
    let session = made_id(&reply, RecordKind::Session);
    let reply = control(
        &mut port,
        Request::new(
            "gr",
            Body::EnvGrant {
                project: None,
                session: Some(session.clone()),
                runner: false,
                set: "deploy".into(),
                remove: false,
            },
        ),
    );
    assert!(matches!(reply, Reply::Persisted { .. }), "{reply:?}");
    let mut status = tickets_status();
    let t1 = &mut status.tickets[1];
    t1.body = body;
    t1.attempts[0].session = Some(session.clone());
    t1.attempts[0]
        .artifacts
        .push(("checks".into(), CHECKS_PATH.into()));
    port.app.dispatch(AppAction::DispatchStatus(Some(status)));
    (port, session)
}

/// The key the listing gives the file named `name` on t1's attempt.
fn file_key_named(port: &mut Port, token: &str, name: &str) -> String {
    let (status, reply) = get(port, "/api/v1/dispatch/tickets/t1", Some(token));
    assert_eq!(status, 200, "{reply}");
    reply["body"]["attempts"][0]["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .and_then(|f| f["key"].as_str())
        .unwrap_or_else(|| panic!("no {name}: {reply}"))
        .to_owned()
}

#[test]
fn a_tickets_text_reaches_a_device_with_granted_values_named() {
    let (mut port, session) = port_with_a_granted_ticket(false);
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let t = Some(token.as_str());

    // The listing names the log once, though the runner lists it twice.
    let (status, reply) = get(&mut port, "/api/v1/dispatch/tickets/t1", t);
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["body"], "Deploy with <DEPLOY_KEY>.");
    assert!(!reply.to_string().contains("q7Zp"), "{reply}");
    let names: Vec<&str> = reply["body"]["attempts"][0]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["notes", "token", "checks", "probe.log", "shot.png"]);

    // The checks log and the evidence text, first through the runner's
    // read and then from the Mac's cache.
    let checks = file_key_named(&mut port, &token, "checks");
    let probe = file_key_named(&mut port, &token, "probe.log");
    for _ in 0..2 {
        let (status, reply) = get(
            &mut port,
            &format!("/api/v1/dispatch/tickets/t1/files/{checks}"),
            t,
        );
        assert_eq!(status, 200, "{reply}");
        assert_eq!(reply["body"]["text"], "+ deploy --key <DEPLOY_KEY>\nok\n");
        let (status, reply) = get(
            &mut port,
            &format!("/api/v1/dispatch/tickets/t1/files/{probe}"),
            t,
        );
        assert_eq!(status, 200, "{reply}");
        assert_eq!(reply["body"]["text"], "probe saw <DEPLOY_KEY>\n");
    }

    // The events, read and then from the cache, and the list.
    for _ in 0..2 {
        let (status, reply) = get(&mut port, "/api/v1/dispatch/tickets/t1/events", t);
        assert_eq!(status, 200, "{reply}");
        assert_eq!(reply["body"]["events"][0]["text"], "ran with <DEPLOY_KEY>");
    }
    let (status, reply) = get(&mut port, "/api/v1/dispatch/tickets", t);
    assert_eq!(status, 200, "{reply}");
    assert!(!reply.to_string().contains("q7Zp"), "{reply}");

    // A grant taken back leaves the value masked: text printed while
    // it was granted may still be on the port.
    let reply = control(
        &mut port,
        Request::new(
            "rm",
            Body::EnvGrant {
                project: None,
                session: Some(session.clone()),
                runner: false,
                set: "deploy".into(),
                remove: true,
            },
        ),
    );
    assert!(matches!(reply, Reply::Persisted { .. }), "{reply:?}");
    let (_, reply) = get(&mut port, "/api/v1/dispatch/tickets/t1", t);
    assert_eq!(reply["body"]["body"], "Deploy with <DEPLOY_KEY>.");

    // A store that cannot read the set's secret refuses the read.
    let reply = control(
        &mut port,
        Request::new(
            "re",
            Body::EnvGrant {
                project: None,
                session: Some(session),
                runner: false,
                set: "deploy".into(),
                remove: false,
            },
        ),
    );
    assert!(matches!(reply, Reply::Persisted { .. }), "{reply:?}");
    port.secrets.set_failing(true);
    let (status, reply) = get(
        &mut port,
        &format!("/api/v1/dispatch/tickets/t1/files/{checks}"),
        t,
    );
    assert_eq!((status, code(&reply)), (503, "policy"), "{reply}");
    assert!(!reply.to_string().contains("q7Zp"), "{reply}");
}

#[test]
fn a_tickets_file_over_the_cap_is_refused_whole() {
    let (mut port, _) = port_with_a_granted_ticket(true);
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let checks = file_key_named(&mut port, &token, "checks");
    let (status, reply) = get(
        &mut port,
        &format!("/api/v1/dispatch/tickets/t1/files/{checks}"),
        Some(&token),
    );
    assert_eq!((status, code(&reply)), (413, "too_large"), "{reply}");
    let reason = reply["error"]["message"].as_str().unwrap_or_default();
    assert!(reason.contains(&(800 * 1024).to_string()), "{reply}");
}

/// The checks log of `len` bytes as a device reads it.
fn checks_log_read(len: usize) -> (u16, serde_json::Value) {
    let (mut port, _) = port_with_a_checks_log_of(len);
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let checks = file_key_named(&mut port, &token, "checks");
    get(
        &mut port,
        &format!("/api/v1/dispatch/tickets/t1/files/{checks}"),
        Some(&token),
    )
}

#[test]
fn a_tickets_file_under_the_file_cap_reaches_a_device_whole() {
    let (status, reply) = checks_log_read(200 * 1024);
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["text"].as_str().unwrap().len(), 200 * 1024);
    assert!(
        reply["body"]["truncated"].is_null(),
        "{}",
        reply["body"]["truncated"]
    );
}

#[test]
fn a_tickets_file_past_the_file_cap_is_cut_and_says_so() {
    let (status, reply) = checks_log_read(300 * 1024);
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["text"].as_str().unwrap().len(), 256 * 1024);
    assert_eq!(reply["body"]["truncated"], serde_json::json!(["text"]));
}

#[test]
fn a_secret_across_the_file_cap_is_masked_before_the_cut() {
    // Unmasked, the cut would keep `dk-"q7Zp`.
    let before = 256 * 1024 - 8;
    let checks = format!(
        "{}{SET_VALUE}{}",
        "a".repeat(before),
        "b".repeat(300 * 1024 - before - SET_VALUE.len())
    );
    let (mut port, _) = port_with_ticket_text(checks, "Short.".into());
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let key = file_key_named(&mut port, &token, "checks");
    let (status, reply) = get(
        &mut port,
        &format!("/api/v1/dispatch/tickets/t1/files/{key}"),
        Some(&token),
    );
    assert_eq!(status, 200, "{reply}");
    let text = reply["body"]["text"].as_str().unwrap();
    assert_eq!(text.len(), 256 * 1024);
    assert!(text.ends_with("<DEPLOY_"), "{}", &text[text.len() - 16..]);
    assert!(!reply.to_string().contains("q7Zp"));
    assert_eq!(reply["body"]["truncated"], serde_json::json!(["text"]));
}

#[test]
fn a_tickets_long_body_reaches_a_device_whole() {
    let body = "w".repeat(20 * 1024);
    let (mut port, _) = port_with_ticket_text("ok\n".into(), body.clone());
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let (status, reply) = get(&mut port, "/api/v1/dispatch/tickets/t1", Some(&token));
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["body"], body.as_str());
    assert!(
        reply["body"]["truncated"].is_null(),
        "{}",
        reply["body"]["truncated"]
    );
}

/// A tester's screenshots reach the device whether or not the try ran
/// with grants (the client pipeline's always does); the attempt says
/// so, and a client labels them rather than losing them.
#[test]
fn an_image_of_an_attempt_that_ran_with_grants_is_served_and_the_attempt_says_so() {
    let mut status = tickets_status();
    status.tickets[1].attempts[0].checks.as_mut().unwrap().env = true;
    let mut port = port_dispatching(status);
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let shot = file_key_named(&mut port, &token, "shot.png");
    let read = format!("/api/v1/dispatch/tickets/t1/files/{shot}");
    let (status, reply) = get(&mut port, &read, Some(&token));
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["body"]["media"], "image/png");
    let (_, ticket) = get(&mut port, "/api/v1/dispatch/tickets/t1", Some(&token));
    assert_eq!(ticket["body"]["attempts"][0]["env"], true, "{ticket}");
    // The same attempt without grants is unmarked.
    port.app
        .dispatch(AppAction::DispatchStatus(Some(tickets_status())));
    let (_, ticket) = get(&mut port, "/api/v1/dispatch/tickets/t1", Some(&token));
    assert_eq!(ticket["body"]["attempts"][0]["env"], false, "{ticket}");
}

/// A port whose `shot.png` evidence is `bytes` at a path of its own,
/// so tests running in parallel never share the file.
fn port_with_a_shot_of(bytes: &[u8], name: &str) -> Port {
    let mut status = tickets_status();
    let path = format!("/tmp/switchboard-remote-tests/{name}.png");
    std::fs::create_dir_all("/tmp/switchboard-remote-tests").unwrap();
    std::fs::write(&path, bytes).unwrap();
    let shot = status.tickets[1].attempts[0]
        .evidence
        .iter_mut()
        .find(|e| e.rel == "shot.png")
        .unwrap();
    shot.path = path.into();
    port_dispatching(status)
}

/// `len` bytes: a PNG signature and then a pattern that does not repeat
/// soon, so a cut or shifted copy cannot compare equal.
fn shot_bytes(len: usize) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend((0..len - bytes.len()).map(|i| u8::try_from(i * 31 % 251).unwrap()));
    bytes
}

/// The `shot.png` evidence of `bytes` as a device reads it.
fn shot_read(bytes: &[u8], name: &str) -> (u16, serde_json::Value) {
    let mut port = port_with_a_shot_of(bytes, name);
    let token = pair(&mut port, "ABCD2345", Scope::Read, "Vision Pro");
    let shot = file_key_named(&mut port, &token, "shot.png");
    get(
        &mut port,
        &format!("/api/v1/dispatch/tickets/t1/files/{shot}"),
        Some(&token),
    )
}

fn assert_shot_whole(bytes: &[u8], status: u16, reply: &serde_json::Value) {
    use base64::Engine as _;
    assert_eq!(status, 200, "{}", code(reply));
    assert_eq!(reply["body"]["media"], "image/png");
    assert!(
        reply["body"]["truncated"].is_null(),
        "{}",
        reply["body"]["truncated"]
    );
    let data = reply["body"]["data"].as_str().unwrap();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(data)
        .unwrap();
    assert!(decoded == bytes, "decoded {} bytes", decoded.len());
}

#[test]
fn an_evidence_image_of_a_realistic_size_reaches_a_device_whole() {
    let bytes = shot_bytes(200 * 1024);
    let (status, reply) = shot_read(&bytes, "shot-realistic");
    assert_shot_whole(&bytes, status, &reply);
}

#[test]
fn an_evidence_image_at_the_limit_is_served_and_one_byte_over_is_refused() {
    let bytes = shot_bytes(700 * 1024);
    let (status, reply) = shot_read(&bytes, "shot-at-limit");
    assert_shot_whole(&bytes, status, &reply);

    let bytes = shot_bytes(700 * 1024 + 1);
    let (status, reply) = shot_read(&bytes, "shot-over-limit");
    assert_eq!((status, code(&reply)), (413, "too_large"), "{reply}");
}
