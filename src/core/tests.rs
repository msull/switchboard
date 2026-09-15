//! State-transition tests for the core. Every test dispatches actions at
//! an explicit `Clock` and asserts on state and returned effects.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use uuid::Uuid;

use super::action::{AppAction, AppCore, Clock, Effect, View};
use super::definitions::entry_hash;
use super::model::{
    Activity, AgentKind, Approval, CardState, GridRect, Launch, PinTarget, Project, ProjectEnv,
    ProjectId, RecordId, ResumeHandle, SavedView, SessionKind, SessionRecord, Settings, SideTab,
    ThemeMode, Views, Workspace,
};
use super::reconcile::RECORD_ID_ENV;
use crate::ports::agent::AgentLaunch;
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
        created: t,
        last_active: t,
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
    assert!(matches!(effects[0], Effect::Kill(ref h) if h.0 == id.host_name()));
    assert!(matches!(effects[1], Effect::Spawn { id: sid, .. } if sid == id));
    assert_eq!(effects.len(), 2);
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
    let effects = core.dispatch(AppAction::RemoveSession(id), Clock::at(1));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::Forget(h) if h.0 == id.host_name()))
    );
    assert!(!effects.iter().any(|e| matches!(e, Effect::Kill(_))));
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
fn exclusive_mode_hides_all_but_the_active_project() {
    let mut core = AppCore::new();
    let (a, b) = (Workspace::new(project("a")), Workspace::new(project("b")));
    let (ida, idb) = (a.project.id, b.project.id);
    core.seed(vec![a, b], Vec::new());
    core.dispatch(AppAction::ShowBoard(idb), Clock::at(5));
    assert!(core.project_visible(ida) && core.project_visible(idb));
    core.dispatch(AppAction::SetExclusive(true), Clock::at(6));
    assert_eq!(core.active_project(), Some(idb));
    assert!(!core.project_visible(ida) && core.project_visible(idb));
    assert_eq!(core.visible_workspaces().count(), 1);
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
fn spawned_agent_attaches_and_updates_last_seen() {
    let (mut core, pid, _) = with_records(&[], |_| None);
    let (id, _) = new_session(&mut core, pid, agent(), Launch::Argv(vec![]));
    let effects = launch_agent(&mut core, id, Some(claude_handle()));
    assert_eq!(saves(&effects), 1);
    assert!(effects.contains(&Effect::Attach {
        id,
        host: HostId(id.host_name()),
        title: id.host_name(),
        cwd: "/tmp/proj".into(),
    }));
    assert!(!effects.iter().any(|e| matches!(e, Effect::Discover { .. })));
    assert_eq!(core.session(id).unwrap().last_seen, Clock::at(20).wall);
    assert!(!core.is_in_flight(id));
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
    assert!(effects.iter().any(|e| matches!(e, Effect::Attach { .. })));
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
    assert!(e4.iter().any(|e| matches!(e, Effect::Attach { .. })));
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
    let Effect::Spawn { spec, .. } = &e[0] else {
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
    assert_eq!(e.len(), 2);
    assert!(matches!(e[0], Effect::Save(_)));
    assert!(matches!(e[1], Effect::SaveSettings(_)), "the view moved");
    assert!(core.session(ids[0]).is_none());
    assert!(core.workspace(pid).unwrap().sessions.is_empty());
    assert_ne!(core.view(), View::Session(ids[0]));
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
    }
}

/// The reader's success shape for a file with these entries.
#[allow(clippy::unnecessary_wraps)]
fn config(entries: Vec<DefinedEntry>) -> Result<Option<ProjectConfig>, String> {
    Ok(Some(ProjectConfig {
        entries,
        warnings: vec![],
        shell: "/bin/zsh".into(),
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
            w: 7,
            h: 5
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
