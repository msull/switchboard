//! The first slice, end to end against a Switchboard in memory: an issue
//! becomes a ticket, a worktree, one project and four sessions, and
//! stops at the finalize decision; and every way the path can be cut
//! short (a lost reply, a removed record, a launch the app died in, an
//! agent that never wrote) ends as a decision, never a second launch.

mod support;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use dispatch::git::FakeRepo;
use dispatch::scheduler::Runner;
use dispatch::store::DataDir;
use dispatch::ticket::{AttemptState, Decision, SourceSnapshot, Ticket, TicketState};
use support::{FakeSwitchboard, SharedPort};
use switchboard_control::{Body, Liveness, RunState, SessionKind};

const PROJECT: &str = "Switchboard";

fn pipeline(worktrees: &std::path::Path) -> String {
    format!(
        r#"
version = 1

[project]
name = "Switchboard"
repo = "git@example.com:msull/switchboard.git"
worktrees = "{worktrees}"
space = "Dispatch · Switchboard"

[source]
kind = "github"
repo = "msull/switchboard"
label = "dispatch"

[[lanes]]
name = "repo"
path = "."
setup = ["cargo", "fetch", "--locked"]

[operators.investigator]
kind = "claude"
guidance = "Read CLAUDE.md first."

[operators.planner]
kind = "claude"

[operators.reviewer]
kind = "codex"
[operators.reviewer.review]
reviewer = "codex"
review_first = "Review {{plan}} for the tree at {{worktree}}; write to {{feedback}}; else {{no_feedback}}"
review_round = "Again {{response}} {{plan}} {{feedback}} {{no_feedback}}"
respond = "Feedback at {{feedback}}; edit {{plan}}; answer at {{response}}."
respond_to_user = "{{text}} {{plan}} {{response}}"
handoff = "The plan at {{plan}} is final."
no_feedback = "No further feedback."
cap = 4

[operators.implementer]
kind = "claude"

[[stages]]
name = "investigate"
operator = "investigator"
context = "root"
writes = ["notes"]
prompt = "Issue #{{issue.number}}: {{issue.title}}\n\n{{issue.body}}\n\nWrite to {{notes}}."

[[stages]]
name = "lanes"
gate = {{ kind = "human", decision = "lanes" }}

[[stages]]
name = "plan"
operator = "planner"
context = "each"
writes = ["plan"]
prompt = "Using {{inputs.notes}}, plan #{{issue.number}} to {{plan}} on {{branch}}."

[[stages]]
name = "review"
review = "reviewer"
context = "each"
subject = "plan"
gate = {{ kind = "external", check = "review-finalized" }}

[[stages]]
name = "implement"
operator = "implementer"
context = "each"
writes = ["notes"]
prompt = "Implement {{inputs.plan}} on {{branch}}."
gate = {{ kind = "command", argv = ["sh", "-c", "cargo test"], in = "lane" }}

[policy]
slots = 2
waiting_on_me = 3
decisions = {{ lanes = "auto", finalize = "ask" }}
"#,
        worktrees = worktrees.display()
    )
}

struct Env {
    _dir: tempfile::TempDir,
    data: DataDir,
    sb: Arc<Mutex<FakeSwitchboard>>,
    runner: Runner,
    worktrees: PathBuf,
    now: u64,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path().join("dispatch"));
        let worktrees = dir.path().join("wt");
        std::fs::create_dir_all(data.root.join("pipelines")).unwrap();
        std::fs::write(data.pipeline(PROJECT), pipeline(&worktrees)).unwrap();
        let sb = Arc::new(Mutex::new(FakeSwitchboard::new()));
        let runner = Runner::new(
            data.clone(),
            Box::new(SharedPort(Arc::clone(&sb))),
            Box::new(FakeRepo::default()),
        );
        Self {
            _dir: dir,
            data,
            sb,
            runner,
            worktrees,
            now: 1_000,
        }
    }

    /// Dispatch restarts: a fresh runner over the same records and the
    /// same Switchboard, reconciling its ledger first.
    fn restart(&mut self) {
        self.runner = Runner::new(
            self.data.clone(),
            Box::new(SharedPort(Arc::clone(&self.sb))),
            Box::new(FakeRepo::default()),
        );
        let now = self.tick();
        self.runner.recover(now).unwrap();
    }

    fn tick(&mut self) -> u64 {
        self.now += 1_000;
        self.now
    }

    fn take(&mut self, number: u64) -> Ticket {
        let text = std::fs::read_to_string(self.data.pipeline(PROJECT)).unwrap();
        let now = self.tick();
        self.runner
            .take(
                PROJECT,
                &text,
                SourceSnapshot {
                    kind: "github".into(),
                    identity: format!("msull/switchboard#{number}"),
                    number: Some(number),
                    title: format!("Issue {number}: escape leaves the field"),
                    body: "Escape should leave the field, not the set.".into(),
                    url: None,
                    labels: vec!["dispatch".into()],
                    taken_at_ms: now,
                },
                now,
            )
            .unwrap()
    }

    fn step(&mut self) {
        let now = self.tick();
        self.runner.step_project(PROJECT, now).unwrap();
    }

    fn ticket(&self, id: &str) -> Ticket {
        self.runner.load_ticket(id).unwrap()
    }

    fn sb(&self) -> std::sync::MutexGuard<'_, FakeSwitchboard> {
        self.sb.lock().unwrap()
    }

    /// The agent wrote its artifact and reported a stop; then enough
    /// polls for the file to settle.
    fn finish(&mut self, session: &str, artifact: &std::path::Path, text: &str) {
        std::fs::write(artifact, text).unwrap();
        let now = self.now;
        self.sb().stop(session, now);
        for _ in 0..dispatch::ticket::SETTLE_POLLS {
            self.step();
        }
    }

    fn pending(&self, id: &str) -> Vec<Decision> {
        self.ticket(id)
            .pending_decisions()
            .into_iter()
            .cloned()
            .collect()
    }

    fn steps_until(
        &mut self,
        id: &str,
        what: &str,
        mut done: impl FnMut(&Ticket, &FakeSwitchboard) -> bool,
    ) {
        for _ in 0..12 {
            {
                let t = self.ticket(id);
                let sb = self.sb();
                if done(&t, &sb) {
                    return;
                }
            }
            self.step();
        }
        let t = self.ticket(id);
        panic!("never reached {what}: {t:#?}");
    }
}

fn session_of(t: &Ticket, stage: &str) -> String {
    t.attempts_of(stage)
        .last()
        .and_then(|a| a.session.clone())
        .unwrap_or_else(|| panic!("no session for {stage}: {t:#?}"))
}

fn artifact_of(t: &Ticket, stage: &str, name: &str) -> PathBuf {
    t.attempts_of(stage)
        .last()
        .and_then(|a| a.artifacts.get(name).cloned())
        .unwrap_or_else(|| panic!("no {name} for {stage}: {t:#?}"))
}

/// Drive a fresh ticket through investigate and lanes into the plan
/// stage, returning the ticket id.
fn through_plan(env: &mut Env, number: u64) -> String {
    let id = env.take(number).id;
    env.step();
    let t = env.ticket(&id);
    let investigator = session_of(&t, "investigate");
    env.finish(
        &investigator,
        &artifact_of(&t, "investigate", "notes"),
        "# notes\nfindings",
    );
    env.steps_until(&id, "the plan stage", |t, _| {
        t.attempts_of("plan").next().is_some()
    });
    id
}

// The whole happy path is one story; reading it in pieces loses the
// order that the acceptance table fixes.
#[test]
#[allow(clippy::too_many_lines)]
fn an_issue_becomes_one_project_four_sessions_one_run_and_a_finalize_decision() {
    let mut env = Env::new();
    let t = env.take(7);
    let id = t.id.clone();
    assert_eq!(t.stage, 0);
    assert!(t.pipeline_file.exists());
    assert_eq!(
        env.runner.load_project(PROJECT).unwrap().queue,
        vec![id.clone()]
    );

    // Step one: the tree is cut from Dispatch's clone, then the space,
    // the ticket's one project at that tree, and the investigator in it.
    env.step();
    let t = env.ticket(&id);
    assert_eq!(t.lanes.len(), 1, "{t:#?}");
    assert_eq!(
        t.lanes[0].branch,
        "dispatch/7-issue-7-escape-leaves-the-field"
    );
    assert_eq!(t.lanes[0].worktree, env.worktrees.join(&id));
    let tree = t.lanes[0].worktree.clone();
    {
        let sb = env.sb();
        assert_eq!(
            sb.spaces
                .iter()
                .filter(|s| s.name == "Dispatch · Switchboard")
                .count(),
            1
        );
        assert_eq!(sb.projects.len(), 1);
        assert_eq!(sb.projects[0].name, "#7 Issue 7: escape leaves the field");
        assert_eq!(sb.projects[0].root, tree);
        let sessions = sb.sessions_named("investigator");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].cwd, tree);
        assert!(sessions[0].op.is_some());
        assert!(sessions[0].notes.contains(&id));
        let prompt = sb
            .calls
            .iter()
            .find_map(|r| match &r.body {
                Body::SessionNew { prompt, .. } => prompt.clone(),
                _ => None,
            })
            .unwrap();
        assert!(prompt.starts_with("Read CLAUDE.md first."), "{prompt}");
        assert!(prompt.contains("Issue #7: Issue 7"), "{prompt}");
        assert!(
            prompt.contains(
                &artifact_of(&t, "investigate", "notes")
                    .display()
                    .to_string()
            )
        );
    }
    let investigator = session_of(&t, "investigate");
    assert_eq!(t.attempts[0].state, AttemptState::Running);
    assert!(t.ledger.iter().all(|o| o.reply.is_some()));
    // Every request was written down before it went out.
    let kinds: Vec<&str> = t.ledger.iter().map(|o| o.kind.as_str()).collect();
    assert_eq!(
        kinds,
        [
            "space.new",
            "project.add",
            "session.new",
            "set.new",
            "set.sync"
        ]
    );

    // Nothing moves while the agent works, or after a stop with no file.
    env.step();
    env.step();
    assert_eq!(env.ticket(&id).stage, 0);

    // The notes settle: the investigator is killed, the one-lane gate
    // passes without asking, and the planner starts in the same tree
    // under the same project.
    env.finish(
        &investigator,
        &artifact_of(&t, "investigate", "notes"),
        "# notes\nfindings",
    );
    let t = env.ticket(&id);
    assert_eq!(t.attempts[0].state, AttemptState::Complete);
    assert!(env.sb().killed.contains(&investigator));
    env.steps_until(&id, "the plan stage", |t, _| {
        t.attempts_of("plan").next().is_some()
    });
    let t = env.ticket(&id);
    assert!(
        t.pending_decisions().is_empty(),
        "the lanes decision was auto"
    );
    {
        let sb = env.sb();
        assert_eq!(sb.projects.len(), 1, "one project per ticket");
        let planner = sb.sessions_named("planner");
        assert_eq!(planner.len(), 1);
        assert_eq!(planner[0].cwd, tree);
        assert_eq!(planner[0].project, sb.projects[0].id);
        let prompt = sb
            .calls
            .iter()
            .find_map(|r| match &r.body {
                Body::SessionNew { prompt, name, .. } if name == "planner" => prompt.clone(),
                _ => None,
            })
            .unwrap();
        assert!(
            prompt.contains(
                &artifact_of(&t, "investigate", "notes")
                    .display()
                    .to_string()
            ),
            "{prompt}"
        );
        assert!(prompt.contains("on dispatch/7-"), "{prompt}");
    }

    // The plan settles: the review starts on a copy, with the reviewer
    // in the copy's directory and a definition named for its content.
    let planner = session_of(&t, "plan");
    let plan = artifact_of(&t, "plan", "plan");
    env.finish(&planner, &plan, "# plan\nsteps");
    env.steps_until(&id, "the review run", |t, _| {
        t.attempts_of("review")
            .last()
            .is_some_and(|a| a.run.is_some())
    });
    let t = env.ticket(&id);
    let review = t.attempts_of("review").last().unwrap().clone();
    let copy = review.artifacts["plan"].clone();
    assert_ne!(copy, plan);
    assert!(copy.starts_with(env.data.ticket_dir(&id).join("review").join("1")));
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "# plan\nsteps");
    {
        let sb = env.sb();
        assert_eq!(sb.runs.len(), 1);
        assert_eq!(sb.runs[0].plan, copy);
        assert_eq!(sb.runs[0].source, planner);
        assert!(sb.definitions[0].name.starts_with("Dispatch: reviewer@"));
        assert_eq!(sb.runs[0].definition, sb.definitions[0].name);
        assert_eq!(sb.session(&sb.runs[0].reviewer).cwd, copy.parent().unwrap());
        assert_eq!(
            sb.sessions.len(),
            4,
            "investigator, planner, reviewer, planner clone"
        );
        assert!(sb.sessions.iter().all(|s| s.op.is_some()));
    }

    // The reviewer has nothing further: a decision names the copy and
    // the planner's card reads as waiting.
    let run = env.sb().runs[0].id.clone();
    env.sb().run_mut(&run).state = RunState::Converged;
    env.step();
    let pending = env.pending(&id);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].name, "finalize");
    assert!(pending[0].question.contains(&copy.display().to_string()));
    let face = env.ticket(&id).current_session().cloned().unwrap();
    {
        let sb = env.sb();
        assert_eq!(face, sb.runs[0].planner.clone().unwrap());
        assert!(sb.waiting[&face].0);
        assert!(sb.notes[&face].contains(&format!("dispatch decide {id} {}", pending[0].id)));
    }
    // Nothing else starts while it waits.
    env.step();
    assert_eq!(env.sb().runs.len(), 1);
    assert_eq!(env.sb().sessions.len(), 4);

    // Answered by hand: the run is finalized, its agents killed, the
    // stage complete; the next stage is not built here, so the ticket
    // parks with a reason rather than guessing.
    let now = env.tick();
    env.runner
        .decide(&id, &pending[0].id, "finalize", None, now)
        .unwrap();
    env.step();
    assert_eq!(env.sb().runs[0].state, RunState::Finalized);
    assert!(!env.sb().waiting[&face].0);
    env.steps_until(&id, "the review completing", |t, _| {
        t.attempts_of("review")
            .last()
            .is_some_and(|a| a.state == AttemptState::Complete)
    });
    {
        let sb = env.sb();
        let reviewer = sb.runs[0].reviewer.clone();
        assert!(sb.killed.contains(&reviewer) && sb.killed.contains(&face));
    }
    env.steps_until(&id, "parking at implement", |t, _| !t.active());
    let t = env.ticket(&id);
    assert!(
        matches!(&t.state, TicketState::Parked { reason } if reason.contains("implement") && reason.contains("not built"))
    );
    assert_eq!(
        env.sb().sessions.len(),
        4,
        "nothing was started for implement"
    );
}

#[test]
fn a_reply_lost_after_the_launch_is_found_again_and_nothing_launches_twice() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.sb().drop_reply_for = Some("session.new".into());
    env.step();
    let t = env.ticket(&id);
    let last = t.ledger.last().unwrap();
    assert_eq!(last.kind, "session.new");
    assert!(last.reply.is_none() && last.error.is_some(), "{t:#?}");
    assert_eq!(t.attempts[0].state, AttemptState::Starting);
    assert!(t.attempts[0].session.is_none());
    assert_eq!(env.sb().sessions.len(), 1, "the session exists");

    env.restart();
    let t = env.ticket(&id);
    assert!(t.ledger.last().unwrap().reply.is_some());
    assert_eq!(t.attempts[0].state, AttemptState::Running);
    assert_eq!(
        t.attempts[0].session.as_deref(),
        Some(env.sb().sessions[0].id.as_str())
    );
    env.step();
    env.step();
    assert_eq!(env.sb().kinds_called("session.new"), 1);
    assert_eq!(env.sb().sessions.len(), 1);
}

#[test]
fn a_record_removed_in_the_window_fails_the_attempt_by_that_name() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.sb().drop_reply_for = Some("session.new".into());
    env.step();
    let session = env.sb().sessions[0].id.clone();
    env.sb().remove(&session);
    env.restart();
    let t = env.ticket(&id);
    assert!(
        matches!(&t.attempts[0].state, AttemptState::Failed { reason } if reason.contains("removed by hand"))
    );
    let pending = env.pending(&id);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].name, "rerun");
    env.step();
    assert_eq!(
        env.sb().kinds_called("session.new"),
        1,
        "no second launch without an answer"
    );
    // Authorised, it runs again as a new attempt with its own files.
    let now = env.tick();
    env.runner
        .decide(&id, &pending[0].id, "rerun", None, now)
        .unwrap();
    env.step();
    let t = env.ticket(&id);
    assert_eq!(t.attempts.len(), 2);
    assert_eq!(t.attempts[1].n, 2);
    assert_ne!(
        t.attempts[1].artifacts["notes"],
        t.attempts[0].artifacts["notes"]
    );
    assert_eq!(env.sb().kinds_called("session.new"), 2);
}

#[test]
fn a_review_whose_reply_was_lost_is_adopted_with_its_run() {
    let mut env = Env::new();
    let id = through_plan(&mut env, 7);
    let t = env.ticket(&id);
    let planner = session_of(&t, "plan");
    env.sb().drop_reply_for = Some("workflow.start".into());
    env.finish(&planner, &artifact_of(&t, "plan", "plan"), "# plan");
    env.steps_until(&id, "a review attempt", |t, _| {
        t.attempts_of("review").next().is_some()
    });
    let t = env.ticket(&id);
    assert!(t.attempts_of("review").last().unwrap().run.is_none());
    env.restart();
    let t = env.ticket(&id);
    let review = t.attempts_of("review").last().unwrap();
    assert_eq!(review.run.as_deref(), Some(env.sb().runs[0].id.as_str()));
    assert_eq!(review.state, AttemptState::Running);
    env.step();
    assert_eq!(env.sb().kinds_called("workflow.start"), 1);
    assert_eq!(env.sb().runs.len(), 1);
}

#[test]
fn a_request_that_never_reached_switchboard_is_lost_not_repeated() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.step();
    // Dispatch died between writing the request down and sending it:
    // Switchboard never saw it.
    let session = env.sb().sessions[0].id.clone();
    {
        let mut sb = env.sb();
        sb.sessions.clear();
        sb.log.clear();
        sb.resumable.clear();
    }
    let mut t = env.ticket(&id);
    let op = t
        .ledger
        .iter_mut()
        .find(|o| o.kind == "session.new")
        .unwrap();
    op.reply = None;
    t.attempts[0].session = None;
    t.attempts[0].state = AttemptState::Starting;
    let now = env.tick();
    env.runner.save_ticket(&mut t, now).unwrap();
    env.restart();
    let t = env.ticket(&id);
    assert!(
        matches!(&t.attempts[0].state, AttemptState::Failed { reason } if reason.contains("lost")),
        "{t:#?}"
    );
    assert_eq!(env.pending(&id)[0].name, "rerun");
    env.step();
    assert_eq!(env.sb().sessions.len(), 0, "nothing launched: {session}");
}

#[test]
fn a_launch_switchboard_died_in_is_interrupted_and_fails() {
    let mut env = Env::new();
    let id = through_plan(&mut env, 7);
    let t = env.ticket(&id);
    let planner = session_of(&t, "plan");
    env.sb().die_launching = Some("workflow.start".into());
    env.finish(&planner, &artifact_of(&t, "plan", "plan"), "# plan");
    env.steps_until(&id, "a review attempt", |t, _| {
        t.attempts_of("review").next().is_some()
    });
    env.restart();
    let t = env.ticket(&id);
    let review = t.attempts_of("review").last().unwrap();
    assert!(
        matches!(&review.state, AttemptState::Failed { reason } if reason.contains("stopped while starting"))
    );
    assert!(review.run.is_none());
    assert_eq!(env.pending(&id)[0].name, "rerun");
    env.step();
    assert_eq!(env.sb().kinds_called("workflow.start"), 1);
}

#[test]
fn an_agent_that_vanishes_after_a_partial_file_fails_and_nothing_advances() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.step();
    let t = env.ticket(&id);
    let investigator = session_of(&t, "investigate");
    std::fs::write(artifact_of(&t, "investigate", "notes"), "# half").unwrap();
    env.sb().vanish(&investigator);
    env.step();
    let t = env.ticket(&id);
    assert!(
        matches!(&t.attempts[0].state, AttemptState::Failed { reason } if reason.contains("no stop"))
    );
    assert_eq!(t.stage, 0);
    assert_eq!(env.pending(&id)[0].name, "rerun");
    // A stop that never wrote the file fails too, after a few polls.
    let now = env.tick();
    env.runner
        .decide(&id, &env.pending(&id)[0].id, "rerun", None, now)
        .unwrap();
    env.step();
    let t = env.ticket(&id);
    let second = session_of(&t, "investigate");
    assert_ne!(second, investigator);
    // The first attempt's file exists; the second attempt's does not,
    // so a stop settles nothing.
    assert!(t.attempts[0].artifacts["notes"].exists());
    assert!(!t.attempts[1].artifacts["notes"].exists());
    let now = env.now;
    env.sb().stop(&second, now);
    for _ in 0..dispatch::ticket::SETTLE_POLLS {
        env.step();
    }
    let t = env.ticket(&id);
    assert!(
        matches!(&t.attempts[1].state, AttemptState::Failed { reason } if reason.contains("without writing notes"))
    );
    assert_eq!(t.stage, 0);
}

#[test]
fn an_objection_changes_the_copy_and_not_the_plan() {
    let mut env = Env::new();
    let id = through_plan(&mut env, 7);
    let t = env.ticket(&id);
    let planner = session_of(&t, "plan");
    let plan = artifact_of(&t, "plan", "plan");
    env.finish(&planner, &plan, "# plan v1");
    env.steps_until(&id, "the review run", |t, _| {
        t.attempts_of("review")
            .last()
            .is_some_and(|a| a.run.is_some())
    });
    let t = env.ticket(&id);
    let copy = t.attempts_of("review").last().unwrap().artifacts["plan"].clone();
    // The planner clone answers an objection by editing the copy.
    std::fs::write(&copy, "# plan v2").unwrap();
    assert_eq!(std::fs::read_to_string(&plan).unwrap(), "# plan v1");
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "# plan v2");
    assert_eq!(
        t.input("plan"),
        Some(&plan),
        "the review's copy is its output only once finalized"
    );
}

#[test]
fn a_restart_while_the_clone_is_in_progress_waits_then_adopts() {
    let mut env = Env::new();
    let id = through_plan(&mut env, 7);
    let t = env.ticket(&id);
    let planner = session_of(&t, "plan");
    env.sb().in_progress_for = Some("workflow.start".into());
    env.finish(&planner, &artifact_of(&t, "plan", "plan"), "# plan");
    env.steps_until(&id, "a review attempt", |t, _| {
        t.attempts_of("review").next().is_some()
    });
    env.restart();
    let t = env.ticket(&id);
    let review = t.attempts_of("review").last().unwrap();
    assert!(review.is_open());
    assert!(
        review.run.is_none(),
        "still in progress: not adopted, not failed"
    );
    env.step();
    assert_eq!(env.sb().kinds_called("workflow.start"), 1);
    env.sb().in_progress_for = None;
    env.step();
    let t = env.ticket(&id);
    let review = t.attempts_of("review").last().unwrap();
    assert!(review.run.is_some());
    assert_eq!(env.sb().kinds_called("workflow.start"), 1);
}

#[test]
fn the_queue_view_follows_the_current_session_and_the_order() {
    let mut env = Env::new();
    let a = env.take(7).id;
    env.step();
    let investigator = session_of(&env.ticket(&a), "investigate");
    {
        let sb = env.sb();
        assert_eq!(sb.sets.len(), 1);
        assert_eq!(sb.sets[0].name, "Dispatch · Switchboard");
        let items = &sb.sets[0].items;
        assert_eq!(items.len(), 1);
        assert!(
            matches!(&items[0].target, switchboard_control::PinTarget::Session { session } if session == &investigator)
        );
    }
    // A second ticket takes the row below.
    let b = env.take(8).id;
    env.step();
    let b_session = session_of(&env.ticket(&b), "investigate");
    {
        let sb = env.sb();
        let items = &sb.sets[0].items;
        assert_eq!(items.len(), 2);
        assert_eq!((items[0].rect.y, items[1].rect.y), (0, 8));
        assert!(
            matches!(&items[1].target, switchboard_control::PinTarget::Session { session } if session == &b_session)
        );
    }
    // The plan replaces the investigator on the card.
    let t = env.ticket(&a);
    env.finish(
        &investigator,
        &artifact_of(&t, "investigate", "notes"),
        "# notes",
    );
    env.steps_until(&a, "the plan stage", |t, _| {
        t.attempts_of("plan").next().is_some()
    });
    let planner = session_of(&env.ticket(&a), "plan");
    {
        let sb = env.sb();
        let items = &sb.sets[0].items;
        assert_eq!(items.len(), 2, "no stale card: {items:?}");
        assert!(
            matches!(&items[0].target, switchboard_control::PinTarget::Session { session } if session == &planner)
        );
    }
    // Swapped by hand: redrawn whole, no overlap in between.
    let mut ps = env.runner.load_project(PROJECT).unwrap();
    ps.queue.reverse();
    env.runner.save_project(&ps).unwrap();
    env.step();
    {
        let sb = env.sb();
        let items = &sb.sets[0].items;
        assert!(
            matches!(&items[0].target, switchboard_control::PinTarget::Session { session } if session == &b_session)
        );
        assert!(
            matches!(&items[1].target, switchboard_control::PinTarget::Session { session } if session == &planner)
        );
    }
    let syncs = env.sb().kinds_called("set.sync");
    env.step();
    assert_eq!(
        env.sb().kinds_called("set.sync"),
        syncs,
        "unchanged: not redrawn"
    );
}

#[test]
fn a_plan_session_with_no_transcript_fails_the_review_once() {
    let mut env = Env::new();
    let id = through_plan(&mut env, 7);
    let t = env.ticket(&id);
    let planner = session_of(&t, "plan");
    env.sb().resumable.retain(|s| s != &planner);
    env.finish(&planner, &artifact_of(&t, "plan", "plan"), "# plan");
    env.steps_until(&id, "the review failing", |t, _| {
        t.attempts_of("review")
            .last()
            .is_some_and(|a| matches!(a.state, AttemptState::Failed { .. }))
    });
    let t = env.ticket(&id);
    assert!(
        matches!(&t.attempts_of("review").last().unwrap().state, AttemptState::Failed { reason } if reason.contains("cannot be the planner"))
    );
    assert_eq!(env.pending(&id)[0].name, "rerun");
    env.step();
    env.step();
    assert_eq!(env.sb().kinds_called("workflow.start"), 1);
    assert!(env.sb().runs.is_empty());
}

#[test]
fn a_project_that_cannot_be_saved_makes_nothing_else() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.sb().fail_next = Some("project.add".into());
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(&t.state, TicketState::Parked { reason } if reason.contains("project.add")));
    assert!(t.attempts.is_empty());
    assert!(env.sb().sessions.is_empty());
    assert_eq!(env.sb().kinds_called("session.new"), 0);
    let liveness: Vec<Liveness> = env.sb().sessions.iter().map(|s| s.liveness).collect();
    assert!(liveness.is_empty());
}

// --- what a review of the slice found: the runner's pass and a command
// from the terminal share one lock, idempotent requests are replayed,
// an attempt saved without its request fails instead of waiting, a
// record's primary is never absent, parking stops what runs, and the
// reviewer is told which tree the plan is about.

/// A port that fails one request of the given kind before it reaches
/// Switchboard, then behaves.
struct FailBefore {
    inner: SharedPort,
    kind: &'static str,
    fired: bool,
}

impl dispatch::port::Port for FailBefore {
    fn call(
        &mut self,
        request: &switchboard_control::Request,
    ) -> std::io::Result<switchboard_control::Reply> {
        if !self.fired && request.body.kind() == self.kind {
            self.fired = true;
            return Err(std::io::Error::other(
                "the socket closed before the request went out",
            ));
        }
        self.inner.call(request)
    }
}

/// Drive a ticket to the finalize decision.
/// A reviewer operator of kind claude runs the review as Claude Code in
/// the ticket's tree, where the code is and the trust was granted, with
/// the operator's flags and the allow rule for the attempt directory on
/// its command line, so the feedback file is still written unasked.
#[test]
fn a_claude_reviewer_runs_in_the_tree_with_its_flags_and_an_allow_rule_for_the_attempt() {
    let mut env = Env::new();
    let path = env.data.pipeline(PROJECT);
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace(
            "[operators.reviewer]\nkind = \"codex\"",
            "[operators.reviewer]\nkind = \"claude\"\nargs = [\"--model\", \"haiku\"]",
        )
        .replace("reviewer = \"codex\"", "reviewer = \"claude\"");
    std::fs::write(&path, text).unwrap();
    let id = through_plan(&mut env, 7);
    let t = env.ticket(&id);
    env.finish(
        &session_of(&t, "plan"),
        &artifact_of(&t, "plan", "plan"),
        "# plan",
    );
    env.steps_until(&id, "the review run", |t, _| {
        t.attempts_of("review")
            .last()
            .is_some_and(|a| a.run.is_some())
    });
    let sb = env.sb();
    let start = sb
        .calls
        .iter()
        .find_map(|r| match &r.body {
            Body::WorkflowStart {
                reviewer_cwd,
                reviewer_args,
                ..
            } => Some((reviewer_cwd.clone().unwrap(), reviewer_args.clone())),
            _ => None,
        })
        .expect("a workflow.start was sent");
    let t = env.ticket(&id);
    let attempt_dir = t
        .attempts_of("review")
        .last()
        .and_then(|a| a.artifacts.get("plan"))
        .and_then(|copy| copy.parent())
        .unwrap()
        .to_path_buf();
    assert_eq!(
        start.0,
        t.tree.clone().unwrap(),
        "the reviewer works in the tree"
    );
    assert_eq!(
        start.1,
        vec![
            "--model".to_owned(),
            "haiku".into(),
            "--allowedTools".into(),
            format!("Edit(//{}/**)", attempt_dir.display()),
        ]
    );
    let run = &sb.runs[0];
    let reviewer = sb.sessions.iter().find(|s| s.id == run.reviewer).unwrap();
    assert_eq!(reviewer.kind, SessionKind::Claude);
    assert_eq!(reviewer.cwd, start.0);
}

fn at_finalize(env: &mut Env) -> String {
    let id = through_plan(env, 7);
    let t = env.ticket(&id);
    env.finish(
        &session_of(&t, "plan"),
        &artifact_of(&t, "plan", "plan"),
        "# plan",
    );
    env.steps_until(&id, "the review run", |t, _| {
        t.attempts_of("review")
            .last()
            .is_some_and(|a| a.run.is_some())
    });
    env.sb().runs[0].state = RunState::Converged;
    env.step();
    assert_eq!(env.pending(&id)[0].name, "finalize");
    id
}

#[test]
fn an_authorised_finalize_survives_a_lost_reply() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let decision = env.pending(&id)[0].id.clone();
    let now = env.tick();
    env.runner
        .decide(&id, &decision, "finalize", None, now)
        .unwrap();
    env.runner.port = Box::new(FailBefore {
        inner: SharedPort(Arc::clone(&env.sb)),
        kind: "workflow.finalize",
        fired: false,
    });
    env.step();
    assert_ne!(
        env.sb().runs[0].state,
        RunState::Finalized,
        "the request was lost"
    );
    let t = env.ticket(&id);
    let lost = t
        .ledger
        .iter()
        .find(|o| o.kind == "workflow.finalize")
        .unwrap();
    assert!(lost.reply.is_none() && lost.error.is_some());
    // The next pass sends the same operation again; a restart would too.
    env.step();
    assert_eq!(env.sb().runs[0].state, RunState::Finalized);
    let t = env.ticket(&id);
    let replayed = t
        .ledger
        .iter()
        .find(|o| o.kind == "workflow.finalize")
        .unwrap();
    assert!(replayed.reply.is_some() && replayed.error.is_none());
    assert_eq!(
        env.sb().kinds_called("workflow.finalize"),
        1,
        "one request reached Switchboard"
    );
}

#[test]
fn parking_pauses_the_run_and_kills_every_process_before_reading_as_parked() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let decision = env.pending(&id)[0].id.clone();
    let now = env.tick();
    env.runner
        .decide(&id, &decision, "park", None, now)
        .unwrap();
    env.step();
    let t = env.ticket(&id);
    assert!(
        matches!(&t.state, TicketState::Parked { reason } if reason.contains("by hand")),
        "{t:#?}"
    );
    let review = t.attempts_of("review").last().unwrap();
    assert!(
        matches!(&review.state, AttemptState::Cancelled { .. }),
        "{review:#?}"
    );
    {
        let sb = env.sb();
        assert!(
            matches!(sb.runs[0].state, RunState::Paused { .. }),
            "{:?}",
            sb.runs[0].state
        );
        assert!(
            sb.sessions.iter().all(|s| s.liveness != Liveness::Running),
            "parking left something running: {:?}",
            sb.sessions
                .iter()
                .filter(|s| s.liveness == Liveness::Running)
                .map(|s| &s.name)
                .collect::<Vec<_>>()
        );
    }
    // Parked is quiet: nothing more is asked of Switchboard.
    let calls = env.sb().calls.len();
    env.step();
    assert_eq!(env.sb().calls.len(), calls);
}

#[test]
fn an_attempt_saved_without_its_request_fails_instead_of_waiting_forever() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.step();
    // Dispatch stopped between writing the attempt and writing its
    // request: no session, no operation. (The runner no longer writes
    // these apart, but a record from a crash may still look like it.)
    let mut t = env.ticket(&id);
    let session = t.attempts[0].session.take().unwrap();
    t.attempts[0].state = AttemptState::Starting;
    t.ledger.retain(|o| o.kind != "session.new");
    {
        let mut sb = env.sb();
        sb.sessions.retain(|s| s.id != session);
        sb.log.retain(|l| l.kind != "session.new");
    }
    let now = env.tick();
    env.runner.save_ticket(&mut t, now).unwrap();
    env.restart();
    env.step();
    let t = env.ticket(&id);
    assert!(
        matches!(&t.attempts[0].state, AttemptState::Failed { reason } if reason.contains("never recorded")),
        "{t:#?}"
    );
    assert_eq!(env.pending(&id)[0].name, "rerun");
    assert_eq!(
        env.sb().kinds_called("session.new"),
        1,
        "nothing launched without an answer"
    );
}

#[test]
fn a_decision_written_while_the_runner_is_mid_pass_is_kept() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let decision = env.pending(&id)[0].id.clone();
    let data = env.data.clone();
    let sb = Arc::clone(&env.sb);
    let now = env.tick();
    // The runner's pass holds the lock; a `dispatch decide` from the
    // terminal waits for it and lands afterwards, on the saved record,
    // instead of being overwritten by the pass's own save.
    let (ticket_id, decision_id) = (id.clone(), decision.clone());
    let answered = env
        .runner
        .transaction(|r| {
            let mut stale = r.load_ticket(&ticket_id)?;
            let cli = std::thread::spawn(move || {
                let cli = Runner::new(
                    data,
                    Box::new(SharedPort(sb)),
                    Box::new(FakeRepo::default()),
                );
                cli.decide(&ticket_id, &decision_id, "finalize", None, now + 1)
                    .map(|d| d.id)
            });
            r.save_ticket(&mut stale, now)?;
            Ok(cli)
        })
        .unwrap()
        .join()
        .unwrap()
        .unwrap();
    assert_eq!(answered, decision);
    let t = env.ticket(&id);
    assert!(
        t.pending_decisions().is_empty(),
        "the answer was overwritten: {t:#?}"
    );
    env.step();
    assert_eq!(env.sb().runs[0].state, RunState::Finalized);
}

#[test]
fn a_project_whose_primary_vanished_reads_its_backup() {
    let mut env = Env::new();
    let id = env.take(7).id;
    let path = env.data.project_file(PROJECT);
    // Only a crash of an older write could leave this; the backup is
    // still the truth.
    std::fs::rename(&path, path.with_file_name("Switchboard.json.bak")).unwrap();
    assert_eq!(
        env.runner.load_project(PROJECT).unwrap().queue,
        vec![id.clone()]
    );
    // And a pass rewrites the primary from it.
    env.step();
    assert!(path.exists());
    assert_eq!(env.runner.load_project(PROJECT).unwrap().queue, vec![id]);
}

#[test]
fn the_reviewer_is_told_which_tree_the_plan_is_about() {
    let mut env = Env::new();
    let id = through_plan(&mut env, 7);
    let t = env.ticket(&id);
    env.finish(
        &session_of(&t, "plan"),
        &artifact_of(&t, "plan", "plan"),
        "# plan",
    );
    env.steps_until(&id, "the review run", |t, _| {
        t.attempts_of("review")
            .last()
            .is_some_and(|a| a.run.is_some())
    });
    let t = env.ticket(&id);
    let sb = env.sb();
    let definition = &sb.definitions[0];
    let worktree = t.lanes[0].worktree.display().to_string();
    assert!(
        definition.review_first.contains(&worktree),
        "{}",
        definition.review_first
    );
    assert!(
        definition.review_first.contains("{plan}"),
        "Switchboard's own placeholders stay"
    );
    assert!(definition.name.starts_with("Dispatch: reviewer@"));
}

#[test]
fn a_rerun_retires_the_replaced_attempt_before_launching() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.step();
    let t = env.ticket(&id);
    let first = session_of(&t, "investigate");
    // Stopped without writing: the attempt fails, but the pane is still
    // up.
    let now = env.now;
    env.sb().stop(&first, now);
    for _ in 0..dispatch::ticket::SETTLE_POLLS {
        env.step();
    }
    let t = env.ticket(&id);
    assert!(
        matches!(&t.attempts[0].state, AttemptState::Failed { .. }),
        "{t:#?}"
    );
    assert_eq!(env.sb().session(&first).liveness, Liveness::Running);
    let now = env.tick();
    env.runner
        .decide(&id, &env.pending(&id)[0].id, "rerun", None, now)
        .unwrap();
    env.step();
    let t = env.ticket(&id);
    assert_eq!(t.attempts.len(), 2, "{t:#?}");
    {
        let sb = env.sb();
        assert!(sb.killed.contains(&first), "the old pane was killed first");
        let kill = sb
            .calls
            .iter()
            .position(|r| matches!(&r.body, Body::SessionKill { session } if session == &first))
            .unwrap();
        let launch = sb
            .calls
            .iter()
            .rposition(|r| matches!(&r.body, Body::SessionNew { .. }))
            .unwrap();
        assert!(kill < launch, "kill at {kill}, launch at {launch}");
    }
}

// --- the follow-up review: cleanup gates a rerun and is persisted first,
// parking resumes whole after a restart, and the documented pipeline's
// own reviewer is told its repository.

#[test]
fn a_rerun_waits_while_the_replaced_process_survives_its_kill() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.step();
    let old = session_of(&env.ticket(&id), "investigate");
    let now = env.now;
    env.sb().stop(&old, now);
    for _ in 0..dispatch::ticket::SETTLE_POLLS {
        env.step();
    }
    let now = env.tick();
    env.runner
        .decide(&id, &env.pending(&id)[0].id, "rerun", None, now)
        .unwrap();
    env.sb().fail_next = Some("session.kill".into());
    env.step();
    assert_eq!(env.sb().session(&old).liveness, Liveness::Running);
    let t = env.ticket(&id);
    assert_eq!(t.attempts.len(), 1, "launched beside the survivor: {t:#?}");
    assert!(
        t.decisions
            .iter()
            .any(|d| d.unacted_answer() == Some("rerun")),
        "the answer stays unacted until the cleanup is done"
    );
    // The next pass kills it and launches.
    env.step();
    let t = env.ticket(&id);
    assert_eq!(t.attempts.len(), 2, "{t:#?}");
    assert_ne!(env.sb().session(&old).liveness, Liveness::Running);
}

#[test]
fn parking_resumed_after_a_restart_still_pauses_the_run_first() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    // The intent was saved, then Dispatch died before the pause.
    let mut t = env.ticket(&id);
    t.state = TicketState::Parking {
        reason: "parked by hand".into(),
    };
    let now = env.tick();
    env.runner.save_ticket(&mut t, now).unwrap();
    env.restart();
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Parked { .. }), "{t:#?}");
    assert!(matches!(
        &t.attempts_of("review").last().unwrap().state,
        AttemptState::Cancelled { .. }
    ));
    let sb = env.sb();
    assert!(
        matches!(sb.runs[0].state, RunState::Paused { .. }),
        "{:?}",
        sb.runs[0].state
    );
    assert!(sb.sessions.iter().all(|s| s.liveness != Liveness::Running));
}

#[test]
fn a_pause_that_does_not_take_keeps_the_ticket_parking() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let decision = env.pending(&id)[0].id.clone();
    let now = env.tick();
    env.runner
        .decide(&id, &decision, "park", None, now)
        .unwrap();
    env.sb().fail_next = Some("workflow.pause".into());
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Parking { .. }), "{t:#?}");
    assert!(
        t.attempts_of("review").last().unwrap().is_open(),
        "not cancelled until paused"
    );
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Parked { .. }), "{t:#?}");
}

/// The Switchboard pipeline exactly as `docs/dispatch.md` shows it, with
/// its paths pointed at this test's directories.
fn documented_pipeline() -> String {
    let design = include_str!("../../docs/dispatch.md");
    design
        .split("### Pipeline: Switchboard")
        .nth(1)
        .unwrap()
        .split("```toml\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap()
        .to_owned()
}

#[test]
fn the_documented_pipelines_reviewer_is_told_its_repository() {
    let mut env = Env::new();
    std::fs::write(env.data.pipeline(PROJECT), documented_pipeline()).unwrap();
    let id = at_finalize(&mut env);
    let worktree = env.ticket(&id).lanes[0].worktree.display().to_string();
    let sb = env.sb();
    let d = &sb.definitions[0];
    assert!(d.review_first.contains(&worktree), "{}", d.review_first);
    assert!(d.review_round.contains(&worktree), "{}", d.review_round);
    assert!(d.review_first.contains("{plan}") && d.review_first.contains("{feedback}"));
}

#[test]
fn a_template_that_never_names_the_tree_is_told_it_anyway() {
    let mut env = Env::new();
    let text = std::fs::read_to_string(env.data.pipeline(PROJECT))
        .unwrap()
        .replace(" for the tree at {worktree}", "");
    assert!(!text.contains("{worktree}"));
    std::fs::write(env.data.pipeline(PROJECT), text).unwrap();
    let id = at_finalize(&mut env);
    let worktree = env.ticket(&id).lanes[0].worktree.display().to_string();
    let sb = env.sb();
    assert!(
        sb.definitions[0]
            .review_first
            .starts_with("The repository this is about is at ")
    );
    assert!(sb.definitions[0].review_first.contains(&worktree));
}

// --- the answer and its action reach disk together: a stop right after
// either the mark or the first request leaves something recovery acts on.

/// A port that fails one request of the given kind with a socket error.
struct SocketFails {
    inner: SharedPort,
    kind: &'static str,
    fired: bool,
}

impl dispatch::port::Port for SocketFails {
    fn call(
        &mut self,
        request: &switchboard_control::Request,
    ) -> std::io::Result<switchboard_control::Reply> {
        if !self.fired && request.body.kind() == self.kind {
            self.fired = true;
            return Err(std::io::Error::other("the socket dropped"));
        }
        self.inner.call(request)
    }
}

#[test]
fn a_park_answer_cut_off_at_its_first_request_is_finished_after_a_restart() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let decision = env.pending(&id)[0].id.clone();
    let now = env.tick();
    env.runner
        .decide(&id, &decision, "park", None, now)
        .unwrap();
    // The pass dies at the pause: the socket error ends the pass.
    env.runner.port = Box::new(SocketFails {
        inner: SharedPort(Arc::clone(&env.sb)),
        kind: "workflow.pause",
        fired: false,
    });
    env.step();
    let t = env.ticket(&id);
    assert!(
        matches!(t.state, TicketState::Parking { .. }),
        "the intent is on disk: {t:#?}"
    );
    assert!(
        t.pending_decisions().is_empty(),
        "the answer is consumed only with its intent"
    );
    env.restart();
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Parked { .. }), "{t:#?}");
    assert!(
        env.sb()
            .sessions
            .iter()
            .all(|s| s.liveness != Liveness::Running)
    );
}

#[test]
fn a_finalize_answer_cut_off_at_its_request_is_replayed_after_a_restart() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let decision = env.pending(&id)[0].id.clone();
    let now = env.tick();
    env.runner
        .decide(&id, &decision, "finalize", None, now)
        .unwrap();
    env.runner.port = Box::new(SocketFails {
        inner: SharedPort(Arc::clone(&env.sb)),
        kind: "workflow.finalize",
        fired: false,
    });
    env.step();
    let t = env.ticket(&id);
    assert!(t.pending_decisions().is_empty());
    let op = t
        .ledger
        .iter()
        .find(|o| o.kind == "workflow.finalize")
        .unwrap();
    assert!(op.reply.is_none(), "the request is on disk for recovery");
    assert_ne!(env.sb().runs[0].state, RunState::Finalized);
    env.restart();
    assert_eq!(
        env.sb().runs[0].state,
        RunState::Finalized,
        "recovery replayed it"
    );
}

#[test]
fn a_lane_cut_before_a_stop_is_adopted_not_cut_twice() {
    let mut env = Env::new();
    let t = env.take(7);
    let id = t.id.clone();
    // The worktree exists from a pass that stopped before the lane
    // record was written: git knows it as this repository's, on the
    // ticket's branch.
    let dir = env.worktrees.join(&id);
    std::fs::create_dir_all(&dir).unwrap();
    let branch = dispatch::git::branch_name(7, &t.source.title);
    let clone = env.data.repo_dir(PROJECT).join(".");
    env.runner.git = Box::new(FakeRepo {
        worktrees: vec![(clone, dir.clone(), branch, "origin/main".into())],
        ..FakeRepo::default()
    });
    env.step();
    let t = env.ticket(&id);
    assert_eq!(t.lanes[0].worktree, dir);
    assert!(t.active(), "{t:#?}");
    assert_eq!(env.sb().sessions_named("investigator").len(), 1);
}

#[test]
fn a_directory_in_the_way_that_is_not_the_worktree_parks_the_ticket() {
    let mut env = Env::new();
    let id = env.take(7).id;
    // An empty directory, or anything git does not know as this
    // repository's worktree on the branch: not adopted, nothing
    // launched in it.
    std::fs::create_dir_all(env.worktrees.join(&id)).unwrap();
    env.step();
    let t = env.ticket(&id);
    assert!(
        matches!(&t.state, TicketState::Parked { reason } if reason.contains("not a worktree")),
        "{t:#?}"
    );
    assert!(t.lanes.is_empty());
    assert!(
        env.sb().sessions.is_empty(),
        "nothing runs outside a worktree"
    );
}

#[test]
fn the_tree_comes_from_dispatches_own_clone_fetched_first() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.step();
    let t = env.ticket(&id);
    assert!(env.data.repo_dir(PROJECT).exists(), "the private clone");
    assert_eq!(t.lanes[0].worktree, env.worktrees.join(&id));
    assert_eq!(
        t.lanes[0].branch,
        "dispatch/7-issue-7-escape-leaves-the-field"
    );
    // The reviewer's and the investigator's tree are the same one.
    let investigator = env.sb().sessions_named("investigator")[0].clone();
    assert_eq!(investigator.cwd, t.lanes[0].worktree);
}

// --- a workspace of several repositories: the ticket's tree is the
// workspace, each lane a worktree of its own repository inside it, the
// lanes decision chooses where the work runs, and a lane's setup waits
// for its first agent.

const WORKSPACE: &str = r#"
version = 1

[project]
name = "Delta"
repo = "git@example.com:k3/delta-workspace.git"
worktrees = "{worktrees}"
space = "Dispatch · Delta"

[source]
kind = "github"
repo = "k3/delta-workspace"
label = "dispatch"
lane_hints = { "area:backend" = "backend", "area:frontend" = "frontend" }

[[lanes]]
name = "backend"
path = "delta-backend"
repo = "git@example.com:k3/delta-backend.git"
base = "main"
setup = ["uv", "sync"]

[[lanes]]
name = "frontend"
path = "delta-frontend"
repo = "git@example.com:k3/delta-frontend.git"
base = "dev"
setup = ["npm", "ci"]

[operators.investigator]
kind = "claude"

[operators.planner]
kind = "claude"

[operators.reviewer]
kind = "codex"
[operators.reviewer.review]
reviewer = "codex"
review_first = "Review {plan} for {worktree} on {branch}; write to {feedback}; else {no_feedback}"
review_round = "Again {response} {plan} {feedback} {no_feedback}"
respond = "Feedback at {feedback}; edit {plan}; answer at {response}."
respond_to_user = "{text} {plan} {response}"
handoff = "The plan at {plan} is final."
no_feedback = "No further feedback."
cap = 4

[[stages]]
name = "investigate"
operator = "investigator"
context = "root"
writes = ["notes"]
prompt = "Issue #{issue.number} in {worktree} on {branch}. Write to {notes}."

[[stages]]
name = "lanes"
gate = { kind = "human", decision = "lanes" }

[[stages]]
name = "plan"
operator = "planner"
context = "each"
writes = ["plan"]
prompt = "Plan in {worktree} ({lane}) on {branch} to {plan}."

[[stages]]
name = "review"
review = "reviewer"
context = "each"
subject = "plan"
gate = { kind = "external", check = "review-finalized" }

[policy]
slots = 2
waiting_on_me = 3
decisions = { lanes = "auto", finalize = "ask" }
"#;

fn workspace_env(labels: &[&str]) -> (Env, String) {
    let mut env = Env::new();
    let text = WORKSPACE.replace("{worktrees}", &env.worktrees.display().to_string());
    std::fs::write(env.data.pipeline("Delta"), text).unwrap();
    let now = env.tick();
    let id = env
        .runner
        .take(
            "Delta",
            &std::fs::read_to_string(env.data.pipeline("Delta")).unwrap(),
            SourceSnapshot {
                kind: "github".into(),
                identity: "k3/delta-workspace#42".into(),
                number: Some(42),
                title: "Asset report column missing".into(),
                body: String::new(),
                url: None,
                labels: labels.iter().map(|l| (*l).to_owned()).collect(),
                taken_at_ms: now,
            },
            now,
        )
        .unwrap()
        .id;
    (env, id)
}

#[test]
fn a_workspace_ticket_gets_the_workspace_tree_with_every_lane_inside_it() {
    let (mut env, id) = workspace_env(&["area:backend", "type:bug"]);
    let now = env.tick();
    env.runner.step_project("Delta", now).unwrap();
    let t = env.ticket(&id);
    let tree = env.worktrees.join(&id);
    assert_eq!(t.tree.as_deref(), Some(tree.as_path()), "{t:#?}");
    assert_eq!(t.lanes.len(), 2);
    assert_eq!(t.lanes[0].worktree, tree.join("delta-backend"));
    assert_eq!(t.lanes[1].worktree, tree.join("delta-frontend"));
    assert!(
        t.lanes
            .iter()
            .all(|l| l.branch == "dispatch/42-asset-report-column-missing")
    );
    assert!(
        t.lanes.iter().all(|l| !l.setup_done),
        "setups wait for an agent"
    );
    for name in ["Delta", "Delta@backend", "Delta@frontend"] {
        assert!(env.data.repo_dir(name).exists(), "clone {name}");
    }
    let sb = env.sb();
    assert_eq!(sb.projects.len(), 1);
    assert_eq!(sb.projects[0].name, "#42 Asset report column missing");
    assert_eq!(sb.projects[0].root, tree);
    let inv = &sb.sessions_named("investigator")[0];
    assert_eq!(inv.cwd, tree, "investigate runs in the workspace tree");
    let prompt = sb
        .calls
        .iter()
        .find_map(|r| match &r.body {
            Body::SessionNew { prompt, .. } => prompt.clone(),
            _ => None,
        })
        .unwrap();
    assert!(prompt.contains(&tree.display().to_string()), "{prompt}");
    assert!(prompt.contains("on dispatch/42-"), "{prompt}");
}

#[test]
fn the_label_hints_choose_the_lanes_and_only_those_get_a_planner() {
    let (mut env, id) = workspace_env(&["area:backend"]);
    let now = env.tick();
    env.runner.step_project("Delta", now).unwrap();
    let t = env.ticket(&id);
    let investigator = session_of(&t, "investigate");
    std::fs::write(artifact_of(&t, "investigate", "notes"), "# notes").unwrap();
    let now = env.now;
    env.sb().stop(&investigator, now);
    for _ in 0..8 {
        let now = env.tick();
        env.runner.step_project("Delta", now).unwrap();
        if env.ticket(&id).attempts_of("plan").next().is_some() {
            break;
        }
    }
    let t = env.ticket(&id);
    assert!(
        t.pending_decisions().is_empty(),
        "auto with a hint asks nothing"
    );
    let chosen: Vec<&str> = t
        .lanes
        .iter()
        .filter(|l| l.chosen)
        .map(|l| l.name.as_str())
        .collect();
    assert_eq!(chosen, ["backend"]);
    let plans: Vec<&dispatch::ticket::Attempt> = t.attempts_of("plan").collect();
    assert_eq!(plans.len(), 1, "{t:#?}");
    assert_eq!(plans[0].context, "backend");
    let sb = env.sb();
    let planner = &sb.sessions_named("planner")[0];
    assert_eq!(planner.cwd, t.lanes[0].worktree);
    assert_eq!(
        planner.project, sb.projects[0].id,
        "one project, the lane's cwd"
    );
    // The backend setup ran once, in the lane, before its planner; the
    // frontend's never did.
    assert!(t.lanes[0].setup_done && !t.lanes[1].setup_done);
}

#[test]
fn without_a_hint_the_lanes_are_asked_and_the_answer_chooses() {
    let (mut env, id) = workspace_env(&["type:bug"]);
    let now = env.tick();
    env.runner.step_project("Delta", now).unwrap();
    let t = env.ticket(&id);
    let investigator = session_of(&t, "investigate");
    std::fs::write(artifact_of(&t, "investigate", "notes"), "# notes").unwrap();
    let now = env.now;
    env.sb().stop(&investigator, now);
    for _ in 0..5 {
        let now = env.tick();
        env.runner.step_project("Delta", now).unwrap();
    }
    let pending = env.pending(&id);
    assert_eq!(pending.len(), 1, "{pending:#?}");
    assert_eq!(pending[0].name, "lanes");
    assert!(pending[0].question.contains("backend, frontend"));
    let now = env.tick();
    env.runner
        .decide(&id, &pending[0].id, "backend, frontend", None, now)
        .unwrap();
    for _ in 0..4 {
        let now = env.tick();
        env.runner.step_project("Delta", now).unwrap();
    }
    let t = env.ticket(&id);
    assert!(t.lanes.iter().all(|l| l.chosen));
    assert_eq!(
        t.attempts_of("plan").count(),
        2,
        "a planner per chosen lane: {t:#?}"
    );
}
