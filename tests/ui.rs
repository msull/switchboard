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
use switchboard::adapters::fakes::{
    FakeAgents, FakeEvents, FakeHost, FakeOpener, FakeSecrets, FakeTranscripts, MemoryStore,
};
use switchboard::app::Services;
use switchboard::core::{
    Activity, AgentKind, AppAction, CardLayout, Launch, Notice, Project, ProjectEnv, ProjectId,
    RecordId, ResumeHandle, SessionKind, SessionRecord, ThemeMode, View, Workspace,
};
use switchboard::ports::host::{HostId, HostStatus, Liveness};
use switchboard::ports::transcript::{
    Activity as TranscriptActivity, ActivityKind, Conversation, ToolDetail, Turn, Usage,
};

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
        env: ProjectEnv::default(),
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
    harness_with(FakeOpener::default())
}

/// A harness whose opener the test keeps a handle to.
fn harness_with(opener: FakeOpener) -> (Harness<'static, SwitchboardApp>, Seeded) {
    harness_full(opener, FakeSecrets::default())
}

fn harness_full(
    opener: FakeOpener,
    secrets: FakeSecrets,
) -> (Harness<'static, SwitchboardApp>, Seeded) {
    let services = Services {
        store: Box::new(MemoryStore::default()),
        host: Box::new(FakeHost::default()),
        events: Box::new(FakeEvents::default()),
        agents: Box::new(FakeAgents::default()),
        opener: Box::new(opener),
        transcripts: Box::new(FakeTranscripts::default()),
        secrets: Box::new(secrets),
        wake: None,
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

#[test]
fn settings_menu_sets_the_theme() {
    let (mut harness, _) = harness();
    click(&mut harness, "Settings");
    click(&mut harness, "Dark");
    assert!(actions(&harness).contains(&AppAction::SetTheme(ThemeMode::Dark)));
    assert_eq!(harness.state().core().settings().theme, ThemeMode::Dark);
}

#[test]
fn exclusive_mode_hides_the_other_projects() {
    let (mut harness, _) = harness();
    assert!(harness.query_all_by_label("beta").count() > 0);
    click(&mut harness, "Settings");
    click(&mut harness, "Exclusive: only the active project");
    assert!(actions(&harness).contains(&AppAction::SetExclusive(true)));
    // alpha was active most recently, so it is the one that stays.
    assert!(harness.query_all_by_label("alpha").count() > 0);
    assert_eq!(harness.query_all_by_label("beta").count(), 0);
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

/// A project rooted in a real temp directory, for the file side.
fn file_project(harness: &mut Harness<'static, SwitchboardApp>) -> (tempfile::TempDir, ProjectId) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("docs")).unwrap();
    std::fs::create_dir_all(dir.path().join("target")).unwrap();
    std::fs::write(dir.path().join("README.md"), "# Hello\n\nfrom the readme").unwrap();
    std::fs::write(dir.path().join("docs/design.md"), "# Design").unwrap();
    std::fs::write(dir.path().join("target/out.bin"), "x").unwrap();
    std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
    let mut p = project("files", at(200));
    p.root = dir.path().to_path_buf();
    let pid = p.id;
    harness
        .state_mut()
        .core_mut_for_seeding()
        .seed(vec![Workspace::new(p)], Vec::new());
    showing(harness, View::Board(pid));
    (dir, pid)
}

#[test]
fn file_tree_previews_a_file_and_hands_off_to_the_editor() {
    let opener = FakeOpener::default();
    let (mut harness, _) = harness_with(opener.clone());
    let (dir, pid) = file_project(&mut harness);
    // Ignored directories stay out of the tree; folders open on click.
    assert!(harness.query_by_label("⏵ target").is_none());
    click(&mut harness, "⏵ docs");
    harness.get_by_label("  design.md");
    click(&mut harness, "  README.md");
    let readme = dir.path().join("README.md");
    assert!(actions(&harness).contains(&AppAction::ShowDocument(pid, readme.clone())));
    assert_eq!(
        harness.state().core().view(),
        View::Document(pid, readme.clone())
    );
    harness.get_by_label("from the readme");
    click(&mut harness, "Open in editor");
    click(&mut harness, "Reveal");
    // The seeded project already pins README.md, so the button unpins first.
    click(&mut harness, "Unpin");
    click(&mut harness, "Pin");
    assert_eq!(opener.state().edited, vec![(String::new(), readme.clone())]);
    assert_eq!(opener.state().revealed, vec![readme.clone()]);
    assert!(actions(&harness).contains(&AppAction::UnpinDocument(pid, "README.md".into())));
    assert!(actions(&harness).contains(&AppAction::PinDocument(pid, "README.md".into())));
    harness.get_by_label("Unpin");
    // One Back in the top bar, one in the document header; either works.
    harness
        .get_all_by_role_and_label(Role::Button, "Back")
        .next()
        .unwrap()
        .click();
    harness.run_steps(2);
    assert_eq!(harness.state().core().view(), View::Board(pid));
}

#[test]
fn finder_matches_across_the_project() {
    let (mut harness, _) = harness();
    let (dir, pid) = file_project(&mut harness);
    type_into(&mut harness, "Find", "dsgn");
    // The index is built on a thread; give it a moment.
    for _ in 0..40 {
        if harness.query_by_label("docs/design.md").is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
        harness.run_steps(2);
    }
    click(&mut harness, "docs/design.md");
    assert!(actions(&harness).contains(&AppAction::ShowDocument(
        pid,
        dir.path().join("docs/design.md")
    )));
    assert!(harness.query_by_label("target/out.bin").is_none());
}

#[test]
fn pinned_card_previews_and_opens() {
    let opener = FakeOpener::default();
    let (mut harness, _) = harness_with(opener.clone());
    let (dir, _pid) = file_project(&mut harness);
    // README.md is pinned by the seed.
    click(&mut harness, "Open in app");
    assert_eq!(opener.state().opened, vec![dir.path().join("README.md")]);
}

#[test]
fn environment_dialog_saves_variables_and_stores_secrets() {
    let secrets = FakeSecrets::default();
    let (mut harness, ids) = harness_full(FakeOpener::default(), secrets.clone());
    showing(&mut harness, View::Board(ids.alpha));
    click(&mut harness, "Environment");
    click(&mut harness, "Add variable");
    type_into(&mut harness, "Name 1", "TOKEN");
    type_into(&mut harness, "Value 1", "s3cret");
    click(&mut harness, "Secret 1");
    click(&mut harness, "Save");
    let dispatched = actions(&harness);
    assert!(dispatched.iter().any(|a| matches!(
        a,
        AppAction::SetProjectEnv(pid, env)
            if *pid == ids.alpha && env.vars.len() == 1 && env.vars[0].secret && env.vars[0].value.is_empty()
    )));
    let account = format!("project/{}/TOKEN", ids.alpha.0);
    assert_eq!(
        secrets.state().get(&account).map(String::as_str),
        Some("s3cret")
    );
    assert!(harness.state().ui_state.env_dialog.is_none());
}

#[test]
fn quick_switcher_opens_the_best_match_on_enter() {
    let (mut harness, ids) = harness();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::K);
    harness.run_steps(2);
    let field = harness.get_by_label("Search");
    field.focus();
    field.type_text("srv");
    harness.run_steps(2);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    assert!(actions(&harness).contains(&AppAction::ShowSession(ids.server)));
    assert_eq!(harness.state().core().view(), View::Session(ids.server));
    assert!(harness.state().ui_state.palette.is_none());
}

#[test]
fn service_header_offers_restart_and_autostart() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Session(ids.deploy));
    click(&mut harness, "Restart");
    click(&mut harness, "Autostart");
    let dispatched = actions(&harness);
    assert!(dispatched.contains(&AppAction::RestartSession(ids.deploy)));
    assert!(dispatched.contains(&AppAction::SetAutostart(ids.deploy, true)));
    // Agents cannot be restarted from the header.
    showing(&mut harness, View::Session(ids.agent));
    assert!(harness.query_by_label("Restart").is_none());
}

#[test]
fn session_view_renames_on_enter() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Session(ids.server));
    click(&mut harness, "Rename");
    let field = harness.get_by_label("Session name");
    field.focus();
    field.type_text(" v2");
    harness.run_steps(2);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    assert!(
        actions(&harness).contains(&AppAction::RenameSession(ids.server, "server v2".into())),
        "{:?}",
        actions(&harness)
    );
    harness.get_by_label("server v2");
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

/// A Claude Code session, running, with a resume handle: the shape the
/// conversation view needs. Added on top of the standard seed.
fn seed_claude(harness: &mut Harness<'static, SwitchboardApp>, ids: &Seeded) -> RecordId {
    let mut claude = record(
        ids.beta,
        "claude-agent",
        SessionKind::Agent(AgentKind::ClaudeCode),
        1,
    );
    claude.resume = Some(ResumeHandle::ClaudeCode {
        session_id: uuid::Uuid::nil(),
        transcript: Some(PathBuf::from("/nowhere/x.jsonl")),
    });
    let id = claude.id;
    let core = harness.state_mut().core_mut_for_seeding();
    let mut workspaces = core.workspaces().to_vec();
    workspaces
        .iter_mut()
        .find(|w| w.project.id == ids.beta)
        .unwrap()
        .sessions
        .push(claude);
    let host = vec![HostStatus {
        id: HostId(id.host_name()),
        liveness: Liveness::Running {
            pid: 43,
            command: "claude".into(),
        },
        cwd: None,
        last_activity: None,
        title: None,
    }];
    core.seed(workspaces, host);
    id
}

fn two_turns() -> Conversation {
    let tool = TranscriptActivity {
        kind: ActivityKind::Tool,
        line: "Bash: Read crate name from Cargo.toml".into(),
        at: Some(at(10)),
        error: false,
        detail: Some(ToolDetail {
            name: "Bash".into(),
            input: "{\"command\": \"grep name Cargo.toml\"}".into(),
            result: "name = \"switchboard\"".into(),
        }),
    };
    Conversation {
        title: Some("explain-repo".into()),
        model: Some("claude-fable-5-1".into()),
        version: Some("2.1.263".into()),
        branch: Some("main".into()),
        start: Some(at(0)),
        end: Some(at(70)),
        turns: vec![
            Turn {
                n: 1,
                at: Some(at(0)),
                end: Some(at(2)),
                user: "reply with the single word pong".into(),
                final_text: "pong".into(),
                assistant_msgs: 1,
                ..Turn::default()
            },
            Turn {
                n: 2,
                at: Some(at(5)),
                end: Some(at(70)),
                user: "What is the crate called?".into(),
                activity: vec![tool],
                final_text: "The crate is called switchboard.".into(),
                assistant_msgs: 2,
                tools: 1,
                ..Turn::default()
            },
        ],
        usage: Usage {
            output: 18,
            ..Usage::default()
        },
    }
}

#[test]
fn claude_session_shows_the_conversation_and_message_box() {
    let (mut harness, ids) = harness();
    let id = seed_claude(&mut harness, &ids);
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, two_turns()));
    showing(&mut harness, View::Session(id));
    harness.get_by_label("explain-repo");
    harness.get_by_label("reply with the single word pong");
    harness.get_by_label("What is the crate called?");
    harness.get_by_label("pong");
    harness.get_by_label("The crate is called switchboard.");
    harness.get_by_label("2 msgs · 1 tools · 2m");
    harness.get_by_label("Message");
    harness.get_by_role_and_label(Role::Button, "Send");
    harness.get_by_label("Terminal");
    // Activity is folded by default; the toggle opens every turn's list.
    assert!(
        harness
            .query_by_label_contains("Bash: Read crate name")
            .is_none()
    );
    click(&mut harness, "Expand activity");
    harness.get_by_label_contains("Bash: Read crate name");
}

#[test]
fn claude_session_without_a_conversation_falls_back_to_the_snapshot() {
    let (mut harness, ids) = harness();
    let id = seed_claude(&mut harness, &ids);
    harness
        .state_mut()
        .ui_state
        .conversation_errors
        .insert(id, "no transcript".into());
    harness
        .state_mut()
        .ui_state
        .snapshots
        .insert(id, "⏺ working".into());
    showing(&mut harness, View::Session(id));
    harness.get_by_label_contains("runs in Ghostty");
    harness.get_by_label_contains("no transcript");
    assert!(harness.query_all_by_value("⏺ working").next().is_some());
    harness.get_by_label("Message");
}

#[test]
fn polling_reads_the_transcript_into_the_ui_state() {
    let services = Services {
        store: Box::new(MemoryStore::default()),
        host: Box::new(FakeHost::default()),
        events: Box::new(FakeEvents::default()),
        agents: Box::new(FakeAgents::default()),
        opener: Box::new(FakeOpener::default()),
        transcripts: Box::new(FakeTranscripts {
            conversation: Some(two_turns()),
        }),
        secrets: Box::new(FakeSecrets::default()),
        wake: None,
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_eframe(move |_cc| {
            let mut app = SwitchboardApp::with_services(services);
            app.ui_state.embed_terminals = false;
            app
        });
    let ids = seed(harness.state_mut());
    let id = seed_claude(&mut harness, &ids);
    showing(&mut harness, View::Session(id));
    harness.state_mut().poll_now();
    let app = harness.state();
    assert_eq!(app.ui_state.conversations[&id].1.turns.len(), 2);
    assert!(!app.ui_state.conversation_errors.contains_key(&id));
}
