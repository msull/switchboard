//! The control port end to end inside one process: a real socket in a
//! temp directory, the wire crate's client on a thread, the app with
//! fakes answering on its "frame". What another process (Dispatch) relies
//! on is checked here: records carry the op, replies are logged and
//! repeated from the log, the window is left alone, and a crash between
//! the log line and the launch reads as interrupted afterwards.

// Tests assert emptiness with `assert!` throughout.
#![allow(clippy::assert_is_empty)]

use std::path::PathBuf;
use std::time::Duration;

use switchboard::SwitchboardApp;
use switchboard::adapters::fakes::{
    self, FakeHost, FakeOpener, FakeOperations, FakeSecrets, MemoryStore,
};
use switchboard::app::Services;
use switchboard::core::{AppAction, View};
use switchboard::ports::control::OpLine;
use switchboard::ports::host::{HostId, HostStatus, Liveness as HostLiveness};
use switchboard::ports::store::Loaded;
use switchboard_control::{
    AwsMethod, Body, Client, EnvVarView, Launch, Liveness, OpStatus, RecordKind, Reply, Request,
    SessionKind,
};

struct Port {
    app: SwitchboardApp,
    path: PathBuf,
    opener: FakeOpener,
    operations: FakeOperations,
    _dir: tempfile::TempDir,
}

/// A started app listening on a socket of its own. `initial` is what the
/// store loads; the operations log is shared so a test can pre-fill it.
fn port(initial: Loaded, operations: FakeOperations) -> Port {
    port_on(initial, operations, FakeHost::default())
}

/// A port whose host the test keeps a handle to.
fn port_on(initial: Loaded, operations: FakeOperations, host: FakeHost) -> Port {
    port_with(initial, operations, host, FakeSecrets::default())
}

/// A port whose host and secret store the test keeps handles to.
fn port_with(
    initial: Loaded,
    operations: FakeOperations,
    host: FakeHost,
    secrets: FakeSecrets,
) -> Port {
    let opener = FakeOpener::default();
    let services = Services {
        secrets: Box::new(secrets),
        store: Box::new(MemoryStore {
            initial,
            ..MemoryStore::default()
        }),
        host: Box::new(host),
        opener: Box::new(opener.clone()),
        operations: Box::new(operations.clone()),
        ..fakes::services()
    };
    let mut app = SwitchboardApp::with_services(services);
    app.start();
    // A short path: the socket's is capped near 100 bytes.
    let dir = tempfile::Builder::new()
        .prefix("sbc")
        .tempdir_in("/tmp")
        .expect("temp dir");
    let path = app.listen_at(dir.path(), || {}).expect("listen");
    Port {
        app,
        path,
        opener,
        operations,
        _dir: dir,
    }
}

/// One call from a client thread, served by the app on this thread.
fn call(port: &mut Port, request: Request) -> Reply {
    call_with(port, request, SwitchboardApp::serve_pending)
}

/// Send `request` from a client thread and run `turn` on the app until
/// the reply arrives, failing after five seconds.
fn call_with(
    port: &mut Port,
    request: Request,
    mut turn: impl FnMut(&mut SwitchboardApp),
) -> Reply {
    let path = port.path.clone();
    let client = std::thread::spawn(move || {
        let mut client = Client::connect(&path).expect("connect");
        client.call(&request).expect("reply")
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !client.is_finished() {
        assert!(std::time::Instant::now() < deadline, "no reply");
        turn(&mut port.app);
        std::thread::sleep(Duration::from_millis(2));
    }
    client.join().expect("client thread")
}

fn made_id(reply: &Reply, kind: RecordKind) -> String {
    reply
        .made()
        .iter()
        .find(|m| m.kind == kind)
        .unwrap_or_else(|| panic!("no {kind:?} in {reply:?}"))
        .id
        .clone()
}

fn session_new(project: &str, prompt: Option<&str>) -> Body {
    Body::SessionNew {
        project: project.into(),
        name: "investigator".into(),
        session_kind: SessionKind::Claude,
        cwd: PathBuf::from("/tmp"),
        launch: Launch::Shell,
        prompt: prompt.map(str::to_owned),
        notes: "Dispatch ticket 1".into(),
        env: std::collections::BTreeMap::new(),
        env_sets: Vec::new(),
        replaces: None,
    }
}

fn lines_of(operations: &FakeOperations) -> Vec<OpLine> {
    operations.lines.lock().unwrap().clone()
}

#[test]
fn a_space_a_project_and_a_session_are_made_found_and_the_window_is_left_alone() {
    let mut port = port(Loaded::default(), FakeOperations::default());
    port.app.dispatch(AppAction::SetOpenTerminalOnLaunch(true));
    let view = port.app.core().view();
    assert_eq!(view, View::Switchboard);

    let reply = call(
        &mut port,
        Request::new(
            "sp",
            Body::SpaceNew {
                name: "Dispatch · Switchboard".into(),
            },
        ),
    );
    let space = made_id(&reply, RecordKind::Space);
    assert!(matches!(reply, Reply::Persisted { .. }));
    let reply = call(
        &mut port,
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
    let reply = call(
        &mut port,
        Request::new("se", session_new(&project, Some("Investigate issue 1"))),
    );
    assert!(matches!(reply, Reply::Launched { .. }), "{reply:?}");
    let session = made_id(&reply, RecordKind::Session);

    // Found by its op, running, carrying the op and the notes.
    let reply = call(
        &mut port,
        Request::new(
            "q1",
            Body::Find {
                operation: "se".into(),
            },
        ),
    );
    let Reply::Found { records } = reply else {
        panic!("{reply:?}")
    };
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, session);
    assert!(!records[0].removed);
    let view_of = records[0].session.as_ref().expect("a session view");
    assert_eq!(view_of.liveness, Liveness::Running);
    assert_eq!(view_of.op.as_deref(), Some("se"));
    assert_eq!(view_of.notes, "Dispatch ticket 1");
    assert_eq!(view_of.kind, SessionKind::Claude);

    // The queries see the same things.
    let reply = call(
        &mut port,
        Request::new(
            "q2",
            Body::Projects {
                space: Some(space.clone()),
            },
        ),
    );
    assert!(
        matches!(&reply, Reply::Projects { projects } if projects.len() == 1 && projects[0].id == project)
    );
    let reply = call(
        &mut port,
        Request::new(
            "q3",
            Body::Sessions {
                project: project.clone(),
            },
        ),
    );
    assert!(matches!(&reply, Reply::Sessions { sessions } if sessions.len() == 1));

    // Nothing moved on screen and no terminal window opened, though the
    // setting asks for one on every launch.
    assert_eq!(port.app.core().view(), view);
    assert!(port.opener.0.lock().unwrap().terminals.is_empty());
    assert!(port.app.core().notice().is_none());

    // The log has a request line then a reply line per creation, and
    // nothing for the queries.
    let lines = lines_of(&port.operations);
    let ops: Vec<&str> = lines.iter().map(OpLine::op).collect();
    assert_eq!(ops, ["sp", "sp", "pj", "pj", "se", "se"]);
    assert!(
        matches!(&lines[4], OpLine::Requested { kind, ids, .. } if kind == "session.new" && ids == std::slice::from_ref(&session))
    );
    assert!(matches!(&lines[5], OpLine::Replied { .. }));
}

#[test]
fn a_repeated_op_is_answered_from_the_log_and_runs_nothing_again() {
    let mut port = port(Loaded::default(), FakeOperations::default());
    let reply = call(
        &mut port,
        Request::new("sp", Body::SpaceNew { name: "D".into() }),
    );
    let space = made_id(&reply, RecordKind::Space);
    let reply = call(
        &mut port,
        Request::new(
            "pj",
            Body::ProjectAdd {
                space,
                name: "#1".into(),
                root: PathBuf::from("/tmp"),
            },
        ),
    );
    let project = made_id(&reply, RecordKind::Project);
    let first = call(&mut port, Request::new("se", session_new(&project, None)));
    let again = call(&mut port, Request::new("se", session_new(&project, None)));
    assert_eq!(first, again);
    assert_eq!(port.app.core().workspaces()[0].sessions.len(), 1);
    // A non-replayable command is deduplicated the same way: the second
    // send never reaches the pane.
    let session = made_id(&first, RecordKind::Session);
    let sent = call(
        &mut port,
        Request::new(
            "tx",
            Body::SessionSend {
                session: session.clone(),
                text: "hello".into(),
            },
        ),
    );
    assert!(matches!(sent, Reply::Persisted { .. }), "{sent:?}");
    let sent_again = call(
        &mut port,
        Request::new(
            "tx",
            Body::SessionSend {
                session,
                text: "hello".into(),
            },
        ),
    );
    assert_eq!(sent, sent_again);
    let lines = lines_of(&port.operations);
    assert_eq!(
        lines.iter().filter(|l| l.op() == "tx").count(),
        1,
        "{lines:?}"
    );
}

#[test]
fn a_record_removed_in_the_window_is_still_reported_as_made() {
    let mut port = port(Loaded::default(), FakeOperations::default());
    let reply = call(
        &mut port,
        Request::new("sp", Body::SpaceNew { name: "D".into() }),
    );
    let space = made_id(&reply, RecordKind::Space);
    let reply = call(
        &mut port,
        Request::new(
            "pj",
            Body::ProjectAdd {
                space,
                name: "#1".into(),
                root: PathBuf::from("/tmp"),
            },
        ),
    );
    let project = made_id(&reply, RecordKind::Project);
    let reply = call(&mut port, Request::new("se", session_new(&project, None)));
    let session = made_id(&reply, RecordKind::Session);
    // The user removes it from the board.
    port.app
        .dispatch(AppAction::RemoveSession(switchboard::core::RecordId(
            uuid::Uuid::parse_str(&session).unwrap(),
        )));
    let reply = call(
        &mut port,
        Request::new(
            "q",
            Body::Find {
                operation: "se".into(),
            },
        ),
    );
    let Reply::Found { records } = reply else {
        panic!("{reply:?}")
    };
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, session);
    assert!(records[0].removed);
    assert_eq!(records[0].kind, RecordKind::Session);
    assert!(records[0].session.is_none());
    // An op never seen finds nothing.
    let reply = call(
        &mut port,
        Request::new(
            "q2",
            Body::Find {
                operation: "never".into(),
            },
        ),
    );
    assert!(matches!(reply, Reply::Found { records } if records.is_empty()));
}

#[test]
fn op_status_tells_a_lost_reply_from_a_launch_the_app_died_in() {
    let operations = FakeOperations::default();
    let mut port = port(Loaded::default(), operations.clone());
    let reply = call(
        &mut port,
        Request::new("sp", Body::SpaceNew { name: "D".into() }),
    );
    let space = made_id(&reply, RecordKind::Space);
    let reply = call(
        &mut port,
        Request::new(
            "pj",
            Body::ProjectAdd {
                space,
                name: "#1".into(),
                root: PathBuf::from("/tmp"),
            },
        ),
    );
    let project = made_id(&reply, RecordKind::Project);
    let launched = call(&mut port, Request::new("se", session_new(&project, None)));
    let status = |port: &mut Port, op: &str| match call(
        port,
        Request::new(
            "q",
            Body::OpStatus {
                operation: op.into(),
            },
        ),
    ) {
        Reply::OpStatus { status } => status,
        other => panic!("{other:?}"),
    };
    assert_eq!(status(&mut port, "never"), OpStatus::Unknown);
    assert_eq!(
        status(&mut port, "se"),
        OpStatus::Done {
            reply: Box::new(launched)
        }
    );

    // The app dies. Two things can be left behind: a request line with
    // no reply (it died running the command), and a record saved with
    // its launch pending (it died between the save and the spawn).
    let mut workspaces = port.app.core().workspaces().to_vec();
    workspaces[0].sessions[0].pending_launch = true;
    let views = switchboard::core::Views {
        spaces: port.app.core().spaces().to_vec(),
        sets: port.app.core().working_sets().to_vec(),
        ..switchboard::core::Views::default()
    };
    drop(port);
    operations.lines.lock().unwrap().push(OpLine::Requested {
        op: "lost".into(),
        kind: "session.new".into(),
        ids: vec!["x".into()],
        at: std::time::SystemTime::UNIX_EPOCH,
    });
    let mut port = crate::port(
        Loaded {
            workspaces,
            views,
            ..Loaded::default()
        },
        operations,
    );
    assert_eq!(status(&mut port, "lost"), OpStatus::Interrupted);
    assert_eq!(status(&mut port, "se"), OpStatus::Interrupted);
    assert_eq!(status(&mut port, "never"), OpStatus::Unknown);
    // `find` for the lost one names the id from the log as removed:
    // nothing on disk carries it.
    let reply = call(
        &mut port,
        Request::new(
            "q",
            Body::Find {
                operation: "lost".into(),
            },
        ),
    );
    assert!(matches!(&reply, Reply::Found { records } if records.len() == 1 && records[0].removed));
}

/// The global space's fixed id, as the wire spells it.
const GLOBAL: &str = "00000000-0000-0000-0000-000000000002";

/// A session made to replace another takes its card's place on a set;
/// an id that does not parse refuses the request.
#[test]
fn session_new_with_replaces_moves_the_pin() {
    let mut port = port(Loaded::default(), FakeOperations::default());
    let reply = call(
        &mut port,
        Request::new("sp", Body::SpaceNew { name: "s".into() }),
    );
    let space = made_id(&reply, RecordKind::Space);
    let reply = call(
        &mut port,
        Request::new(
            "pj",
            Body::ProjectAdd {
                space: space.clone(),
                name: "p".into(),
                root: PathBuf::from("/tmp"),
            },
        ),
    );
    let project = made_id(&reply, RecordKind::Project);
    let mut sessions = Vec::new();
    for n in ["a", "b"] {
        let reply = call(
            &mut port,
            Request::new(format!("se-{n}"), session_new(&project, None)),
        );
        sessions.push(made_id(&reply, RecordKind::Session));
    }
    let reply = call(
        &mut port,
        Request::new(
            "set",
            Body::SetNew {
                space: space.clone(),
                name: "queue".into(),
            },
        ),
    );
    let set = made_id(&reply, RecordKind::Set);
    let pin = |session: &str, x: u32| switchboard_control::Pin {
        target: switchboard_control::PinTarget::Session {
            session: session.to_owned(),
        },
        rect: switchboard_control::Rect {
            x,
            y: 0,
            w: 10,
            h: 8,
        },
    };
    let items = vec![pin(&sessions[0], 0), pin(&sessions[1], 10)];
    let reply = call(
        &mut port,
        Request::new("sync", Body::SetSync { set, items }),
    );
    assert!(matches!(reply, Reply::Persisted { .. }), "{reply:?}");

    let mut body = session_new(&project, None);
    if let Body::SessionNew { replaces, .. } = &mut body {
        *replaces = Some(sessions[0].clone());
    }
    let reply = call(&mut port, Request::new("se-new", body));
    let new = made_id(&reply, RecordKind::Session);
    let Reply::Sets { sets } = call(&mut port, Request::new("sets", Body::Sets { space })) else {
        panic!("sets");
    };
    assert_eq!(sets[0].items, vec![pin(&new, 0), pin(&sessions[1], 10)]);

    let mut body = session_new(&project, None);
    if let Body::SessionNew { replaces, .. } = &mut body {
        *replaces = Some("not-an-id".into());
    }
    let reply = call(&mut port, Request::new("se-bad", body));
    assert!(matches!(reply, Reply::Failed { .. }), "{reply:?}");
}

#[test]
fn a_global_set_takes_cards_from_two_spaces_and_holds_no_project() {
    let mut port = port(Loaded::default(), FakeOperations::default());
    let mut sessions = Vec::new();
    for n in ["a", "b"] {
        let reply = call(
            &mut port,
            Request::new(format!("sp-{n}"), Body::SpaceNew { name: n.into() }),
        );
        let space = made_id(&reply, RecordKind::Space);
        let reply = call(
            &mut port,
            Request::new(
                format!("pj-{n}"),
                Body::ProjectAdd {
                    space,
                    name: n.into(),
                    root: PathBuf::from("/tmp"),
                },
            ),
        );
        let project = made_id(&reply, RecordKind::Project);
        let reply = call(
            &mut port,
            Request::new(format!("se-{n}"), session_new(&project, None)),
        );
        sessions.push(made_id(&reply, RecordKind::Session));
    }
    let reply = call(
        &mut port,
        Request::new(
            "set",
            Body::SetNew {
                space: GLOBAL.into(),
                name: "everything".into(),
            },
        ),
    );
    let set = made_id(&reply, RecordKind::Set);
    let items = sessions
        .iter()
        .enumerate()
        .map(|(i, session)| switchboard_control::Pin {
            target: switchboard_control::PinTarget::Session {
                session: session.clone(),
            },
            rect: switchboard_control::Rect {
                x: 0,
                y: u32::try_from(i).unwrap() * 8,
                w: 10,
                h: 8,
            },
        })
        .collect();
    let reply = call(
        &mut port,
        Request::new("sync", Body::SetSync { set, items }),
    );
    assert!(matches!(reply, Reply::Persisted { .. }), "{reply:?}");
    let global = &port.app.core().working_sets()[0];
    assert!(global.space.is_global());
    assert_eq!(global.items.len(), 2);

    // A client walking the listed spaces finds the global set too.
    let Reply::Spaces { spaces } = call(&mut port, Request::new("ls", Body::Spaces)) else {
        panic!("spaces");
    };
    let listed: Vec<_> = spaces.iter().filter(|s| s.view).collect();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, GLOBAL);
    assert_eq!(spaces.iter().filter(|s| !s.view).count(), 3);
    let Reply::Sets { sets } = call(
        &mut port,
        Request::new(
            "sets",
            Body::Sets {
                space: listed[0].id.clone(),
            },
        ),
    ) else {
        panic!("sets");
    };
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].items.len(), 2);

    let reply = call(
        &mut port,
        Request::new(
            "pj-g",
            Body::ProjectAdd {
                space: GLOBAL.into(),
                name: "g".into(),
                root: PathBuf::from("/tmp"),
            },
        ),
    );
    assert!(
        matches!(&reply, Reply::Failed { reason } if reason.contains("holds no projects")),
        "{reply:?}"
    );
    assert_eq!(port.app.core().workspaces().len(), 2);
}

#[test]
fn bad_input_is_failed_and_changes_nothing() {
    let mut port = port(Loaded::default(), FakeOperations::default());
    let reply = call(
        &mut port,
        Request::new(
            "a",
            Body::ProjectAdd {
                space: "nope".into(),
                name: "#1".into(),
                root: PathBuf::from("/tmp"),
            },
        ),
    );
    assert!(matches!(&reply, Reply::Failed { reason } if reason.contains("uuid")));
    let reply = call(
        &mut port,
        Request::new(
            "b",
            Body::ProjectAdd {
                space: uuid::Uuid::new_v4().to_string(),
                name: "#1".into(),
                root: PathBuf::from("/tmp"),
            },
        ),
    );
    assert!(matches!(&reply, Reply::Failed { reason } if reason.contains("space")));
    assert!(port.app.core().workspaces().is_empty());
    let reply = call(
        &mut port,
        Request::new(
            "c",
            Body::SessionSend {
                session: uuid::Uuid::new_v4().to_string(),
                text: "hi".into(),
            },
        ),
    );
    assert!(matches!(reply, Reply::Failed { .. }), "{reply:?}");
    let reply = call(
        &mut port,
        Request::new("", Body::SpaceNew { name: "D".into() }),
    );
    assert!(matches!(&reply, Reply::Failed { reason } if reason.contains("op")));
    // A failed command leaves no request line: nothing was made.
    assert!(
        lines_of(&port.operations)
            .iter()
            .all(|l| matches!(l, OpLine::Replied { .. })),
        "{:?}",
        lines_of(&port.operations)
    );
    assert!(
        port.app.core().notice().is_none(),
        "errors went to the asker"
    );
}

#[test]
fn a_read_only_instance_does_not_listen() {
    let services = Services {
        store: Box::new(MemoryStore {
            lock_result: Some(false),
            ..MemoryStore::default()
        }),
        ..fakes::services()
    };
    let mut app = SwitchboardApp::with_services(services);
    app.start();
    let dir = tempfile::Builder::new()
        .prefix("sbc")
        .tempdir_in("/tmp")
        .unwrap();
    assert!(app.listen_at(dir.path(), || {}).is_err());
    assert!(!dir.path().join("control.sock").exists());
}

/// Claude Code's folder trust question comes before any hook, so the
/// app reads it off the pane: the session waits on the user while the
/// question shows, and not once it is gone.
#[test]
fn a_pane_at_claudes_trust_question_waits_on_the_user_until_it_is_answered() {
    let host = FakeHost::default();
    let mut port = port_on(Loaded::default(), FakeOperations::default(), host.clone());
    let reply = call(
        &mut port,
        Request::new("sp", Body::SpaceNew { name: "D".into() }),
    );
    let space = made_id(&reply, RecordKind::Space);
    let reply = call(
        &mut port,
        Request::new(
            "pj",
            Body::ProjectAdd {
                space,
                name: "#1".into(),
                root: PathBuf::from("/tmp"),
            },
        ),
    );
    let project = made_id(&reply, RecordKind::Project);
    let reply = call(&mut port, Request::new("se", session_new(&project, None)));
    let session = made_id(&reply, RecordKind::Session);
    let record = switchboard::core::RecordId(uuid::Uuid::parse_str(&session).unwrap());
    let pane = HostId(record.host_name());
    host.state().statuses.push(HostStatus {
        id: pane.clone(),
        liveness: HostLiveness::Running {
            pid: 7,
            command: "claude".into(),
        },
        cwd: None,
        last_activity: None,
        title: None,
    });
    host.state().snapshots.insert(
        pane.clone(),
        "Quick safety check: Is this a project you created or one you trust?\n\
         \u{276f} No, exit\n  Yes, I trust this folder\n"
            .into(),
    );
    port.app.poll_now();
    let waiting = call(&mut port, Request::new("w1", Body::Waiting));
    let Reply::Waiting { sessions } = waiting else {
        panic!("{waiting:?}");
    };
    assert_eq!(
        sessions.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        vec![session.as_str()],
        "the question is the user's turn"
    );
    host.state().snapshots.insert(pane, "> \n".into());
    port.app.poll_now();
    let waiting = call(&mut port, Request::new("w2", Body::Waiting));
    assert!(
        matches!(&waiting, Reply::Waiting { sessions } if sessions.is_empty()),
        "{waiting:?}"
    );
}

/// `session.screen` reads a running pane's last lines, refuses a
/// session that is not running, and never lets a project secret's value
/// through: Dispatch hands the text to an agent as a tool result.
#[test]
fn a_screen_is_the_panes_tail_with_secret_values_named() {
    let host = FakeHost::default();
    let secrets = FakeSecrets::default();
    let mut port = port_with(
        Loaded::default(),
        FakeOperations::default(),
        host.clone(),
        secrets.clone(),
    );
    let reply = call(
        &mut port,
        Request::new("sp", Body::SpaceNew { name: "D".into() }),
    );
    let space = made_id(&reply, RecordKind::Space);
    let reply = call(
        &mut port,
        Request::new(
            "pj",
            Body::ProjectAdd {
                space,
                name: "#1".into(),
                root: PathBuf::from("/tmp"),
            },
        ),
    );
    let project = made_id(&reply, RecordKind::Project);
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
    secrets.state().insert(
        switchboard::core::SecretScope::Project(pid).account("API_TOKEN"),
        "tok-5e3cr3t".into(),
    );
    let reply = call(&mut port, Request::new("se", session_new(&project, None)));
    let session = made_id(&reply, RecordKind::Session);
    let screen = |port: &mut Port, op: &str, lines: Option<u32>| {
        call(
            port,
            Request::new(
                op,
                Body::SessionScreen {
                    session: session.clone(),
                    lines,
                },
            ),
        )
    };
    // The host lists no pane for it: a session that is not running.
    port.app.poll_now();
    assert_eq!(screen(&mut port, "q0", None), Reply::failed("not running"));

    let record = switchboard::core::RecordId(uuid::Uuid::parse_str(&session).unwrap());
    let pane = HostId(record.host_name());
    host.state().statuses.push(HostStatus {
        id: pane.clone(),
        liveness: HostLiveness::Running {
            pid: 7,
            command: "claude".into(),
        },
        cwd: None,
        last_activity: None,
        title: None,
    });
    host.state().snapshots.insert(
        pane,
        "one\ntwo\n$ env | grep TOKEN\nAPI_TOKEN=tok-5e3cr3t\n\n\n".into(),
    );
    port.app.poll_now();
    let reply = screen(&mut port, "q1", Some(2));
    assert_eq!(
        reply,
        Reply::Screen {
            text: "$ env | grep TOKEN\nAPI_TOKEN=<API_TOKEN>".into()
        }
    );
    let Reply::Screen { text } = screen(&mut port, "q2", None) else {
        panic!("a running session has a screen");
    };
    assert!(text.starts_with("one\ntwo"), "{text}");
    assert!(!text.contains("tok-5e3cr3t"), "{text}");

    // A value the pane wrapped, its first half above the asked-for line.
    host.state().snapshots.insert(
        HostId(record.host_name()),
        "ok\nAPI_TOKEN=tok-5e\n3cr3t\n".into(),
    );
    assert_eq!(
        screen(&mut port, "q3", Some(1)),
        Reply::Screen {
            text: "API_TOKEN=<API_TOKEN>".into()
        }
    );
}

/// eframe calls only `logic` while the window is occluded or minimized,
/// so a request must be answered with no `ui` frame at all.
#[test]
fn served_while_the_window_is_hidden() {
    let mut port = port(Loaded::default(), FakeOperations::default());
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    let reply = call_with(
        &mut port,
        Request::new("q", Body::Projects { space: None }),
        |app| eframe::App::logic(app, &ctx, &mut frame),
    );
    assert!(matches!(reply, Reply::Projects { .. }), "{reply:?}");
}

// --- environment sets: `env.*` over the real socket, and the binary

/// A port with a real operations log in its own temp dir, so a test can
/// read the file every reply line went to.
struct EnvPort {
    port: Port,
    host: FakeHost,
    secrets: FakeSecrets,
    log_dir: tempfile::TempDir,
}

fn env_port() -> EnvPort {
    let host = FakeHost::default();
    let secrets = FakeSecrets::default();
    let log_dir = tempfile::tempdir().expect("log dir");
    let operations = switchboard::adapters::control::OperationsLog::open(log_dir.path())
        .expect("operations log");
    let services = Services {
        secrets: Box::new(secrets.clone()),
        host: Box::new(host.clone()),
        operations: Box::new(operations),
        ..fakes::services()
    };
    let mut app = SwitchboardApp::with_services(services);
    app.start();
    let dir = tempfile::Builder::new()
        .prefix("sbe")
        .tempdir_in("/tmp")
        .expect("temp dir");
    let path = app.listen_at(dir.path(), || {}).expect("listen");
    EnvPort {
        port: Port {
            app,
            path,
            opener: FakeOpener::default(),
            operations: FakeOperations::default(),
            _dir: dir,
        },
        host,
        secrets,
        log_dir,
    }
}

const SECRET: &str = "s3cr3t-v4lue-0042";

/// A shell session launched through the port with the `dev` set granted
/// to it; returns its id and the token its pane was given.
fn granted_session(env: &mut EnvPort) -> (String, String) {
    let port = &mut env.port;
    let space = made_id(
        &call(
            port,
            Request::new("sp", Body::SpaceNew { name: "D".into() }),
        ),
        RecordKind::Space,
    );
    let project = made_id(
        &call(
            port,
            Request::new(
                "pj",
                Body::ProjectAdd {
                    space,
                    name: "#1".into(),
                    root: PathBuf::from("/tmp"),
                },
            ),
        ),
        RecordKind::Project,
    );
    port.app.dispatch(AppAction::SetEnvSetup { open: true });
    for (op, body) in [
        (
            "up",
            Body::EnvSetUpsert {
                name: "dev".into(),
                vars: vec![EnvVarView {
                    name: "REGION".into(),
                    value: "us-east-1".into(),
                    secret: false,
                }],
                aws: None,
            },
        ),
        (
            "sec",
            Body::EnvSecretStore {
                set: "dev".into(),
                name: "API_KEY".into(),
                value: SECRET.into(),
            },
        ),
    ] {
        let reply = call(port, Request::new(op, body));
        assert!(matches!(reply, Reply::Persisted { .. }), "{op}: {reply:?}");
    }
    let reply = call(
        port,
        Request::new(
            "se",
            Body::SessionNew {
                project,
                name: "shell".into(),
                session_kind: SessionKind::Shell,
                cwd: PathBuf::from("/tmp"),
                launch: Launch::Shell,
                prompt: None,
                notes: String::new(),
                env: std::collections::BTreeMap::new(),
                env_sets: vec!["dev".into()],
                replaces: None,
            },
        ),
    );
    let session = made_id(&reply, RecordKind::Session);
    let token = env
        .host
        .state()
        .spawned
        .last()
        .and_then(|s| {
            s.env
                .iter()
                .find(|(k, _)| k == "SWITCHBOARD_RECORD_TOKEN")
                .map(|(_, v)| v.clone())
        })
        .expect("the pane's token");
    (session, token)
}

fn resolve(port: &mut Port, op: &str, session: &str, token: &str) -> Reply {
    call(
        port,
        Request::new(
            op,
            Body::EnvResolve {
                session: session.into(),
                token: token.into(),
            },
        ),
    )
}

/// Only the holder of the pane's token resolves its sets; nothing the
/// app keeps or logs holds a value or the token.
#[test]
fn env_resolve_answers_only_the_token_holder_and_nothing_keeps_a_value() {
    let mut env = env_port();
    let (session, token) = granted_session(&mut env);
    assert_eq!(
        env.secrets
            .state()
            .get("set/dev/API_KEY")
            .map(String::as_str),
        Some(SECRET)
    );
    let reply = resolve(&mut env.port, "r1", &session, &token);
    let Reply::Env {
        pairs,
        aws,
        missing,
    } = reply
    else {
        panic!("{reply:?}");
    };
    assert_eq!(
        pairs,
        vec![
            ("REGION".to_owned(), "us-east-1".to_owned()),
            ("API_KEY".to_owned(), SECRET.to_owned())
        ]
    );
    assert_eq!((aws, missing), (None, Vec::new()));
    for (op, bad) in [("r2", ""), ("r3", "not-the-token")] {
        assert_eq!(
            resolve(&mut env.port, op, &session, bad),
            Reply::failed("token does not match"),
            "{op}"
        );
    }

    // The sets as listed carry no secret value.
    let listed = call(&mut env.port, Request::new("ls", Body::EnvSets));
    let text = format!("{listed:?}");
    assert!(text.contains("API_KEY") && !text.contains(SECRET), "{text}");
    let view = call(
        &mut env.port,
        Request::new(
            "s",
            Body::Session {
                session: session.clone(),
            },
        ),
    );
    assert!(!format!("{view:?}").contains(SECRET));

    // The operations log: no value, no token, no resolve line, and the
    // secret store's reply is a bare `persisted`.
    let log = std::fs::read_to_string(env.log_dir.path().join("operations.log")).unwrap();
    assert!(!log.contains(SECRET), "{log}");
    assert!(!log.contains(&token), "{log}");
    assert!(!log.contains("env.resolve"), "{log}");
    let stored = log
        .lines()
        .find(|l| l.contains("\"op\":\"sec\"") && l.contains("replied"))
        .expect("the secret store's reply line");
    assert!(stored.contains("persisted"), "{stored}");

    // Nothing the store would write holds the value or the token.
    let kept = format!(
        "{:?}{:?}",
        env.port.app.core().workspaces(),
        env.port.app.core().settings()
    );
    assert!(!kept.contains(SECRET) && !kept.contains(&token));
}

#[test]
fn a_setup_command_is_refused_while_locked_and_persisted_once_unlocked() {
    let mut env = env_port();
    let body = || Body::EnvSetUpsert {
        name: "dev".into(),
        vars: Vec::new(),
        aws: None,
    };
    let reply = call(&mut env.port, Request::new("u1", body()));
    assert_eq!(reply, Reply::failed(switchboard::core::ENV_SETUP_LOCKED));
    env.port.app.dispatch(AppAction::SetEnvSetup { open: true });
    let reply = call(&mut env.port, Request::new("u2", body()));
    assert!(matches!(reply, Reply::Persisted { .. }), "{reply:?}");
}

#[test]
fn session_screen_redacts_a_set_secret_too() {
    let mut env = env_port();
    let (session, _) = granted_session(&mut env);
    let record = switchboard::core::RecordId(uuid::Uuid::parse_str(&session).unwrap());
    let pane = HostId(record.host_name());
    env.host.state().statuses.push(HostStatus {
        id: pane.clone(),
        liveness: HostLiveness::Running {
            pid: 7,
            command: "zsh".into(),
        },
        cwd: None,
        last_activity: None,
        title: None,
    });
    env.host
        .state()
        .snapshots
        .insert(pane, format!("$ env\nAPI_KEY={SECRET}\nREGION=us-east-1\n"));
    env.port.app.poll_now();
    let reply = call(
        &mut env.port,
        Request::new(
            "q",
            Body::SessionScreen {
                session,
                lines: None,
            },
        ),
    );
    assert_eq!(
        reply,
        Reply::Screen {
            text: "$ env\nAPI_KEY=<API_KEY>\nREGION=us-east-1".into()
        }
    );
}

/// Run `switchboard-env` against the port, serving the app until it
/// exits. `vars` are set on the binary's own environment.
fn run_env(port: &mut Port, args: &[&str], vars: &[(&str, &str)]) -> std::process::Output {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_switchboard-env"));
    cmd.args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("SWITCHBOARD_DATA_DIR", port.path.parent().unwrap())
        .stdin(std::process::Stdio::null());
    for (k, v) in vars {
        cmd.env(k, v);
    }
    let child = std::thread::spawn(move || cmd.output().expect("run switchboard-env"));
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !child.is_finished() {
        assert!(std::time::Instant::now() < deadline, "switchboard-env hung");
        port.app.serve_pending();
        std::thread::sleep(Duration::from_millis(2));
    }
    child.join().expect("binary thread")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A directory holding one executable shell script named `name`.
fn fake_bin(name: &str, script: &str) -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

fn set_aws(port: &mut Port, op: &str, aws: AwsMethod) {
    let reply = call(
        port,
        Request::new(
            op,
            Body::EnvSetUpsert {
                name: "dev".into(),
                vars: Vec::new(),
                aws: Some(aws),
            },
        ),
    );
    assert!(matches!(reply, Reply::Persisted { .. }), "{reply:?}");
}

#[test]
fn the_binary_runs_a_child_with_the_pairs_and_without_the_token() {
    let mut env = env_port();
    let (session, token) = granted_session(&mut env);
    let ids = [
        ("SWITCHBOARD_RECORD_ID", session.as_str()),
        ("SWITCHBOARD_RECORD_TOKEN", token.as_str()),
    ];
    let out = run_env(&mut env.port, &["exec", "--", "env"], &ids);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let printed = text(&out.stdout);
    assert!(printed.contains(&format!("API_KEY={SECRET}")), "{printed}");
    assert!(printed.contains("REGION=us-east-1"), "{printed}");
    assert!(!printed.contains("SWITCHBOARD_RECORD_TOKEN"), "{printed}");

    // vault: the command runs under aws-vault.
    set_aws(
        &mut env.port,
        "v",
        AwsMethod::Vault {
            profile: "dev-admin".into(),
        },
    );
    let bin = fake_bin("aws-vault", r#"echo "argv: $*""#);
    let path = format!("{}:/usr/bin:/bin", bin.path().display());
    let mut vars = ids.to_vec();
    vars.push(("PATH", &path));
    let out = run_env(&mut env.port, &["exec", "--", "echo", "hi"], &vars);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout).trim(), "argv: exec dev-admin -- echo hi");

    // sso: an expired session stops with the login line.
    set_aws(
        &mut env.port,
        "s",
        AwsMethod::Sso {
            profile: "dev-sso".into(),
        },
    );
    let bin = fake_bin("aws", "exit 255");
    let path = format!("{}:/usr/bin:/bin", bin.path().display());
    let mut vars = ids.to_vec();
    vars.push(("PATH", &path));
    let out = run_env(&mut env.port, &["exec", "--", "true"], &vars);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains("aws sso login --profile dev-sso"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn the_binary_refuses_without_credentials_and_while_setup_is_locked() {
    let mut env = env_port();
    let out = run_env(&mut env.port, &["exec", "--", "true"], &[]);
    assert_eq!(out.status.code(), Some(64), "{}", text(&out.stderr));
    let out = run_env(
        &mut env.port,
        &["exec", "--", "true"],
        &[("SWITCHBOARD_RECORD_ID", "x")],
    );
    assert_eq!(out.status.code(), Some(64));
    let out = run_env(&mut env.port, &["exec"], &[]);
    assert_eq!(out.status.code(), Some(64));
    let out = run_env(&mut env.port, &["grant", "--runner", "dev"], &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains(switchboard::core::ENV_SETUP_LOCKED),
        "{}",
        text(&out.stderr)
    );
}

/// The binary links only std and the wire crate, so it never grows the
/// app's start-up or its dependencies.
#[test]
fn the_binary_never_uses_the_app_crate() {
    let source = include_str!("../src/bin/switchboard-env.rs");
    assert!(!source.contains(concat!("switchboard", "::")));
}
