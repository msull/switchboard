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
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};
use switchboard::SwitchboardApp;
use switchboard::adapters::fakes::{
    self, FakeHost, FakeOperations, FakeRemoteStore, FakeSecrets, MemoryStore,
};
use switchboard::adapters::remote::{self, pinned_client};
use switchboard::app::Services;
use switchboard::core::{AppAction, RecordId};
use switchboard::ports::host::{HostId, HostStatus, Liveness as HostLiveness};
use switchboard::ports::remote::{Listen, Scope};
use switchboard_control::{Body, Client, Launch, Made, RecordKind, Reply, Request, SessionKind};

struct Port {
    app: SwitchboardApp,
    addr: SocketAddr,
    client: Arc<ClientConfig>,
    control: PathBuf,
    host: FakeHost,
    secrets: FakeSecrets,
    remote: FakeRemoteStore,
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
    let services = Services {
        secrets: Box::new(secrets.clone()),
        host: Box::new(host.clone()),
        remote: Box::new(remote.clone()),
        operations: Box::new(FakeOperations::default()),
        ..fakes::services()
    };
    let mut app = SwitchboardApp::with_services(services);
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
        _dir: dir,
    }
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
fn a_pairing_with_another_hosts_key_or_the_wrong_code_counts_and_three_close_the_window() {
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
    assert!(
        port.app.core().pairing().is_none(),
        "three failures close the window"
    );
    // The right code, too late.
    let right = serde_json::json!({ "code": "abcd2345", "device_name": "x", "spki_sha256": spki });
    let (status, reply) = call(&mut port, "POST", "/api/v1/pair", None, &right.to_string());
    assert_eq!((status, code(&reply)), (403, "pairing_closed"), "{reply}");
    assert!(port.remote.state().devices.is_empty());
    assert!(port.app.core().devices().is_empty());
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
        ("POST", format!("/api/v1/sessions/{session}/send")),
        ("POST", format!("/api/v1/sessions/{session}/resume")),
        ("POST", format!("/api/v1/sessions/{session}/trust")),
        ("GET", format!("/api/v1/sessions/{session}/env")),
        ("GET", "/api/v1/env/resolve".into()),
        ("POST", "/api/v1/projects".into()),
        ("DELETE", format!("/api/v1/sessions/{session}")),
        ("GET", format!("/api/v1/projects/{project}/files/README.md")),
        ("GET", "/api/v2/projects".into()),
    ] {
        let (status, reply) = call(&mut port, method, &path, t, r#"{"text":"hi"}"#);
        assert_eq!(
            (status, code(&reply)),
            (404, "not_found"),
            "{method} {path}: {reply}"
        );
    }
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
fn pair_is_rate_limited_per_listener() {
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
    let port = port_with(FakeHost::default(), secrets.clone(), store.clone());
    let fresh = store.state().identity.clone().unwrap();
    assert_eq!(fresh.server_id, old.server_id, "the same installation");
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
