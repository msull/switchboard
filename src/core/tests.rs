//! State-transition tests for the core. Every test dispatches actions at
//! an explicit `Clock` and asserts on state and returned effects.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use uuid::Uuid;

use super::action::{AppAction, AppCore, Clock, Effect, UNDO_WINDOW, View};
use super::controller::UiRequest;
use super::definitions::entry_hash;
use super::model::{
    Activity, AgentKind, Approval, CardState, Discarded, GridRect, Launch, PinTarget, Project,
    ProjectEnv, ProjectId, RecordId, ResumeHandle, SavedView, SessionKind, SessionRecord, Settings,
    SideTab, SpaceId, ThemeMode, Views, WindowFrame, Workspace,
};
use super::reconcile::RECORD_ID_ENV;
use crate::ports::agent::AgentLaunch;
use crate::ports::controller::{Button, ControllerEvent, Direction};
use crate::ports::events::{EventKind, SessionEvent};
use crate::ports::host::{HostId, HostStatus, Liveness};
use crate::ports::project_config::{DefinedEntry, ProjectConfig};
use crate::ports::store::{Loaded, StoreError};

// --- fixtures

fn project(name: &str) -> Project {
    let t = Clock::at(0).wall;
    Project {
        id: ProjectId::new(),
        name: name.into(),
        root: PathBuf::from("/tmp/proj"),
        tags: vec![],
        notes: String::new(),
        pinned: vec![],
        env: ProjectEnv::default(),
        shown: Vec::new(),
        created: t,
        last_active: t,
        space: SpaceId::DEFAULT,
    }
}

fn record(project: ProjectId, kind: SessionKind, order: u32) -> SessionRecord {
    let t = Clock::at(0).wall;
    SessionRecord {
        id: RecordId::new(),
        project,
        name: format!("s{order}"),
        kind,
        cwd: PathBuf::from("/tmp/proj"),
        launch: match kind {
            SessionKind::Agent(_) => Launch::Argv(vec!["agent".into()]),
            SessionKind::Shell => Launch::Shell,
            SessionKind::Command | SessionKind::Service => Launch::Command {
                command: "npm run dev".into(),
                shell: "/bin/zsh".into(),
            },
        },
        env_profile: None,
        created: t,
        last_seen: t,
        notes: String::new(),
        resume: None,
        autostart: false,
        layout: super::model::CardLayout { order, group: None },
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

fn claude_handle() -> ResumeHandle {
    ResumeHandle::ClaudeCode {
        session_id: Uuid::new_v4(),
        transcript: Some(PathBuf::from("/tmp/transcript.jsonl")),
    }
}

fn running(id: RecordId) -> HostStatus {
    HostStatus {
        id: HostId(id.host_name()),
        liveness: Liveness::Running {
            pid: 42,
            command: "x".into(),
        },
        cwd: None,
        last_activity: None,
        title: None,
    }
}

fn exited(id: RecordId, code: Option<i32>) -> HostStatus {
    HostStatus {
        liveness: Liveness::Exited { code },
        ..running(id)
    }
}

/// A core that has loaded `workspaces` and reconciled against `host`.
fn loaded(workspaces: Vec<Workspace>, host: Vec<HostStatus>) -> (AppCore, Vec<Effect>) {
    let mut core = AppCore::new();
    core.dispatch(
        AppAction::StoreLoaded(Ok(Loaded {
            workspaces,
            notices: vec![],
            ..Loaded::default()
        })),
        Clock::at(0),
    );
    let effects = core.dispatch(AppAction::HostListed(host), Clock::at(1));
    (core, effects)
}

/// One workspace with the given records; returns the core, project id, and record ids.
fn with_records(
    kinds: &[SessionKind],
    host: impl Fn(&SessionRecord) -> Option<HostStatus>,
) -> (AppCore, ProjectId, Vec<RecordId>) {
    let p = project("p");
    let pid = p.id;
    let mut w = Workspace::new(p);
    for (i, kind) in kinds.iter().enumerate() {
        w.sessions
            .push(record(pid, *kind, u32::try_from(i).unwrap()));
    }
    let ids: Vec<_> = w.sessions.iter().map(|s| s.id).collect();
    let statuses: Vec<_> = w.sessions.iter().filter_map(&host).collect();
    let (core, _) = loaded(vec![w], statuses);
    (core, pid, ids)
}

fn saves(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::Save(_)))
        .count()
}

fn spawns(effects: &[Effect]) -> Vec<&Effect> {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::Spawn { .. }))
        .collect()
}

fn event(kind: EventKind, at_ms: u64) -> SessionEvent {
    SessionEvent {
        at: Clock::at(at_ms).wall,
        seq: 0,
        record_id: None,
        provider_session_id: None,
        cwd: Some(PathBuf::from("/tmp/proj")),
        transcript_path: None,
        kind,
    }
}

fn agent() -> SessionKind {
    SessionKind::Agent(AgentKind::ClaudeCode)
}

fn codex() -> SessionKind {
    SessionKind::Agent(AgentKind::Codex)
}

/// One cold Claude Code record with a resume handle.
fn resumable_agent() -> (AppCore, RecordId) {
    let p = project("p");
    let mut w = Workspace::new(p.clone());
    let mut r = record(p.id, agent(), 0);
    r.resume = Some(claude_handle());
    let id = r.id;
    w.sessions.push(r);
    let (core, _) = loaded(vec![w], vec![]);
    (core, id)
}

/// Runs a fresh Claude Code launch through to `Spawned(Ok)`.
fn launch_agent(core: &mut AppCore, id: RecordId, handle: Option<ResumeHandle>) -> Vec<Effect> {
    core.dispatch(
        AppAction::LaunchPrepared {
            id,
            result: Ok(AgentLaunch {
                argv: vec!["claude".into()],
                env: vec![],
                resume: handle,
            }),
        },
        Clock::at(10),
    );
    core.dispatch(AppAction::Spawned { id, result: Ok(()) }, Clock::at(20))
}

// --- 1. startup reconcile

#[test]
fn set_theme_saves_settings_once_per_change() {
    let mut core = AppCore::new();
    let effects = core.dispatch(AppAction::SetTheme(ThemeMode::Dark), Clock::at(0));
    assert_eq!(
        effects,
        vec![Effect::SaveSettings(Settings {
            theme: ThemeMode::Dark,
            ..Settings::default()
        })]
    );
    assert_eq!(core.settings().theme, ThemeMode::Dark);
    assert!(
        core.dispatch(AppAction::SetTheme(ThemeMode::Dark), Clock::at(1))
            .is_empty()
    );
}

#[test]
fn restart_kills_the_pane_and_spawns_again() {
    let p = project("p");
    let mut w = Workspace::new(p.clone());
    let r = record(p.id, SessionKind::Service, 0);
    let id = r.id;
    w.sessions.push(r);
    let (mut core, _) = loaded(vec![w], vec![running(id)]);
    let effects = core.dispatch(AppAction::RestartSession(id), Clock::at(1));
    // The run opened on the record is saved first.
    assert_eq!(saves(&effects), 1);
    assert!(matches!(effects[1], Effect::Kill(ref h) if h.0 == id.host_name()));
    assert!(matches!(effects[2], Effect::Spawn { id: sid, .. } if sid == id));
    assert_eq!(effects.len(), 3);
    assert_eq!(core.session(id).unwrap().runs.len(), 1);
    // While the relaunch is in flight a second restart does nothing.
    assert!(
        core.dispatch(AppAction::RestartSession(id), Clock::at(2))
            .is_empty()
    );
}

#[test]
fn remove_of_a_cold_record_forgets_its_scrollback() {
    let p = project("p");
    let mut w = Workspace::new(p.clone());
    let r = record(p.id, SessionKind::Shell, 0);
    let id = r.id;
    w.sessions.push(r);
    let (mut core, _) = loaded(vec![w], vec![]);
    core.dispatch(AppAction::RemoveSession(id), Clock::at(1));
    assert!(core.session(id).is_none(), "off the board at once");
    // The record and its scrollback go once the undo window closes.
    let effects = core.dispatch(AppAction::Tick, Clock::at(12_000));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::Forget(h) if h.0 == id.host_name()))
    );
    assert!(!effects.iter().any(|e| matches!(e, Effect::Kill(_))));
    assert!(core.session(id).is_none());
}

#[test]
fn a_removed_session_comes_back_within_the_undo_window() {
    let (mut core, pid, ids) = with_records(&[SessionKind::Shell], |s| Some(running(s.id)));
    let id = ids[0];
    let shell = PinTarget::Session(id);
    core.dispatch(
        AppAction::NewWorkingSet {
            name: None,
            clone_of: None,
            with: Some(shell.clone()),
            columns: 24,
        },
        Clock::at(1),
    );
    let set = core.working_sets()[0].id;
    let e = core.dispatch(AppAction::RemoveSession(id), Clock::at(2));
    assert!(core.session(id).is_none());
    assert!(
        !e.iter().any(|e| matches!(e, Effect::Save(_))),
        "nothing on disk changes until the window closes: {e:?}"
    );
    assert!(core.sets_holding(&shell).is_empty(), "its card is gone");
    let notice = core.notice().expect("the removal notice");
    assert_eq!(notice.text, "Removed s0");
    assert_eq!(notice.undo, Some(id));
    assert_eq!(notice.expires_at, Some(Clock::at(2).mono + UNDO_WINDOW));
    // Undo puts the record and its cards back and drops the notice.
    let e = core.dispatch(AppAction::UndoRemove(id), Clock::at(5));
    assert!(core.session(id).is_some());
    assert_eq!(core.workspace(pid).unwrap().sessions.len(), 1);
    assert_eq!(core.sets_holding(&shell), vec![set]);
    assert!(core.notice().is_none());
    assert!(e.iter().any(|e| matches!(e, Effect::SaveViews(_))));
    // Once the window has closed there is nothing to undo.
    core.dispatch(AppAction::RemoveSession(id), Clock::at(6));
    let e = core.dispatch(AppAction::Tick, Clock::at(17_000));
    assert!(e.iter().any(|e| matches!(e, Effect::Save(_))));
    assert!(core.notice().is_none(), "the notice went with the window");
    core.dispatch(AppAction::UndoRemove(id), Clock::at(18_000));
    assert!(core.session(id).is_none());
}

#[test]
fn quiet_codex_pane_reads_idle_until_output_resumes() {
    let p = project("p");
    let mut w = Workspace::new(p.clone());
    let r = record(p.id, codex(), 0);
    let id = r.id;
    w.sessions.push(r);
    let (mut core, _) = loaded(vec![w], vec![]);
    let now = Clock::at(60_000);
    let mut status = running(id);
    status.last_activity = Some(now.wall - Duration::from_secs(5));
    core.dispatch(AppAction::HostListed(vec![status.clone()]), now);
    assert_eq!(core.card_state(id), CardState::Working);
    status.last_activity = Some(now.wall - Duration::from_secs(30));
    core.dispatch(AppAction::HostListed(vec![status.clone()]), now);
    assert_eq!(core.card_state(id), CardState::Idle);
    // Claude Code has hooks, so silence means nothing for it.
    let mut w2 = Workspace::new(project("q"));
    let r2 = record(w2.project.id, agent(), 0);
    let id2 = r2.id;
    w2.sessions.push(r2);
    let (mut core2, _) = loaded(vec![w2], vec![]);
    let mut s2 = running(id2);
    s2.last_activity = Some(now.wall - Duration::from_secs(300));
    core2.dispatch(AppAction::HostListed(vec![s2]), now);
    // Claude Code has hooks, so silence means nothing for it: still starting.
    assert_eq!(core2.card_state(id2), CardState::Starting);
}

#[test]
fn document_view_and_file_actions() {
    let p = project("p");
    let pid = p.id;
    let (mut core, _) = loaded(vec![Workspace::new(p)], vec![]);
    let doc = PathBuf::from("/work/p/docs/design.md");
    let e = core.dispatch(AppAction::ShowDocument(pid, doc.clone()), Clock::at(1));
    assert_eq!(core.view(), View::Document(pid, doc.clone()));
    assert_eq!(saves(&e), 1, "showing a document marks the project active");
    assert_eq!(
        core.dispatch(AppAction::OpenDocument(doc.clone()), Clock::at(2)),
        vec![Effect::OpenPath(doc.clone())]
    );
    assert_eq!(
        core.dispatch(AppAction::RevealDocument(doc.clone()), Clock::at(3)),
        vec![Effect::Reveal(doc.clone())]
    );
    core.dispatch(AppAction::SetEditor(" zed ".into()), Clock::at(4));
    assert_eq!(
        core.dispatch(AppAction::OpenInEditor(doc.clone()), Clock::at(5)),
        vec![Effect::OpenInEditor {
            editor: "zed".into(),
            path: doc
        }]
    );
    core.dispatch(AppAction::Back, Clock::at(6));
    assert_eq!(core.view(), View::Switchboard);
}

#[test]
fn a_space_shows_only_its_own_projects_and_sets() {
    let mut core = AppCore::new();
    let (a, b) = (Workspace::new(project("a")), Workspace::new(project("b")));
    let (ida, idb) = (a.project.id, b.project.id);
    core.seed(vec![a, b], Vec::new());
    core.seed_views(Views::default());
    assert_eq!(core.active_space(), SpaceId::DEFAULT);
    assert!(core.project_visible(ida) && core.project_visible(idb));
    // A new space is shown at once and starts empty.
    let effects = core.dispatch(AppAction::NewSpace("Client".into()), Clock::at(5));
    let client = core.spaces()[1].id;
    assert_eq!(core.active_space(), client);
    assert_eq!(core.view(), View::Switchboard);
    assert_eq!(core.visible_workspaces().count(), 0);
    assert!(!core.project_visible(ida));
    assert!(effects.iter().any(|e| matches!(e, Effect::SaveViews(_))));
    assert!(effects.iter().any(|e| matches!(e, Effect::SaveSettings(_))));
    // Blank names are refused.
    core.dispatch(AppAction::NewSpace("  ".into()), Clock::at(6));
    assert_eq!(core.spaces().len(), 2);
    // What is made while a space is active belongs to it.
    core.dispatch(
        AppAction::AddProject {
            name: "c".into(),
            root: "/tmp/c".into(),
        },
        Clock::at(7),
    );
    let idc = core.visible_workspaces().next().unwrap().project.id;
    assert_eq!(core.project_space(idc), Some(client));
    core.dispatch(
        AppAction::NewWorkingSet {
            name: None,
            clone_of: None,
            with: None,
            columns: 24,
        },
        Clock::at(8),
    );
    assert_eq!(core.visible_working_sets().count(), 1);
    assert_eq!(core.working_sets()[0].space, client);
    assert_eq!(core.working_sets()[0].name, "Working Set");
    // Back in the default space, the other projects show and the
    // client's set does not.
    core.dispatch(AppAction::ShowSpace(SpaceId::DEFAULT), Clock::at(9));
    assert_eq!(core.visible_workspaces().count(), 2);
    assert_eq!(core.visible_working_sets().count(), 0);
    assert!(core.project_visible(ida) && !core.project_visible(idc));
}

#[test]
fn showing_something_in_another_space_steps_into_it_and_leaving_goes_to_the_switchboard() {
    let mut core = AppCore::new();
    let a = Workspace::new(project("a"));
    let ida = a.project.id;
    core.seed(vec![a], Vec::new());
    core.seed_views(Views::default());
    core.dispatch(AppAction::NewSpace("Client".into()), Clock::at(1));
    let client = core.active_space();
    // The board of a project in the default space: the space follows.
    core.dispatch(AppAction::ShowBoard(ida), Clock::at(2));
    assert_eq!(core.active_space(), SpaceId::DEFAULT);
    assert_eq!(core.view(), View::Board(ida));
    // Switching away from the space of the page showing lands on the
    // switchboard, so nothing of the old space stays on screen.
    core.dispatch(AppAction::ShowSpace(client), Clock::at(3));
    assert_eq!(core.view(), View::Switchboard);
    // The switchboard is in every space: switching keeps it.
    core.dispatch(AppAction::ShowSpace(SpaceId::DEFAULT), Clock::at(4));
    assert_eq!(core.view(), View::Switchboard);
    // Renames stick; deleting refuses a space with something in it and
    // the last one.
    core.dispatch(
        AppAction::RenameSpace(client, "  Client work ".into()),
        Clock::at(5),
    );
    assert_eq!(core.space(client).unwrap().name, "Client work");
    core.dispatch(AppAction::DeleteSpace(SpaceId::DEFAULT), Clock::at(6));
    assert_eq!(core.spaces().len(), 2, "the default space has a project");
    core.dispatch(AppAction::DeleteSpace(client), Clock::at(7));
    assert_eq!(core.spaces().len(), 1);
    core.dispatch(AppAction::DeleteSpace(SpaceId::DEFAULT), Clock::at(8));
    assert_eq!(core.spaces().len(), 1, "the last space stays");
}

#[test]
fn moving_a_project_between_spaces_takes_it_off_the_old_space_sets() {
    let mut core = AppCore::new();
    let p = project("p");
    let mut w = Workspace::new(p.clone());
    let r = record(p.id, SessionKind::Shell, 0);
    let id = r.id;
    w.sessions.push(r);
    core.seed(vec![w], Vec::new());
    core.seed_views(Views::default());
    core.dispatch(
        AppAction::NewWorkingSet {
            name: None,
            clone_of: None,
            with: Some(PinTarget::Session(id)),
            columns: 24,
        },
        Clock::at(1),
    );
    let set = core.working_sets()[0].id;
    assert_eq!(core.working_sets()[0].items.len(), 1);
    core.dispatch(AppAction::NewSpace("Client".into()), Clock::at(2));
    let client = core.active_space();
    core.dispatch(AppAction::MoveProjectToSpace(p.id, client), Clock::at(3));
    assert_eq!(core.project_space(p.id), Some(client));
    assert!(
        core.working_set(set).unwrap().items.is_empty(),
        "a set holds only its own space's cards"
    );
    // A card from another space is refused.
    core.dispatch(
        AppAction::AddToWorkingSet {
            set,
            target: PinTarget::Session(id),
            columns: 24,
        },
        Clock::at(4),
    );
    assert!(core.working_set(set).unwrap().items.is_empty());
    // The set can follow the project.
    core.dispatch(AppAction::MoveSetToSpace(set, client), Clock::at(5));
    core.dispatch(
        AppAction::AddToWorkingSet {
            set,
            target: PinTarget::Session(id),
            columns: 24,
        },
        Clock::at(6),
    );
    assert_eq!(core.working_set(set).unwrap().items.len(), 1);
    // A space's waiting count is its own; the badge counts every space.
    core.dispatch(AppAction::ShowSpace(SpaceId::DEFAULT), Clock::at(7));
    assert_eq!(core.waiting_count_in(client), 0);
    // Notices about the project carry its space.
    core.dispatch(AppAction::UndoDiscard(id), Clock::at(8));
    assert!(
        core.notices().iter().any(|n| n.space == Some(client)),
        "{:?}",
        core.notices()
    );
}

#[test]
fn a_load_puts_records_of_a_lost_space_in_the_first_and_reopens_the_last_space() {
    let mut core = AppCore::new();
    let mut p = project("p");
    let lost = SpaceId::new();
    p.space = lost;
    let mut views = Views::default();
    views.spaces.clear();
    let mut set = crate::core::WorkingSet::named("s");
    set.space = lost;
    views.sets.push(set);
    let settings = Settings {
        space: lost,
        last_view: SavedView::Board(p.id),
        ..Default::default()
    };
    let pid = p.id;
    core.dispatch(
        AppAction::StoreLoaded(Ok(Loaded {
            workspaces: vec![Workspace::new(p)],
            notices: Vec::new(),
            settings,
            views,
        })),
        Clock::at(1),
    );
    assert_eq!(core.spaces().len(), 1, "the default space is put back");
    assert_eq!(core.active_space(), SpaceId::DEFAULT);
    assert_eq!(core.project_space(pid), Some(SpaceId::DEFAULT));
    assert_eq!(core.working_sets()[0].space, SpaceId::DEFAULT);
    assert_eq!(
        core.view(),
        View::Board(pid),
        "the last view is in the active space"
    );
}

#[test]
fn the_file_side_toggle_is_a_saved_setting() {
    let mut core = AppCore::new();
    core.seed(Vec::new(), Vec::new());
    assert!(!core.settings().files_open);
    let effects = core.dispatch(AppAction::SetFilesOpen(true), Clock::at(1));
    assert!(core.settings().files_open);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::SaveSettings(s) if s.files_open))
    );
    // Setting it again to the same value writes nothing.
    let effects = core.dispatch(AppAction::SetFilesOpen(true), Clock::at(2));
    assert!(!effects.iter().any(|e| matches!(e, Effect::SaveSettings(_))));
}

#[test]
fn a_popped_out_session_is_shown_in_its_window_and_the_main_window_steps_back() {
    let (mut core, _, ids) = with_records(&[SessionKind::Shell], |r| Some(running(r.id)));
    let id = ids[0];
    core.dispatch(
        AppAction::ShowBoard(core.workspaces()[0].project.id),
        Clock::at(1),
    );
    core.dispatch(AppAction::ShowSession(id), Clock::at(2));
    assert_eq!(core.view(), View::Session(id));
    let effects = core.dispatch(AppAction::PopOut(id), Clock::at(3));
    assert!(core.popped_out(id));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::SaveSettings(s) if s.popouts.len() == 1))
    );
    assert!(
        matches!(core.view(), View::Board(_)),
        "the page left the main window"
    );
    // Showing it again raises the window instead of drawing it here.
    let effects = core.dispatch(AppAction::ShowSession(id), Clock::at(4));
    assert!(matches!(core.view(), View::Board(_)));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::FocusWindow(w) if *w == id))
    );
    // A second pop out only focuses; a move is remembered.
    let effects = core.dispatch(AppAction::PopOut(id), Clock::at(5));
    assert!(effects.iter().any(|e| matches!(e, Effect::FocusWindow(_))));
    assert!(!effects.iter().any(|e| matches!(e, Effect::SaveSettings(_))));
    let frame = WindowFrame {
        x: 10,
        y: 20,
        w: 800,
        h: 600,
        monitor: "DELL".into(),
    };
    core.dispatch(AppAction::PopoutMoved(id, frame.clone()), Clock::at(6));
    assert_eq!(core.settings().popouts[0].frame, Some(frame));
    // Closing the window gives the page back to the main window.
    core.dispatch(AppAction::ClosePopout(id), Clock::at(7));
    assert!(!core.popped_out(id));
    core.dispatch(AppAction::ShowSession(id), Clock::at(8));
    assert_eq!(core.view(), View::Session(id));
    // A removed session takes its window with it, once it is gone for
    // good.
    core.dispatch(AppAction::PopOut(id), Clock::at(9));
    core.dispatch(AppAction::RemoveSession(id), Clock::at(10));
    core.dispatch(AppAction::Tick, Clock::at(21_000));
    assert!(core.settings().popouts.is_empty());
}

#[test]
fn the_main_window_frame_is_a_saved_setting() {
    let mut core = AppCore::new();
    core.seed(Vec::new(), Vec::new());
    assert!(core.settings().main_window.is_none());
    let main = WindowFrame {
        x: -3440,
        y: -797,
        w: 3440,
        h: 1400,
        monitor: "DELL".into(),
    };
    let effects = core.dispatch(AppAction::MainWindowMoved(main.clone()), Clock::at(1));
    assert_eq!(core.settings().main_window, Some(main));
    assert!(effects.iter().any(|e| matches!(e, Effect::SaveSettings(_))));
}

#[test]
fn zoom_is_a_saved_setting_per_display_and_native_is_the_default() {
    let mut core = AppCore::new();
    core.seed(Vec::new(), Vec::new());
    assert_eq!(core.monitor_zoom("DELL"), 100);
    core.dispatch(AppAction::SetMonitorZoom("DELL".into(), 120), Clock::at(1));
    assert_eq!(core.monitor_zoom("DELL"), 120);
    assert_eq!(core.monitor_zoom("Built-in"), 100, "each display its own");
    core.dispatch(AppAction::SetMonitorZoom("DELL".into(), 130), Clock::at(2));
    assert_eq!(core.settings().monitor_zoom.len(), 1, "replaced, not added");
    assert_eq!(core.notices().len(), 1, "one zoom notice, the latest");
    assert_eq!(core.notice().unwrap().text, "Zoom 130% on DELL");
    // Native is the default, so it is not kept as an entry.
    core.dispatch(AppAction::SetMonitorZoom("DELL".into(), 100), Clock::at(3));
    assert!(core.settings().monitor_zoom.is_empty());
}

#[test]
fn the_side_position_is_a_saved_setting() {
    let mut core = AppCore::new();
    core.seed(Vec::new(), Vec::new());
    assert!(!core.settings().side_left);
    let effects = core.dispatch(AppAction::SetSideLeft(true), Clock::at(1));
    assert!(core.settings().side_left);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::SaveSettings(s) if s.side_left))
    );
}

#[test]
fn store_loaded_installs_workspaces_and_notices() {
    let mut core = AppCore::new();
    let w = Workspace::new(project("a"));
    let effects = core.dispatch(
        AppAction::StoreLoaded(Ok(Loaded {
            workspaces: vec![w.clone()],
            notices: vec![StoreError::Corrupt {
                path: "/x.json".into(),
                recovered: true,
                detail: "eof".into(),
            }],
            ..Loaded::default()
        })),
        Clock::at(0),
    );
    // Loading launches nothing; it only asks for each definition file.
    assert_eq!(
        effects,
        vec![Effect::ReadProjectConfig {
            project: w.project.id,
            root: w.project.root.clone(),
        }],
        "{effects:?}"
    );
    assert_eq!(core.workspaces(), &[w]);
    let n = core.notice().unwrap();
    assert!(n.is_error && n.expires_at.is_none());
    assert!(n.text.contains("recovered from backup"));
    assert!(!core.read_only());
}

#[test]
fn locked_store_means_read_only_and_no_saves() {
    let mut core = AppCore::new();
    core.dispatch(
        AppAction::StoreLoaded(Err(StoreError::Locked)),
        Clock::at(0),
    );
    assert!(core.read_only());
    assert!(core.notice().unwrap().is_error);
    let effects = core.dispatch(
        AppAction::AddProject {
            name: "p".into(),
            root: "/p".into(),
        },
        Clock::at(1),
    );
    assert_eq!(saves(&effects), 0);
    assert_eq!(core.workspaces().len(), 1, "state still updates in memory");
}

#[test]
fn other_load_error_is_a_notice() {
    let mut core = AppCore::new();
    core.dispatch(
        AppAction::StoreLoaded(Err(StoreError::Io("disk".into()))),
        Clock::at(0),
    );
    assert!(!core.read_only());
    assert!(core.notice().unwrap().text.contains("disk"));
}

#[test]
fn reconcile_marks_warm_exited_and_cold() {
    let (core, _, ids) = with_records(&[agent(), SessionKind::Shell, agent()], |s| {
        match s.layout.order {
            0 => Some(running(s.id)),
            1 => Some(exited(s.id, Some(3))),
            _ => None,
        }
    });
    assert_eq!(core.card_state(ids[0]), CardState::Starting);
    assert_eq!(core.card_state(ids[1]), CardState::Exited(Some(3)));
    assert_eq!(core.card_state(ids[2]), CardState::NotRunning);
}

#[test]
fn reconcile_never_spawns_agents_or_non_autostart() {
    let p = project("p");
    let pid = p.id;
    let mut w = Workspace::new(p);
    let mut a = record(pid, agent(), 0);
    a.resume = Some(claude_handle());
    a.autostart = true; // hostile/odd data: still never launched
    w.sessions.push(a);
    w.sessions.push(record(pid, SessionKind::Service, 1)); // autostart false
    let mut cmd = record(pid, SessionKind::Command, 2);
    cmd.autostart = true;
    w.sessions.push(cmd);
    let (_, effects) = loaded(vec![w], vec![]);
    assert!(spawns(&effects).is_empty(), "{effects:?}");
}

#[test]
fn reconcile_spawns_cold_autostart_services_only() {
    let p = project("p");
    let pid = p.id;
    let mut w = Workspace::new(p);
    let mut cold = record(pid, SessionKind::Service, 0);
    cold.autostart = true;
    let mut warm = record(pid, SessionKind::Service, 1);
    warm.autostart = true;
    let (cold_id, warm_id) = (cold.id, warm.id);
    w.sessions.push(cold);
    w.sessions.push(warm);
    let (core, effects) = loaded(vec![w], vec![running(warm_id)]);
    let s = spawns(&effects);
    assert_eq!(s.len(), 1);
    let Effect::Spawn { id, spec } = s[0] else {
        unreachable!()
    };
    assert_eq!(*id, cold_id);
    assert_eq!(spec.id, HostId(cold_id.host_name()));
    assert_eq!(spec.cwd, PathBuf::from("/tmp/proj"));
    assert_eq!(
        spec.command.as_deref(),
        Some(&["/bin/zsh".to_string(), "-lc".into(), "npm run dev".into()][..])
    );
    assert!(
        spec.env
            .contains(&(RECORD_ID_ENV.into(), cold_id.0.to_string()))
    );
    assert!(spec.scrollback.is_none());
    assert!(core.is_in_flight(cold_id));
}

#[test]
fn only_the_first_host_poll_reconciles() {
    let p = project("p");
    let pid = p.id;
    let mut w = Workspace::new(p);
    let mut svc = record(pid, SessionKind::Service, 0);
    svc.autostart = true;
    let id = svc.id;
    w.sessions.push(svc);
    let (mut core, first) = loaded(vec![w], vec![]);
    assert_eq!(spawns(&first).len(), 1);
    core.dispatch(AppAction::Spawned { id, result: Ok(()) }, Clock::at(2));
    let again = core.dispatch(AppAction::HostListed(vec![]), Clock::at(3));
    assert!(spawns(&again).is_empty());
}

#[test]
fn host_poll_records_exit_code_once() {
    let (mut core, _, ids) = with_records(&[SessionKind::Shell], |s| Some(running(s.id)));
    let e1 = core.dispatch(
        AppAction::HostListed(vec![exited(ids[0], Some(1))]),
        Clock::at(5),
    );
    assert_eq!(saves(&e1), 1);
    assert_eq!(core.session(ids[0]).unwrap().last_exit, Some(1));
    let e2 = core.dispatch(
        AppAction::HostListed(vec![exited(ids[0], Some(1))]),
        Clock::at(6),
    );
    assert_eq!(saves(&e2), 0, "unchanged status does not save");
}

#[test]
fn host_unavailable_blocks_launches_with_a_notice() {
    let (mut core, pid, ids) = with_records(&[SessionKind::Shell], |_| None);
    core.dispatch(
        AppAction::HostUnavailable(Some("tmux not found".into())),
        Clock::at(1),
    );
    assert_eq!(core.host_error(), Some("tmux not found"));
    let e1 = core.dispatch(AppAction::ReturnToSession(ids[0]), Clock::at(2));
    assert!(e1.is_empty());
    let e2 = core.dispatch(
        AppAction::NewSession {
            project: pid,
            name: "n".into(),
            kind: SessionKind::Shell,
            cwd: "/tmp".into(),
            launch: Launch::Shell,
            outputs: Vec::new(),
        },
        Clock::at(3),
    );
    assert!(e2.is_empty());
    assert_eq!(core.notices().len(), 2);
    assert!(core.notices().iter().all(|n| n.is_error));
    core.dispatch(AppAction::HostUnavailable(None), Clock::at(4));
    assert!(core.host_error().is_none());
}

// --- 2. card state

#[test]
fn card_state_follows_activity_while_running() {
    let (mut core, _, ids) = with_records(&[agent()], |s| Some(running(s.id)));
    let cases = [
        (
            EventKind::PermissionRequested { tool: None },
            CardState::WaitingOnYou,
        ),
        (EventKind::PromptSubmitted, CardState::Working),
        (EventKind::Stopped { last_message: None }, CardState::Idle),
    ];
    for (i, (kind, expected)) in cases.into_iter().enumerate() {
        core.dispatch(
            AppAction::Events(vec![SessionEvent {
                record_id: Some(ids[0]),
                // strictly increasing timestamps so none is ignored as stale
                ..event(kind, 100 + 1_000 * u64::try_from(i).unwrap())
            }]),
            Clock::at(1),
        );
        assert_eq!(core.card_state(ids[0]), expected);
    }
}

#[test]
fn unknown_activity_is_starting_for_claude_working_for_codex_idle_for_others() {
    let (mut core, _, ids) = with_records(
        &[
            agent(),
            codex(),
            SessionKind::Shell,
            SessionKind::Service,
            SessionKind::Command,
        ],
        |s| Some(running(s.id)),
    );
    assert_eq!(core.card_state(ids[0]), CardState::Starting);
    assert_eq!(core.state_text(ids[0]), "starting");
    assert_eq!(core.card_state(ids[1]), CardState::Working);
    for id in &ids[2..] {
        assert_eq!(core.card_state(*id), CardState::Idle);
    }
    // The first hook ends the starting state.
    core.dispatch(
        AppAction::Events(vec![SessionEvent {
            record_id: Some(ids[0]),
            ..event(EventKind::SessionStart, 100)
        }]),
        Clock::at(1),
    );
    assert_eq!(core.card_state(ids[0]), CardState::Working);
}

#[test]
fn waiting_reason_follows_the_event() {
    let (mut core, _, ids) = with_records(&[agent()], |s| Some(running(s.id)));
    let cases: [(EventKind, &str); 5] = [
        (
            EventKind::PermissionRequested {
                tool: Some("Bash".into()),
            },
            "waiting on you: permission for Bash",
        ),
        (
            EventKind::PermissionRequested {
                tool: Some("AskUserQuestion".into()),
            },
            "waiting on you: question",
        ),
        (
            EventKind::StopFailed {
                reason: Some("rate_limit".into()),
            },
            "waiting on you: rate limit",
        ),
        (
            EventKind::StopFailed { reason: None },
            "waiting on you: failed",
        ),
        (EventKind::PromptSubmitted, "working"),
    ];
    for (i, (kind, expected)) in cases.into_iter().enumerate() {
        core.dispatch(
            AppAction::Events(vec![SessionEvent {
                record_id: Some(ids[0]),
                ..event(kind.clone(), 1_000 * (u64::try_from(i).unwrap() + 1))
            }]),
            Clock::at(1),
        );
        assert_eq!(core.state_text(ids[0]), expected, "{kind:?}");
    }
    // A working session carries no stale reason.
    assert_eq!(core.session(ids[0]).unwrap().activity_reason, None);
}

#[test]
fn host_liveness_overrides_activity() {
    let (mut core, _, ids) = with_records(&[agent()], |s| Some(running(s.id)));
    core.dispatch(
        AppAction::Events(vec![SessionEvent {
            record_id: Some(ids[0]),
            ..event(EventKind::PermissionRequested { tool: None }, 100)
        }]),
        Clock::at(1),
    );
    assert_eq!(core.card_state(ids[0]), CardState::WaitingOnYou);
    core.dispatch(
        AppAction::HostListed(vec![exited(ids[0], None)]),
        Clock::at(2),
    );
    assert_eq!(core.card_state(ids[0]), CardState::Exited(None));
    core.dispatch(AppAction::HostListed(vec![]), Clock::at(3));
    assert_eq!(core.card_state(ids[0]), CardState::NotRunning);
}

#[test]
fn missing_host_with_not_resumable_flag() {
    let p = project("p");
    let mut w = Workspace::new(p.clone());
    let mut r = record(p.id, agent(), 0);
    r.not_resumable = true;
    let id = r.id;
    w.sessions.push(r);
    let (core, _) = loaded(vec![w], vec![]);
    assert_eq!(core.card_state(id), CardState::NotResumable);
    assert_eq!(core.card_state(RecordId::new()), CardState::NotRunning);
}

// --- 3. projects

#[test]
fn add_project_creates_workspace_shows_board_and_saves() {
    let mut core = AppCore::new();
    let now = Clock::at(500);
    let effects = core.dispatch(
        AppAction::AddProject {
            name: "New".into(),
            root: "/r".into(),
        },
        now,
    );
    let w = &core.workspaces()[0];
    assert_eq!(w.project.name, "New");
    assert_eq!(w.project.created, now.wall);
    assert_eq!(w.project.last_active, now.wall);
    assert_eq!(core.view(), View::Board(w.project.id));
    assert_eq!(
        effects,
        vec![
            Effect::Save(w.clone()),
            Effect::ReadProjectConfig {
                project: w.project.id,
                root: "/r".into(),
            },
            Effect::SaveSettings(Settings {
                last_view: SavedView::Board(w.project.id),
                ..Settings::default()
            }),
        ]
    );
}

#[test]
fn remove_rename_pin_unpin_project() {
    let mut core = AppCore::new();
    core.dispatch(
        AppAction::AddProject {
            name: "a".into(),
            root: "/a".into(),
        },
        Clock::at(0),
    );
    let id = core.workspaces()[0].project.id;
    let e = core.dispatch(AppAction::RenameProject(id, "b".into()), Clock::at(1));
    assert_eq!(saves(&e), 1);
    assert_eq!(core.workspace(id).unwrap().project.name, "b");

    let e = core.dispatch(AppAction::PinDocument(id, "docs/x.md".into()), Clock::at(2));
    assert_eq!(saves(&e), 1);
    core.dispatch(AppAction::PinDocument(id, "docs/x.md".into()), Clock::at(3));
    assert_eq!(
        core.workspace(id).unwrap().project.pinned.len(),
        1,
        "no duplicate pins"
    );
    let e = core.dispatch(
        AppAction::UnpinDocument(id, "docs/x.md".into()),
        Clock::at(4),
    );
    assert_eq!(saves(&e), 1);
    assert!(core.workspace(id).unwrap().project.pinned.is_empty());

    let e = core.dispatch(AppAction::RemoveProject(id), Clock::at(5));
    assert_eq!(
        e,
        vec![
            Effect::Delete(id),
            Effect::SaveSettings(Settings::default())
        ]
    );
    assert!(core.workspaces().is_empty());
    assert_eq!(
        core.view(),
        View::Switchboard,
        "board of a removed project is gone"
    );
}

#[test]
fn showing_a_board_bumps_last_active_and_saves() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    let now = Clock::at(9_000);
    let e = core.dispatch(AppAction::ShowBoard(pid), now);
    assert_eq!(saves(&e), 1);
    assert_eq!(core.workspace(pid).unwrap().project.last_active, now.wall);
}

// --- 4. sessions

fn new_session(
    core: &mut AppCore,
    pid: ProjectId,
    kind: SessionKind,
    launch: Launch,
) -> (RecordId, Vec<Effect>) {
    let effects = core.dispatch(
        AppAction::NewSession {
            project: pid,
            name: "n".into(),
            kind,
            cwd: "/tmp/proj".into(),
            launch,
            outputs: Vec::new(),
        },
        Clock::at(7_000),
    );
    let id = core.workspace(pid).unwrap().sessions.last().unwrap().id;
    (id, effects)
}

#[test]
fn new_agent_session_saves_then_prepares_launch() {
    let (mut core, pid, ids) = with_records(&[SessionKind::Shell], |_| None);
    let (id, effects) = new_session(&mut core, pid, agent(), Launch::Argv(vec![]));
    assert_ne!(id, ids[0]);
    let r = core.session(id).unwrap();
    assert_eq!(r.created, Clock::at(7_000).wall);
    assert_eq!(r.layout.order, 1, "next order in the project");
    assert!(matches!(effects[0], Effect::Save(_)));
    assert_eq!(
        effects[1],
        Effect::PrepareLaunch {
            id,
            kind: AgentKind::ClaudeCode,
            name: "n".into(),
            cwd: "/tmp/proj".into()
        }
    );
    assert!(core.is_in_flight(id));
}

#[test]
fn new_shell_command_service_spawn_directly() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    let (id, e) = new_session(&mut core, pid, SessionKind::Shell, Launch::Shell);
    let Effect::Spawn { spec, .. } = &e[1] else {
        panic!("{e:?}")
    };
    assert_eq!(spec.command, None);
    assert!(
        spec.env
            .iter()
            .any(|(k, v)| k == RECORD_ID_ENV && *v == id.0.to_string())
    );

    let (_, e) = new_session(
        &mut core,
        pid,
        SessionKind::Command,
        Launch::Argv(vec!["ls".into(), "-l".into()]),
    );
    let Effect::Spawn { spec, .. } = &e[1] else {
        panic!("{e:?}")
    };
    assert_eq!(spec.command, Some(vec!["ls".into(), "-l".into()]));

    let (_, e) = new_session(
        &mut core,
        pid,
        SessionKind::Service,
        Launch::Command {
            command: "make serve".into(),
            shell: "/bin/bash".into(),
        },
    );
    let Effect::Spawn { spec, .. } = &e[1] else {
        panic!("{e:?}")
    };
    assert_eq!(
        spec.command,
        Some(vec!["/bin/bash".into(), "-lc".into(), "make serve".into()])
    );
    assert_eq!(core.workspace(pid).unwrap().sessions.len(), 3);
}

#[test]
fn launch_prepared_stores_resume_and_spawns_with_record_id() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    let (id, _) = new_session(&mut core, pid, agent(), Launch::Argv(vec![]));
    let handle = claude_handle();
    let effects = core.dispatch(
        AppAction::LaunchPrepared {
            id,
            result: Ok(AgentLaunch {
                argv: vec!["claude".into(), "--session-id".into(), "x".into()],
                env: vec![("A".into(), "1".into())],
                resume: Some(handle.clone()),
            }),
        },
        Clock::at(10),
    );
    assert_eq!(core.session(id).unwrap().resume, Some(handle));
    assert_eq!(saves(&effects), 1);
    let Effect::Spawn { spec, .. } = &effects[1] else {
        panic!("{effects:?}")
    };
    assert_eq!(spec.command.as_ref().unwrap()[0], "claude");
    assert!(spec.env.contains(&("A".into(), "1".into())));
    assert!(spec.env.contains(&(RECORD_ID_ENV.into(), id.0.to_string())));
    assert!(core.is_in_flight(id), "still in flight until spawned");
}

#[test]
fn launch_prepared_error_clears_flight_with_notice() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    let (id, _) = new_session(&mut core, pid, agent(), Launch::Argv(vec![]));
    let e = core.dispatch(
        AppAction::LaunchPrepared {
            id,
            result: Err("no claude".into()),
        },
        Clock::at(10),
    );
    assert!(e.is_empty());
    assert!(!core.is_in_flight(id));
    assert!(core.notice().unwrap().text.contains("no claude"));
}

#[test]
fn spawned_agent_stays_in_its_pane_and_updates_last_seen() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    let (id, _) = new_session(&mut core, pid, agent(), Launch::Argv(vec![]));
    let effects = launch_agent(&mut core, id, Some(claude_handle()));
    assert_eq!(saves(&effects), 1);
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Attach { .. })),
        "the window is opened from Open, not by the launch: {effects:?}"
    );
    assert!(!effects.iter().any(|e| matches!(e, Effect::Discover { .. })));
    assert_eq!(core.session(id).unwrap().last_seen, Clock::at(20).wall);
    assert!(!core.is_in_flight(id));
    // Open on the running session still brings the window up.
    let e = core.dispatch(AppAction::ReturnToSession(id), Clock::at(21));
    assert!(matches!(&e[0], Effect::Attach { id: i, .. } if *i == id));
}

#[test]
fn spawned_agent_attaches_when_the_setting_asks_for_it() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    let e = core.dispatch(AppAction::SetOpenTerminalOnLaunch(true), Clock::at(0));
    assert!(matches!(&e[0], Effect::SaveSettings(s) if s.open_terminal_on_launch));
    let (id, _) = new_session(&mut core, pid, agent(), Launch::Argv(vec![]));
    let effects = launch_agent(&mut core, id, Some(claude_handle()));
    assert!(effects.contains(&Effect::Attach {
        id,
        host: HostId(id.host_name()),
        title: id.host_name(),
        cwd: "/tmp/proj".into(),
    }));
}

#[test]
fn spawned_shell_just_clears_flight() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    let (id, _) = new_session(&mut core, pid, SessionKind::Shell, Launch::Shell);
    let e = core.dispatch(AppAction::Spawned { id, result: Ok(()) }, Clock::at(1));
    assert!(!e.iter().any(|e| matches!(e, Effect::Attach { .. })));
    assert!(!core.is_in_flight(id));
}

#[test]
fn spawned_error_is_a_notice() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    let (id, _) = new_session(&mut core, pid, SessionKind::Shell, Launch::Shell);
    core.dispatch(
        AppAction::Spawned {
            id,
            result: Err("boom".into()),
        },
        Clock::at(1),
    );
    assert!(!core.is_in_flight(id));
    assert!(core.notice().unwrap().is_error);
    assert!(
        !core.session(id).unwrap().not_resumable,
        "a fresh launch failing is not a resume failure"
    );
}

#[test]
fn codex_spawn_discovers_id_then_binds_it() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    let (id, _) = new_session(&mut core, pid, codex(), Launch::Argv(vec![]));
    let effects = launch_agent(&mut core, id, None);
    assert!(!effects.iter().any(|e| matches!(e, Effect::Attach { .. })));
    assert!(effects.contains(&Effect::Discover {
        id,
        kind: AgentKind::Codex,
        cwd: "/tmp/proj".into(),
        since: Clock::at(7_000).wall,
    }));
    assert!(core.is_in_flight(id), "in flight until discovered");
    let handle = ResumeHandle::Codex {
        rollout_id: "r1".into(),
        transcript: None,
    };
    let e = core.dispatch(
        AppAction::Discovered {
            id,
            result: Ok(Some(handle.clone())),
        },
        Clock::at(30),
    );
    assert_eq!(saves(&e), 1);
    assert_eq!(core.session(id).unwrap().resume, Some(handle));
    assert!(!core.is_in_flight(id));
}

#[test]
fn codex_discovery_failure_notices_and_unblocks() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    let (id, _) = new_session(&mut core, pid, codex(), Launch::Argv(vec![]));
    launch_agent(&mut core, id, None);
    core.dispatch(
        AppAction::Discovered {
            id,
            result: Ok(None),
        },
        Clock::at(30),
    );
    assert!(
        core.notice()
            .unwrap()
            .text
            .contains("Codex session id unknown")
    );
    assert!(!core.is_in_flight(id));
    assert!(core.session(id).unwrap().resume.is_none());
}

#[test]
fn codex_discovery_refuses_an_id_bound_to_another_record() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    let (first, _) = new_session(&mut core, pid, codex(), Launch::Argv(vec![]));
    let (second, _) = new_session(&mut core, pid, codex(), Launch::Argv(vec![]));
    let handle = ResumeHandle::Codex {
        rollout_id: "r1".into(),
        transcript: None,
    };
    launch_agent(&mut core, first, None);
    core.dispatch(
        AppAction::Discovered {
            id: first,
            result: Ok(Some(handle.clone())),
        },
        Clock::at(30),
    );
    launch_agent(&mut core, second, None);
    // The first session kept writing its rollout, so a naive scan finds
    // it again for the second launch.
    let e = core.dispatch(
        AppAction::Discovered {
            id: second,
            result: Ok(Some(handle.clone())),
        },
        Clock::at(40),
    );
    assert_eq!(saves(&e), 0);
    assert_eq!(core.session(first).unwrap().resume, Some(handle));
    assert!(core.session(second).unwrap().resume.is_none());
    assert!(!core.is_in_flight(second));
    assert!(core.notice().unwrap().is_error);
    assert!(core.notice().unwrap().text.contains("another card"));
}

#[test]
fn fresh_launch_of_a_not_resumable_codex_record_drops_the_dead_handle() {
    let p = project("p");
    let mut w = Workspace::new(p.clone());
    let mut r = record(p.id, codex(), 0);
    r.resume = Some(ResumeHandle::Codex {
        rollout_id: "dead".into(),
        transcript: None,
    });
    r.not_resumable = true;
    let id = r.id;
    w.sessions.push(r);
    let (mut core, _) = loaded(vec![w], vec![]);

    let e = core.dispatch(AppAction::ReturnToSession(id), Clock::at(5));
    assert!(e.iter().any(|e| matches!(e, Effect::PrepareLaunch { .. })));
    assert_eq!(saves(&e), 1);
    let record = core.session(id).unwrap();
    assert!(!record.not_resumable);
    assert!(record.resume.is_none(), "the old rollout id is gone");

    // Without a handle the new conversation is discovered and bound.
    let e = launch_agent(&mut core, id, None);
    assert!(
        e.iter().any(|e| matches!(e, Effect::Discover { .. })),
        "{e:?}"
    );
}

#[test]
fn removing_a_project_forgets_its_flights_views_and_codex_turn() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    let (first, _) = new_session(&mut core, pid, codex(), Launch::Argv(vec![]));
    launch_agent(&mut core, first, None);
    assert!(core.is_in_flight(first), "discovery pending");
    core.dispatch(AppAction::ShowSession(first), Clock::at(30));
    assert_eq!(core.view(), View::Session(first));

    let other = project("other");
    let opid = other.id;
    core.dispatch(
        AppAction::StoreLoaded(Ok(Loaded {
            workspaces: vec![core.workspace(pid).unwrap().clone(), Workspace::new(other)],
            ..Loaded::default()
        })),
        Clock::at(31),
    );
    let (queued, _) = new_session(&mut core, opid, codex(), Launch::Argv(vec![]));
    assert!(core.queued_codex(queued), "blocked behind the discovery");

    let e = core.dispatch(AppAction::RemoveProject(pid), Clock::at(40));
    assert!(core.workspace(pid).is_none());
    assert!(e.contains(&Effect::Delete(pid)));
    assert!(!core.is_in_flight(first));
    assert_ne!(core.view(), View::Session(first));
    assert!(
        e.iter().any(
            |e| matches!(e, Effect::PrepareLaunch { id, kind: AgentKind::Codex, .. } if *id == queued)
        ),
        "the queued Codex launch proceeds: {e:?}"
    );
    assert!(!core.queued_codex(queued));

    // A late result for the removed record is ignored.
    let e = core.dispatch(
        AppAction::Discovered {
            id: first,
            result: Ok(Some(ResumeHandle::Codex {
                rollout_id: "r".into(),
                transcript: None,
            })),
        },
        Clock::at(50),
    );
    assert!(e.is_empty(), "{e:?}");
}

#[test]
fn a_failed_effect_is_an_error_notice() {
    let mut core = AppCore::new();
    let e = core.dispatch(
        AppAction::Failed("kill s-1 failed: gone".into()),
        Clock::at(1),
    );
    assert!(e.is_empty());
    let notice = core.notice().unwrap();
    assert!(notice.is_error);
    assert_eq!(notice.text, "kill s-1 failed: gone");
}

#[test]
fn codex_launches_serialize() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    let (first, e1) = new_session(&mut core, pid, codex(), Launch::Argv(vec![]));
    assert!(e1.iter().any(|e| matches!(e, Effect::PrepareLaunch { .. })));
    let (second, e2) = new_session(&mut core, pid, codex(), Launch::Argv(vec![]));
    assert_eq!(saves(&e2), 1);
    assert!(
        !e2.iter().any(|e| matches!(e, Effect::PrepareLaunch { .. })),
        "second Codex launch waits: {e2:?}"
    );
    assert!(core.queued_codex(second));
    assert!(core.is_in_flight(second), "queued counts as in flight");
    assert!(
        core.dispatch(AppAction::ReturnToSession(second), Clock::at(8))
            .is_empty()
    );

    launch_agent(&mut core, first, None);
    assert!(core.queued_codex(second), "still waiting on discovery");
    let e = core.dispatch(
        AppAction::Discovered {
            id: first,
            result: Ok(Some(ResumeHandle::Codex {
                rollout_id: "r1".into(),
                transcript: None,
            })),
        },
        Clock::at(40_000),
    );
    assert!(!core.queued_codex(second));
    assert!(e.iter().any(
        |e| matches!(e, Effect::PrepareLaunch { id, kind: AgentKind::Codex, .. } if *id == second)
    ));
    let e = launch_agent(&mut core, second, None);
    let Some(Effect::Discover { since, .. }) =
        e.iter().find(|e| matches!(e, Effect::Discover { .. }))
    else {
        panic!("{e:?}")
    };
    assert_eq!(
        *since,
        Clock::at(40_000).wall,
        "discovery window starts at the real launch"
    );
}

#[test]
fn codex_queue_advances_after_a_failed_launch() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    let (first, _) = new_session(&mut core, pid, codex(), Launch::Argv(vec![]));
    let (second, _) = new_session(&mut core, pid, codex(), Launch::Argv(vec![]));
    let e = core.dispatch(
        AppAction::LaunchPrepared {
            id: first,
            result: Err("nope".into()),
        },
        Clock::at(5),
    );
    assert!(
        e.iter()
            .any(|e| matches!(e, Effect::PrepareLaunch { id, .. } if *id == second))
    );
    assert!(!core.queued_codex(second));
}

#[test]
fn claude_launch_does_not_block_codex() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    new_session(&mut core, pid, agent(), Launch::Argv(vec![]));
    let (id, e) = new_session(&mut core, pid, codex(), Launch::Argv(vec![]));
    assert!(e.iter().any(|e| matches!(e, Effect::PrepareLaunch { .. })));
    assert!(!core.queued_codex(id));
}

#[test]
fn attached_error_is_a_notice() {
    let (mut core, _, ids) = with_records(&[agent()], |s| Some(running(s.id)));
    core.dispatch(
        AppAction::Attached {
            id: ids[0],
            result: Err("no ghostty".into()),
        },
        Clock::at(1),
    );
    assert!(core.notice().unwrap().text.contains("no ghostty"));
}

#[test]
fn session_edits_save() {
    let (mut core, _, ids) = with_records(&[SessionKind::Shell], |_| None);
    let id = ids[0];
    assert_eq!(
        saves(&core.dispatch(AppAction::RenameSession(id, "x".into()), Clock::at(1))),
        1
    );
    assert_eq!(
        saves(&core.dispatch(AppAction::SetSessionNotes(id, "n".into()), Clock::at(2))),
        1
    );
    assert_eq!(
        saves(&core.dispatch(AppAction::SetAutostart(id, true), Clock::at(3))),
        1
    );
    let e = core.dispatch(
        AppAction::MoveCard {
            id,
            order: 7,
            group: Some("g".into()),
        },
        Clock::at(4),
    );
    assert_eq!(saves(&e), 1);
    let s = core.session(id).unwrap();
    assert_eq!(
        (s.name.as_str(), s.notes.as_str(), s.autostart),
        ("x", "n", true)
    );
    assert_eq!((s.layout.order, s.layout.group.as_deref()), (7, Some("g")));
}

#[test]
fn cloning_a_session_asks_for_a_transcript_copy_then_makes_a_cold_record() {
    let (mut core, id) = resumable_agent();
    let handle = core.session(id).unwrap().resume.clone().unwrap();
    let e = core.dispatch(
        AppAction::CloneSession {
            id,
            before: 3,
            prompt: "third prompt".into(),
        },
        Clock::at(1),
    );
    assert_eq!(
        e,
        vec![Effect::CloneTranscript {
            source: id,
            handle,
            before: 3,
            prompt: "third prompt".into(),
        }],
        "nothing is made until the copy exists"
    );
    let forked = ResumeHandle::ClaudeCode {
        session_id: Uuid::new_v4(),
        transcript: Some(PathBuf::from("/tmp/forked.jsonl")),
    };
    let e = core.dispatch(
        AppAction::TranscriptCloned {
            source: id,
            prompt: "third prompt".into(),
            result: Ok(forked.clone()),
        },
        Clock::at(2),
    );
    assert_eq!(saves(&e), 1);
    assert!(
        !e.iter().any(|e| matches!(
            e,
            Effect::PrepareResume { .. } | Effect::PrepareLaunch { .. } | Effect::Spawn { .. }
        )),
        "a clone is never launched on its own"
    );
    let source = core.session(id).unwrap().clone();
    let clone = core
        .workspace(source.project)
        .unwrap()
        .sessions
        .iter()
        .find(|s| s.id != id)
        .unwrap()
        .clone();
    assert_eq!(clone.name, "s0 clone");
    assert_eq!(clone.kind, source.kind);
    assert_eq!(clone.cwd, source.cwd);
    assert_eq!(clone.resume, Some(forked));
    assert_eq!(clone.layout.order, source.layout.order + 1);
    assert_eq!(clone.created, Clock::at(2).wall);
    assert!(!clone.not_resumable && !core.is_in_flight(clone.id));
    assert_eq!(core.view(), View::Session(clone.id));
    assert_eq!(
        core.take_primed(),
        vec![(clone.id, "third prompt".to_owned())]
    );
    assert!(core.take_primed().is_empty(), "handed over once");
}

#[test]
fn discarding_cuts_the_session_in_place_and_undo_puts_it_back() {
    let (mut core, id) = resumable_agent();
    let original = core.session(id).unwrap().resume.clone().unwrap();
    let e = core.dispatch(
        AppAction::DiscardTo {
            id,
            before: 2,
            prompt: "second prompt".into(),
        },
        Clock::at(1),
    );
    assert_eq!(
        e,
        vec![Effect::DiscardTranscript {
            id,
            handle: original.clone(),
            before: 2,
            prompt: "second prompt".into(),
        }]
    );
    let cut = ResumeHandle::ClaudeCode {
        session_id: Uuid::new_v4(),
        transcript: Some(PathBuf::from("/tmp/cut.jsonl")),
    };
    let e = core.dispatch(
        AppAction::TranscriptDiscarded {
            id,
            before: 2,
            prompt: "second prompt".into(),
            result: Ok(cut.clone()),
        },
        Clock::at(2),
    );
    assert_eq!(saves(&e), 1);
    assert!(
        !e.iter().any(|e| matches!(e, Effect::Kill(_))),
        "nothing ran, nothing to stop"
    );
    let s = core.session(id).unwrap();
    assert_eq!(s.resume, Some(cut.clone()));
    assert_eq!(
        s.discard,
        Some(Discarded {
            previous: original.clone(),
            before: 2,
            prompt: "second prompt".into(),
        })
    );
    assert_eq!(core.workspaces()[0].sessions.len(), 1, "no new record");
    assert_eq!(core.take_primed(), vec![(id, "second prompt".to_owned())]);

    let e = core.dispatch(AppAction::UndoDiscard(id), Clock::at(3));
    assert_eq!(saves(&e), 1);
    let s = core.session(id).unwrap();
    assert_eq!(s.resume, Some(original));
    assert_eq!(s.discard, None);
    assert!(
        core.dispatch(AppAction::UndoDiscard(id), Clock::at(4))
            .is_empty()
    );
    assert!(
        core.notices()
            .last()
            .unwrap()
            .text
            .contains("nothing to undo")
    );
}

#[test]
fn a_discard_stops_a_running_agent_and_a_sent_message_ends_the_undo() {
    let project = project("p");
    let mut workspace = Workspace::new(project.clone());
    let mut record = record(project.id, agent(), 0);
    let original = claude_handle();
    record.resume = Some(original.clone());
    let id = record.id;
    workspace.sessions.push(record);
    let (mut core, _) = loaded(vec![workspace], vec![running(id)]);
    let cut = ResumeHandle::ClaudeCode {
        session_id: Uuid::new_v4(),
        transcript: Some(PathBuf::from("/tmp/cut.jsonl")),
    };
    let e = core.dispatch(
        AppAction::TranscriptDiscarded {
            id,
            before: 1,
            prompt: String::new(),
            result: Ok(cut.clone()),
        },
        Clock::at(2),
    );
    assert!(
        e.iter()
            .any(|e| matches!(e, Effect::Kill(h) if h.0 == id.host_name())),
        "the running agent is on the old conversation: {e:?}"
    );
    assert!(core.session(id).unwrap().discard.is_some());
    // A message into the pane is the point of no return.
    let e = core.dispatch(
        AppAction::SendInput {
            id,
            text: "go".into(),
        },
        Clock::at(3),
    );
    assert_eq!(saves(&e), 1);
    assert!(e.iter().any(|e| matches!(e, Effect::SendInput { .. })));
    assert_eq!(core.session(id).unwrap().discard, None);
    assert_eq!(core.session(id).unwrap().resume, Some(cut));
}

#[test]
fn cloning_refuses_codex_and_transcript_less_sessions_and_reports_a_failed_copy() {
    let (mut core, pid, ids) = with_records(&[codex(), agent()], |_| None);
    let clone = |core: &mut AppCore, id| {
        core.dispatch(
            AppAction::CloneSession {
                id,
                before: 1,
                prompt: String::new(),
            },
            Clock::at(1),
        )
    };
    assert!(clone(&mut core, ids[0]).is_empty());
    assert!(core.notice().unwrap().text.contains("only Claude Code"));
    assert!(clone(&mut core, ids[1]).is_empty());
    assert!(
        core.notices()
            .last()
            .unwrap()
            .text
            .contains("no transcript")
    );

    let (mut core2, id) = resumable_agent();
    let e = core2.dispatch(
        AppAction::TranscriptCloned {
            source: id,
            prompt: String::new(),
            result: Err("disk full".into()),
        },
        Clock::at(2),
    );
    assert!(e.is_empty());
    assert!(core2.notice().unwrap().text.contains("disk full"));
    assert_eq!(core2.workspaces()[0].sessions.len(), 1);
    assert_eq!(core.workspace(pid).unwrap().sessions.len(), 2);
}

// --- 5. return (idempotent)

#[test]
fn return_to_running_attaches_and_return_to_exited_relaunches() {
    let (mut core, _, ids) = with_records(&[agent(), SessionKind::Shell], |s| {
        Some(if s.layout.order == 0 {
            running(s.id)
        } else {
            exited(s.id, Some(0))
        })
    });
    // Running agent: attach only.
    let e = core.dispatch(AppAction::ReturnToSession(ids[0]), Clock::at(1));
    assert_eq!(e.len(), 1);
    assert!(matches!(&e[0], Effect::Attach { id: i, .. } if *i == ids[0]));
    assert!(!core.is_in_flight(ids[0]));
    // Exited shell: the dead pane is killed, then the shell is spawned again.
    let e = core.dispatch(AppAction::ReturnToSession(ids[1]), Clock::at(2));
    assert!(matches!(&e[0], Effect::Kill(h) if h.0 == ids[1].host_name()));
    assert_eq!(spawns(&e).len(), 1);
    assert!(!e.iter().any(|x| matches!(x, Effect::Attach { .. })));
}

#[test]
fn repeated_return_while_in_flight_spawns_once() {
    let (mut core, _, ids) = with_records(&[SessionKind::Shell], |_| None);
    let id = ids[0];
    let e1 = core.dispatch(AppAction::ReturnToSession(id), Clock::at(1));
    assert_eq!(spawns(&e1).len(), 1);
    let e2 = core.dispatch(AppAction::ReturnToSession(id), Clock::at(2));
    let e3 = core.dispatch(AppAction::ReturnToSession(id), Clock::at(3));
    assert!(e2.is_empty() && e3.is_empty(), "{e2:?} {e3:?}");
    core.dispatch(AppAction::Spawned { id, result: Ok(()) }, Clock::at(4));
    // Host now reports it; a further return attaches instead of spawning.
    core.dispatch(AppAction::HostListed(vec![running(id)]), Clock::at(5));
    let e4 = core.dispatch(AppAction::ReturnToSession(id), Clock::at(6));
    assert!(matches!(e4[0], Effect::Attach { .. }));
}

#[test]
fn repeated_return_during_agent_resume_is_ignored() {
    let p = project("p");
    let mut w = Workspace::new(p.clone());
    let mut r = record(p.id, agent(), 0);
    r.resume = Some(claude_handle());
    let id = r.id;
    w.sessions.push(r);
    let (mut core, _) = loaded(vec![w], vec![]);
    let e1 = core.dispatch(AppAction::ReturnToSession(id), Clock::at(1));
    assert!(matches!(e1[0], Effect::CheckTranscript { .. }));
    assert!(
        core.dispatch(AppAction::ReturnToSession(id), Clock::at(2))
            .is_empty()
    );
    let e2 = core.dispatch(
        AppAction::TranscriptChecked { id, exists: true },
        Clock::at(3),
    );
    assert!(matches!(e2[0], Effect::PrepareResume { .. }));
    assert!(
        core.dispatch(AppAction::ReturnToSession(id), Clock::at(4))
            .is_empty()
    );
    let e3 = core.dispatch(
        AppAction::LaunchPrepared {
            id,
            result: Ok(AgentLaunch {
                argv: vec!["claude".into(), "--resume".into()],
                env: vec![],
                resume: None,
            }),
        },
        Clock::at(5),
    );
    assert_eq!(spawns(&e3).len(), 1);
    assert!(
        core.dispatch(AppAction::ReturnToSession(id), Clock::at(6))
            .is_empty()
    );
    let e4 = core.dispatch(AppAction::Spawned { id, result: Ok(()) }, Clock::at(7));
    assert!(!e4.iter().any(|e| matches!(e, Effect::Attach { .. })));
    assert!(!core.is_in_flight(id));
    assert!(
        core.session(id).unwrap().resume.is_some(),
        "resume handle kept"
    );
}

#[test]
fn missing_transcript_marks_not_resumable() {
    let (mut core, id) = resumable_agent();
    core.dispatch(AppAction::ReturnToSession(id), Clock::at(1));
    let effects = core.dispatch(
        AppAction::TranscriptChecked { id, exists: false },
        Clock::at(2),
    );
    assert_eq!(saves(&effects), 1);
    assert_eq!(effects.len(), 1, "no launch: {effects:?}");
    assert!(core.session(id).unwrap().not_resumable);
    assert_eq!(core.card_state(id), CardState::NotResumable);
    assert!(!core.is_in_flight(id));
    let notice = core.notice().unwrap();
    assert!(notice.text.contains("not resumable"));
    assert!(!notice.is_error && notice.expires_at.is_some());

    // Returning again offers a fresh session instead of failing.
    let effects = core.dispatch(AppAction::ReturnToSession(id), Clock::at(3));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::PrepareLaunch { .. })),
        "{effects:?}"
    );
    assert!(!core.session(id).unwrap().not_resumable);
}

#[test]
fn failed_resume_spawn_marks_not_resumable() {
    let p = project("p");
    let mut w = Workspace::new(p.clone());
    let mut r = record(p.id, agent(), 0);
    r.resume = Some(claude_handle());
    let id = r.id;
    w.sessions.push(r);
    let (mut core, _) = loaded(vec![w], vec![]);
    core.dispatch(AppAction::ReturnToSession(id), Clock::at(1));
    core.dispatch(
        AppAction::TranscriptChecked { id, exists: true },
        Clock::at(2),
    );
    core.dispatch(
        AppAction::LaunchPrepared {
            id,
            result: Ok(AgentLaunch {
                argv: vec![],
                env: vec![],
                resume: None,
            }),
        },
        Clock::at(3),
    );
    let e = core.dispatch(
        AppAction::Spawned {
            id,
            result: Err("exit 1".into()),
        },
        Clock::at(4),
    );
    assert_eq!(saves(&e), 1);
    assert!(core.session(id).unwrap().not_resumable);
    assert!(core.notice().unwrap().is_error);
}

#[test]
fn return_to_agent_without_resume_starts_fresh() {
    let (mut core, _, ids) = with_records(&[agent()], |_| None);
    let e = core.dispatch(AppAction::ReturnToSession(ids[0]), Clock::at(1));
    assert!(matches!(e[0], Effect::PrepareLaunch { .. }), "{e:?}");
    assert!(core.is_in_flight(ids[0]));
}

#[test]
fn return_to_missing_service_reruns_it() {
    let (mut core, _, ids) = with_records(&[SessionKind::Service], |_| None);
    let e = core.dispatch(AppAction::ReturnToSession(ids[0]), Clock::at(1));
    let Some(Effect::Spawn { spec, .. }) = e.iter().find(|e| matches!(e, Effect::Spawn { .. }))
    else {
        panic!("{e:?}")
    };
    assert_eq!(spec.command.as_ref().unwrap()[2], "npm run dev");
}

#[test]
fn stray_results_for_unknown_flights_are_ignored() {
    let (mut core, _, ids) = with_records(&[agent()], |_| None);
    let id = ids[0];
    assert!(
        core.dispatch(
            AppAction::TranscriptChecked { id, exists: true },
            Clock::at(1)
        )
        .is_empty()
    );
    assert!(
        core.dispatch(AppAction::Spawned { id, result: Ok(()) }, Clock::at(2))
            .is_empty()
    );
    assert!(
        core.dispatch(
            AppAction::LaunchPrepared {
                id,
                result: Ok(AgentLaunch {
                    argv: vec![],
                    env: vec![],
                    resume: None
                })
            },
            Clock::at(3)
        )
        .is_empty()
    );
}

// --- 6. kill / remove

#[test]
fn kill_emits_kill_only_when_host_has_it() {
    let (mut core, _, ids) = with_records(&[SessionKind::Shell, SessionKind::Shell], |s| {
        (s.layout.order == 0).then(|| running(s.id))
    });
    let e = core.dispatch(AppAction::KillSession(ids[0]), Clock::at(1));
    assert_eq!(e, vec![Effect::Kill(HostId(ids[0].host_name()))]);
    assert!(
        core.dispatch(AppAction::KillSession(ids[1]), Clock::at(2))
            .is_empty()
    );
}

#[test]
fn remove_session_drops_record_and_saves_without_killing() {
    let (mut core, pid, ids) = with_records(&[SessionKind::Shell], |s| Some(running(s.id)));
    core.dispatch(AppAction::ShowSession(ids[0]), Clock::at(1));
    let e = core.dispatch(AppAction::RemoveSession(ids[0]), Clock::at(2));
    assert!(matches!(e[0], Effect::SaveSettings(_)), "the view moved");
    assert!(core.session(ids[0]).is_none());
    assert!(core.workspace(pid).unwrap().sessions.is_empty());
    assert_ne!(core.view(), View::Session(ids[0]));
    let e = core.dispatch(AppAction::Tick, Clock::at(13_000));
    assert!(matches!(e[0], Effect::Save(_)), "{e:?}");
    assert!(!e.iter().any(|e| matches!(e, Effect::Kill(_))));
}

#[test]
fn navigation_is_remembered_in_settings() {
    let (mut core, pid, ids) = with_records(&[SessionKind::Shell], |_| None);
    core.dispatch(AppAction::ShowBoard(pid), Clock::at(0));
    let e = core.dispatch(AppAction::ShowSession(ids[0]), Clock::at(1));
    assert!(
        e.iter().any(|e| matches!(
            e,
            Effect::SaveSettings(s) if s.last_view == SavedView::Session(ids[0])
        )),
        "{e:?}"
    );
    let e = core.dispatch(AppAction::Back, Clock::at(2));
    assert_eq!(
        e,
        vec![Effect::SaveSettings(Settings {
            last_view: SavedView::Board(pid),
            ..Settings::default()
        })]
    );
    assert!(
        core.dispatch(AppAction::Tick, Clock::at(3)).is_empty(),
        "nothing to save when the view did not move"
    );
    // A document remembers its board: previews are not restored.
    core.dispatch(
        AppAction::ShowDocument(pid, "README.md".into()),
        Clock::at(4),
    );
    assert_eq!(core.settings().last_view, SavedView::Board(pid));
}

#[test]
fn the_last_view_is_restored_at_startup_if_it_still_exists() {
    let p = project("p");
    let mut w = Workspace::new(p.clone());
    let r = record(p.id, SessionKind::Shell, 0);
    let id = r.id;
    w.sessions.push(r);
    let load = |last_view: SavedView, workspaces: Vec<Workspace>| {
        let mut core = AppCore::new();
        let effects = core.dispatch(
            AppAction::StoreLoaded(Ok(Loaded {
                workspaces,
                settings: Settings {
                    last_view,
                    ..Settings::default()
                },
                ..Loaded::default()
            })),
            Clock::at(0),
        );
        (core, effects)
    };
    let (core, effects) = load(SavedView::Session(id), vec![w.clone()]);
    assert_eq!(core.view(), View::Session(id));
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::SaveSettings(_))),
        "restoring changes nothing on disk: {effects:?}"
    );
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Spawn { .. })),
        "restoring a view never launches"
    );
    let (core, _) = load(SavedView::Board(p.id), vec![w.clone()]);
    assert_eq!(core.view(), View::Board(p.id));

    // A record that is gone falls back to the switchboard, and the
    // stale reference is written over.
    let (core, effects) = load(SavedView::Session(RecordId::new()), vec![w]);
    assert_eq!(core.view(), View::Switchboard);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::SaveSettings(s) if s.last_view == SavedView::Switchboard)),
        "{effects:?}"
    );
}

// --- 7. events

#[test]
fn events_match_by_record_id_not_cwd() {
    // Two agents in the same cwd; only the addressed one changes.
    let (mut core, _, ids) = with_records(&[agent(), agent()], |s| Some(running(s.id)));
    let e = core.dispatch(
        AppAction::Events(vec![SessionEvent {
            record_id: Some(ids[1]),
            ..event(EventKind::PermissionRequested { tool: None }, 100)
        }]),
        Clock::at(1),
    );
    assert_eq!(saves(&e), 1);
    assert_eq!(core.card_state(ids[0]), CardState::Starting);
    assert_eq!(core.card_state(ids[1]), CardState::WaitingOnYou);
    assert_eq!(core.waiting_count(), 1);
    assert_eq!(
        core.session(ids[1]).unwrap().last_event_at,
        Some(Clock::at(100).wall)
    );
    assert_eq!(core.session(ids[1]).unwrap().last_seen, Clock::at(100).wall);
}

#[test]
fn events_match_by_provider_session_id() {
    let p = project("p");
    let mut w = Workspace::new(p.clone());
    let mut a = record(p.id, agent(), 0);
    let mut b = record(p.id, agent(), 1);
    let (ha, hb) = (claude_handle(), claude_handle());
    a.resume = Some(ha);
    b.resume = Some(hb.clone());
    let (ida, idb) = (a.id, b.id);
    w.sessions.push(a);
    w.sessions.push(b);
    let (mut core, _) = loaded(vec![w], vec![running(ida), running(idb)]);
    core.dispatch(
        AppAction::Events(vec![SessionEvent {
            provider_session_id: Some(hb.provider_id()),
            ..event(EventKind::Stopped { last_message: None }, 100)
        }]),
        Clock::at(1),
    );
    assert_eq!(core.card_state(ida), CardState::Starting);
    assert_eq!(core.card_state(idb), CardState::Idle);
}

#[test]
fn unmatched_events_are_ignored() {
    let (mut core, _, ids) = with_records(&[agent()], |s| Some(running(s.id)));
    let e = core.dispatch(
        AppAction::Events(vec![
            event(EventKind::Stopped { last_message: None }, 100),
            SessionEvent {
                provider_session_id: Some("nobody".into()),
                ..event(EventKind::Stopped { last_message: None }, 101)
            },
            SessionEvent {
                record_id: Some(RecordId::new()),
                ..event(EventKind::Stopped { last_message: None }, 102)
            },
        ]),
        Clock::at(1),
    );
    assert!(e.is_empty());
    assert_eq!(core.card_state(ids[0]), CardState::Starting);
}

#[test]
fn older_or_equal_events_are_ignored() {
    let (mut core, _, ids) = with_records(&[agent()], |s| Some(running(s.id)));
    let at = |kind, ms| SessionEvent {
        record_id: Some(ids[0]),
        ..event(kind, ms)
    };
    core.dispatch(
        AppAction::Events(vec![at(EventKind::PromptSubmitted, 5_000)]),
        Clock::at(1),
    );
    // A spooled Stop from before the live prompt must not win.
    let e = core.dispatch(
        AppAction::Events(vec![
            at(EventKind::Stopped { last_message: None }, 4_000),
            at(EventKind::Stopped { last_message: None }, 5_000),
        ]),
        Clock::at(2),
    );
    assert!(e.is_empty());
    assert_eq!(core.card_state(ids[0]), CardState::Working);
    core.dispatch(
        AppAction::Events(vec![at(EventKind::Stopped { last_message: None }, 6_000)]),
        Clock::at(3),
    );
    assert_eq!(core.card_state(ids[0]), CardState::Idle);
}

#[test]
fn event_kinds_map_to_activities() {
    let (mut core, _, ids) = with_records(&[agent()], |s| Some(running(s.id)));
    let cases = [
        (EventKind::SessionStart, CardState::Working),
        (
            EventKind::PermissionRequested {
                tool: Some("Bash".into()),
            },
            CardState::WaitingOnYou,
        ),
        (EventKind::PermissionDenied, CardState::Working),
        (
            EventKind::Notification {
                kind: "permission_prompt".into(),
            },
            CardState::WaitingOnYou,
        ),
        (
            EventKind::Notification {
                kind: "idle_prompt".into(),
            },
            CardState::Idle,
        ),
        (EventKind::ToolFinished, CardState::Working),
        (
            EventKind::Notification {
                kind: "other".into(),
            },
            CardState::Working,
        ),
        (
            EventKind::Notification {
                kind: "agent_needs_input".into(),
            },
            CardState::WaitingOnYou,
        ),
        (
            EventKind::Notification {
                kind: "elicitation_dialog".into(),
            },
            CardState::WaitingOnYou,
        ),
        (
            EventKind::Notification {
                kind: "quota_auto_resume_stale".into(),
            },
            CardState::WaitingOnYou,
        ),
        (
            EventKind::Notification {
                kind: "quota_auto_resume_fired".into(),
            },
            CardState::Working,
        ),
        (
            EventKind::Notification {
                kind: "agent_completed".into(),
            },
            CardState::Idle,
        ),
        (
            EventKind::StopFailed {
                reason: Some("server_error".into()),
            },
            CardState::WaitingOnYou,
        ),
        (
            EventKind::Stopped {
                last_message: Some("done".into()),
            },
            CardState::Idle,
        ),
        (EventKind::SessionEnded { reason: None }, CardState::Idle),
    ];
    for (i, (kind, expected)) in cases.into_iter().enumerate() {
        core.dispatch(
            AppAction::Events(vec![SessionEvent {
                record_id: Some(ids[0]),
                ..event(kind.clone(), 1_000 * (u64::try_from(i).unwrap() + 1))
            }]),
            Clock::at(1),
        );
        assert_eq!(core.card_state(ids[0]), expected, "{kind:?}");
    }
    assert_eq!(core.session(ids[0]).unwrap().activity, Activity::Ended);
}

#[test]
fn session_start_fills_in_transcript_path() {
    let p = project("p");
    let mut w = Workspace::new(p.clone());
    let mut r = record(p.id, agent(), 0);
    let sid = Uuid::new_v4();
    r.resume = Some(ResumeHandle::ClaudeCode {
        session_id: sid,
        transcript: None,
    });
    let id = r.id;
    w.sessions.push(r);
    let (mut core, _) = loaded(vec![w], vec![]);
    core.dispatch(
        AppAction::Events(vec![SessionEvent {
            record_id: Some(id),
            transcript_path: Some("/t/1.jsonl".into()),
            ..event(EventKind::SessionStart, 100)
        }]),
        Clock::at(1),
    );
    assert_eq!(
        core.session(id)
            .unwrap()
            .resume
            .as_ref()
            .unwrap()
            .transcript(),
        Some(&PathBuf::from("/t/1.jsonl"))
    );
    // A later start with a different path does not overwrite what we have.
    core.dispatch(
        AppAction::Events(vec![SessionEvent {
            record_id: Some(id),
            transcript_path: Some("/t/2.jsonl".into()),
            ..event(EventKind::SessionStart, 200)
        }]),
        Clock::at(2),
    );
    assert_eq!(
        core.session(id)
            .unwrap()
            .resume
            .as_ref()
            .unwrap()
            .transcript(),
        Some(&PathBuf::from("/t/1.jsonl"))
    );
}

#[test]
fn a_new_session_id_from_the_pane_rebinds_the_record_after_clear() {
    let p = project("p");
    let mut w = Workspace::new(p.clone());
    let mut r = record(p.id, agent(), 0);
    let old = Uuid::new_v4();
    r.resume = Some(ResumeHandle::ClaudeCode {
        session_id: old,
        transcript: Some("/t/old.jsonl".into()),
    });
    r.discard = Some(Discarded {
        previous: claude_handle(),
        before: 2,
        prompt: "x".into(),
    });
    let id = r.id;
    w.sessions.push(r);
    let (mut core, _) = loaded(vec![w], vec![]);
    // The same id again changes nothing, and a different id without the
    // record id (another process in the cwd) is not even matched.
    let new = Uuid::new_v4();
    core.dispatch(
        AppAction::Events(vec![
            SessionEvent {
                record_id: Some(id),
                provider_session_id: Some(old.to_string()),
                ..event(EventKind::PromptSubmitted, 1000)
            },
            SessionEvent {
                provider_session_id: Some(new.to_string()),
                transcript_path: Some("/t/new.jsonl".into()),
                ..event(EventKind::SessionStart, 1500)
            },
        ]),
        Clock::at(1),
    );
    assert_eq!(
        core.session(id)
            .unwrap()
            .resume
            .as_ref()
            .unwrap()
            .provider_id(),
        old.to_string()
    );
    let effects = core.dispatch(
        AppAction::Events(vec![SessionEvent {
            record_id: Some(id),
            provider_session_id: Some(new.to_string()),
            transcript_path: Some("/t/new.jsonl".into()),
            ..event(EventKind::SessionStart, 2000)
        }]),
        Clock::at(2),
    );
    let record = core.session(id).unwrap();
    assert_eq!(
        record.resume,
        Some(ResumeHandle::ClaudeCode {
            session_id: new,
            transcript: Some("/t/new.jsonl".into()),
        })
    );
    assert!(record.discard.is_none(), "the cut conversation is gone too");
    assert_eq!(saves(&effects), 1);
    assert!(
        core.notices()
            .iter()
            .any(|n| n.text.contains("new conversation"))
    );
}

#[test]
fn events_coalesce_saves_per_workspace() {
    let pa = project("a");
    let pb = project("b");
    let mut wa = Workspace::new(pa.clone());
    let mut wb = Workspace::new(pb.clone());
    wa.sessions.push(record(pa.id, agent(), 0));
    wa.sessions.push(record(pa.id, agent(), 1));
    wb.sessions.push(record(pb.id, agent(), 0));
    let ids: Vec<_> = wa
        .sessions
        .iter()
        .chain(&wb.sessions)
        .map(|s| s.id)
        .collect();
    let (mut core, _) = loaded(vec![wa, wb], vec![]);
    let events = ids
        .iter()
        .map(|id| SessionEvent {
            record_id: Some(*id),
            ..event(EventKind::PromptSubmitted, 100)
        })
        .collect();
    let e = core.dispatch(AppAction::Events(events), Clock::at(1));
    assert_eq!(e.len(), 2);
    assert_eq!(saves(&e), 2);
}

// --- 8. navigation and notices

#[test]
fn view_stack_pushes_without_duplicates_and_pops_to_switchboard() {
    let (mut core, pid, ids) = with_records(&[SessionKind::Shell], |_| None);
    assert_eq!(core.view(), View::Switchboard);
    core.dispatch(AppAction::ShowBoard(pid), Clock::at(1));
    core.dispatch(AppAction::ShowBoard(pid), Clock::at(2));
    core.dispatch(AppAction::ShowSession(ids[0]), Clock::at(3));
    core.dispatch(AppAction::ShowSession(ids[0]), Clock::at(4));
    assert_eq!(core.view(), View::Session(ids[0]));
    core.dispatch(AppAction::Back, Clock::at(5));
    assert_eq!(core.view(), View::Board(pid));
    core.dispatch(AppAction::Back, Clock::at(6));
    assert_eq!(core.view(), View::Switchboard);
    core.dispatch(AppAction::Back, Clock::at(7));
    assert_eq!(
        core.view(),
        View::Switchboard,
        "never below the switchboard"
    );
    core.dispatch(AppAction::ShowSwitchboard, Clock::at(8));
    core.dispatch(AppAction::Back, Clock::at(9));
    assert_eq!(core.view(), View::Switchboard);
}

#[test]
fn notices_dismiss_and_expire() {
    let (mut core, _, ids) = with_records(&[agent()], |_| None);
    let id = ids[0];
    core.dispatch(
        AppAction::Attached {
            id,
            result: Err("e".into()),
        },
        Clock::at(1),
    );
    assert_eq!(core.notices().len(), 1);
    core.dispatch(AppAction::DismissNotice, Clock::at(2));
    assert!(core.notice().is_none());
    core.dispatch(AppAction::DismissNotice, Clock::at(3));

    // A success notice expires after 4 s; an error stays.
    core.session_mut(id).unwrap().resume = Some(claude_handle());
    core.dispatch(AppAction::ReturnToSession(id), Clock::at(1_000));
    core.dispatch(
        AppAction::TranscriptChecked { id, exists: false },
        Clock::at(1_000),
    );
    core.dispatch(
        AppAction::Attached {
            id,
            result: Err("e".into()),
        },
        Clock::at(1_001),
    );
    assert_eq!(core.notices().len(), 2);
    core.dispatch(AppAction::Tick, Clock::at(4_999));
    assert_eq!(core.notices().len(), 2);
    core.dispatch(AppAction::Tick, Clock::at(5_000));
    assert_eq!(core.notices().len(), 1);
    assert!(core.notice().unwrap().is_error);
}

#[test]
fn save_failure_is_a_notice() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    core.dispatch(
        AppAction::SaveFinished(pid, Err(StoreError::Io("full".into()))),
        Clock::at(1),
    );
    assert!(core.notice().unwrap().text.contains("full"));
    core.dispatch(AppAction::SaveFinished(pid, Ok(())), Clock::at(2));
    assert_eq!(core.notices().len(), 1);
}

// --- 9. read model

#[test]
fn sessions_sorted_by_state_then_order() {
    let (mut core, pid, ids) = with_records(
        &[SessionKind::Shell, agent(), SessionKind::Shell, agent()],
        |s| (s.layout.order != 2).then(|| running(s.id)),
    );
    core.dispatch(
        AppAction::Events(vec![SessionEvent {
            record_id: Some(ids[3]),
            ..event(EventKind::PermissionRequested { tool: None }, 100)
        }]),
        Clock::at(1),
    );
    let sorted: Vec<_> = core.sessions_sorted(pid).iter().map(|s| s.id).collect();
    // waiting (3), working (1), idle (0), not running (2)
    assert_eq!(sorted, vec![ids[3], ids[1], ids[0], ids[2]]);
    assert!(core.sessions_sorted(ProjectId::new()).is_empty());
}

#[test]
fn all_sessions_sorted_waiting_first_then_project_recency() {
    let mut old = project("old");
    old.last_active = Clock::at(0).wall;
    let mut new = project("new");
    new.last_active = Clock::at(10_000).wall;
    let mut wo = Workspace::new(old.clone());
    let mut wn = Workspace::new(new.clone());
    wo.sessions.push(record(old.id, agent(), 0));
    wo.sessions.push(record(old.id, SessionKind::Shell, 1));
    wn.sessions.push(record(new.id, SessionKind::Shell, 0));
    let (old_agent, old_shell, new_shell) =
        (wo.sessions[0].id, wo.sessions[1].id, wn.sessions[0].id);
    let (mut core, _) = loaded(
        vec![wo, wn],
        vec![running(old_agent), running(old_shell), running(new_shell)],
    );
    core.dispatch(
        AppAction::Events(vec![SessionEvent {
            record_id: Some(old_agent),
            ..event(EventKind::PermissionRequested { tool: None }, 100)
        }]),
        Clock::at(1),
    );
    let sorted: Vec<_> = core.all_sessions_sorted().iter().map(|s| s.id).collect();
    assert_eq!(sorted, vec![old_agent, new_shell, old_shell]);
    let recency: Vec<_> = core
        .workspaces_by_recency()
        .iter()
        .map(|w| w.project.name.as_str())
        .collect();
    assert_eq!(recency, vec!["new", "old"]);
}

#[test]
fn seed_counts_as_reconciled() {
    let p = project("p");
    let mut w = Workspace::new(p.clone());
    let mut svc = record(p.id, SessionKind::Service, 0);
    svc.autostart = true;
    w.sessions.push(svc);
    let mut core = AppCore::new();
    core.seed(vec![w], vec![]);
    let e = core.dispatch(AppAction::HostListed(vec![]), Clock::at(1));
    assert!(e.is_empty(), "seeded state never launches: {e:?}");
}

#[test]
fn clock_at_is_monotone_in_both_scales() {
    let (a, b) = (Clock::at(1_000), Clock::at(2_000));
    assert_eq!(b.mono.checked_sub(a.mono), Some(Duration::from_secs(1)));
    assert_eq!(
        b.wall.duration_since(a.wall).unwrap(),
        Duration::from_secs(1)
    );
    assert!(a.wall > SystemTime::UNIX_EPOCH);
}

#[test]
fn return_right_after_spawn_attaches_instead_of_spawning_again() {
    let mut core = AppCore::new();
    let root = PathBuf::from("/tmp/p");
    core.dispatch(AppAction::StoreLoaded(Ok(Loaded::default())), Clock::at(0));
    core.dispatch(AppAction::HostListed(vec![]), Clock::at(1));
    core.dispatch(
        AppAction::AddProject {
            name: "p".into(),
            root: root.clone(),
        },
        Clock::at(2),
    );
    let project = core.workspaces()[0].project.id;
    let effects = core.dispatch(
        AppAction::NewSession {
            project,
            name: "sh".into(),
            kind: SessionKind::Shell,
            cwd: root,
            launch: Launch::Shell,
            outputs: Vec::new(),
        },
        Clock::at(3),
    );
    assert!(effects.iter().any(|e| matches!(e, Effect::Spawn { .. })));
    let id = core.workspaces()[0].sessions[0].id;
    core.dispatch(AppAction::Spawned { id, result: Ok(()) }, Clock::at(4));
    // No host poll has happened yet.
    let effects = core.dispatch(AppAction::ReturnToSession(id), Clock::at(5));
    assert!(
        effects.iter().any(|e| matches!(e, Effect::Attach { .. })),
        "expected Attach, got {effects:?}"
    );
    assert!(!effects.iter().any(|e| matches!(e, Effect::Spawn { .. })));
}

#[test]
fn send_input_targets_a_running_session_only() {
    let mut core = AppCore::new();
    let root = PathBuf::from("/tmp/p");
    core.dispatch(AppAction::StoreLoaded(Ok(Loaded::default())), Clock::at(0));
    core.dispatch(AppAction::HostListed(vec![]), Clock::at(1));
    core.dispatch(
        AppAction::AddProject {
            name: "p".into(),
            root: root.clone(),
        },
        Clock::at(2),
    );
    let project = core.workspaces()[0].project.id;
    core.dispatch(
        AppAction::NewSession {
            project,
            name: "sh".into(),
            kind: SessionKind::Shell,
            cwd: root,
            launch: Launch::Shell,
            outputs: Vec::new(),
        },
        Clock::at(3),
    );
    let id = core.workspaces()[0].sessions[0].id;
    // Not running yet (spawn not confirmed): an error notice, no effect.
    let effects = core.dispatch(
        AppAction::SendInput {
            id,
            text: "ls".into(),
        },
        Clock::at(4),
    );
    assert!(effects.is_empty());
    assert!(core.notices().iter().any(|n| n.is_error));
    core.dispatch(
        AppAction::HostListed(vec![HostStatus {
            id: HostId(id.host_name()),
            liveness: Liveness::Running {
                pid: 1,
                command: "zsh".into(),
            },
            cwd: None,
            last_activity: None,
            title: None,
        }]),
        Clock::at(5),
    );
    let effects = core.dispatch(
        AppAction::SendInput {
            id,
            text: "ls".into(),
        },
        Clock::at(6),
    );
    assert_eq!(
        effects,
        vec![Effect::SendInput {
            host: HostId(id.host_name()),
            text: "ls".into()
        }]
    );
}

#[test]
fn interrupt_sends_escape_only_to_a_running_pane() {
    let (mut core, _, ids) = with_records(&[SessionKind::Shell], |_| None);
    let id = ids[0];
    let effects = core.dispatch(AppAction::Interrupt(id), Clock::at(1));
    assert!(effects.is_empty());
    assert!(core.notices().iter().any(|n| n.is_error));

    core.dispatch(AppAction::HostListed(vec![running(id)]), Clock::at(2));
    let effects = core.dispatch(AppAction::Interrupt(id), Clock::at(3));
    assert_eq!(
        effects,
        vec![Effect::SendKeys {
            host: HostId(id.host_name()),
            bytes: vec![0x1b],
        }]
    );
}

#[test]
fn return_to_an_exited_pane_kills_it_and_resumes() {
    let mut core = AppCore::new();
    let root = PathBuf::from("/tmp/p");
    core.dispatch(AppAction::StoreLoaded(Ok(Loaded::default())), Clock::at(0));
    core.dispatch(AppAction::HostListed(vec![]), Clock::at(1));
    core.dispatch(
        AppAction::AddProject {
            name: "p".into(),
            root: root.clone(),
        },
        Clock::at(2),
    );
    let project = core.workspaces()[0].project.id;
    core.dispatch(
        AppAction::NewSession {
            project,
            name: "sh".into(),
            kind: SessionKind::Shell,
            cwd: root,
            launch: Launch::Shell,
            outputs: Vec::new(),
        },
        Clock::at(3),
    );
    let id = core.workspaces()[0].sessions[0].id;
    core.dispatch(AppAction::Spawned { id, result: Ok(()) }, Clock::at(4));
    core.dispatch(
        AppAction::HostListed(vec![HostStatus {
            id: HostId(id.host_name()),
            liveness: Liveness::Exited { code: Some(0) },
            cwd: None,
            last_activity: None,
            title: None,
        }]),
        Clock::at(5),
    );
    assert_eq!(core.card_state(id), CardState::Exited(Some(0)));
    let effects = core.dispatch(AppAction::ReturnToSession(id), Clock::at(6));
    assert!(matches!(effects.first(), Some(Effect::Kill(h)) if h.0 == id.host_name()));
    assert!(effects.iter().any(|e| matches!(e, Effect::Spawn { .. })));
    assert!(!effects.iter().any(|e| matches!(e, Effect::Attach { .. })));
}

// --- 9. definitions from .switchboard/project.json

fn entry(name: &str, kind: SessionKind, command: &str) -> DefinedEntry {
    DefinedEntry {
        name: name.into(),
        kind,
        command: command.into(),
        cwd: None,
        env: Vec::new(),
        autostart: false,
        outputs: Vec::new(),
    }
}

/// The reader's success shape for a file with these entries.
#[allow(clippy::unnecessary_wraps)]
#[test]
fn saving_the_config_writes_it_then_reads_it_back() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    let e = core.dispatch(
        AppAction::SaveProjectConfig {
            project: pid,
            text: "{}".into(),
        },
        Clock::at(1),
    );
    assert_eq!(
        e,
        vec![Effect::WriteProjectConfig {
            project: pid,
            root: PathBuf::from("/tmp/proj"),
            text: "{}".into(),
        }]
    );
    let e = core.dispatch(
        AppAction::ProjectConfigWritten {
            project: pid,
            result: Ok(()),
        },
        Clock::at(2),
    );
    assert_eq!(
        e,
        vec![Effect::ReadProjectConfig {
            project: pid,
            root: PathBuf::from("/tmp/proj"),
        }]
    );
    assert!(core.notices().last().unwrap().text.contains("saved"));
    let e = core.dispatch(
        AppAction::ProjectConfigWritten {
            project: pid,
            result: Err("read-only".into()),
        },
        Clock::at(3),
    );
    assert!(e.is_empty());
    assert!(core.notices().last().unwrap().text.contains("read-only"));
}

#[test]
fn the_config_files_show_list_lands_on_the_project_record() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    let mut cfg = ProjectConfig {
        shell: "/bin/zsh".into(),
        show: vec![PathBuf::from("manager")],
        ..ProjectConfig::default()
    };
    let e = core.dispatch(
        AppAction::ProjectConfigRead {
            project: pid,
            result: Ok(Some(cfg.clone())),
        },
        Clock::at(1),
    );
    assert_eq!(saves(&e), 1);
    assert_eq!(
        core.workspace(pid).unwrap().project.shown,
        [PathBuf::from("manager")]
    );
    // The same list again is not a change; an unreadable file keeps it;
    // a removed file clears it.
    let e = core.dispatch(
        AppAction::ProjectConfigRead {
            project: pid,
            result: Ok(Some(cfg.clone())),
        },
        Clock::at(2),
    );
    assert_eq!(saves(&e), 0);
    cfg.show.clear();
    let e = core.dispatch(
        AppAction::ProjectConfigRead {
            project: pid,
            result: Err("unreadable".into()),
        },
        Clock::at(3),
    );
    assert_eq!(saves(&e), 0);
    assert_eq!(core.workspace(pid).unwrap().project.shown.len(), 1);
    let e = core.dispatch(
        AppAction::ProjectConfigRead {
            project: pid,
            result: Ok(None),
        },
        Clock::at(4),
    );
    assert_eq!(saves(&e), 1);
    assert!(core.workspace(pid).unwrap().project.shown.is_empty());
}

#[allow(clippy::unnecessary_wraps)]
fn config(entries: Vec<DefinedEntry>) -> Result<Option<ProjectConfig>, String> {
    Ok(Some(ProjectConfig {
        entries,
        warnings: vec![],
        shell: "/bin/zsh".into(),
        show: vec![],
    }))
}

fn read(
    core: &mut AppCore,
    pid: ProjectId,
    result: Result<Option<ProjectConfig>, String>,
    at: u64,
) -> Vec<Effect> {
    core.dispatch(
        AppAction::ProjectConfigRead {
            project: pid,
            result,
        },
        Clock::at(at),
    )
}

fn defined(core: &AppCore, pid: ProjectId, name: &str) -> SessionRecord {
    core.workspace(pid)
        .unwrap()
        .sessions
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no record {name}"))
        .clone()
}

#[test]
fn project_config_read_adds_pending_records_once() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    let entries = vec![
        entry("lint", SessionKind::Command, "cargo clippy"),
        DefinedEntry {
            cwd: Some("web".into()),
            env: vec!["PORT".into()],
            autostart: true,
            ..entry("web", SessionKind::Service, "npm run dev")
        },
    ];
    let effects = read(&mut core, pid, config(entries.clone()), 1);
    assert_eq!(saves(&effects), 1);
    assert!(spawns(&effects).is_empty(), "listing never runs anything");

    let lint = defined(&core, pid, "lint");
    assert_eq!(lint.kind, SessionKind::Command);
    assert_eq!(lint.cwd, PathBuf::from("/tmp/proj"));
    assert_eq!(
        lint.launch,
        Launch::Command {
            command: "cargo clippy".into(),
            shell: "/bin/zsh".into()
        }
    );
    assert_eq!(lint.approval(), Approval::Pending);
    assert!(!lint.runnable());
    let web = defined(&core, pid, "web");
    assert_eq!(web.cwd, PathBuf::from("/tmp/proj/web"));
    let source = web.source.as_ref().unwrap();
    assert_eq!(source.env, vec!["PORT".to_string()]);
    assert!(source.autostart);
    assert!(!web.effective_autostart(), "not approved, so no autostart");
    assert_eq!(source.hash, entry_hash(&entries[1]));
    let status = core.config_status(pid).unwrap();
    assert!(status.present && status.warnings.is_empty() && status.error.is_none());

    // The same file again changes nothing and saves nothing.
    let ids_before: Vec<_> = core
        .workspace(pid)
        .unwrap()
        .sessions
        .iter()
        .map(|s| s.id)
        .collect();
    let effects = read(&mut core, pid, config(entries), 2);
    assert!(effects.is_empty(), "{effects:?}");
    let ids_after: Vec<_> = core
        .workspace(pid)
        .unwrap()
        .sessions
        .iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(ids_before, ids_after);
}

#[test]
fn approve_and_revoke_save_and_gate_launch() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    read(
        &mut core,
        pid,
        config(vec![entry("lint", SessionKind::Command, "cargo clippy")]),
        1,
    );
    let id = defined(&core, pid, "lint").id;

    // Unapproved: Return and Restart refuse with a notice, no spawn.
    for action in [
        AppAction::ReturnToSession(id),
        AppAction::RestartSession(id),
    ] {
        let effects = core.dispatch(action, Clock::at(2));
        assert!(spawns(&effects).is_empty(), "{effects:?}");
        assert!(
            core.notices()
                .iter()
                .any(|n| n.is_error && n.text.contains("approve"))
        );
        assert!(!core.is_in_flight(id));
    }

    let effects = core.dispatch(AppAction::ApproveDefinition(id), Clock::at(3));
    assert_eq!(saves(&effects), 1);
    let lint = defined(&core, pid, "lint");
    assert_eq!(lint.approval(), Approval::Approved);
    assert!(lint.runnable());

    // Approved: a Return launches it.
    let effects = core.dispatch(AppAction::ReturnToSession(id), Clock::at(4));
    assert_eq!(spawns(&effects).len(), 1);

    let effects = core.dispatch(AppAction::RevokeApproval(id), Clock::at(5));
    assert_eq!(saves(&effects), 1);
    assert_eq!(defined(&core, pid, "lint").approval(), Approval::Pending);
}

#[test]
fn changed_entry_drops_approval_but_keeps_the_record() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    read(
        &mut core,
        pid,
        config(vec![entry("lint", SessionKind::Command, "cargo clippy")]),
        1,
    );
    let id = defined(&core, pid, "lint").id;
    core.dispatch(AppAction::ApproveDefinition(id), Clock::at(2));
    core.dispatch(
        AppAction::HostListed(vec![exited(id, Some(1))]),
        Clock::at(3),
    );
    assert_eq!(defined(&core, pid, "lint").last_exit, Some(1));

    let effects = read(
        &mut core,
        pid,
        config(vec![entry(
            "lint",
            SessionKind::Command,
            "cargo clippy -- -D warnings",
        )]),
        4,
    );
    assert_eq!(saves(&effects), 1);
    let lint = defined(&core, pid, "lint");
    assert_eq!(lint.id, id, "same record");
    assert_eq!(lint.approval(), Approval::Changed);
    assert!(!lint.runnable());
    assert_eq!(lint.last_exit, Some(1), "history kept");
    assert!(
        matches!(&lint.launch, Launch::Command { command, .. } if command.ends_with("warnings"))
    );

    // Reverting the edit restores the approval: it is keyed to content.
    read(
        &mut core,
        pid,
        config(vec![entry("lint", SessionKind::Command, "cargo clippy")]),
        5,
    );
    assert_eq!(defined(&core, pid, "lint").approval(), Approval::Approved);
}

#[test]
fn removed_entry_or_missing_file_orphans_not_deletes() {
    let (mut core, pid, _) = with_records(&[SessionKind::Shell], |_| None);
    read(
        &mut core,
        pid,
        config(vec![
            entry("lint", SessionKind::Command, "x"),
            entry("web", SessionKind::Service, "y"),
        ]),
        1,
    );
    let effects = read(
        &mut core,
        pid,
        config(vec![entry("web", SessionKind::Service, "y")]),
        2,
    );
    assert_eq!(saves(&effects), 1);
    assert_eq!(defined(&core, pid, "lint").approval(), Approval::Orphaned);
    assert_eq!(defined(&core, pid, "web").approval(), Approval::Pending);

    read(&mut core, pid, Ok(None), 3);
    assert_eq!(defined(&core, pid, "web").approval(), Approval::Orphaned);
    assert!(!core.config_status(pid).unwrap().present);
    // The user's own shell record is untouched and still runnable.
    let shell = defined(&core, pid, "s0");
    assert_eq!(shell.approval(), Approval::NotApplicable);
    assert!(shell.runnable());
    assert_eq!(core.workspace(pid).unwrap().sessions.len(), 3);

    // An orphan cannot be approved.
    let id = defined(&core, pid, "web").id;
    let effects = core.dispatch(AppAction::ApproveDefinition(id), Clock::at(4));
    assert!(effects.is_empty());
    assert!(core.notices().iter().any(|n| n.is_error));
}

#[test]
fn autostart_from_the_file_needs_approval_and_a_live_definition() {
    let p = project("p");
    let pid = p.id;
    let mut core = AppCore::new();
    core.dispatch(
        AppAction::StoreLoaded(Ok(Loaded {
            workspaces: vec![Workspace::new(p)],
            ..Loaded::default()
        })),
        Clock::at(0),
    );
    let web = DefinedEntry {
        autostart: true,
        ..entry("web", SessionKind::Service, "npm run dev")
    };
    read(&mut core, pid, config(vec![web.clone()]), 1);
    // First host poll is the reconcile: pending entries never start.
    let effects = core.dispatch(AppAction::HostListed(vec![]), Clock::at(2));
    assert!(spawns(&effects).is_empty(), "{effects:?}");

    let id = defined(&core, pid, "web").id;
    core.dispatch(AppAction::ApproveDefinition(id), Clock::at(3));
    assert!(defined(&core, pid, "web").effective_autostart());
    // Approval alone launches nothing; the next reconcile does.
    core.dispatch(
        AppAction::StoreLoaded(Ok(Loaded {
            workspaces: core.workspaces().to_vec(),
            ..Loaded::default()
        })),
        Clock::at(4),
    );
    read(&mut core, pid, config(vec![web]), 5);
    let effects = core.dispatch(AppAction::HostListed(vec![]), Clock::at(6));
    assert_eq!(spawns(&effects).len(), 1);

    // Orphaned: approved once, but gone from the file, so no autostart.
    core.dispatch(AppAction::HostListed(vec![]), Clock::at(7));
    read(&mut core, pid, Ok(None), 8);
    assert!(!defined(&core, pid, "web").effective_autostart());
}

#[test]
fn config_error_is_one_notice_until_it_changes() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    read(
        &mut core,
        pid,
        config(vec![entry("lint", SessionKind::Command, "x")]),
        1,
    );
    core.dispatch(AppAction::DismissNotice, Clock::at(1));
    for at in 2..5 {
        read(&mut core, pid, Err("project.json: bad".into()), at);
    }
    assert_eq!(core.notices().len(), 1);
    assert!(core.notices()[0].text.contains("project.json: bad"));
    // The records from the last good read stay as they were.
    assert_eq!(defined(&core, pid, "lint").approval(), Approval::Pending);
    let status = core.config_status(pid).unwrap();
    assert_eq!(status.error.as_deref(), Some("project.json: bad"));
    read(&mut core, pid, Err("project.json: worse".into()), 5);
    assert_eq!(core.notices().len(), 2);
}

#[test]
fn entry_hash_is_stable_and_order_sensitive() {
    let e = DefinedEntry {
        cwd: Some("web".into()),
        env: vec!["A".into(), "B".into()],
        ..entry("web", SessionKind::Service, "npm run dev")
    };
    let h = entry_hash(&e);
    assert_eq!(h.len(), 64);
    assert_eq!(h, entry_hash(&e.clone()));
    // Pinned: a change here would silently drop every saved approval.
    assert_eq!(
        h,
        "8bb9356dd9ae7d4f35d7078088e284357a0e308721da3e0b3e09e518dd44b436"
    );
    let swapped = DefinedEntry {
        env: vec!["B".into(), "A".into()],
        ..e.clone()
    };
    assert_ne!(h, entry_hash(&swapped));
    let auto = DefinedEntry {
        autostart: true,
        ..e.clone()
    };
    assert_ne!(h, entry_hash(&auto));
    let kind = DefinedEntry {
        kind: SessionKind::Command,
        ..e
    };
    assert_ne!(h, entry_hash(&kind));
}

#[test]
fn set_side_tab_saves_settings() {
    let mut core = AppCore::new();
    let effects = core.dispatch(AppAction::SetSideTab(SideTab::Run), Clock::at(1));
    assert_eq!(core.settings().side_tab, SideTab::Run);
    assert!(matches!(effects[..], [Effect::SaveSettings(_)]));
}

#[test]
fn working_sets_are_made_and_take_cards_once() {
    let (mut core, pid, ids) = with_records(&[SessionKind::Shell, SessionKind::Command], |_| None);
    assert!(core.working_sets().is_empty());
    let shell = PinTarget::Session(ids[0]);
    // A new set with a card is shown at once and saved.
    let e = core.dispatch(
        AppAction::NewWorkingSet {
            name: None,
            clone_of: None,
            with: Some(shell.clone()),
            columns: 24,
        },
        Clock::at(1),
    );
    let first = core.working_sets()[0].id;
    assert_eq!(core.view(), View::WorkingSet(first));
    assert_eq!(core.working_sets()[0].name, "Working Set");
    assert_eq!(core.sets_holding(&shell), vec![first]);
    assert!(
        e.iter()
            .any(|e| matches!(e, Effect::SaveViews(v) if v.sets.len() == 1)),
        "{e:?}"
    );
    // Adding to a set: once, in the first free spot; twice is once.
    let command = PinTarget::Session(ids[1]);
    core.dispatch(
        AppAction::AddToWorkingSet {
            set: first,
            target: command.clone(),
            columns: 24,
        },
        Clock::at(2),
    );
    let e = core.dispatch(
        AppAction::AddToWorkingSet {
            set: first,
            target: command.clone(),
            columns: 24,
        },
        Clock::at(3),
    );
    assert!(e.is_empty(), "{e:?}");
    let rects: Vec<GridRect> = core
        .working_set(first)
        .unwrap()
        .items
        .iter()
        .map(|i| i.rect)
        .collect();
    assert_eq!(
        rects[0],
        GridRect {
            x: 0,
            y: 0,
            w: 10,
            h: 8
        }
    );
    assert_eq!(
        rects[1],
        GridRect {
            x: 10,
            y: 0,
            w: 10,
            h: 7
        }
    );
    // A target that does not exist is not added; a file needs its project.
    let e = core.dispatch(
        AppAction::AddToWorkingSet {
            set: first,
            target: PinTarget::Session(RecordId::new()),
            columns: 24,
        },
        Clock::at(11),
    );
    assert!(e.is_empty());
    core.dispatch(
        AppAction::AddToWorkingSet {
            set: first,
            target: PinTarget::File(pid, "README.md".into()),
            columns: 24,
        },
        Clock::at(12),
    );
    assert_eq!(core.working_set(first).unwrap().items.len(), 3);
}

#[test]
fn working_sets_are_named_cloned_renamed_and_deleted() {
    let (mut core, _, ids) = with_records(&[SessionKind::Shell, SessionKind::Command], |_| None);
    let shell = PinTarget::Session(ids[0]);
    core.dispatch(
        AppAction::NewWorkingSet {
            name: None,
            clone_of: None,
            with: Some(shell.clone()),
            columns: 24,
        },
        Clock::at(1),
    );
    let first = core.working_sets()[0].id;
    core.dispatch(
        AppAction::AddToWorkingSet {
            set: first,
            target: PinTarget::Session(ids[1]),
            columns: 24,
        },
        Clock::at(2),
    );
    // A second, named set; a target can sit on both.
    core.dispatch(
        AppAction::NewWorkingSet {
            name: Some("  Release  ".into()),
            clone_of: None,
            with: None,
            columns: 24,
        },
        Clock::at(4),
    );
    let second = core.working_sets()[1].id;
    assert_eq!(core.working_sets()[1].name, "Release");
    assert_eq!(core.view(), View::WorkingSet(second));
    core.dispatch(
        AppAction::AddToWorkingSet {
            set: second,
            target: shell.clone(),
            columns: 24,
        },
        Clock::at(5),
    );
    assert_eq!(core.sets_holding(&shell), vec![first, second]);
    // A clone copies the cards and takes the name with "copy".
    core.dispatch(
        AppAction::NewWorkingSet {
            name: None,
            clone_of: Some(first),
            with: None,
            columns: 24,
        },
        Clock::at(6),
    );
    let third = core.working_sets()[2].id;
    assert_eq!(core.working_sets()[2].name, "Working Set copy");
    assert_eq!(core.working_set(third).unwrap().items.len(), 2);
    assert_ne!(third, first);
    // Rename trims and refuses blank; delete drops the set and its view.
    core.dispatch(
        AppAction::RenameWorkingSet {
            set: third,
            name: " Hotfix ".into(),
        },
        Clock::at(7),
    );
    assert_eq!(core.working_set(third).unwrap().name, "Hotfix");
    let e = core.dispatch(
        AppAction::RenameWorkingSet {
            set: third,
            name: "  ".into(),
        },
        Clock::at(8),
    );
    assert!(e.is_empty());
    assert_eq!(core.view(), View::WorkingSet(third));
    let e = core.dispatch(AppAction::DeleteWorkingSet(third), Clock::at(9));
    assert!(matches!(e[0], Effect::SaveViews(_)));
    assert!(core.working_set(third).is_none());
    assert_eq!(
        core.view(),
        View::WorkingSet(second),
        "back to the one before"
    );
    // Removing from one set leaves the other.
    core.dispatch(
        AppAction::RemoveFromWorkingSet {
            set: first,
            target: shell.clone(),
        },
        Clock::at(10),
    );
    assert_eq!(core.sets_holding(&shell), vec![second]);
}

#[test]
fn working_set_places_a_card_only_where_it_fits() {
    let (mut core, pid, ids) = with_records(&[SessionKind::Shell], |_| None);
    let file = PinTarget::File(pid, "README.md".into());
    core.dispatch(
        AppAction::NewWorkingSet {
            name: None,
            clone_of: None,
            with: Some(PinTarget::Session(ids[0])),
            columns: 24,
        },
        Clock::at(1),
    );
    let set = core.working_sets()[0].id;
    core.dispatch(
        AppAction::AddToWorkingSet {
            set,
            target: file.clone(),
            columns: 24,
        },
        Clock::at(1),
    );
    assert_eq!(
        core.working_set(set).unwrap().items[1].rect,
        GridRect {
            x: 10,
            y: 0,
            w: 10,
            h: 10
        }
    );
    // A move onto another card is refused; a move into free space is kept.
    let e = core.dispatch(
        AppAction::PlacePin {
            set,
            target: file.clone(),
            rect: GridRect {
                x: 5,
                y: 2,
                w: 10,
                h: 10,
            },
        },
        Clock::at(5),
    );
    assert!(e.is_empty(), "{e:?}");
    let e = core.dispatch(
        AppAction::PlacePin {
            set,
            target: file.clone(),
            rect: GridRect {
                x: 17,
                y: 0,
                w: 1,
                h: 1,
            },
        },
        Clock::at(6),
    );
    assert_eq!(e.len(), 1);
    assert_eq!(
        core.working_set(set).unwrap().items[1].rect,
        GridRect {
            x: 17,
            y: 0,
            w: 3,
            h: 2
        },
        "clamped to the minimum"
    );
}

#[test]
fn working_set_drops_cards_whose_session_or_project_is_gone() {
    let (mut core, pid, ids) = with_records(&[SessionKind::Shell], |_| None);
    let shell = PinTarget::Session(ids[0]);
    let file = PinTarget::File(pid, "a.md".into());
    core.dispatch(
        AppAction::NewWorkingSet {
            name: None,
            clone_of: None,
            with: Some(shell.clone()),
            columns: 24,
        },
        Clock::at(1),
    );
    let set = core.working_sets()[0].id;
    core.dispatch(
        AppAction::AddToWorkingSet {
            set,
            target: file.clone(),
            columns: 24,
        },
        Clock::at(1),
    );
    let e = core.dispatch(AppAction::RemoveSession(ids[0]), Clock::at(2));
    assert!(
        e.iter()
            .any(|e| matches!(e, Effect::SaveViews(v) if v.sets[0].items.len() == 1)),
        "{e:?}"
    );
    assert!(core.sets_holding(&shell).is_empty());
    assert_eq!(core.sets_holding(&file), vec![set]);
    core.dispatch(AppAction::RemoveProject(pid), Clock::at(3));
    assert!(core.working_set(set).unwrap().items.is_empty());
}

#[test]
fn working_set_loads_and_is_pruned_and_the_view_is_restored() {
    let p = project("p");
    let mut w = Workspace::new(p.clone());
    let r = record(p.id, SessionKind::Shell, 0);
    let id = r.id;
    w.sessions.push(r);
    let mut views = Views::default();
    let set = crate::core::SetId::new();
    views.sets.push(crate::core::WorkingSet {
        id: set,
        name: "Working Set".into(),
        space: SpaceId::DEFAULT,
        items: vec![
            crate::core::PinnedItem {
                target: PinTarget::Session(id),
                rect: GridRect {
                    x: 0,
                    y: 0,
                    w: 10,
                    h: 8,
                },
            },
            crate::core::PinnedItem {
                target: PinTarget::Session(RecordId::new()),
                rect: GridRect {
                    x: 10,
                    y: 0,
                    w: 10,
                    h: 8,
                },
            },
        ],
    });
    let load = |last_view: SavedView| {
        let mut core = AppCore::new();
        let effects = core.dispatch(
            AppAction::StoreLoaded(Ok(Loaded {
                workspaces: vec![w.clone()],
                settings: Settings {
                    last_view,
                    ..Settings::default()
                },
                views: views.clone(),
                ..Loaded::default()
            })),
            Clock::at(0),
        );
        (core, effects)
    };
    let (core, effects) = load(SavedView::Set(set));
    assert_eq!(core.view(), View::WorkingSet(set));
    assert_eq!(
        core.working_set(set).unwrap().items.len(),
        1,
        "the stale card is dropped"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::SaveViews(v) if v.sets[0].items.len() == 1)),
        "{effects:?}"
    );
    assert!(!effects.iter().any(|e| matches!(e, Effect::Spawn { .. })));
    // The old unit variant means the first set; a gone set means the switchboard.
    let (core, _) = load(SavedView::WorkingSet);
    assert_eq!(core.view(), View::WorkingSet(set));
    let (core, _) = load(SavedView::Set(crate::core::SetId::new()));
    assert_eq!(core.view(), View::Switchboard);
}

// --- workflows: the plan review loop

mod workflow {
    use super::*;
    use crate::core::model::{
        BUILTIN_WORKFLOW, HandoffMode, RunState, Verdict, WorkflowDefinition, WorkflowId,
    };
    use crate::core::{SETTLE_PROBES, round_paths};
    use crate::ports::round_files::{FileStamp, Probed};

    const PLAN: &str = "/tmp/proj/docs/plan.md";

    #[allow(clippy::unnecessary_wraps)] // the shape `found` takes
    fn probed(first_line: &str, version: u64) -> Option<Probed> {
        Some(Probed {
            stamp: FileStamp {
                modified: std::time::UNIX_EPOCH + Duration::from_secs(version),
                len: version,
            },
            first_line: first_line.into(),
        })
    }

    fn run_of(core: &AppCore) -> &crate::core::WorkflowRun {
        core.workflows().next().expect("a run")
    }

    fn spawn_argv(effects: &[Effect]) -> Vec<String> {
        effects
            .iter()
            .find_map(|e| match e {
                Effect::Spawn { spec, .. } => spec.command.clone(),
                _ => None,
            })
            .expect("a spawn")
    }

    fn sent(effects: &[Effect]) -> Vec<(HostId, String)> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::SendInput { host, text } => Some((host.clone(), text.clone())),
                _ => None,
            })
            .collect()
    }

    /// Settle the awaited file with `first_line`; returns the effects of
    /// the probe that settled it.
    fn settle(core: &mut AppCore, run: WorkflowId, first_line: &str, at: u64) -> Vec<Effect> {
        let path = core.workflow(run).unwrap().awaited_file().unwrap().clone();
        let mut last = Vec::new();
        for i in 0..SETTLE_PROBES {
            last = core.dispatch(
                AppAction::RoundFileProbed {
                    run,
                    path: path.clone(),
                    found: probed(first_line, 7),
                },
                Clock::at(at + u64::from(i)),
            );
        }
        last
    }

    /// A run past its start: the reviewer spawned (so its pane counts as
    /// running), the planner cloned, the first feedback awaited.
    fn started() -> (AppCore, WorkflowId, RecordId, RecordId, RecordId) {
        let (mut core, source) = resumable_agent();
        let effects = core.dispatch(
            AppAction::StartWorkflow {
                source,
                plan: PLAN.into(),
                definition: BUILTIN_WORKFLOW.into(),
            },
            Clock::at(100),
        );
        let run = run_of(&core).id;
        let reviewer = run_of(&core).reviewer;
        assert!(matches!(
            effects.iter().find(|e| matches!(e, Effect::PrepareLaunch { .. })),
            Some(Effect::PrepareLaunch { id, kind: AgentKind::Codex, .. }) if *id == reviewer
        ));
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::CloneAllTranscript { run: r, .. } if *r == run))
        );
        let effects = core.dispatch(
            AppAction::LaunchPrepared {
                id: reviewer,
                result: Ok(AgentLaunch {
                    argv: vec!["codex".into()],
                    env: vec![],
                    resume: None,
                }),
            },
            Clock::at(110),
        );
        let argv = spawn_argv(&effects);
        assert_eq!(argv[0], "codex");
        assert!(
            argv[1].contains("/tmp/proj/docs/plan.feedback-1.md"),
            "{argv:?}"
        );
        assert!(argv[1].contains("No further feedback."), "{argv:?}");
        core.dispatch(
            AppAction::Spawned {
                id: reviewer,
                result: Ok(()),
            },
            Clock::at(120),
        );
        // Codex ids are discovered after the spawn; until then the
        // reviewer is in flight.
        core.dispatch(
            AppAction::Discovered {
                id: reviewer,
                result: Ok(Some(ResumeHandle::Codex {
                    rollout_id: "r-1".into(),
                    transcript: None,
                })),
            },
            Clock::at(125),
        );
        core.dispatch(
            AppAction::WorkflowCloned {
                run,
                result: Ok(claude_handle()),
            },
            Clock::at(130),
        );
        let r = run_of(&core);
        assert_eq!(r.state, RunState::AwaitingFeedback);
        let planner = r.planner.expect("planner");
        assert!(core.session(planner).unwrap().resume.is_some());
        assert_eq!(core.view(), View::Workflow(run));
        (core, run, source, reviewer, planner)
    }

    #[test]
    fn start_needs_a_clonable_planner_and_an_absolute_plan() {
        let (mut core, _, ids) = with_records(&[codex()], |_| None);
        core.dispatch(
            AppAction::StartWorkflow {
                source: ids[0],
                plan: PLAN.into(),
                definition: BUILTIN_WORKFLOW.into(),
            },
            Clock::at(1),
        );
        assert_eq!(core.workflows().count(), 0);
        assert!(core.notices().iter().any(|n| n.is_error));
        let (mut core, source) = resumable_agent();
        core.dispatch(
            AppAction::StartWorkflow {
                source,
                plan: "docs/plan.md".into(),
                definition: BUILTIN_WORKFLOW.into(),
            },
            Clock::at(1),
        );
        assert_eq!(core.workflows().count(), 0);
    }

    #[test]
    fn the_run_probes_once_a_tick_and_ignores_the_missing_file() {
        let (mut core, run, ..) = started();
        let (feedback, _) = round_paths(std::path::Path::new(PLAN), 1);
        let effects = core.dispatch(AppAction::Tick, Clock::at(200));
        assert_eq!(
            effects,
            vec![Effect::ProbeRoundFile {
                run,
                path: feedback.clone()
            }]
        );
        core.dispatch(
            AppAction::RoundFileProbed {
                run,
                path: feedback,
                found: None,
            },
            Clock::at(201),
        );
        assert_eq!(run_of(&core).state, RunState::AwaitingFeedback);
    }

    #[test]
    fn feedback_settles_after_unchanged_probes_and_the_planner_is_resumed_with_the_prompt() {
        let (mut core, run, _, _, planner) = started();
        let (feedback, response) = round_paths(std::path::Path::new(PLAN), 1);
        // A file still being written does not count.
        for v in 0..2 {
            core.dispatch(
                AppAction::RoundFileProbed {
                    run,
                    path: feedback.clone(),
                    found: probed("- item", v),
                },
                Clock::at(300 + v),
            );
        }
        assert_eq!(run_of(&core).state, RunState::AwaitingFeedback);
        let effects = settle(&mut core, run, "- item", 400);
        let r = run_of(&core);
        assert_eq!(r.state, RunState::AwaitingResponse);
        assert_eq!(r.rounds[0].verdict, Some(Verdict::Changes));
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::SnapshotRound { n: 1, files, note: None, .. } if files.len() == 3
        )));
        // The planner is cold, so the prompt rides on its resume.
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::CheckTranscript { id, .. } if *id == planner))
        );
        core.dispatch(
            AppAction::TranscriptChecked {
                id: planner,
                exists: true,
            },
            Clock::at(410),
        );
        let effects = core.dispatch(
            AppAction::LaunchPrepared {
                id: planner,
                result: Ok(AgentLaunch {
                    argv: vec!["claude".into(), "--resume".into(), "x".into()],
                    env: vec![],
                    resume: Some(claude_handle()),
                }),
            },
            Clock::at(420),
        );
        let argv = spawn_argv(&effects);
        assert_eq!(argv.len(), 4);
        assert!(argv[3].contains(&feedback.display().to_string()));
        assert!(argv[3].contains(&response.display().to_string()));
        assert!(argv[3].contains("Never mention the reviewer"));
    }

    #[test]
    fn a_response_opens_the_next_round_for_the_running_reviewer() {
        let (mut core, run, _, reviewer, _) = started();
        settle(&mut core, run, "- item", 400);
        let effects = settle(&mut core, run, "accepted", 500);
        let r = run_of(&core);
        assert_eq!(r.state, RunState::AwaitingFeedback);
        assert_eq!(r.rounds.len(), 2);
        assert!(r.rounds[0].responded);
        let sent = sent(&effects);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, HostId(reviewer.host_name()));
        assert!(sent[0].1.contains("plan.response-1.md"), "{}", sent[0].1);
        assert!(sent[0].1.contains("plan.feedback-2.md"), "{}", sent[0].1);
    }

    #[test]
    fn the_no_feedback_line_converges_the_run() {
        let (mut core, run, ..) = started();
        let effects = settle(&mut core, run, "  No further feedback.  ", 400);
        let r = run_of(&core);
        assert_eq!(r.state, RunState::Converged);
        assert_eq!(r.rounds[0].verdict, Some(Verdict::Nothing));
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::SnapshotRound { .. }))
        );
        assert!(sent(&effects).is_empty());
        core.dispatch(
            AppAction::RoundSnapshotted {
                run,
                n: 1,
                result: Ok(()),
            },
            Clock::at(401),
        );
        assert!(run_of(&core).rounds[0].snapshot);
    }

    #[test]
    fn the_cap_stops_the_loop_and_continue_runs_one_more_round() {
        let (mut core, source) = resumable_agent();
        core.dispatch(AppAction::SetWorkflowRoundCap(1), Clock::at(1));
        assert_eq!(core.settings().workflow_round_cap, 1);
        core.dispatch(
            AppAction::StartWorkflow {
                source,
                plan: PLAN.into(),
                definition: BUILTIN_WORKFLOW.into(),
            },
            Clock::at(100),
        );
        let run = run_of(&core).id;
        let reviewer = run_of(&core).reviewer;
        launch_agent(&mut core, reviewer, None);
        core.dispatch(
            AppAction::WorkflowCloned {
                run,
                result: Ok(claude_handle()),
            },
            Clock::at(130),
        );
        settle(&mut core, run, "- item", 400);
        settle(&mut core, run, "accepted", 500);
        assert_eq!(run_of(&core).state, RunState::AtCap);
        let effects = core.dispatch(AppAction::ContinueWorkflow(run), Clock::at(600));
        let r = run_of(&core);
        assert_eq!(r.state, RunState::AwaitingFeedback);
        assert_eq!((r.rounds.len(), r.cap), (2, 2));
        assert_eq!(sent(&effects).len(), 1);
    }

    #[test]
    fn a_quiet_agent_stalls_the_run_once_and_the_mark_lifts_when_it_moves() {
        let (mut core, run, _, reviewer, _) = started();
        let quiet_since = |secs: u64| HostStatus {
            last_activity: Some(Clock::at(secs * 1000).wall),
            ..running(reviewer)
        };
        // Quiet for a minute: still working as far as anyone can tell.
        core.dispatch(
            AppAction::HostListed(vec![quiet_since(140)]),
            Clock::at(200_000),
        );
        core.dispatch(AppAction::Tick, Clock::at(200_000));
        assert!(!core.stalled(run));
        assert!(core.notice().is_none());
        // Past the stall threshold: noticed once, the agent reads as
        // waiting on the user.
        core.dispatch(AppAction::Tick, Clock::at(261_000));
        assert!(core.stalled(run));
        let text = core.notice().expect("a notice").text.clone();
        assert!(
            text.contains("plan review") && text.contains("2 min"),
            "{text}"
        );
        assert_eq!(core.card_state(reviewer), CardState::WaitingOnYou);
        core.dispatch(AppAction::DismissNotice, Clock::at(262_000));
        core.dispatch(AppAction::Tick, Clock::at(263_000));
        assert!(core.notice().is_none(), "said once");
        // Output again: the mark lifts, and a later stall is noticed anew.
        core.dispatch(
            AppAction::HostListed(vec![quiet_since(262)]),
            Clock::at(264_000),
        );
        core.dispatch(AppAction::Tick, Clock::at(264_000));
        assert!(!core.stalled(run));
        assert_eq!(core.card_state(reviewer), CardState::Working);
        core.dispatch(AppAction::Tick, Clock::at(400_000));
        assert!(core.stalled(run));
        assert!(core.notice().is_some());
    }

    #[test]
    fn an_exited_agent_pauses_the_run_and_continue_relaunches_it() {
        let (mut core, run, _, reviewer, _) = started();
        core.dispatch(
            AppAction::HostListed(vec![exited(reviewer, Some(1))]),
            Clock::at(200),
        );
        let effects = core.dispatch(AppAction::Tick, Clock::at(201));
        assert!(matches!(run_of(&core).state, RunState::Paused(ref why) if why.contains("exited")));
        assert!(
            effects.is_empty()
                || !effects
                    .iter()
                    .any(|e| matches!(e, Effect::ProbeRoundFile { .. }))
        );
        let effects = core.dispatch(AppAction::ContinueWorkflow(run), Clock::at(300));
        assert_eq!(run_of(&core).state, RunState::AwaitingFeedback);
        // The dead pane is cleared and the reviewer resumed, with the
        // first prompt on its command line again.
        assert!(effects.iter().any(|e| matches!(e, Effect::Kill(_))));
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::CheckTranscript { id, .. } if *id == reviewer))
        );
        core.dispatch(
            AppAction::TranscriptChecked {
                id: reviewer,
                exists: true,
            },
            Clock::at(310),
        );
        let effects = core.dispatch(
            AppAction::LaunchPrepared {
                id: reviewer,
                result: Ok(AgentLaunch {
                    argv: vec!["codex".into(), "resume".into(), "r-1".into()],
                    env: vec![],
                    resume: None,
                }),
            },
            Clock::at(320),
        );
        let argv = spawn_argv(&effects);
        assert_eq!(argv.len(), 4);
        assert!(argv[3].contains("plan.feedback-1.md"), "{argv:?}");
    }

    #[test]
    fn pause_by_hand_then_continue_waits_without_re_prompting_a_running_agent() {
        let (mut core, run, ..) = started();
        core.dispatch(AppAction::PauseWorkflow(run), Clock::at(200));
        assert_eq!(
            run_of(&core).state,
            RunState::Paused("paused by you".into())
        );
        assert!(core.dispatch(AppAction::Tick, Clock::at(201)).is_empty());
        let effects = core.dispatch(AppAction::ContinueWorkflow(run), Clock::at(300));
        assert_eq!(run_of(&core).state, RunState::AwaitingFeedback);
        assert!(sent(&effects).is_empty());
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::PrepareLaunch { .. }))
        );
    }

    #[test]
    fn finalize_clean_up_and_hand_off() {
        let (mut core, run, source, ..) = started();
        settle(&mut core, run, "No further feedback.", 400);
        core.dispatch(AppAction::CleanUpWorkflow(run), Clock::at(410));
        core.dispatch(AppAction::FinalizeWorkflow(run), Clock::at(420));
        assert_eq!(run_of(&core).state, RunState::Finalized);
        let (feedback, response) = round_paths(std::path::Path::new(PLAN), 1);
        let effects = core.dispatch(AppAction::CleanUpWorkflow(run), Clock::at(430));
        assert_eq!(
            effects,
            vec![Effect::RemoveRoundFiles {
                run,
                files: vec![feedback, response]
            }]
        );
        core.dispatch(
            AppAction::RoundFilesRemoved {
                run,
                result: Ok(()),
            },
            Clock::at(431),
        );
        assert!(run_of(&core).cleaned);
        // Compact needs a live pane; as-is rides on the resume.
        core.dispatch(
            AppAction::HandOffWorkflow {
                run,
                mode: HandoffMode::Compact,
            },
            Clock::at(440),
        );
        assert_eq!(run_of(&core).state, RunState::Finalized);
        let effects = core.dispatch(
            AppAction::HandOffWorkflow {
                run,
                mode: HandoffMode::AsIs,
            },
            Clock::at(450),
        );
        assert_eq!(run_of(&core).state, RunState::HandedOff);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::CheckTranscript { id, .. } if *id == source))
        );
        assert_eq!(core.view(), View::Session(source));
        core.dispatch(
            AppAction::TranscriptChecked {
                id: source,
                exists: true,
            },
            Clock::at(460),
        );
        let effects = core.dispatch(
            AppAction::LaunchPrepared {
                id: source,
                result: Ok(AgentLaunch {
                    argv: vec!["claude".into()],
                    env: vec![],
                    resume: Some(claude_handle()),
                }),
            },
            Clock::at(470),
        );
        let argv = spawn_argv(&effects);
        assert!(argv[1].contains("Enter plan mode"), "{argv:?}");
    }

    #[test]
    fn compact_hand_off_sends_the_command_then_the_prompt() {
        let (mut core, run, source, ..) = started();
        settle(&mut core, run, "No further feedback.", 400);
        core.dispatch(AppAction::HostListed(vec![running(source)]), Clock::at(410));
        let effects = core.dispatch(
            AppAction::HandOffWorkflow {
                run,
                mode: HandoffMode::Compact,
            },
            Clock::at(420),
        );
        let sent = sent(&effects);
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].1, "/compact");
        assert!(sent[1].1.contains(PLAN));
    }

    #[test]
    fn fresh_hand_off_launches_a_new_session_with_the_prompt() {
        let (mut core, run, source, ..) = started();
        settle(&mut core, run, "No further feedback.", 400);
        let before = core.all_sessions_sorted().len();
        let effects = core.dispatch(
            AppAction::HandOffWorkflow {
                run,
                mode: HandoffMode::Fresh,
            },
            Clock::at(420),
        );
        assert_eq!(core.all_sessions_sorted().len(), before + 1);
        let fresh = effects
            .iter()
            .find_map(|e| match e {
                Effect::PrepareLaunch { id, .. } => Some(*id),
                _ => None,
            })
            .expect("launch");
        assert_ne!(fresh, source);
        assert!(core.session(fresh).unwrap().name.ends_with("implement"));
    }

    #[test]
    fn the_users_own_feedback_is_a_round_for_the_planner_only() {
        let (mut core, run, _, _, planner) = started();
        settle(&mut core, run, "No further feedback.", 400);
        core.dispatch(
            AppAction::HostListed(vec![running(planner)]),
            Clock::at(410),
        );
        let effects = core.dispatch(
            AppAction::UserFeedback {
                run,
                text: "  Split step 3.  ".into(),
            },
            Clock::at(420),
        );
        let r = run_of(&core);
        assert_eq!(r.state, RunState::AwaitingResponse);
        assert_eq!(r.rounds.len(), 2);
        assert_eq!(r.rounds[1].user_feedback.as_deref(), Some("Split step 3."));
        let sent = sent(&effects);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, HostId(planner.host_name()));
        assert!(sent[0].1.contains("Split step 3."));
        assert!(sent[0].1.contains("plan.response-2.md"));
        let effects = settle(&mut core, run, "ok", 500);
        assert_eq!(run_of(&core).state, RunState::Converged);
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::SnapshotRound { n: 2, note: Some(t), .. } if t == "Split step 3."
        )));
    }

    #[test]
    fn a_loaded_run_keeps_waiting_and_removal_leaves_its_sessions() {
        let (mut core, run, _, reviewer, planner) = started();
        let workspace = core.workspaces()[0].clone();
        let (mut fresh, _) = loaded(vec![workspace], vec![]);
        let effects = fresh.dispatch(AppAction::Tick, Clock::at(5));
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::ProbeRoundFile { run: r, .. } if *r == run))
        );
        fresh.dispatch(AppAction::RemoveWorkflow(run), Clock::at(6));
        assert_eq!(fresh.workflows().count(), 1, "a waiting run is not removed");
        fresh.dispatch(AppAction::PauseWorkflow(run), Clock::at(7));
        let effects = fresh.dispatch(AppAction::RemoveWorkflow(run), Clock::at(8));
        assert_eq!(fresh.workflows().count(), 0);
        assert_eq!(saves(&effects), 1);
        assert!(fresh.session(reviewer).is_some() && fresh.session(planner).is_some());
        let _ = &mut core;
    }

    #[test]
    fn definitions_are_the_builtin_or_the_users_copy() {
        let mut settings = Settings::default();
        assert_eq!(settings.workflow("nope"), None);
        let builtin = settings.workflow(BUILTIN_WORKFLOW).unwrap();
        assert_eq!(builtin, WorkflowDefinition::default());
        let mine = WorkflowDefinition {
            name: BUILTIN_WORKFLOW.into(),
            cap: Some(9),
            ..WorkflowDefinition::default()
        };
        settings.workflows.push(mine.clone());
        assert_eq!(settings.workflow(BUILTIN_WORKFLOW), Some(mine));
        let round = crate::core::Round {
            n: 2,
            feedback: "/p/plan.feedback-2.md".into(),
            response: "/p/plan.response-2.md".into(),
            verdict: None,
            user_feedback: None,
            responded: false,
            snapshot: false,
        };
        let text = builtin.render(
            "{plan} {feedback} {response} {round}/{cap} {no_feedback}",
            &round,
            std::path::Path::new("/p/plan.md"),
            4,
        );
        assert_eq!(
            text,
            "/p/plan.md /p/plan.feedback-2.md /p/plan.response-2.md 2/4 No further feedback."
        );
    }
}

// --- runs: one record per execution of a command

mod runs {
    use super::*;
    use crate::core::model::RUNS_KEPT;

    fn command_with_outputs() -> (AppCore, RecordId) {
        let p = project("p");
        let mut w = Workspace::new(p.clone());
        let mut r = record(p.id, SessionKind::Command, 0);
        r.outputs = vec!["reports/*.pdf".into()];
        let id = r.id;
        w.sessions.push(r);
        let (core, _) = loaded(vec![w], vec![]);
        (core, id)
    }

    #[test]
    fn a_launch_opens_a_run_with_its_own_log() {
        let (mut core, id) = command_with_outputs();
        let effects = core.dispatch(AppAction::ReturnToSession(id), Clock::at(5_000));
        let run = core.session(id).unwrap().last_run().unwrap().clone();
        assert_eq!(run.n, 1);
        assert_eq!(run.started, Clock::at(5_000).wall);
        assert!(run.open());
        assert_eq!(run.log, format!("{}-r1.vt", id.host_name()));
        assert!(effects.iter().any(|e| matches!(e, Effect::Spawn { .. })));
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::FindArtifacts { .. }))
        );
    }

    #[test]
    fn the_host_poll_closes_the_run_with_its_exit_code_and_asks_for_artifacts() {
        let (mut core, id) = command_with_outputs();
        core.dispatch(AppAction::ReturnToSession(id), Clock::at(5_000));
        core.dispatch(AppAction::Spawned { id, result: Ok(()) }, Clock::at(5_100));
        // Still running: nothing closes.
        core.dispatch(AppAction::HostListed(vec![running(id)]), Clock::at(6_000));
        assert!(core.session(id).unwrap().last_run().unwrap().open());
        let effects = core.dispatch(
            AppAction::HostListed(vec![exited(id, Some(0))]),
            Clock::at(9_000),
        );
        let run = core.session(id).unwrap().last_run().unwrap().clone();
        assert_eq!(run.ended, Some(Clock::at(9_000).wall));
        assert_eq!(run.exit, Some(0));
        assert_eq!(
            run.duration(Clock::at(20_000).wall),
            Some(Duration::from_secs(4))
        );
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::FindArtifacts { id: i, n: 1, patterns, since, .. }
                if *i == id && patterns == &vec!["reports/*.pdf".to_owned()] && *since == run.started
        )));
        // A second poll with the same exit changes nothing more.
        let effects = core.dispatch(
            AppAction::HostListed(vec![exited(id, Some(0))]),
            Clock::at(10_000),
        );
        assert!(effects.is_empty());
        let effects = core.dispatch(
            AppAction::ArtifactsFound {
                id,
                n: 1,
                paths: vec!["/tmp/proj/reports/a.pdf".into()],
            },
            Clock::at(11_000),
        );
        assert_eq!(saves(&effects), 1);
        assert_eq!(
            core.session(id).unwrap().last_run().unwrap().artifacts,
            vec![PathBuf::from("/tmp/proj/reports/a.pdf")]
        );
    }

    #[test]
    fn a_run_whose_pane_is_gone_closes_without_a_code_and_no_outputs_means_no_search() {
        let (mut core, _, ids) = with_records(&[SessionKind::Command], |_| None);
        let id = ids[0];
        core.dispatch(AppAction::ReturnToSession(id), Clock::at(5_000));
        core.dispatch(AppAction::Spawned { id, result: Ok(()) }, Clock::at(5_100));
        let effects = core.dispatch(AppAction::HostListed(vec![]), Clock::at(7_000));
        let run = core.session(id).unwrap().last_run().unwrap().clone();
        assert_eq!(run.ended, Some(Clock::at(7_000).wall));
        assert_eq!(run.exit, None);
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::FindArtifacts { .. }))
        );
    }

    #[test]
    fn only_the_last_runs_are_kept_and_the_older_logs_are_removed() {
        let (mut core, _, ids) = with_records(&[SessionKind::Command], |_| None);
        let id = ids[0];
        let mut removed = Vec::new();
        for i in 0..=RUNS_KEPT {
            let at = 1_000 * (u64::try_from(i).unwrap() + 1);
            let effects = core.dispatch(AppAction::RestartSession(id), Clock::at(at));
            removed.extend(effects.iter().filter_map(|e| match e {
                Effect::RemoveLog(name) => Some(name.clone()),
                _ => None,
            }));
            core.dispatch(AppAction::Spawned { id, result: Ok(()) }, Clock::at(at + 1));
            core.dispatch(
                AppAction::HostListed(vec![exited(id, Some(0))]),
                Clock::at(at + 500),
            );
        }
        let runs = &core.session(id).unwrap().runs;
        assert_eq!(runs.len(), RUNS_KEPT);
        assert_eq!(runs.first().unwrap().n, 2);
        assert_eq!(removed, vec![format!("{}-r1.vt", id.host_name())]);
    }

    #[test]
    fn a_new_command_carries_its_output_patterns_and_they_can_be_changed() {
        let (mut core, pid, _) = with_records(&[], |_| None);
        core.dispatch(
            AppAction::NewSession {
                project: pid,
                name: "report".into(),
                kind: SessionKind::Command,
                cwd: "/tmp/proj".into(),
                launch: Launch::Command {
                    command: "make report".into(),
                    shell: "/bin/zsh".into(),
                },
                outputs: vec!["out/*.pdf".into()],
            },
            Clock::at(1),
        );
        let id = core.workspace(pid).unwrap().sessions[0].id;
        assert_eq!(
            core.session(id).unwrap().outputs,
            vec!["out/*.pdf".to_owned()]
        );
        let effects = core.dispatch(AppAction::SetOutputs(id, vec!["a.md".into()]), Clock::at(2));
        assert_eq!(saves(&effects), 1);
        assert_eq!(core.session(id).unwrap().outputs, vec!["a.md".to_owned()]);
    }
}

/// A set of an agent (left) and a shell (right), shown.
fn controller_set(
    core: &mut AppCore,
    agent_id: RecordId,
    shell_id: RecordId,
) -> super::model::SetId {
    core.dispatch(
        AppAction::NewWorkingSet {
            name: None,
            clone_of: None,
            with: Some(PinTarget::Session(agent_id)),
            columns: 24,
        },
        Clock::at(1),
    );
    let set = core.working_sets()[0].id;
    core.dispatch(
        AppAction::AddToWorkingSet {
            set,
            target: PinTarget::Session(shell_id),
            columns: 24,
        },
        Clock::at(1),
    );
    core.dispatch(AppAction::ShowWorkingSet(set), Clock::at(1));
    set
}

fn press(core: &mut AppCore, button: Button, down: bool, at: u64) {
    core.dispatch(
        AppAction::Controller(ControllerEvent::Button { button, down }),
        Clock::at(at),
    );
}

fn flick(core: &mut AppCore, direction: Direction, at: u64) {
    core.dispatch(
        AppAction::Controller(ControllerEvent::Flick(direction)),
        Clock::at(at),
    );
}

#[test]
fn the_stick_moves_the_selection_and_so_does_the_keyboard() {
    let (mut core, _, ids) = with_records(&[agent(), SessionKind::Shell], |s| Some(running(s.id)));
    let (agent_id, shell_id) = (ids[0], ids[1]);
    let set = controller_set(&mut core, agent_id, shell_id);
    let (left, right) = (PinTarget::Session(agent_id), PinTarget::Session(shell_id));
    assert_eq!(
        core.active_card(set),
        Some(left.clone()),
        "with nothing chosen the top-left card is selected"
    );
    flick(&mut core, Direction::Right, 4);
    assert_eq!(core.active_card(set), Some(right.clone()));
    flick(&mut core, Direction::Right, 5);
    assert_eq!(
        core.active_card(set),
        Some(right.clone()),
        "no wrap at the edge"
    );
    flick(&mut core, Direction::Left, 6);
    assert_eq!(core.active_card(set), Some(left.clone()));
    core.dispatch(AppAction::StepCard(Direction::Right), Clock::at(7));
    assert_eq!(core.active_card(set), Some(right.clone()));
    core.dispatch(AppAction::StepCard(Direction::Left), Clock::at(7));
    assert_eq!(core.active_card(set), Some(left.clone()));
    // A click picks a card too, and a card that leaves the set gives
    // the selection back to the top-left one.
    core.dispatch(
        AppAction::ActivateCard {
            set,
            target: right.clone(),
        },
        Clock::at(8),
    );
    assert_eq!(core.active_card(set), Some(right.clone()));
    core.dispatch(AppAction::RemoveSession(shell_id), Clock::at(9));
    assert_eq!(core.active_card(set), Some(left));
}

#[test]
fn c_holds_the_selected_agent_open_for_dictation() {
    let (mut core, _, ids) = with_records(&[agent(), SessionKind::Shell], |s| Some(running(s.id)));
    let (agent_id, shell_id) = (ids[0], ids[1]);
    let set = controller_set(&mut core, agent_id, shell_id);
    assert_eq!(core.hold_listen(), None);
    press(&mut core, Button::C, true, 2);
    assert_eq!(core.hold_listen(), Some(agent_id));
    // Moving the selection while C is down does not move the listener.
    flick(&mut core, Direction::Right, 4);
    assert_eq!(core.hold_listen(), Some(agent_id));
    press(&mut core, Button::C, false, 5);
    assert_eq!(core.hold_listen(), None);
    // The shell is selected now: nothing to dictate into, and a notice says so.
    assert_eq!(core.active_card(set), Some(PinTarget::Session(shell_id)));
    press(&mut core, Button::C, true, 7);
    assert_eq!(core.hold_listen(), None);
    assert_eq!(
        core.notice().map(|n| n.text.clone()),
        Some("No agent selected to listen into".to_owned())
    );
    press(&mut core, Button::C, false, 8);
    // On a session's own page C holds that session.
    core.dispatch(AppAction::ShowSession(agent_id), Clock::at(9));
    press(&mut core, Button::C, true, 10);
    assert_eq!(core.hold_listen(), Some(agent_id));
    // Losing the device lets go of everything.
    core.dispatch(
        AppAction::Controller(ControllerEvent::Connected(false)),
        Clock::at(11),
    );
    assert_eq!(core.hold_listen(), None);
    assert!(!core.controller_connected());
}

#[test]
fn controller_lines_parse_to_events_and_the_rest_are_ignored() {
    assert_eq!(
        ControllerEvent::parse("Z1\n"),
        Some(ControllerEvent::Button {
            button: Button::Z,
            down: true
        })
    );
    assert_eq!(
        ControllerEvent::parse("SL"),
        Some(ControllerEvent::Flick(Direction::Left))
    );
    assert_eq!(
        ControllerEvent::parse("S0"),
        Some(ControllerEvent::StickCentred)
    );
    assert_eq!(ControllerEvent::parse("P"), None);
    assert_eq!(ControllerEvent::parse("# nunchuk error"), None);
}

#[test]
fn z_holds_a_radial_menu_on_the_selected_card_and_letting_go_picks_the_slice() {
    let (mut core, _, ids) = with_records(&[agent(), SessionKind::Shell], |s| Some(running(s.id)));
    let (agent_id, shell_id) = (ids[0], ids[1]);
    let set = controller_set(&mut core, agent_id, shell_id);
    assert!(core.radial_menu().is_none());
    press(&mut core, Button::Z, true, 2);
    let menu = core.radial_menu().expect("the menu is open");
    assert_eq!((menu.target, menu.highlighted), (agent_id, None));
    // The stick points at slices instead of moving the selection.
    flick(&mut core, Direction::Right, 3);
    assert_eq!(
        core.radial_menu().unwrap().highlighted,
        Some(Direction::Right)
    );
    assert_eq!(core.active_card(set), Some(PinTarget::Session(agent_id)));
    core.dispatch(
        AppAction::Controller(ControllerEvent::StickCentred),
        Clock::at(4),
    );
    assert_eq!(core.radial_menu().unwrap().highlighted, None);
    // Letting go on nothing does nothing.
    press(&mut core, Button::Z, false, 5);
    assert!(core.radial_menu().is_none());
    assert_eq!(core.view(), View::WorkingSet(set));
    // Up is View and Left is Terminal: requests for the UI.
    press(&mut core, Button::Z, true, 6);
    flick(&mut core, Direction::Up, 7);
    press(&mut core, Button::Z, false, 8);
    assert_eq!(
        core.take_ui_requests(),
        vec![UiRequest::ViewAnswer(agent_id)]
    );
    press(&mut core, Button::Z, true, 9);
    flick(&mut core, Direction::Left, 10);
    press(&mut core, Button::Z, false, 11);
    assert_eq!(core.take_ui_requests(), vec![UiRequest::Terminal(agent_id)]);
    assert!(core.take_ui_requests().is_empty(), "taken once");
    // Down is Stop: Escape to the pane.
    press(&mut core, Button::Z, true, 12);
    flick(&mut core, Direction::Down, 13);
    let e = core.dispatch(
        AppAction::Controller(ControllerEvent::Button {
            button: Button::Z,
            down: false,
        }),
        Clock::at(14),
    );
    assert!(
        e.iter()
            .any(|e| matches!(e, Effect::SendKeys { bytes, .. } if bytes == &[0x1b]))
    );
    // Right is Open.
    press(&mut core, Button::Z, true, 15);
    flick(&mut core, Direction::Right, 16);
    press(&mut core, Button::Z, false, 17);
    assert_eq!(core.view(), View::Session(agent_id));
    // Off a working set Z opens nothing.
    press(&mut core, Button::Z, true, 18);
    assert!(core.radial_menu().is_none());
    press(&mut core, Button::Z, false, 19);
}

#[test]
fn a_double_press_of_c_latches_listening_and_the_next_press_stops_it() {
    let (mut core, _, ids) = with_records(&[agent(), SessionKind::Shell], |s| Some(running(s.id)));
    let (agent_id, shell_id) = (ids[0], ids[1]);
    controller_set(&mut core, agent_id, shell_id);
    // Press, release: a hold. Press again inside the window: latched.
    press(&mut core, Button::C, true, 1_000);
    assert_eq!(
        (core.hold_listen(), core.listen_presses()),
        (Some(agent_id), 1)
    );
    press(&mut core, Button::C, false, 1_100);
    assert_eq!(core.hold_listen(), None);
    press(&mut core, Button::C, true, 1_300);
    assert_eq!(
        (core.hold_listen(), core.listen_presses()),
        (Some(agent_id), 2)
    );
    press(&mut core, Button::C, false, 1_400);
    assert_eq!(core.hold_listen(), Some(agent_id), "latched on");
    assert_eq!(
        core.notice().map(|n| n.text.clone()),
        Some("Listening stays on; press C to stop".to_owned())
    );
    // A single press later turns it off: down is a fresh press, up lets go.
    press(&mut core, Button::C, true, 5_000);
    assert_eq!(
        (core.hold_listen(), core.listen_presses()),
        (Some(agent_id), 3)
    );
    press(&mut core, Button::C, false, 5_100);
    assert_eq!(core.hold_listen(), None);
    // The device repeats the button states once a second: a repeated
    // release is not a release. Latch again and let the heartbeat come.
    press(&mut core, Button::C, true, 6_000);
    press(&mut core, Button::C, false, 6_100);
    press(&mut core, Button::C, true, 6_300);
    press(&mut core, Button::C, false, 6_400);
    assert_eq!(core.hold_listen(), Some(agent_id));
    press(&mut core, Button::C, false, 7_400);
    press(&mut core, Button::Z, false, 7_400);
    assert_eq!(
        core.hold_listen(),
        Some(agent_id),
        "the heartbeat changed nothing"
    );
    press(&mut core, Button::C, true, 8_000);
    press(&mut core, Button::C, false, 8_100);
    assert_eq!(core.hold_listen(), None);
    // Two presses far apart are two holds.
    press(&mut core, Button::C, true, 9_000);
    press(&mut core, Button::C, false, 9_100);
    press(&mut core, Button::C, true, 9_600);
    press(&mut core, Button::C, false, 9_700);
    assert_eq!(core.hold_listen(), None);
}

#[test]
fn c_on_a_file_card_holds_it_for_the_stick_to_scroll() {
    let (mut core, pid, ids) = with_records(&[agent()], |s| Some(running(s.id)));
    let agent_id = ids[0];
    let file = PinTarget::File(pid, "README.md".into());
    core.dispatch(
        AppAction::NewWorkingSet {
            name: None,
            clone_of: None,
            with: Some(file.clone()),
            columns: 24,
        },
        Clock::at(1),
    );
    let set = core.working_sets()[0].id;
    core.dispatch(
        AppAction::AddToWorkingSet {
            set,
            target: PinTarget::Session(agent_id),
            columns: 24,
        },
        Clock::at(1),
    );
    core.dispatch(AppAction::ShowWorkingSet(set), Clock::at(1));
    assert_eq!(core.active_card(set), Some(file.clone()));
    assert_eq!(core.scroll_hold(), None);
    press(&mut core, Button::C, true, 2);
    assert_eq!(core.scroll_hold(), Some((&file, None)));
    assert_eq!(core.hold_listen(), None, "a file is not listened into");
    // The stick scrolls instead of moving the selection.
    flick(&mut core, Direction::Right, 3);
    assert_eq!(core.scroll_hold(), Some((&file, Some(Direction::Right))));
    assert_eq!(core.active_card(set), Some(file.clone()));
    core.dispatch(
        AppAction::Controller(ControllerEvent::StickCentred),
        Clock::at(4),
    );
    assert_eq!(core.scroll_hold(), Some((&file, None)));
    press(&mut core, Button::C, false, 5);
    assert_eq!(core.scroll_hold(), None);
    flick(&mut core, Direction::Right, 6);
    assert_eq!(core.active_card(set), Some(PinTarget::Session(agent_id)));
}
