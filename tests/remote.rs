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
    self, FakeAdvertiser, FakeHost, FakeOperations, FakeRemoteStore, FakeSecrets, MemoryStore,
};
use switchboard::adapters::remote::{self, pinned_client};
use switchboard::app::Services;
use switchboard::core::{AppAction, RecordId};
use switchboard::ports::host::{HostId, HostStatus, Liveness as HostLiveness};
use switchboard::ports::remote::{Listen, Scope};
use switchboard_control::{Body, Client, Launch, Made, RecordKind, Reply, Request, SessionKind};
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
    let advertiser = FakeAdvertiser::default();
    let services = Services {
        secrets: Box::new(secrets.clone()),
        host: Box::new(host.clone()),
        remote: Box::new(remote.clone()),
        advertiser: Box::new(advertiser.clone()),
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
        advertiser,
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
fn write_fixtures(dir: &std::path::Path) {
    let mut port = port();
    let (space, project, session) = seed(&mut port, "ticket 1\nnotes with two lines");
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
    for (name, value) in &mut files {
        if *name == "pair" {
            // A fixture never carries a real token.
            value["body"]["token"] = serde_json::json!("A".repeat(43));
        }
        let text = serde_json::to_string_pretty(value).unwrap();
        std::fs::write(dir.join(format!("{name}.json")), text + "\n").unwrap();
    }
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
struct Subscriber {
    frames: std::sync::mpsc::Receiver<Result<String, String>>,
    paused: Arc<std::sync::atomic::AtomicBool>,
}

fn subscribe(port: &mut Port, token: &str) -> Subscriber {
    let addr = port.addr;
    let config = Arc::clone(&port.client);
    let token = token.to_owned();
    let (tx, frames) = std::sync::mpsc::channel();
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
        let mut request = "wss://localhost/api/v1/events"
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("Authorization", format!("Bearer {token}").parse().unwrap());
        let (mut ws, _) = match tungstenite::client(request, s) {
            Ok(ok) => ok,
            Err(e) => {
                let _ = tx.send(Err(format!("upgrade refused: {e}")));
                return;
            }
        };
        loop {
            while hold.load(std::sync::atomic::Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
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
                Err(e) => {
                    let _ = tx.send(Err(format!("closed: {e}")));
                    return;
                }
            }
        }
    });
    let subscriber = Subscriber { frames, paused };
    let hello = next_frame(port, &subscriber, 5)
        .expect("hello")
        .expect("a frame");
    let hello: serde_json::Value = serde_json::from_str(&hello).unwrap();
    assert_eq!(hello["type"], "hello", "{hello}");
    subscriber
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
