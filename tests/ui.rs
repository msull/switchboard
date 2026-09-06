//! UI tests: the real `eframe::App` headlessly via `egui_kittest` with
//! fake adapters. State is seeded directly (the core's `dispatch` is
//! exercised by the core tests), the widgets are found by label, and the
//! assertions are on what is drawn and which actions a click dispatched.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use switchboard::SwitchboardApp;
use switchboard::adapters::fakes::{FakeAgents, FakeEvents, FakeHost, FakeOpener, MemoryStore};
use switchboard::app::Services;
use switchboard::core::{
    Activity, AgentKind, AppAction, CardLayout, Launch, Notice, Project, ProjectId, RecordId,
    SessionKind, SessionRecord, View, Workspace,
};
use switchboard::ports::host::{HostId, HostStatus, Liveness};

/// Ids of the seeded records, so tests can name them in assertions.
struct Seeded {
    alpha: ProjectId,
    beta: ProjectId,
    build: RecordId,
    server: RecordId,
    deploy: RecordId,
    agent: RecordId,
}

fn at(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_000 + secs)
}

fn project(name: &str, last_active: SystemTime) -> Project {
    Project {
        id: ProjectId::new(),
        name: name.into(),
        root: PathBuf::from(format!("/work/{name}")),
        tags: Vec::new(),
        notes: format!("notes for {name}"),
        pinned: vec![PathBuf::from("README.md")],
        created: at(0),
        last_active,
    }
}

fn record(project: ProjectId, name: &str, kind: SessionKind, order: u32) -> SessionRecord {
    SessionRecord {
        id: RecordId::new(),
        project,
        name: name.into(),
        kind,
        cwd: PathBuf::from(format!("/work/{name}")),
        launch: Launch::Shell,
        env_profile: None,
        created: at(0),
        last_seen: at(0),
        notes: String::new(),
        resume: None,
        autostart: false,
        layout: CardLayout { order, group: None },
        activity: Activity::Unknown,
        last_event_at: None,
        last_exit: None,
        not_resumable: false,
        scrollback: None,
    }
}

/// Two projects. `alpha` (most recent) has a session that is not running,
/// one running, and one exited; `beta` has one agent session.
fn seed(app: &mut SwitchboardApp) -> Seeded {
    let alpha = project("alpha", at(100));
    let beta = project("beta", at(50));
    let build = record(alpha.id, "build", SessionKind::Command, 0);
    let mut server = record(alpha.id, "server", SessionKind::Shell, 1);
    server.activity = Activity::WaitingOnYou;
    let deploy = record(alpha.id, "deploy", SessionKind::Service, 2);
    let agent = record(
        beta.id,
        "codex-agent",
        SessionKind::Agent(AgentKind::Codex),
        0,
    );
    let ids = Seeded {
        alpha: alpha.id,
        beta: beta.id,
        build: build.id,
        server: server.id,
        deploy: deploy.id,
        agent: agent.id,
    };
    let host = vec![
        HostStatus {
            id: HostId(server.id.host_name()),
            liveness: Liveness::Running {
                pid: 42,
                command: "zsh".into(),
            },
            cwd: None,
            last_activity: None,
            title: Some("make: all".into()),
        },
        HostStatus {
            id: HostId(deploy.id.host_name()),
            liveness: Liveness::Exited { code: Some(1) },
            cwd: None,
            last_activity: None,
            title: None,
        },
    ];
    let mut alpha_ws = Workspace::new(alpha);
    alpha_ws.sessions = vec![build, server, deploy];
    let mut beta_ws = Workspace::new(beta);
    beta_ws.sessions = vec![agent];
    app.core_mut_for_seeding()
        .seed(vec![alpha_ws, beta_ws], host);
    ids
}

fn harness() -> (Harness<'static, SwitchboardApp>, Seeded) {
    let services = Services {
        store: Box::new(MemoryStore::default()),
        host: Box::new(FakeHost::default()),
        events: Box::new(FakeEvents::default()),
        agents: Box::new(FakeAgents::default()),
        opener: Box::new(FakeOpener::default()),
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_eframe(move |_cc| {
            let mut app = SwitchboardApp::with_services(services);
            app.record_actions = true;
            app.ui_state.embed_terminals = false;
            app
        });
    let ids = seed(harness.state_mut());
    harness.run_steps(2);
    (harness, ids)
}

fn showing(harness: &mut Harness<'static, SwitchboardApp>, view: View) {
    harness.state_mut().core_mut_for_seeding().seed_view(view);
    harness.run_steps(2);
}

/// Every action a test's clicks dispatched, without the per-frame ticks.
fn actions(harness: &Harness<'static, SwitchboardApp>) -> Vec<AppAction> {
    harness
        .state()
        .dispatched
        .iter()
        .filter(|a| **a != AppAction::Tick)
        .cloned()
        .collect()
}

fn click(harness: &mut Harness<'static, SwitchboardApp>, label: &str) {
    harness.get_by_label(label).click();
    harness.run_steps(2);
}

fn type_into(harness: &mut Harness<'static, SwitchboardApp>, label: &str, text: &str) {
    let field = harness.get_by_label(label);
    field.focus();
    field.type_text(text);
    harness.run_steps(2);
}

#[test]
fn shows_the_switchboard_heading() {
    let (harness, _) = harness();
    harness.get_by_label("Switchboard");
}

#[test]
fn switcher_lists_projects_most_recent_first() {
    let (harness, _) = harness();
    let alpha = harness.get_by_role_and_label(Role::Button, "alpha").rect();
    let beta = harness.get_by_role_and_label(Role::Button, "beta").rect();
    assert!(
        alpha.min.x < beta.min.x,
        "alpha was active last, so it comes first"
    );
}

#[test]
fn clicking_a_project_dispatches_show_board() {
    let (mut harness, ids) = harness();
    harness.get_by_role_and_label(Role::Button, "beta").click();
    harness.run_steps(2);
    assert_eq!(actions(&harness), vec![AppAction::ShowBoard(ids.beta)]);
}

#[test]
fn switchboard_lists_sessions_from_both_projects() {
    let (harness, _) = harness();
    harness.get_by_label("All sessions");
    for name in ["build", "server", "deploy", "codex-agent"] {
        harness.get_by_label(name);
    }
}

#[test]
fn board_lists_the_projects_sessions_and_notes() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Board(ids.alpha));
    for name in ["build", "server", "deploy"] {
        harness.get_by_label(name);
    }
    assert!(harness.query_by_label("codex-agent").is_none());
    harness.get_by_label("notes for alpha");
    harness.get_by_label("README.md");
    // The running session's pane title is its caption.
    harness.get_by_label("make: all");
}

#[test]
fn clicking_a_card_dispatches_show_session() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Board(ids.alpha));
    click(&mut harness, "build");
    assert_eq!(actions(&harness), vec![AppAction::ShowSession(ids.build)]);
}

#[test]
fn card_buttons_kill_running_and_remove_stopped_sessions() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Board(ids.alpha));
    // Only `server` is running, so it alone offers Open and Kill.
    click(&mut harness, "Kill");
    click(&mut harness, "Open");
    harness.get_all_by_label("Remove").next().unwrap().click();
    harness.run_steps(2);
    let dispatched = actions(&harness);
    assert_eq!(dispatched[0], AppAction::KillSession(ids.server));
    assert_eq!(dispatched[1], AppAction::ReturnToSession(ids.server));
    assert!(matches!(
        dispatched[2],
        AppAction::RemoveSession(id) if id == ids.build || id == ids.deploy
    ));
}

#[test]
fn new_session_dialog_dispatches_new_session() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Board(ids.alpha));
    click(&mut harness, "New session");
    harness.get_by_label("Create a session");
    type_into(&mut harness, "Name", "review");
    click(&mut harness, "Codex");
    click(&mut harness, "Create");
    assert_eq!(
        actions(&harness),
        vec![AppAction::NewSession {
            project: ids.alpha,
            name: "review".into(),
            kind: SessionKind::Agent(AgentKind::Codex),
            cwd: PathBuf::from("/work/alpha"),
            launch: Launch::Shell,
        }]
    );
    assert!(harness.query_by_label("Create a session").is_none());
}

#[test]
fn new_command_session_carries_the_command() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Board(ids.alpha));
    click(&mut harness, "New session");
    type_into(&mut harness, "Name", "tests");
    click(&mut harness, "Command");
    type_into(&mut harness, "Command line", "make test");
    click(&mut harness, "Create");
    let dispatched = actions(&harness);
    assert_eq!(dispatched.len(), 1);
    match &dispatched[0] {
        AppAction::NewSession {
            name,
            kind,
            launch: Launch::Command { command, shell },
            ..
        } => {
            assert_eq!(name, "tests");
            assert_eq!(*kind, SessionKind::Command);
            assert_eq!(command, "make test");
            assert!(!shell.is_empty());
        }
        other => panic!("expected a command launch, got {other:?}"),
    }
}

#[test]
fn cancelling_the_session_dialog_dispatches_nothing() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Board(ids.alpha));
    click(&mut harness, "New session");
    click(&mut harness, "Cancel");
    assert!(actions(&harness).is_empty());
    assert!(harness.query_by_label("Create a session").is_none());
}

#[test]
fn add_project_dialog_dispatches_add_project() {
    let (mut harness, _) = harness();
    click(&mut harness, "Add project");
    type_into(&mut harness, "Name", "gamma");
    type_into(&mut harness, "Root path", "/work/gamma");
    click(&mut harness, "Add");
    assert_eq!(
        actions(&harness),
        vec![AppAction::AddProject {
            name: "gamma".into(),
            root: PathBuf::from("/work/gamma"),
        }]
    );
}

#[test]
fn notice_bar_shows_the_notice_and_dismisses_it() {
    let (mut harness, _) = harness();
    harness.state_mut().core_mut_for_seeding().seed_status(
        Some(Notice {
            text: "Recovered alpha from backup".into(),
            is_error: true,
            expires_at: None,
        }),
        None,
        false,
    );
    harness.run_steps(2);
    harness.get_by_label("Recovered alpha from backup");
    click(&mut harness, "Dismiss");
    assert_eq!(actions(&harness), vec![AppAction::DismissNotice]);
}

#[test]
fn host_error_and_read_only_tag_are_shown() {
    let (mut harness, _) = harness();
    harness.state_mut().core_mut_for_seeding().seed_status(
        None,
        Some("tmux 3.2 or newer is required".into()),
        true,
    );
    harness.run_steps(2);
    harness.get_by_label("tmux 3.2 or newer is required");
    harness.get_by_label("read-only");
}

#[test]
fn session_view_header_buttons_dispatch() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Session(ids.server));
    harness.get_by_label("server");
    harness.get_by_label("shell");
    harness.get_by_label("/work/server");
    harness.get_by_label("Embedded terminal disabled.");
    click(&mut harness, "Open in terminal");
    click(&mut harness, "Kill");
    // One Back in the top bar, one in the session header.
    assert_eq!(
        harness
            .get_all_by_role_and_label(Role::Button, "Back")
            .count(),
        2
    );
    assert_eq!(
        actions(&harness),
        vec![
            AppAction::ReturnToSession(ids.server),
            AppAction::KillSession(ids.server),
        ]
    );
}

#[test]
fn agent_session_runs_in_ghostty_and_shows_its_snapshot() {
    let (mut harness, ids) = harness();
    harness
        .state_mut()
        .ui_state
        .snapshots
        .insert(ids.agent, "⏺ pong".into());
    showing(&mut harness, View::Session(ids.agent));
    harness.get_by_label_contains("runs in Ghostty");
    assert!(harness.query_all_by_value("⏺ pong").next().is_some());
    harness.get_by_label("Return");
}

#[test]
fn editing_notes_dispatches_set_session_notes() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Session(ids.build));
    type_into(&mut harness, "Notes", "flaky on CI");
    let dispatched = actions(&harness);
    assert!(
        dispatched.contains(&AppAction::SetSessionNotes(ids.build, "flaky on CI".into())),
        "got {dispatched:?}"
    );
}

#[test]
fn escape_goes_back() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Board(ids.alpha));
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert_eq!(actions(&harness), vec![AppAction::Back]);
}

#[test]
fn command_digits_switch_projects() {
    let (mut harness, ids) = harness();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num2);
    harness.run_steps(2);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num0);
    harness.run_steps(2);
    assert_eq!(
        actions(&harness),
        vec![AppAction::ShowBoard(ids.beta), AppAction::ShowSwitchboard]
    );
}

#[test]
fn waiting_badge_counts_waiting_sessions() {
    let (harness, _) = harness();
    harness.get_by_label("1 waiting");
    harness.get_by_label("waiting on you");
    harness.get_by_label("exited (1)");
}
