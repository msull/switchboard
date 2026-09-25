//! UI tests: the real `eframe::App` headlessly via `egui_kittest` with
//! fake adapters. State is seeded directly (the core's `dispatch` is
//! exercised by the core tests), the widgets are found by label, and the
//! assertions are on what is drawn and which actions a click dispatched.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use switchboard::SwitchboardApp;
use switchboard::adapters::fakes::{
    FakeAgents, FakeArtifacts, FakeEvents, FakeHost, FakeOpener, FakeProjectConfig, FakeRoundFiles,
    FakeSecrets, FakeTranscripts, MemoryStore,
};
use switchboard::app::Services;
use switchboard::core::{
    Activity, AgentKind, AppAction, Approval, BUILTIN_WORKFLOW, CardLayout, Definition,
    HandoffMode, Launch, Notice, PinTarget, Project, ProjectEnv, ProjectId, RecordId, ResumeHandle,
    Round, RunState, SessionKind, SessionRecord, SideTab, SpaceId, ThemeMode, Verdict, View,
    VoiceSettings, WorkflowId, WorkflowRun, Workspace, round_paths,
};
use switchboard::ports::host::{HostId, HostStatus, Liveness};
use switchboard::ports::store::Store;
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
    /// A command defined in alpha's `.switchboard/project.json`, unapproved.
    lint: RecordId,
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
        shown: Vec::new(),
        created: at(0),
        last_active,
        space: SpaceId::DEFAULT,
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
        activity_reason: None,
        last_event_at: None,
        last_exit: None,
        not_resumable: false,
        scrollback: None,
        source: None,
        approved_hash: None,
        discard: None,
        runs: Vec::new(),
        outputs: Vec::new(),
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
    server.activity_reason = Some("permission for Bash".into());
    let deploy = record(alpha.id, "deploy", SessionKind::Service, 2);
    let mut lint = record(alpha.id, "lint", SessionKind::Command, 3);
    lint.launch = Launch::Command {
        command: "cargo clippy".into(),
        shell: "/bin/zsh".into(),
    };
    lint.source = Some(Definition {
        name: "lint".into(),
        hash: "h1".into(),
        env: vec!["RUSTFLAGS".into()],
        autostart: false,
        orphaned: false,
    });
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
        lint: lint.id,
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
    alpha_ws.sessions = vec![build, server, deploy, lint];
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

/// A harness whose host the test scripted (a failing write, say).
fn harness_with_host(host: FakeHost) -> (Harness<'static, SwitchboardApp>, Seeded) {
    harness_build(FakeOpener::default(), FakeSecrets::default(), host)
}

fn harness_full(
    opener: FakeOpener,
    secrets: FakeSecrets,
) -> (Harness<'static, SwitchboardApp>, Seeded) {
    harness_build(opener, secrets, FakeHost::default())
}

fn harness_build(
    opener: FakeOpener,
    secrets: FakeSecrets,
    host: FakeHost,
) -> (Harness<'static, SwitchboardApp>, Seeded) {
    let services = Services {
        store: Box::new(MemoryStore::default()),
        host: Box::new(host),
        events: Box::new(FakeEvents::default()),
        agents: Box::new(FakeAgents::default()),
        opener: Box::new(opener),
        transcripts: Box::new(FakeTranscripts::default()),
        secrets: Box::new(secrets),
        project_config: Box::new(FakeProjectConfig::default()),
        round_files: Box::new(FakeRoundFiles::default()),
        artifacts: Box::new(FakeArtifacts::default()),
        wake: None,
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_eframe(move |cc| {
            switchboard::ui::theme::install(&cc.egui_ctx);
            let mut app = SwitchboardApp::with_services(services);
            app.record_actions = true;
            app.ui_state.embed_terminals = false;
            app.ui_state.prompt_boxes.native = false;
            app
        });
    let ids = seed(harness.state_mut());
    harness.run_steps(2);
    (harness, ids)
}

/// Turns the Prompt Box editor off so the plain message box is drawn.
fn plain_message_box(harness: &mut Harness<'static, SwitchboardApp>) {
    harness.state_mut().dispatch(AppAction::SetPromptBox(false));
    harness.state_mut().dispatched.clear();
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
fn a_new_workspace_shows_nothing_of_the_others_until_the_selector_opens() {
    let (mut harness, _) = harness();
    assert!(harness.query_all_by_label("beta").count() > 0);
    // The rail's head names the active workspace and opens the selector.
    click(&mut harness, "Default ▾");
    click(&mut harness, "New workspace…");
    harness.get_by_label("New workspace");
    type_into(&mut harness, "Name", "Client");
    click(&mut harness, "Create");
    harness.run_steps(2);
    assert!(
        actions(&harness).contains(&AppAction::NewSpace("Client".into())),
        "{:?}",
        actions(&harness)
    );
    harness.get_by_label("Client ▾");
    // Nothing of the default workspace is on screen: no project names,
    // no rows, and the quick switcher finds none of them.
    assert_eq!(harness.query_all_by_label("alpha").count(), 0);
    assert_eq!(harness.query_all_by_label("beta").count(), 0);
    assert_eq!(harness.query_all_by_label("Default").count(), 0);
    click(&mut harness, "Go to ⌘K");
    harness.run_steps(2);
    assert_eq!(harness.query_all_by_label_contains("alpha").count(), 0);
    click(&mut harness, "All workspaces");
    harness.run_steps(2);
    harness.get_by_label("Default · alpha");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    // A notice about the default workspace's session says only that
    // something needs attention.
    harness.state_mut().core_mut_for_seeding().seed_status(
        Some(Notice {
            text: "cannot start server: gone".into(),
            is_error: true,
            expires_at: None,
            space: Some(SpaceId::DEFAULT),
        }),
        None,
        false,
    );
    harness.run_steps(2);
    harness.get_by_label(Notice::ELSEWHERE);
    assert_eq!(harness.query_all_by_label_contains("server").count(), 0);
    // The selector lists every workspace; picking one steps into it.
    click(&mut harness, "Client ▾");
    // The row carries the workspace's waiting count: a number only.
    harness.get_by_label_contains("Default 1").click();
    harness.run_steps(2);
    assert!(actions(&harness).contains(&AppAction::ShowSpace(SpaceId::DEFAULT)));
    harness.get_by_label("Default ▾");
    assert!(harness.query_all_by_label("alpha").count() > 0);
    harness.get_by_label("cannot start server: gone");
}

#[test]
fn a_project_moves_to_another_workspace_from_its_board() {
    let (mut harness, ids) = harness();
    let clock = switchboard::core::Clock::at(1);
    harness
        .state_mut()
        .core_mut_for_seeding()
        .dispatch(AppAction::NewSpace("Client".into()), clock);
    harness
        .state_mut()
        .core_mut_for_seeding()
        .dispatch(AppAction::ShowSpace(SpaceId::DEFAULT), clock);
    showing(&mut harness, View::Board(ids.alpha));
    click(&mut harness, "Move to");
    click(&mut harness, "Client");
    harness.run_steps(2);
    let client = harness.state().core().spaces()[1].id;
    assert!(
        actions(&harness).contains(&AppAction::MoveProjectToSpace(ids.alpha, client)),
        "{:?}",
        actions(&harness)
    );
    // The project is gone from this workspace's rail and the board gave
    // way to the switchboard.
    assert_eq!(harness.query_all_by_label("alpha").count(), 0);
    assert!(harness.query_all_by_label("beta").count() > 0);
}

#[test]
fn the_settings_menu_toggles_opening_the_terminal_on_launch() {
    let (mut harness, _) = harness();
    assert!(!harness.state().core().settings().open_terminal_on_launch);
    click(&mut harness, "Settings");
    click(&mut harness, "Open the terminal when an agent starts");
    assert!(actions(&harness).contains(&AppAction::SetOpenTerminalOnLaunch(true)));
    assert!(harness.state().core().settings().open_terminal_on_launch);
}

/// The Settings menu's trigger word field. The fields carry hint text,
/// not labels: editor command, then the trigger word, then the model.
fn trigger_field<'h>(h: &'h mut Harness<'static, SwitchboardApp>) -> egui_kittest::Node<'h> {
    h.query_all_by_role(Role::TextInput)
        .nth(1)
        .expect("the trigger word field")
}

#[test]
fn clicking_into_a_settings_field_keeps_the_menu_open() {
    let (mut harness, _) = harness();
    click(&mut harness, "Settings");
    harness.get_by_label("Captions while listening");
    trigger_field(&mut harness).click();
    harness.run_steps(2);
    harness.get_by_label("Captions while listening");
    let field = trigger_field(&mut harness);
    field.focus();
    field.type_text("Jarvis");
    harness.run_steps(2);
    // Leaving the field commits the trigger word.
    harness.get_by_label("Captions while listening").focus();
    harness.run_steps(2);
    assert_eq!(harness.state().core().settings().voice.trigger, "Jarvis");
    harness.get_by_label("Captions while listening");
}

fn type_into(harness: &mut Harness<'static, SwitchboardApp>, label: &str, text: &str) {
    let field = harness.get_by_label(label);
    field.focus();
    field.type_text(text);
    harness.run_steps(2);
}

#[test]
fn the_rail_is_headed_by_the_active_workspace() {
    let (harness, _) = harness();
    harness.get_by_label("Default ▾");
}

#[test]
fn switcher_lists_projects_most_recent_first() {
    let (harness, _) = harness();
    let alpha = harness.get_by_role_and_label(Role::Button, "alpha").rect();
    let beta = harness.get_by_role_and_label(Role::Button, "beta").rect();
    assert!(
        alpha.min.y < beta.min.y,
        "alpha was active last, so it comes first in the rail"
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
    // Once in the rail, once as the page title.
    assert_eq!(harness.query_all_by_label("All sessions").count(), 2);
    // Agents and shells only; commands and services stay on the board.
    for name in ["server", "codex-agent"] {
        harness.get_by_label(name);
    }
    assert!(harness.query_by_label("build").is_none());
    assert!(harness.query_by_label("deploy").is_none());
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
    // The waiting session's card says why it waits; the pane title
    // would only get in the way of that.
    harness.get_by_label("permission for Bash");
    assert!(harness.query_by_label("make: all").is_none());
}

#[test]
fn clicking_a_card_dispatches_show_session() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Board(ids.alpha));
    click(&mut harness, "server");
    assert_eq!(actions(&harness), vec![AppAction::ShowSession(ids.server)]);
}

#[test]
fn board_separates_commands_and_services_from_sessions() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Board(ids.alpha));
    harness.get_by_label("AGENTS AND SHELLS");
    harness.get_by_label("COMMANDS AND SERVICES");
    // The shell is a card with Open and Kill; a command's card has Open
    // too (its page), and ▶ Run is the only thing that runs it.
    assert!(harness.get_all_by_label("Open").count() >= 2);
    harness.get_all_by_label("▶ Run").next().unwrap();
    harness.get_all_by_label("Open").nth(1).unwrap().click();
    harness.run_steps(2);
    let dispatched = actions(&harness);
    assert!(matches!(
        dispatched[0],
        AppAction::ShowSession(id) if id == ids.build || id == ids.deploy || id == ids.lint
    ));
}

#[test]
fn run_bar_runs_commands_and_toggles_services() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Session(ids.server));
    // The glyph runs or starts; the name opens.
    click(&mut harness, "Run build");
    click(&mut harness, "Start deploy");
    click(&mut harness, "Open lint");
    let dispatched = actions(&harness);
    assert!(dispatched.contains(&AppAction::RestartSession(ids.build)));
    assert!(dispatched.contains(&AppAction::RestartSession(ids.deploy)));
    // Unapproved: the bar opens the Run tab instead of the page, and
    // its glyph does nothing.
    assert!(!dispatched.contains(&AppAction::RestartSession(ids.lint)));
    assert!(!dispatched.contains(&AppAction::ShowSession(ids.lint)));
    assert!(dispatched.contains(&AppAction::SetSideTab(SideTab::Run)));
    assert!(dispatched.contains(&AppAction::SetFilesOpen(true)));
    harness.get_by_label("cargo clippy");
    harness.state_mut().dispatched.clear();
    click(&mut harness, "Run lint");
    assert!(actions(&harness).is_empty());
    click(&mut harness, "Open build");
    assert_eq!(actions(&harness), vec![AppAction::ShowSession(ids.build)]);
    // The bar is on the board too, without opening anything; build is
    // running since the click, so its glyph now stops it.
    showing(&mut harness, View::Board(ids.alpha));
    harness.get_by_label("Open build");
    harness.get_by_label("Stop build");
}

#[test]
fn run_bar_shows_a_running_commands_last_line_and_hover_after() {
    let (mut harness, ids) = harness();
    harness
        .state_mut()
        .ui_state
        .captions
        .insert(ids.build, "compiled 12 files".into());
    showing(&mut harness, View::Session(ids.server));
    // Finished: the line is on the hover only.
    assert!(harness.query_by_label("compiled 12 files").is_none());
    harness
        .ctx
        .all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
    harness.get_by_label("Open build").hover();
    harness.run_steps(3);
    harness.get_by_label_contains("never run");
    harness.get_by_label_contains("Open: runs and output");
    // Running: the line sits next to the button with a spinner.
    let mut host = harness
        .state()
        .core()
        .host_status(ids.server)
        .unwrap()
        .clone();
    host.id = HostId(ids.build.host_name());
    harness
        .state_mut()
        .dispatch(AppAction::HostListed(vec![host]));
    harness.run_steps(2);
    harness.get_by_label("compiled 12 files");
}

#[test]
fn card_buttons_kill_running_and_remove_stopped_sessions() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Board(ids.alpha));
    // Only `server` is running, so it alone offers Kill; its Open (the
    // first on the board, in the sessions grid) goes last: it leaves the
    // board for the session.
    click(&mut harness, "Kill");
    harness.get_all_by_label("Remove").next().unwrap().click();
    harness.run_steps(2);
    harness.get_all_by_label("Open").next().unwrap().click();
    harness.run_steps(2);
    let dispatched = actions(&harness);
    assert_eq!(dispatched[0], AppAction::KillSession(ids.server));
    assert!(matches!(
        dispatched[1],
        AppAction::RemoveSession(id) if id == ids.build || id == ids.deploy
    ));
    assert_eq!(dispatched[2], AppAction::ReturnToSession(ids.server));
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
            outputs: Vec::new(),
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
    click(&mut harness, "+ Add project");
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
            space: None,
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

/// A project rooted in a real temp directory, for the file side, with one
/// agent session so the side can be toggled next to its message box.
fn file_project(
    harness: &mut Harness<'static, SwitchboardApp>,
) -> (tempfile::TempDir, ProjectId, RecordId) {
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
    let shell = record(pid, "sh", SessionKind::Agent(AgentKind::ClaudeCode), 0);
    let sid = shell.id;
    let mut ws = Workspace::new(p);
    ws.sessions = vec![shell];
    harness
        .state_mut()
        .core_mut_for_seeding()
        .seed(vec![ws], Vec::new());
    showing(harness, View::Board(pid));
    (dir, pid, sid)
}

#[test]
fn a_wide_table_in_a_document_scrolls_sideways_while_prose_wraps() {
    let (mut harness, _) = harness();
    let (dir, pid, _) = file_project(&mut harness);
    let prose = "words that wrap ".repeat(60);
    let cell = "cell text ".repeat(40);
    let unbreakable = "x".repeat(400);
    std::fs::write(
        dir.path().join("README.md"),
        format!(
            "# T\n\n{prose}\n\n| a | b |\n|---|---|\n| {cell} | last |\n\n\
             | c | d |\n|---|---|\n| {unbreakable} | far |\n"
        ),
    )
    .unwrap();
    showing(
        &mut harness,
        View::Document(pid, dir.path().join("README.md")),
    );
    // The side panel begins where the document view ends.
    let viewport = harness.get_by_label("Find").rect().left();
    let prose_rect = harness.get_by_label_contains("words that wrap").rect();
    assert!(
        prose_rect.right() <= viewport + 1.0,
        "prose wraps at the view: {prose_rect:?} vs {viewport}"
    );
    assert!(prose_rect.height() > 40.0, "prose takes several lines");
    // A cell of words wraps inside its column, so the table fits.
    let cell_rect = harness.get_by_label_contains("cell text").rect();
    assert!(cell_rect.height() > 40.0, "the cell wraps: {cell_rect:?}");
    let last = harness.get_by_label("last").rect();
    assert!(last.right() <= viewport, "the table fits: {last:?}");
    assert!(last.left() >= cell_rect.right(), "columns apart");
    // A word that cannot break makes its table wider than the view;
    // the document scrolls sideways to reach the far cell.
    let before = harness.get_by_label("far").rect().left();
    assert!(before > viewport, "the far cell starts off screen");
    harness.event(egui::Event::PointerMoved(prose_rect.center()));
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(-400.0, 0.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run_steps(3);
    let after = harness.get_by_label("far").rect().left();
    assert!(after < before - 100.0, "{before} -> {after}");
}

#[test]
fn file_tree_previews_a_file_and_hands_off_to_the_editor() {
    let opener = FakeOpener::default();
    let (mut harness, _) = harness_with(opener.clone());
    let (dir, pid, _) = file_project(&mut harness);
    // Ignored directories stay out of the tree; folders open on click.
    assert!(harness.query_by_label("⏵ target").is_none());
    click(&mut harness, "⏵ docs");
    harness.get_by_label("  design.md");
    harness.get_by_label("Select a file to preview it here.");
    // A click previews in the side's bottom half and stays on the board;
    // Expand opens the full view.
    click(&mut harness, "  README.md");
    let readme = dir.path().join("README.md");
    harness.get_by_label("from the readme");
    assert_eq!(harness.state().core().view(), View::Board(pid));
    assert!(!actions(&harness).contains(&AppAction::ShowDocument(pid, readme.clone())));
    click(&mut harness, "Expand");
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
    let (_dir, pid, _) = file_project(&mut harness);
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
    harness.get_by_label("Design");
    assert_eq!(harness.state().core().view(), View::Board(pid));
    assert!(harness.query_by_label("target/out.bin").is_none());
}

#[test]
fn session_view_toggles_the_file_side_with_a_preview() {
    let (mut harness, _) = harness();
    let (_dir, pid, sid) = file_project(&mut harness);
    showing(&mut harness, View::Session(sid));
    assert!(harness.query_by_label("Find").is_none());
    let toggle = |harness: &mut Harness<'static, SwitchboardApp>| {
        harness.get_by_role_and_label(Role::Button, "Side").click();
        harness.run_steps(2);
    };
    toggle(&mut harness);
    harness.get_by_label("Find");
    click(&mut harness, "  README.md");
    harness.get_by_label("from the readme");
    // The preview lives in the side; the session stays on screen.
    assert_eq!(harness.state().core().view(), View::Session(sid));
    // The pane's × clears the selection.
    click(&mut harness, "×");
    harness.get_by_label("Select a file to preview it here.");
    assert!(harness.query_by_label("from the readme").is_none());
    toggle(&mut harness);
    assert!(harness.query_by_label("Find").is_none());
    // The rail's Files item flips the same state.
    click(&mut harness, "Files ⌘B");
    harness.get_by_label("Find");
    click(&mut harness, "Files ⌘B");
    assert!(harness.query_by_label("Find").is_none());
    // Only sessions have the toggle; boards always show the side, where
    // "Files" is the tab.
    showing(&mut harness, View::Board(pid));
    assert!(harness.query_by_label("Find").is_some());
}

#[test]
fn pinned_card_previews_and_opens() {
    let opener = FakeOpener::default();
    let (mut harness, _) = harness_with(opener.clone());
    let (dir, _pid, _) = file_project(&mut harness);
    // README.md is pinned by the seed.
    click(&mut harness, "Open in app");
    assert_eq!(opener.state().opened, vec![dir.path().join("README.md")]);
}

#[test]
fn the_boards_config_button_edits_project_json_and_save_writes_it() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Board(ids.alpha));
    click(&mut harness, "Config");
    // No file: the template, and a note that Save creates it.
    harness.get_by_label("No file yet; Save creates it.");
    harness.get_by_label_contains("Valid: 0 commands");
    {
        let draft = harness.state_mut().ui_state.config_dialog.as_mut().unwrap();
        draft.text = "{\"version\":1,\"show\":[\"manager\", \"../up\"]".into();
    }
    harness.run_steps(2);
    harness.get_by_label_contains("Not valid:");
    {
        let draft = harness.state_mut().ui_state.config_dialog.as_mut().unwrap();
        draft.text = "{\"version\":1,\"show\":[\"manager\", \"../up\"]}".into();
    }
    harness.run_steps(2);
    harness.get_by_label_contains("1 shown folders");
    harness.get_by_label_contains("show: \"../up\" skipped");
    click(&mut harness, "Save");
    assert!(harness.state().ui_state.config_dialog.is_none());
    assert!(actions(&harness).iter().any(|a| matches!(
        a,
        AppAction::SaveProjectConfig { project, text } if *project == ids.alpha && text.contains("manager")
    )));
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
fn run_tab_lists_definitions_and_approves() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Board(ids.alpha));
    // The board's command card shows the command line; the Run tab
    // shows it again with the definition.
    assert_eq!(harness.query_all_by_label("cargo clippy").count(), 1);
    click(&mut harness, "Run");
    assert!(actions(&harness).contains(&AppAction::SetSideTab(SideTab::Run)));
    assert_eq!(harness.query_all_by_label("cargo clippy").count(), 2);
    harness.get_by_label_contains("RUSTFLAGS (not defined)");
    harness.get_by_label("needs approval");
    // The user's own service is listed too (also on the board's cards).
    assert!(harness.query_all_by_label("deploy").next().is_some());
    click(&mut harness, "Approve");
    assert!(actions(&harness).contains(&AppAction::ApproveDefinition(ids.lint)));
    let lint = harness.state().core().session(ids.lint).unwrap().clone();
    assert_eq!(lint.approved_hash.as_deref(), Some("h1"));
    assert_eq!(lint.approval(), Approval::Approved);
    harness.get_by_label("approved");
    click(&mut harness, "Revoke");
    assert!(actions(&harness).contains(&AppAction::RevokeApproval(ids.lint)));
    harness.get_by_label("needs approval");
    // Back to the files.
    click(&mut harness, "Files");
    harness.get_by_label("Find");
}

#[test]
fn cmd_r_opens_the_run_tab_beside_a_session() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Session(ids.server));
    assert!(harness.query_by_label("cargo clippy").is_none());
    let press = |harness: &mut Harness<'static, SwitchboardApp>, key| {
        harness.key_press_modifiers(egui::Modifiers::COMMAND, key);
        harness.run_steps(2);
    };
    press(&mut harness, egui::Key::R);
    let dispatched = actions(&harness);
    assert!(dispatched.contains(&AppAction::SetSideTab(SideTab::Run)));
    assert!(dispatched.contains(&AppAction::SetFilesOpen(true)));
    harness.get_by_label("cargo clippy");
    // Cmd+B switches the open side to Files; a second Cmd+B closes it,
    // and Cmd+R then reopens it straight on Run.
    press(&mut harness, egui::Key::B);
    harness.get_by_label("Find");
    assert!(harness.query_by_label("cargo clippy").is_none());
    press(&mut harness, egui::Key::B);
    assert!(harness.query_by_label("Find").is_none());
    assert!(!harness.state().core().settings().files_open);
    press(&mut harness, egui::Key::R);
    harness.get_by_label("cargo clippy");
    press(&mut harness, egui::Key::R);
    assert!(harness.query_by_label("cargo clippy").is_none());
    // An unapproved command is refused even if something asks for it.
    harness
        .state_mut()
        .dispatch(AppAction::RestartSession(ids.lint));
    harness.run_steps(2);
    harness.get_by_label_contains("not approved");
}

#[test]
fn exited_command_shows_its_last_output() {
    let (mut harness, ids) = harness();
    let dir = MemoryStore::default().data_dir().join("scrollback");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{}.vt", ids.deploy.host_name()));
    std::fs::write(&path, "npm ERR! deploy failed\n").unwrap();
    showing(&mut harness, View::Session(ids.deploy));
    harness.state_mut().poll_now();
    harness.run_steps(2);
    harness.get_by_label_contains("last output kept on disk");
    assert!(
        harness
            .query_all_by_value("npm ERR! deploy failed")
            .next()
            .is_some()
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn service_header_offers_restart_and_autostart() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Session(ids.deploy));
    click(&mut harness, "▶ Start");
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
    // The title and the rail row both carry the new name.
    assert_eq!(harness.query_all_by_label("server v2").count(), 2);
}

#[test]
fn session_view_header_buttons_dispatch() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Session(ids.server));
    // The title, and the rail row for the same session.
    assert_eq!(harness.query_all_by_label("server").count(), 2);
    harness.get_by_label("shell");
    harness.get_by_label("/work/server");
    harness.get_by_label("Embedded terminal disabled.");
    click(&mut harness, "Open in terminal");
    click(&mut harness, "Kill");
    // Back lives in the session header; the rail offers the board.
    assert_eq!(
        harness
            .get_all_by_role_and_label(Role::Button, "Back")
            .count(),
        1
    );
    harness.get_by_role_and_label(Role::Button, "← Board");
    // The rail keeps agents and shells apart from commands and services,
    // in that order, as the board does.
    let shell = harness.get_all_by_label("server").last().unwrap().rect();
    let label = harness.get_by_label("COMMANDS AND SERVICES").rect();
    let command = harness.get_by_role_and_label(Role::Button, "build").rect();
    assert!(shell.bottom() <= label.top() && label.bottom() <= command.top());
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
    assert!(
        harness.query_by_label("Session notes").is_none(),
        "notes live in the side"
    );
    // The rail item opens the side on the Notes tab.
    click(&mut harness, "Notes ⌘N");
    harness.get_by_label("Session notes");
    type_into(&mut harness, "Session notes", "flaky on CI");
    let dispatched = actions(&harness);
    assert!(
        dispatched
            .iter()
            .any(|a| matches!(a, AppAction::SetSessionNotes(id, text) if *id == ids.build && text.ends_with("flaky on CI"))),
        "got {dispatched:?}"
    );
    // The same item closes the side again.
    click(&mut harness, "Notes ⌘N");
    assert!(!harness.state().core().settings().files_open);
}

#[test]
fn the_notes_tab_is_only_offered_beside_a_session() {
    let (mut harness, ids) = harness();
    harness
        .state_mut()
        .dispatch(AppAction::SetSideTab(SideTab::Notes));
    harness.state_mut().dispatch(AppAction::SetFilesOpen(true));
    showing(&mut harness, View::Session(ids.build));
    harness.get_by_label("Session notes");
    // A board keeps the choice but shows Files.
    showing(&mut harness, View::Board(ids.alpha));
    assert!(harness.query_by_label("Notes").is_none());
    harness.get_by_label("Find");
    assert_eq!(harness.state().core().settings().side_tab, SideTab::Notes);
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
    // The summary line of the all-sessions view.
    harness.get_by_label("1 waiting on you");
    // The card's kicker names the state; its body says why.
    harness.get_by_label_contains("WAITING ON YOU");
    harness.get_by_label("permission for Bash");
}

#[test]
fn waiting_reason_is_shown_in_the_session_header() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Session(ids.server));
    harness.get_by_label("waiting on you: permission for Bash");
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
            path: None,
        }),
        text: None,
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
        last_usage: Some(Usage {
            input: 4_000,
            output: 12,
            cache_read: 80_000,
            cache_create: 0,
        }),
    }
}

#[test]
fn claude_session_shows_the_conversation_and_message_box() {
    let (mut harness, ids) = harness();
    plain_message_box(&mut harness);
    let id = seed_claude(&mut harness, &ids);
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, two_turns()));
    showing(&mut harness, View::Session(id));
    // Claude's own name for the conversation is shown only while it
    // differs from the record's name.
    harness.get_by_label("Claude: explain-repo");
    harness
        .state_mut()
        .dispatch(AppAction::RenameSession(id, "explain-repo".into()));
    harness.run_steps(2);
    assert!(harness.query_by_label_contains("Claude: ").is_none());
    harness
        .state_mut()
        .dispatch(AppAction::RenameSession(id, "claude-agent".into()));
    harness.run_steps(2);
    harness.get_by_label("reply with the single word pong");
    harness.get_by_label("What is the crate called?");
    harness.get_by_label("pong");
    harness.get_by_label("The crate is called switchboard.");
    harness.get_by_label("2 msgs · 1 tools · 2m");
    harness.get_by_label_contains("ctx 84k / 1000k (8%)");
    harness.get_by_label("Message");
    harness.get_by_role_and_label(Role::Button, "Send");
    harness.get_by_role_and_label(Role::Button, "Stop");
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
fn terminal_panel_toggles_from_the_header() {
    let (mut harness, ids) = harness();
    let id = seed_claude(&mut harness, &ids);
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, two_turns()));
    harness
        .state_mut()
        .ui_state
        .snapshots
        .insert(id, "$ raw pane text".into());
    showing(&mut harness, View::Session(id));
    assert!(
        harness
            .query_all_by_value("$ raw pane text")
            .next()
            .is_none()
    );
    click(&mut harness, "Terminal");
    assert!(
        harness
            .query_all_by_value("$ raw pane text")
            .next()
            .is_some()
    );
    click(&mut harness, "Hide");
    assert!(
        harness
            .query_all_by_value("$ raw pane text")
            .next()
            .is_none()
    );
    assert!(!harness.state().ui_state.terminal_open);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::T);
    harness.run_steps(2);
    assert!(harness.state().ui_state.terminal_open);
}

#[test]
fn messages_before_the_answer_are_their_own_blocks_with_the_tools_between() {
    let (mut harness, ids) = harness();
    let id = seed_claude(&mut harness, &ids);
    let mut conversation = two_turns();
    let tool = conversation.turns[1].activity[0].clone();
    let mut later = tool.clone();
    later.line = "Edit: src/lib.rs".into();
    let message = "Findings so far: the crate is named switchboard.".to_owned();
    conversation.turns[1].activity = vec![
        tool,
        TranscriptActivity {
            kind: ActivityKind::Text,
            line: "Findings so far: …".into(),
            at: Some(at(11)),
            error: false,
            detail: None,
            text: Some(message.clone()),
        },
        later,
    ];
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, conversation));
    showing(&mut harness, View::Session(id));
    // The message reads in full without expanding anything; the tools
    // around it stay folded in two groups.
    harness.get_by_label(message.as_str());
    assert!(harness.query_by_label("Findings so far: …").is_none());
    assert!(
        harness
            .query_by_label_contains("Bash: Read crate name")
            .is_none()
    );
    assert!(
        harness
            .query_by_label_contains("Edit: src/lib.rs")
            .is_none()
    );
    assert_eq!(harness.query_all_by_label_contains("1 tools").count(), 2);
    click(&mut harness, "Expand activity");
    harness.get_by_label_contains("Bash: Read crate name");
    harness.get_by_label_contains("Edit: src/lib.rs");
}

#[test]
fn long_activity_lines_wrap_instead_of_being_cut() {
    let (mut harness, ids) = harness();
    let id = seed_claude(&mut harness, &ids);
    let long = "word ".repeat(80).trim_end().to_owned();
    let mut conversation = two_turns();
    conversation.turns[1].activity.push(TranscriptActivity {
        kind: ActivityKind::Text,
        line: long.clone(),
        at: None,
        error: false,
        detail: None,
        text: Some(long.clone()),
    });
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, conversation));
    showing(&mut harness, View::Session(id));
    click(&mut harness, "Expand activity");
    let row = harness.get_by_label(long.as_str());
    let one_line = 14.0 * 2.0;
    assert!(
        row.rect().height() > one_line,
        "a 400-character line should take several rows, got {}",
        row.rect().height()
    );
}

#[test]
fn stop_button_and_cmd_period_interrupt() {
    let (mut harness, ids) = harness();
    plain_message_box(&mut harness);
    let id = seed_claude(&mut harness, &ids);
    showing(&mut harness, View::Session(id));
    click(&mut harness, "Stop");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Period);
    harness.run_steps(2);
    let interrupts = actions(&harness)
        .iter()
        .filter(|a| **a == AppAction::Interrupt(id))
        .count();
    assert_eq!(interrupts, 2);
}

#[test]
fn message_box_is_multiline_and_enter_sends() {
    let (mut harness, ids) = harness();
    plain_message_box(&mut harness);
    let id = seed_claude(&mut harness, &ids);
    showing(&mut harness, View::Session(id));
    let field = harness.get_by_label("Message");
    field.focus();
    field.type_text("first line");
    harness.run_steps(2);
    harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Enter);
    harness.run_steps(2);
    harness.get_by_label("Message").type_text("second line");
    harness.run_steps(2);
    // Nothing has been sent yet; the draft survives a trip to the board.
    assert!(actions(&harness).is_empty());
    showing(&mut harness, View::Board(ids.beta));
    showing(&mut harness, View::Session(id));
    assert_eq!(
        harness
            .state()
            .ui_state
            .input_drafts
            .get(&id)
            .map(String::as_str),
        Some("first line\nsecond line")
    );
    harness.get_by_label("Message").focus();
    harness.run_steps(2);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    assert!(actions(&harness).contains(&AppAction::SendInput {
        id,
        text: "first line\nsecond line".into()
    }));
    // The fake pane took it, so the draft is gone (the box re-creates an
    // empty one on the next frame).
    assert_eq!(
        harness
            .state()
            .ui_state
            .input_drafts
            .get(&id)
            .map_or("", String::as_str),
        ""
    );
}

#[test]
fn a_failed_send_keeps_the_draft() {
    let host = FakeHost::default();
    host.state().fail_write = Some("pane is dead".into());
    let (mut harness, ids) = harness_with_host(host);
    plain_message_box(&mut harness);
    let id = seed_claude(&mut harness, &ids);
    showing(&mut harness, View::Session(id));
    let field = harness.get_by_label("Message");
    field.focus();
    field.type_text("do not lose me");
    harness.run_steps(2);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    assert!(actions(&harness).contains(&AppAction::SendInput {
        id,
        text: "do not lose me".into()
    }));
    assert_eq!(
        harness
            .state()
            .ui_state
            .input_drafts
            .get(&id)
            .map(String::as_str),
        Some("do not lose me")
    );
    harness.get_by_label_contains("send input");
    assert_eq!(
        harness.get_by_label("Message").value().as_deref(),
        Some("do not lose me")
    );
}

#[test]
fn a_file_row_dragged_onto_the_message_box_adds_its_path() {
    let (mut harness, _) = harness();
    plain_message_box(&mut harness);
    let (dir, _pid, sid) = file_project(&mut harness);
    showing(&mut harness, View::Session(sid));
    harness.get_by_role_and_label(Role::Button, "Side").click();
    harness.run_steps(2);
    let from = harness.get_by_label("  README.md").rect().center();
    let to = harness.get_by_label("Message").rect().center();
    // Press on the row, move well past the drag threshold, release over
    // the message panel.
    harness.event(egui::Event::PointerMoved(from));
    harness.event(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run_steps(2);
    for k in 1..=4u8 {
        harness.event(egui::Event::PointerMoved(from.lerp(to, f32::from(k) / 4.0)));
        harness.run_steps(2);
    }
    harness.event(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run_steps(2);
    assert_eq!(
        harness
            .state()
            .ui_state
            .input_drafts
            .get(&sid)
            .map(String::as_str),
        Some(dir.path().join("README.md").display().to_string().as_str())
    );
}

#[test]
fn a_directory_can_become_the_file_sides_top_and_the_project_root_comes_back() {
    let (mut harness, _) = harness();
    let (dir, pid, _) = file_project(&mut harness);
    harness.get_by_label("⏵ docs").click_secondary();
    harness.run_steps(2);
    click(&mut harness, "Show as top level");
    assert!(actions(&harness).contains(&AppAction::SetFileRoot(pid, Some(PathBuf::from("docs")))));
    assert_eq!(
        harness.state().core().file_root(pid),
        Some(&PathBuf::from("docs"))
    );
    harness.run_steps(2);
    // The tell names the directory, the tree starts inside it, and the
    // finder searches only there.
    harness.get_by_label("⏵ docs");
    harness.get_by_label("  design.md");
    assert!(harness.query_by_label("  README.md").is_none());
    let find = harness.get_by_label("Find");
    find.focus();
    find.type_text("md");
    // The index is built on a thread; a few frames let it land.
    for _ in 0..50 {
        harness.run_steps(2);
        if harness.query_by_label("docs/design.md").is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    harness.get_by_label("docs/design.md");
    // The board's pinned README card is the one "README.md" on screen;
    // the finder adds none while narrowed, one once the root is back.
    assert_eq!(harness.query_all_by_label("README.md").count(), 1);
    click(&mut harness, "Project root");
    assert!(actions(&harness).contains(&AppAction::SetFileRoot(pid, None)));
    assert!(harness.state().core().file_root(pid).is_none());
    harness.run_steps(2);
    assert_eq!(harness.query_all_by_label("README.md").count(), 2);
    assert!(harness.query_by_label("Project root").is_none());
    drop(dir);
}

#[test]
fn shift_click_the_menu_and_the_pane_put_a_path_in_the_message() {
    let (mut harness, _) = harness();
    plain_message_box(&mut harness);
    let (dir, pid, sid) = file_project(&mut harness);
    let readme = dir.path().join("README.md").display().to_string();
    // On a board there is no message box, so none of the ways show up.
    click(&mut harness, "  README.md");
    assert!(harness.query_by_label("To message").is_none());
    harness.get_by_label("  README.md").click_secondary();
    harness.run_steps(2);
    assert!(harness.query_by_label("Add path to message").is_none());
    harness.get_by_label("Preview").click();
    harness.run_steps(2);
    assert_eq!(
        harness.state().core().view(),
        View::Document(pid, dir.path().join("README.md"))
    );

    showing(&mut harness, View::Session(sid));
    harness.get_by_role_and_label(Role::Button, "Side").click();
    harness.run_steps(2);
    harness
        .get_by_label("  README.md")
        .click_modifiers(egui::Modifiers::SHIFT);
    harness.run_steps(2);
    harness.get_by_label("  README.md").click_secondary();
    harness.run_steps(2);
    click(&mut harness, "Add path to message");
    click(&mut harness, "  README.md");
    click(&mut harness, "To message");
    assert_eq!(
        harness
            .state()
            .ui_state
            .input_drafts
            .get(&sid)
            .map(String::as_str),
        Some(format!("{readme} {readme} {readme}").as_str())
    );
}

#[test]
fn saving_from_the_prompt_box_refreshes_the_projects_file_tree() {
    let (mut harness, _) = harness();
    let (dir, pid, sid) = file_project(&mut harness);
    let target = dir.path().join("notes.md");
    harness.state_mut().ui_state.prompt_boxes.test_save_path = Some(target);
    showing(&mut harness, View::Session(sid));
    harness.get_by_role_and_label(Role::Button, "Side").click();
    harness.run_steps(2);
    harness.get_by_label("  README.md");
    let field = prompt_field(&mut harness);
    field.focus();
    field.type_text("a note");
    harness.run_steps(2);
    assert!(
        !harness.state().ui_state.files[&pid].children.is_empty(),
        "the tree was read"
    );
    // The fake saver does not write; the file appears as a real save
    // would leave it, and the cached tree does not know it yet.
    std::fs::write(dir.path().join("notes.md"), "a note").unwrap();
    harness.run_steps(2);
    assert!(harness.query_by_label("  notes.md").is_none());
    click(&mut harness, "Save…");
    harness.get_by_label("  notes.md");
    harness.get_by_label("  README.md");
}

#[test]
fn the_file_side_hands_a_path_to_the_prompt_box_editor() {
    let (mut harness, _) = harness();
    let (dir, _, sid) = file_project(&mut harness);
    let readme = dir.path().join("README.md").display().to_string();
    showing(&mut harness, View::Session(sid));
    harness.get_by_role_and_label(Role::Button, "Side").click();
    harness.run_steps(2);
    click(&mut harness, "  README.md");
    click(&mut harness, "To message");
    assert_eq!(
        prompt_field(&mut harness).value().as_deref(),
        Some(readme.as_str())
    );
    assert!(
        !harness.state().ui_state.input_drafts.contains_key(&sid),
        "nothing left in the plain draft"
    );
}

#[test]
fn messages_have_a_context_menu_with_copy() {
    let (mut harness, ids) = harness();
    plain_message_box(&mut harness);
    let id = seed_claude(&mut harness, &ids);
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, two_turns()));
    showing(&mut harness, View::Session(id));
    assert!(harness.query_by_label("Copy").is_none());
    // The prompt and the answer each get the menu.
    harness
        .get_by_label("What is the crate called?")
        .click_secondary();
    harness.run_steps(2);
    harness.get_by_label("Copy").click();
    harness.step();
    let copied = harness
        .output()
        .platform_output
        .commands
        .iter()
        .any(|c| *c == egui::OutputCommand::CopyText("What is the crate called?".into()));
    assert!(copied, "{:?}", harness.output().platform_output.commands);
    harness.run_steps(2);
    assert!(harness.query_by_label("Copy").is_none());
    harness.get_by_label("pong").click_secondary();
    harness.run_steps(2);
    harness.get_by_role_and_label(Role::Button, "Copy");
}

#[test]
fn discard_to_a_prompt_cuts_in_place_and_the_header_offers_undo() {
    let (mut harness, ids) = harness();
    let id = seed_claude(&mut harness, &ids);
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, two_turns()));
    showing(&mut harness, View::Session(id));
    assert!(harness.query_by_label("Undo discard").is_none());
    let original = harness.state().core().session(id).unwrap().resume.clone();
    harness
        .get_by_label("What is the crate called?")
        .click_secondary();
    harness.run_steps(2);
    click(&mut harness, "Discard to here");
    assert_eq!(
        actions(&harness),
        vec![AppAction::DiscardTo {
            id,
            before: 2,
            prompt: "What is the crate called?".into(),
        }]
    );
    {
        let app = harness.state();
        let record = app.core().session(id).unwrap();
        assert_ne!(record.resume, original, "resumes through the cut copy");
        assert_eq!(record.discard.as_ref().map(|d| d.before), Some(2));
        assert_eq!(app.core().view(), View::Session(id), "same session");
        assert!(
            !app.ui_state.conversations.contains_key(&id),
            "cache dropped"
        );
    }
    // The cut conversation, as the next read would load it.
    let mut cut = two_turns();
    cut.turns.truncate(1);
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, cut));
    harness.run_steps(2);
    assert_eq!(
        prompt_field(&mut harness).value().as_deref(),
        Some("What is the crate called?"),
        "the prompt is ready to send again"
    );
    // The discard's toast sits over the header's right end, where the
    // button now is; a real user waits it out or dismisses it.
    harness.state_mut().dispatch(AppAction::DismissNotice);
    harness.run_steps(2);
    harness.state_mut().dispatched.clear();
    click(&mut harness, "Undo discard");
    assert_eq!(actions(&harness), vec![AppAction::UndoDiscard(id)]);
    let app = harness.state();
    let record = app.core().session(id).unwrap();
    assert_eq!(record.resume, original);
    assert!(record.discard.is_none());
    harness.run_steps(2);
    assert!(harness.query_by_label("Undo discard").is_none());
}

#[test]
fn clone_session_on_a_prompt_makes_a_new_session_with_that_prompt_primed() {
    let (mut harness, ids) = harness();
    let id = seed_claude(&mut harness, &ids);
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, two_turns()));
    showing(&mut harness, View::Session(id));
    // The agent's answers only copy or show raw; the user's prompts fork.
    harness.get_by_label("pong").click_secondary();
    harness.run_steps(2);
    assert!(harness.query_by_label("Clone session").is_none());
    harness.get_by_label("View raw").click();
    harness.run_steps(2);
    harness.state_mut().ui_state.raw_message = None;
    harness.run_steps(2);
    harness
        .get_by_label("What is the crate called?")
        .click_secondary();
    harness.run_steps(2);
    click(&mut harness, "Clone session");
    let prompt = "What is the crate called?".to_owned();
    assert_eq!(
        actions(&harness),
        vec![AppAction::CloneSession {
            id,
            before: 2,
            prompt: prompt.clone(),
        }]
    );
    let app = harness.state();
    let clone = app
        .core()
        .workspaces()
        .iter()
        .flat_map(|w| &w.sessions)
        .find(|s| s.name == "claude-agent clone")
        .expect("the clone record");
    assert_eq!(app.core().view(), View::Session(clone.id));
    assert_ne!(clone.resume, app.core().session(id).unwrap().resume);
    let clone_id = clone.id;
    harness.run_steps(2);
    assert!(
        harness.query_all_by_value(prompt.as_str()).next().is_some(),
        "the message box shows the primed prompt"
    );
    // The Prompt Box editor took the primed draft as its text.
    let boxes = &harness.state().ui_state.prompt_boxes;
    assert_eq!(boxes.editors[&clone_id].core().doc().committed(), prompt);
}

#[test]
fn view_links_lists_a_messages_urls_in_a_dialog() {
    let (mut harness, ids) = harness();
    plain_message_box(&mut harness);
    let id = seed_claude(&mut harness, &ids);
    let mut conversation = two_turns();
    conversation.turns[0].final_text =
        "Read https://docs.rs/egui and [the repo](https://github.com/emilk/egui).".into();
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, conversation));
    showing(&mut harness, View::Session(id));
    // A message without a link does not offer the item.
    harness
        .get_by_label("The crate is called switchboard.")
        .click_secondary();
    harness.run_steps(2);
    assert!(harness.query_by_label("View links").is_none());
    harness.get_by_label("Copy").click();
    harness.run_steps(2);
    harness
        .query_all_by_label_contains("docs.rs/egui")
        .next()
        .expect("the answer")
        .click_secondary();
    harness.run_steps(2);
    harness.get_by_label("View links").click();
    harness.run_steps(2);
    harness.get_by_label("Links in message");
    assert_eq!(
        harness.state().ui_state.message_links.as_deref(),
        Some(
            &[
                "https://docs.rs/egui".to_owned(),
                "https://github.com/emilk/egui".to_owned()
            ][..]
        )
    );
    harness.get_by_role_and_label(Role::Link, "https://docs.rs/egui");
    harness.get_by_label("Copy all").click();
    harness.step();
    let copied = harness.output().platform_output.commands.iter().any(|c| {
        *c == egui::OutputCommand::CopyText(
            "https://docs.rs/egui\nhttps://github.com/emilk/egui".into(),
        )
    });
    assert!(copied);
    harness.run_steps(2);
    harness.get_by_label("Close").click();
    harness.run_steps(2);
    assert!(harness.query_by_label("Links in message").is_none());
    assert!(harness.state().ui_state.message_links.is_none());
}

#[test]
fn view_raw_shows_the_message_unformatted_in_a_dialog() {
    let (mut harness, ids) = harness();
    plain_message_box(&mut harness);
    let id = seed_claude(&mut harness, &ids);
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, two_turns()));
    showing(&mut harness, View::Session(id));
    assert!(harness.query_by_label("Full message").is_none());
    harness.get_by_label("pong").click_secondary();
    harness.run_steps(2);
    harness.get_by_label("View raw").click();
    harness.run_steps(2);
    harness.get_by_label("Full message");
    assert_eq!(
        harness.state().ui_state.raw_message.as_deref(),
        Some("pong")
    );
    // Rendered shows the Markdown drawn; Raw the text as it is. The
    // choice is kept for the next message.
    assert_eq!(
        harness.state().ui_state.message_view,
        switchboard::ui::dialogs::MessageView::Rendered,
        "rendered by default"
    );
    click(&mut harness, "Raw");
    assert_eq!(
        harness.state().ui_state.message_view,
        switchboard::ui::dialogs::MessageView::Raw
    );
    click(&mut harness, "Rendered");
    assert_eq!(
        harness.state().ui_state.message_view,
        switchboard::ui::dialogs::MessageView::Rendered
    );
    // The dialog's Copy puts the same text on the clipboard.
    harness.get_by_role_and_label(Role::Button, "Copy").click();
    harness.step();
    let copied = harness
        .output()
        .platform_output
        .commands
        .iter()
        .any(|c| *c == egui::OutputCommand::CopyText("pong".into()));
    assert!(copied);
    harness.run_steps(2);
    harness.get_by_label("Close").click();
    harness.run_steps(2);
    assert!(harness.query_by_label("Full message").is_none());
    assert!(harness.state().ui_state.raw_message.is_none());
}

#[test]
fn claude_session_without_a_conversation_falls_back_to_the_snapshot() {
    let (mut harness, ids) = harness();
    plain_message_box(&mut harness);
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
        project_config: Box::new(FakeProjectConfig::default()),
        round_files: Box::new(FakeRoundFiles::default()),
        artifacts: Box::new(FakeArtifacts::default()),
        wake: None,
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_eframe(move |cc| {
            switchboard::ui::theme::install(&cc.egui_ctx);
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
    // A board reads the transcripts of its agent cards too: the card
    // excerpts the last answer, never the pane's own chrome.
    harness.state_mut().ui_state.conversations.clear();
    harness
        .state_mut()
        .ui_state
        .captions
        .insert(id, "new task? /clear to save 465.5k tokens".into());
    showing(&mut harness, View::Board(ids.beta));
    harness.state_mut().poll_now();
    harness.run_steps(2);
    harness.get_by_label("The crate is called switchboard.");
    assert!(harness.query_by_label_contains("/clear to save").is_none());
}

#[test]
fn run_tab_beside_a_session_keeps_the_sides_width() {
    let (mut harness, ids) = harness();
    harness
        .state_mut()
        .ui_state
        .snapshots
        .insert(ids.lint, "a line of output\n".repeat(5));
    showing(&mut harness, View::Session(ids.server));
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::R);
    harness.run_steps(2);
    let before = harness.get_by_label("Run").rect().left();
    harness.run_steps(6);
    let after = harness.get_by_label("Run").rect().left();
    assert!(
        (before - after).abs() < 0.5,
        "the side grew from {before} to {after}"
    );
}

#[test]
fn conversation_wraps_beside_the_open_side() {
    let (mut harness, ids) = harness();
    let id = seed_claude(&mut harness, &ids);
    let mut conversation = two_turns();
    conversation.turns[0].final_text = "word ".repeat(120).trim_end().to_owned();
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, conversation));
    showing(&mut harness, View::Session(id));
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::B);
    harness.run_steps(2);
    let side = harness.get_by_label("Find").rect().left();
    let answer = harness
        .get_by_label("The crate is called switchboard.")
        .rect();
    let long = harness
        .query_all_by_label_contains("word word")
        .map(|n| n.rect().right())
        .fold(0.0_f32, f32::max);
    assert!(
        answer.right() < side,
        "{answer:?} runs under the side at {side}"
    );
    assert!(
        long < side,
        "the long answer reaches {long}, past the side at {side}"
    );
}

#[test]
fn rail_item_and_side_tab_open_and_close_the_side_and_the_text_reflows() {
    let (mut harness, ids) = harness();
    let id = seed_claude(&mut harness, &ids);
    let mut conversation = two_turns();
    conversation.turns[0].final_text = "word ".repeat(120).trim_end().to_owned();
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, conversation));
    showing(&mut harness, View::Session(id));
    let long_right = |h: &Harness<'static, SwitchboardApp>| {
        h.query_all_by_label_contains("word word")
            .map(|n| n.rect().right())
            .fold(0.0_f32, f32::max)
    };
    let is_open = |h: &Harness<'static, SwitchboardApp>| h.state().core().settings().files_open;
    let wide = long_right(&harness);
    // The rail's Files item opens the side and the answer wraps to it.
    click(&mut harness, "Files ⌘B");
    assert!(is_open(&harness));
    let side = harness.get_by_label("Find").rect().left();
    assert!(long_right(&harness) < side);
    // The same item closes it and the answer takes the width back.
    click(&mut harness, "Files ⌘B");
    assert!(!is_open(&harness));
    assert!((long_right(&harness) - wide).abs() < 1.0);
    // A click on the tab already showing closes the side too; a click on
    // the other tab only switches.
    click(&mut harness, "Files ⌘B");
    click(&mut harness, "Run");
    assert!(is_open(&harness));
    assert_eq!(harness.state().core().settings().side_tab, SideTab::Run);
    click(&mut harness, "Run");
    assert!(!is_open(&harness));
}

#[test]
fn a_word_that_cannot_break_does_not_widen_the_turns_after_it() {
    let (mut harness, ids) = harness();
    plain_message_box(&mut harness);
    let id = seed_claude(&mut harness, &ids);
    let mut conversation = two_turns();
    // Roughly 1400 px of one word in the first turn, prose in the second.
    conversation.turns[0].final_text = "x".repeat(200);
    conversation.turns[1].final_text = "word ".repeat(120).trim_end().to_owned();
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, conversation));
    showing(&mut harness, View::Session(id));
    let left = harness.get_by_label("Message").rect().left();
    let prose = harness
        .query_all_by_label_contains("word word")
        .map(|n| n.rect().right())
        .fold(0.0_f32, f32::max);
    assert!(prose > 0.0, "the second answer is not on screen");
    assert!(
        prose <= left + 860.0 + 40.0,
        "the second answer wraps at {} from {left}: the first turn's word widened it",
        prose - left
    );
    // And beside a wide side it wraps to what is visible.
    click(&mut harness, "Files ⌘B");
    widen_side(&mut harness, 500.0);
    let edge = harness.get_by_label("Find").rect().left() - 8.0;
    let prose = harness
        .query_all_by_label_contains("word word")
        .map(|n| n.rect().right())
        .fold(0.0_f32, f32::max);
    assert!(
        prose <= edge,
        "the second answer reaches {prose}, past the side at {edge}"
    );
}

/// Set the side panel's stored width, as a drag on its edge would.
fn widen_side(harness: &mut Harness<'static, SwitchboardApp>, left: f32) {
    let state = egui::containers::panel::PanelState {
        outer_rect: egui::Rect::from_min_max(egui::pos2(left, 0.0), egui::pos2(1200.0, 900.0)),
    };
    harness
        .ctx
        .data_mut(|d| d.insert_persisted(egui::Id::new("files"), state));
    harness.run_steps(4);
}

#[test]
fn a_session_shrinks_to_the_width_a_wide_side_leaves_it() {
    let (mut harness, ids) = harness();
    plain_message_box(&mut harness);
    let id = seed_claude(&mut harness, &ids);
    let mut conversation = two_turns();
    conversation.turns[0].final_text = "word ".repeat(120).trim_end().to_owned();
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, conversation));
    showing(&mut harness, View::Session(id));
    click(&mut harness, "Files ⌘B");
    click(&mut harness, "Expand activity");
    widen_side(&mut harness, 500.0);
    let edge = harness.get_by_label("Find").rect().left() - 8.0;
    assert!(edge < 520.0, "the side did not widen: edge at {edge}");
    // Nothing in the session view may reach under the side: neither the
    // header rows, nor the conversation, nor the message box.
    for label in ["Back", "Open in terminal", "Message", "Send", "Side"] {
        let right = harness.get_by_label(label).rect().right();
        assert!(
            right <= edge,
            "{label} reaches {right}, past the side at {edge}"
        );
    }
    for needle in ["word word", "ctx 84k", "Bash: Read crate name", "/work/"] {
        let right = harness
            .query_all_by_label_contains(needle)
            .map(|n| n.rect().right())
            .fold(0.0_f32, f32::max);
        assert!(right > 0.0, "{needle} is not on screen");
        assert!(
            right <= edge,
            "{needle} reaches {right}, past the side at {edge}"
        );
    }
    // And it takes the room back when the side narrows again.
    widen_side(&mut harness, 840.0);
    let wide = harness.get_by_label("Message").rect().right();
    assert!(wide > 700.0, "the message box stayed narrow at {wide}");
    // A smaller window squeezes the same way: nothing may reach past it.
    harness.set_size(egui::vec2(900.0, 600.0));
    harness.run_steps(4);
    let edge = harness.get_by_label("Find").rect().left() - 8.0;
    assert!(edge < 900.0, "the side is off screen at {edge}");
    for label in ["Back", "Open in terminal", "Message", "Send"] {
        let right = harness.get_by_label(label).rect().right();
        assert!(
            right <= edge,
            "{label} reaches {right}, past the side at {edge} in a 900 px window"
        );
    }
    let answer = harness
        .query_all_by_label_contains("word word")
        .map(|n| n.rect().right())
        .fold(0.0_f32, f32::max);
    assert!(
        answer <= edge,
        "the answer reaches {answer}, past the side at {edge} in a 900 px window"
    );
}

#[test]
fn a_long_excerpt_is_clamped_above_the_card_buttons() {
    let (mut harness, ids) = harness();
    let id = seed_claude(&mut harness, &ids);
    let mut conversation = two_turns();
    conversation.turns[1].final_text = "The answer is long ".repeat(20).trim_end().to_owned();
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, conversation));
    showing(&mut harness, View::Board(ids.beta));
    let excerpt = harness.get_by_label_contains("The answer is long").rect();
    assert!(
        harness.query_by_label_contains("…").is_some(),
        "the excerpt was not clamped"
    );
    let open = harness.get_by_role_and_label(Role::Button, "Open").rect();
    assert!(
        excerpt.bottom() <= open.top(),
        "the excerpt ({excerpt:?}) runs into the buttons ({open:?})"
    );
    // Two lines of 13 px italic, not three.
    assert!(
        excerpt.height() < 3.0 * 13.0,
        "{} px tall",
        excerpt.height()
    );
}

#[test]
fn a_long_file_name_is_cut_before_the_document_buttons() {
    let (mut harness, ids) = harness();
    let root = harness
        .state()
        .core()
        .workspace(ids.beta)
        .unwrap()
        .project
        .root
        .clone();
    let path = root.join(
        "a-really-long-file-name-that-nobody-would-ever-choose-for-a-document-but-here-we-are.md",
    );
    showing(&mut harness, View::Document(ids.beta, path));
    // Wide: the title is cut and every button sits to its right.
    harness.set_size(egui::vec2(1400.0, 600.0));
    harness.run_steps(4);
    let title = harness
        .query_all_by_label_contains("a-really-long")
        .map(|n| n.rect())
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap();
    let side = harness.get_by_label("Find").rect().left();
    assert!(
        title.right() < side,
        "title {title:?} runs under the side at {side}"
    );
    let mut lefts = Vec::new();
    for label in [
        "Open",
        "Open in editor",
        "Reveal",
        "Copy path",
        "Pin",
        "Back",
    ] {
        let r = harness.get_by_label(label).rect();
        assert!(
            r.left() >= title.right(),
            "{label} at {r:?} is under the title {title:?}"
        );
        assert!(r.right() <= side, "{label} at {r:?} is under the side");
        lefts.push(r.left());
    }
    assert!(lefts.windows(2).all(|w| w[0] < w[1]), "order {lefts:?}");
    // Narrow: the buttons take a row of their own, in the same order.
    harness.set_size(egui::vec2(900.0, 600.0));
    harness.run_steps(4);
    let title = harness
        .query_all_by_label_contains("a-really-long")
        .map(|n| n.rect())
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap();
    let side = harness.get_by_label("Find").rect().left();
    assert!(title.right() < side);
    let open = harness.get_by_label("Open").rect();
    let back = harness.get_by_label("Back").rect();
    assert!(
        open.top() >= title.bottom(),
        "Open {open:?} beside the title {title:?}"
    );
    assert!(back.right() <= side);
    assert!(
        open.top() < back.top() || open.left() < back.left(),
        "Open before Back"
    );
}

#[test]
fn a_markdown_table_keeps_its_columns_apart_and_inside_the_answer() {
    let (mut harness, ids) = harness();
    let id = seed_claude(&mut harness, &ids);
    let mut conversation = two_turns();
    conversation.turns[0].final_text = "Here is a table:\n\n\
| Key | Description | N |\n\
|---|---|---|\n\
| `alpha` | A description that is long enough to need wrapping when the column is squeezed by its neighbours in the row | 1 |\n\
| beta | Short | 22 |\n\
\nAnd a line after it."
        .to_owned();
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, conversation));
    showing(&mut harness, View::Session(id));
    click(&mut harness, "Expand activity");
    harness.set_size(egui::vec2(900.0, 700.0));
    harness.run_steps(4);
    let rect = |h: &Harness<'static, SwitchboardApp>, text: &str| {
        h.query_all_by_label_contains(text)
            .map(|n| n.rect())
            .reduce(egui::Rect::union)
            .unwrap_or_else(|| panic!("{text} is not on screen"))
    };
    let answer_left = rect(&harness, "Here is a table").left();
    let limit = (answer_left + 860.0).min(900.0 - 16.0);
    for text in [
        "Key",
        "Description",
        "alpha",
        "wrapping when",
        "beta",
        "Short",
        "22",
    ] {
        let r = rect(&harness, text);
        assert!(r.right() <= limit, "{text} at {r:?} is past {limit}");
    }
    // Columns do not overlap: each cell starts after the one before it.
    let alpha = rect(&harness, "alpha");
    let desc = rect(&harness, "A description");
    let one = rect(&harness, "wrapping when");
    // The exact cell: a looser match can pick a row-wide node.
    let n = harness
        .query_all_by_label("22")
        .map(|n| n.rect())
        .find(|r| r.top() > alpha.top())
        .unwrap();
    assert!(
        desc.left() >= alpha.right(),
        "description {desc:?} over key {alpha:?}"
    );
    assert!(
        n.left() >= one.right(),
        "number {n:?} over description {one:?}"
    );
    // The long cell wrapped instead of taking the row; the short
    // columns stayed narrow.
    assert!(
        desc.height() > 30.0,
        "the description did not wrap: {desc:?}"
    );
    assert!(
        alpha.width() < 120.0,
        "the key column is too wide: {alpha:?}"
    );
}

/// The first working set's id.
fn first_set(h: &Harness<'static, SwitchboardApp>) -> switchboard::core::SetId {
    h.state().core().working_sets()[0].id
}

#[test]
fn the_working_set_takes_a_session_from_its_header_and_a_file_from_the_tree() {
    let (mut harness, ids) = harness();
    let id = seed_claude(&mut harness, &ids);
    harness.set_size(egui::vec2(1200.0, 800.0));
    harness.run_steps(2);
    // None at first; the rail makes one and shows it, empty.
    harness.get_by_label("WORKING SETS");
    click(&mut harness, "+ New working set");
    harness.run_steps(2);
    let set = first_set(&harness);
    assert_eq!(harness.state().core().view(), View::WorkingSet(set));
    harness.get_by_label("Nothing here yet.");
    // The session header's menu lists the set; choosing it adds the
    // session, and the card appears at the working set's size.
    showing(&mut harness, View::Session(id));
    click(&mut harness, "Working sets");
    click(&mut harness, "   Working Set");
    harness.run_steps(2);
    assert_eq!(
        harness.state().core().sets_holding(&PinTarget::Session(id)),
        vec![set]
    );
    showing(&mut harness, View::WorkingSet(set));
    harness.get_by_label("1 card");
    let card = harness.get_by_label("claude-agent").rect();
    let open = harness.get_by_label("Open").rect();
    assert!(
        open.top() > card.bottom() + 150.0,
        "the card is tall: {card:?} {open:?}"
    );
    // The card's menu shows the set checked; choosing it takes the
    // session off again.
    harness.get_by_label("claude-agent").click_secondary();
    harness.run_steps(2);
    click(&mut harness, "✓ Working Set");
    harness.run_steps(2);
    harness.get_by_label("Nothing here yet.");
    // A file joins from the tree's submenu and shows as a file card.
    let (_dir, pid, _) = file_project(&mut harness);
    harness.get_by_label("  README.md").click_secondary();
    harness.run_steps(2);
    // The tree's menu holds the sets as a submenu.
    harness.get_by_label("Working sets ⏵").hover();
    harness.run_steps(2);
    click(&mut harness, "   Working Set");
    harness.run_steps(2);
    assert_eq!(
        harness
            .state()
            .core()
            .sets_holding(&PinTarget::File(pid, "README.md".into())),
        vec![set]
    );
    showing(&mut harness, View::WorkingSet(set));
    harness.get_by_label("README.md");
    click(&mut harness, "Take off");
    harness.run_steps(2);
    harness.get_by_label("Nothing here yet.");
}

#[test]
fn working_sets_are_renamed_cloned_and_deleted_from_the_header() {
    let (mut harness, ids) = harness();
    let (id, _) = working_set_of_two(&mut harness, &ids);
    let set = first_set(&harness);
    // "New working set with this" from a card menu makes a second set.
    harness.get_by_label("claude-agent").click_secondary();
    harness.run_steps(2);
    click(&mut harness, "New working set with this");
    harness.run_steps(2);
    let sets = harness.state().core().working_sets().to_vec();
    assert_eq!(sets.len(), 2);
    assert_eq!(sets[1].name, "Working Set 2");
    assert_eq!(harness.state().core().view(), View::WorkingSet(sets[1].id));
    assert_eq!(
        harness.state().core().sets_holding(&PinTarget::Session(id)),
        vec![set, sets[1].id]
    );
    // Rename in the header: Enter commits.
    click(&mut harness, "Rename");
    harness.run_steps(2);
    let field = harness.get_by_label("Working set name");
    field.focus();
    harness.run_steps(1);
    harness
        .get_by_label("Working set name")
        .type_text(" (hotfix)");
    harness.step();
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    assert_eq!(
        harness.state().core().working_sets()[1].name,
        "Working Set 2 (hotfix)"
    );
    assert!(harness.query_all_by_label("Working Set 2 (hotfix)").count() >= 1);
    // Clone copies the cards and shows the copy.
    click(&mut harness, "Clone");
    harness.run_steps(2);
    let sets = harness.state().core().working_sets().to_vec();
    assert_eq!(sets.len(), 3);
    assert_eq!(sets[2].name, "Working Set 2 (hotfix) copy");
    assert_eq!(sets[2].items.len(), 1);
    assert_eq!(harness.state().core().view(), View::WorkingSet(sets[2].id));
    // Delete asks first; Cancel keeps it, Delete drops it and goes back.
    click(&mut harness, "Delete");
    harness.run_steps(2);
    harness.get_by_label("Delete working set");
    click(&mut harness, "Cancel");
    harness.run_steps(2);
    assert_eq!(harness.state().core().working_sets().len(), 3);
    click(&mut harness, "Delete");
    harness.run_steps(2);
    click(&mut harness, "Delete set");
    assert_eq!(harness.state().core().working_sets().len(), 2);
    assert_eq!(harness.state().core().view(), View::WorkingSet(sets[1].id));
}

/// Press, move, release with the primary button, a few frames apart.
fn drag(harness: &mut Harness<'static, SwitchboardApp>, from: egui::Pos2, to: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(from));
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    let mid = from.lerp(to, 0.5);
    harness.event(egui::Event::PointerMoved(mid));
    harness.step();
    harness.event(egui::Event::PointerMoved(to));
    harness.run_steps(2);
    harness.event(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run_steps(3);
}

/// The claude agent and alpha's command on the working set, at 30
/// columns, the view showing in a large window.
fn working_set_of_two(
    harness: &mut Harness<'static, SwitchboardApp>,
    ids: &Seeded,
) -> (RecordId, RecordId) {
    let id = seed_claude(harness, ids);
    let shell = harness
        .state()
        .core()
        .workspace(ids.alpha)
        .unwrap()
        .sessions[0]
        .id;
    harness.state_mut().core_mut_for_seeding().dispatch(
        AppAction::NewWorkingSet {
            name: None,
            clone_of: None,
            with: Some(PinTarget::Session(id)),
            columns: 30,
        },
        switchboard::core::Clock::at(1),
    );
    let set = first_set(harness);
    harness.state_mut().core_mut_for_seeding().dispatch(
        AppAction::AddToWorkingSet {
            set,
            target: PinTarget::Session(shell),
            columns: 30,
        },
        switchboard::core::Clock::at(1),
    );
    harness.set_size(egui::vec2(1400.0, 900.0));
    showing(harness, View::WorkingSet(set));
    (id, shell)
}

fn rect_of(
    h: &Harness<'static, SwitchboardApp>,
    target: &PinTarget,
) -> switchboard::core::GridRect {
    h.state().core().working_sets()[0]
        .items
        .iter()
        .find(|i| i.target == *target)
        .unwrap()
        .rect
}

#[test]
fn arranging_moves_and_resizes_cards_in_units_and_refuses_an_overlap() {
    use switchboard::core::GridRect;
    use switchboard::ui::working_set::UNIT;
    let (mut harness, ids) = harness();
    let (id, shell) = working_set_of_two(&mut harness, &ids);
    let me = PinTarget::Session(id);
    assert_eq!(
        rect_of(&harness, &me),
        GridRect {
            x: 0,
            y: 0,
            w: 10,
            h: 8
        }
    );
    assert_eq!(
        rect_of(&harness, &PinTarget::Session(shell)),
        GridRect {
            x: 10,
            y: 0,
            w: 10,
            h: 7
        },
        "a command card is shorter: it shows output, not a conversation"
    );
    // Outside arrange mode a drag does nothing.
    let title = harness.get_by_label("claude-agent").rect();
    drag(
        &mut harness,
        title.center(),
        title.center() + egui::vec2(UNIT * 12.0, 0.0),
    );
    assert_eq!(rect_of(&harness, &me).x, 0);
    click(&mut harness, "Arrange");
    harness.get_by_label("Done");
    // Twelve units right lands on the shell's card: refused, back home.
    let title = harness.get_by_label("claude-agent").rect();
    drag(
        &mut harness,
        title.center(),
        title.center() + egui::vec2(UNIT * 12.0, 0.0),
    );
    assert_eq!(
        rect_of(&harness, &me),
        GridRect {
            x: 0,
            y: 0,
            w: 10,
            h: 8
        }
    );
    // Twenty units right is free.
    let title = harness.get_by_label("claude-agent").rect();
    drag(
        &mut harness,
        title.center(),
        title.center() + egui::vec2(UNIT * 20.0, 0.0),
    );
    assert_eq!(
        rect_of(&harness, &me),
        GridRect {
            x: 20,
            y: 0,
            w: 10,
            h: 8
        }
    );
    // The corner handle resizes: the card is 10 by 8 units and its
    // title sits 14 px in from the left and about 30 px under the top.
    let title = harness.get_by_label("claude-agent").rect();
    let open = harness
        .query_all_by_label("Open")
        .map(|n| n.rect())
        .find(|r| r.left() >= title.left() - 1.0 && r.left() < title.left() + 40.0)
        .expect("the card's Open button");
    let right = title.left() - 14.0 + 10.0 * UNIT - 8.0;
    let bottom = open.bottom() + 12.0;
    let corner = egui::pos2(right - 6.0, bottom - 6.0);
    drag(
        &mut harness,
        corner,
        corner + egui::vec2(-UNIT * 3.0, UNIT * 2.0),
    );
    assert_eq!(
        rect_of(&harness, &me),
        GridRect {
            x: 20,
            y: 0,
            w: 7,
            h: 10
        }
    );
    click(&mut harness, "Done");
    harness.get_by_label("Arrange");
}

#[test]
fn the_microphone_on_a_working_set_card_binds_listening_to_that_session() {
    let (mut harness, ids) = harness();
    let (id, _) = working_set_of_two(&mut harness, &ids);
    harness.run_steps(2);
    assert!(
        !harness
            .state()
            .ui_state
            .prompt_boxes
            .editors
            .contains_key(&id),
        "the session was never opened"
    );
    click(&mut harness, "Listen here");
    {
        let boxes = &harness.state().ui_state.prompt_boxes;
        assert_eq!(boxes.bound, Some(id), "bound without opening the session");
        assert!(boxes.editors.contains_key(&id), "its editor was made");
    }
    // No speech model in tests: the demo stands in for the microphone.
    {
        let boxes = &mut harness.state_mut().ui_state.prompt_boxes;
        let started = boxes.voice.start_demo(false);
        boxes.editors.get_mut(&id).unwrap().apply(started);
    }
    harness.run_steps(2);
    harness.get_by_label_contains("Listening · ");
    click(&mut harness, "Listening here");
    assert!(harness.state().ui_state.prompt_boxes.listening().is_none());
    assert!(harness.query_by_label_contains("Listening · ").is_none());
}

#[test]
fn hovering_terminal_on_an_agent_card_shows_the_panes_tail() {
    let (mut harness, ids) = harness();
    let (id, _) = working_set_of_two(&mut harness, &ids);
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, two_turns()));
    harness.run_steps(2);
    harness
        .ctx
        .all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
    harness.get_by_label("Terminal").hover();
    harness.run_steps(3);
    harness.get_by_label("No terminal output yet");
    harness
        .state_mut()
        .ui_state
        .snapshots
        .insert(id, "⏺ Reading Cargo.toml\n\n❯ █\n\n".into());
    harness.run_steps(2);
    harness.get_by_label("Terminal").hover();
    harness.run_steps(3);
    assert!(
        harness
            .query_all_by_value("⏺ Reading Cargo.toml\n\n❯ █")
            .next()
            .is_some(),
        "the peek shows the pane's tail without its trailing blank lines"
    );
}

#[test]
fn a_rendered_answer_on_a_working_set_card_opens_in_the_message_dialog() {
    let (mut harness, ids) = harness();
    let (id, _) = working_set_of_two(&mut harness, &ids);
    let mut conversation = two_turns();
    conversation.turns[1].final_text = "A full answer **that is** the agent's latest.".into();
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, conversation));
    harness.run_steps(2);
    // View opens it in the message dialog, where Raw shows the source.
    click(&mut harness, "View");
    harness.get_by_label("Full message");
    assert_eq!(
        harness.state().ui_state.raw_message.as_deref(),
        Some("A full answer **that is** the agent's latest.")
    );
    click(&mut harness, "Raw");
    assert_eq!(
        harness.state().ui_state.message_view,
        switchboard::ui::dialogs::MessageView::Raw
    );
    click(&mut harness, "Rendered");
    click(&mut harness, "Close");
    harness.run_steps(2);
    assert!(harness.query_by_label("Full message").is_none());
}

#[test]
fn working_set_cards_show_the_last_exchange_and_send_a_line() {
    let (mut harness, ids) = harness();
    let (id, _) = working_set_of_two(&mut harness, &ids);
    let mut conversation = two_turns();
    conversation.turns[1].user = "second question, please".into();
    conversation.turns[1].final_text = "A full answer that is the agent's latest.".into();
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, conversation));
    harness.run_steps(2);
    harness.get_by_label("You: second question, please");
    harness.get_by_label("A full answer that is the agent's latest.");
    // A long answer is cut at the body, not allowed to push the send
    // box and the actions out of the card.
    let card_top = harness.get_by_label("claude-agent").rect().top();
    let field_top = harness.get_by_label("Line to send").rect().top();
    let long = "A long answer that goes on and on. ".repeat(60);
    harness
        .state_mut()
        .ui_state
        .conversations
        .get_mut(&id)
        .unwrap()
        .1
        .turns[1]
        .final_text = long;
    harness.run_steps(2);
    let after = harness.get_by_label("Line to send").rect().top();
    assert!(
        (after - field_top).abs() < 0.5,
        "the send box moved: {field_top} -> {after}"
    );
    let card_bottom = card_top + 8.0 * switchboard::ui::working_set::UNIT;
    let open = harness
        .query_all_by_label("Open")
        .map(|n| n.rect())
        .find(|r| r.top() > card_top && r.top() < card_bottom)
        .expect("the card's Open button is inside the card");
    assert!(open.bottom() <= card_bottom, "{open:?} past {card_bottom}");
    harness
        .state_mut()
        .ui_state
        .conversations
        .get_mut(&id)
        .unwrap()
        .1
        .turns[1]
        .final_text = "A full answer **that is** the agent's latest.".into();
    harness.run_steps(2);
    // The final answer is rendered: the emphasis marks are gone.
    harness.get_by_label_contains("that is");
    assert!(
        harness.query_by_label_contains("**that is**").is_none(),
        "the answer is Markdown, not its source"
    );
    // The one-line box sends on Enter and asks for nothing else.
    harness.get_by_label("Line to send").focus();
    harness.run_steps(2);
    harness.state_mut().dispatched.clear();
    harness.get_by_label("Line to send").type_text("ls -la");
    harness.step();
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    assert!(
        harness.state().dispatched.iter().any(|a| matches!(
            a,
            AppAction::SendInput { id: sid, text } if *sid == id && text == "ls -la"
        )),
        "{:?}",
        harness.state().dispatched
    );
    // An agent still working shows its activity line, which opens
    // unformatted on click.
    let conversation = &mut harness
        .state_mut()
        .ui_state
        .conversations
        .get_mut(&id)
        .unwrap()
        .1;
    conversation.turns[1].final_text.clear();
    conversation.turns[1].activity.push(TranscriptActivity {
        kind: ActivityKind::Text,
        line: "Reading the tests".into(),
        at: Some(at(12)),
        error: false,
        detail: None,
        text: None,
    });
    harness.run_steps(2);
    harness.get_by_label("Reading the tests").click();
    harness.run_steps(2);
    harness.get_by_label("Full message");
}

#[test]
fn a_file_card_previews_the_file_rendered_or_raw() {
    let (mut harness, _) = harness();
    let (_dir, pid, _) = file_project(&mut harness);
    harness.state_mut().core_mut_for_seeding().dispatch(
        AppAction::NewWorkingSet {
            name: None,
            clone_of: None,
            with: Some(PinTarget::File(pid, "README.md".into())),
            columns: 30,
        },
        switchboard::core::Clock::at(1),
    );
    let set = first_set(&harness);
    harness.set_size(egui::vec2(1400.0, 900.0));
    showing(&mut harness, View::WorkingSet(set));
    harness.run_steps(2);
    // Rendered: the heading is text without its mark.
    harness.get_by_label("Hello");
    harness.get_by_label("from the readme");
    click(&mut harness, "Raw");
    harness.get_by_label_contains("# Hello");
    harness.get_by_label("Rendered");
    // Raw text wraps by default and can be set to scroll sideways.
    click(&mut harness, "Sideways");
    harness.get_by_label("Wrap");
    click(&mut harness, "Rendered");
    harness.get_by_label("Hello");
}

// ---- the Prompt Box editor -----------------------------------------------

/// The embedded editor's prompt field.
fn prompt_field<'h>(harness: &'h mut Harness<'static, SwitchboardApp>) -> egui_kittest::Node<'h> {
    harness.get_by_role_and_label(Role::MultilineTextInput, "Prompt")
}

#[test]
fn agent_sessions_get_the_prompt_box_and_send_goes_to_the_pane() {
    let (mut harness, ids) = harness();
    let id = seed_claude(&mut harness, &ids);
    showing(&mut harness, View::Session(id));
    assert!(harness.query_by_label("Message").is_none(), "no plain box");
    harness.get_by_label("Send →");
    harness.get_by_label("Undo");
    harness.get_by_label("Stop agent");
    assert!(
        harness.query_by_label("⚙").is_none(),
        "embedded: no settings gear"
    );
    let field = prompt_field(&mut harness);
    field.focus();
    field.type_text("fix the failing test");
    harness.run_steps(2);
    harness.get_by_label("Send →").click();
    harness.run_steps(3);
    assert!(actions(&harness).contains(&AppAction::SendInput {
        id,
        text: "fix the failing test".into(),
    }));
    let editor = &harness.state().ui_state.prompt_boxes.editors[&id];
    assert_eq!(
        editor.core().doc().committed(),
        "",
        "cleared after the send"
    );
    // Sending never touched the clipboard: nothing was copied.
    harness.get_by_label("Prompt sent");
}

#[test]
fn a_send_into_a_cold_session_fails_and_keeps_the_prompt() {
    let (mut harness, ids) = harness();
    // codex-agent is seeded without a running pane.
    showing(&mut harness, View::Session(ids.agent));
    let field = prompt_field(&mut harness);
    field.focus();
    field.type_text("hello");
    harness.run_steps(2);
    harness.get_by_label("Send →").click();
    harness.run_steps(3);
    assert!(
        !actions(&harness)
            .iter()
            .any(|a| matches!(a, AppAction::SendInput { .. }))
    );
    harness.get_by_label("Send failed: the session is not running. Prompt kept.");
    let editor = &harness.state().ui_state.prompt_boxes.editors[&ids.agent];
    assert_eq!(editor.core().doc().committed(), "hello");
}

#[test]
fn each_session_keeps_its_own_prompt() {
    let (mut harness, ids) = harness();
    let claude = seed_claude(&mut harness, &ids);
    showing(&mut harness, View::Session(claude));
    let field = prompt_field(&mut harness);
    field.focus();
    field.type_text("for claude");
    harness.run_steps(2);
    showing(&mut harness, View::Session(ids.agent));
    let field = prompt_field(&mut harness);
    field.focus();
    field.type_text("for codex");
    harness.run_steps(2);
    // A card's quick-send line leaves an empty draft behind; it is not
    // the editor's.
    harness
        .state_mut()
        .ui_state
        .input_drafts
        .insert(claude, String::new());
    showing(&mut harness, View::Session(claude));
    let boxes = &harness.state().ui_state.prompt_boxes;
    assert_eq!(
        boxes.editors[&claude].core().doc().committed(),
        "for claude"
    );
    assert_eq!(
        boxes.editors[&ids.agent].core().doc().committed(),
        "for codex"
    );
    // Removing the session drops its editor on the next frame.
    harness
        .state_mut()
        .dispatch(AppAction::RemoveSession(ids.agent));
    harness.run_steps(2);
    assert!(
        !harness
            .state()
            .ui_state
            .prompt_boxes
            .editors
            .contains_key(&ids.agent)
    );
}

#[test]
fn listening_is_bound_to_one_session_and_shown_in_the_rail() {
    let (mut harness, ids) = harness();
    let claude = seed_claude(&mut harness, &ids);
    showing(&mut harness, View::Session(claude));
    assert!(harness.query_by_label_contains("Listening ·").is_none());
    // The row is there before listening starts, so nothing shifts later.
    let idle = harness.get_by_label("Not listening").rect();
    // No speech model in tests: the demo stands in for the microphone.
    {
        let boxes = &mut harness.state_mut().ui_state.prompt_boxes;
        boxes.bound = Some(claude);
        let started = boxes.voice.start_demo(false);
        boxes.editors.get_mut(&claude).unwrap().apply(started);
    }
    harness.run_steps(2);
    let live = harness
        .get_by_label_contains("Listening · claude-agent")
        .rect();
    assert!(
        (live.top() - idle.top()).abs() < 1.0,
        "the row stays put: idle {idle:?}, live {live:?}"
    );
    assert!(
        harness
            .query_by_label("Settings")
            .is_some_and(|n| n.rect().bottom() <= live.top())
    );
    // Still listening while another view is up, and the rail still says so.
    showing(&mut harness, View::Session(ids.agent));
    harness.get_by_label_contains("Listening · claude-agent");
    harness.get_by_label("Start listening");
    // Stop from the rail ends it wherever we are.
    click(&mut harness, "Stop listening");
    assert!(harness.query_by_label_contains("Listening ·").is_none());
    assert!(
        !harness
            .state()
            .ui_state
            .prompt_boxes
            .voice
            .is_demo_running()
    );
}

#[test]
fn the_cc_button_turns_captions_off_and_the_setting_follows() {
    let (mut harness, ids) = harness();
    let id = seed_claude(&mut harness, &ids);
    showing(&mut harness, View::Session(id));
    assert!(harness.state().core().settings().voice.captions);
    click(&mut harness, "CC");
    assert!(
        !harness.state().core().settings().voice.captions,
        "written back"
    );
    harness.run_steps(2);
    assert!(
        !harness
            .state()
            .ui_state
            .prompt_boxes
            .voice
            .captions_enabled(),
        "and not overwritten by the next frame"
    );
    click(&mut harness, "CC");
    assert!(harness.state().core().settings().voice.captions);
}

#[test]
fn the_settings_menu_picks_the_screen_for_the_overlays_and_the_editor_follows() {
    let (mut harness, ids) = harness();
    let id = seed_claude(&mut harness, &ids);
    showing(&mut harness, View::Session(id));
    click(&mut harness, "Settings");
    harness.get_by_label("Overlays on");
    // Headless there is no screen list, so the choice arrives as a
    // setting; the menu shows an unplugged choice as not connected.
    assert!(
        harness
            .query_all_by_value("Same screen as Switchboard")
            .next()
            .is_some()
    );
    harness
        .state_mut()
        .dispatch(AppAction::SetVoiceSettings(VoiceSettings {
            overlay_screen: "DELL U3415W".into(),
            ..VoiceSettings::default()
        }));
    harness.run_steps(2);
    assert!(
        harness
            .query_all_by_value("DELL U3415W (not connected)")
            .next()
            .is_some()
    );
    let editor = &harness.state().ui_state.prompt_boxes.editors[&id];
    assert_eq!(editor.settings().overlay_screen, "DELL U3415W");
}

#[test]
fn the_id_menu_copies_the_session_id_or_its_transcript_path() {
    let (mut harness, ids) = harness();
    plain_message_box(&mut harness);
    let id = seed_claude(&mut harness, &ids);
    showing(&mut harness, View::Session(id));
    let handle = harness
        .state()
        .core()
        .session(id)
        .unwrap()
        .resume
        .clone()
        .unwrap();
    // The id is on the clipboard, not on the screen.
    assert!(
        harness
            .query_by_label_contains(&handle.provider_id())
            .is_none()
    );
    click(&mut harness, "ID");
    harness.get_by_label("Copy session id").click();
    harness.step();
    let copied = |h: &Harness<'static, SwitchboardApp>, text: String| {
        h.output()
            .platform_output
            .commands
            .iter()
            .any(|c| *c == egui::OutputCommand::CopyText(text.clone()))
    };
    assert!(copied(&harness, handle.provider_id()));
    harness.run_steps(2);
    click(&mut harness, "ID");
    harness.get_by_label("Copy transcript path").click();
    harness.step();
    assert!(copied(
        &harness,
        handle.transcript().unwrap().display().to_string()
    ));
}

#[test]
fn pop_out_moves_the_session_page_into_its_own_window() {
    let (mut harness, ids) = harness();
    plain_message_box(&mut harness);
    let id = seed_claude(&mut harness, &ids);
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, two_turns()));
    showing(&mut harness, View::Board(ids.beta));
    showing(&mut harness, View::Session(id));
    click(&mut harness, "Pop out");
    assert!(actions(&harness).contains(&AppAction::PopOut(id)));
    let app = harness.state();
    assert!(app.core().popped_out(id));
    assert_eq!(
        app.core().view(),
        View::Board(ids.beta),
        "the main window stepped back"
    );
    harness.run_steps(2);
    // Headless egui embeds the window in the main one: the page is
    // there, with Close window in place of Back, and the conversation.
    harness.get_by_label("Close window");
    harness.get_by_label("pong");
    assert!(harness.query_by_label("Pop out").is_none());
    // The main window's page for it only points at the window.
    showing(&mut harness, View::Session(id));
    harness.get_by_label("This session is open in its own window.");
    click(&mut harness, "Close window");
    assert!(actions(&harness).contains(&AppAction::ClosePopout(id)));
    assert!(!harness.state().core().popped_out(id));
    harness.run_steps(2);
    harness.get_by_label("Back");
    assert!(harness.query_by_label("Close window").is_none());
}

#[test]
fn the_main_window_draws_at_the_zoom_saved_for_its_display() {
    let (mut harness, _ids) = harness();
    harness.run_steps(2);
    assert!((harness.ctx.zoom_factor() - 1.0).abs() < f32::EPSILON);
    // The harness lists no displays, so its window is on "main".
    harness
        .state_mut()
        .dispatch(AppAction::SetMonitorZoom("main".into(), 150));
    harness.run_steps(3);
    assert!(
        (harness.ctx.zoom_factor() - 1.5).abs() < f32::EPSILON,
        "zoom follows the setting"
    );
    assert!(
        !harness.ctx.options(|o| o.zoom_with_keyboard),
        "egui's zoom keys are off; the app handles them per window"
    );
}

#[test]
fn the_settings_menu_moves_the_side_panel_to_the_left() {
    let (mut harness, ids) = harness();
    showing(&mut harness, View::Board(ids.alpha));
    let right_of = |h: &Harness<'static, SwitchboardApp>| {
        let files = h.get_by_label("Files").rect().left();
        let all = h.get_by_label("All sessions").rect().left();
        files > all
    };
    assert!(
        right_of(&harness),
        "the side starts on the right of the rail"
    );
    click(&mut harness, "Settings");
    click(&mut harness, "Side panel on the left");
    assert!(actions(&harness).contains(&AppAction::SetSideLeft(true)));
    harness.run_steps(2);
    let files = harness.get_by_label("Files").rect();
    let rail = harness.get_by_label("All sessions").rect();
    assert!(files.left() > rail.left(), "still right of the rail");
    let page_x = harness
        .get_by_role_and_label(Role::Button, "Config")
        .rect()
        .left();
    assert!(files.left() < page_x, "and left of the board's content");
}

#[test]
fn the_settings_menu_turns_the_prompt_box_off_and_on() {
    let (mut harness, ids) = harness();
    let id = seed_claude(&mut harness, &ids);
    showing(&mut harness, View::Session(id));
    click(&mut harness, "Settings");
    click(&mut harness, "Prompt Box editor for agent sessions");
    assert!(actions(&harness).contains(&AppAction::SetPromptBox(false)));
    harness.get_by_label("Message");
    assert!(harness.query_by_label("Send →").is_none());
}

// --- plan review

/// A converged one-round review on beta, driven by the seeded Claude
/// session, with its reviewer and planner records.
fn seed_review(
    harness: &mut Harness<'static, SwitchboardApp>,
    ids: &Seeded,
    source: RecordId,
) -> WorkflowId {
    let reviewer = record(
        ids.beta,
        "plan review",
        SessionKind::Agent(AgentKind::Codex),
        2,
    );
    let mut planner = record(
        ids.beta,
        "claude-agent planner",
        SessionKind::Agent(AgentKind::ClaudeCode),
        3,
    );
    planner.resume = Some(ResumeHandle::ClaudeCode {
        session_id: uuid::Uuid::new_v4(),
        transcript: Some(PathBuf::from("/nowhere/y.jsonl")),
    });
    let (feedback, response) = round_paths(Path::new("/nowhere/docs/plan.md"), 1);
    let run = WorkflowRun {
        id: WorkflowId::new(),
        project: ids.beta,
        definition: BUILTIN_WORKFLOW.into(),
        source,
        plan: PathBuf::from("/nowhere/docs/plan.md"),
        planner: Some(planner.id),
        reviewer: reviewer.id,
        rounds: vec![Round {
            n: 1,
            feedback,
            response,
            verdict: Some(Verdict::Nothing),
            user_feedback: None,
            responded: false,
            snapshot: false,
        }],
        state: RunState::Converged,
        cap: 4,
        cleaned: false,
        created: at(200),
        updated: at(200),
    };
    let id = run.id;
    let core = harness.state_mut().core_mut_for_seeding();
    let mut workspaces = core.workspaces().to_vec();
    let beta = workspaces
        .iter_mut()
        .find(|w| w.project.id == ids.beta)
        .unwrap();
    beta.sessions.push(reviewer);
    beta.sessions.push(planner);
    beta.workflows.push(run);
    core.seed(workspaces, vec![]);
    id
}

#[test]
fn review_plan_starts_from_the_session_header_with_a_file_the_session_wrote() {
    let (mut harness, ids) = harness();
    let id = seed_claude(&mut harness, &ids);
    let mut conversation = two_turns();
    conversation.turns[1].activity.push(TranscriptActivity {
        kind: ActivityKind::Tool,
        line: "Write docs/plan.md".into(),
        at: None,
        error: false,
        detail: Some(ToolDetail {
            name: "Write".into(),
            // A long `content` sorts before `file_path` in the printed
            // input and is cut at the cap; `path` was read before that.
            input: "{\n  \"content\": \"# Plan…\"".into(),
            result: "ok".into(),
            path: Some(PathBuf::from("/nowhere/docs/plan.md")),
        }),
        text: None,
    });
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, conversation));
    showing(&mut harness, View::Session(id));
    click(&mut harness, "Review plan");
    harness.get_by_label("Review a plan");
    // The session's cwd is /tmp/proj, so the path shows in full.
    click(&mut harness, "/nowhere/docs/plan.md");
    click(&mut harness, "Start review");
    assert!(actions(&harness).contains(&AppAction::StartWorkflow {
        source: id,
        plan: PathBuf::from("/nowhere/docs/plan.md"),
        definition: BUILTIN_WORKFLOW.into(),
    }));
    let core = harness.state().core();
    let run = core.workflows().next().expect("a run");
    assert_eq!(run.source, id);
    assert_eq!(core.view(), View::Workflow(run.id));
    assert!(core.session(run.reviewer).unwrap().name.ends_with("review"));
    harness.get_by_label("PLAN REVIEW");
    harness.get_by_label("Round 1");
}

#[test]
fn the_review_page_lists_rounds_and_its_controls_dispatch() {
    let (mut harness, ids) = harness();
    let source = seed_claude(&mut harness, &ids);
    let run = seed_review(&mut harness, &ids, source);
    showing(&mut harness, View::Workflow(run));
    harness.get_by_label("Round 1");
    harness.get_by_label("nothing further");
    harness.get_by_label("converged");
    // The user's own feedback becomes a round for the planner.
    let field = harness.get_by_label("Your feedback");
    field.focus();
    field.type_text("Split step 3");
    harness.run_steps(2);
    click(&mut harness, "Send my feedback");
    assert!(actions(&harness).contains(&AppAction::UserFeedback {
        run,
        text: "Split step 3".into(),
    }));
    assert_eq!(
        harness.state().core().workflow(run).unwrap().state,
        RunState::AwaitingResponse
    );
    harness.get_by_label("Round 2");
    click(&mut harness, "Pause");
    assert!(actions(&harness).contains(&AppAction::PauseWorkflow(run)));
    click(&mut harness, "Finalize");
    assert!(actions(&harness).contains(&AppAction::FinalizeWorkflow(run)));
    assert_eq!(
        harness.state().core().workflow(run).unwrap().state,
        RunState::Finalized
    );
    click(&mut harness, "Hand off");
    click(&mut harness, "As is");
    assert!(actions(&harness).contains(&AppAction::HandOffWorkflow {
        run,
        mode: HandoffMode::AsIs,
    }));
    assert_eq!(harness.state().core().view(), View::Session(source));
    // The board lists the review.
    showing(&mut harness, View::Board(ids.beta));
    harness.get_by_label("Review: plan.md");
}

#[test]
fn a_round_in_progress_shows_the_working_agents_terminal_and_opens_its_session() {
    let (mut harness, ids) = harness();
    let source = seed_claude(&mut harness, &ids);
    let run = seed_review(&mut harness, &ids, source);
    // Back to the first round in progress: the reviewer is at work.
    let (reviewer, planner) = {
        let core = harness.state_mut().core_mut_for_seeding();
        let mut workspaces = core.workspaces().to_vec();
        let beta = workspaces
            .iter_mut()
            .find(|w| w.project.id == ids.beta)
            .unwrap();
        let wf = beta.workflows.iter_mut().find(|w| w.id == run).unwrap();
        wf.state = RunState::AwaitingFeedback;
        wf.rounds[0].verdict = None;
        let pair = (wf.reviewer, wf.planner.unwrap());
        core.seed(workspaces, vec![]);
        pair
    };
    showing(&mut harness, View::Workflow(run));
    harness.get_by_label("reviewing ↗");
    harness.get_by_label("plan review at work");
    // Terminals are off in the harness; the pane says so in their place.
    harness.get_by_label("Embedded terminal disabled.");
    click(&mut harness, "Open session");
    assert!(actions(&harness).contains(&AppAction::ShowSession(reviewer)));
    assert_eq!(harness.state().core().view(), View::Session(reviewer));
    // The planner's turn shows the planner instead, in the Response pane.
    {
        let core = harness.state_mut().core_mut_for_seeding();
        let mut workspaces = core.workspaces().to_vec();
        let beta = workspaces
            .iter_mut()
            .find(|w| w.project.id == ids.beta)
            .unwrap();
        let wf = beta.workflows.iter_mut().find(|w| w.id == run).unwrap();
        wf.state = RunState::AwaitingResponse;
        wf.rounds[0].verdict = Some(Verdict::Changes);
        core.seed(workspaces, vec![]);
    }
    showing(&mut harness, View::Workflow(run));
    click(&mut harness, "answering ↗");
    assert!(actions(&harness).contains(&AppAction::ShowSession(planner)));
}

#[test]
fn cleaning_up_asks_first_and_lists_the_files() {
    let (mut harness, ids) = harness();
    let source = seed_claude(&mut harness, &ids);
    let run = seed_review(&mut harness, &ids, source);
    showing(&mut harness, View::Workflow(run));
    click(&mut harness, "Clean up");
    harness.get_by_label("Delete the round files");
    harness.get_by_label("/nowhere/docs/plan.feedback-1.md");
    click(&mut harness, "Delete files");
    assert!(actions(&harness).contains(&AppAction::CleanUpWorkflow(run)));
    assert!(harness.state().core().workflow(run).unwrap().cleaned);
}
