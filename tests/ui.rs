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
    FakeAgents, FakeEvents, FakeHost, FakeOpener, FakeProjectConfig, FakeSecrets, FakeTranscripts,
    MemoryStore,
};
use switchboard::app::Services;
use switchboard::core::{
    Activity, AgentKind, AppAction, Approval, CardLayout, Definition, Launch, Notice, PinTarget,
    Project, ProjectEnv, ProjectId, RecordId, ResumeHandle, SessionKind, SessionRecord, SideTab,
    ThemeMode, View, Workspace,
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
        activity_reason: None,
        last_event_at: None,
        last_exit: None,
        not_resumable: false,
        scrollback: None,
        source: None,
        approved_hash: None,
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
        wake: None,
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_eframe(move |cc| {
            switchboard::ui::theme::install(&cc.egui_ctx);
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
    // The shell is a card with Open and Kill; the command is a card whose
    // Show button opens it.
    harness.get_by_role_and_label(Role::Button, "Open");
    harness.get_all_by_label("Show").next().unwrap().click();
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
    click(&mut harness, "▶ build");
    click(&mut harness, "• deploy");
    click(&mut harness, "▶ lint");
    let dispatched = actions(&harness);
    assert!(dispatched.contains(&AppAction::RestartSession(ids.build)));
    assert!(dispatched.contains(&AppAction::RestartSession(ids.deploy)));
    // Unapproved: the bar opens the Run tab instead of running it.
    assert!(!dispatched.contains(&AppAction::RestartSession(ids.lint)));
    assert!(dispatched.contains(&AppAction::SetSideTab(SideTab::Run)));
    assert!(dispatched.contains(&AppAction::SetFilesOpen(true)));
    harness.get_by_label("cargo clippy");
    // The bar is on the board too, without opening anything.
    showing(&mut harness, View::Board(ids.alpha));
    harness.get_by_label("▶ build");
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
    harness.get_by_label("▶ build").hover();
    harness.run_steps(3);
    harness.get_by_label_contains("compiled 12 files");
    harness.get_by_label_contains("not running: click to run again");
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
    // Only `server` is running, so it alone offers Open and Kill. Open
    // goes last: it leaves the board for the session.
    click(&mut harness, "Kill");
    harness.get_all_by_label("Remove").next().unwrap().click();
    harness.run_steps(2);
    click(&mut harness, "Open");
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
fn shift_click_the_menu_and_the_pane_put_a_path_in_the_message() {
    let (mut harness, _) = harness();
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
fn messages_have_a_context_menu_with_copy() {
    let (mut harness, ids) = harness();
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
fn view_raw_shows_the_message_unformatted_in_a_dialog() {
    let (mut harness, ids) = harness();
    let id = seed_claude(&mut harness, &ids);
    harness
        .state_mut()
        .ui_state
        .conversations
        .insert(id, (None, two_turns()));
    showing(&mut harness, View::Session(id));
    assert!(harness.query_by_label("Raw message").is_none());
    harness.get_by_label("pong").click_secondary();
    harness.run_steps(2);
    harness.get_by_label("View raw").click();
    harness.run_steps(2);
    harness.get_by_label("Raw message");
    assert_eq!(
        harness.state().ui_state.raw_message.as_deref(),
        Some("pong")
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
    assert!(harness.query_by_label("Raw message").is_none());
    assert!(harness.state().ui_state.raw_message.is_none());
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
        project_config: Box::new(FakeProjectConfig::default()),
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
    let n = harness
        .query_all_by_label_contains("22")
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

#[test]
fn the_working_set_takes_a_session_from_its_header_and_a_file_from_the_tree() {
    let (mut harness, ids) = harness();
    let id = seed_claude(&mut harness, &ids);
    harness.set_size(egui::vec2(1200.0, 800.0));
    harness.run_steps(2);
    // Empty at first, reachable from the rail.
    click(&mut harness, "Working Set");
    harness.run_steps(2);
    assert_eq!(harness.state().core().view(), View::WorkingSet);
    harness.get_by_label("Nothing here yet.");
    // The session header adds the session; the card appears at the
    // working set's size, wider than a board card.
    showing(&mut harness, View::Session(id));
    click(&mut harness, "Add to working set");
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .core()
            .in_working_set(&PinTarget::Session(id))
    );
    harness.get_by_label("Remove from working set");
    showing(&mut harness, View::WorkingSet);
    harness.get_by_label("1 card");
    let card = harness.get_by_label("claude-agent").rect();
    let open = harness.get_by_label("Open").rect();
    assert!(
        open.top() > card.bottom() + 150.0,
        "the card is tall: {card:?} {open:?}"
    );
    // The board's card menu takes it off again.
    harness.get_by_label("claude-agent").click_secondary();
    harness.run_steps(2);
    click(&mut harness, "Remove from working set");
    harness.run_steps(2);
    harness.get_by_label("Nothing here yet.");
    // A file joins from the tree's menu and shows as a file card.
    let (_dir, pid, _) = file_project(&mut harness);
    harness.get_by_label("  README.md").click_secondary();
    harness.run_steps(2);
    click(&mut harness, "Add to working set");
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .core()
            .in_working_set(&PinTarget::File(pid, "README.md".into()))
    );
    showing(&mut harness, View::WorkingSet);
    harness.get_by_label("README.md");
    click(&mut harness, "Take off");
    harness.run_steps(2);
    harness.get_by_label("Nothing here yet.");
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
    for target in [PinTarget::Session(id), PinTarget::Session(shell)] {
        harness.state_mut().core_mut_for_seeding().dispatch(
            AppAction::AddToWorkingSet {
                target,
                columns: 30,
            },
            switchboard::core::Clock::at(1),
        );
    }
    harness.set_size(egui::vec2(1400.0, 900.0));
    showing(harness, View::WorkingSet);
    (id, shell)
}

fn rect_of(
    h: &Harness<'static, SwitchboardApp>,
    target: &PinTarget,
) -> switchboard::core::GridRect {
    h.state()
        .core()
        .working_set()
        .unwrap()
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
            w: 7,
            h: 5
        },
        "a command card is smaller"
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
    // Clicking the answer opens it unformatted.
    harness
        .get_by_label("A full answer that is the agent's latest.")
        .click();
    harness.run_steps(2);
    harness.get_by_label("Raw message");
}

#[test]
fn a_file_card_previews_the_file_rendered_or_raw() {
    let (mut harness, _) = harness();
    let (_dir, pid, _) = file_project(&mut harness);
    harness.state_mut().core_mut_for_seeding().dispatch(
        AppAction::AddToWorkingSet {
            target: PinTarget::File(pid, "README.md".into()),
            columns: 30,
        },
        switchboard::core::Clock::at(1),
    );
    harness.set_size(egui::vec2(1400.0, 900.0));
    showing(&mut harness, View::WorkingSet);
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
