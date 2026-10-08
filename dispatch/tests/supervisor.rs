//! A project's supervisor session against a Switchboard in memory: made,
//! replaced, killed, resumed and recovered through its own ledger; and,
//! through the command line with `SWITCHBOARD_RECORD_ID` set as its pane
//! would set it, what a supervisor may and may not do.

// Tests assert emptiness with `assert!` throughout.
#![allow(clippy::assert_is_empty)]

// The in-memory Switchboard is shared with `first_slice`, which uses the
// parts this file does not.
#[allow(dead_code)]
mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

use dispatch::events::{For, Kind, Waited};
use dispatch::git::FakeRepo;
use dispatch::scheduler::{BY_HAND, BY_SUPERVISOR, Runner};
use dispatch::store::DataDir;
use dispatch::supervisor::{Actor, permit};
use dispatch::ticket::{
    Decision, DecisionKind, DecisionState, SourceSnapshot, SupervisorRecord, Ticket, TicketState,
};
use support::{FakeSwitchboard, SharedPort};
use switchboard_control::{Body, Liveness};

const ORCHARD: &str = "Orchard";
const GROVE: &str = "Grove";
/// The record id of Orchard's supervisor session in the CLI tests.
const SUPERVISOR: &str = "sup-orchard";

fn pipeline(project: &str, decides: &[&str]) -> String {
    let decides: Vec<String> = decides.iter().map(|d| format!("{d:?}")).collect();
    format!(
        r#"
version = 1

[project]
name = "{project}"
repo = "git@example.com:o/{project}.git"
space = "Dispatch · {project}"

[source]
kind = "github"
repo = "o/{project}"
label = "dispatch"

[[lanes]]
name = "repo"
path = "."

[[stages]]
name = "inspect"
gate = {{ kind = "human", decision = "inspect" }}

[supervisor]
guidance = "Keep {project}'s queue moving."
read = ["CLAUDE.md"]
decides = [{}]
"#,
        decides.join(", ")
    )
}

struct Env {
    dir: tempfile::TempDir,
    data: DataDir,
    sb: Arc<Mutex<FakeSwitchboard>>,
    repo: Arc<Mutex<FakeRepo>>,
    runner: Runner,
    now: u64,
}

impl Env {
    fn new() -> Self {
        dispatch::store::skip_fsync_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path().join("dispatch"));
        std::fs::create_dir_all(data.root.join("pipelines")).unwrap();
        data.write_settings(&dispatch::store::Settings {
            worktrees: Some(dir.path().join("wt")),
        })
        .unwrap();
        std::fs::write(data.pipeline(ORCHARD), pipeline(ORCHARD, &["finalize"])).unwrap();
        std::fs::write(data.pipeline(GROVE), pipeline(GROVE, &[])).unwrap();
        let sb = Arc::new(Mutex::new(FakeSwitchboard::new()));
        let repo = Arc::new(Mutex::new(FakeRepo::default()));
        let runner = Runner::new(
            data.clone(),
            Box::new(SharedPort(Arc::clone(&sb))),
            Box::new(Arc::clone(&repo)),
        );
        Self {
            dir,
            data,
            sb,
            repo,
            runner,
            now: 1_000,
        }
    }

    fn tick(&mut self) -> u64 {
        self.now += 1_000;
        self.now
    }

    fn workspace(&self) -> PathBuf {
        self.dir
            .path()
            .join("wt")
            .join(format!("supervisor-{ORCHARD}"))
    }

    fn handoff(&self) -> PathBuf {
        self.data.supervisor_dir(ORCHARD).join("handoff.md")
    }

    fn fresh(&mut self) -> anyhow::Result<SupervisorRecord> {
        let now = self.tick();
        self.runner.supervisor_fresh(ORCHARD, false, "fresh", now)
    }

    fn take(&mut self, project: &str, number: u64) -> Ticket {
        let now = self.tick();
        let text = std::fs::read_to_string(self.data.pipeline(project)).unwrap();
        self.runner
            .take(project, &text, source(project, number, now), now)
            .unwrap()
    }

    /// A pending decision of `name` on the ticket; its id.
    fn ask(&mut self, ticket: &str, name: &str) -> String {
        let now = self.tick();
        let mut t = self.runner.load_ticket(ticket).unwrap();
        let id = format!("d{}", t.decisions.len() + 1);
        t.decisions.push(Decision {
            id: id.clone(),
            stage: "inspect".into(),
            name: name.into(),
            kind: DecisionKind::Permission,
            question: "Go on?".into(),
            options: vec![name.into(), "park".into()],
            recommendation: None,
            attempt: None,
            state: DecisionState::Pending,
            made_ms: now,
            refusals: Vec::new(),
        });
        self.runner.save_ticket(&mut t, now).unwrap();
        id
    }

    fn parked(&mut self, ticket: &str) {
        let now = self.tick();
        let mut t = self.runner.load_ticket(ticket).unwrap();
        t.state = TicketState::Parked {
            reason: "parked by hand".into(),
        };
        self.runner.save_ticket(&mut t, now).unwrap();
    }

    /// `session` recorded as the project's current supervisor.
    fn seat(&self, project: &str, session: &str) {
        let mut ps = self.runner.load_project(project).unwrap();
        ps.supervisor.current = Some(SupervisorRecord {
            session: session.into(),
            seed_hash: "seed".into(),
            created_ms: 1,
            model: None,
        });
        self.runner.save_project(&ps).unwrap();
    }

    /// The command line, as the owner (`None`) or as a Switchboard
    /// session.
    fn cli(&self, record: Option<&str>, args: &[&str]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_dispatch"));
        cmd.args(args)
            .env("DISPATCH_DATA_DIR", &self.data.root)
            .env("SWITCHBOARD_DATA_DIR", self.dir.path().join("sb"))
            .env_remove("SWITCHBOARD_RECORD_ID");
        if let Some(id) = record {
            cmd.env("SWITCHBOARD_RECORD_ID", id);
        }
        cmd.output().unwrap()
    }

    fn ticket(&self, id: &str) -> Ticket {
        self.runner.load_ticket(id).unwrap()
    }
}

fn source(project: &str, number: u64, now: u64) -> SourceSnapshot {
    SourceSnapshot {
        kind: "github".into(),
        identity: format!("o/{project}#{number}"),
        number: Some(number),
        title: format!("Issue {number}"),
        body: String::new(),
        url: None,
        labels: vec!["dispatch".into()],
        taken_at_ms: now,
        taken_by: None,
        pull_requests: Vec::new(),
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Exit 1 with `want` in the message.
fn refused(out: &Output, want: &str) {
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(
        stderr(out).contains(want),
        "{:?} lacks {want:?}",
        stderr(out)
    );
}

fn accepted(out: &Output) {
    assert_eq!(out.status.code(), Some(0), "{out:?}");
}

// --- making and replacing the session

#[test]
fn a_fresh_supervisor_is_set_up_seeded_and_recorded_before_it_is_sent() {
    let mut env = Env::new();
    let project_file = env.data.project_file(ORCHARD);
    env.sb().snapshot_on = Some(("session.new".into(), project_file));
    let current = env.fresh().unwrap();
    let ps = env.runner.load_project(ORCHARD).unwrap();
    assert_eq!(ps.supervisor.current.as_ref(), Some(&current));
    assert_eq!(ps.supervisor.workspace.as_deref(), Some(&*env.workspace()));
    assert_eq!(ps.supervisor.error, None);
    assert!(env.workspace().is_dir());
    // No setup in the table and a repo: the default clone, in the
    // workspace.
    let ran = env.repo.lock().unwrap().ran.clone();
    assert_eq!(
        ran,
        [(
            env.workspace(),
            vec![
                "git".to_owned(),
                "clone".into(),
                "git@example.com:o/Orchard.git".into(),
                ".".into()
            ]
        )]
    );
    // The request was on the record before Switchboard saw it.
    let snapshot = env.sb().snapshots[0].clone();
    let before: dispatch::ticket::ProjectState = serde_json::from_str(&snapshot).unwrap();
    let op = before.supervisor.op.unwrap();
    assert_eq!(op.kind, "session.new");
    assert!(op.op.starts_with("sup-Orchard-"), "{}", op.op);
    assert_eq!(op.reply, None);
    // The session: a Claude Code agent in the workspace, with the
    // seed's path as its first prompt and the allow rules as flags.
    let sb = env.sb();
    let s = sb.session(&current.session);
    assert_eq!(s.name, "Supervisor · Orchard");
    assert_eq!(s.cwd, env.workspace());
    let seed = env.data.supervisor_dir(ORCHARD).join("seed.md");
    let text = std::fs::read_to_string(&seed).unwrap();
    assert!(text.contains("Keep Orchard's queue moving."), "{text}");
    assert!(text.contains(&env.workspace().join("CLAUDE.md").display().to_string()));
    let new = sb
        .calls
        .iter()
        .find_map(|c| match &c.body {
            Body::SessionNew { prompt, launch, .. } => Some((prompt.clone(), launch.clone())),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        new.0,
        Some(format!("Read {} and do what it says.", seed.display()))
    );
    let switchboard_control::Launch::Argv(flags) = new.1 else {
        panic!("{:?}", new.1);
    };
    assert!(
        flags
            .iter()
            .any(|f| f.starts_with("Bash(") && f.ends_with(":*)"))
    );
    assert!(!flags.iter().any(|f| f == "--settings"));
}

#[test]
fn a_second_fresh_replaces_the_first_and_rotates_the_handoff() {
    let mut env = Env::new();
    let first = env.fresh().unwrap();
    std::fs::write(env.handoff(), "watching #12\n").unwrap();
    let second = env.fresh().unwrap();
    assert_ne!(first.session, second.session);
    let ps = env.runner.load_project(ORCHARD).unwrap();
    assert_eq!(ps.supervisor.past.len(), 1);
    assert_eq!(ps.supervisor.past[0].session, first.session);
    assert_eq!(ps.supervisor.past[0].why, "fresh");
    assert!(env.sb().killed.contains(&first.session));
    // The Switchboard project is made once and reused.
    assert_eq!(env.sb().kinds_called("project.add"), 1);
    let handoff = std::fs::read_to_string(env.handoff()).unwrap();
    assert!(handoff.starts_with("## From the session of "), "{handoff}");
    assert!(handoff.ends_with("watching #12\n"));
    let kept: Vec<_> = std::fs::read_dir(env.data.supervisor_dir(ORCHARD))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("handoff.") && n != "handoff.md")
        .collect();
    assert_eq!(kept.len(), 1, "{kept:?}");
    let old = std::fs::read_to_string(env.data.supervisor_dir(ORCHARD).join(&kept[0])).unwrap();
    assert_eq!(old, "watching #12\n");
}

#[test]
fn a_failed_setup_makes_no_session_and_removes_the_workspace_it_made() {
    let mut env = Env::new();
    env.repo.lock().unwrap().fail_run = Some("remote not found".into());
    let e = env.fresh().unwrap_err();
    let text = format!("{e:#}");
    assert!(
        text.contains("git clone git@example.com:o/Orchard.git ."),
        "{text}"
    );
    assert!(text.contains("remote not found"), "{text}");
    assert!(!env.workspace().exists());
    assert_eq!(env.sb().kinds_called("session.new"), 0);
    let ps = env.runner.load_project(ORCHARD).unwrap();
    assert_eq!(ps.supervisor.current, None);
    assert!(ps.supervisor.error.unwrap().contains("remote not found"));
}

#[test]
fn a_failed_setup_leaves_the_current_supervisor_running() {
    let mut env = Env::new();
    let first = env.fresh().unwrap();
    env.repo.lock().unwrap().fail_run = Some("no network".into());
    let now = env.tick();
    env.runner
        .supervisor_fresh(ORCHARD, true, "fresh", now)
        .unwrap_err();
    let ps = env.runner.load_project(ORCHARD).unwrap();
    assert_eq!(ps.supervisor.current, Some(first.clone()));
    assert!(!env.sb().killed.contains(&first.session));
    // A workspace this call did not make is kept.
    assert!(env.workspace().is_dir());
}

#[test]
fn a_lost_session_reply_is_found_again_at_recovery() {
    let mut env = Env::new();
    env.sb().drop_reply_for = Some("session.new".into());
    env.fresh().unwrap_err();
    let ps = env.runner.load_project(ORCHARD).unwrap();
    assert_eq!(ps.supervisor.current, None);
    assert!(ps.supervisor.op.as_ref().unwrap().unresolved());
    let now = env.tick();
    env.runner.recover(now).unwrap();
    let ps = env.runner.load_project(ORCHARD).unwrap();
    let current = ps.supervisor.current.expect("found by its op");
    assert_eq!(env.sb().sessions_named("Supervisor · Orchard").len(), 1);
    assert_eq!(
        env.sb().sessions_named("Supervisor · Orchard")[0].id,
        current.session
    );
    assert_eq!(ps.supervisor.error, None);
    assert!(!ps.supervisor.op.unwrap().unresolved());
}

#[test]
fn a_kill_keeps_the_reason_and_a_resume_needs_the_workspace() {
    let mut env = Env::new();
    let first = env.fresh().unwrap();
    // A running pane: the resume is Switchboard's to leave alone.
    assert_eq!(
        env.runner.supervisor_resume(ORCHARD).unwrap(),
        first.session
    );
    std::fs::remove_dir_all(env.workspace()).unwrap();
    let e = env.runner.supervisor_resume(ORCHARD).unwrap_err();
    assert!(format!("{e:#}").contains("--fresh"), "{e:#}");
    let now = env.tick();
    env.runner
        .supervisor_kill(ORCHARD, "the owner is away", now)
        .unwrap();
    let ps = env.runner.load_project(ORCHARD).unwrap();
    assert_eq!(ps.supervisor.current, None);
    assert_eq!(ps.supervisor.past[0].why, "kill: the owner is away");
    assert!(env.sb().killed.contains(&first.session));
    let e = env.runner.supervisor_resume(ORCHARD).unwrap_err();
    assert!(format!("{e:#}").contains("no supervisor"), "{e:#}");
}

#[test]
fn a_resume_reaches_switchboard_for_an_exited_supervisor() {
    let mut env = Env::new();
    let first = env.fresh().unwrap();
    env.sb().session_mut(&first.session).liveness = Liveness::Exited { code: Some(0) };
    env.runner.supervisor_resume(ORCHARD).unwrap();
    assert_eq!(env.sb().kinds_called("session.resume"), 1);
    assert_eq!(env.sb().session(&first.session).liveness, Liveness::Running);
}

#[test]
fn an_intent_from_the_port_is_carried_out_by_the_next_pass() {
    let mut env = Env::new();
    let mut handler = dispatch::serve::Handler {
        runner: Runner::new(
            env.data.clone(),
            Box::new(SharedPort(Arc::clone(&env.sb))),
            Box::new(Arc::clone(&env.repo)),
        ),
        issues: Box::new(dispatch::github::FakeIssues::default()),
    };
    let reply = handler.handle(
        &dispatch_control::Request::new(
            "a",
            dispatch_control::Body::SupervisorFresh {
                project: ORCHARD.into(),
            },
        ),
        env.now,
    );
    let dispatch_control::Reply::Status(status) = reply else {
        panic!("{reply:?}");
    };
    let view = status
        .projects
        .iter()
        .find(|p| p.name == ORCHARD)
        .and_then(|p| p.supervisor.clone())
        .unwrap();
    assert!(view.fresh_pending);
    assert_eq!(view.session, None);
    // Nothing is made while the caller waits on the reply.
    assert_eq!(env.sb().kinds_called("session.new"), 0);
    let now = env.tick();
    env.runner.step_all(now).unwrap();
    let ps = env.runner.load_project(ORCHARD).unwrap();
    assert!(ps.supervisor.current.is_some());
    assert_eq!(ps.supervisor.intent, None);
    // A project without the table is refused at the port.
    std::fs::write(
        env.data.pipeline(GROVE),
        pipeline(GROVE, &[]).split("[supervisor]").next().unwrap(),
    )
    .unwrap();
    let reply = handler.handle(
        &dispatch_control::Request::new(
            "b",
            dispatch_control::Body::SupervisorFresh {
                project: GROVE.into(),
            },
        ),
        env.now,
    );
    assert!(
        matches!(&reply, dispatch_control::Reply::Failed { reason } if reason.contains("no [supervisor] table")),
        "{reply:?}"
    );
}

#[test]
fn a_fresh_from_the_command_line_settles_a_pending_intent() {
    let mut env = Env::new();
    env.runner.request_supervisor_fresh(ORCHARD).unwrap();
    let made = env.fresh().unwrap();
    let ps = env.runner.load_project(ORCHARD).unwrap();
    assert_eq!(ps.supervisor.intent, None);
    let now = env.tick();
    env.runner.step_all(now).unwrap();
    let ps = env.runner.load_project(ORCHARD).unwrap();
    assert_eq!(ps.supervisor.current, Some(made));
    assert_eq!(ps.supervisor.past.len(), 0);
    assert_eq!(env.sb().kinds_called("session.new"), 1);
}

#[test]
fn the_seed_goes_stale_when_the_table_changes() {
    let mut env = Env::new();
    env.fresh().unwrap();
    assert!(!env.runner.supervisor_standing(ORCHARD).unwrap().stale);
    std::fs::write(
        env.data.pipeline(ORCHARD),
        pipeline(ORCHARD, &["finalize", "rerun"]),
    )
    .unwrap();
    assert!(env.runner.supervisor_standing(ORCHARD).unwrap().stale);
    let status = dispatch::serve::status(&env.runner).unwrap();
    let view = status
        .projects
        .iter()
        .find(|p| p.name == ORCHARD)
        .and_then(|p| p.supervisor.clone())
        .unwrap();
    assert!(view.seed_stale);
}

// --- brief

#[test]
fn brief_shows_the_tickets_what_waits_the_events_and_the_handoff() {
    let mut env = Env::new();
    let t = env.take(ORCHARD, 12);
    let d = env.ask(&t.id, "inspect");
    let other = env.take(GROVE, 3);
    std::fs::create_dir_all(env.data.supervisor_dir(ORCHARD)).unwrap();
    std::fs::write(env.handoff(), "watching #12\n").unwrap();
    let out = env.cli(None, &["brief", ORCHARD]);
    accepted(&out);
    let text = stdout(&out);
    for want in [
        "supervisor: none; `dispatch supervisor Orchard --fresh` starts one",
        &format!("{} #12 Issue 12 · stage inspect · active", t.id),
        &format!(
            "dispatch decide {} {d} <answer> [--note <text> | --file <path>]",
            t.id
        ),
        "taken",
        "follow from seq ",
        "watching #12",
    ] {
        assert!(text.contains(want), "brief lacks {want:?}:\n{text}");
    }
    assert!(
        !text.contains(&other.id),
        "another project's ticket:\n{text}"
    );
    std::fs::remove_file(env.handoff()).unwrap();
    assert!(stdout(&env.cli(None, &["brief", ORCHARD])).contains("(no hand-off yet)"));
}

#[test]
fn a_supervisors_brief_and_show_print_decide_with_the_full_path() {
    let mut env = Env::new();
    env.seat(ORCHARD, SUPERVISOR);
    let t = env.take(ORCHARD, 12);
    let d = env.ask(&t.id, "inspect");
    let line = format!("/dispatch decide {} {d} <answer>", t.id);
    for args in [["brief", ORCHARD], ["show", &t.id]] {
        let text = stdout(&env.cli(Some(SUPERVISOR), &args));
        assert!(text.contains(&line), "{args:?} lacks {line:?}:\n{text}");
        assert!(!text.contains("    dispatch decide"), "{text}");
        let owners = stdout(&env.cli(None, &args));
        assert!(
            owners.contains(&format!("    dispatch decide {}", t.id)),
            "{owners}"
        );
    }
}

#[test]
fn events_with_a_timeout_return_as_soon_as_they_print() {
    let mut env = Env::new();
    env.take(ORCHARD, 1);
    let started = std::time::Instant::now();
    let out = env.cli(
        None,
        &["events", "--since", "0", "--follow", "--timeout", "60"],
    );
    accepted(&out);
    assert!(stdout(&out).contains("taken"), "{out:?}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(30),
        "it waited out the timeout"
    );
}

#[test]
fn events_with_a_timeout_and_nothing_new_exit_two() {
    let env = Env::new();
    let out = env.cli(None, &["events", "--follow", "--timeout", "1"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let out = env.cli(None, &["events", "--timeout", "1"]);
    assert_eq!(out.status.code(), Some(64), "a timeout without --follow");
}

// --- what a supervisor may do

#[test]
fn a_supervisor_answers_what_decides_lists_and_nothing_else() {
    let mut env = Env::new();
    env.seat(ORCHARD, SUPERVISOR);
    let t = env.take(ORCHARD, 1);
    let finalize = env.ask(&t.id, "finalize");
    accepted(&env.cli(Some(SUPERVISOR), &["decide", &t.id, &finalize, "finalize"]));
    let d = env.ticket(&t.id).decisions[0].clone();
    assert!(
        matches!(&d.state, DecisionState::Answered { by, .. } if by == BY_SUPERVISOR),
        "{d:?}"
    );
    let since = dispatch::events::last_seq(&dispatch::events::log_path(&env.data)).unwrap();
    let inspect = env.ask(&t.id, "inspect");
    let since_ask = dispatch::events::last_seq(&dispatch::events::log_path(&env.data)).unwrap();
    assert!(since_ask > since);
    let out = env.cli(Some(SUPERVISOR), &["decide", &t.id, &inspect, "inspect"]);
    refused(
        &out,
        "the supervisor may not answer `inspect`; the owner does",
    );
    refused(&out, dispatch::supervisor::OWNER_ROUTES);
    refused(
        &out,
        "a command typed in the supervisor's pane is the supervisor's",
    );
    let d = env.ticket(&t.id).decisions[1].clone();
    assert!(d.pending());
    assert_eq!(d.refusals.len(), 1);
    assert_eq!(d.refusals[0].by, BY_SUPERVISOR);
    assert_eq!(d.refusals[0].answer, "inspect");
    // The refusal is an event a waiter is handed.
    let waited = dispatch::events::wait(
        &env.data,
        &t.id,
        For::Any,
        Some(since_ask),
        Some(0),
        &mut || 1,
        &mut || {},
    )
    .unwrap();
    let Waited::Matched(e) = waited else {
        panic!("{waited:?}");
    };
    assert_eq!(e.kind, Kind::Refused);
    assert_eq!(e.actor.as_deref(), Some(BY_SUPERVISOR));
    assert_eq!(
        e.text,
        "inspect: the supervisor asked inspect; refused, the owner answers"
    );
    // `show` lists it.
    let shown = stdout(&env.cli(None, &["show", &t.id]));
    assert!(shown.contains("answered finalize by supervisor"), "{shown}");
    assert!(shown.contains("the supervisor asked inspect"), "{shown}");
    // With no id, or an id no supervisor had, it is the owner's.
    accepted(&env.cli(
        Some("someone-else"),
        &["decide", &t.id, &inspect, "inspect"],
    ));
    let d = env.ticket(&t.id).decisions[1].clone();
    assert!(
        matches!(&d.state, DecisionState::Answered { by, .. } if by == BY_HAND),
        "{d:?}"
    );
    let again = env.ask(&t.id, "inspect");
    accepted(&env.cli(None, &["decide", &t.id, &again, "inspect"]));
}

#[test]
fn a_past_supervisor_is_still_a_supervisor() {
    let mut env = Env::new();
    let mut ps = env.runner.load_project(ORCHARD).unwrap();
    ps.supervisor.past.push(dispatch::ticket::PastSupervisor {
        session: "old".into(),
        seed_hash: "x".into(),
        created_ms: 1,
        replaced_ms: 2,
        why: "fresh".into(),
    });
    env.runner.save_project(&ps).unwrap();
    assert_eq!(
        dispatch::supervisor::actor_of(&env.data, Some("old")).unwrap(),
        Actor::Supervisor(ORCHARD.into())
    );
    assert_eq!(
        dispatch::supervisor::actor_of(&env.data, Some("new")).unwrap(),
        Actor::Owner
    );
    assert_eq!(
        dispatch::supervisor::actor_of(&env.data, None).unwrap(),
        Actor::Owner
    );
    let t = env.take(ORCHARD, 1);
    refused(
        &env.cli(Some("old"), &["restart", &t.id]),
        &format!(
            "the supervisor may not run `dispatch restart {}` unless its table's `may` lists \
             `restart`",
            t.id
        ),
    );
}

#[test]
fn a_supervisor_stays_on_its_own_project() {
    let mut env = Env::new();
    env.seat(ORCHARD, SUPERVISOR);
    let theirs = env.take(GROVE, 1);
    let d = env.ask(&theirs.id, "inspect");
    let before = env.ticket(&theirs.id);
    refused(
        &env.cli(Some(SUPERVISOR), &["decide", &theirs.id, &d, "inspect"]),
        "the supervisor of Orchard may not act on Grove's tickets",
    );
    refused(
        &env.cli(Some(SUPERVISOR), &["park", &theirs.id]),
        "may not act on Grove's tickets",
    );
    refused(
        &env.cli(Some(SUPERVISOR), &["take", GROVE, "7"]),
        "the supervisor of Orchard may not act on Grove",
    );
    refused(
        &env.cli(Some(SUPERVISOR), &["take", GROVE, "pr", "repo/5"]),
        "may not act on Grove",
    );
    refused(
        &env.cli(Some(SUPERVISOR), &["queue", GROVE, &theirs.id]),
        "may not act on Grove",
    );
    assert_eq!(env.ticket(&theirs.id), before);
    assert_eq!(env.runner.tickets().unwrap().len(), 1);
    // Its own project's queue is its to read and order.
    let mine = env.take(ORCHARD, 2);
    let out = env.cli(Some(SUPERVISOR), &["queue", ORCHARD, &mine.id]);
    accepted(&out);
    assert!(stdout(&out).contains(&mine.id));
    let data = &env.data;
    let me = Actor::Supervisor(ORCHARD.into());
    permit(&me, &["take", ORCHARD, "7"], data).unwrap();
    permit(&me, &["take", ORCHARD, "pr", "repo/5"], data).unwrap();
}

#[test]
fn a_take_by_a_supervisor_is_stamped_and_logged() {
    let mut env = Env::new();
    env.runner.actor = Some(BY_SUPERVISOR.into());
    let t = env.take(ORCHARD, 4);
    assert_eq!(t.source.taken_by.as_deref(), Some(BY_SUPERVISOR));
    let events = dispatch::events::read_since(&dispatch::events::log_path(&env.data), 0).unwrap();
    let taken = events.iter().find(|e| e.kind == Kind::Taken).unwrap();
    assert_eq!(taken.actor.as_deref(), Some(BY_SUPERVISOR));
    assert!(taken.text.ends_with("(by supervisor)"), "{}", taken.text);
}

#[test]
fn a_park_by_the_supervisor_says_who_parked() {
    let mut env = Env::new();
    env.seat(ORCHARD, SUPERVISOR);
    let t = env.take(ORCHARD, 1);
    accepted(&env.cli(
        Some(SUPERVISOR),
        &["park", &t.id, "--reason", "waits on #2"],
    ));
    let parked = env.ticket(&t.id);
    assert_eq!(parked.state_by.as_deref(), Some(BY_SUPERVISOR));
    let events = dispatch::events::read_since(&dispatch::events::log_path(&env.data), 0).unwrap();
    let parking = events.iter().find(|e| e.kind == Kind::Parking).unwrap();
    assert_eq!(parking.text, "waits on #2 (by supervisor)");
    assert_eq!(parking.actor.as_deref(), Some(BY_SUPERVISOR));
}

#[test]
fn a_supervisor_resumes_with_reruns_only_when_rerun_is_its_to_answer() {
    let mut env = Env::new();
    env.seat(ORCHARD, SUPERVISOR);
    let t = env.take(ORCHARD, 1);
    env.parked(&t.id);
    refused(
        &env.cli(Some(SUPERVISOR), &["resume", &t.id]),
        "the supervisor may not resume with reruns; `--no-rerun`, or the owner does",
    );
    let still = env.ticket(&t.id);
    assert!(matches!(still.state, TicketState::Parked { .. }));
    assert!(still.decisions.is_empty(), "no rerun was answered");
    let out = env.cli(Some(SUPERVISOR), &["resume", &t.id, "--no-rerun"]);
    accepted(&out);
    let resumed = env.ticket(&t.id);
    assert!(resumed.active());
    assert_eq!(resumed.state_by.as_deref(), Some(BY_SUPERVISOR));
    // With `rerun` in `decides` a plain resume is its too.
    std::fs::write(
        env.data.pipeline(ORCHARD),
        pipeline(ORCHARD, &["finalize", "rerun"]),
    )
    .unwrap();
    env.parked(&t.id);
    accepted(&env.cli(Some(SUPERVISOR), &["resume", &t.id]));
    assert!(env.ticket(&t.id).active());
}

/// `pipeline(project, decides)` with a tester and a `tried`
/// confirmation that both hold one stack, in place of `inspect`.
fn tried_pipeline(project: &str, decides: &[&str]) -> String {
    pipeline(project, decides).replace(
        "[[stages]]\nname = \"inspect\"\ngate = { kind = \"human\", decision = \"inspect\" }",
        "[[resources]]\nname = \"stack\"\n\n\
         [operators.tester]\nkind = \"claude\"\n\n\
         [[stages]]\nname = \"try\"\noperator = \"tester\"\ncontext = \"joined\"\n\
         needs = [\"stack\"]\nwrites = [\"notes\"]\nprompt = \"Report to {notes}.\"\n\n\
         [[stages]]\nname = \"tried\"\nneeds = [\"stack\"]\n\
         gate = { kind = \"human\", decision = \"tried\", confirm = true }",
    )
}

#[test]
fn a_supervisor_answers_rerun_on_tried_only_when_decides_names_tried() {
    let mut env = Env::new();
    env.seat(ORCHARD, SUPERVISOR);
    let text = tried_pipeline(ORCHARD, &["tried"]);
    assert!(text.contains("name = \"tried\""), "{text}");
    std::fs::write(env.data.pipeline(ORCHARD), text).unwrap();
    let t = env.take(ORCHARD, 1);
    let ask = |env: &mut Env| {
        let now = env.tick();
        let mut t = env.runner.load_ticket(&t.id).unwrap();
        let id = format!("d{}", t.decisions.len() + 1);
        t.decisions.push(Decision {
            id: id.clone(),
            stage: "tried".into(),
            name: "tried".into(),
            kind: DecisionKind::Confirmation,
            question: "tried (joined): Tree: /t".into(),
            options: vec!["done".into(), "rerun".into(), "park".into()],
            recommendation: None,
            attempt: None,
            state: DecisionState::Pending,
            made_ms: now,
            refusals: Vec::new(),
        });
        env.runner.save_ticket(&mut t, now).unwrap();
        id
    };
    let d = ask(&mut env);
    accepted(&env.cli(
        Some(SUPERVISOR),
        &[
            "decide",
            &t.id,
            &d,
            "rerun",
            "--note",
            "nothing could be tested",
        ],
    ));
    let answered = env.ticket(&t.id).decisions[0].clone();
    assert!(
        matches!(&answered.state, DecisionState::Answered { answer, by, .. }
            if answer == "rerun" && by == BY_SUPERVISOR),
        "{answered:?}"
    );
    // Naming `rerun` in `decides` does not reach a gate's `rerun`.
    std::fs::write(
        env.data.pipeline(ORCHARD),
        tried_pipeline(ORCHARD, &["rerun"]),
    )
    .unwrap();
    let d = ask(&mut env);
    refused(
        &env.cli(Some(SUPERVISOR), &["decide", &t.id, &d, "rerun"]),
        "the supervisor may not answer `tried`; the owner does",
    );
    let refused_one = env.ticket(&t.id).decisions[1].clone();
    assert!(refused_one.pending());
    assert_eq!(refused_one.refusals.len(), 1);
    assert_eq!(refused_one.refusals[0].answer, "rerun");
}

/// `pipeline(project, &[])` whose first stage is a deploy, and whose
/// supervisor may use the runner.
fn deploying_pipeline(project: &str) -> String {
    pipeline(project, &[])
        .replace(
            "[[stages]]\nname = \"inspect\"",
            "[[stages]]\nname = \"deploy\"\n\
             gate = { kind = \"command\", argv = [\"true\"], in = \"lane\" }\n\n\
             [[stages]]\nname = \"inspect\"",
        )
        .replace("decides = []", "decides = []\nmay = [\"runner\"]")
}

#[test]
fn the_runner_is_refused_to_a_supervisor_without_may_and_the_refusal_kept() {
    let env = Env::new();
    env.seat(ORCHARD, SUPERVISOR);
    for malformed in [&["runner"][..], &["runner", "bogus"]] {
        let out = env.cli(Some(SUPERVISOR), malformed);
        assert_eq!(out.status.code(), Some(64), "{out:?}");
    }
    let out = env.cli(Some(SUPERVISOR), &["runner", "restart"]);
    refused(
        &out,
        "the supervisor may not run `dispatch runner restart` unless its table's `may` \
         lists `runner`; the owner does",
    );
    let ps = env.runner.load_project(ORCHARD).unwrap();
    let answers: Vec<&str> = ps
        .supervisor
        .refusals
        .iter()
        .map(|r| r.answer.as_str())
        .collect();
    assert_eq!(answers, ["runner restart"]);
    assert_eq!(ps.supervisor.refusals[0].by, BY_SUPERVISOR);
    let shown = env.cli(None, &["supervisor", ORCHARD]);
    accepted(&shown);
    assert!(
        stdout(&shown).contains("refused: runner restart at "),
        "{}",
        stdout(&shown)
    );
}

#[test]
fn the_runner_is_a_supervisors_once_its_table_says_may() {
    let env = Env::new();
    env.seat(ORCHARD, SUPERVISOR);
    let text = pipeline(ORCHARD, &["finalize"]).replace(
        "decides = [\"finalize\"]",
        "decides = [\"finalize\"]\nmay = [\"runner\"]",
    );
    std::fs::write(env.data.pipeline(ORCHARD), text).unwrap();
    let out = env.cli(Some(SUPERVISOR), &["runner", "restart"]);
    // Past the rule: it fails on the missing Switchboard socket.
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(stderr(&out).contains("control socket"), "{}", stderr(&out));
    assert!(!stderr(&out).contains("may not"), "{}", stderr(&out));
    let ps = env.runner.load_project(ORCHARD).unwrap();
    assert_eq!(ps.supervisor.refusals.len(), 0);
}

#[test]
fn a_restart_is_refused_while_a_ticket_runs_a_deploy() {
    let mut env = Env::new();
    std::fs::write(env.data.pipeline(ORCHARD), deploying_pipeline(ORCHARD)).unwrap();
    env.seat(ORCHARD, SUPERVISOR);
    let t = env.take(ORCHARD, 1);
    let now = env.tick();
    env.runner.step_all(now).unwrap();
    let deploy = env.ticket(&t.id);
    assert!(
        deploy
            .attempts
            .iter()
            .any(|a| a.stage == "deploy" && a.is_open()),
        "{:?}",
        deploy.attempts
    );
    for record in [Some(SUPERVISOR), None] {
        let out = env.cli(record, &["runner", "restart"]);
        refused(&out, &format!("ticket {} is running `deploy`", t.id));
    }
    // A start kills nothing, so a deploy does not hold it back.
    let out = env.cli(Some(SUPERVISOR), &["runner", "start"]);
    assert!(stderr(&out).contains("control socket"), "{}", stderr(&out));
}

/// The refusals saved on `project`'s record, as typed after `dispatch`.
fn refusals(env: &Env, project: &str) -> Vec<String> {
    let ps = env.runner.load_project(project).unwrap();
    for r in &ps.supervisor.refusals {
        assert_eq!(r.by, BY_SUPERVISOR);
    }
    ps.supervisor
        .refusals
        .iter()
        .map(|r| r.answer.clone())
        .collect()
}

#[test]
fn a_restart_is_refused_to_a_supervisor_without_may_and_the_refusal_kept() {
    for may in ["", "may = [\"runner\"]"] {
        let mut env = Env::new();
        let text = pipeline(ORCHARD, &["finalize"]).replace(
            "decides = [\"finalize\"]",
            &format!("decides = [\"finalize\"]\n{may}"),
        );
        std::fs::write(env.data.pipeline(ORCHARD), text).unwrap();
        env.seat(ORCHARD, SUPERVISOR);
        let t = env.take(ORCHARD, 1);
        let before = env.ticket(&t.id);
        let out = env.cli(Some(SUPERVISOR), &["restart"]);
        assert_eq!(out.status.code(), Some(64), "{out:?}");
        assert_eq!(refusals(&env, ORCHARD), Vec::<String>::new());
        let out = env.cli(Some(SUPERVISOR), &["restart", &t.id, "inspect"]);
        refused(
            &out,
            &format!(
                "the supervisor may not run `dispatch restart {} inspect` unless its table's \
                 `may` lists `restart`; the owner does",
                t.id
            ),
        );
        assert_eq!(env.ticket(&t.id), before);
        assert_eq!(
            refusals(&env, ORCHARD),
            [format!("restart {} inspect", t.id)]
        );
        // A note does not get a restart past the capability, and is
        // not kept with the refusal.
        let out = env.cli(
            Some(SUPERVISOR),
            &["restart", &t.id, "inspect", "--note", "try again"],
        );
        refused(
            &out,
            &format!(
                "the supervisor may not run `dispatch restart {} inspect` unless its table's \
                 `may` lists `restart`; the owner does",
                t.id
            ),
        );
        assert_eq!(env.ticket(&t.id), before);
        assert_eq!(
            refusals(&env, ORCHARD),
            [
                format!("restart {} inspect", t.id),
                format!("restart {} inspect", t.id)
            ]
        );
        let shown = env.cli(None, &["supervisor", ORCHARD]);
        accepted(&shown);
        assert!(
            stdout(&shown).contains(&format!("refused: restart {} inspect at ", t.id)),
            "{}",
            stdout(&shown)
        );
    }
}

#[test]
fn a_restart_is_a_supervisors_once_its_table_says_may() {
    let mut env = Env::new();
    let text = pipeline(ORCHARD, &["finalize"]).replace(
        "decides = [\"finalize\"]",
        "decides = [\"finalize\"]\nmay = [\"runner\", \"restart\"]",
    );
    std::fs::write(env.data.pipeline(ORCHARD), text).unwrap();
    env.seat(ORCHARD, SUPERVISOR);
    let a = env.take(ORCHARD, 1);
    let b = env.take(ORCHARD, 2);
    let theirs = env.take(GROVE, 3);
    let by_owner = env.cli(None, &["restart", &a.id, "inspect"]);
    let by_supervisor = env.cli(Some(SUPERVISOR), &["restart", &b.id, "inspect"]);
    accepted(&by_owner);
    accepted(&by_supervisor);
    // Nothing runs on a ticket only taken, so each restart parks and
    // applies in the one command; the park's stamp is on its event.
    let events = dispatch::events::read_since(&dispatch::events::log_path(&env.data), 0).unwrap();
    for (t, actor) in [(&a, None), (&b, Some(BY_SUPERVISOR))] {
        let t = env.ticket(&t.id);
        assert!(t.active(), "{:?}", t.state);
        assert_eq!(t.restarts.len(), 1);
        let parking = events
            .iter()
            .find(|e| e.ticket == t.id && e.kind == Kind::Parking)
            .expect("a parking event");
        assert_eq!(parking.actor.as_deref(), actor);
    }
    refused(
        &env.cli(Some(SUPERVISOR), &["restart", &theirs.id]),
        "the supervisor of Orchard may not act on Grove's tickets",
    );
    // With `may`, a note passes the capability; the restart itself
    // refuses it, as inspect runs no agent.
    let out = env.cli(
        Some(SUPERVISOR),
        &["restart", &b.id, "inspect", "--note", "try again"],
    );
    assert!(
        stderr(&out).contains("a note reaches only an agent stage's prompt; inspect runs none"),
        "{out:?}"
    );
    assert!(!stderr(&out).contains("may not"), "{out:?}");
    assert_eq!(refusals(&env, ORCHARD), Vec::<String>::new());
}

#[test]
fn a_supervisors_restart_is_refused_while_its_ticket_runs_a_deploy() {
    let mut env = Env::new();
    let text = deploying_pipeline(ORCHARD)
        .replace("may = [\"runner\"]", "may = [\"runner\", \"restart\"]");
    std::fs::write(env.data.pipeline(ORCHARD), text).unwrap();
    env.seat(ORCHARD, SUPERVISOR);
    let t = env.take(ORCHARD, 1);
    let now = env.tick();
    env.runner.step_all(now).unwrap();
    let deploy = env.ticket(&t.id);
    assert!(
        deploy
            .attempts
            .iter()
            .any(|a| a.stage == "deploy" && a.is_open()),
        "{:?}",
        deploy.attempts
    );
    let out = env.cli(Some(SUPERVISOR), &["restart", &t.id, "inspect"]);
    refused(
        &out,
        &format!(
            "ticket {} is running `deploy`; restart it when that stage ends",
            t.id
        ),
    );
    assert!(!stderr(&out).contains("may not"), "{out:?}");
    assert_eq!(env.ticket(&t.id), deploy);
    assert_eq!(refusals(&env, ORCHARD), Vec::<String>::new());
}

#[test]
fn the_owners_verbs_are_refused_to_a_supervisor() {
    let mut env = Env::new();
    env.seat(ORCHARD, SUPERVISOR);
    let t = env.take(ORCHARD, 1);
    let ticket_before = env.ticket(&t.id);
    let project_before = env.runner.load_project(ORCHARD).unwrap();
    let settings = env.data.root.join("settings.json");
    let settings_before = std::fs::read_to_string(&settings).unwrap();
    let elsewhere = env.dir.path().join("elsewhere").display().to_string();
    for (args, form) in [
        (vec!["run", "--once"], "run"),
        (vec!["run"], "run"),
        (vec!["worktrees", elsewhere.as_str()], "worktrees "),
        (vec!["worktrees", "--migrate"], "worktrees --migrate"),
        (
            vec!["supervisor", ORCHARD, "--fresh"],
            "supervisor Orchard --fresh",
        ),
        (
            vec!["supervisor", ORCHARD, "--resume"],
            "supervisor Orchard --resume",
        ),
        (
            vec!["supervisor", ORCHARD, "--kill"],
            "supervisor Orchard --kill",
        ),
        (vec!["frob"], "frob"),
    ] {
        let out = env.cli(Some(SUPERVISOR), &args);
        refused(
            &out,
            &format!("the supervisor may not run `dispatch {form}"),
        );
    }
    assert_eq!(env.ticket(&t.id), ticket_before);
    assert_eq!(env.runner.load_project(ORCHARD).unwrap(), project_before);
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), settings_before);
    assert!(
        !env.data.root.join("runner.json").exists(),
        "nothing stepped"
    );
    // The reads are its.
    for args in [
        vec!["worktrees"],
        vec!["supervisor", ORCHARD],
        vec!["status"],
        vec!["decisions"],
        vec!["show", t.id.as_str()],
        vec!["report", t.id.as_str()],
    ] {
        accepted(&env.cli(Some(SUPERVISOR), &args));
    }
    // The owner runs them all; the offline ones here.
    accepted(&env.cli(None, &["worktrees"]));
    accepted(&env.cli(None, &["queue", GROVE]));
    accepted(&env.cli(None, &["park", &t.id]));
    let owner = Actor::Owner;
    for args in [
        vec!["restart", "x"],
        vec!["run", "--once"],
        vec!["worktrees", "/x", "--migrate"],
        vec!["supervisor", ORCHARD, "--kill"],
        vec!["take", GROVE, "7"],
        vec!["frob"],
    ] {
        permit(&owner, &args, &env.data).unwrap();
    }
}

#[test]
fn the_supervisor_command_shows_the_session_and_the_paths() {
    let mut env = Env::new();
    let out = env.cli(None, &["supervisor", ORCHARD]);
    accepted(&out);
    assert!(stdout(&out).contains("supervisor: none; `dispatch supervisor Orchard --fresh`"));
    let c = env.fresh().unwrap();
    let out = stdout(&env.cli(None, &["supervisor", ORCHARD]));
    for want in [
        format!("session {}", c.session),
        "(current)".to_owned(),
        format!("workspace {}", env.workspace().display()),
        format!("hand-off {}", env.handoff().display()),
        "replaced 0 supervisor(s)".to_owned(),
        "open it from the Dispatch page".to_owned(),
    ] {
        assert!(out.contains(&want), "lacks {want:?}:\n{out}");
    }
}

#[test]
fn a_workspace_path_a_shell_would_split_is_refused() {
    let mut env = Env::new();
    let spaced: &Path = &env.dir.path().join("with space");
    env.data
        .write_settings(&dispatch::store::Settings {
            worktrees: Some(spaced.to_path_buf()),
        })
        .unwrap();
    let e = env.fresh().unwrap_err();
    assert!(format!("{e:#}").contains("supervisor workspace"), "{e:#}");
    assert_eq!(env.sb().kinds_called("session.new"), 0);
}

impl Env {
    fn sb(&self) -> std::sync::MutexGuard<'_, FakeSwitchboard> {
        self.sb.lock().unwrap()
    }
}

// --- subscriptions: the runner types a ticket's moves into the pane

impl Env {
    /// A fresh supervisor for Orchard at its prompt; its session id.
    fn idle_supervisor(&mut self) -> String {
        let session = self.fresh().unwrap().session;
        let now = self.now;
        self.sb().stop(&session, now);
        session
    }

    fn subscribe(&mut self, ticket: &str) {
        let now = self.tick();
        self.runner
            .subscribe(ticket, "move", None, "owner", now)
            .unwrap();
    }

    /// One delivery pass, late enough that every line written before it
    /// has settled.
    fn deliver(&mut self) {
        self.now += 3_000;
        self.deliver_now();
    }

    /// One delivery pass at the clock as it is.
    fn deliver_now(&mut self) {
        let now = self.now;
        self.runner.deliver_subscriptions(ORCHARD, now).unwrap();
    }

    /// The ticket moved on a stage.
    fn moved(&mut self, ticket: &str) {
        let now = self.tick();
        let mut t = self.runner.load_ticket(ticket).unwrap();
        t.stage += 1;
        self.runner.save_ticket(&mut t, now).unwrap();
    }

    fn set_state(&mut self, ticket: &str, state: TicketState) {
        let now = self.tick();
        let mut t = self.runner.load_ticket(ticket).unwrap();
        t.state = state;
        self.runner.save_ticket(&mut t, now).unwrap();
    }

    fn supervision(&self) -> dispatch::ticket::Supervision {
        self.runner.load_project(ORCHARD).unwrap().supervisor
    }

    fn since(&self, ticket: &str) -> Option<u64> {
        self.supervision()
            .subscriptions
            .iter()
            .find(|s| s.ticket == ticket)
            .map(|s| s.since)
    }

    /// The `session.prompt` requests seen: op and session.
    fn prompts(&self) -> Vec<(String, String)> {
        self.sb()
            .calls
            .iter()
            .filter_map(|c| match &c.body {
                Body::SessionPrompt { session, .. } => Some((c.op.clone(), session.clone())),
                _ => None,
            })
            .collect()
    }

    fn sent(&self) -> Vec<(String, String)> {
        self.sb().sent.clone()
    }

    fn tail(&self) -> u64 {
        dispatch::events::last_seq(&dispatch::events::log_path(&self.data)).unwrap()
    }
}

#[test]
fn a_subscribed_move_is_typed_into_the_current_supervisor() {
    let mut env = Env::new();
    let sup = env.idle_supervisor();
    let t = env.take(ORCHARD, 1);
    env.subscribe(&t.id);
    env.deliver();
    assert!(env.prompts().is_empty(), "nothing is due, nothing is asked");
    assert_eq!(env.sb().kinds_called("session"), 0);
    env.moved(&t.id);
    let seq = env.tail();
    env.deliver();
    let sent = env.sent();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].0, sup);
    let text = &sent[0].1;
    assert!(
        text.starts_with(&format!("Dispatch subscription: events on {}\n", t.id)),
        "{text}"
    );
    assert!(text.contains(&format!("{seq}  ")), "{text}");
    assert!(text.contains("  stage  "), "{text}");
    assert!(
        text.ends_with("Act on these as your seed says; nothing to re-arm."),
        "{text}"
    );
    assert_eq!(env.since(&t.id), Some(seq));
    let s = env.supervision();
    assert_eq!(s.delivery, None);
    assert_eq!(s.delivery_waits, None);
    // Delivered once.
    env.sb().stop(&sup, 1);
    env.deliver();
    assert_eq!(env.sent().len(), 1);
}

#[test]
fn a_burst_is_held_until_it_settles_and_goes_in_as_one_prompt() {
    let mut env = Env::new();
    env.idle_supervisor();
    let t = env.take(ORCHARD, 1);
    env.subscribe(&t.id);
    env.moved(&t.id);
    env.deliver_now();
    assert!(env.prompts().is_empty(), "the burst may still grow");
    let d = env.ask(&t.id, "inspect");
    env.deliver();
    let sent = env.sent();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].1.contains("  stage  "), "{}", sent[0].1);
    assert!(sent[0].1.contains("  decision  "), "{}", sent[0].1);
    assert!(
        sent[0].1.contains(&format!(
            " decide {} {d} <answer> [--note <text> | --file <path>]",
            t.id
        )),
        "{}",
        sent[0].1
    );
}

#[test]
fn a_lost_reply_is_sent_again_under_its_op_and_typed_once() {
    let mut env = Env::new();
    env.idle_supervisor();
    let t = env.take(ORCHARD, 1);
    env.subscribe(&t.id);
    let before = env.since(&t.id);
    env.moved(&t.id);
    env.sb().drop_reply_for = Some("session.prompt".into());
    env.deliver();
    assert!(env.supervision().delivery.is_some());
    assert_eq!(env.since(&t.id), before, "no reply, no cursor");
    env.deliver();
    let prompts = env.prompts();
    assert_eq!(prompts.len(), 2);
    assert_eq!(prompts[0], prompts[1], "the same op, to the same session");
    assert_eq!(env.sent().len(), 1, "answered from the log");
    assert_eq!(env.supervision().delivery, None);
    assert_eq!(env.since(&t.id), Some(env.tail()));
}

#[test]
fn a_refused_delivery_keeps_the_cursor_and_goes_in_later() {
    let mut env = Env::new();
    env.idle_supervisor();
    let t = env.take(ORCHARD, 1);
    env.subscribe(&t.id);
    let before = env.since(&t.id);
    env.moved(&t.id);
    env.sb().fail_next = Some("session.prompt".into());
    env.deliver();
    let s = env.supervision();
    assert_eq!(s.delivery, None);
    assert_eq!(
        s.delivery_waits.as_deref(),
        Some("session.prompt refused by the test")
    );
    assert_eq!(env.since(&t.id), before);
    env.deliver();
    assert_eq!(env.sent().len(), 1);
    assert_eq!(env.supervision().delivery_waits, None);
}

#[test]
fn a_busy_supervisor_is_asked_once_a_pass_and_written_once() {
    let mut env = Env::new();
    let sup = env.fresh().unwrap().session;
    let t = env.take(ORCHARD, 1);
    env.subscribe(&t.id);
    env.moved(&t.id);
    env.deliver();
    let file = env.data.project_file(ORCHARD);
    let written = std::fs::read_to_string(&file).unwrap();
    assert_eq!(env.supervision().delivery_waits.as_deref(), Some("busy"));
    for _ in 0..3 {
        env.deliver();
    }
    assert_eq!(std::fs::read_to_string(&file).unwrap(), written);
    assert!(env.prompts().is_empty());
    assert_eq!(env.sb().kinds_called("session"), 4);
    // The turn ends.
    env.sb().stop(&sup, 1);
    env.deliver();
    assert_eq!(env.sent().len(), 1);
}

#[test]
fn an_idle_supervisor_with_its_own_ask_takes_a_delivery_and_one_at_a_prompt_does_not() {
    let mut env = Env::new();
    let sup = env.idle_supervisor();
    let t = env.take(ORCHARD, 1);
    env.subscribe(&t.id);
    env.moved(&t.id);
    env.sb().at_prompt.insert(sup.clone());
    env.deliver();
    assert!(env.prompts().is_empty());
    assert_eq!(
        env.supervision().delivery_waits.as_deref(),
        Some("at a prompt (permission for Bash)")
    );
    env.sb().at_prompt.clear();
    env.sb().session_mut(&sup).card = "waiting on you".into();
    env.deliver();
    assert_eq!(env.sent().len(), 1);
}

#[test]
fn a_park_is_delivered_once_and_a_second_park_after_a_resume_again() {
    let mut env = Env::new();
    let sup = env.idle_supervisor();
    let t = env.take(ORCHARD, 1);
    env.subscribe(&t.id);
    env.parked(&t.id);
    env.deliver();
    assert_eq!(env.sent().len(), 1);
    assert!(env.sent()[0].1.contains("  parked  "), "{:?}", env.sent());
    // A write to the parked ticket moves its time, not its park.
    for _ in 0..3 {
        env.sb().stop(&sup, 1);
        let now = env.tick();
        let mut parked = env.ticket(&t.id);
        parked.source.body.push('x');
        env.runner.save_ticket(&mut parked, now).unwrap();
        env.deliver();
    }
    assert_eq!(env.sent().len(), 1);
    assert_eq!(env.supervision().subscriptions.len(), 1, "a park keeps it");
    env.set_state(&t.id, TicketState::Active);
    env.deliver();
    assert!(!env.supervision().subscriptions[0].park_seen);
    env.parked(&t.id);
    env.deliver();
    assert_eq!(env.sent().len(), 2, "{:?}", env.sent());
}

#[test]
fn subscribing_to_a_parked_ticket_delivers_nothing_until_it_moves() {
    let mut env = Env::new();
    env.idle_supervisor();
    let t = env.take(ORCHARD, 1);
    env.parked(&t.id);
    env.subscribe(&t.id);
    assert!(env.supervision().subscriptions[0].park_seen);
    env.deliver();
    env.deliver();
    assert!(env.prompts().is_empty());
    assert_eq!(env.sb().kinds_called("session"), 0, "nothing was due");
}

#[test]
fn the_runner_pass_delivers_a_park() {
    let mut env = Env::new();
    env.idle_supervisor();
    let t = env.take(ORCHARD, 1);
    env.subscribe(&t.id);
    env.parked(&t.id);
    env.now += 3_000;
    let now = env.now;
    env.runner.step_all(now).unwrap();
    assert_eq!(env.sent().len(), 1, "{:?}", env.sent());
}

#[test]
fn a_close_delivered_ends_the_subscription() {
    let mut env = Env::new();
    env.idle_supervisor();
    let t = env.take(ORCHARD, 1);
    env.subscribe(&t.id);
    env.set_state(
        &t.id,
        TicketState::Closed {
            reason: "done".into(),
        },
    );
    env.deliver();
    assert!(env.sent()[0].1.contains("  closed  "), "{:?}", env.sent());
    assert!(env.supervision().subscriptions.is_empty());
    let out = env.cli(None, &["subscribe", &t.id]);
    refused(&out, "is closed");
}

#[test]
fn an_app_without_session_prompt_is_tried_again_after_a_while() {
    let mut env = Env::new();
    env.idle_supervisor();
    let t = env.take(ORCHARD, 1);
    env.subscribe(&t.id);
    env.moved(&t.id);
    env.sb().old_app = true;
    env.deliver();
    assert_eq!(env.prompts().len(), 1);
    assert_eq!(
        env.supervision().delivery_waits.as_deref(),
        Some(dispatch::subscribe::UPDATE_THE_APP)
    );
    for _ in 0..5 {
        env.deliver();
    }
    assert_eq!(env.prompts().len(), 1);
    env.now += dispatch::subscribe::UNSUPPORTED_RETRY_MS;
    env.deliver();
    assert_eq!(env.prompts().len(), 2);
    for _ in 0..5 {
        env.deliver();
    }
    assert_eq!(
        env.prompts().len(),
        2,
        "the second refusal restarts the wait"
    );
    env.now += dispatch::subscribe::UNSUPPORTED_RETRY_MS;
    env.deliver();
    assert_eq!(env.prompts().len(), 3);
    assert!(env.sent().is_empty());
}

#[test]
fn a_supervisor_not_running_is_never_resumed_for_a_delivery() {
    for gone in [Liveness::Exited { code: Some(0) }, Liveness::Missing] {
        let mut env = Env::new();
        let sup = env.idle_supervisor();
        let t = env.take(ORCHARD, 1);
        env.subscribe(&t.id);
        env.moved(&t.id);
        env.sb().session_mut(&sup).liveness = gone;
        env.deliver();
        env.deliver();
        assert!(env.prompts().is_empty());
        assert_eq!(env.sb().kinds_called("session.resume"), 0);
        assert_eq!(
            env.supervision().delivery_waits.as_deref(),
            Some("not running")
        );
    }
}

#[test]
fn a_fresh_supervisor_inherits_the_backlog() {
    let mut env = Env::new();
    let old = env.fresh().unwrap().session;
    let t = env.take(ORCHARD, 1);
    env.subscribe(&t.id);
    env.moved(&t.id);
    env.deliver();
    assert!(env.prompts().is_empty(), "the old one was busy");
    let new = env.idle_supervisor();
    assert_ne!(old, new);
    env.deliver();
    let sent = env.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, new);
}

#[test]
fn a_delivery_in_flight_to_a_replaced_supervisor_is_dropped_and_the_new_one_gets_it() {
    let mut env = Env::new();
    let old = env.idle_supervisor();
    let t = env.take(ORCHARD, 1);
    env.subscribe(&t.id);
    let before = env.since(&t.id);
    env.moved(&t.id);
    env.sb().drop_reply_for = Some("session.prompt".into());
    env.deliver();
    assert_eq!(env.supervision().delivery.unwrap().session, old);
    let new = env.idle_supervisor();
    env.deliver();
    assert_eq!(env.supervision().delivery, None);
    assert_eq!(env.since(&t.id), before);
    let to_old = env.prompts().iter().filter(|(_, s)| *s == old).count();
    assert_eq!(to_old, 1, "never sent again to the old session");
    env.deliver();
    let sent = env.sent();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].0, new);
    assert_eq!(sent[0].1, sent[1].1, "the same lines");
}

#[test]
fn brief_and_subscriptions_show_what_is_not_delivered() {
    let mut env = Env::new();
    env.seat(ORCHARD, SUPERVISOR);
    let t = env.take(ORCHARD, 1);
    let other = env.take(GROVE, 2);
    accepted(&env.cli(Some(SUPERVISOR), &["subscribe", &t.id]));
    refused(
        &env.cli(Some(SUPERVISOR), &["subscribe", &other.id]),
        "may not act on Grove's tickets",
    );
    let sub = env.supervision().subscriptions[0].clone();
    assert_eq!(sub.by, SUPERVISOR);
    env.moved(&t.id);
    let text = stdout(&env.cli(Some(SUPERVISOR), &["brief", ORCHARD]));
    assert!(
        text.contains("subscriptions (the runner types these into your pane when you are idle):"),
        "{text}"
    );
    assert!(
        text.contains(&format!("{} from seq {}: 1 undelivered", t.id, sub.since)),
        "{text}"
    );
    let text = stdout(&env.cli(Some(SUPERVISOR), &["subscriptions", ORCHARD]));
    assert!(text.contains("1 undelivered"), "{text}");
    refused(
        &env.cli(Some(SUPERVISOR), &["subscriptions", GROVE]),
        "may not act on Grove",
    );
    accepted(&env.cli(Some(SUPERVISOR), &["unsubscribe", &t.id]));
    assert!(env.supervision().subscriptions.is_empty());
}
