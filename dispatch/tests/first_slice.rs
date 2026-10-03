//! Dispatch end to end against a Switchboard in memory: an issue becomes
//! a ticket, a worktree, one project and its sessions, and goes through
//! agent, workflow, review and gate stages to its close; and every way
//! the path can be cut short (a lost reply, a removed record, a launch
//! the app died in, an agent that never wrote) ends as a decision, never
//! a second launch.

// Tests assert emptiness with `assert!` throughout.
#![allow(clippy::assert_is_empty)]

mod support;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use dispatch::git::FakeRepo;
use dispatch::github::{Checks, FakePullRequests, PullRequest};
use dispatch::scheduler::{PR_ERROR_GRACE_MS, PR_POLL_MS, PR_YOUNG_HEAD_MS, Runner};
use dispatch::store::DataDir;
use dispatch::ticket::{
    Attempt, AttemptKind, AttemptState, Decision, ReviewerResult, RoundState, SETTLE_POLLS,
    STOP_IDLE_POLLS, SourceSnapshot, Ticket, TicketState,
};
use support::{FakeSwitchboard, SharedPort};
use switchboard_control::{Body, Liveness, RunState, SessionKind};

const PROJECT: &str = "Switchboard";

fn pipeline(worktrees: &std::path::Path) -> String {
    format!(
        r#"
version = 1

[project]
name = "Switchboard"
repo = "git@github.com:msull/switchboard.git"
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
guidance = "Read CLAUDE.md first; the branch is {{branch}}."

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

[operators.rebaser]
kind = "claude"
guidance = "Rebase carefully on {{branch}}."

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

[[stages]]
name = "inspect"
context = "each"
gate = {{ kind = "human", decision = "inspect" }}

[[stages]]
name = "ready"
context = "each"
gate = {{ kind = "external", check = "pr-checks" }}

[[stages]]
name = "merge"
context = "each"
gate = {{ kind = "external", check = "pr-merged", decision = "merge" }}

[policy]
slots = 2
waiting_on_me = 3
rebaser = "rebaser"
decisions = {{ lanes = "auto", finalize = "ask" }}
"#,
        worktrees = worktrees.display()
    )
}

struct Env {
    _dir: tempfile::TempDir,
    data: DataDir,
    sb: Arc<Mutex<FakeSwitchboard>>,
    repo: Arc<Mutex<FakeRepo>>,
    prs: Arc<Mutex<FakePullRequests>>,
    bitbucket: Arc<Mutex<FakePullRequests>>,
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
        data.write_settings(&dispatch::store::Settings {
            worktrees: Some(worktrees.clone()),
        })
        .unwrap();
        std::fs::write(data.pipeline(PROJECT), pipeline(&worktrees)).unwrap();
        let sb = Arc::new(Mutex::new(FakeSwitchboard::new()));
        let repo = Arc::new(Mutex::new(FakeRepo::default()));
        let prs = Arc::new(Mutex::new(FakePullRequests::default()));
        let bitbucket = Arc::new(Mutex::new(FakePullRequests::default()));
        let mut runner = Runner::new(
            data.clone(),
            Box::new(SharedPort(Arc::clone(&sb))),
            Box::new(Arc::clone(&repo)),
        );
        runner.prs = Box::new(Arc::clone(&prs));
        runner.bitbucket = Box::new(Arc::clone(&bitbucket));
        Self {
            _dir: dir,
            data,
            sb,
            repo,
            prs,
            bitbucket,
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
            Box::new(Arc::clone(&self.repo)),
        );
        self.runner.prs = Box::new(Arc::clone(&self.prs));
        self.runner.bitbucket = Box::new(Arc::clone(&self.bitbucket));
        let now = self.tick();
        self.runner.recover(now).unwrap();
    }

    fn tick(&mut self) -> u64 {
        self.now += 1_000;
        self.now
    }

    /// Time passes without a pass.
    fn wait(&mut self, ms: u64) {
        self.now += ms;
    }

    /// The pull request for the ticket's one lane, as the provider
    /// will report it from now on.
    fn pr_is(&mut self, id: &str, head: &str, state: &str, checks: Checks) {
        self.pr_is_with(id, head, state, checks, None);
    }

    /// The same, with what the provider says about merging it.
    fn pr_is_with(
        &mut self,
        id: &str,
        head: &str,
        state: &str,
        checks: Checks,
        mergeable: Option<&str>,
    ) {
        let t = self.ticket(id);
        let branch = t.lanes[0].branch.clone();
        let mut prs = self.prs.lock().unwrap();
        prs.prs.clear();
        prs.checks.clear();
        prs.prs.push((
            "msull/switchboard".into(),
            branch,
            PullRequest {
                number: 7,
                url: "https://github.com/msull/switchboard/pull/7".into(),
                head: head.into(),
                state: state.into(),
                mergeable: mergeable.map(str::to_owned),
                branch: String::new(),
                base: String::new(),
                title: String::new(),
            },
        ));
        prs.checks.push(("msull/switchboard".into(), 7, checks));
    }

    /// The ticket at `merge` with its PR open and clean.
    fn at_merge(&mut self, id: &str) {
        self.pr_is(id, "base0000", "open", Checks::Passed);
        self.recheck(id);
        self.steps_until(id, "the merge decision", |t, _| {
            t.pending_decisions().iter().any(|d| d.name == "merge")
        });
    }

    /// The pending `inspect` decision, answered.
    fn inspect(&mut self, id: &str, answer: &str, note: Option<&str>) {
        self.steps_until(id, "the inspect question", |t, _| {
            t.pending_decisions().iter().any(|d| d.name == "inspect")
        });
        let d = self
            .pending(id)
            .into_iter()
            .find(|d| d.name == "inspect")
            .unwrap();
        let now = self.tick();
        self.runner.decide(id, &d.id, answer, note, now).unwrap();
    }

    /// The pending `pr` decision answered with `recheck`.
    fn recheck(&mut self, id: &str) {
        let pending = self.pending(id);
        assert_eq!(pending.len(), 1, "one pr decision: {pending:?}");
        assert_eq!(pending[0].name, "pr");
        assert_eq!(pending[0].options, vec!["recheck", "park"]);
        let now = self.tick();
        self.runner
            .decide(id, &pending[0].id, "recheck", None, now)
            .unwrap();
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
                    pull_requests: Vec::new(),
                },
                now,
            )
            .unwrap()
    }

    /// One runner tick over every project, so a test's tickets advance
    /// whichever project they belong to.
    fn step(&mut self) {
        let now = self.tick();
        self.runner.step_all(now).unwrap();
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
        for _ in 0..SETTLE_POLLS {
            self.step();
        }
    }

    /// Passes enough for a stopped agent idle without its artifact to
    /// be given up on.
    fn idle_past_grace(&mut self) {
        for _ in 0..STOP_IDLE_POLLS {
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
    assert!(env.data.repo_dir(PROJECT).exists(), "the private clone");
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
        assert!(
            prompt.starts_with(&format!(
                "Read CLAUDE.md first; the branch is {}.",
                t.lanes[0].branch
            )),
            "guidance is rendered too: {prompt}"
        );
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
    // stage complete.
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
    // The finalized copy is the plan the implementer is told to follow,
    // in the lane's worktree.
    env.steps_until(&id, "the implementer", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(Attempt::is_open)
    });
    let t = env.ticket(&id);
    let sb = env.sb();
    assert_eq!(sb.sessions.len(), 5);
    let implementer = &sb.sessions_named("implementer")[0];
    assert_eq!(implementer.cwd, t.lanes[0].worktree);
    let prompt = sb
        .calls
        .iter()
        .find_map(|r| match &r.body {
            Body::SessionNew { prompt, name, .. } if name == "implementer" => prompt.clone(),
            _ => None,
        })
        .unwrap();
    assert!(prompt.contains(&copy.display().to_string()), "{prompt}");
}

/// Drive a ticket to the implementer running in the lane, returning the
/// ticket id and the implementer's session.
fn at_implement(env: &mut Env) -> (String, String) {
    let id = at_finalize(env);
    let decision = env.pending(&id)[0].id.clone();
    let now = env.tick();
    env.runner
        .decide(&id, &decision, "finalize", None, now)
        .unwrap();
    env.steps_until(&id, "the implementer", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(Attempt::is_open)
    });
    let t = env.ticket(&id);
    (id, session_of(&t, "implement"))
}

/// The implementer stopped and wrote its notes; then the checks.
fn implementer_stops(env: &mut Env, id: &str, session: &str) {
    let t = env.ticket(id);
    env.finish(
        session,
        &artifact_of(&t, "implement", "notes"),
        "# done\nchanged two files",
    );
}

#[test]
fn checks_run_after_the_agent_on_a_clean_tree_and_pass_bound_to_its_head() {
    let mut env = Env::new();
    let (id, implementer) = at_implement(&mut env);
    implementer_stops(&mut env, &id, &implementer);
    env.steps_until(&id, "the checks starting", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.gate.is_some())
    });
    let t = env.ticket(&id);
    let a = t.attempts_of("implement").last().unwrap();
    assert!(a.is_open(), "the attempt is not complete on a stop alone");
    let lane = t.lanes[0].clone();
    {
        let repo = env.repo.lock().unwrap();
        let check = &repo.checks[0];
        assert_eq!(check.dir, lane.worktree);
        assert_eq!(check.argv, vec!["sh", "-c", "cargo test"]);
        assert!(
            check
                .env
                .contains(&("DISPATCH_LANE".to_owned(), "repo".to_owned()))
        );
        assert!(
            check
                .env
                .contains(&("DISPATCH_BRANCH".to_owned(), lane.branch.clone()))
        );
        assert!(
            check
                .env
                .contains(&("DISPATCH_HEAD".to_owned(), "base0000".to_owned()))
        );
        assert_eq!(check.log, a.artifacts["checks"]);
    }
    assert!(env.sb().killed.contains(&implementer), "the agent is done");
    let key = format!("{id}/implement/{}", a.n);
    // Still running: nothing changes, nothing is launched.
    env.step();
    env.step();
    assert!(
        env.ticket(&id)
            .attempts_of("implement")
            .last()
            .unwrap()
            .is_open()
    );
    assert_eq!(env.sb().sessions.len(), 5);
    env.repo.lock().unwrap().check_exits.insert(key, 0);
    env.steps_until(&id, "the attempt completing", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.state == AttemptState::Complete)
    });
    let t = env.ticket(&id);
    let a = t.attempts_of("implement").last().unwrap();
    assert_eq!(
        a.head.as_deref(),
        Some("base0000"),
        "the result is bound to the head"
    );
    assert_eq!(a.gate.as_ref().and_then(|g| g.exit), Some(0));
    env.pr_is(&id, "base0000", "open", Checks::Passed);
    env.inspect(&id, "proceed", None);
    env.steps_until(&id, "the merge decision", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "merge")
    });
    env.pr_is(&id, "base0000", "merged", Checks::Passed);
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the ticket closing", |t, _| !t.active());
    let t = env.ticket(&id);
    assert!(matches!(&t.state, TicketState::Closed { .. }));
    // The merged ticket's tree goes; its branch is never touched.
    let tree = t.tree.clone().unwrap();
    assert!(t.close.tree_removed && !tree.exists());
    let repo = env.repo.lock().unwrap();
    assert_eq!(repo.removed, [(env.data.repo_dir(PROJECT), tree)]);
    assert_eq!(repo.worktrees.len(), 0, "only the worktree is forgotten");
}

/// The user's own feedback round after convergence takes the finalize
/// question away while the planner answers, and it comes back, with
/// the new round count, once the run converges again.
#[test]
fn a_users_round_after_convergence_withdraws_finalize_until_it_converges_again() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let first = env.pending(&id)[0].clone();
    assert_eq!(first.name, "finalize");
    let session = env.ticket(&id).current_session().cloned().unwrap();
    assert!(env.sb().session(&session).waiting);
    {
        let mut sb = env.sb();
        sb.runs[0].state = RunState::AwaitingResponse;
        sb.runs[0].round = 2;
    }
    env.step();
    assert!(env.pending(&id).is_empty(), "the question is withdrawn");
    assert!(
        !env.sb().session(&session).waiting,
        "the session no longer waits on it"
    );
    assert!(
        env.ticket(&id)
            .attempts_of("review")
            .last()
            .unwrap()
            .is_open()
    );
    env.sb().runs[0].state = RunState::Converged;
    env.step();
    let again = env.pending(&id);
    assert_eq!(again.len(), 1);
    assert_ne!(again[0].id, first.id, "a new decision");
    assert!(
        again[0].question.contains("converged after 2 round(s)"),
        "{}",
        again[0].question
    );
}

/// `implement` done with the checks green: the ticket stands at `ready`.
fn at_ready(env: &mut Env) -> String {
    let (id, implementer) = at_implement(env);
    implementer_stops(env, &id, &implementer);
    env.steps_until(&id, "the checks starting", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.gate.is_some())
    });
    let key = format!("{id}/implement/1");
    env.repo.lock().unwrap().check_exits.insert(key, 0);
    env.inspect(&id, "proceed", None);
    env.steps_until(&id, "the ready attempt", |t, _| {
        t.attempts_of("ready").next().is_some()
    });
    id
}

/// `implement` done with the checks green and a summary for the tree,
/// stepped to the `inspect` question.
fn at_inspect(env: &mut Env) -> String {
    let (id, implementer) = at_implement(env);
    let tree = env.ticket(&id).lanes[0].worktree.clone();
    env.repo.lock().unwrap().summaries.insert(
        tree,
        "abc1234 Escape leaves the field\n src/ui/set.rs | 4 +-".into(),
    );
    implementer_stops(env, &id, &implementer);
    env.steps_until(&id, "the checks starting", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.gate.is_some())
    });
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(format!("{id}/implement/1"), 0);
    env.steps_until(&id, "the inspect question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "inspect")
    });
    id
}

/// The `inspect` stage asks once per lane with the branch, what it
/// adds and where to look, on a gate-only attempt.
#[test]
fn an_inspect_stage_shows_the_work_and_asks() {
    let mut env = Env::new();
    let id = at_inspect(&mut env);
    let t = env.ticket(&id);
    let tree = t.lanes[0].worktree.clone();
    let d = env
        .pending(&id)
        .into_iter()
        .find(|d| d.name == "inspect")
        .unwrap();
    assert_eq!(d.options, vec!["proceed", "rerun", "park"]);
    assert!(d.question.contains(&t.lanes[0].branch), "{}", d.question);
    assert!(
        d.question.contains("base0000 over origin/main"),
        "{}",
        d.question
    );
    assert!(
        d.question.contains("Escape leaves the field"),
        "{}",
        d.question
    );
    assert!(
        d.question.contains(&tree.display().to_string()),
        "{}",
        d.question
    );
    let notes = artifact_of(&t, "implement", "notes");
    assert!(
        d.question
            .contains(&format!("Notes (implement): {}", notes.display())),
        "{}",
        d.question
    );
    assert!(
        t.attempts_of("inspect")
            .last()
            .is_some_and(Attempt::is_open),
        "a gate-only attempt, open while asked"
    );
    assert_eq!(env.sb().sessions_named("implementer").len(), 1);
    env.inspect(&id, "proceed", None);
    env.steps_until(&id, "past inspect", |t, _| {
        t.attempts_of("inspect")
            .last()
            .is_some_and(|a| a.state == AttemptState::Complete)
    });
    assert_eq!(
        env.ticket(&id)
            .attempts_of("inspect")
            .last()
            .unwrap()
            .head
            .as_deref(),
        Some("base0000")
    );
}

/// `rerun` with a note at `inspect` sends that lane back to `implement`:
/// a fresh implementer with the note in its prompt, on the same branch,
/// and `inspect` asks again on a new attempt when it is done.
#[test]
fn a_note_at_inspect_sends_the_lane_back_to_implement() {
    let mut env = Env::new();
    let id = at_inspect(&mut env);
    env.inspect(&id, "rerun", Some("use a set, not a vec"));
    env.steps_until(&id, "a second implementer", |t, _| {
        t.attempts_of("implement").count() == 2
    });
    let t = env.ticket(&id);
    assert_eq!(t.stage, 4, "back at implement");
    assert!(matches!(
        t.attempts_of("implement").next().unwrap().state,
        AttemptState::Cancelled { .. }
    ));
    assert!(matches!(
        t.attempts_of("inspect").next().unwrap().state,
        AttemptState::Cancelled { .. }
    ));
    let prompt = last_prompt_of(&env, "implementer");
    assert!(
        prompt.ends_with("sent it back: use a set, not a vec"),
        "{prompt}"
    );
    assert!(t.rework.is_empty(), "the note was taken");
    // The second implementer finishes; inspect asks again, on a new attempt.
    let second = session_of(&t, "implement");
    implementer_stops(&mut env, &id, &second);
    env.steps_until(&id, "the checks starting", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.gate.is_some())
    });
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(format!("{id}/implement/2"), 0);
    env.steps_until(&id, "inspect again", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "inspect")
    });
    let t = env.ticket(&id);
    assert_eq!(t.attempts_of("inspect").count(), 2);
    env.inspect(&id, "proceed", None);
    env.steps_until(&id, "past inspect", |t, _| {
        t.attempts_of("inspect")
            .last()
            .is_some_and(|a| a.state == AttemptState::Complete)
    });
    assert_eq!(
        env.ticket(&id)
            .attempts_of("inspect")
            .last()
            .unwrap()
            .head
            .as_deref(),
        Some("base0000")
    );
}

/// A PR that conflicts with its base at `merge` gets the policy's
/// rebaser: a session cloned from the lane's implementer, told the PR,
/// the base and where the notes go; when it stops, the gate reads the
/// PR again and the merge goes on.
#[test]
fn a_conflicting_pr_is_rebased_by_a_clone_of_the_implementer() {
    let mut env = Env::new();
    let id = at_ready(&mut env);
    env.at_merge(&id);
    let implementer = session_of(&env.ticket(&id), "implement");
    env.pr_is_with(&id, "base0000", "open", Checks::Passed, Some("conflicting"));
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the rebaser", |t, _| {
        t.attempts_of("merge")
            .any(|a| a.kind == AttemptKind::Agent && a.session.is_some())
    });
    let t = env.ticket(&id);
    let rebase = t
        .attempts_of("merge")
        .find(|a| a.kind == AttemptKind::Agent)
        .unwrap()
        .clone();
    assert!(rebase.is_open());
    assert_eq!(rebase.context, "repo");
    assert_eq!(
        rebase.pr.as_ref().map(|p| p.head.as_str()),
        Some("base0000"),
        "the conflicting head is on the attempt"
    );
    let rebaser = rebase.session.clone().unwrap();
    assert_eq!(
        env.sb().cloned,
        vec![(implementer.clone(), rebaser.clone())],
        "cloned from the implementer"
    );
    let prompt = env
        .sb()
        .calls
        .iter()
        .rev()
        .find_map(|r| match &r.body {
            Body::SessionClone { prompt, name, .. } if name == "rebaser" => Some(prompt.clone()),
            _ => None,
        })
        .unwrap();
    assert!(
        prompt.starts_with("Rebase carefully on dispatch/"),
        "{prompt}"
    );
    assert!(
        prompt.contains("PR #7 (https://github.com/msull/switchboard/pull/7)"),
        "{prompt}"
    );
    assert!(prompt.contains("conflicts with origin/main"), "{prompt}");
    assert!(prompt.contains("--force-with-lease"), "{prompt}");
    let notes = rebase.artifacts["notes"].clone();
    assert!(prompt.contains(&notes.display().to_string()), "{prompt}");
    assert!(
        t.pending_decisions().iter().any(|d| d.name == "merge"),
        "the merge decision stays while the rebaser works"
    );
    // The rebaser pushed a new head and stopped.
    env.repo
        .lock()
        .unwrap()
        .heads
        .insert(t.lanes[0].worktree.clone(), "rebased1".into());
    env.pr_is_with(&id, "rebased1", "open", Checks::Passed, Some("clean"));
    env.finish(&rebaser, &notes, "# rebased\nkept both changelog entries");
    env.steps_until(&id, "the rebase completing", |t, _| {
        t.attempts_of("merge")
            .find(|a| a.kind == AttemptKind::Agent)
            .is_some_and(|a| a.state == AttemptState::Complete)
    });
    assert!(env.sb().killed.contains(&rebaser), "the rebaser is done");
    env.wait(PR_POLL_MS);
    env.step();
    let t = env.ticket(&id);
    let gate = t
        .attempts_of("merge")
        .find(|a| a.kind == AttemptKind::GateOnly)
        .unwrap();
    assert!(gate.is_open(), "the same gate attempt watches on");
    assert_eq!(gate.pr.as_ref().map(|p| p.head.as_str()), Some("rebased1"));
    env.pr_is_with(&id, "rebased1", "merged", Checks::Passed, None);
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the ticket closing", |t, _| !t.active());
    assert!(matches!(&env.ticket(&id).state, TicketState::Closed { .. }));
}

/// A rebaser that could not start (no transcript to clone) is a rerun
/// question, counts as nothing, and runs once the answer is given.
#[test]
fn a_rebaser_that_could_not_start_is_rerun_on_request() {
    let mut env = Env::new();
    let id = at_ready(&mut env);
    env.at_merge(&id);
    let implementer = session_of(&env.ticket(&id), "implement");
    env.sb().resumable.retain(|s| s != &implementer);
    env.pr_is_with(&id, "base0000", "open", Checks::Passed, Some("conflicting"));
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the rerun question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    let d = env
        .pending(&id)
        .into_iter()
        .find(|d| d.name == "rerun")
        .unwrap();
    assert!(d.question.contains("no transcript"), "{}", d.question);
    env.wait(PR_POLL_MS);
    env.step();
    assert_eq!(
        env.ticket(&id)
            .attempts_of("merge")
            .filter(|a| a.kind == AttemptKind::Agent)
            .count(),
        1,
        "nothing more until the answer"
    );
    env.sb().resumable.push(implementer.clone());
    let now = env.tick();
    env.runner.decide(&id, &d.id, "rerun", None, now).unwrap();
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the rebaser", |t, _| {
        t.attempts_of("merge")
            .any(|a| a.kind == AttemptKind::Agent && a.session.is_some())
    });
    assert_eq!(env.sb().cloned.len(), 1);
}

/// A rebase that leaves the PR at the same head, and a policy whose
/// `max_rebases` is spent, are each a `pr` question, not another run.
#[test]
fn a_rebase_that_changes_nothing_or_past_the_cap_is_a_question() {
    let mut env = Env::new();
    let id = at_ready(&mut env);
    env.at_merge(&id);
    env.pr_is_with(&id, "base0000", "open", Checks::Passed, Some("conflicting"));
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the rebaser", |t, _| {
        t.attempts_of("merge")
            .any(|a| a.kind == AttemptKind::Agent && a.session.is_some())
    });
    let t = env.ticket(&id);
    let rebaser = session_of(&t, "merge");
    let notes = artifact_of(&t, "merge", "notes");
    env.finish(&rebaser, &notes, "# could not");
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "pr")
    });
    let d = env
        .pending(&id)
        .into_iter()
        .find(|d| d.name == "pr")
        .unwrap();
    assert!(
        d.question.contains("still at the same head"),
        "{}",
        d.question
    );
    assert_eq!(
        env.ticket(&id)
            .attempts_of("merge")
            .filter(|a| a.kind == AttemptKind::Agent)
            .count(),
        1
    );
    // The cap: no rebaser runs when it is spent.
    let mut env = Env::new();
    let path = env.data.pipeline(PROJECT);
    let text = std::fs::read_to_string(&path).unwrap().replace(
        "rebaser = \"rebaser\"\n",
        "rebaser = \"rebaser\"\nmax_rebases = 0\n",
    );
    std::fs::write(&path, text).unwrap();
    let id = at_ready(&mut env);
    env.at_merge(&id);
    env.pr_is_with(&id, "base0000", "open", Checks::Passed, Some("conflicting"));
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "pr")
    });
    let d = env
        .pending(&id)
        .into_iter()
        .find(|d| d.name == "pr")
        .unwrap();
    assert!(
        d.question.contains("max_rebases of 0 is spent"),
        "{}",
        d.question
    );
    assert!(env.sb().cloned.is_empty(), "no rebaser");
}

/// The test pipeline with a `fixer` operator in the policy, plus
/// `extra` policy lines.
fn with_fixer(env: &Env, extra: &str) {
    let path = env.data.pipeline(PROJECT);
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace(
            "[operators.rebaser]\n",
            "[operators.fixer]\nkind = \"claude\"\nguidance = \"Fix it properly.\"\n\n[operators.rebaser]\n",
        )
        .replace(
            "rebaser = \"rebaser\"\n",
            &format!("rebaser = \"rebaser\"\nfixer = \"fixer\"\n{extra}"),
        );
    std::fs::write(&path, text).unwrap();
}

/// Red checks on the PR at the tree's head get the policy's fixer: a
/// session cloned from the lane's implementer, told the PR and the
/// failed checks; when it pushes and stops, the gate reads again and
/// green checks pass it. Without a fixer, red checks stay a question
/// (`ready_asks_about_red_moved_or_missing_checks_and_none_can_be_expected`).
#[test]
fn red_checks_are_fixed_by_a_clone_of_the_implementer() {
    let mut env = Env::new();
    with_fixer(&env, "");
    let id = at_ready(&mut env);
    let implementer = session_of(&env.ticket(&id), "implement");
    env.pr_is(&id, "base0000", "open", Checks::Failed(vec!["test".into()]));
    env.recheck(&id);
    env.steps_until(&id, "the fixer", |t, _| {
        t.attempts_of("ready")
            .any(|a| a.kind == AttemptKind::Agent && a.session.is_some())
    });
    let t = env.ticket(&id);
    assert!(
        t.pending_decisions().is_empty(),
        "no question while the fixer works"
    );
    let fix = t
        .attempts_of("ready")
        .find(|a| a.kind == AttemptKind::Agent)
        .unwrap()
        .clone();
    assert_eq!(
        fix.pr.as_ref().map(|p| p.checks.as_str()),
        Some("failed: test"),
        "the failure is on the attempt"
    );
    let fixer = fix.session.clone().unwrap();
    assert_eq!(env.sb().cloned, vec![(implementer, fixer.clone())]);
    let prompt = env
        .sb()
        .calls
        .iter()
        .rev()
        .find_map(|r| match &r.body {
            Body::SessionClone { prompt, name, .. } if name == "fixer" => Some(prompt.clone()),
            _ => None,
        })
        .unwrap();
    assert!(prompt.starts_with("Fix it properly."), "{prompt}");
    assert!(prompt.contains("has failing checks: test"), "{prompt}");
    assert!(prompt.contains("gh pr checks 7"), "{prompt}");
    let notes = fix.artifacts["notes"].clone();
    assert!(prompt.contains(&notes.display().to_string()), "{prompt}");
    // The fixer pushed a new head and stopped; the checks come back green.
    env.repo
        .lock()
        .unwrap()
        .heads
        .insert(t.lanes[0].worktree.clone(), "fixed001".into());
    env.pr_is(&id, "fixed001", "open", Checks::Passed);
    env.finish(&fixer, &notes, "# fixed\nthe fixture needed the endpoint");
    env.steps_until(&id, "the fix completing", |t, _| {
        t.attempts_of("ready")
            .find(|a| a.kind == AttemptKind::Agent)
            .is_some_and(|a| a.state == AttemptState::Complete)
    });
    assert!(env.sb().killed.contains(&fixer), "the fixer is done");
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "ready passing", |t, _| {
        t.attempts_of("ready")
            .any(|a| a.kind == AttemptKind::GateOnly && a.state == AttemptState::Complete)
    });
}

/// A spent `max_fixes` makes red checks a `pr` question, not a run.
#[test]
fn red_checks_past_the_fix_cap_are_a_question() {
    let mut env2 = Env::new();
    with_fixer(&env2, "max_fixes = 0\n");
    let id2 = at_ready(&mut env2);
    env2.pr_is(
        &id2,
        "base0000",
        "open",
        Checks::Failed(vec!["test".into()]),
    );
    env2.recheck(&id2);
    env2.steps_until(&id2, "the question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "pr")
    });
    let d = env2
        .pending(&id2)
        .into_iter()
        .find(|d| d.name == "pr")
        .unwrap();
    assert!(
        d.question
            .contains("checks failed: test; the policy's max_fixes of 0 is spent"),
        "{}",
        d.question
    );
    assert!(env2.sb().cloned.is_empty(), "no fixer");
}

/// `merge` is a confirmation the provider resolves: the decision has no
/// answer but park, `merged` by hand is refused, and the PR reading as
/// merged completes the stage with the decision answered by Dispatch.
#[test]
fn merge_waits_for_the_provider_and_refuses_a_hand_answer() {
    let mut env = Env::new();
    let id = at_ready(&mut env);
    env.pr_is(&id, "base0000", "open", Checks::Passed);
    env.recheck(&id);
    env.steps_until(&id, "the merge decision", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "merge")
    });
    let d = env
        .pending(&id)
        .into_iter()
        .find(|d| d.name == "merge")
        .unwrap();
    assert_eq!(d.options, vec!["park"]);
    assert!(
        d.question
            .contains("PR #7 https://github.com/msull/switchboard/pull/7 is open"),
        "{}",
        d.question
    );
    let now = env.tick();
    let err = env
        .runner
        .decide(&id, &d.id, "merged", None, now)
        .unwrap_err()
        .to_string();
    assert!(err.contains("takes one of: park"), "{err}");
    let session = env.ticket(&id).current_session().cloned().unwrap();
    assert!(
        env.sb().session(&session).waiting,
        "the session waits on the merge"
    );
    env.pr_is(&id, "base0000", "merged", Checks::Passed);
    env.step();
    assert!(env.ticket(&id).active(), "not read again within the minute");
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the ticket closing", |t, _| !t.active());
    let t = env.ticket(&id);
    assert!(matches!(&t.state, TicketState::Closed { .. }));
    let d = t.decisions.iter().find(|d| d.name == "merge").unwrap();
    assert!(
        matches!(&d.state, dispatch::ticket::DecisionState::Answered { answer, by, acted: true, .. } if answer == "merged" && by == "dispatch"),
        "{d:?}"
    );
    assert_eq!(
        t.attempts_of("merge")
            .last()
            .unwrap()
            .pr
            .as_ref()
            .map(|p| p.checks.as_str()),
        Some("merged")
    );
}

/// The `ready` stage finds the lane's PR, waits while its checks are
/// pending, reads the provider at most once a minute, and completes
/// bound to the tree's head once they pass.
#[test]
fn ready_waits_for_the_prs_checks_and_passes_green_at_the_trees_head() {
    let mut env = Env::new();
    let id = at_ready(&mut env);
    // No PR yet: a question, not a failure, and the attempt stays open.
    let t = env.ticket(&id);
    let a = t.attempts_of("ready").last().unwrap();
    assert!(a.is_open());
    assert_eq!(a.context, "repo");
    let pending = env.pending(&id);
    assert_eq!(pending.len(), 1);
    assert!(
        pending[0].question.contains("no pull request for branch"),
        "{}",
        pending[0].question
    );
    env.step();
    assert_eq!(env.prs.lock().unwrap().looked, 1, "held by the decision");
    env.pr_is(&id, "base0000", "open", Checks::Pending);
    env.recheck(&id);
    env.step();
    let t = env.ticket(&id);
    let a = t.attempts_of("ready").last().unwrap();
    assert!(a.is_open());
    let pr = a.pr.clone().expect("the PR is on the attempt");
    assert_eq!((pr.number, pr.checks.as_str()), (7, "pending"));
    assert_eq!(pr.url, "https://github.com/msull/switchboard/pull/7");
    assert!(env.pending(&id).is_empty(), "pending is waiting");
    let looked = env.prs.lock().unwrap().looked;
    env.step();
    env.step();
    assert_eq!(env.prs.lock().unwrap().looked, looked, "once a minute");
    env.wait(PR_POLL_MS);
    env.step();
    assert_eq!(env.prs.lock().unwrap().looked, looked + 1);
    env.pr_is(&id, "base0000", "open", Checks::Passed);
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "ready completing", |t, _| {
        t.attempts_of("ready")
            .last()
            .is_some_and(|a| a.state == AttemptState::Complete)
    });
    let t = env.ticket(&id);
    let a = t.attempts_of("ready").last().unwrap();
    assert_eq!(a.head.as_deref(), Some("base0000"));
    assert_eq!(a.pr.as_ref().map(|p| p.checks.as_str()), Some("passed"));
    env.steps_until(&id, "the merge stage", |t, _| {
        t.attempts_of("merge").next().is_some()
    });
    assert_eq!(
        env.sb().sessions_named("implementer").len(),
        1,
        "no agent for a gate-only stage"
    );
}

/// A `ready` attempt launches nothing, so it holds no slot: another
/// ticket's agent starts beside it, and it finishes beside that agent.
#[test]
fn a_ready_stage_costs_no_slot() {
    let mut env = Env::new();
    let path = env.data.pipeline(PROJECT);
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("slots = 2\n", "slots = 1\n");
    std::fs::write(&path, text).unwrap();
    let id = at_ready(&mut env);
    assert!(
        env.ticket(&id)
            .attempts_of("ready")
            .last()
            .unwrap()
            .is_open()
    );
    let other = env.take(8).id;
    env.steps_until(&other, "the other ticket's investigator", |t, _| {
        t.attempts_of("investigate")
            .next()
            .is_some_and(Attempt::is_open)
    });
    env.pr_is(&id, "base0000", "open", Checks::Passed);
    env.recheck(&id);
    env.steps_until(&id, "the merge stage", |t, _| {
        t.attempts_of("merge").next().is_some()
    });
    env.pr_is(&id, "base0000", "merged", Checks::Passed);
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the ticket closing", |t, _| !t.active());
    assert!(matches!(&env.ticket(&id).state, TicketState::Closed { .. }));
    assert!(
        env.ticket(&other)
            .attempts_of("investigate")
            .next()
            .is_some_and(Attempt::is_open),
        "the other ticket kept its slot"
    );
}

/// A full disk holds new starts: the attempt would fail for nothing
/// and cost the run. Running work is still watched, and the hold lifts
/// by itself once space is back.
#[test]
fn a_full_disk_holds_new_starts_until_space_is_back() {
    let mut env = Env::new();
    let first = env.take(7).id;
    env.steps_until(&first, "the first investigator", |t, _| {
        t.attempts_of("investigate")
            .next()
            .is_some_and(Attempt::is_open)
    });
    env.repo.lock().unwrap().free_bytes = Some(2_000_000_000);
    let second = env.take(8).id;
    for _ in 0..3 {
        env.step();
    }
    assert!(
        env.ticket(&second).attempts.is_empty(),
        "2 GB free, the default floor is 10"
    );
    let status = dispatch::serve::status(&env.runner).unwrap();
    let p = status.projects.iter().find(|p| p.name == PROJECT).unwrap();
    assert_eq!(p.free_gb, Some(2));
    assert!(
        p.held().is_some_and(|why| why.contains("2 GB free")),
        "{:?}",
        p.held()
    );
    // The first ticket's running attempt is still watched to its end.
    env.finish(
        &session_of(&env.ticket(&first), "investigate"),
        &artifact_of(&env.ticket(&first), "investigate", "notes"),
        "notes\n",
    );
    env.steps_until(&first, "the first investigator done", |t, _| {
        t.attempts_of("investigate")
            .next()
            .is_some_and(|a| !a.is_open())
    });
    assert!(env.ticket(&second).attempts.is_empty());
    env.repo.lock().unwrap().free_bytes = None;
    env.steps_until(&second, "the second investigator", |t, _| {
        t.attempts_of("investigate")
            .next()
            .is_some_and(Attempt::is_open)
    });
}

/// `slots` is read from the project's live pipeline file on every
/// pass, not from a ticket's frozen copy: a second ticket waits while
/// the file says one slot and starts as soon as the file says two.
#[test]
fn slots_come_from_the_live_pipeline_file_not_a_tickets_copy() {
    let mut env = Env::new();
    let path = env.data.pipeline(PROJECT);
    let one = std::fs::read_to_string(&path)
        .unwrap()
        .replace("slots = 2\n", "slots = 1\n");
    std::fs::write(&path, &one).unwrap();
    let first = env.take(7).id;
    env.steps_until(&first, "the first investigator", |t, _| {
        t.attempts_of("investigate")
            .next()
            .is_some_and(Attempt::is_open)
    });
    let second = env.take(8).id;
    for _ in 0..3 {
        env.step();
    }
    assert!(
        env.ticket(&second).attempts.is_empty(),
        "one slot, held by the first ticket"
    );
    std::fs::write(&path, one.replace("slots = 1\n", "slots = 2\n")).unwrap();
    env.steps_until(&second, "the second investigator", |t, _| {
        t.attempts_of("investigate")
            .next()
            .is_some_and(Attempt::is_open)
    });
    assert!(
        env.ticket(&first)
            .attempts_of("investigate")
            .next()
            .is_some_and(Attempt::is_open),
        "the first ticket kept its slot"
    );
}

/// Red checks, a PR at another head, and a repository with no checks
/// are each a question with `recheck`; a stage that says
/// `checks = "none"` passes on the PR at the head alone.
#[test]
fn ready_asks_about_red_moved_or_missing_checks_and_none_can_be_expected() {
    let mut env = Env::new();
    let id = at_ready(&mut env);
    env.pr_is(&id, "other000", "open", Checks::Passed);
    env.recheck(&id);
    env.step();
    let pending = env.pending(&id);
    assert!(
        pending[0]
            .question
            .contains("PR #7 is at other000 but the tree is at base0000"),
        "{}",
        pending[0].question
    );
    env.pr_is(&id, "base0000", "open", Checks::Failed(vec!["lint".into()]));
    env.recheck(&id);
    env.step();
    let pending = env.pending(&id);
    assert!(
        pending[0].question.contains("PR #7 checks failed: lint"),
        "{}",
        pending[0].question
    );
    env.pr_is(&id, "base0000", "open", Checks::None);
    env.recheck(&id);
    // Past the window in which a pushed head's checks may not exist yet.
    env.wait(PR_YOUNG_HEAD_MS);
    env.step();
    let pending = env.pending(&id);
    assert!(
        pending[0].question.contains("no checks configured"),
        "{}",
        pending[0].question
    );
    let t = env.ticket(&id);
    for path in [env.data.pipeline(PROJECT), t.pipeline_file.clone()] {
        let text = std::fs::read_to_string(&path).unwrap().replace(
            r#"check = "pr-checks" }"#,
            r#"check = "pr-checks", checks = "none" }"#,
        );
        std::fs::write(&path, text).unwrap();
    }
    env.recheck(&id);
    env.steps_until(&id, "ready completing", |t, _| {
        t.attempts_of("ready")
            .last()
            .is_some_and(|a| a.state == AttemptState::Complete)
    });
    let t = env.ticket(&id);
    assert_eq!(
        t.attempts_of("ready").count(),
        1,
        "the same attempt throughout"
    );
    assert_eq!(
        t.attempts_of("ready")
            .last()
            .unwrap()
            .pr
            .as_ref()
            .map(|p| p.checks.as_str()),
        Some("none")
    );
}

/// Soon after the head moved, a PR with no checks may only be one whose
/// checks GitHub has not created yet: the reading is recorded and the
/// gate waits; past the window it asks.
#[test]
fn a_none_reading_soon_after_a_push_waits_then_asks() {
    let mut env = Env::new();
    let id = at_ready(&mut env);
    env.pr_is(&id, "base0000", "open", Checks::None);
    env.recheck(&id);
    env.wait(29_000);
    env.step();
    assert!(env.pending(&id).is_empty(), "{:#?}", env.pending(&id));
    let t = env.ticket(&id);
    let ready = t.attempts_of("ready").last().unwrap();
    assert!(ready.is_open(), "{ready:?}");
    assert_eq!(
        ready.pr.as_ref().map(|p| p.checks.as_str()),
        Some("none"),
        "{ready:?}"
    );
    env.wait(PR_YOUNG_HEAD_MS);
    env.step();
    let pending = env.pending(&id);
    assert_eq!(pending.len(), 1, "{pending:#?}");
    assert_eq!(pending[0].name, "pr");
    assert!(
        pending[0].question.contains("no checks configured"),
        "{}",
        pending[0].question
    );
}

/// A Bitbucket remote is read through the Bitbucket adapter, and a
/// short head from the provider still matches the tree's.
#[test]
fn a_bitbucket_remote_is_read_through_bitbucket_and_a_short_head_matches() {
    let mut env = Env::new();
    let path = env.data.pipeline(PROJECT);
    let text = std::fs::read_to_string(&path).unwrap().replace(
        "git@github.com:msull/switchboard.git",
        "git@bitbucket.org:msull/switchboard.git",
    );
    std::fs::write(&path, text).unwrap();
    let id = at_ready(&mut env);
    assert_eq!(env.prs.lock().unwrap().looked, 0, "GitHub was not asked");
    assert_eq!(env.bitbucket.lock().unwrap().looked, 1);
    let branch = env.ticket(&id).lanes[0].branch.clone();
    {
        let mut bb = env.bitbucket.lock().unwrap();
        bb.prs.push((
            "msull/switchboard".into(),
            branch,
            PullRequest {
                number: 12,
                url: "https://bitbucket.org/msull/switchboard/pull-requests/12".into(),
                head: "base0000".chars().take(7).collect(),
                state: "open".into(),
                mergeable: None,
                branch: String::new(),
                base: String::new(),
                title: String::new(),
            },
        ));
        bb.checks
            .push(("msull/switchboard".into(), 12, Checks::Passed));
    }
    env.recheck(&id);
    env.steps_until(&id, "ready completing", |t, _| {
        t.attempts_of("ready")
            .last()
            .is_some_and(|a| a.state == AttemptState::Complete)
    });
    let t = env.ticket(&id);
    let pr = t.attempts_of("ready").last().unwrap().pr.clone().unwrap();
    assert_eq!((pr.provider.as_str(), pr.number), ("bitbucket", 12));
}

/// A provider that cannot be read is retried quietly for an hour, then
/// asked about; a merged PR passes whatever its checks say.
#[test]
fn ready_backs_off_a_failing_provider_for_an_hour_and_a_merged_pr_passes() {
    let mut env = Env::new();
    env.prs.lock().unwrap().fail = Some("api.github.com: connection refused".into());
    let id = at_ready(&mut env);
    let t = env.ticket(&id);
    let a = t.attempts_of("ready").last().unwrap();
    assert!(a.is_open());
    assert!(
        a.pr.as_ref()
            .is_some_and(|p| p.checks.starts_with("error: ")),
        "{:?}",
        a.pr
    );
    assert!(env.pending(&id).is_empty(), "no question in the first hour");
    env.wait(PR_ERROR_GRACE_MS);
    env.step();
    let pending = env.pending(&id);
    assert_eq!(pending.len(), 1);
    assert!(
        pending[0]
            .question
            .contains("could not be read for an hour"),
        "{}",
        pending[0].question
    );
    env.prs.lock().unwrap().fail = None;
    env.pr_is(&id, "elsewhere", "merged", Checks::None);
    env.recheck(&id);
    env.steps_until(&id, "ready completing", |t, _| {
        t.attempts_of("ready")
            .last()
            .is_some_and(|a| a.state == AttemptState::Complete)
    });
    let t = env.ticket(&id);
    let a = t.attempts_of("ready").last().unwrap();
    assert_eq!(a.pr.as_ref().map(|p| p.checks.as_str()), Some("merged"));
    assert_eq!(a.pr.as_ref().and_then(|p| p.error_since_ms), None);
}

#[test]
fn failed_checks_are_a_decision_and_a_rerun_is_a_fresh_agent() {
    let mut env = Env::new();
    let (id, implementer) = at_implement(&mut env);
    implementer_stops(&mut env, &id, &implementer);
    env.steps_until(&id, "the checks starting", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.gate.is_some())
    });
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(format!("{id}/implement/1"), 1);
    env.steps_until(&id, "the failure", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| matches!(a.state, AttemptState::Failed { .. }))
    });
    let t = env.ticket(&id);
    let AttemptState::Failed { reason } = &t.attempts_of("implement").last().unwrap().state else {
        panic!()
    };
    assert!(reason.contains("checks exited 1"), "{reason}");
    let pending = env.pending(&id);
    assert_eq!((pending.len(), pending[0].name.as_str()), (1, "rerun"));
    env.step();
    env.step();
    assert_eq!(env.sb().sessions.len(), 5, "no retry on its own");
    let now = env.tick();
    env.runner
        .decide(&id, &pending[0].id, "rerun", None, now)
        .unwrap();
    env.steps_until(&id, "a second implementer", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.n == 2 && a.is_open())
    });
    assert_eq!(env.sb().sessions_named("implementer").len(), 2);
}

/// Failed checks can be run again on the same attempt: the agent's
/// work stands, no agent is launched, and the gate starts over.
#[test]
fn failed_checks_are_run_again_on_the_same_attempt_without_an_agent() {
    let mut env = Env::new();
    let (id, implementer) = at_implement(&mut env);
    implementer_stops(&mut env, &id, &implementer);
    env.steps_until(&id, "the checks starting", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.gate.is_some())
    });
    let key = format!("{id}/implement/1");
    env.repo.lock().unwrap().check_exits.insert(key.clone(), 1);
    env.steps_until(&id, "the failure", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| matches!(a.state, AttemptState::Failed { .. }))
    });
    let pending = env.pending(&id);
    assert_eq!(pending[0].options, vec!["rerun", "check", "park"]);
    // The environment is fixed; the checks pass this time.
    env.repo.lock().unwrap().check_exits.insert(key, 0);
    let now = env.tick();
    env.runner
        .decide(&id, &pending[0].id, "check", None, now)
        .unwrap();
    env.steps_until(&id, "the checks passing", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.state == AttemptState::Complete)
    });
    let t = env.ticket(&id);
    assert_eq!(t.attempts_of("implement").count(), 1, "the same attempt");
    assert_eq!(
        env.sb().sessions_named("implementer").len(),
        1,
        "no new agent"
    );
    assert_eq!(
        env.repo.lock().unwrap().checks.len(),
        2,
        "the checks ran twice"
    );
}

/// A stage that keeps failing parks the ticket once the policy's
/// `max_reruns` is spent, instead of asking for another run.
#[test]
fn a_stage_failing_past_max_reruns_parks_the_ticket() {
    let mut env = Env::new();
    let (id, implementer) = at_implement(&mut env);
    let t = env.ticket(&id);
    with_max_reruns(&env, &t, 1);
    env.repo
        .lock()
        .unwrap()
        .dirty
        .push(t.lanes[0].worktree.clone());
    implementer_stops(&mut env, &id, &implementer);
    env.steps_until(&id, "the first failure", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| matches!(a.state, AttemptState::Failed { .. }))
    });
    let pending = env.pending(&id);
    assert_eq!(pending.len(), 1, "one failure: asked");
    let now = env.tick();
    env.runner
        .decide(&id, &pending[0].id, "rerun", None, now)
        .unwrap();
    env.steps_until(&id, "a second implementer", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.n == 2 && a.is_open())
    });
    let second = session_of(&env.ticket(&id), "implement");
    implementer_stops(&mut env, &id, &second);
    env.steps_until(&id, "the ticket parking", |t, _| !t.active());
    let t = env.ticket(&id);
    let (TicketState::Parked { reason } | TicketState::Parking { reason }) = &t.state else {
        panic!("{t:#?}");
    };
    assert!(reason.contains("max_reruns"), "{reason}");
    assert!(env.pending(&id).is_empty(), "nothing more is asked");
}

#[test]
fn a_dirty_tree_after_the_agent_never_runs_the_checks() {
    let mut env = Env::new();
    let (id, implementer) = at_implement(&mut env);
    let worktree = env.ticket(&id).lanes[0].worktree.clone();
    env.repo.lock().unwrap().dirty.push(worktree);
    implementer_stops(&mut env, &id, &implementer);
    env.steps_until(&id, "the failure", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| matches!(a.state, AttemptState::Failed { .. }))
    });
    let t = env.ticket(&id);
    let AttemptState::Failed { reason } = &t.attempts_of("implement").last().unwrap().state else {
        panic!()
    };
    assert!(reason.contains("not clean"), "{reason}");
    assert!(
        env.repo.lock().unwrap().checks.is_empty(),
        "nothing ran on a dirty tree"
    );
    assert_eq!(env.pending(&id)[0].name, "rerun");
}

#[test]
fn checks_lost_to_a_restart_start_again_on_the_same_head() {
    let mut env = Env::new();
    let (id, implementer) = at_implement(&mut env);
    implementer_stops(&mut env, &id, &implementer);
    env.steps_until(&id, "the checks starting", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.gate.is_some())
    });
    // The runner dies with its child; the record says the checks were
    // running at a head.
    env.repo.lock().unwrap().checks.clear();
    env.restart();
    let repo = Arc::clone(&env.repo);
    env.steps_until(&id, "the checks again", |_, _| {
        !repo.lock().unwrap().checks.is_empty()
    });
    let t = env.ticket(&id);
    let a = t.attempts_of("implement").last().unwrap();
    assert!(a.is_open() && a.gate.as_ref().is_some_and(|g| g.head == "base0000"));
    assert_eq!(env.sb().sessions.len(), 5, "no second agent");
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(format!("{id}/implement/1"), 0);
    env.steps_until(&id, "completion", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.state == AttemptState::Complete)
    });
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
    env.idle_past_grace();
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
    assert!(env.sb().runs.is_empty(), "no run was started");
}

#[test]
fn a_project_that_cannot_be_saved_makes_nothing_else() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.sb().fail_next = Some("project.add".into());
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(&t.state, TicketState::Parked { reason } if reason.contains("project.add")));
    assert!(t.attempts.is_empty(), "no attempt was made");
    assert!(env.sb().sessions.is_empty(), "no session was made");
    assert_eq!(env.sb().kinds_called("session.new"), 0);
}

/// The pull-request pipeline of the test project: two lanes, one in
/// the project's repository and one with a repository of its own, a
/// sign-off and a merge watch. No agent stages, nothing pushes.
fn pr_pipeline(worktrees: &std::path::Path) -> String {
    format!(
        r#"
version = 1

[project]
name = "Switchboard"
repo = "git@github.com:msull/switchboard.git"
worktrees = "{worktrees}"
space = "Dispatch · Switchboard"

[source]
kind = "pull-request"

[[lanes]]
name = "repo"
path = "."

[[lanes]]
name = "docs"
path = "docs"
repo = "git@github.com:msull/docs.git"

[[stages]]
name = "inspect"
context = "each"
gate = {{ kind = "human", decision = "inspect" }}

[[stages]]
name = "merge"
context = "each"
gate = {{ kind = "external", check = "pr-merged", decision = "merge" }}

[policy]
slots = 2
waiting_on_me = 5
"#,
        worktrees = worktrees.display()
    )
}

/// A pull request's base is where its branch forked from the branch
/// it targets, in each lane's own clone.
fn seed_pr_bases(env: &Env) {
    let mut repo = env.repo.lock().unwrap();
    repo.bases
        .insert(env.data.repo_dir(PROJECT), "fork0009".into());
    repo.bases
        .insert(env.data.repo_dir("Switchboard@docs"), "fork0003".into());
}

/// Each lane's base is the merge base with the PR's target branch,
/// read at the cut.
fn assert_pr_bases(t: &Ticket) {
    let bases: Vec<Option<&str>> = t.lanes.iter().map(|l| l.base_sha.as_deref()).collect();
    assert_eq!(bases, vec![Some("fork0009"), Some("fork0003")]);
}

/// One open PR the fake provider hands out, by repository and number.
fn open_pr(env: &Env, repo: &str, number: u64, branch: &str, title: &str) {
    env.prs.lock().unwrap().prs.push((
        repo.to_owned(),
        branch.to_owned(),
        PullRequest {
            number,
            url: format!("https://github.com/{repo}/pull/{number}"),
            head: format!("head{number:04}"),
            state: "open".into(),
            mergeable: Some("clean".into()),
            branch: branch.to_owned(),
            base: "main".to_owned(),
            title: title.to_owned(),
        },
    ));
}

/// A ticket from someone else's pull requests: one per lane, each
/// lane checked out on the PR's own branch tracking the remote; the
/// sign-off question comes after the branches are brought up to date;
/// proceed leads to the merge watch, and the merges close the ticket.
#[test]
fn a_ticket_from_pull_requests_checks_out_their_branches_and_watches_the_merges() {
    let mut env = Env::new();
    let worktrees = env.data.root.join("wt");
    std::fs::write(env.data.pr_pipeline(PROJECT), pr_pipeline(&worktrees)).unwrap();
    open_pr(
        &env,
        "msull/switchboard",
        9,
        "feature/escape",
        "Escape leaves the field",
    );
    open_pr(
        &env,
        "msull/docs",
        3,
        "feature/escape-docs",
        "Document escape",
    );
    seed_pr_bases(&env);
    let now = env.tick();
    let t =
        dispatch::serve::take_pull_requests(&mut env.runner, PROJECT, &["repo/9", "docs/3"], now)
            .unwrap();
    let id = t.id.clone();
    assert_eq!(t.source.kind, "pull-request");
    assert_eq!(
        t.source.identity,
        "github:msull/switchboard!9+github:msull/docs!3"
    );
    assert_eq!(t.source.title, "Escape leaves the field");
    assert_eq!(t.source.pull_requests.len(), 2);
    env.steps_until(&id, "the sign-off question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "inspect")
    });
    let t = env.ticket(&id);
    let branches: Vec<(String, String, bool)> = t
        .lanes
        .iter()
        .map(|l| (l.name.clone(), l.branch.clone(), l.chosen))
        .collect();
    assert_eq!(
        branches,
        vec![
            ("repo".to_owned(), "pr/9".to_owned(), true),
            ("docs".to_owned(), "pr/3".to_owned(), true),
        ]
    );
    assert_pr_bases(&t);
    {
        let repo = env.repo.lock().unwrap();
        let tracked: Vec<(String, String)> = repo
            .tracked
            .iter()
            .map(|(_, _, b, r)| (b.clone(), r.clone()))
            .collect();
        assert_eq!(
            tracked,
            vec![
                ("pr/9".to_owned(), "origin".to_owned()),
                ("pr/3".to_owned(), "origin".to_owned()),
            ],
            "GitHub PRs are checked out from their pull refs, tracking the remote"
        );
        let pulls: Vec<(String, u64)> = repo
            .fetched_pulls
            .iter()
            .map(|(_, r, n)| (r.clone(), *n))
            .collect();
        assert!(
            pulls.starts_with(&[("origin".to_owned(), 9), ("origin".to_owned(), 3)]),
            "{pulls:?}"
        );
        assert!(repo.worktrees.is_empty(), "no branch of Dispatch's own");
        let refreshed = repo
            .ran
            .iter()
            .filter(|(_, argv)| argv.join(" ").starts_with("git merge --ff-only origin/pr/"))
            .count();
        assert_eq!(refreshed, 2, "each branch was brought up to date first");
    }
    assert!(t.attempts.iter().all(|a| a.kind == AttemptKind::GateOnly));
    let pending = env.pending(&id);
    assert_eq!(pending.len(), 2, "one sign-off per lane");
    for d in &pending {
        let now = env.tick();
        env.runner.decide(&id, &d.id, "proceed", None, now).unwrap();
    }
    env.steps_until(&id, "the merge watch", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "merge")
    });
    let merge = env.pending(&id);
    assert!(
        merge.iter().any(|d| d.question.contains("PR #9")),
        "{merge:?}"
    );
    for (_, _, pr) in &mut env.prs.lock().unwrap().prs {
        pr.state = "merged".into();
    }
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the ticket closing", |t, _| !t.active());
    assert!(matches!(&env.ticket(&id).state, TicketState::Closed { .. }));
}

/// A pull request on a mirror: the spec names the remote, the clone
/// gains it, the lane checks the PR's branch out from there (Bitbucket
/// has the branch itself, no pull ref), and the merge is read from
/// that provider by number.
#[test]
fn a_pull_request_on_a_named_mirror_is_fetched_from_that_remote() {
    let mut env = Env::new();
    let worktrees = env.data.root.join("wt");
    let text = pr_pipeline(&worktrees).replace(
        "repo = \"git@github.com:msull/docs.git\"\n",
        "repo = \"git@github.com:msull/docs.git\"\nremotes = { bb = \"git@bitbucket.org:msull/docs.git\" }\n",
    );
    std::fs::write(env.data.pr_pipeline(PROJECT), text).unwrap();
    env.bitbucket.lock().unwrap().prs.push((
        "msull/docs".into(),
        "feature/escape-docs".into(),
        PullRequest {
            number: 3,
            url: "https://bitbucket.org/msull/docs/pull-requests/3".into(),
            head: "bb00003".into(),
            state: "open".into(),
            mergeable: None,
            branch: "feature/escape-docs".into(),
            base: "dev".into(),
            title: "Document escape".into(),
        },
    ));
    let now = env.tick();
    let err = dispatch::serve::take_pull_requests(&mut env.runner, PROJECT, &["gh:docs/3"], now)
        .unwrap_err()
        .to_string();
    assert!(err.contains("no remote \"gh\""), "{err}");
    let t =
        dispatch::serve::take_pull_requests(&mut env.runner, PROJECT, &["bb:docs/3"], now).unwrap();
    let id = t.id.clone();
    assert_eq!(t.source.identity, "bitbucket:msull/docs!3");
    let pr = &t.source.pull_requests[0];
    assert_eq!(
        (pr.remote.as_str(), pr.local(), pr.base.as_str()),
        ("bb", "feature/escape-docs", "dev")
    );
    env.steps_until(&id, "the sign-off question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "inspect")
    });
    {
        let repo = env.repo.lock().unwrap();
        assert!(
            repo.remotes_set
                .iter()
                .any(|(_, n, u)| n == "bb" && u == "git@bitbucket.org:msull/docs.git"),
            "{:?}",
            repo.remotes_set
        );
        let tracked: Vec<(String, String)> = repo
            .tracked
            .iter()
            .map(|(_, _, b, r)| (b.clone(), r.clone()))
            .collect();
        assert_eq!(
            tracked,
            vec![("feature/escape-docs".to_owned(), "bb".to_owned())]
        );
        assert!(repo.fetched_pulls.is_empty(), "no pull ref on Bitbucket");
        assert_eq!(
            repo.worktrees.len(),
            1,
            "the tree itself is Dispatch's own branch"
        );
    }
    let t = env.ticket(&id);
    assert_eq!(t.lanes.len(), 1, "only the docs lane is cut");
    let d = env.pending(&id).into_iter().next().unwrap();
    assert!(d.question.contains("over bb/dev"), "{}", d.question);
    let now = env.tick();
    env.runner.decide(&id, &d.id, "proceed", None, now).unwrap();
    env.steps_until(&id, "the merge watch", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "merge")
    });
    env.bitbucket.lock().unwrap().prs[0].2.state = "merged".into();
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the ticket closing", |t, _| !t.active());
    assert!(matches!(&env.ticket(&id).state, TicketState::Closed { .. }));
}

/// A pull request take is refused for a spec without its lane, a lane
/// the pipeline lacks, a PR that is not open, two lanes in the one
/// repository, and a PR already on a live ticket.
#[test]
fn taking_pull_requests_refuses_bad_specs_and_doubles() {
    let mut env = Env::new();
    let worktrees = env.data.root.join("wt");
    std::fs::write(env.data.pr_pipeline(PROJECT), pr_pipeline(&worktrees)).unwrap();
    open_pr(
        &env,
        "msull/switchboard",
        9,
        "feature/escape",
        "Escape leaves the field",
    );
    env.prs.lock().unwrap().prs[0].2.state = "closed".into();
    let now = env.tick();
    let refused = |env: &mut Env, specs: &[&str], expected: &str| {
        let err = dispatch::serve::take_pull_requests(&mut env.runner, PROJECT, specs, now)
            .unwrap_err()
            .to_string();
        assert!(err.contains(expected), "{specs:?}: {err}");
    };
    refused(&mut env, &["9"], "name the lane");
    refused(&mut env, &["web/9"], "no lane \"web\"");
    refused(&mut env, &["repo/9"], "is closed");
    refused(&mut env, &["repo/9", "repo/9"], "is closed");
    env.prs.lock().unwrap().prs[0].2.state = "open".into();
    refused(&mut env, &[], "at least one");
    refused(&mut env, &["repo/9", "repo/9"], "named twice");
    dispatch::serve::take_pull_requests(&mut env.runner, PROJECT, &["repo/9"], now).unwrap();
    refused(&mut env, &["repo/9"], "is already ticket");
    // A second lane in the project's repository cannot carry a PR of
    // its own.
    let text = pr_pipeline(&worktrees).replace("repo = \"git@github.com:msull/docs.git\"\n", "");
    std::fs::write(env.data.pr_pipeline(PROJECT), text).unwrap();
    open_pr(&env, "msull/switchboard", 10, "feature/docs", "Docs");
    refused(
        &mut env,
        &["repo/10", "docs/10"],
        "share the project's repository",
    );
}

/// A socket failure (Switchboard restarted under the runner) is not a
/// refusal: the ticket stays active with nothing made, and the next
/// pass makes the project and starts the stage.
#[test]
fn a_socket_failure_ends_the_pass_without_parking() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.fail_once("spaces", "the socket closed before the request went out");
    env.step();
    let t = env.ticket(&id);
    assert!(t.active(), "{:?}", t.state);
    assert!(t.attempts.is_empty(), "no attempt was made");
    assert_eq!(env.sb().kinds_called("project.add"), 0);
    env.step();
    let t = env.ticket(&id);
    assert_eq!(env.sb().kinds_called("project.add"), 1);
    assert_eq!(t.attempts.len(), 1, "investigate started on the next pass");
}

// --- the runner's pass and a command from the terminal share one lock,
// idempotent requests are replayed, an attempt saved without its request
// fails instead of waiting, a record's primary is never absent, parking
// stops what runs, and the reviewer is told which tree the plan is about.

/// A port that fails the first request of the given kind with a socket
/// error, `error`, then behaves. Where the failure falls (before the
/// request went out, or after) is the same to the runner: no reply.
struct FailsOnce {
    inner: SharedPort,
    kind: &'static str,
    error: &'static str,
    fired: bool,
}

impl dispatch::port::Port for FailsOnce {
    fn call(
        &mut self,
        request: &switchboard_control::Request,
    ) -> std::io::Result<switchboard_control::Reply> {
        if !self.fired && request.body.kind() == self.kind {
            self.fired = true;
            return Err(std::io::Error::other(self.error));
        }
        self.inner.call(request)
    }
}

impl Env {
    /// The runner's next request of `kind` fails with `error`.
    fn fail_once(&mut self, kind: &'static str, error: &'static str) {
        self.runner.port = Box::new(FailsOnce {
            inner: SharedPort(Arc::clone(&self.sb)),
            kind,
            error,
            fired: false,
        });
    }
}

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

/// Through the plan, its review run started.
fn at_review_run(env: &mut Env) -> String {
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
    id
}

fn at_finalize(env: &mut Env) -> String {
    let id = at_review_run(env);
    env.sb().runs[0].state = RunState::Converged;
    env.step();
    assert_eq!(env.pending(&id)[0].name, "finalize");
    id
}

#[test]
fn an_authorised_finalize_survives_a_lost_request() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let decision = env.pending(&id)[0].id.clone();
    let now = env.tick();
    env.runner
        .decide(&id, &decision, "finalize", None, now)
        .unwrap();
    env.fail_once(
        "workflow.finalize",
        "the socket closed before the request went out",
    );
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

/// A workflow query the app could not answer, as a failed reply or a
/// socket timeout, says nothing about the run: the attempt keeps
/// running and the next pass asks again.
#[test]
fn a_workflow_query_the_app_could_not_answer_leaves_the_attempt_running() {
    for timeout in [false, true] {
        let mut env = Env::new();
        let id = at_review_run(&mut env);
        env.sb().runs[0].state = RunState::Converged;
        if timeout {
            env.fail_once("workflow", "Resource temporarily unavailable");
        } else {
            env.sb().fail_workflow_query = Some("the app did not answer in time".into());
        }
        env.step();
        let t = env.ticket(&id);
        let a = t.attempts_of("review").last().unwrap();
        assert!(a.is_open(), "{a:?}");
        assert!(env.pending(&id).is_empty(), "nothing waits on the user");
        env.step();
        let pending = env.pending(&id);
        assert_eq!(pending.len(), 1, "{pending:#?}");
        assert_eq!(pending[0].name, "finalize");
    }
}

/// Only Switchboard's word that it has no such run fails the attempt.
#[test]
fn a_run_switchboard_no_longer_has_fails_the_attempt() {
    let mut env = Env::new();
    let id = at_review_run(&mut env);
    env.sb().runs.clear();
    env.step();
    let t = env.ticket(&id);
    let a = t.attempts_of("review").last().unwrap();
    assert!(
        matches!(&a.state, AttemptState::Failed { reason } if reason == "review run gone: no such run"),
        "{a:?}"
    );
    let pending = env.pending(&id);
    assert_eq!(pending.len(), 1, "{pending:#?}");
    assert_eq!(pending[0].name, "rerun");
}

/// A park waits while the app cannot say the run paused: the attempt is
/// written cancelled only once the run is confirmed stopped.
#[test]
fn parking_waits_while_the_app_cannot_say_the_run_paused() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let decision = env.pending(&id)[0].id.clone();
    let now = env.tick();
    env.runner
        .decide(&id, &decision, "park", None, now)
        .unwrap();
    env.sb().fail_workflow_query = Some("the app did not answer in time".into());
    env.step();
    let t = env.ticket(&id);
    assert!(!matches!(t.state, TicketState::Parked { .. }), "{t:#?}");
    let review = t.attempts_of("review").last().unwrap();
    assert!(
        !matches!(review.state, AttemptState::Cancelled { .. }),
        "{review:#?}"
    );
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Parked { .. }), "{t:#?}");
    let review = t.attempts_of("review").last().unwrap();
    assert!(
        matches!(review.state, AttemptState::Cancelled { .. }),
        "{review:#?}"
    );
}

#[test]
fn an_attempt_saved_without_its_request_fails_instead_of_waiting_forever() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.step();
    // An attempt with no session and no operation: what a stop between
    // writing the attempt and writing its request leaves, which an old
    // record may still hold.
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
    // A primary gone and its backup kept: the backup is the truth.
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
    env.idle_past_grace();
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

// --- cleanup gates a rerun and is persisted first, parking resumes
// whole after a restart, and the documented pipeline's own reviewer is
// told its repository.

#[test]
fn a_rerun_waits_while_the_replaced_process_survives_its_kill() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.step();
    let old = session_of(&env.ticket(&id), "investigate");
    let now = env.now;
    env.sb().stop(&old, now);
    env.idle_past_grace();
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

/// The intent was saved, then Dispatch died before the pause, with the
/// finalize question still pending and the session still marked: after
/// a restart the run is paused first, the question withdrawn and the
/// session unmarked.
#[test]
fn parking_resumed_after_a_restart_pauses_the_run_withdraws_and_unmarks() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let finalize = env.pending(&id)[0].id.clone();
    let mut t = env.ticket(&id);
    t.state = TicketState::Parking {
        reason: "parked by hand".into(),
    };
    let now = env.tick();
    env.runner.save_ticket(&mut t, now).unwrap();
    let session = t.current_session().unwrap().clone();
    assert!(env.sb().waiting[&session].0);
    env.restart();
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Parked { .. }), "{t:#?}");
    assert!(matches!(
        &t.attempts_of("review").last().unwrap().state,
        AttemptState::Cancelled { .. }
    ));
    assert_eq!(
        t.decisions.iter().find(|d| d.id == finalize).unwrap().state,
        dispatch::ticket::DecisionState::Cancelled
    );
    let sb = env.sb();
    assert!(
        matches!(sb.runs[0].state, RunState::Paused { .. }),
        "{:?}",
        sb.runs[0].state
    );
    assert!(sb.sessions.iter().all(|s| s.liveness != Liveness::Running));
    assert!(!sb.waiting[&session].0);
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

#[test]
fn a_park_answer_cut_off_at_the_pause_is_finished_after_a_restart() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let decision = env.pending(&id)[0].id.clone();
    let now = env.tick();
    env.runner
        .decide(&id, &decision, "park", None, now)
        .unwrap();
    // The pass dies at the pause: the socket error ends the pass.
    env.fail_once("workflow.pause", "the socket dropped");
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
    env.fail_once("workflow.finalize", "the socket dropped");
    env.step();
    let t = env.ticket(&id);
    assert!(
        t.pending_decisions().is_empty(),
        "nothing waits on the user"
    );
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
    assert!(t.lanes.is_empty(), "no lane was cut");
    assert!(
        env.sb().sessions.is_empty(),
        "nothing runs outside a worktree"
    );
}

// --- a workspace of several repositories: the ticket's tree is the
// workspace, each lane a worktree of its own repository inside it, the
// lanes decision chooses where the work runs, and a lane's setup waits
// for its first agent.

const WORKSPACE: &str = r#"
version = 1

[project]
name = "Orchard"
repo = "git@example.com:k3/orchard-workspace.git"
worktrees = "{worktrees}"
space = "Dispatch · Orchard"

[source]
kind = "github"
repo = "k3/orchard-workspace"
label = "dispatch"
lane_hints = { "area:backend" = "backend", "area:frontend" = "frontend" }

[[lanes]]
name = "backend"
path = "orchard-backend"
repo = "git@example.com:k3/orchard-backend.git"
base = "main"
setup = ["uv", "sync"]

[[lanes]]
name = "frontend"
path = "orchard-frontend"
repo = "git@example.com:k3/orchard-frontend.git"
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
trust_folders = true
"#;

/// The worktree root: the data directory's setting, a pipeline's own
/// `worktrees` with `~` expanded, and a root a repository's tooling
/// could not survive refused at take.
#[test]
fn the_worktree_root_is_a_setting_a_tilde_is_the_home_and_a_space_is_refused() {
    let env = Env::new();
    assert_eq!(env.data.worktrees_dir(), env.worktrees);
    let home = std::env::var("HOME").unwrap();
    let text = std::fs::read_to_string(env.data.pipeline(PROJECT))
        .unwrap()
        .replace(
            &format!("worktrees = \"{}\"", env.worktrees.display()),
            "worktrees = \"~/trees\"",
        );
    let p = dispatch::pipeline::Pipeline::parse(&text).unwrap();
    assert_eq!(
        p.project.worktrees.as_deref(),
        Some(std::path::Path::new(&home).join("trees").as_path())
    );
    // No worktrees line: the setting rules, and one with a space is
    // refused before a ticket exists.
    let mut env = Env::new();
    let text = std::fs::read_to_string(env.data.pipeline(PROJECT))
        .unwrap()
        .replace(
            &format!("worktrees = \"{}\"\n", env.worktrees.display()),
            "",
        );
    std::fs::write(env.data.pipeline(PROJECT), &text).unwrap();
    env.data
        .write_settings(&dispatch::store::Settings {
            worktrees: Some(env.data.root.parent().unwrap().join("has space")),
        })
        .unwrap();
    let now = env.tick();
    let err = env
        .runner
        .take(
            PROJECT,
            &text,
            SourceSnapshot {
                kind: "github".into(),
                identity: "msull/switchboard#7".into(),
                number: Some(7),
                title: "x".into(),
                body: String::new(),
                url: None,
                labels: vec![],
                taken_at_ms: now,
                pull_requests: Vec::new(),
            },
            now,
        )
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("holds") && err.contains("dispatch worktrees"),
        "{err}"
    );
    assert!(
        env.runner.tickets().unwrap().is_empty(),
        "no ticket was made"
    );
}

/// Moving the worktree root moves every idle ticket's tree with git,
/// re-points each lane clone at its tree inside it, and tells
/// Switchboard the projects' new roots; a ticket with something running
/// is left where it is and named.
#[test]
fn moving_the_worktree_root_moves_idle_trees_and_repoints_lanes_and_projects() {
    let (mut env, id) = workspace_env(&["type:bug"]);
    // The pipeline defers to the setting.
    for path in [env.data.pipeline("Orchard"), env.ticket(&id).pipeline_file] {
        let text = std::fs::read_to_string(&path).unwrap().replace(
            &format!("worktrees = \"{}\"\n", env.worktrees.display()),
            "",
        );
        std::fs::write(&path, text).unwrap();
    }
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    let old_tree = env.worktrees.join(&id);
    assert_eq!(t.tree.as_deref(), Some(old_tree.as_path()));
    let new_root = env.data.root.parent().unwrap().join("wt2");
    let now = env.tick();
    let view = env
        .runner
        .set_worktrees(Some(new_root.clone()), true, now)
        .unwrap();
    assert_eq!(view.root, new_root);
    assert_eq!(view.moved, Vec::<String>::new());
    assert_eq!(view.skipped[0].0, id, "{view:?}");
    assert!(view.skipped[0].1.contains("running"));
    assert_eq!(env.ticket(&id).tree.as_deref(), Some(old_tree.as_path()));
    // The investigator finishes; the lanes question is pending and
    // nothing runs, so the tree moves.
    let investigator = session_of(&t, "investigate");
    std::fs::write(artifact_of(&t, "investigate", "notes"), "# notes").unwrap();
    let now = env.now;
    env.sb().stop(&investigator, now);
    for _ in 0..5 {
        let now = env.tick();
        env.runner.step_project("Orchard", now).unwrap();
    }
    assert_eq!(env.pending(&id).len(), 1);
    let now = env.tick();
    let view = env.runner.set_worktrees(None, true, now).unwrap();
    assert_eq!(view.root, new_root, "unchanged setting, migration only");
    assert_eq!(view.moved, vec![id.clone()], "{view:?}");
    let new_tree = new_root.join(&id);
    let t = env.ticket(&id);
    assert_eq!(t.tree.as_deref(), Some(new_tree.as_path()));
    assert!(new_tree.is_dir() && !old_tree.exists());
    for lane in &t.lanes {
        assert!(lane.worktree.starts_with(&new_tree), "{lane:?}");
    }
    let repo = env.repo.lock().unwrap();
    assert_eq!(
        repo.moved,
        vec![(
            env.data.repo_dir("Orchard"),
            old_tree.clone(),
            new_tree.clone()
        )]
    );
    let repaired: Vec<&std::path::Path> = repo.repaired.iter().map(|(_, d)| d.as_path()).collect();
    assert_eq!(repaired.len(), t.lanes.len(), "{repaired:?}");
    assert!(repaired.iter().all(|d| d.starts_with(&new_tree)));
    drop(repo);
    let sb = env.sb();
    let project = sb
        .projects
        .iter()
        .find(|p| Some(&p.id) == t.root_project.as_ref())
        .unwrap();
    assert_eq!(project.root, new_tree);
    assert_eq!(sb.kinds_called("project.root"), 1);
    drop(sb);
    // Once moved, a second migration finds nothing under the old root.
    let now = env.tick();
    let view = env.runner.set_worktrees(None, true, now).unwrap();
    assert!(view.moved.is_empty() && view.skipped.is_empty(), "{view:?}");
}

/// A session query the app could not answer says nothing about the
/// session: the attempt is asked about again, not failed.
#[test]
fn a_query_the_app_could_not_answer_leaves_the_attempt_running() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.step();
    let investigator = session_of(&env.ticket(&id), "investigate");
    env.sb().fail_session_query = Some("the app did not answer in time".into());
    env.step();
    let t = env.ticket(&id);
    let a = t.attempts_of("investigate").last().unwrap();
    assert!(a.is_open(), "{a:?}");
    assert_eq!(a.session.as_deref(), Some(investigator.as_str()));
    assert!(env.pending(&id).is_empty(), "nothing waits on the user");
    env.step();
    assert!(
        env.ticket(&id)
            .attempts_of("investigate")
            .last()
            .unwrap()
            .is_open()
    );
}

/// A Stop while the card still reads `working` ended a turn, not the
/// work (background agents still at it): the attempt is held however
/// long that takes, and notes written mid-turn complete nothing until a
/// later Stop leaves the card idle.
#[test]
fn a_stop_while_still_working_holds_the_attempt_until_the_notes_land() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.step();
    let t = env.ticket(&id);
    let investigator = session_of(&t, "investigate");
    let notes = artifact_of(&t, "investigate", "notes");
    let now = env.now;
    env.sb().stop(&investigator, now);
    env.sb().session_mut(&investigator).card = "working".into();
    for _ in 0..STOP_IDLE_POLLS + 5 {
        env.step();
    }
    let t = env.ticket(&id);
    let a = t.attempts_of("investigate").last().unwrap();
    assert!(a.is_open(), "{a:?}");
    assert!(env.pending(&id).is_empty(), "nothing waits on the user");
    assert!(!env.sb().killed.contains(&investigator));
    // Written mid-turn: settled, but no completion while it works.
    std::fs::write(&notes, "# notes\nfindings").unwrap();
    for _ in 0..SETTLE_POLLS + 2 {
        env.step();
    }
    let t = env.ticket(&id);
    assert!(t.attempts_of("investigate").last().unwrap().is_open());
    assert!(!env.sb().killed.contains(&investigator));
    let now = env.now;
    env.sb().stop(&investigator, now);
    env.steps_until(&id, "the investigation completing", |t, _| {
        t.attempts_of("investigate").next().unwrap().state == AttemptState::Complete
    });
    assert!(env.sb().killed.contains(&investigator));
}

/// A stopped agent with no notes is given up on only after
/// `STOP_IDLE_POLLS` passes in a row at its prompt: a card reading
/// `waiting on you` or `working` in between starts the count again.
#[test]
fn a_stop_idle_without_notes_fails_after_the_grace() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.step();
    let investigator = session_of(&env.ticket(&id), "investigate");
    let open = |env: &Env| {
        env.ticket(&id)
            .attempts_of("investigate")
            .last()
            .unwrap()
            .is_open()
    };
    let now = env.now;
    env.sb().stop(&investigator, now);
    env.sb().session_mut(&investigator).card = "waiting on you".into();
    for _ in 0..=STOP_IDLE_POLLS {
        env.step();
    }
    assert!(open(&env), "a question to the user holds it");
    env.sb().session_mut(&investigator).card = "idle".into();
    for _ in 0..STOP_IDLE_POLLS - 1 {
        env.step();
    }
    assert!(open(&env));
    env.sb().session_mut(&investigator).card = "working".into();
    env.step();
    env.sb().session_mut(&investigator).card = "idle".into();
    for _ in 0..STOP_IDLE_POLLS - 1 {
        env.step();
    }
    assert!(open(&env), "the count started again");
    env.step();
    let t = env.ticket(&id);
    assert!(
        matches!(&t.attempts_of("investigate").last().unwrap().state, AttemptState::Failed { reason } if reason.contains("stopped without writing notes")),
        "{t:#?}"
    );
    let pending = env.pending(&id);
    assert_eq!(pending.len(), 1, "{pending:#?}");
    assert_eq!(pending[0].name, "rerun");
}

/// A reply recovered on a pass that changes nothing else is still
/// written, so the operation is not recovered again on every pass.
#[test]
fn a_recovered_reply_is_kept_even_when_the_pass_changes_nothing_else() {
    let (mut env, id) = workspace_env(&["type:bug"]);
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    let investigator = session_of(&t, "investigate");
    std::fs::write(artifact_of(&t, "investigate", "notes"), "# notes").unwrap();
    let now = env.now;
    env.sb().stop(&investigator, now);
    // The lanes question marks the session with notes; that reply is lost.
    env.sb().drop_reply_for = Some("session.notes".into());
    for _ in 0..5 {
        let now = env.tick();
        env.runner.step_project("Orchard", now).unwrap();
    }
    assert_eq!(env.pending(&id).len(), 1);
    let unanswered = |t: &Ticket| t.ledger.iter().filter(|o| o.reply.is_none()).count();
    let t = env.ticket(&id);
    assert!(unanswered(&t) <= 1, "{:#?}", t.ledger);
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    assert_eq!(unanswered(&t), 0, "recovered and kept: {:#?}", t.ledger);
    assert_eq!(
        env.sb().kinds_called("session.notes"),
        2,
        "sent once, recovered once, then left alone"
    );
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    assert_eq!(env.sb().kinds_called("session.notes"), 2);
}

/// A project whose policy pre-authorises Claude's folder trust question
/// has it answered for its agents; one without leaves it to the user.
#[test]
fn the_trust_question_is_answered_only_where_the_policy_says_so() {
    let (mut env, id) = workspace_env(&["type:bug"]);
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    let investigator = session_of(&env.ticket(&id), "investigate");
    env.sb().session_mut(&investigator).trust_question = true;
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    {
        let sb = env.sb();
        assert_eq!(sb.trusted, vec![investigator.clone()]);
        assert!(!sb.session(&investigator).trust_question);
    }
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    assert_eq!(env.sb().trusted.len(), 1, "answered once");
    // The Switchboard pipeline says nothing, so its agent waits.
    let mut plain = Env::new();
    let other = plain.take(7).id;
    plain.step();
    let investigator = session_of(&plain.ticket(&other), "investigate");
    plain.sb().session_mut(&investigator).trust_question = true;
    plain.step();
    plain.step();
    assert!(plain.sb().trusted.is_empty(), "no folder was trusted");
    assert!(plain.sb().session(&investigator).trust_question);
}

fn workspace_env(labels: &[&str]) -> (Env, String) {
    let mut env = Env::new();
    let text = WORKSPACE.replace("{worktrees}", &env.worktrees.display().to_string());
    std::fs::write(env.data.pipeline("Orchard"), text).unwrap();
    let now = env.tick();
    let id = env
        .runner
        .take(
            "Orchard",
            &std::fs::read_to_string(env.data.pipeline("Orchard")).unwrap(),
            SourceSnapshot {
                kind: "github".into(),
                identity: "k3/orchard-workspace#42".into(),
                number: Some(42),
                title: "Asset report column missing".into(),
                body: String::new(),
                url: None,
                labels: labels.iter().map(|l| (*l).to_owned()).collect(),
                taken_at_ms: now,
                pull_requests: Vec::new(),
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
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    let tree = env.worktrees.join(&id);
    assert_eq!(t.tree.as_deref(), Some(tree.as_path()), "{t:#?}");
    assert_eq!(t.lanes.len(), 2);
    assert_eq!(t.lanes[0].worktree, tree.join("orchard-backend"));
    assert_eq!(t.lanes[1].worktree, tree.join("orchard-frontend"));
    assert!(
        t.lanes
            .iter()
            .all(|l| l.branch == "dispatch/42-asset-report-column-missing")
    );
    assert!(
        t.lanes.iter().all(|l| !l.setup_done),
        "setups wait for an agent"
    );
    for name in ["Orchard", "Orchard@backend", "Orchard@frontend"] {
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
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    let investigator = session_of(&t, "investigate");
    std::fs::write(artifact_of(&t, "investigate", "notes"), "# notes").unwrap();
    let now = env.now;
    env.sb().stop(&investigator, now);
    for _ in 0..8 {
        let now = env.tick();
        env.runner.step_project("Orchard", now).unwrap();
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
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    let investigator = session_of(&t, "investigate");
    std::fs::write(artifact_of(&t, "investigate", "notes"), "# notes").unwrap();
    let now = env.now;
    env.sb().stop(&investigator, now);
    for _ in 0..5 {
        let now = env.tick();
        env.runner.step_project("Orchard", now).unwrap();
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
        env.runner.step_project("Orchard", now).unwrap();
    }
    let t = env.ticket(&id);
    assert!(t.lanes.iter().all(|l| l.chosen));
    assert_eq!(
        t.attempts_of("plan").count(),
        2,
        "a planner per chosen lane: {t:#?}"
    );
    // Side by side, the two attempts are distinct to every lookup: one
    // stops and completes while the other keeps its own session; a
    // rerun of one is numbered after both.
    let numbers: Vec<u32> = t.attempts_of("plan").map(|a| a.n).collect();
    assert_eq!(numbers, vec![1, 2], "one number per attempt of the stage");
    let backend = plan_of(&t, "backend");
    let frontend = plan_of(&t, "frontend");
    assert_ne!(backend.session, frontend.session);
    env.finish(
        backend.session.as_ref().unwrap(),
        &backend.artifacts["plan"],
        "# backend plan",
    );
    env.steps_until(&id, "the backend plan completing", |t, _| {
        plan_of(t, "backend").state == AttemptState::Complete
    });
    let t = env.ticket(&id);
    assert!(plan_of(&t, "frontend").is_open(), "{t:#?}");
    assert_eq!(
        plan_of(&t, "frontend").session,
        frontend.session,
        "untouched by the other's poll"
    );
    // The frontend's pane goes away: only its attempt fails, and the
    // rerun takes the next number of the stage.
    env.sb().remove(frontend.session.as_ref().unwrap());
    env.steps_until(&id, "the frontend plan failing", |t, _| {
        matches!(plan_of(t, "frontend").state, AttemptState::Failed { .. })
    });
    let t = env.ticket(&id);
    assert_eq!(plan_of(&t, "backend").state, AttemptState::Complete);
    let rerun = env.pending(&id).remove(0);
    assert_eq!(
        (rerun.name.as_str(), rerun.attempt.clone()),
        ("rerun", Some(("plan".to_owned(), 2)))
    );
    let now = env.tick();
    env.runner
        .decide(&id, &rerun.id, "rerun", Some("keep it short"), now)
        .unwrap();
    env.steps_until(&id, "the rerun starting", |t, _| {
        plan_of(t, "frontend").n == 3
    });
    // A rerun answer's own note goes at the end of the new prompt.
    let prompt = last_prompt_of(&env, "planner");
    assert!(prompt.ends_with("sent it back: keep it short"), "{prompt}");
    let t = env.ticket(&id);
    let latest = plan_of(&t, "frontend");
    assert_eq!((latest.n, latest.is_open()), (3, true), "{t:#?}");
}

/// The Orchard ticket at its two planners, the lanes question answered,
/// then the backend planner given up on: its `rerun` question pending
/// and marked on the frontend planner (the newest session), which has
/// stopped without a plan. Returns the env, the ticket, the investigator
/// and the frontend planner's session.
fn marked_frontend_planner() -> (Env, String, String, String) {
    let (mut env, id) = workspace_env(&["type:bug"]);
    env.step();
    let t = env.ticket(&id);
    let investigator = session_of(&t, "investigate");
    std::fs::write(artifact_of(&t, "investigate", "notes"), "# notes").unwrap();
    let now = env.now;
    env.sb().stop(&investigator, now);
    env.steps_until(&id, "the lanes question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "lanes")
    });
    assert!(env.sb().waiting[&investigator].0, "the question marked it");
    let lanes = env.pending(&id).remove(0);
    let now = env.tick();
    env.runner
        .decide(&id, &lanes.id, "backend, frontend", None, now)
        .unwrap();
    env.steps_until(&id, "a planner per lane", |t, _| {
        t.attempts_of("plan").filter(|a| a.is_open()).count() == 2
    });
    assert!(
        !env.sb().waiting[&investigator].0,
        "the answer unmarked the investigator, not only the newest planner"
    );
    let t = env.ticket(&id);
    let backend = plan_of(&t, "backend");
    let frontend = plan_of(&t, "frontend");
    assert_eq!((backend.n, frontend.n), (1, 2));
    let now = env.now;
    env.sb().stop(backend.session.as_ref().unwrap(), now);
    env.idle_past_grace();
    let t = env.ticket(&id);
    assert!(
        matches!(plan_of(&t, "backend").state, AttemptState::Failed { .. }),
        "{t:#?}"
    );
    assert!(rerun_for(&t, "backend").is_some(), "{t:#?}");
    let frontend = frontend.session.unwrap();
    assert!(env.sb().waiting[&frontend].0, "the question marked it");
    let now = env.now;
    env.sb().stop(&frontend, now);
    for _ in 0..STOP_IDLE_POLLS + 5 {
        env.step();
    }
    let t = env.ticket(&id);
    assert!(plan_of(&t, "frontend").is_open(), "held while marked");
    (env, id, investigator, frontend)
}

/// The `rerun` question about the backend planner, answered.
fn rerun_backend(env: &mut Env, id: &str) {
    let rerun = rerun_for(&env.ticket(id), "backend").unwrap();
    let now = env.tick();
    env.runner
        .decide(id, &rerun.id, "rerun", None, now)
        .unwrap();
}

fn plan_failed_without_writing(t: &Ticket, ctx: &str) -> bool {
    matches!(&plan_of(t, ctx).state, AttemptState::Failed { reason } if reason.contains("stopped without writing plan"))
}

/// Answering clears every session a decision marked, not only the
/// ticket's newest, so an agent held by a mark is given up on once the
/// question it was showing is answered.
#[test]
fn an_answer_clears_every_mark_so_a_marked_agent_is_still_given_up_on() {
    let (mut env, id, _, frontend) = marked_frontend_planner();
    rerun_backend(&mut env, &id);
    env.steps_until(&id, "the backend's rerun", |t, _| {
        let a = plan_of(t, "backend");
        a.n == 3 && a.session.is_some()
    });
    assert!(
        env.sb().waiting.values().all(|(on, _)| !on),
        "{:?}",
        env.sb().waiting
    );
    env.idle_past_grace();
    let t = env.ticket(&id);
    assert!(plan_failed_without_writing(&t, "frontend"), "{t:#?}");
    assert!(!env.sb().waiting[&frontend].0);
}

/// A `waiting off` that never reached Switchboard, or whose reply was
/// lost, is sent again on a later pass until it lands.
#[test]
fn a_lost_unmark_is_sent_again_until_it_lands() {
    for lost_reply in [false, true] {
        let (mut env, id, _, frontend) = marked_frontend_planner();
        rerun_backend(&mut env, &id);
        if lost_reply {
            env.sb().drop_reply_for = Some("session.waiting".into());
        } else {
            env.fail_once("session.waiting", "Resource temporarily unavailable");
        }
        env.step();
        let t = env.ticket(&id);
        let off = waiting_ops(&t)
            .into_iter()
            .rfind(|o| {
                matches!(&o.body, Some(Body::SessionWaiting { session, on: false, .. }) if session == &frontend)
            })
            .unwrap();
        assert!(off.reply.is_none(), "{off:#?}");
        env.idle_past_grace();
        let t = env.ticket(&id);
        assert!(
            waiting_ops(&t).iter().all(|o| o.reply.is_some()),
            "{:#?}",
            waiting_ops(&t)
        );
        assert!(!env.sb().waiting[&frontend].0);
        assert!(plan_failed_without_writing(&t, "frontend"), "{t:#?}");
    }
}

/// A waiting request whose reply never came, followed by a later one
/// for the same session that was answered (what a replay that failed
/// leaves behind), is never sent after the later one: the mark stays as
/// the later request left it.
#[test]
fn a_replaced_waiting_request_is_never_sent_after_the_one_that_replaced_it() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.step();
    let mut t = env.ticket(&id);
    let investigator = session_of(&t, "investigate");
    assert!(t.pending_decisions().is_empty());
    let template = t.ledger.last().unwrap().clone();
    let waiting = |op: &str, on: bool, answered: bool| dispatch::ticket::Operation {
        op: op.into(),
        kind: "session.waiting".into(),
        class: "idempotent".into(),
        attempt: None,
        intent: String::new(),
        body: Some(Body::SessionWaiting {
            session: investigator.clone(),
            on,
            reason: if on {
                "decision d-1: rerun".into()
            } else {
                String::new()
            },
        }),
        reply: answered.then(|| switchboard_control::Reply::Persisted { made: vec![] }),
        error: (!answered).then(|| "the socket dropped".into()),
        asked: false,
        settled: false,
        ..template.clone()
    };
    t.ledger.push(waiting("op-on", true, false));
    t.ledger.push(waiting("op-off", false, true));
    env.sb()
        .waiting
        .insert(investigator.clone(), (false, String::new()));
    let now = env.tick();
    env.runner.save_ticket(&mut t, now).unwrap();
    let calls = env.sb().calls.len();
    env.step();
    env.step();
    let sb = env.sb();
    assert!(
        !sb.calls[calls..]
            .iter()
            .any(|r| matches!(&r.body, Body::SessionWaiting { on: true, .. })),
        "the replaced mark was sent again: {:?}",
        &sb.calls[calls..]
    );
    assert!(!sb.waiting[&investigator].0);
    drop(sb);
    let t = env.ticket(&id);
    let on = t.ledger.iter().find(|o| o.op == "op-on").unwrap();
    assert!(on.settled && on.reply.is_none(), "{on:#?}");
}

/// The workspace pipeline, on disk and in the ticket's copy, with a
/// human `inspect` gate after `plan` in place of the review.
fn inspect_instead_of_review(env: &Env, id: &str) {
    let t = env.ticket(id);
    let review = "[[stages]]\nname = \"review\"\nreview = \"reviewer\"\ncontext = \"each\"\nsubject = \"plan\"\ngate = { kind = \"external\", check = \"review-finalized\" }\n";
    let inspect = "[[stages]]\nname = \"inspect\"\ncontext = \"each\"\ngate = { kind = \"human\", decision = \"inspect\" }\n";
    for path in [env.data.pipeline("Orchard"), t.pipeline_file.clone()] {
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(review), "{text}");
        std::fs::write(&path, text.replace(review, inspect)).unwrap();
    }
}

/// `max_reruns = n` in the project's pipeline and in the ticket's copy.
fn with_max_reruns(env: &Env, t: &Ticket, n: u32) {
    for path in [env.data.pipeline(PROJECT), t.pipeline_file.clone()] {
        let text = std::fs::read_to_string(&path).unwrap().replace(
            "waiting_on_me = 3\n",
            &format!("waiting_on_me = 3\nmax_reruns = {n}\n"),
        );
        std::fs::write(&path, text).unwrap();
    }
}

/// The investigator's launch with its reply lost and the attempt back to
/// `Starting`, after three failed attempts: failing it again spends
/// `max_reruns`, which would park a running ticket.
fn lost_launch_with_reruns_spent(t: &mut Ticket) {
    let op = t
        .ledger
        .iter_mut()
        .find(|o| o.kind == "session.new")
        .unwrap();
    op.reply = None;
    t.attempts[0].session = None;
    t.attempts[0].state = AttemptState::Starting;
    for n in 1..=3 {
        let mut failed = t.attempts[0].clone();
        failed.n += n;
        failed.state = AttemptState::Failed {
            reason: "earlier".into(),
        };
        t.attempts.push(failed);
    }
}

/// The plan attempt of one lane, by context.
fn plan_of(t: &Ticket, ctx: &str) -> Attempt {
    t.attempts_of("plan")
        .filter(|a| a.context == ctx)
        .max_by_key(|a| a.n)
        .unwrap()
        .clone()
}

// --- parking withdraws every question, and a resume asks afresh

/// The Orchard ticket with both lanes' planners gone: two failed plan
/// attempts and a pending `rerun` question about each.
fn two_failed_plans() -> (Env, String) {
    let (mut env, id) = workspace_env(&["type:bug"]);
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    let investigator = session_of(&t, "investigate");
    std::fs::write(artifact_of(&t, "investigate", "notes"), "# notes").unwrap();
    let now = env.now;
    env.sb().stop(&investigator, now);
    env.steps_until(&id, "the lanes question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "lanes")
    });
    let lanes = env.pending(&id).remove(0);
    let now = env.tick();
    env.runner
        .decide(&id, &lanes.id, "backend, frontend", None, now)
        .unwrap();
    env.steps_until(&id, "a planner per lane", |t, _| {
        t.attempts_of("plan").filter(|a| a.is_open()).count() == 2
    });
    let t = env.ticket(&id);
    for ctx in ["backend", "frontend"] {
        env.sb().remove(plan_of(&t, ctx).session.as_ref().unwrap());
    }
    env.steps_until(&id, "both plans failing", |t, _| {
        t.attempts_of("plan")
            .all(|a| matches!(a.state, AttemptState::Failed { .. }))
            && t.pending_decisions().len() == 2
    });
    (env, id)
}

/// The pending `rerun` question about a lane's latest plan attempt.
fn rerun_for(t: &Ticket, ctx: &str) -> Option<Decision> {
    let n = plan_of(t, ctx).n;
    t.pending_decisions()
        .into_iter()
        .find(|d| d.name == "rerun" && d.attempt == Some(("plan".to_owned(), n)))
        .cloned()
}

/// The waiting requests on the ledger, oldest first.
fn waiting_ops(t: &Ticket) -> Vec<dispatch::ticket::Operation> {
    t.ledger
        .iter()
        .filter(|o| o.kind == "session.waiting")
        .cloned()
        .collect()
}

fn waiting_on(op: &dispatch::ticket::Operation) -> bool {
    matches!(op.body, Some(Body::SessionWaiting { on: true, .. }))
}

/// The state of decision `id`.
fn decision_state(t: &Ticket, id: &str) -> dispatch::ticket::DecisionState {
    t.decisions
        .iter()
        .find(|d| d.id == id)
        .unwrap()
        .state
        .clone()
}

/// Park the two-lane ticket from the backend's question, then resume it.
fn parked_and_resumed() -> (Env, String, Vec<String>) {
    let (mut env, id) = two_failed_plans();
    let t = env.ticket(&id);
    let parked_from = rerun_for(&t, "backend").unwrap();
    let now = env.tick();
    env.runner
        .decide(&id, &parked_from.id, "park", None, now)
        .unwrap();
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Parked { .. }), "{t:#?}");
    let earlier: Vec<String> = t.decisions.iter().map(|d| d.id.clone()).collect();
    let now = env.tick();
    env.runner.resume(&id, now).unwrap();
    (env, id, earlier)
}

#[test]
fn parking_cancels_every_pending_decision_and_a_resume_asks_afresh() {
    let (mut env, id) = two_failed_plans();
    let t = env.ticket(&id);
    let parked_from = rerun_for(&t, "backend").unwrap();
    let other = rerun_for(&t, "frontend").unwrap();
    let now = env.tick();
    env.runner
        .decide(&id, &parked_from.id, "park", None, now)
        .unwrap();
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    assert!(
        matches!(&t.state, TicketState::Parked { reason } if reason.contains("by hand")),
        "{t:#?}"
    );
    assert!(t.pending_decisions().is_empty(), "{t:#?}");
    assert!(
        matches!(
            decision_state(&t, &parked_from.id),
            dispatch::ticket::DecisionState::Answered { answer, acted: true, .. } if answer == "park"
        ),
        "{t:#?}"
    );
    assert_eq!(
        decision_state(&t, &other.id),
        dispatch::ticket::DecisionState::Cancelled
    );
    let status = dispatch::serve::status(&env.runner).unwrap();
    let orchard = status
        .projects
        .iter()
        .find(|p| p.name == "Orchard")
        .unwrap();
    assert_eq!(orchard.pending, 0, "a parked ticket waits on no one");
    let session = t.current_session().unwrap().clone();
    assert!(!env.sb().waiting[&session].0, "the session is unmarked");
    let last = waiting_ops(&t).pop().unwrap();
    assert!(!waiting_on(&last) && last.reply.is_some(), "{last:#?}");
    let now = env.tick();
    let refused = env
        .runner
        .decide(&id, &other.id, "rerun", None, now)
        .unwrap_err();
    assert!(
        refused.to_string().contains("has no pending decision"),
        "{refused}"
    );

    // Resumed: the park answer is not acted on again, and each lane is
    // asked afresh, under a new id. Nothing launches without an answer.
    let earlier: Vec<String> = t.decisions.iter().map(|d| d.id.clone()).collect();
    let sessions = env.sb().sessions.len();
    let now = env.tick();
    env.runner.resume(&id, now).unwrap();
    for _ in 0..3 {
        let now = env.tick();
        env.runner.step_project("Orchard", now).unwrap();
    }
    let t = env.ticket(&id);
    assert!(t.active(), "{t:#?}");
    let asked = t.pending_decisions();
    assert_eq!(asked.len(), 2, "{t:#?}");
    for ctx in ["backend", "frontend"] {
        let d = rerun_for(&t, ctx).unwrap_or_else(|| panic!("no rerun for {ctx}: {t:#?}"));
        assert!(!earlier.contains(&d.id), "a new id: {}", d.id);
        assert!(d.question.contains("failed"), "{}", d.question);
    }
    assert_eq!(t.attempts_of("plan").count(), 2, "{t:#?}");
    assert_eq!(env.sb().sessions.len(), sessions);

    // One lane's answer starts that lane; the other's question still
    // holds only the other lane.
    let backend = rerun_for(&t, "backend").unwrap();
    let frontend = rerun_for(&t, "frontend").unwrap();
    let now = env.tick();
    env.runner
        .decide(&id, &backend.id, "rerun", None, now)
        .unwrap();
    for _ in 0..2 {
        let now = env.tick();
        env.runner.step_project("Orchard", now).unwrap();
    }
    let t = env.ticket(&id);
    let started = plan_of(&t, "backend");
    assert_eq!((started.n, started.is_open()), (3, true), "{t:#?}");
    assert_eq!(rerun_for(&t, "frontend").unwrap().id, frontend.id);
    assert_eq!(plan_of(&t, "frontend").n, 2, "the other lane is untouched");
    assert_eq!(t.attempts_of("plan").count(), 3);
    assert_eq!(env.sb().sessions.len(), sessions + 1);
}

#[test]
fn a_pass_cut_off_between_two_lanes_finishes_asking_on_the_next_pass() {
    let (mut env, id, earlier) = parked_and_resumed();
    let sessions = env.sb().sessions.len();
    // A question is saved before its notes go out, so the pass ends
    // with one lane asked and the other not looked at.
    env.fail_once("session.notes", "the socket dropped");
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    let asked = t.pending_decisions();
    assert_eq!(asked.len(), 1, "{t:#?}");
    let first = rerun_for(&t, "backend").unwrap();
    assert!(!earlier.contains(&first.id));
    assert!(rerun_for(&t, "frontend").is_none());
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    assert_eq!(t.pending_decisions().len(), 2, "{t:#?}");
    assert_eq!(rerun_for(&t, "backend").unwrap().id, first.id, "asked once");
    let second = rerun_for(&t, "frontend").unwrap();
    assert!(!earlier.contains(&second.id) && second.id != first.id);
    assert_eq!(t.attempts_of("plan").count(), 2);
    assert_eq!(env.sb().sessions.len(), sessions);
}

#[test]
fn a_park_cut_off_after_its_intent_asks_nothing_and_unmarks_on_the_next_pass() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let finalize = env.pending(&id)[0].clone();
    // A second question open on the same ticket.
    let mut t = env.ticket(&id);
    let mut second = finalize.clone();
    second.id = format!("d{}", t.decisions.len() + 1);
    second.name = "paused".into();
    t.decisions.push(second.clone());
    let now = env.tick();
    env.runner.save_ticket(&mut t, now).unwrap();
    let now = env.tick();
    env.runner
        .decide(&id, &finalize.id, "park", None, now)
        .unwrap();
    // The unmark is written down and never delivered.
    env.fail_once("session.waiting", "the socket dropped");
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Parking { .. }), "{t:#?}");
    assert!(t.pending_decisions().is_empty(), "{t:#?}");
    assert_eq!(
        decision_state(&t, &second.id),
        dispatch::ticket::DecisionState::Cancelled
    );
    assert!(matches!(
        decision_state(&t, &finalize.id),
        dispatch::ticket::DecisionState::Answered { answer, acted: true, .. } if answer == "park"
    ));
    let now = env.tick();
    assert!(
        env.runner
            .decide(&id, &second.id, "continue", None, now)
            .is_err()
    );
    let ops = waiting_ops(&t);
    let unmark = ops.last().unwrap().clone();
    assert!(
        !waiting_on(&unmark) && unmark.reply.is_none(),
        "{unmark:#?}"
    );
    let session = t.current_session().unwrap().clone();
    assert!(env.sb().waiting[&session].0, "still marked");

    // The next pass, no restart: the same operation, delivered.
    env.step();
    let t = env.ticket(&id);
    let after = waiting_ops(&t);
    assert_eq!(after.len(), ops.len(), "no second unmark written");
    let last = after.last().unwrap();
    assert_eq!(last.op, unmark.op);
    assert!(last.reply.is_some());
    assert!(!env.sb().waiting[&session].0);
    assert!(matches!(t.state, TicketState::Parked { .. }), "{t:#?}");
    assert!(matches!(env.sb().runs[0].state, RunState::Paused { .. }));
    assert!(
        env.sb()
            .sessions
            .iter()
            .all(|s| s.liveness != Liveness::Running)
    );
    let calls = env.sb().calls.len();
    env.step();
    assert_eq!(env.sb().calls.len(), calls, "Parked is quiet");
}

#[test]
fn an_unanswered_mark_is_resolved_before_the_unmark_and_stays_off_after_a_restart() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    // As `park` would leave it just after its intent, with the reply to
    // the finalize question's mark lost.
    let mut t = env.ticket(&id);
    t.state = TicketState::Parking {
        reason: "parked by hand".into(),
    };
    let d = t.decisions.iter_mut().find(|d| d.pending()).unwrap();
    d.state = dispatch::ticket::DecisionState::Answered {
        answer: "park".into(),
        note: None,
        by: dispatch::scheduler::BY_HAND.into(),
        at_ms: env.now,
        acted: true,
    };
    let mark = t
        .ledger
        .iter_mut()
        .rev()
        .find(|o| o.kind == "session.waiting" && waiting_on(o))
        .unwrap();
    mark.reply = None;
    let mark = mark.op.clone();
    let now = env.tick();
    env.runner.save_ticket(&mut t, now).unwrap();
    let session = t.current_session().unwrap().clone();
    let before = waiting_ops(&t).len();
    // No restart, whose recovery would replay it first: the replay
    // inside parking is the request that fails.
    env.fail_once("session.waiting", "the socket dropped");
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Parking { .. }), "{t:#?}");
    let ops = waiting_ops(&t);
    let replayed = ops.iter().find(|o| o.op == mark).unwrap();
    assert!(replayed.reply.is_none() && replayed.error.is_some());
    assert_eq!(ops.len(), before, "the unmark waits for the mark");

    env.step();
    let t = env.ticket(&id);
    let ops = waiting_ops(&t);
    let at = ops.iter().position(|o| o.op == mark).unwrap();
    assert!(ops[at].reply.is_some(), "answered under its own id");
    assert_eq!(ops.len(), at + 2, "{ops:#?}");
    assert!(!waiting_on(&ops[at + 1]) && ops[at + 1].reply.is_some());
    assert!(ops.iter().all(|o| o.reply.is_some()));
    assert!(!env.sb().waiting[&session].0);
    assert!(matches!(t.state, TicketState::Parked { .. }), "{t:#?}");

    let sent = env.sb().kinds_called("session.waiting");
    env.restart();
    env.step();
    assert_eq!(
        env.sb().kinds_called("session.waiting"),
        sent,
        "recovery sends nothing"
    );
    assert!(!env.sb().waiting[&session].0, "the mark stays off");
    assert!(matches!(env.ticket(&id).state, TicketState::Parked { .. }));
}

#[test]
fn a_waiting_request_without_its_body_does_not_hold_parking() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    // Parking, with a mark whose reply was lost and whose body was
    // never recorded: it cannot be sent again.
    let mut t = env.ticket(&id);
    t.state = TicketState::Parking {
        reason: "parked by hand".into(),
    };
    let mark = t
        .ledger
        .iter_mut()
        .rev()
        .find(|o| o.kind == "session.waiting" && waiting_on(o))
        .unwrap();
    mark.reply = None;
    mark.body = None;
    let mark = mark.op.clone();
    let now = env.tick();
    env.runner.save_ticket(&mut t, now).unwrap();
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Parked { .. }), "{t:#?}");
    let op = t.ledger.iter().find(|o| o.op == mark).unwrap();
    assert!(op.reply.is_none(), "left alone: {op:#?}");
}

#[test]
fn a_resumed_ticket_is_asked_again_with_every_slot_taken() {
    let (mut env, id, earlier) = parked_and_resumed();
    let path = env.data.pipeline("Orchard");
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("slots = 2\n", "slots = 0\n");
    std::fs::write(&path, text).unwrap();
    let sessions = env.sb().sessions.len();
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    for ctx in ["backend", "frontend"] {
        let d = rerun_for(&t, ctx).unwrap_or_else(|| panic!("no rerun for {ctx}: {t:#?}"));
        assert!(!earlier.contains(&d.id), "a new id: {}", d.id);
    }
    assert_eq!(t.attempts_of("plan").count(), 2);
    assert_eq!(env.sb().sessions.len(), sessions, "nothing launched");
}

/// A request that can never be answered (lost without its body) does
/// not keep a ticket without a slot from being asked again.
#[test]
fn a_resumed_ticket_with_a_dead_request_is_asked_again_with_every_slot_taken() {
    let (mut env, id, earlier) = parked_and_resumed();
    let path = env.data.pipeline("Orchard");
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("slots = 2\n", "slots = 0\n");
    std::fs::write(&path, text).unwrap();
    let mut t = env.ticket(&id);
    let dead = t
        .ledger
        .iter_mut()
        .rev()
        .find(|o| o.kind == "session.waiting")
        .unwrap();
    dead.reply = None;
    dead.body = None;
    let now = env.tick();
    env.runner.save_ticket(&mut t, now).unwrap();
    let sessions = env.sb().sessions.len();
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    for ctx in ["backend", "frontend"] {
        let d = rerun_for(&t, ctx).unwrap_or_else(|| panic!("no rerun for {ctx}: {t:#?}"));
        assert!(!earlier.contains(&d.id), "a new id: {}", d.id);
    }
    assert_eq!(env.sb().sessions.len(), sessions, "nothing launched");
}

/// A request whose reply was lost and may not be repeated is asked
/// about once, and a `park` answer to it parks with every slot taken.
#[test]
fn a_resumed_ticket_with_a_lost_unrepeatable_request_is_asked_once_and_parks_unslotted() {
    let (mut env, id, _) = parked_and_resumed();
    let path = env.data.pipeline("Orchard");
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("slots = 2\n", "slots = 0\n");
    std::fs::write(&path, text).unwrap();
    let mut t = env.ticket(&id);
    let n = plan_of(&t, "backend").n;
    let mut lost = t.ledger.last().unwrap().clone();
    lost.op = format!("{id}-lost");
    lost.class = "non-replayable".into();
    lost.attempt = Some(("plan".into(), n));
    lost.reply = None;
    lost.error = None;
    lost.asked = false;
    t.ledger.push(lost);
    let now = env.tick();
    env.runner.save_ticket(&mut t, now).unwrap();
    let lost_sends = |t: &Ticket| -> Vec<Decision> {
        t.decisions
            .iter()
            .filter(|d| d.name == "lost-send")
            .cloned()
            .collect()
    };
    for _ in 0..2 {
        let now = env.tick();
        env.runner.step_project("Orchard", now).unwrap();
    }
    let t = env.ticket(&id);
    let asked = lost_sends(&t);
    assert_eq!(asked.len(), 1, "asked once: {t:#?}");
    assert!(rerun_for(&t, "backend").is_none(), "the question holds it");
    assert!(rerun_for(&t, "frontend").is_some(), "{t:#?}");
    let now = env.tick();
    env.runner
        .decide(&id, &asked[0].id, "park", None, now)
        .unwrap();
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    assert!(
        matches!(&t.state, TicketState::Parked { reason } if reason.contains("lost-send")),
        "{t:#?}"
    );
    assert_eq!(lost_sends(&t).len(), 1, "not asked again: {t:#?}");
}

/// A `rerun` answer still waiting for a slot when a `park` answer parks
/// the ticket is withdrawn with the questions: the resume asks about
/// its lane afresh and launches nothing on the old answer.
#[test]
fn an_answer_waiting_for_a_slot_is_withdrawn_by_an_unslotted_park() {
    let (mut env, id) = two_failed_plans();
    let path = env.data.pipeline("Orchard");
    let full = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, full.replace("slots = 2\n", "slots = 0\n")).unwrap();
    let t = env.ticket(&id);
    let waiting = rerun_for(&t, "frontend").unwrap();
    let parked_from = rerun_for(&t, "backend").unwrap();
    let now = env.tick();
    env.runner
        .decide(&id, &waiting.id, "rerun", None, now)
        .unwrap();
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    assert!(
        env.ticket(&id)
            .decisions
            .iter()
            .any(|d| d.id == waiting.id && d.unacted_answer() == Some("rerun")),
        "the answer waits for a slot"
    );
    let now = env.tick();
    env.runner
        .decide(&id, &parked_from.id, "park", None, now)
        .unwrap();
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Parked { .. }), "{t:#?}");
    assert_eq!(
        decision_state(&t, &waiting.id),
        dispatch::ticket::DecisionState::Cancelled
    );
    let earlier: Vec<String> = t.decisions.iter().map(|d| d.id.clone()).collect();
    let sessions = env.sb().sessions.len();
    let now = env.tick();
    env.runner.resume(&id, now).unwrap();
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    for ctx in ["backend", "frontend"] {
        let d = rerun_for(&t, ctx).unwrap_or_else(|| panic!("no rerun for {ctx}: {t:#?}"));
        assert!(!earlier.contains(&d.id), "a new id: {}", d.id);
    }
    // With slots free again, the old answer still launches nothing.
    std::fs::write(&path, &full).unwrap();
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    assert_eq!(t.attempts_of("plan").count(), 2, "{t:#?}");
    assert_eq!(env.sb().sessions.len(), sessions, "nothing launched");
}

/// Two answers before one pass with slots free: the lower-indexed
/// lane's `rerun` is acted on first (its attempt retired, the answer
/// marked acted), then the other's `park` parks the ticket. The acted
/// rerun has not launched its replacement, so the park withdraws it:
/// the resume asks both lanes afresh and launches nothing.
#[test]
fn a_rerun_acted_before_a_park_in_the_same_pass_launches_nothing() {
    let (mut env, id) = two_failed_plans();
    let t = env.ticket(&id);
    let mut asked = [
        rerun_for(&t, "backend").unwrap(),
        rerun_for(&t, "frontend").unwrap(),
    ];
    let index = |d: &Decision| t.decisions.iter().position(|x| x.id == d.id).unwrap();
    asked.sort_by_key(index);
    let [rerun, parked_from] = asked;
    let now = env.tick();
    env.runner
        .decide(&id, &rerun.id, "rerun", None, now)
        .unwrap();
    let now = env.tick();
    env.runner
        .decide(&id, &parked_from.id, "park", None, now)
        .unwrap();
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Parked { .. }), "{t:#?}");
    assert_eq!(
        decision_state(&t, &rerun.id),
        dispatch::ticket::DecisionState::Cancelled
    );
    let earlier: Vec<String> = t.decisions.iter().map(|d| d.id.clone()).collect();
    let sessions = env.sb().sessions.len();
    let now = env.tick();
    env.runner.resume(&id, now).unwrap();
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    for ctx in ["backend", "frontend"] {
        let d = rerun_for(&t, ctx).unwrap_or_else(|| panic!("no rerun for {ctx}: {t:#?}"));
        assert!(!earlier.contains(&d.id), "a new id: {}", d.id);
    }
    assert_eq!(t.attempts_of("plan").count(), 2, "{t:#?}");
    assert_eq!(env.sb().sessions.len(), sessions, "nothing launched");
}

/// A note at `inspect` sends one lane back, and the other lane's
/// `inspect` is answered `park` in the same pass: the park drops the
/// note before a planner carries it, so the resume asks about the
/// sent-back plan, quoting the note, and launches nothing. Answered
/// `rerun`, the new planner's prompt ends with the note.
#[test]
fn a_send_back_acted_before_a_park_in_the_same_pass_launches_nothing() {
    let (mut env, id) = workspace_env(&["type:bug"]);
    inspect_instead_of_review(&env, &id);
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    let investigator = session_of(&t, "investigate");
    std::fs::write(artifact_of(&t, "investigate", "notes"), "# notes").unwrap();
    let now = env.now;
    env.sb().stop(&investigator, now);
    env.steps_until(&id, "the lanes question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "lanes")
    });
    let lanes = env.pending(&id).remove(0);
    let now = env.tick();
    env.runner
        .decide(&id, &lanes.id, "backend, frontend", None, now)
        .unwrap();
    env.steps_until(&id, "a planner per lane", |t, _| {
        t.attempts_of("plan").filter(|a| a.is_open()).count() == 2
    });
    let t = env.ticket(&id);
    for ctx in ["backend", "frontend"] {
        let a = plan_of(&t, ctx);
        env.finish(
            a.session.as_ref().unwrap(),
            &a.artifacts["plan"],
            &format!("# {ctx} plan"),
        );
    }
    env.steps_until(&id, "an inspect question per lane", |t, _| {
        t.pending_decisions()
            .iter()
            .filter(|d| d.name == "inspect")
            .count()
            == 2
    });
    let t = env.ticket(&id);
    let asked: Vec<Decision> = t
        .pending_decisions()
        .into_iter()
        .filter(|d| d.name == "inspect")
        .cloned()
        .collect();
    let [sent_back, parked_from] = <[Decision; 2]>::try_from(asked).unwrap();
    let lane = t
        .attempts
        .iter()
        .find(|a| Some((a.stage.clone(), a.n)) == sent_back.attempt)
        .unwrap()
        .context
        .clone();
    let now = env.tick();
    env.runner
        .decide(
            &id,
            &sent_back.id,
            "rerun",
            Some("use a set, not a vec"),
            now,
        )
        .unwrap();
    let now = env.tick();
    env.runner
        .decide(&id, &parked_from.id, "park", None, now)
        .unwrap();
    let sessions = env.sb().sessions.len();
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Parked { .. }), "{t:#?}");
    assert!(t.rework.is_empty(), "the note was dropped: {:?}", t.rework);
    assert!(matches!(
        plan_of(&t, &lane).state,
        AttemptState::Cancelled { .. }
    ));
    let now = env.tick();
    env.runner.resume(&id, now).unwrap();
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    let t = env.ticket(&id);
    let d = rerun_for(&t, &lane).unwrap_or_else(|| panic!("no rerun for {lane}: {t:#?}"));
    assert!(
        d.question.contains("use a set, not a vec"),
        "{}",
        d.question
    );
    assert_eq!(t.attempts_of("plan").count(), 2, "{t:#?}");
    assert_eq!(env.sb().sessions.len(), sessions, "nothing launched");
    // Answered `rerun`, the replacement carries the quoted note.
    let now = env.tick();
    env.runner.decide(&id, &d.id, "rerun", None, now).unwrap();
    env.steps_until(&id, "a new planner", |t, _| {
        t.attempts_of("plan").count() == 3
    });
    let prompt = last_prompt_of(&env, "planner");
    assert!(
        prompt.ends_with("sent it back: use a set, not a vec"),
        "{prompt}"
    );
    assert!(env.ticket(&id).rework.is_empty());
}

/// Failing at the checks past `max_reruns` parks without a question,
/// and the resume still offers the checks again.
#[test]
fn a_resume_after_failed_checks_past_max_reruns_offers_check_again() {
    let mut env = Env::new();
    let (id, implementer) = at_implement(&mut env);
    let t = env.ticket(&id);
    with_max_reruns(&env, &t, 0);
    implementer_stops(&mut env, &id, &implementer);
    env.steps_until(&id, "the checks starting", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.gate.is_some())
    });
    let key = format!("{id}/implement/1");
    env.repo.lock().unwrap().check_exits.insert(key, 1);
    env.steps_until(&id, "parked", |t, _| {
        matches!(t.state, TicketState::Parked { .. })
    });
    let t = env.ticket(&id);
    assert!(
        !t.decisions.iter().any(|d| d.name == "rerun"),
        "the park asked nothing: {t:#?}"
    );
    let now = env.tick();
    env.runner.resume(&id, now).unwrap();
    env.steps_until(&id, "the question again", |t, _| {
        !t.pending_decisions().is_empty()
    });
    let again = env.pending(&id).remove(0);
    assert_eq!(again.options, vec!["rerun", "check", "park"]);
}

#[test]
fn a_resume_after_failed_checks_offers_check_again() {
    let mut env = Env::new();
    let (id, implementer) = at_implement(&mut env);
    implementer_stops(&mut env, &id, &implementer);
    env.steps_until(&id, "the checks starting", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.gate.is_some())
    });
    let key = format!("{id}/implement/1");
    env.repo.lock().unwrap().check_exits.insert(key.clone(), 1);
    env.steps_until(&id, "the failure", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| matches!(a.state, AttemptState::Failed { .. }))
    });
    let first = env.pending(&id).remove(0);
    let now = env.tick();
    env.runner
        .decide(&id, &first.id, "park", None, now)
        .unwrap();
    env.steps_until(&id, "parked", |t, _| {
        matches!(t.state, TicketState::Parked { .. })
    });
    let now = env.tick();
    env.runner.resume(&id, now).unwrap();
    env.steps_until(&id, "the question again", |t, _| {
        !t.pending_decisions().is_empty()
    });
    let again = env.pending(&id).remove(0);
    assert_ne!(again.id, first.id);
    assert!(again.question.contains("failed: "), "{}", again.question);
    assert_eq!(again.options, vec!["rerun", "check", "park"]);
    // The flake is gone; the checks pass on the same attempt.
    env.repo.lock().unwrap().check_exits.insert(key, 0);
    let now = env.tick();
    env.runner
        .decide(&id, &again.id, "check", None, now)
        .unwrap();
    env.steps_until(&id, "the checks passing", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.state == AttemptState::Complete)
    });
    assert_eq!(env.ticket(&id).attempts_of("implement").count(), 1);
    assert_eq!(env.sb().sessions_named("implementer").len(), 1);
}

#[test]
fn a_resume_after_a_cancelled_attempt_quotes_its_reason() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let finalize = env.pending(&id)[0].id.clone();
    let now = env.tick();
    env.runner
        .decide(&id, &finalize, "park", None, now)
        .unwrap();
    env.steps_until(&id, "parked", |t, _| {
        matches!(t.state, TicketState::Parked { .. })
    });
    let t = env.ticket(&id);
    let cancelled = t.attempts.iter().rev().find_map(|a| match &a.state {
        AttemptState::Cancelled { reason } => Some(reason.clone()),
        _ => None,
    });
    let reason = cancelled.expect("the park cancels the open attempt");
    let now = env.tick();
    env.runner.resume(&id, now).unwrap();
    env.steps_until(&id, "the question again", |t, _| {
        !t.pending_decisions().is_empty()
    });
    let again = env.pending(&id).remove(0);
    assert_eq!(again.name, "rerun");
    assert!(
        again.question.contains(&format!("was cancelled: {reason}")),
        "{}",
        again.question
    );
    assert_eq!(again.options, vec!["rerun", "park"]);
}

// --- the code review stage

/// The test pipeline with a `review-code` stage between `implement`
/// and `inspect`: a Claude reviewer, a command reviewer, the
/// implementer as the fixer, a cap of two passes, and `implement`'s
/// checks by reference.
fn review_pipeline(worktrees: &std::path::Path, dial: &str) -> String {
    pipeline(worktrees)
        .replace(
            "[operators.rebaser]\n",
            "[operators.style]\nkind = \"claude\"\nguidance = \"Hold {branch} to CLAUDE.md.\"\n\n[operators.lint]\nkind = \"command\"\nargv = [\"sh\", \"-c\", \"lint\"]\n\n[operators.rebaser]\n",
        )
        .replace(
            "[[stages]]\nname = \"inspect\"\n",
            "[[stages]]\nname = \"review-code\"\ncontext = \"each\"\nreviewers = [\"style\", \"lint\"]\nimplementer = \"implementer\"\ncap = 2\ngate = { kind = \"command\", like = \"implement\" }\n\n[[stages]]\nname = \"inspect\"\n",
        )
        .replace(
            "decisions = { lanes = \"auto\", finalize = \"ask\" }",
            &format!("decisions = {{ lanes = \"auto\", finalize = \"ask\", review-code = \"{dial}\" }}"),
        )
}

/// A ticket at the first review round: implement done, its checks
/// green, both reviewers started. The lane's base is `root0000`.
fn at_review(env: &mut Env) -> String {
    let path = env.data.pipeline(PROJECT);
    if !std::fs::read_to_string(&path)
        .unwrap()
        .contains("review-code")
    {
        env.with_review_stage("ask");
    }
    // The base is read from the clone at the cut, before anything runs.
    env.repo
        .lock()
        .unwrap()
        .bases
        .insert(env.data.repo_dir(PROJECT), "root0000".into());
    let (id, implementer) = at_implement(env);
    implementer_stops(env, &id, &implementer);
    env.steps_until(&id, "the checks starting", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.gate.is_some())
    });
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(format!("{id}/implement/1"), 0);
    env.steps_until(&id, "the review round", |t, _| {
        t.attempts_of("review-code").last().is_some_and(|a| {
            a.rounds.last().is_some_and(|r| {
                r.reviewers
                    .iter()
                    .all(|x| x.session.is_some() || x.launched)
            })
        })
    });
    id
}

/// The prompt of the newest session started under `name`.
fn last_prompt_of(env: &Env, name: &str) -> String {
    env.sb()
        .calls
        .iter()
        .rev()
        .find_map(|r| match &r.body {
            Body::SessionNew {
                prompt, name: n, ..
            } if n == name => prompt.clone(),
            _ => None,
        })
        .unwrap()
}

fn review_attempt(t: &Ticket) -> Attempt {
    t.attempts_of("review-code").last().unwrap().clone()
}

fn reviewer(t: &Ticket, round: u32, name: &str) -> dispatch::ticket::ReviewerRun {
    review_attempt(t)
        .rounds
        .iter()
        .find(|r| r.n == round)
        .and_then(|r| r.reviewers.iter().find(|x| x.name == name).cloned())
        .unwrap_or_else(|| panic!("no reviewer {name} in round {round}"))
}

fn lint_key(t: &Ticket, round: u32) -> String {
    format!("{}/review-code/{}/r{round}/lint", t.id, review_attempt(t).n)
}

fn checks_key(t: &Ticket, round: u32) -> String {
    format!(
        "{}/review-code/{}/r{round}/checks",
        t.id,
        review_attempt(t).n
    )
}

/// The command reviewer exits with `code`, having written `stdout`.
fn lint_exits(env: &mut Env, id: &str, round: u32, code: i32, stdout: &str) {
    let t = env.ticket(id);
    std::fs::write(reviewer(&t, round, "lint").feedback, stdout).unwrap();
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(lint_key(&t, round), code);
}

/// The Claude reviewer writes its feedback and stops.
fn style_says(env: &mut Env, id: &str, round: u32, text: &str) {
    let t = env.ticket(id);
    let r = reviewer(&t, round, "style");
    env.finish(&r.session.clone().unwrap(), &r.feedback, text);
}

/// Round one: both reviewers started at once against the recorded
/// base and head; no findings converges; `implement`'s checks at the
/// same clean head are reused, so the stage completes with no check
/// run and no implementer.
#[test]
fn a_review_with_no_findings_completes_at_its_head_reusing_implements_checks() {
    let mut env = Env::new();
    let id = at_review(&mut env);
    let t = env.ticket(&id);
    let a = review_attempt(&t);
    assert_eq!(a.kind, AttemptKind::Review);
    let round = &a.rounds[0];
    assert_eq!(
        (round.base.as_str(), round.head.as_str()),
        ("root0000", "base0000")
    );
    assert_eq!(
        t.lanes[0].base_sha.as_deref(),
        Some("root0000"),
        "kept on the lane"
    );
    let style = reviewer(&t, 1, "style");
    let prompt = last_prompt_of(&env, "style");
    assert!(prompt.starts_with("Hold dispatch/"), "{prompt}");
    assert!(prompt.contains("root0000..base0000"), "{prompt}");
    assert!(
        prompt.contains(&style.feedback.display().to_string()),
        "{prompt}"
    );
    assert!(prompt.contains("No findings."), "{prompt}");
    {
        let repo = env.repo.lock().unwrap();
        let (lint, _) = repo.reviewers.last().unwrap();
        assert_eq!(lint.argv, vec!["sh", "-c", "lint"]);
        assert_eq!(lint.dir, t.lanes[0].worktree);
        assert!(
            lint.env
                .contains(&("DISPATCH_BASE".to_owned(), "root0000".to_owned()))
        );
        assert!(
            lint.env
                .contains(&("DISPATCH_HEAD".to_owned(), "base0000".to_owned()))
        );
    }
    let checks_before = env.repo.lock().unwrap().checks.len();
    lint_exits(&mut env, &id, 1, 0, "all fine\n");
    style_says(&mut env, &id, 1, "No findings.");
    env.steps_until(&id, "the stage completing", |t, _| {
        review_attempt(t).state == AttemptState::Complete
    });
    let t = env.ticket(&id);
    let a = review_attempt(&t);
    assert_eq!(a.head.as_deref(), Some("base0000"));
    assert_eq!(a.rounds[0].state, RoundState::Converged);
    assert!(a.rounds[0].implementer.is_none(), "no implementer ran");
    assert!(a.gate.is_none(), "no checks of its own ran");
    assert_eq!(env.repo.lock().unwrap().checks.len(), checks_before);
    assert!(
        env.sb()
            .killed
            .contains(&reviewer(&t, 1, "style").session.unwrap())
    );
    let feedback = std::fs::read_to_string(a.artifacts["r1/feedback"].clone()).unwrap();
    assert!(feedback.contains("No findings."), "{feedback}");
    env.step();
    assert_eq!(env.ticket(&id).stage, 6, "on to inspect");
}

/// Findings: gathered with the reviewer's name and a stable id; the
/// user's `fix` starts a fresh implementer with the file. Returns the
/// environment with the implementer started, and the first feedback
/// file's path.
fn findings_asked_and_fixed() -> (Env, String, PathBuf) {
    let mut env = Env::new();
    let id = at_review(&mut env);
    lint_exits(&mut env, &id, 1, 1, "src/a.rs:3: unused import\n");
    style_says(
        &mut env,
        &id,
        1,
        "- src/x.rs: the name `tmp` says nothing\n- docs: no changelog line\n",
    );
    env.steps_until(&id, "the round question", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == "review-code")
    });
    let t = env.ticket(&id);
    let a = review_attempt(&t);
    assert_eq!(a.rounds[0].state, RoundState::Findings);
    assert_eq!(a.rounds[0].open_points, 3);
    let feedback = std::fs::read_to_string(a.rounds[0].feedback.clone().unwrap()).unwrap();
    assert!(
        feedback.contains("- r1/style-1 (style): src/x.rs: the name `tmp` says nothing"),
        "{feedback}"
    );
    assert!(
        feedback.contains("- r1/style-2 (style): docs: no changelog line"),
        "{feedback}"
    );
    assert!(
        feedback.contains("- r1/lint-1 (lint): src/a.rs:3: unused import"),
        "{feedback}"
    );
    let d = env
        .pending(&id)
        .into_iter()
        .find(|d| d.name == "review-code")
        .unwrap();
    assert_eq!(d.options, vec!["fix", "accept", "park"]);
    assert!(d.question.contains("3 point(s)"), "{}", d.question);
    let sessions_before = env.sb().sessions.len();
    let now = env.tick();
    env.runner.decide(&id, &d.id, "fix", None, now).unwrap();
    env.steps_until(&id, "the implementer", |t, _| {
        review_attempt(t).rounds[0].implementer.is_some()
    });
    let t = env.ticket(&id);
    let round = &review_attempt(&t).rounds[0];
    assert_eq!(round.state, RoundState::Fixing);
    assert_eq!(
        env.sb().sessions.len(),
        sessions_before + 1,
        "a fresh session"
    );
    let prompt = last_prompt_of(&env, "implementer");
    let feedback_path = round.feedback.clone().unwrap();
    let response = round.response.clone().unwrap();
    assert!(
        prompt.contains(&feedback_path.display().to_string()),
        "{prompt}"
    );
    assert!(prompt.contains(&response.display().to_string()), "{prompt}");
    (env, id, feedback_path)
}

/// `findings_asked_and_fixed` with the implementer's commit checked at
/// the new head and round two opened there, with the earlier file in
/// the reviewers' prompt.
fn fixed_once() -> (Env, String) {
    let (mut env, id, feedback_path) = findings_asked_and_fixed();
    let t = env.ticket(&id);
    let round = &review_attempt(&t).rounds[0];
    let fixer = round.implementer.clone().unwrap();
    let response = round.response.clone().unwrap();
    // The implementer commits and answers every point.
    let tree = t.lanes[0].worktree.clone();
    env.repo
        .lock()
        .unwrap()
        .heads
        .insert(tree.clone(), "fix00001".into());
    env.finish(
        &fixer,
        &response,
        "- r1/style-1: fixed renamed it\n- r1/style-2: fixed added the line\n- r1/lint-1: disputed the import is used by a macro\n",
    );
    env.steps_until(&id, "the checks after the fix", |t, _| {
        review_attempt(t).gate.is_some()
    });
    let t = env.ticket(&id);
    let a = review_attempt(&t);
    assert_eq!(a.rounds[0].state, RoundState::Fixed);
    assert_eq!(a.rounds[0].head_after.as_deref(), Some("fix00001"));
    assert_eq!(a.gate.as_ref().unwrap().head, "fix00001");
    assert!(env.sb().killed.contains(&fixer));
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(checks_key(&t, 1), 0);
    env.steps_until(&id, "round two", |t, _| review_attempt(t).rounds.len() == 2);
    let t = env.ticket(&id);
    let a = review_attempt(&t);
    let round2 = &a.rounds[1];
    assert_eq!(
        (round2.base.as_str(), round2.head.as_str()),
        ("root0000", "fix00001")
    );
    assert!(
        a.gate.is_none(),
        "the checks are history once the round opened"
    );
    let prompt = last_prompt_of(&env, "style");
    assert!(prompt.contains("root0000..fix00001"), "{prompt}");
    assert!(prompt.contains("withdraw <id>"), "{prompt}");
    assert!(
        prompt.contains(&feedback_path.display().to_string()),
        "{prompt}"
    );
    (env, id)
}

/// Pass two after one fix: the disputed point is kept under its id,
/// nothing new; the cap is reached, so the user is asked, and `accept`
/// runs the checks at the reviewed head and completes the stage.
#[test]
fn the_cap_offers_the_reviewed_head_and_accept_completes_at_it() {
    let (mut env, id) = fixed_once();
    lint_exits(&mut env, &id, 2, 0, "");
    style_says(&mut env, &id, 2, "keep r1/lint-1: a macro is no excuse\n");
    env.steps_until(&id, "the cap question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "review-cap")
    });
    let t = env.ticket(&id);
    let a = review_attempt(&t);
    assert_eq!(a.rounds[1].open_points, 1);
    let feedback2 = std::fs::read_to_string(a.rounds[1].feedback.clone().unwrap()).unwrap();
    assert!(
        feedback2.contains("Still open from earlier rounds"),
        "{feedback2}"
    );
    assert!(
        feedback2.contains(
            "- r1/lint-1: src/a.rs:3: unused import (kept by style: a macro is no excuse)"
        ),
        "{feedback2}"
    );
    assert!(
        !feedback2.contains("r1/style-1"),
        "fixed points are closed: {feedback2}"
    );
    let d = env
        .pending(&id)
        .into_iter()
        .find(|d| d.name == "review-cap")
        .unwrap();
    assert_eq!(d.options, vec!["accept", "more", "park"]);
    assert!(d.question.contains("round 2 of 2"), "{}", d.question);
    let now = env.tick();
    env.runner.decide(&id, &d.id, "accept", None, now).unwrap();
    env.steps_until(&id, "the final checks", |t, _| {
        review_attempt(t).gate.is_some()
    });
    let t = env.ticket(&id);
    assert_eq!(review_attempt(&t).rounds[1].state, RoundState::Accepted);
    assert_eq!(
        review_attempt(&t).gate.as_ref().unwrap().head,
        "fix00001",
        "not reused: a different head than implement's"
    );
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(checks_key(&t, 2), 0);
    env.steps_until(&id, "the stage completing", |t, _| {
        review_attempt(t).state == AttemptState::Complete
    });
    let a = review_attempt(&env.ticket(&id));
    assert_eq!(a.head.as_deref(), Some("fix00001"));
    assert_eq!(a.rounds.len(), 2, "no pass after the cap");
}

/// A disputed point a reviewer withdraws in the next pass closes; the
/// `auto` dial runs the fix pass without asking.
#[test]
fn a_withdrawn_point_closes_and_the_auto_dial_fixes_without_asking() {
    let mut env = Env::new();
    env.with_review_stage("auto");
    let id = at_review(&mut env);
    lint_exits(&mut env, &id, 1, 0, "");
    style_says(&mut env, &id, 1, "- src/x.rs: too clever\n");
    env.steps_until(&id, "the implementer without a question", |t, _| {
        review_attempt(t).rounds[0].implementer.is_some()
    });
    let t = env.ticket(&id);
    assert!(
        t.pending_decisions().is_empty(),
        "nothing waits on the user"
    );
    let round = &review_attempt(&t).rounds[0];
    let fixer = round.implementer.clone().unwrap();
    env.repo
        .lock()
        .unwrap()
        .heads
        .insert(t.lanes[0].worktree.clone(), "fix00001".into());
    env.finish(
        &fixer,
        &round.response.clone().unwrap(),
        "- r1/style-1: disputed it reads fine\n",
    );
    env.steps_until(&id, "the checks", |t, _| review_attempt(t).gate.is_some());
    let t = env.ticket(&id);
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(checks_key(&t, 1), 0);
    env.steps_until(&id, "round two", |t, _| review_attempt(t).rounds.len() == 2);
    lint_exits(&mut env, &id, 2, 0, "");
    // As a list item, the way reviewers tend to write it.
    style_says(&mut env, &id, 2, "- withdraw r1/style-1\n");
    env.steps_until(&id, "the final checks", |t, _| {
        review_attempt(t).gate.is_some()
    });
    let t = env.ticket(&id);
    assert_eq!(review_attempt(&t).rounds[1].state, RoundState::Converged);
    assert_eq!(review_attempt(&t).rounds[1].open_points, 0);
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(checks_key(&t, 2), 0);
    env.steps_until(&id, "the stage completing", |t, _| {
        review_attempt(t).state == AttemptState::Complete
    });
}

/// `review_pipeline` with a `dial` is the default `Env` pipeline for
/// these tests: `Env::new` writes `pipeline()`, so each test rewrites
/// it before taking.
impl Env {
    fn with_review_stage(&self, dial: &str) {
        let worktrees = self.data.root.join("wt");
        std::fs::write(
            self.data.pipeline(PROJECT),
            review_pipeline(&worktrees, dial),
        )
        .unwrap();
    }
}

/// A command reviewer that exits 2 and an agent reviewer that stops with
/// no feedback file each fail the round: the attempt fails into the
/// rerun question and the siblings are killed first.
#[test]
fn a_failed_reviewer_fails_the_round_after_its_siblings_are_killed() {
    let mut env = Env::new();
    let id = at_review(&mut env);
    let t = env.ticket(&id);
    let style = reviewer(&t, 1, "style").session.unwrap();
    lint_exits(&mut env, &id, 1, 2, "");
    env.steps_until(&id, "the rerun question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    let t = env.ticket(&id);
    let a = review_attempt(&t);
    assert!(
        matches!(&a.state, AttemptState::Failed { reason } if reason.contains("lint: exited 2")),
        "{:?}",
        a.state
    );
    assert!(matches!(&a.rounds[0].state, RoundState::Failed { .. }));
    assert!(
        env.sb().killed.contains(&style),
        "the sibling was retired first"
    );
    // A rerun: a new attempt, a new round; the agent stops without
    // writing anything.
    let d = env.pending(&id)[0].clone();
    let now = env.tick();
    env.runner.decide(&id, &d.id, "rerun", None, now).unwrap();
    env.steps_until(&id, "the second attempt", |t, _| {
        t.attempts_of("review-code").count() == 2
            && review_attempt(t).rounds.first().is_some_and(|r| {
                r.reviewers
                    .iter()
                    .all(|x| x.session.is_some() || x.launched)
            })
    });
    let t = env.ticket(&id);
    let style = reviewer(&t, 1, "style");
    lint_exits(&mut env, &id, 1, 0, "");
    let now = env.now;
    env.sb().stop(&style.session.clone().unwrap(), now);
    env.idle_past_grace();
    env.steps_until(&id, "the rerun question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    let a = review_attempt(&env.ticket(&id));
    assert!(
        matches!(&a.state, AttemptState::Failed { reason } if reason.contains("stopped without writing feedback")),
        "{:?}",
        a.state
    );
}

/// A Claude reviewer whose Stop leaves its card `working` is held, its
/// sibling with it, and a later write and Stop finish the round.
#[test]
fn a_reviewer_that_stops_busy_and_writes_later_completes_the_round() {
    let mut env = Env::new();
    let id = at_review(&mut env);
    let style = reviewer(&env.ticket(&id), 1, "style").session.unwrap();
    let now = env.now;
    env.sb().stop(&style, now);
    env.sb().session_mut(&style).card = "working".into();
    for _ in 0..8 {
        env.step();
    }
    let t = env.ticket(&id);
    assert_eq!(reviewer(&t, 1, "style").result, None);
    assert_eq!(reviewer(&t, 1, "lint").result, None);
    assert_eq!(review_attempt(&t).rounds[0].state, RoundState::Reviewing);
    assert!(!env.pending(&id).iter().any(|d| d.name == "rerun"));
    assert!(!env.sb().killed.contains(&style));
    // Written mid-turn: settled, but no result while it works.
    let feedback = reviewer(&env.ticket(&id), 1, "style").feedback;
    std::fs::write(&feedback, "- src/x.rs: draft\n").unwrap();
    for _ in 0..SETTLE_POLLS + 2 {
        env.step();
    }
    let t = env.ticket(&id);
    assert_eq!(reviewer(&t, 1, "style").result, None);
    assert_eq!(review_attempt(&t).rounds[0].state, RoundState::Reviewing);
    assert!(!env.sb().killed.contains(&style));
    style_says(
        &mut env,
        &id,
        1,
        "- src/x.rs: the name `tmp` says nothing\n",
    );
    lint_exits(&mut env, &id, 1, 0, "");
    env.steps_until(&id, "the round question", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == "review-code")
    });
    let t = env.ticket(&id);
    assert_eq!(
        reviewer(&t, 1, "style").result,
        Some(ReviewerResult::Findings)
    );
    assert_eq!(reviewer(&t, 1, "lint").result, Some(ReviewerResult::Clean));
    assert!(review_attempt(&t).is_open());
}

/// The round's implementer is held by a busy Stop the same way, and
/// finishes the round when it commits and answers.
#[test]
fn an_implementer_that_stops_busy_keeps_its_round() {
    let (mut env, id, _) = findings_asked_and_fixed();
    let t = env.ticket(&id);
    let round = &review_attempt(&t).rounds[0];
    let fixer = round.implementer.clone().unwrap();
    let response = round.response.clone().unwrap();
    let now = env.now;
    env.sb().stop(&fixer, now);
    env.sb().session_mut(&fixer).card = "working".into();
    for _ in 0..=STOP_IDLE_POLLS {
        env.step();
    }
    let t = env.ticket(&id);
    assert_eq!(review_attempt(&t).rounds[0].state, RoundState::Fixing);
    assert!(!env.sb().killed.contains(&fixer));
    // Answered mid-turn: settled, but the round stays open while it works.
    std::fs::write(&response, "- r1/style-1: fixed\n").unwrap();
    for _ in 0..SETTLE_POLLS + 2 {
        env.step();
    }
    let t = env.ticket(&id);
    assert_eq!(review_attempt(&t).rounds[0].state, RoundState::Fixing);
    assert_eq!(review_attempt(&t).rounds[0].head_after, None);
    assert!(!env.sb().killed.contains(&fixer));
    let tree = t.lanes[0].worktree.clone();
    env.repo
        .lock()
        .unwrap()
        .heads
        .insert(tree, "fix00001".into());
    env.finish(
        &fixer,
        &response,
        "- r1/style-1: fixed\n- r1/style-2: fixed\n- r1/lint-1: fixed\n",
    );
    env.steps_until(&id, "the round fixed", |t, _| {
        review_attempt(t).rounds[0].state == RoundState::Fixed
    });
    let t = env.ticket(&id);
    assert_eq!(
        review_attempt(&t).rounds[0].head_after.as_deref(),
        Some("fix00001")
    );
}

/// A command reviewer's exit 1 with nothing on stdout is an execution
/// error, not a finding.
#[test]
fn a_command_reviewers_exit_codes_are_read_as_the_protocol_says() {
    let mut env = Env::new();
    let id = at_review(&mut env);
    lint_exits(&mut env, &id, 1, 1, "");
    env.steps_until(&id, "the rerun question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    let a = review_attempt(&env.ticket(&id));
    assert!(
        matches!(&a.state, AttemptState::Failed { reason } if reason.contains("nothing on stdout")),
        "{:?}",
        a.state
    );
}

/// The tree changing under the reviewers voids the round; the
/// implementer leaving it dirty fails its round.
#[test]
fn a_changed_tree_voids_the_round_and_a_dirty_implementer_fails_it() {
    let mut env = Env::new();
    env.with_review_stage("auto");
    let id = at_review(&mut env);
    let tree = env.ticket(&id).lanes[0].worktree.clone();
    lint_exits(&mut env, &id, 1, 0, "");
    env.repo.lock().unwrap().dirty.push(tree.clone());
    style_says(&mut env, &id, 1, "No findings.");
    env.steps_until(&id, "the rerun question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    let a = review_attempt(&env.ticket(&id));
    assert!(
        matches!(&a.state, AttemptState::Failed { reason } if reason.contains("changed while the reviewers read it")),
        "{:?}",
        a.state
    );
    env.repo.lock().unwrap().dirty.clear();
    // Again, with findings this time; the implementer leaves the tree dirty.
    let d = env.pending(&id)[0].clone();
    let now = env.tick();
    env.runner.decide(&id, &d.id, "rerun", None, now).unwrap();
    env.steps_until(&id, "the second attempt", |t, _| {
        t.attempts_of("review-code").count() == 2
            && review_attempt(t).rounds.first().is_some_and(|r| {
                r.reviewers
                    .iter()
                    .all(|x| x.session.is_some() || x.launched)
            })
    });
    lint_exits(&mut env, &id, 1, 0, "");
    style_says(&mut env, &id, 1, "- a point\n");
    env.steps_until(&id, "the implementer", |t, _| {
        review_attempt(t).rounds[0].implementer.is_some()
    });
    let t = env.ticket(&id);
    let round = &review_attempt(&t).rounds[0];
    env.repo.lock().unwrap().dirty.push(tree);
    env.finish(
        &round.implementer.clone().unwrap(),
        &round.response.clone().unwrap(),
        "- r1/style-1: fixed\n",
    );
    // A commit in flight: the session lives, so the round waits for
    // the tree, for a while.
    for _ in 0..dispatch::ticket::DIRTY_POLLS - dispatch::ticket::SETTLE_POLLS - 1 {
        env.step();
    }
    assert!(
        env.pending(&id).is_empty(),
        "no question while the tree may still be committed"
    );
    env.steps_until(&id, "the rerun question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    let a = review_attempt(&env.ticket(&id));
    assert!(
        matches!(&a.state, AttemptState::Failed { reason } if reason.contains("left the tree")),
        "{:?}",
        a.state
    );
}

/// The implementer's response settles while its commit's hook still
/// runs; the tree is clean a little later and the round goes on.
#[test]
fn a_commit_that_lands_after_the_response_settles_is_not_a_dirty_tree() {
    let mut env = Env::new();
    env.with_review_stage("auto");
    let id = at_review(&mut env);
    let tree = env.ticket(&id).lanes[0].worktree.clone();
    lint_exits(&mut env, &id, 1, 0, "");
    style_says(&mut env, &id, 1, "- a point\n");
    env.steps_until(&id, "the implementer", |t, _| {
        review_attempt(t).rounds[0].implementer.is_some()
    });
    let t = env.ticket(&id);
    let round = &review_attempt(&t).rounds[0];
    env.repo.lock().unwrap().dirty.push(tree);
    env.finish(
        &round.implementer.clone().unwrap(),
        &round.response.clone().unwrap(),
        "- r1/style-1: fixed\n",
    );
    for _ in 0..5 {
        env.step();
    }
    let t = env.ticket(&id);
    assert!(t.pending_decisions().is_empty(), "{t:#?}");
    assert!(review_attempt(&t).rounds[0].dirty_polls > 0);
    env.repo.lock().unwrap().dirty.clear();
    env.steps_until(&id, "the round fixed", |t, _| {
        review_attempt(t).rounds[0].head_after.is_some()
    });
    assert!(env.pending(&id).is_empty());
}

/// A head that moved while a question was pending makes the answer
/// stale: the ticket parks with both heads named.
#[test]
fn an_answer_for_a_moved_head_is_stale_and_parks_the_ticket() {
    let mut env = Env::new();
    let id = at_review(&mut env);
    lint_exits(&mut env, &id, 1, 0, "");
    style_says(&mut env, &id, 1, "- a point\n");
    env.steps_until(&id, "the round question", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == "review-code")
    });
    let t = env.ticket(&id);
    env.repo
        .lock()
        .unwrap()
        .heads
        .insert(t.lanes[0].worktree.clone(), "moved001".into());
    let d = env.pending(&id)[0].clone();
    let now = env.tick();
    env.runner.decide(&id, &d.id, "fix", None, now).unwrap();
    env.steps_until(&id, "the ticket parking", |t, _| !t.active());
    let t = env.ticket(&id);
    assert!(
        matches!(&t.state, TicketState::Parked { reason } if reason.contains("base0000") && reason.contains("moved001") && reason.contains("stale")),
        "{:?}",
        t.state
    );
    assert!(
        review_attempt(&t).rounds[0].implementer.is_none(),
        "nothing launched"
    );
}

/// A command reviewer the runner lost (a restart) is failed, never
/// started again.
#[test]
fn a_lost_command_reviewer_is_failed_not_started_again() {
    let mut env = Env::new();
    let id = at_review(&mut env);
    let t = env.ticket(&id);
    let key = lint_key(&t, 1);
    env.repo.lock().unwrap().checks.retain(|c| c.key != key);
    env.restart();
    env.steps_until(&id, "the rerun question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    let a = review_attempt(&env.ticket(&id));
    assert!(
        matches!(&a.state, AttemptState::Failed { reason } if reason.contains("lint: lost")),
        "{:?}",
        a.state
    );
    assert!(
        !env.repo.lock().unwrap().checks.iter().any(|c| c.key == key),
        "not started again"
    );
}

/// A plan sits while main moves: when the implementer's stage begins,
/// the branch (no commits of its own yet) is brought up to the base,
/// the lane's base is the new one, and the implementer is told the
/// base moved. Nothing but git ran.
#[test]
fn a_plan_that_sat_is_implemented_on_a_branch_brought_up_to_its_base() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let t = env.ticket(&id);
    let tree = t.lanes[0].worktree.clone();
    let clone = env.runner.data.repo_dir(PROJECT);
    {
        let mut repo = env.repo.lock().unwrap();
        repo.bases.insert(clone, "main0002".into());
        repo.behind.insert(tree.clone(), 3);
    }
    let decision = env.pending(&id)[0].id.clone();
    let now = env.tick();
    env.runner
        .decide(&id, &decision, "finalize", None, now)
        .unwrap();
    env.steps_until(&id, "the implementer", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(Attempt::is_open)
    });
    let t = env.ticket(&id);
    assert_eq!(
        env.repo.lock().unwrap().rebased,
        vec![(tree, "origin/main".to_owned())]
    );
    assert_eq!(t.lanes[0].base_sha.as_deref(), Some("main0002"));
    let moved = t.lanes[0].refreshed.clone().expect("the move is recorded");
    assert_eq!(
        (moved.from.as_str(), moved.to.as_str()),
        ("base0000", "main0002")
    );
    assert!(
        t.attempts_of(dispatch::scheduler::REFRESH).next().is_none(),
        "a clean rebase launches nothing"
    );
    let prompt = last_prompt_of(&env, "implementer");
    assert!(
        prompt.contains("base moved from base0000 to main0002"),
        "{prompt}"
    );
    assert!(
        env.repo.lock().unwrap().pushed.is_empty(),
        "no pull request yet, so nothing is pushed"
    );
}

/// The branch has commits and the mechanical rebase stops: the
/// policy's rebaser is continued from the lane's last finished agent,
/// told the base and the checks, and the stage waits for it; when it
/// stops the branch is read again and the stage goes on.
#[test]
fn a_conflicting_refresh_is_rebased_by_a_clone_of_the_lanes_last_agent() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let t = env.ticket(&id);
    let tree = t.lanes[0].worktree.clone();
    let planner = session_of(&t, "plan");
    let clone = env.runner.data.repo_dir(PROJECT);
    {
        let mut repo = env.repo.lock().unwrap();
        repo.bases.insert(clone, "main0002".into());
        repo.behind.insert(tree.clone(), 2);
        repo.rebase_conflicts.push(tree.clone());
    }
    let decision = env.pending(&id)[0].id.clone();
    let now = env.tick();
    env.runner
        .decide(&id, &decision, "finalize", None, now)
        .unwrap();
    env.steps_until(&id, "the rebaser", |t, _| {
        t.attempts_of(dispatch::scheduler::REFRESH)
            .any(|a| a.session.is_some())
    });
    let t = env.ticket(&id);
    let rebase = t
        .attempts_of(dispatch::scheduler::REFRESH)
        .last()
        .unwrap()
        .clone();
    assert!(rebase.is_open());
    assert_eq!(rebase.context, "repo");
    let rebaser = rebase.session.clone().unwrap();
    assert_eq!(
        env.sb().cloned,
        vec![(planner, rebaser.clone())],
        "cloned from the planner, the lane's last finished agent"
    );
    let prompt = env
        .sb()
        .calls
        .iter()
        .rev()
        .find_map(|r| match &r.body {
            Body::SessionClone { prompt, name, .. } if name == "rebaser" => Some(prompt.clone()),
            _ => None,
        })
        .unwrap();
    assert!(prompt.contains("behind origin/main"), "{prompt}");
    assert!(
        prompt.contains("The checks are: sh -c cargo test"),
        "{prompt}"
    );
    assert!(prompt.contains("abort the rebase"), "{prompt}");
    assert!(
        t.attempts_of("implement").next().is_none(),
        "the implementer waits for the rebaser"
    );
    // The rebaser resolved it and stopped.
    {
        let mut repo = env.repo.lock().unwrap();
        repo.rebase_conflicts.clear();
        repo.behind.remove(&tree);
    }
    env.finish(
        &rebaser,
        &rebase.artifacts["notes"].clone(),
        "# rebased\nkept both sides of the scheduler change",
    );
    env.steps_until(&id, "the implementer", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(Attempt::is_open)
    });
    let t = env.ticket(&id);
    assert_eq!(t.lanes[0].base_sha.as_deref(), Some("main0002"));
    assert!(t.pending_decisions().is_empty());
    assert!(
        env.repo.lock().unwrap().pushed.is_empty(),
        "no pull request yet, so nothing is pushed"
    );
}

/// The ticket at `inspect`, its PR reported at `pr_head`, with main
/// moved and the tree `behind` it: the next stage, `ready`, refreshes a
/// branch that has a pull request. A clean rebase leaves `rebased1`.
fn main_moved_before_ready(env: &mut Env, pr_head: &str, behind: u64) -> (String, PathBuf) {
    let id = at_inspect(env);
    let tree = env.ticket(&id).lanes[0].worktree.clone();
    let clone = env.runner.data.repo_dir(PROJECT);
    {
        let mut repo = env.repo.lock().unwrap();
        repo.bases.insert(clone, "main0002".into());
        repo.behind.insert(tree.clone(), behind);
        repo.rebase_heads.insert(tree.clone(), "rebased1".into());
    }
    // Set before `ready` begins, so its first reading finds the PR.
    env.pr_is(&id, pr_head, "open", Checks::Passed);
    (id, tree)
}

/// Main moved after the PR was opened: the refresh at `ready` rebases
/// the branch and pushes it once, leased on the head the lane's records
/// last saw, and `ready` reads the PR at the tree's head with no
/// question.
#[test]
fn a_refresh_at_ready_pushes_the_rebased_branch_once_with_the_lease() {
    let mut env = Env::new();
    let (id, tree) = main_moved_before_ready(&mut env, "rebased1", 1);
    env.inspect(&id, "proceed", None);
    env.steps_until(&id, "the ready attempt done", |t, _| {
        t.attempts_of("ready").last().is_some_and(|a| !a.is_open())
    });
    let t = env.ticket(&id);
    let branch = t.lanes[0].branch.clone();
    assert_eq!(
        env.repo.lock().unwrap().pushed,
        vec![(tree, "origin".into(), branch, "base0000".into())]
    );
    assert_eq!(t.lanes[0].base_sha.as_deref(), Some("main0002"));
    assert!(t.pending_decisions().is_empty(), "{t:#?}");
    let ready = t.attempts_of("ready").last().unwrap();
    assert_eq!(ready.state, AttemptState::Complete);
    assert_eq!(ready.head.as_deref(), Some("rebased1"));
    for _ in 0..3 {
        env.step();
    }
    assert_eq!(env.repo.lock().unwrap().pushed.len(), 1, "pushed once");
}

/// Someone else pushed to the PR meanwhile: the lease refuses, and
/// `ready` asks its `pr` question about the head.
#[test]
fn a_refused_lease_at_ready_asks_the_pr_question() {
    let mut env = Env::new();
    let (id, tree) = main_moved_before_ready(&mut env, "base0000", 1);
    env.repo.lock().unwrap().lease_stale.push(tree);
    env.inspect(&id, "proceed", None);
    env.steps_until(&id, "the pr question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "pr")
    });
    assert_eq!(env.repo.lock().unwrap().pushed.len(), 1);
    let d = env
        .pending(&id)
        .into_iter()
        .find(|d| d.name == "pr")
        .unwrap();
    assert!(
        d.question
            .contains("PR #7 is at base0000 but the tree is at rebased1"),
        "{}",
        d.question
    );
    assert_eq!(d.options, vec!["recheck", "park"]);
}

/// The same refresh with no pull request for the branch: nothing is
/// pushed, and `ready` asks for one to be opened.
#[test]
fn a_refresh_at_ready_without_a_pull_request_pushes_nothing() {
    let mut env = Env::new();
    let (id, _) = main_moved_before_ready(&mut env, "rebased1", 1);
    env.prs.lock().unwrap().prs.clear();
    env.inspect(&id, "proceed", None);
    env.steps_until(&id, "the pr question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "pr")
    });
    let t = env.ticket(&id);
    assert_eq!(t.lanes[0].base_sha.as_deref(), Some("main0002"));
    assert!(env.repo.lock().unwrap().pushed.is_empty());
    let d = env
        .pending(&id)
        .into_iter()
        .find(|d| d.name == "pr")
        .unwrap();
    assert!(d.question.contains("no pull request"), "{}", d.question);
}

/// A conflicting refresh at `ready`: the rebaser is told not to push,
/// and once it has brought the branch up Dispatch pushes it with the
/// lease before `ready` reads the PR.
#[test]
fn a_refresh_at_ready_pushes_after_the_rebaser_resolves_it() {
    let mut env = Env::new();
    let (id, tree) = main_moved_before_ready(&mut env, "rebased2", 2);
    env.repo.lock().unwrap().rebase_conflicts.push(tree.clone());
    env.inspect(&id, "proceed", None);
    env.steps_until(&id, "the rebaser", |t, _| {
        t.attempts_of(dispatch::scheduler::REFRESH)
            .any(|a| a.session.is_some())
    });
    assert!(env.repo.lock().unwrap().pushed.is_empty());
    let prompt = env
        .sb()
        .calls
        .iter()
        .rev()
        .find_map(|r| match &r.body {
            Body::SessionClone { prompt, name, .. } if name == "rebaser" => Some(prompt.clone()),
            Body::SessionNew { prompt, name, .. } if name == "rebaser" => prompt.clone(),
            _ => None,
        })
        .unwrap();
    assert!(prompt.contains("do not push"), "{prompt}");
    assert!(!prompt.contains("--force-with-lease"), "{prompt}");
    let t = env.ticket(&id);
    let rebase = t
        .attempts_of(dispatch::scheduler::REFRESH)
        .last()
        .unwrap()
        .clone();
    {
        let mut repo = env.repo.lock().unwrap();
        repo.rebase_conflicts.clear();
        repo.behind.remove(&tree);
        repo.heads.insert(tree.clone(), "rebased2".into());
    }
    env.finish(
        &rebase.session.clone().unwrap(),
        &rebase.artifacts["notes"].clone(),
        "# rebased\nkept both sides",
    );
    env.steps_until(&id, "the ready attempt", |t, _| {
        t.attempts_of("ready").next().is_some()
    });
    let branch = env.ticket(&id).lanes[0].branch.clone();
    assert_eq!(
        env.repo.lock().unwrap().pushed,
        vec![(tree, "origin".into(), branch, "base0000".into())]
    );
}

/// Without a rebaser in the policy a conflicting refresh is a question
/// with `recheck`, answered after the user rebased by hand.
#[test]
fn a_conflicting_refresh_without_a_rebaser_is_a_question() {
    let mut env = Env::new();
    let path = env.data.pipeline(PROJECT);
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("rebaser = \"rebaser\"\n", "")).unwrap();
    let id = at_finalize(&mut env);
    let t = env.ticket(&id);
    let tree = t.lanes[0].worktree.clone();
    let clone = env.runner.data.repo_dir(PROJECT);
    {
        let mut repo = env.repo.lock().unwrap();
        repo.bases.insert(clone, "main0002".into());
        repo.behind.insert(tree.clone(), 2);
        repo.rebase_conflicts.push(tree.clone());
    }
    let decision = env.pending(&id)[0].id.clone();
    let now = env.tick();
    env.runner
        .decide(&id, &decision, "finalize", None, now)
        .unwrap();
    env.steps_until(&id, "the refresh question", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == dispatch::scheduler::REFRESH)
    });
    let d = env.pending(&id)[0].clone();
    assert_eq!(d.options, vec!["recheck", "park"]);
    assert!(d.question.contains("names no rebaser"), "{}", d.question);
    assert!(t.attempts_of("implement").next().is_none());
    {
        let mut repo = env.repo.lock().unwrap();
        repo.rebase_conflicts.clear();
        repo.behind.remove(&tree);
    }
    let now = env.tick();
    env.runner.decide(&id, &d.id, "recheck", None, now).unwrap();
    env.steps_until(&id, "the implementer", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(Attempt::is_open)
    });
    assert_eq!(
        env.ticket(&id).lanes[0].base_sha.as_deref(),
        Some("main0002")
    );
}

/// Work in the tree, a rebase by hand say, is never rebased over: the
/// refresh leaves that lane alone and the stage goes on.
#[test]
fn a_refresh_leaves_a_tree_with_work_in_it_alone() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let t = env.ticket(&id);
    let tree = t.lanes[0].worktree.clone();
    let clone = env.runner.data.repo_dir(PROJECT);
    {
        let mut repo = env.repo.lock().unwrap();
        repo.bases.insert(clone, "main0002".into());
        repo.behind.insert(tree.clone(), 2);
        repo.dirty.push(tree.clone());
    }
    let decision = env.pending(&id)[0].id.clone();
    let now = env.tick();
    env.runner
        .decide(&id, &decision, "finalize", None, now)
        .unwrap();
    env.steps_until(&id, "the implementer", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(Attempt::is_open)
    });
    let t = env.ticket(&id);
    assert!(
        env.repo.lock().unwrap().rebased.is_empty(),
        "nothing rebased"
    );
    assert_eq!(t.lanes[0].base_sha.as_deref(), Some("base0000"));
    assert!(t.attempts_of(dispatch::scheduler::REFRESH).next().is_none());
}

// --- closing a ticket: by hand or at the pipeline's end, a sequence from
// a saved intent; the worktrees removed lanes first and never forced,
// the branch, the ticket directory and the record kept.

/// A ticket whose first agent vanished: active, at a `rerun` decision,
/// nothing open, its session marked waiting.
fn at_rerun(env: &mut Env, id: &str) -> String {
    env.step();
    let investigator = session_of(&env.ticket(id), "investigate");
    env.sb().vanish(&investigator);
    env.step();
    let t = env.ticket(id);
    assert_eq!(env.pending(id)[0].name, "rerun", "{t:#?}");
    assert!(!t.attempts.iter().any(Attempt::is_open));
    investigator
}

/// The workspace ticket, parked from its `rerun` decision.
fn parked_workspace() -> (Env, String) {
    let (mut env, id) = workspace_env(&["area:backend"]);
    at_rerun(&mut env, &id);
    let d = env.pending(&id)[0].id.clone();
    let now = env.tick();
    env.runner.decide(&id, &d, "park", None, now).unwrap();
    env.step();
    assert!(matches!(env.ticket(&id).state, TicketState::Parked { .. }));
    (env, id)
}

/// The record as a close cut short would have left it.
fn write_closing(env: &Env, id: &str, edit: impl FnOnce(&mut Ticket)) {
    let mut t = env.ticket(id);
    t.state = TicketState::Closing {
        reason: "closed by hand".into(),
    };
    edit(&mut t);
    env.runner.save_ticket(&mut t, env.now).unwrap();
}

fn workspace_trees(env: &Env, id: &str) -> [(PathBuf, PathBuf); 3] {
    let tree = env.worktrees.join(id);
    [
        (
            env.data.repo_dir("Orchard@backend"),
            tree.join("orchard-backend"),
        ),
        (
            env.data.repo_dir("Orchard@frontend"),
            tree.join("orchard-frontend"),
        ),
        (env.data.repo_dir("Orchard"), tree),
    ]
}

#[test]
fn a_closing_tickets_pending_decision_takes_no_answer() {
    let (mut env, id) = workspace_env(&["area:backend"]);
    at_rerun(&mut env, &id);
    let d = env.pending(&id)[0].id.clone();
    write_closing(&env, &id, |_| {});
    let now = env.tick();
    let err = env.runner.decide(&id, &d, "rerun", None, now).unwrap_err();
    assert!(err.to_string().contains("is closing"), "{err:#}");
    let t = env.ticket(&id);
    assert!(t.decisions.iter().find(|x| x.id == d).unwrap().pending());
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Closed { .. }), "{t:#?}");
    assert!(
        t.decisions
            .iter()
            .all(|d| d.state == dispatch::ticket::DecisionState::Cancelled)
    );
}

#[test]
fn closing_by_hand_removes_the_lanes_then_the_tree_and_keeps_the_rest() {
    let (mut env, id) = workspace_env(&["area:backend"]);
    let investigator = at_rerun(&mut env, &id);
    assert!(env.sb().session(&investigator).waiting);
    let artifact = artifact_of(&env.ticket(&id), "investigate", "notes");
    std::fs::write(&artifact, "# half").unwrap();
    let now = env.tick();
    let t = env.runner.close_by_hand(&id, None, now).unwrap();
    assert!(
        matches!(&t.state, TicketState::Closed { reason } if reason == "closed by hand"),
        "{t:#?}"
    );
    assert_eq!(env.repo.lock().unwrap().removed, workspace_trees(&env, &id));
    assert!(!env.worktrees.join(&id).exists());
    let t = env.ticket(&id);
    assert!(t.close.decisions_cancelled && t.close.waiting_cleared && t.close.card_cleared);
    assert!(t.close.tree_removed && t.close.trees_kept.is_none());
    assert!(t.lanes.iter().all(|l| l.removed));
    assert_eq!(
        t.tree.as_deref(),
        Some(env.worktrees.join(&id).as_path()),
        "the path stays on the record"
    );
    assert!(
        t.decisions
            .iter()
            .all(|d| d.state == dispatch::ticket::DecisionState::Cancelled)
    );
    assert!(env.data.ticket_dir(&id).exists() && artifact.exists());
    let ps = env.runner.load_project("Orchard").unwrap();
    assert!(!ps.queue.contains(&id) && !ps.closing.contains(&id));
    assert!(ps.shown.is_empty());
    let sb = env.sb();
    assert_eq!(sb.waiting[&investigator], (false, String::new()));
    assert!(sb.sets[0].items.is_empty(), "the card is off the set");
    assert_eq!(sb.projects.len(), 1, "the Switchboard project stays");
}

#[test]
fn a_parked_ticket_closes_with_the_reason_given_and_is_never_stepped_again() {
    let (mut env, id) = parked_workspace();
    let now = env.tick();
    let t = env
        .runner
        .close_by_hand(&id, Some("fixed upstream"), now)
        .unwrap();
    assert!(matches!(&t.state, TicketState::Closed { reason } if reason == "fixed upstream"));
    assert_eq!(env.repo.lock().unwrap().removed.len(), 3);
    let calls = env.sb().calls.len();
    env.step();
    assert_eq!(env.sb().calls.len(), calls, "a closed ticket is quiet");
    let e = env.runner.close_by_hand(&id, None, env.now).unwrap_err();
    assert!(e.to_string().contains("already closed"), "{e:#}");
}

#[test]
fn a_ticket_whose_pipeline_copy_is_unreadable_closes_and_keeps_its_trees() {
    let (mut env, id) = parked_workspace();
    let t = env.ticket(&id);
    std::fs::remove_file(&t.pipeline_file).unwrap();
    let now = env.tick();
    let t = env.runner.close_by_hand(&id, None, now).unwrap();
    assert!(matches!(t.state, TicketState::Closed { .. }), "{t:#?}");
    let kept = t.close.trees_kept.as_deref().unwrap_or_default();
    assert!(kept.contains("pipeline copy unreadable"), "{kept}");
    assert!(env.repo.lock().unwrap().removed.is_empty());
}

/// A refusal writes nothing and removes nothing.
fn refused(env: &mut Env, id: &str, why: &str) {
    let before = std::fs::read(env.data.ticket_file(id)).unwrap();
    let project = env.ticket(id).project;
    let ps = env.runner.load_project(&project).unwrap();
    let now = env.tick();
    let e = env.runner.close_by_hand(id, None, now).unwrap_err();
    assert!(format!("{e:#}").contains(why), "{e:#}");
    assert_eq!(std::fs::read(env.data.ticket_file(id)).unwrap(), before);
    assert_eq!(env.runner.load_project(&project).unwrap(), ps);
    assert!(env.repo.lock().unwrap().removed.is_empty());
}

#[test]
fn a_close_is_refused_while_anything_runs_or_a_tree_has_changes() {
    let mut env = Env::new();
    let id = at_inspect(&mut env);
    refused(&mut env, &id, "park it first");
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    refused(&mut env, &id, "park it first");

    let (mut env, id) = parked_workspace();
    let mut t = env.ticket(&id);
    t.state = TicketState::Parking {
        reason: "by hand".into(),
    };
    env.runner.save_ticket(&mut t, env.now).unwrap();
    refused(&mut env, &id, "still parking");

    let (mut env, id) = parked_workspace();
    let tree = env.worktrees.join(&id);
    env.repo
        .lock()
        .unwrap()
        .changes
        .insert(tree.clone(), vec!["notes.txt".into()]);
    refused(&mut env, &id, "commit or clean it");
    // The lanes are nested repositories, not changes of the tree.
    env.repo.lock().unwrap().changes.insert(
        tree,
        vec!["orchard-backend".into(), "orchard-frontend".into()],
    );
    let now = env.tick();
    env.runner.close_by_hand(&id, None, now).unwrap();
}

#[test]
fn a_pipeline_that_works_in_place_closes_and_removes_nothing() {
    let mut env = Env::new();
    let root = env.worktrees.join("checkout");
    let text = std::fs::read_to_string(env.data.pipeline(PROJECT))
        .unwrap()
        .replace(
            "repo = \"git@github.com:msull/switchboard.git\"",
            &format!("root = \"{}\"", root.display()),
        );
    std::fs::write(env.data.pipeline(PROJECT), &text).unwrap();
    let id = env.take(7).id;
    let now = env.tick();
    let t = env.runner.close_by_hand(&id, None, now).unwrap();
    assert!(matches!(t.state, TicketState::Closed { .. }), "{t:#?}");
    assert!(env.repo.lock().unwrap().removed.is_empty());
    assert!(env.sb().calls.is_empty(), "nothing made, nothing to undo");
}

#[test]
fn a_close_cut_off_before_the_project_save_finishes_and_clears_the_set_once_answered() {
    let mut env = Env::new();
    let id = env.take(7).id;
    let investigator = at_rerun(&mut env, &id);
    assert_eq!(env.sb().sets[0].items.len(), 1);
    write_closing(&env, &id, |_| {});
    assert!(
        env.runner
            .load_project(PROJECT)
            .unwrap()
            .queue
            .contains(&id)
    );
    let sessions = env.sb().sessions.len();
    env.sb().drop_reply_for = Some("set.sync".into());
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Closing { .. }), "{t:#?}");
    assert!(!t.close.card_cleared, "no reply, no flag");
    let ps = env.runner.load_project(PROJECT).unwrap();
    assert_eq!(
        (ps.queue.is_empty(), ps.closing.clone()),
        (true, vec![id.clone()])
    );
    let sync = t.ledger.iter().rfind(|o| o.kind == "set.sync").unwrap();
    assert!(sync.reply.is_none(), "on the closing ticket's own ledger");
    assert!(env.runner.resume(&id, env.now).is_err());
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Closed { .. }), "{t:#?}");
    assert!(t.close.card_cleared);
    assert!(t.ledger.iter().all(|o| o.reply.is_some()));
    let ps = env.runner.load_project(PROJECT).unwrap();
    assert!(ps.queue.is_empty() && ps.closing.is_empty() && ps.shown.is_empty());
    let sb = env.sb();
    assert!(sb.sets[0].items.is_empty());
    assert_eq!(sb.sessions.len(), sessions, "nothing started while closing");
    assert!(!sb.session(&investigator).waiting);
}

/// A closing ticket that still holds the set's only card leaves it to
/// its own close: the pass's sync has no other ticket to write under,
/// and that is not an error to report on every pass.
#[test]
fn a_card_held_only_by_a_closing_ticket_is_left_to_its_close() {
    let mut env = Env::new();
    let id = env.take(7).id;
    at_rerun(&mut env, &id);
    write_closing(&env, &id, |_| {});
    let mut ps = env.runner.load_project(PROJECT).unwrap();
    ps.queue.retain(|q| q != &id);
    ps.closing.push(id.clone());
    let calls = env.sb().calls.len();
    let now = env.tick();
    dispatch::view::sync_queue(&mut env.runner, &mut ps, PROJECT, &[], None, now).unwrap();
    assert_eq!(ps.shown.len(), 1, "the card stays for the close to clear");
    assert_eq!(env.sb().calls.len(), calls, "nothing asked");
}

#[test]
fn a_closing_ticket_before_its_first_stage_cuts_nothing() {
    let (mut env, id) = workspace_env(&["area:backend"]);
    write_closing(&env, &id, |_| {});
    env.step();
    assert!(matches!(env.ticket(&id).state, TicketState::Closed { .. }));
    let repo = env.repo.lock().unwrap();
    assert!(repo.worktrees.is_empty() && repo.removed.is_empty());
    assert!(env.sb().sessions.is_empty());
}

#[test]
fn a_lane_removed_before_its_flag_was_saved_is_removed_again_then_the_tree() {
    let (mut env, id) = parked_workspace();
    let [backend, _, _] = workspace_trees(&env, &id);
    std::fs::remove_dir_all(&backend.1).unwrap();
    write_closing(&env, &id, |t| t.close.decisions_cancelled = true);
    env.step();
    assert!(matches!(env.ticket(&id).state, TicketState::Closed { .. }));
    assert_eq!(env.repo.lock().unwrap().removed, workspace_trees(&env, &id));
}

#[test]
fn a_close_cut_off_between_the_lanes_and_the_tree_removes_only_the_tree() {
    let (mut env, id) = parked_workspace();
    let [backend, frontend, tree] = workspace_trees(&env, &id);
    for (_, dir) in [&backend, &frontend] {
        std::fs::remove_dir_all(dir).unwrap();
    }
    write_closing(&env, &id, |t| {
        for lane in &mut t.lanes {
            lane.removed = true;
        }
    });
    env.step();
    assert!(matches!(env.ticket(&id).state, TicketState::Closed { .. }));
    assert_eq!(env.repo.lock().unwrap().removed, [tree]);
}

#[test]
fn a_close_cut_off_after_every_step_only_writes_closed() {
    let (mut env, id) = parked_workspace();
    write_closing(&env, &id, |t| {
        t.close.decisions_cancelled = true;
        t.close.waiting_cleared = true;
        t.close.tree_removed = true;
        t.close.card_cleared = true;
        for lane in &mut t.lanes {
            lane.removed = true;
        }
    });
    let calls = env.sb().calls.len();
    env.step();
    assert!(matches!(env.ticket(&id).state, TicketState::Closed { .. }));
    assert!(env.repo.lock().unwrap().removed.is_empty());
    // The project was never saved with the card gone, so the set is
    // redrawn from it once; nothing else is asked.
    let sb = env.sb();
    let asked: Vec<String> = sb.calls[calls..]
        .iter()
        .filter(|r| r.body.is_command())
        .map(|r| r.body.kind())
        .collect();
    assert_eq!(asked, vec!["set.sync".to_owned()]);
    assert!(sb.sets[0].items.is_empty());
}

#[test]
fn a_waiting_mark_is_cleared_after_a_stop_and_only_counted_once_answered() {
    // Cancelled and saved, the mark never sent.
    let mut env = Env::new();
    let id = env.take(7).id;
    let investigator = at_rerun(&mut env, &id);
    write_closing(&env, &id, |t| {
        for d in &mut t.decisions {
            d.state = dispatch::ticket::DecisionState::Cancelled;
        }
        t.close.decisions_cancelled = true;
    });
    env.step();
    assert!(!env.sb().session(&investigator).waiting);
    assert!(env.ticket(&id).close.waiting_cleared);

    // Sent, its reply lost: not counted until the ledger has a reply.
    let mut env = Env::new();
    let id = env.take(7).id;
    let investigator = at_rerun(&mut env, &id);
    env.sb().drop_reply_for = Some("session.waiting".into());
    let now = env.tick();
    let e = env.runner.close_by_hand(&id, None, now).unwrap_err();
    assert!(format!("{e:#}").contains("is closing"), "{e:#}");
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Closing { .. }));
    assert!(t.close.decisions_cancelled && !t.close.waiting_cleared);
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Closed { .. }), "{t:#?}");
    assert!(t.close.waiting_cleared && t.ledger.iter().all(|o| o.reply.is_some()));
    assert!(!env.sb().session(&investigator).waiting);
}

#[test]
fn a_closed_ticket_left_in_the_closing_list_is_dropped_by_the_next_pass() {
    let (mut env, id) = parked_workspace();
    let now = env.tick();
    env.runner.close_by_hand(&id, None, now).unwrap();
    let mut ps = env.runner.load_project("Orchard").unwrap();
    ps.closing.push(id.clone());
    env.runner.save_project(&ps).unwrap();
    env.step();
    assert!(
        env.runner
            .load_project("Orchard")
            .unwrap()
            .closing
            .is_empty()
    );
}

#[test]
fn a_restart_during_a_close_with_a_lost_launch_still_closes() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.step();
    {
        let mut sb = env.sb();
        sb.sessions.clear();
        sb.log.clear();
        sb.resumable.clear();
    }
    write_closing(&env, &id, lost_launch_with_reruns_spent);
    let mut ps = env.runner.load_project(PROJECT).unwrap();
    ps.queue.retain(|q| q != &id);
    ps.closing.push(id.clone());
    env.runner.save_project(&ps).unwrap();
    env.restart();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Closing { .. }), "{t:#?}");
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Closed { .. }), "{t:#?}");
    let ps = env.runner.load_project(PROJECT).unwrap();
    assert!(ps.queue.is_empty() && ps.closing.is_empty());
    assert!(env.sb().sessions.is_empty(), "nothing launched");
}

#[test]
fn a_ticket_on_the_closing_list_that_is_not_closing_goes_back_to_the_queue() {
    let (mut env, id) = parked_workspace();
    let mut ps = env.runner.load_project("Orchard").unwrap();
    ps.queue.retain(|q| q != &id);
    ps.closing.push(id.clone());
    env.runner.save_project(&ps).unwrap();
    env.step();
    let ps = env.runner.load_project("Orchard").unwrap();
    assert_eq!(
        (ps.queue.clone(), ps.closing.is_empty()),
        (vec![id.clone()], true)
    );
    assert!(matches!(env.ticket(&id).state, TicketState::Parked { .. }));
}

#[test]
fn a_close_waits_for_a_launch_in_flight_and_kills_what_it_brought_up() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.sb().in_progress_for = Some("session.new".into());
    env.step();
    assert_eq!(env.sb().kinds_called("session.new"), 1);
    // A record whose attempt was cancelled while its launch was still
    // in flight: the close must wait for that launch all the same.
    let mut t = env.ticket(&id);
    for a in &mut t.attempts {
        assert!(matches!(a.state, AttemptState::Starting), "{a:#?}");
        a.state = AttemptState::Cancelled {
            reason: "parked".into(),
        };
    }
    t.state = TicketState::Parked {
        reason: "parked by hand".into(),
    };
    env.runner.save_ticket(&mut t, env.now).unwrap();
    let now = env.tick();
    let t = env.runner.close_by_hand(&id, None, now).unwrap();
    assert!(matches!(t.state, TicketState::Closing { .. }), "{t:#?}");
    assert!(env.sb().killed.is_empty(), "{:?}", env.sb().killed);
    env.sb().in_progress_for = None;
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Closed { .. }), "{t:#?}");
    let investigator = session_of(&t, "investigate");
    assert!(env.sb().killed.contains(&investigator));
    assert_eq!(env.sb().kinds_called("session.new"), 1);
    assert!(
        t.attempts
            .iter()
            .all(|a| matches!(a.state, AttemptState::Cancelled { .. })),
        "{:#?}",
        t.attempts
    );
}

#[test]
fn parking_waits_for_a_launch_in_flight_and_kills_what_it_brought_up() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.sb().in_progress_for = Some("session.new".into());
    env.step();
    let mut t = env.ticket(&id);
    t.state = TicketState::Parking {
        reason: "parked by hand".into(),
    };
    let now = env.tick();
    env.runner.save_ticket(&mut t, now).unwrap();
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Parking { .. }), "{t:#?}");
    assert!(t.attempts[0].is_open(), "not cancelled while in flight");
    env.sb().in_progress_for = None;
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Parked { .. }), "{t:#?}");
    let investigator = session_of(&t, "investigate");
    assert!(env.sb().killed.contains(&investigator));
    assert!(matches!(
        t.attempts[0].state,
        AttemptState::Cancelled { .. }
    ));
    assert!(t.pending_decisions().is_empty(), "{:#?}", t.decisions);
    assert_eq!(env.sb().kinds_called("session.new"), 1);
}

#[test]
fn a_late_lost_launch_leaves_a_cancelled_attempt_and_a_parked_ticket_alone() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.sb().in_progress_for = Some("session.new".into());
    env.step();
    let mut t = env.ticket(&id);
    for a in &mut t.attempts {
        a.state = AttemptState::Cancelled {
            reason: "parked".into(),
        };
    }
    t.state = TicketState::Parked {
        reason: "parked by hand".into(),
    };
    env.runner.save_ticket(&mut t, env.now).unwrap();
    {
        let mut sb = env.sb();
        let made: Vec<String> = sb.sessions.iter().map(|s| s.id.clone()).collect();
        sb.interrupted.extend(made);
    }
    env.restart();
    let t = env.ticket(&id);
    assert!(
        matches!(&t.state, TicketState::Parked { reason } if reason == "parked by hand"),
        "{t:#?}"
    );
    assert!(matches!(
        t.attempts[0].state,
        AttemptState::Cancelled { .. }
    ));
    assert!(t.pending_decisions().is_empty(), "{:#?}", t.decisions);
}

#[test]
fn a_lost_send_raises_its_decision_once_however_often_the_close_runs() {
    let mut env = Env::new();
    let id = env.take(7).id;
    let investigator = at_rerun(&mut env, &id);
    write_closing(&env, &id, |t| {
        let mut lost = t
            .ledger
            .iter()
            .rfind(|o| o.kind == "session.new")
            .unwrap()
            .clone();
        lost.op = format!("{id}-lostsend");
        lost.kind = "session.input".into();
        lost.class = "non-replayable".into();
        lost.body = None;
        lost.reply = None;
        lost.error = None;
        t.ledger.push(lost);
    });
    env.sb().drop_reply_for = Some("set.sync".into());
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Closing { .. }), "{t:#?}");
    assert!(t.close.waiting_cleared);
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Closed { .. }), "{t:#?}");
    let lost_sends = t.decisions.iter().filter(|d| d.name == "lost-send").count();
    assert_eq!(lost_sends, 1, "{:#?}", t.decisions);
    assert!(t.pending_decisions().is_empty());
    assert!(!env.sb().session(&investigator).waiting);
}

#[test]
fn an_answered_lost_send_is_not_raised_again_by_later_passes_or_a_restart() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.step();
    let mut t = env.ticket(&id);
    let mut lost = t
        .ledger
        .iter()
        .rfind(|o| o.kind == "session.new")
        .unwrap()
        .clone();
    lost.op = format!("{id}-lostsend");
    lost.kind = "session.input".into();
    lost.class = "non-replayable".into();
    lost.body = None;
    lost.reply = None;
    lost.error = None;
    t.ledger.push(lost);
    env.runner.save_ticket(&mut t, env.now).unwrap();
    env.step();
    let d = env.pending(&id)[0].clone();
    assert_eq!(d.name, "lost-send");
    let now = env.tick();
    env.runner.decide(&id, &d.id, "rerun", None, now).unwrap();
    env.step();
    env.step();
    env.restart();
    env.step();
    let t = env.ticket(&id);
    let lost_sends = t.decisions.iter().filter(|d| d.name == "lost-send").count();
    assert_eq!(lost_sends, 1, "{:#?}", t.decisions);
}

#[test]
fn a_close_that_recovery_parks_and_then_fails_stays_closing() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.step();
    let investigator = session_of(&env.ticket(&id), "investigate");
    assert!(env.ticket(&id).processes.contains(&investigator));
    // Recovery cannot find the launch, so it fails the attempt.
    env.sb().log.clear();
    write_closing(&env, &id, lost_launch_with_reruns_spent);
    // Parking kills the investigator, and that reply is lost.
    env.sb().drop_reply_for = Some("session.kill".into());
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Closing { .. }), "{t:#?}");
    let ps = env.runner.load_project(PROJECT).unwrap();
    assert_eq!(ps.closing, vec![id.clone()]);
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Closed { .. }), "{t:#?}");
    let ps = env.runner.load_project(PROJECT).unwrap();
    assert!(ps.queue.is_empty() && ps.closing.is_empty());
}

#[test]
fn recovery_never_saves_a_closing_ticket_parking() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.step();
    env.sb().log.clear();
    write_closing(&env, &id, lost_launch_with_reruns_spent);
    // A runner killed while the processes are killed leaves this file.
    env.sb().snapshot_on = Some(("session.kill".into(), env.data.ticket_file(&id)));
    env.step();
    let snapshots = env.sb().snapshots.clone();
    assert!(!snapshots.is_empty());
    for text in snapshots {
        let t: Ticket = serde_json::from_str(&text).unwrap();
        assert!(matches!(t.state, TicketState::Closing { .. }), "{t:#?}");
    }
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Closed { .. }), "{t:#?}");
    assert!(
        !t.decisions.iter().any(|d| d.name == "rerun"),
        "{:#?}",
        t.decisions
    );
}

#[test]
fn a_card_left_up_with_nothing_to_show_is_cleared() {
    let mut env = Env::new();
    let id = env.take(7).id;
    env.step();
    assert_eq!(env.sb().sets[0].items.len(), 1);
    // A close stopped after its ticket was saved closed, before the set
    // and the project were.
    let mut t = env.ticket(&id);
    t.state = TicketState::Closed {
        reason: "closed by hand".into(),
    };
    env.runner.save_ticket(&mut t, env.now).unwrap();
    env.step();
    assert!(env.sb().sets[0].items.is_empty());
    let ps = env.runner.load_project(PROJECT).unwrap();
    assert!(ps.queue.is_empty() && ps.shown.is_empty());
    let t = env.ticket(&id);
    let sync = t.ledger.iter().rfind(|o| o.kind == "set.sync").unwrap();
    assert!(sync.reply.is_some(), "the stale ticket's ledger, saved");
    let syncs = env.sb().kinds_called("set.sync");
    env.step();
    assert_eq!(env.sb().kinds_called("set.sync"), syncs, "once");
}

#[test]
fn a_closing_tickets_decisions_count_against_nothing_and_read_as_cancelling() {
    let mut env = Env::new();
    let id = env.take(7).id;
    at_rerun(&mut env, &id);
    let pending = |env: &Env| dispatch::serve::status(&env.runner).unwrap().projects[0].pending;
    assert_eq!(pending(&env), 1);
    write_closing(&env, &id, |_| {});
    assert_eq!(pending(&env), 0);
    let status = dispatch::serve::status(&env.runner).unwrap();
    let decision_states: Vec<&str> = status.tickets[0]
        .decisions
        .iter()
        .map(|d| d.state.as_str())
        .collect();
    assert!(
        decision_states.contains(&"cancelling") && !decision_states.contains(&"pending"),
        "{decision_states:?}"
    );
}

#[test]
fn a_ticket_never_on_the_set_closes_without_a_sync() {
    let mut env = Env::new();
    env.take(7);
    env.step();
    let id = env.take(8).id;
    let syncs = env.sb().kinds_called("set.sync");
    let now = env.tick();
    let t = env.runner.close_by_hand(&id, None, now).unwrap();
    assert!(t.close.card_cleared && matches!(t.state, TicketState::Closed { .. }));
    assert_eq!(env.sb().kinds_called("set.sync"), syncs, "unchanged");
}

#[test]
fn a_close_beside_a_parked_ticket_still_clears_its_card() {
    let mut env = Env::new();
    let a = env.take(7).id;
    let b = env.take(8).id;
    env.step();
    assert_eq!(env.sb().sets[0].items.len(), 2);
    for id in [&a, &b] {
        let session = session_of(&env.ticket(id), "investigate");
        env.sb().vanish(&session);
    }
    env.step();
    let d = env.pending(&a)[0].id.clone();
    let now = env.tick();
    env.runner.decide(&a, &d, "park", None, now).unwrap();
    env.step();
    assert!(matches!(env.ticket(&a).state, TicketState::Parked { .. }));
    let now = env.tick();
    env.runner.close_by_hand(&b, None, now).unwrap();
    let a_session = session_of(&env.ticket(&a), "investigate");
    let sb = env.sb();
    let items = &sb.sets[0].items;
    assert_eq!(items.len(), 1, "{items:?}");
    assert!(
        matches!(&items[0].target, switchboard_control::PinTarget::Session { session } if session == &a_session)
    );
}

#[test]
fn a_dirty_tree_at_the_pipelines_end_is_kept_and_removed_by_hand_later() {
    let mut env = Env::new();
    let (id, implementer) = at_implement(&mut env);
    implementer_stops(&mut env, &id, &implementer);
    env.steps_until(&id, "the checks starting", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.gate.is_some())
    });
    let key = format!("{id}/implement/1");
    env.repo.lock().unwrap().check_exits.insert(key, 0);
    env.pr_is(&id, "base0000", "open", Checks::Passed);
    env.inspect(&id, "proceed", None);
    env.steps_until(&id, "the merge decision", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "merge")
    });
    let tree = env.ticket(&id).tree.clone().unwrap();
    env.repo.lock().unwrap().dirty.push(tree.clone());
    env.pr_is(&id, "base0000", "merged", Checks::Passed);
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the ticket closing", |t, _| !t.active());
    let t = env.ticket(&id);
    assert!(matches!(&t.state, TicketState::Closed { .. }), "{t:#?}");
    let kept = t.close.trees_kept.clone().unwrap();
    assert!(kept.contains(&tree.display().to_string()), "{kept}");
    assert!(!t.close.tree_removed && tree.exists());
    assert!(env.repo.lock().unwrap().removed.is_empty());
    env.repo.lock().unwrap().dirty.clear();
    let now = env.tick();
    let t = env.runner.close_by_hand(&id, None, now).unwrap();
    assert!(t.close.tree_removed && t.close.trees_kept.is_none());
    assert!(matches!(&t.state, TicketState::Closed { reason } if reason == "every stage is done"));
    assert_eq!(
        env.repo.lock().unwrap().removed,
        [(env.data.repo_dir(PROJECT), tree)]
    );
}
