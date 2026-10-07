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

use dispatch::git::{FakeRepo, Gate, Network};
use dispatch::github::{Checks, FakePullRequests, PullRequest};
use dispatch::history::{Commit, Commits, Group};
use dispatch::scheduler::{
    NUDGE_TEXT, PR_ERROR_GRACE_MS, PR_POLL_MS, PR_YOUNG_HEAD_MS, REFRESH, RESOLUTION, Runner,
    STOP_LIMIT_MS,
};
use dispatch::store::DataDir;
use dispatch::ticket::{
    Attempt, AttemptKind, AttemptState, DIRTY_WAIT_MS, Decision, DecisionState, LaneRecord,
    PushedHead, ReviewerResult, Rewrite, RoundState, SETTLE_POLLS, STOP_IDLE_POLLS, ServiceState,
    SourceSnapshot, Ticket, TicketState,
};
use support::{FakeSwitchboard, SharedPort, events_of};
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
        dispatch::store::skip_fsync_for_tests();
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
        let now = self.tick();
        take_issue(&mut self.runner, &self.data, number, now).unwrap()
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

/// Issue `number` taken into the Switchboard project by `runner`, which
/// may be another process's.
fn take_issue(
    runner: &mut Runner,
    data: &DataDir,
    number: u64,
    now: u64,
) -> anyhow::Result<Ticket> {
    let text = std::fs::read_to_string(data.pipeline(PROJECT)).unwrap();
    runner.take(
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
            taken_by: None,
            pull_requests: Vec::new(),
        },
        now,
    )
}

/// `want` appears in `kinds` in this order, not necessarily together.
fn assert_in_order(kinds: &[String], want: &[&str]) {
    let mut rest = kinds.iter();
    for w in want {
        assert!(rest.any(|k| k == w), "{want:?} not in order in {kinds:?}");
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
    // The log tells the same story, the decision and its answer from
    // the terminal included.
    let kinds = events_of(&env.data, &id);
    assert_eq!(
        kinds[..4],
        ["taken", "attempt-started", "attempt-ended", "stage"]
    );
    assert_in_order(
        &kinds,
        &[
            "stage",
            "attempt-started",
            "decision",
            "answered",
            "attempt-ended",
        ],
    );
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

/// The project's pipeline file with `confine` on, the policy's network
/// set to `network`, and `/opt/cargo-cache` writable for the lane.
fn confined(env: &Env, network: &str) {
    let path = env.data.pipeline(PROJECT);
    let text = std::fs::read_to_string(&path).unwrap();
    let text = text
        .replace(
            "setup = [\"cargo\", \"fetch\", \"--locked\"]\n",
            "setup = [\"cargo\", \"fetch\", \"--locked\"]\nwritable = [\"/opt/cargo-cache\"]\n",
        )
        .replace(
            "[policy]\n",
            &format!("[policy]\nconfine = true\nnetwork = \"{network}\"\n"),
        );
    std::fs::write(path, text).unwrap();
}

/// The implement stage's command gate given its own `network`.
fn implement_gate_network(env: &Env, network: &str) {
    let path = env.data.pipeline(PROJECT);
    let gate =
        "gate = { kind = \"command\", argv = [\"sh\", \"-c\", \"cargo test\"], in = \"lane\" }";
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(gate));
    let text = text.replace(
        gate,
        &gate.replace(" }", &format!(", network = \"{network}\" }}")),
    );
    std::fs::write(path, text).unwrap();
}

/// The implement attempt's checks, started once the implementer stops.
fn implement_check(env: &mut Env, id: &str) -> dispatch::git::StartedCheck {
    env.steps_until(id, "the checks starting", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.gate.is_some())
    });
    let key = format!("{id}/implement/1");
    let repo = env.repo.lock().unwrap();
    repo.checks.iter().find(|c| c.key == key).unwrap().clone()
}

#[test]
fn a_confined_pipeline_passes_the_tree_attempt_dir_and_lane_writable_to_the_gate() {
    let mut env = Env::new();
    confined(&env, "allow");
    let (id, implementer) = at_implement(&mut env);
    implementer_stops(&mut env, &id, &implementer);
    let check = implement_check(&mut env, &id);
    let t = env.ticket(&id);
    let a = t.attempts_of("implement").last().unwrap();
    let attempt_dir = a.artifacts["checks"].parent().unwrap().to_path_buf();
    let c = check.confine.expect("started confined");
    assert_eq!(c.network, Network::Allow);
    for path in [
        t.lanes[0].worktree.clone(),
        t.tree.clone().unwrap(),
        attempt_dir,
        PathBuf::from("/opt/cargo-cache"),
    ] {
        assert!(c.writable.contains(&path), "{path:?} in {c:?}");
    }
    assert_eq!(c.writable.len(), {
        let mut distinct = c.writable.clone();
        distinct.dedup();
        distinct.len()
    });
}

/// With the policy denying the network, a gate that says `allow` gets it,
/// a gate given by `like` that gate gets it too, and a command reviewer
/// keeps the policy's; a gate without the key gets the policy's.
#[test]
fn a_gates_network_overrides_the_policy() {
    let mut env = Env::new();
    confined(&env, "deny");
    let id = at_implement(&mut env).0;
    let implementer = session_of(&env.ticket(&id), "implement");
    implementer_stops(&mut env, &id, &implementer);
    let check = implement_check(&mut env, &id);
    assert_eq!(check.confine.unwrap().network, Network::Deny);

    let mut env = Env::new();
    env.with_review_stage("auto");
    confined(&env, "deny");
    implement_gate_network(&env, "allow");
    let id = at_review(&mut env);
    let check = env
        .repo
        .lock()
        .unwrap()
        .checks
        .iter()
        .find(|c| c.key == format!("{id}/implement/1"))
        .cloned()
        .unwrap();
    assert_eq!(check.confine.unwrap().network, Network::Allow);
    let t = env.ticket(&id);
    {
        let repo = env.repo.lock().unwrap();
        let lint = repo
            .checks
            .iter()
            .find(|c| c.key == lint_key(&t, 1))
            .unwrap();
        assert_eq!(lint.confine.as_ref().unwrap().network, Network::Deny);
    }
    lint_exits(&mut env, &id, 1, 1, "src/a.rs:3: unused import\n");
    style_says(&mut env, &id, 1, "No findings.");
    fix_pass(
        &mut env,
        &id,
        1,
        "fix00001",
        "- r1/lint-1: fixed removed\n",
        0,
    );
    let t = env.ticket(&id);
    let repo = env.repo.lock().unwrap();
    let round = repo
        .checks
        .iter()
        .find(|c| c.key == checks_key(&t, 1))
        .unwrap();
    assert_eq!(round.confine.as_ref().unwrap().network, Network::Allow);
}

#[test]
fn setup_under_confine_runs_through_run_confined_and_the_merge_does_not() {
    let mut env = Env::new();
    confined(&env, "allow");
    let (id, _) = at_implement(&mut env);
    let t = env.ticket(&id);
    {
        let repo = env.repo.lock().unwrap();
        assert!(repo.ran.is_empty(), "{:?}", repo.ran);
        let (dir, argv, c) = &repo.ran_confined[0];
        assert_eq!(dir, &t.lanes[0].worktree);
        assert_eq!(argv, &["cargo", "fetch", "--locked"]);
        assert!(c.writable.contains(&PathBuf::from("/opt/cargo-cache")));
        assert!(c.writable.contains(&t.lanes[0].worktree));
    }

    // The scheduler's own fast-forward of a pull request's branch is
    // git work on the clone, never confined.
    let mut env = Env::new();
    let worktrees = env.data.root.join("wt");
    let text = pr_pipeline(&worktrees).replace("[policy]\n", "[policy]\nconfine = true\n");
    std::fs::write(env.data.pr_pipeline(PROJECT), text).unwrap();
    open_pr(&env, "msull/switchboard", 9, "feature/escape", "Escape");
    open_pr(&env, "msull/docs", 3, "feature/escape-docs", "Docs");
    seed_pr_bases(&env);
    let now = env.tick();
    let t =
        dispatch::serve::take_pull_requests(&mut env.runner, PROJECT, &["repo/9", "docs/3"], now)
            .unwrap();
    env.steps_until(&t.id, "the sign-off question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "inspect")
    });
    let repo = env.repo.lock().unwrap();
    let merges = repo
        .ran
        .iter()
        .filter(|(_, argv)| argv.join(" ").starts_with("git merge --ff-only"))
        .count();
    assert_eq!(merges, 2);
    assert!(repo.ran_confined.is_empty(), "{:?}", repo.ran_confined);
}

#[test]
fn a_failed_confined_setup_parks_with_the_header_first() {
    let mut env = Env::new();
    confined(&env, "allow");
    env.repo.lock().unwrap().fail_run =
        Some("cargo exited 101: dispatch: sandbox: cargo(9) deny(1) file-write-create /x".into());
    let id = env.take(1).id;
    env.steps_until(&id, "the ticket parking", |t, _| !t.active());
    let t = env.ticket(&id);
    let (TicketState::Parked { reason } | TicketState::Parking { reason }) = &t.state else {
        panic!("{t:#?}");
    };
    assert!(
        reason.starts_with("lane repo: setup failed: dispatch: confined; writable "),
        "{reason}"
    );
    assert!(reason.contains("deny(1) file-write-create /x"), "{reason}");
    assert!(!t.lanes[0].setup_done);
}

#[test]
fn a_review_rounds_checks_and_command_reviewers_are_confined_to_the_round() {
    let mut env = Env::new();
    env.with_review_stage("auto");
    confined(&env, "allow");
    let id = at_review(&mut env);
    let t = env.ticket(&id);
    let lint = reviewer(&t, 1, "lint");
    {
        let repo = env.repo.lock().unwrap();
        let (started, _) = repo
            .reviewers
            .iter()
            .find(|(c, _)| c.key == lint_key(&t, 1))
            .unwrap();
        let c = started.confine.as_ref().unwrap();
        assert!(c.writable.contains(&lint.dir), "{c:?}");
        assert!(c.writable.contains(&t.lanes[0].worktree), "{c:?}");
    }
    lint_exits(&mut env, &id, 1, 1, "src/a.rs:3: unused import\n");
    style_says(&mut env, &id, 1, "No findings.");
    fix_pass(
        &mut env,
        &id,
        1,
        "fix00001",
        "- r1/lint-1: fixed removed\n",
        0,
    );
    let t = env.ticket(&id);
    let round_dir = lint.dir.parent().unwrap().to_path_buf();
    let repo = env.repo.lock().unwrap();
    let checks = repo
        .checks
        .iter()
        .find(|c| c.key == checks_key(&t, 1))
        .unwrap();
    assert_eq!(checks.log, round_dir.join("checks.log"));
    let c = checks.confine.as_ref().unwrap();
    assert!(c.writable.contains(&round_dir), "{c:?}");
    assert!(c.writable.contains(&PathBuf::from("/opt/cargo-cache")));
}

#[test]
fn confine_off_calls_the_unconfined_methods() {
    let mut env = Env::new();
    let (id, implementer) = at_implement(&mut env);
    implementer_stops(&mut env, &id, &implementer);
    let check = implement_check(&mut env, &id);
    assert_eq!(check.confine, None);
    let repo = env.repo.lock().unwrap();
    assert!(repo.ran_confined.is_empty());
    assert_eq!(repo.ran[0].1, ["cargo", "fetch", "--locked"]);
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
    let previous = &t.attempts_of("implement").next().unwrap().artifacts["notes"];
    assert!(
        prompt.ends_with(&format!(
            "sent it back: use a set, not a vec (the previous attempt's notes are at {})",
            previous.display()
        )),
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

/// A PR the provider reports conflicting at `merge` goes back through
/// `ready`, where the policy's rebaser starts: a session cloned from the
/// lane's implementer, told the PR, the base and where the notes go.
/// When it stops, `ready` reads the checks at the head it pushed, and
/// the ticket stands at `merge` again.
#[test]
fn a_conflicting_pr_is_rebased_by_a_clone_of_the_implementer() {
    let mut env = Env::new();
    let id = at_ready(&mut env);
    env.at_merge(&id);
    let implementer = session_of(&env.ticket(&id), "implement");
    env.pr_is_with(&id, "base0000", "open", Checks::Passed, Some("conflicting"));
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the rebaser", |t, _| {
        t.attempts_of("ready")
            .any(|a| a.kind == AttemptKind::Agent && a.session.is_some())
    });
    let t = env.ticket(&id);
    let left = t.attempts_of("merge").last().unwrap();
    assert!(
        matches!(&left.state, AttemptState::Cancelled { reason }
            if reason == "sent back from merge: the provider reports it conflicting"),
        "{left:?}"
    );
    assert!(
        t.pending_decisions().iter().all(|d| d.name != "merge"),
        "the merge decision went with the trip back"
    );
    let rebase = t
        .attempts_of("ready")
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
    // The rebaser pushed a new head and stopped.
    env.repo
        .lock()
        .unwrap()
        .heads
        .insert(t.lanes[0].worktree.clone(), "rebased1".into());
    env.pr_is_with(&id, "rebased1", "open", Checks::Passed, Some("clean"));
    env.finish(&rebaser, &notes, "# rebased\nkept both changelog entries");
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the merge decision again", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "merge")
    });
    assert!(env.sb().killed.contains(&rebaser), "the rebaser is done");
    let t = env.ticket(&id);
    let ready = t
        .attempts_of("ready")
        .filter(|a| a.kind == AttemptKind::GateOnly)
        .last()
        .unwrap();
    assert_eq!(ready.state, AttemptState::Complete);
    assert_eq!(
        ready.head.as_deref(),
        Some("rebased1"),
        "checks read after it"
    );
    let gate = t.attempts_of("merge").last().unwrap();
    assert!(gate.is_open());
    assert_eq!(gate.pr.as_ref().map(|p| p.head.as_str()), Some("rebased1"));
    env.pr_is_with(&id, "rebased1", "merged", Checks::Passed, None);
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the ticket closing", |t, _| !t.active());
    let t = env.ticket(&id);
    assert!(matches!(&t.state, TicketState::Closed { .. }));
    assert_eq!(
        t.attempts_of("merge")
            .last()
            .and_then(|a| a.pr.as_ref().map(|p| p.checks.as_str())),
        Some("merged")
    );
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
            .attempts_of("ready")
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
        t.attempts_of("ready")
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
        t.attempts_of("ready")
            .any(|a| a.kind == AttemptKind::Agent && a.session.is_some())
    });
    let t = env.ticket(&id);
    let rebaser = session_of(&t, "ready");
    let notes = artifact_of(&t, "ready", "notes");
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
            .attempts_of("ready")
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

/// `merge` is a confirmation the provider resolves: the decision is
/// answered `recheck` or `park` only, `merged` by hand is refused, and the PR reading as
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
    assert_eq!(d.options, vec!["recheck", "park"]);
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
    assert!(err.contains("takes one of: recheck, park"), "{err}");
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
    assert_in_order(
        &events_of(&env.data, &id),
        &["pr", "decision", "answered", "closing", "closed"],
    );
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
    with_policy(&env, &t, "on_dirty = \"ask\"");
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

/// Under `on_dirty = "ask"` a dirty stop is the question at once, with
/// nothing typed into the session.
#[test]
fn a_dirty_tree_after_the_agent_never_runs_the_checks() {
    let mut env = Env::new();
    let (id, implementer) = at_implement(&mut env);
    with_policy(&env, &env.ticket(&id), "on_dirty = \"ask\"");
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
    assert!(env.sb().sent.is_empty(), "no nudge under ask");
}

// --- a dirty stop is first a nudge into the same session

/// The implementer, stopped dirty, nudged once: one line typed, no
/// question, the session still open.
fn nudged_once(env: &mut Env) -> (String, String) {
    let (id, implementer) = at_implement(env);
    let worktree = env.ticket(&id).lanes[0].worktree.clone();
    env.repo.lock().unwrap().dirty.push(worktree);
    implementer_stops(env, &id, &implementer);
    env.steps_until(&id, "the nudge", |_, sb| !sb.sent.is_empty());
    (id, implementer)
}

/// The implementer stops again, after its nudge.
fn stops_again(env: &mut Env, session: &str) {
    let now = env.tick();
    env.sb().stop(session, now);
}

fn the_rerun_question(env: &mut Env, id: &str) -> Decision {
    env.steps_until(id, "the rerun question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    env.pending(id)
        .into_iter()
        .find(|d| d.name == "rerun")
        .unwrap()
}

#[test]
fn a_dirty_implementer_is_nudged_and_its_clean_stop_runs_the_checks() {
    let mut env = Env::new();
    let (id, implementer) = nudged_once(&mut env);
    assert_eq!(
        env.sb().sent,
        vec![(implementer.clone(), NUDGE_TEXT.to_owned())]
    );
    env.step();
    let t = env.ticket(&id);
    assert!(t.pending_decisions().is_empty(), "{t:#?}");
    assert!(!env.sb().killed.contains(&implementer), "the session lives");
    let a = t.attempts_of("implement").last().unwrap();
    assert_eq!(a.nudges.len(), 1);
    assert!(a.is_open() && a.gate.is_none());
    env.repo.lock().unwrap().dirty.clear();
    stops_again(&mut env, &implementer);
    env.steps_until(&id, "the checks starting", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.gate.is_some())
    });
    let t = env.ticket(&id);
    let a = t.attempts_of("implement").last().unwrap();
    assert_eq!(a.gate.as_ref().unwrap().head, "base0000");
    assert!(env.sb().killed.contains(&implementer));
    assert_eq!(env.sb().sent.len(), 1);
}

#[test]
fn a_dirty_implementer_after_its_nudge_is_asked_after_1_nudge() {
    let mut env = Env::new();
    let (id, implementer) = nudged_once(&mut env);
    stops_again(&mut env, &implementer);
    let d = the_rerun_question(&mut env, &id);
    assert_eq!(d.options, vec!["rerun", "check", "park"]);
    assert!(d.question.contains("after 1 nudge"), "{}", d.question);
    assert_eq!(env.sb().sent.len(), 1);
    assert!(env.sb().killed.contains(&implementer));
    assert!(env.repo.lock().unwrap().checks.is_empty());
}

#[test]
fn a_restart_after_a_nudge_sends_no_second_one() {
    let mut env = Env::new();
    let (id, implementer) = nudged_once(&mut env);
    env.restart();
    for _ in 0..3 {
        env.step();
    }
    assert_eq!(env.sb().sent.len(), 1);
    let t = env.ticket(&id);
    assert_eq!(t.attempts_of("implement").last().unwrap().nudges.len(), 1);
    assert!(t.pending_decisions().is_empty(), "{t:#?}");
    env.repo.lock().unwrap().dirty.clear();
    stops_again(&mut env, &implementer);
    env.steps_until(&id, "the checks starting", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.gate.is_some())
    });
}

#[test]
fn a_lost_nudge_reply_asks_nothing_and_the_idle_grace_ends_it() {
    let mut env = Env::new();
    let (id, implementer) = nudged_once(&mut env);
    let mut t = env.ticket(&id);
    let op = t
        .ledger
        .iter_mut()
        .rev()
        .find(|o| o.intent == "nudge")
        .unwrap();
    op.reply = None;
    let op_id = op.op.clone();
    dispatch::store::write_ticket(&env.data.ticket_file(&id), &t).unwrap();
    env.restart();
    let t = env.ticket(&id);
    assert!(
        !t.pending_decisions().iter().any(|d| d.name == "lost-send"),
        "{t:#?}"
    );
    let op = t.ledger.iter().find(|o| o.op == op_id).unwrap();
    assert_eq!(op.error.as_deref(), Some("reply lost; not repeated"));
    assert!(op.settled);
    // The line never reached the agent: it sits at its prompt.
    env.sb().session_mut(&implementer).card = "idle".into();
    env.idle_past_grace();
    let d = the_rerun_question(&mut env, &id);
    assert!(
        d.question
            .contains("after 1 nudge; no stop came after the last nudge"),
        "{}",
        d.question
    );
    assert_eq!(env.sb().sent.len(), 1);
}

#[test]
fn an_unanswered_nudge_is_asked_about_even_with_nudges_left() {
    let mut env = Env::new();
    let (id, implementer) = at_implement(&mut env);
    with_policy(&env, &env.ticket(&id), "on_dirty = { nudge = 2 }");
    let worktree = env.ticket(&id).lanes[0].worktree.clone();
    env.repo.lock().unwrap().dirty.push(worktree);
    implementer_stops(&mut env, &id, &implementer);
    env.steps_until(&id, "the nudge", |_, sb| !sb.sent.is_empty());
    env.sb().session_mut(&implementer).card = "idle".into();
    env.idle_past_grace();
    let d = the_rerun_question(&mut env, &id);
    assert!(
        d.question
            .contains("after 1 nudge; no stop came after the last nudge"),
        "{}",
        d.question
    );
    assert_eq!(env.sb().sent.len(), 1);
}

#[test]
fn two_nudges_answered_by_dirty_stops_then_the_question() {
    let mut env = Env::new();
    let (id, implementer) = at_implement(&mut env);
    with_policy(&env, &env.ticket(&id), "on_dirty = { nudge = 2 }");
    let worktree = env.ticket(&id).lanes[0].worktree.clone();
    env.repo.lock().unwrap().dirty.push(worktree);
    implementer_stops(&mut env, &id, &implementer);
    env.steps_until(&id, "the first nudge", |_, sb| sb.sent.len() == 1);
    stops_again(&mut env, &implementer);
    env.steps_until(&id, "the second nudge", |_, sb| sb.sent.len() == 2);
    assert!(env.pending(&id).is_empty());
    stops_again(&mut env, &implementer);
    let d = the_rerun_question(&mut env, &id);
    assert!(d.question.contains("after 2 nudges"), "{}", d.question);
    assert_eq!(env.sb().sent.len(), 2);
    let t = env.ticket(&id);
    assert_eq!(t.attempts_of("implement").last().unwrap().nudges.len(), 2);
}

#[test]
fn a_nudged_session_that_ends_is_asked_about_at_its_checks() {
    let mut env = Env::new();
    let (id, implementer) = nudged_once(&mut env);
    env.sb().vanish(&implementer);
    let d = the_rerun_question(&mut env, &id);
    assert_eq!(d.options, vec!["rerun", "check", "park"]);
    assert!(
        d.question
            .contains("after 1 nudge; the session ended with no stop after the last nudge"),
        "{}",
        d.question
    );
    assert!(env.repo.lock().unwrap().checks.is_empty());
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

/// The app reports the latest review run failed: its agent stopped
/// without writing its round file.
fn review_run_fails(env: &mut Env) {
    env.sb().runs.last_mut().unwrap().state = RunState::Paused {
        reason: "plan review stopped without writing plan.feedback-1.md".into(),
        failed: true,
    };
}

#[test]
fn a_failed_review_run_fails_the_attempt_and_a_rerun_starts_a_new_run() {
    let mut env = Env::new();
    let id = at_review_run(&mut env);
    review_run_fails(&mut env);
    env.steps_until(&id, "the review failing", |t, _| {
        t.attempts_of("review")
            .last()
            .is_some_and(|a| matches!(a.state, AttemptState::Failed { .. }))
    });
    let t = env.ticket(&id);
    let AttemptState::Failed { reason } = &t.attempts_of("review").last().unwrap().state else {
        panic!()
    };
    assert!(
        reason.starts_with("review: ") && reason.contains("stopped without"),
        "{reason}"
    );
    let pending = env.pending(&id);
    assert_eq!((pending.len(), pending[0].name.as_str()), (1, "rerun"));
    let now = env.tick();
    env.runner
        .decide(&id, &pending[0].id, "rerun", None, now)
        .unwrap();
    env.steps_until(&id, "a second review run", |t, _| {
        t.attempts_of("review")
            .last()
            .is_some_and(|a| a.n == 2 && a.run.is_some())
    });
    assert_eq!(env.sb().runs.len(), 2, "a fresh run");
}

#[test]
fn a_review_failing_past_max_reruns_parks_the_ticket() {
    let mut env = Env::new();
    let id = at_review_run(&mut env);
    let t = env.ticket(&id);
    with_max_reruns(&env, &t, 1);
    review_run_fails(&mut env);
    env.steps_until(&id, "the first failure", |t, _| {
        t.attempts_of("review")
            .last()
            .is_some_and(|a| matches!(a.state, AttemptState::Failed { .. }))
    });
    let pending = env.pending(&id);
    let now = env.tick();
    env.runner
        .decide(&id, &pending[0].id, "rerun", None, now)
        .unwrap();
    env.steps_until(&id, "a second review run", |t, _| {
        t.attempts_of("review")
            .last()
            .is_some_and(|a| a.n == 2 && a.run.is_some())
    });
    review_run_fails(&mut env);
    env.steps_until(&id, "the ticket parking", |t, _| !t.active());
    let t = env.ticket(&id);
    let (TicketState::Parked { reason } | TicketState::Parking { reason }) = &t.state else {
        panic!("{t:#?}");
    };
    assert!(reason.contains("max_reruns"), "{reason}");
}

#[test]
fn a_review_paused_by_hand_still_asks_to_continue() {
    let mut env = Env::new();
    let id = at_review_run(&mut env);
    env.sb().runs[0].state = RunState::Paused {
        reason: "paused by you".into(),
        failed: false,
    };
    env.step();
    let pending = env.pending(&id);
    assert_eq!(pending.len(), 1, "{pending:#?}");
    assert_eq!(pending[0].name, "paused");
    assert_eq!(pending[0].options, vec!["continue", "park"]);
    assert!(
        env.ticket(&id)
            .attempts_of("review")
            .last()
            .unwrap()
            .is_open()
    );
}

#[test]
fn a_review_continued_in_the_app_then_failing_asks_only_to_rerun() {
    let mut env = Env::new();
    let id = at_review_run(&mut env);
    env.sb().runs[0].state = RunState::Paused {
        reason: "paused by you".into(),
        failed: false,
    };
    env.step();
    assert_eq!(env.pending(&id)[0].name, "paused");
    // Continued in the app, not answered here.
    env.sb().runs[0].state = RunState::AwaitingFeedback;
    env.step();
    review_run_fails(&mut env);
    env.steps_until(&id, "the review failing", |t, _| {
        t.attempts_of("review")
            .last()
            .is_some_and(|a| matches!(a.state, AttemptState::Failed { .. }))
    });
    let pending = env.pending(&id);
    assert_eq!(pending.len(), 1, "{pending:#?}");
    assert_eq!(pending[0].name, "rerun");
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

/// `revise` at `finalize`, answered with `note`: the decision's id.
fn revise(env: &mut Env, id: &str, note: &str) -> String {
    let d = env.pending(id)[0].clone();
    assert_eq!(d.name, "finalize");
    assert_eq!(d.options, ["finalize", "revise", "park"]);
    let now = env.tick();
    env.runner
        .decide(id, &d.id, "revise", Some(note), now)
        .unwrap();
    d.id
}

/// The texts of the ticket's events of `kind`.
fn texts_of_kind(env: &Env, id: &str, kind: dispatch::events::Kind) -> Vec<String> {
    dispatch::events::read_since(&dispatch::events::log_path(&env.data), 0)
        .unwrap()
        .into_iter()
        .filter(|e| e.ticket == id && e.kind == kind)
        .map(|e| e.text)
        .collect()
}

/// The owner's `revise` writes the next round's feedback beside the
/// reviewed copy and sends it as `workflow.object`; `finalize` is asked
/// again, naming the new round count, once the run converges again.
#[test]
fn revise_writes_the_owners_round_and_sends_it_as_an_objection() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let session = env.ticket(&id).current_session().cloned().unwrap();
    revise(&mut env, &id, "Step 3 deletes data; keep a backup.");
    env.step();
    let copy = artifact_of(&env.ticket(&id), "review", "plan");
    let file = dispatch::report::round_file(&copy, 2);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.starts_with(&format!("{}\n\n", dispatch::OWNER_ROUND_HEADING)),
        "{text}"
    );
    assert!(
        text.contains("Step 3 deletes data; keep a backup."),
        "{text}"
    );
    let run = env.sb().runs[0].id.clone();
    assert_eq!(
        env.sb().objections,
        [(run, 2, "Step 3 deletes data; keep a backup.".to_owned())]
    );
    let t = env.ticket(&id);
    let review = t.attempts_of("review").last().unwrap();
    assert_eq!(review.revisions.len(), 1);
    assert_eq!(
        (review.revisions[0].round, review.revisions[0].by.as_str()),
        (2, "you")
    );
    assert!(events_of(&env.data, &id).contains(&"revised".to_owned()));
    assert_eq!(
        texts_of_kind(&env, &id, dispatch::events::Kind::Revised),
        ["r2: the owner's objection, by you"]
    );
    assert!(
        texts_of_kind(&env, &id, dispatch::events::Kind::Answered)
            .iter()
            .any(|e| e.contains("revise — Step 3 deletes data")),
        "the answer carries the note"
    );
    assert!(env.pending(&id).is_empty());
    assert!(!env.sb().session(&session).waiting, "unmarked");
    {
        let mut sb = env.sb();
        sb.runs[0].state = RunState::Converged;
        sb.runs[0].round = 3;
    }
    env.step();
    let again = env.pending(&id);
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].name, "finalize");
    assert!(
        again[0].question.contains("converged after 3 round(s)"),
        "{}",
        again[0].question
    );
}

/// `revise` takes a note: none, or only spaces, is a usage error, and
/// the decision still waits.
#[test]
fn revise_without_a_note_is_refused_and_the_decision_waits() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let d = env.pending(&id)[0].id.clone();
    for note in [None, Some("  ")] {
        let now = env.tick();
        let e = env.runner.decide(&id, &d, "revise", note, now).unwrap_err();
        assert_eq!(
            e.downcast_ref::<dispatch::UsageError>()
                .map(ToString::to_string)
                .as_deref(),
            Some("revise takes --note <text> or --file <path>")
        );
        assert_eq!(env.pending(&id)[0].id, d);
    }
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_dispatch"))
        .args(["decide", &id, &d, "revise"])
        .env("DISPATCH_DATA_DIR", &env.data.root)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64), "{out:?}");
    assert_eq!(env.pending(&id)[0].id, d);
    assert!(env.sb().objections.is_empty());
}

/// `--file` reads the note from a file, but never from one of the
/// ticket's secret artifacts.
#[test]
fn revise_reads_its_note_from_a_file_but_never_a_secret_one() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let d = env.pending(&id)[0].id.clone();
    let dir = tempfile::tempdir().unwrap();
    // A secret artifact of an earlier attempt, as the record names it.
    let secret = dir.path().join("personas.md");
    std::fs::write(&secret, "hunter2").unwrap();
    let mut t = env.ticket(&id);
    let a = t.attempts.iter_mut().find(|a| a.stage == "plan").unwrap();
    a.secret.insert("personas".into());
    a.artifacts.insert("personas".into(), secret.clone());
    let now = env.tick();
    env.runner.save_ticket(&mut t, now).unwrap();
    let decide = |path: &std::path::Path| {
        std::process::Command::new(env!("CARGO_BIN_EXE_dispatch"))
            .args(["decide", &id, &d, "revise", "--file"])
            .arg(path)
            .env("DISPATCH_DATA_DIR", &env.data.root)
            .output()
            .unwrap()
    };
    let out = decide(&secret);
    assert_eq!(out.status.code(), Some(64), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stderr).trim(),
        "personas is secret; Dispatch never reads it"
    );
    assert_eq!(env.pending(&id)[0].id, d);
    let note = dir.path().join("objection.md");
    std::fs::write(&note, "Split step 3.\n\nIt does two things.\n").unwrap();
    let out = decide(&note);
    assert!(out.status.success(), "{out:?}");
    assert!(env.pending(&id).is_empty());
    env.step();
    assert_eq!(
        env.sb().objections[0].2,
        "Split step 3.\n\nIt does two things.\n"
    );
}

/// A supervisor whose `decides` lists `finalize` may answer `revise`:
/// the objection is sent, and its event names the supervisor as actor.
#[test]
fn a_supervisor_that_decides_finalize_may_revise() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let path = env.data.pipeline(PROJECT);
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("\n[supervisor]\nguidance = \"Keep it moving.\"\ndecides = [\"finalize\"]\n");
    std::fs::write(path, text).unwrap();
    env.runner.actor = Some(dispatch::scheduler::BY_SUPERVISOR.into());
    revise(&mut env, &id, "The owner says: step 3 deletes data.");
    env.runner.actor = None;
    env.step();
    assert_eq!(env.sb().objections.len(), 1);
    let events = dispatch::events::read_since(&dispatch::events::log_path(&env.data), 0).unwrap();
    let revised = events
        .iter()
        .find(|e| e.ticket == id && e.kind == dispatch::events::Kind::Revised)
        .expect("a revised event");
    assert_eq!(revised.text, "r2: the owner's objection, by supervisor");
    assert_eq!(
        revised.actor.as_deref(),
        Some(dispatch::scheduler::BY_SUPERVISOR)
    );
}

/// Switchboard refusing the objection undoes it: the file and the
/// revision go, nothing reads as revised, and `finalize` is asked again.
#[test]
fn a_refused_objection_is_undone_and_finalize_is_asked_again() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let first = revise(&mut env, &id, "Split step 3.");
    env.sb().fail_next = Some("workflow.object".into());
    env.step();
    let copy = artifact_of(&env.ticket(&id), "review", "plan");
    assert!(!dispatch::report::round_file(&copy, 2).exists());
    let t = env.ticket(&id);
    assert!(t.attempts_of("review").last().unwrap().revisions.is_empty());
    assert!(!events_of(&env.data, &id).contains(&"revised".to_owned()));
    assert!(env.sb().objections.is_empty());
    let d = t.decisions.iter().find(|d| d.id == first).unwrap();
    assert!(matches!(
        d.state,
        DecisionState::Answered { acted: true, .. }
    ));
    env.step();
    let again = env.pending(&id);
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].name, "finalize");
    assert_ne!(again[0].id, first);
}

/// A lost reply may have opened the round: the file and the revision
/// stay, and the send is asked about once rather than repeated.
#[test]
fn a_lost_objection_keeps_its_round_and_asks_about_the_send() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    revise(&mut env, &id, "Split step 3.");
    env.sb().drop_reply_for = Some("workflow.object".into());
    env.step();
    let copy = artifact_of(&env.ticket(&id), "review", "plan");
    assert!(dispatch::report::round_file(&copy, 2).exists());
    let t = env.ticket(&id);
    assert_eq!(t.attempts_of("review").last().unwrap().revisions.len(), 1);
    assert!(events_of(&env.data, &id).contains(&"revised".to_owned()));
    env.restart();
    env.step();
    let t = env.ticket(&id);
    assert!(
        t.pending_decisions().iter().any(|d| d.name == "lost-send"),
        "{:#?}",
        t.decisions
    );
    assert_eq!(env.sb().kinds_called("workflow.object"), 1);
}

/// The port's `NO_ANSWER` is no refusal: the app still has the
/// objection and may open the round, so it is kept like a lost reply
/// and the send is asked about.
#[test]
fn an_objection_the_app_was_slow_to_answer_keeps_its_round() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    revise(&mut env, &id, "Split step 3.");
    env.sb().no_answer_for = Some("workflow.object".into());
    env.step();
    assert_eq!(env.sb().objections.len(), 1, "the app opened the round");
    let copy = artifact_of(&env.ticket(&id), "review", "plan");
    assert!(dispatch::report::round_file(&copy, 2).exists());
    let t = env.ticket(&id);
    assert_eq!(t.attempts_of("review").last().unwrap().revisions.len(), 1);
    let op = t
        .ledger
        .iter()
        .find(|o| o.kind == "workflow.object")
        .unwrap();
    assert!(op.reply.is_none() && op.error.is_some(), "{op:?}");
    assert_eq!(
        texts_of_kind(&env, &id, dispatch::events::Kind::Revised),
        ["r2: the owner's objection, by you"]
    );
    env.step();
    let t = env.ticket(&id);
    assert!(
        t.pending_decisions().iter().any(|d| d.name == "lost-send"),
        "{:#?}",
        t.decisions
    );
    assert_eq!(env.sb().kinds_called("workflow.object"), 1);
}

/// An objection lost before it reached Switchboard leaves the run
/// converged, so `finalize` is asked again; a second `revise` of the
/// same round replaces the first one's revision rather than adding one.
#[test]
fn a_second_objection_to_an_unopened_round_replaces_the_first() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let path = env.data.pipeline(PROJECT);
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("\n[supervisor]\nguidance = \"Keep it moving.\"\ndecides = [\"finalize\"]\n");
    std::fs::write(path, text).unwrap();
    env.runner.actor = Some(dispatch::scheduler::BY_SUPERVISOR.into());
    revise(&mut env, &id, "The supervisor's objection.");
    env.runner.actor = None;
    env.fail_once(
        "workflow.object",
        "the socket closed before the request went out",
    );
    env.step();
    assert!(env.sb().objections.is_empty(), "the request was lost");
    env.step();
    let finalize = env
        .pending(&id)
        .into_iter()
        .find(|d| d.name == "finalize")
        .expect("finalize asked again");
    let now = env.tick();
    env.runner
        .decide(&id, &finalize.id, "revise", Some("The owner's own."), now)
        .unwrap();
    env.step();
    assert_eq!(env.sb().objections.len(), 1);
    let t = env.ticket(&id);
    let revisions = &t.attempts_of("review").last().unwrap().revisions;
    assert_eq!(
        revisions
            .iter()
            .map(|r| (r.round, r.by.as_str()))
            .collect::<Vec<_>>(),
        [(2, "you")]
    );
    let copy = artifact_of(&t, "review", "plan");
    let file = std::fs::read_to_string(dispatch::report::round_file(&copy, 2)).unwrap();
    assert!(file.contains("The owner's own."), "{file}");
    assert_eq!(
        texts_of_kind(&env, &id, dispatch::events::Kind::Revised)
            .last()
            .map(String::as_str),
        Some("r2: the owner's objection, by you")
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

/// The implementer stopped and its checks running: the ticket id and
/// the checks' key.
fn at_running_checks(env: &mut Env) -> (String, String) {
    let (id, implementer) = at_implement(env);
    implementer_stops(env, &id, &implementer);
    env.steps_until(&id, "the checks starting", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.gate.is_some())
    });
    let key = format!("{id}/implement/1");
    assert!(env.repo.lock().unwrap().checks.iter().any(|c| c.key == key));
    (id, key)
}

/// The intent to park saved, as a hand park and a decision's park both
/// leave it for `finish_parking`.
fn park_by_hand(env: &mut Env, id: &str) {
    let mut t = env.ticket(id);
    t.state = TicketState::Parking {
        reason: "parked by hand".into(),
    };
    let now = env.tick();
    env.runner.save_ticket(&mut t, now).unwrap();
}

/// The intent to close saved and the id on the project's closing list.
fn close_by_record(env: &Env, id: &str) {
    write_closing(env, id, |_| {});
    let mut ps = env.runner.load_project(PROJECT).unwrap();
    ps.queue.retain(|q| q != id);
    ps.closing.push(id.to_owned());
    env.runner.save_project(&ps).unwrap();
}

fn first_implement(env: &Env, id: &str) -> Attempt {
    env.ticket(id)
        .attempts_of("implement")
        .next()
        .unwrap()
        .clone()
}

fn implement_state(env: &Env, id: &str) -> AttemptState {
    first_implement(env, id).state
}

#[test]
fn parking_during_checks_kills_them_before_the_attempt_reads_cancelled() {
    let mut env = Env::new();
    let (id, key) = at_running_checks(&mut env);
    env.repo.lock().unwrap().stubborn_checks.push(key.clone());
    park_by_hand(&mut env, &id);
    env.step();
    assert_eq!(env.repo.lock().unwrap().killed_checks, vec![key.clone()]);
    assert!(
        first_implement(&env, &id).is_open(),
        "not cancelled while it runs"
    );
    assert!(matches!(env.ticket(&id).state, TicketState::Parking { .. }));
    {
        let mut repo = env.repo.lock().unwrap();
        repo.checks.retain(|c| c.key != key);
    }
    env.step();
    assert!(
        matches!(&implement_state(&env, &id), AttemptState::Cancelled { reason } if !reason.contains("still running")),
        "{:?}",
        implement_state(&env, &id)
    );
    assert!(matches!(env.ticket(&id).state, TicketState::Parked { .. }));
    // The resume is the answer: what the park cancelled runs again
    // with no question.
    let now = env.tick();
    env.runner.resume(&id, now).unwrap();
    env.step();
    assert!(env.pending(&id).is_empty(), "{:#?}", env.pending(&id));
    env.steps_until(&id, "the second implementer", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.n == 2 && a.session.is_some())
    });
    let implementer = session_of(&env.ticket(&id), "implement");
    implementer_stops(&mut env, &id, &implementer);
    env.steps_until(&id, "the second checks starting", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.n == 2 && a.gate.is_some())
    });
    let repo = env.repo.lock().unwrap();
    assert!(repo.killed_checks.contains(&key));
    let started: Vec<&str> = repo.checks.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(started, vec![format!("{id}/implement/2").as_str()]);
}

/// The implementer running with a question pending about something
/// else: a stage-wide `inspect` question nothing would ask yet.
fn at_implement_with_a_question(env: &mut Env) -> (String, String, String) {
    let (id, implementer) = at_implement(env);
    let mut t = env.ticket(&id);
    let d = format!("d{}", t.decisions.len() + 1);
    t.decisions.push(Decision {
        id: d.clone(),
        stage: "plan".into(),
        name: "inspect".into(),
        kind: dispatch::ticket::DecisionKind::Confirmation,
        question: "Look at it?".into(),
        options: vec!["done".into(), "park".into()],
        recommendation: None,
        attempt: None,
        state: DecisionState::Pending,
        made_ms: env.now,
        refusals: Vec::new(),
    });
    let now = env.tick();
    env.runner.save_ticket(&mut t, now).unwrap();
    (id, implementer, d)
}

/// `dispatch park` writes the intent with the questions withdrawn; the
/// runner's next pass stops what runs and reads as parked.
fn parked_by_command(env: &mut Env, reason: &str) -> (String, String) {
    let (id, implementer, d) = at_implement_with_a_question(env);
    let now = env.tick();
    env.runner.request_park(&id, Some(reason), now).unwrap();
    let t = env.ticket(&id);
    assert!(
        matches!(&t.state, TicketState::Parking { reason: r } if r == reason),
        "{:?}",
        t.state
    );
    let withdrawn = t.decisions.iter().find(|x| x.id == d).unwrap();
    assert_eq!(withdrawn.state, DecisionState::Cancelled);
    assert!(first_implement(env, &id).is_open(), "the runner stops it");
    env.step();
    let t = env.ticket(&id);
    assert!(
        matches!(&t.state, TicketState::Parked { reason: r } if r == reason),
        "{t:#?}"
    );
    assert!(
        matches!(&implement_state(env, &id), AttemptState::Cancelled { reason: r } if r == reason),
        "{:?}",
        implement_state(env, &id)
    );
    (id, implementer)
}

#[test]
fn dispatch_park_stops_a_running_attempt_and_withdraws_an_unrelated_question() {
    let mut env = Env::new();
    let (id, implementer) = parked_by_command(&mut env, "pr launched mid-rebase");
    {
        let sb = env.sb();
        assert!(sb.killed.contains(&implementer), "{:?}", sb.killed);
        assert!(
            sb.sessions.iter().all(|s| s.liveness != Liveness::Running),
            "parking left something running"
        );
        assert!(sb.waiting.values().all(|(on, _)| !on), "{:?}", sb.waiting);
    }
    let kinds = events_of(&env.data, &id);
    assert_in_order(&kinds, &["parking", "parked"]);
    assert!(kinds.iter().any(|k| k == "decision-cancelled"), "{kinds:?}");
    let texts = event_texts(&env, &id);
    assert_eq!(
        texts
            .iter()
            .filter(|t| t.as_str() == "pr launched mid-rebase")
            .count(),
        2,
        "parking and parked carry the reason: {texts:?}"
    );
}

#[test]
fn a_resume_reruns_what_the_park_cancelled_without_asking() {
    let mut env = Env::new();
    let (id, _) = parked_by_command(&mut env, "pr launched mid-rebase");
    let now = env.tick();
    let resumed = env.runner.resume(&id, now).unwrap();
    let authorised: Vec<_> = resumed
        .reruns
        .iter()
        .map(|a| (a.stage.as_str(), a.n))
        .collect();
    assert_eq!(authorised, [("implement", 1)]);
    assert_eq!(resumed.no_reruns, None);
    let t = env.ticket(&id);
    let rerun = t.decisions.last().unwrap();
    assert_eq!(rerun.name, "rerun");
    assert_eq!(rerun.attempt, Some(("implement".to_owned(), 1)));
    assert!(
        matches!(&rerun.state, DecisionState::Answered { answer, by, acted: false, .. } if answer == "rerun" && by == "resume"),
        "{:?}",
        rerun.state
    );
    assert!(
        rerun
            .question
            .contains("was cancelled: pr launched mid-rebase"),
        "{}",
        rerun.question
    );
    assert!(env.pending(&id).is_empty());
    let resumed = dispatch::events::read_since(&dispatch::events::log_path(&env.data), 0)
        .unwrap()
        .into_iter()
        .rfind(|e| e.ticket == id && e.kind == dispatch::events::Kind::Resumed)
        .unwrap();
    assert_eq!(resumed.text, "active again, rerunning implement (repo)");
    env.steps_until(&id, "the second implementer", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.n == 2 && a.session.is_some())
    });
    assert!(env.pending(&id).is_empty(), "{:#?}", env.pending(&id));
}

/// A pass stamps its writes with the time it began, so a park written
/// while it ran can read later than the cancellation it caused; the
/// log's order still counts that attempt as the park's.
#[test]
fn a_resume_reruns_what_a_park_cancelled_in_a_pass_that_began_before_it() {
    let mut env = Env::new();
    let (id, _, _) = at_implement_with_a_question(&mut env);
    let pass_began = env.tick();
    let now = env.tick();
    env.runner.request_park(&id, Some("for now"), now).unwrap();
    env.runner.step_all(pass_began).unwrap();
    let t = env.ticket(&id);
    assert!(
        matches!(t.state, TicketState::Parked { .. }),
        "{:?}",
        t.state
    );
    assert!(
        first_implement(&env, &id)
            .ended_ms
            .is_some_and(|ended| ended < now),
        "{:?}",
        first_implement(&env, &id)
    );
    let now = env.tick();
    let resumed = env.runner.resume(&id, now).unwrap();
    let authorised: Vec<_> = resumed
        .reruns
        .iter()
        .map(|a| (a.stage.as_str(), a.n))
        .collect();
    assert_eq!(authorised, [("implement", 1)]);
}

#[test]
fn a_resume_with_no_rerun_asks_as_before() {
    let mut env = Env::new();
    let (id, _) = parked_by_command(&mut env, "pr launched mid-rebase");
    let now = env.tick();
    env.runner.resume_asking(&id, now).unwrap();
    env.steps_until(&id, "the question again", |t, _| {
        !t.pending_decisions().is_empty()
    });
    let again = env.pending(&id).remove(0);
    assert_eq!(again.name, "rerun");
    assert!(
        again
            .question
            .contains("was cancelled: pr launched mid-rebase"),
        "{}",
        again.question
    );
    assert_eq!(env.ticket(&id).attempts_of("implement").count(), 1);
}

#[test]
fn what_an_earlier_park_with_the_same_reason_cancelled_is_asked_not_rerun() {
    let mut env = Env::new();
    let (id, _) = parked_by_command(&mut env, "for now");
    // Resumed asking, and parked again before any pass asked.
    let now = env.tick();
    env.runner.resume_asking(&id, now).unwrap();
    let now = env.tick();
    env.runner.request_park(&id, Some("for now"), now).unwrap();
    env.step();
    assert!(matches!(env.ticket(&id).state, TicketState::Parked { .. }));
    let now = env.tick();
    let resumed = env.runner.resume(&id, now).unwrap();
    assert!(resumed.reruns.is_empty(), "{:?}", resumed.reruns);
    env.steps_until(&id, "the question again", |t, _| {
        !t.pending_decisions().is_empty()
    });
    assert_eq!(env.pending(&id).remove(0).name, "rerun");
    assert_eq!(env.ticket(&id).attempts_of("implement").count(), 1);
}

#[test]
fn a_resume_reruns_an_attempt_whose_checks_outlived_the_park() {
    let mut env = Env::new();
    let (id, key) = at_running_checks(&mut env);
    env.repo.lock().unwrap().stubborn_checks.push(key.clone());
    park_by_hand(&mut env, &id);
    env.step();
    env.wait(STOP_LIMIT_MS);
    env.step();
    assert!(
        matches!(&implement_state(&env, &id), AttemptState::Cancelled { reason } if reason.starts_with("parked by hand; ") && reason.contains("still running")),
        "{:?}",
        implement_state(&env, &id)
    );
    env.repo.lock().unwrap().checks.retain(|c| c.key != key);
    let now = env.tick();
    env.runner.resume(&id, now).unwrap();
    env.steps_until(&id, "the second implementer", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.n == 2 && a.session.is_some())
    });
    assert!(env.pending(&id).is_empty(), "{:#?}", env.pending(&id));
}

#[test]
fn park_on_a_closed_ticket_is_a_usage_error() {
    let mut env = Env::new();
    let (id, _) = parked_by_command(&mut env, "for now");
    // Parked or parking is refused, but not as a usage error.
    let now = env.tick();
    let again = env.runner.request_park(&id, None, now).unwrap_err();
    assert!(again.downcast_ref::<dispatch::UsageError>().is_none());
    assert_eq!(
        again.to_string(),
        format!("ticket {id} is already parked: for now")
    );
    for (state, word) in [
        (
            TicketState::Closing {
                reason: "done".into(),
            },
            "closing",
        ),
        (
            TicketState::Closed {
                reason: "done".into(),
            },
            "closed",
        ),
    ] {
        let mut t = env.ticket(&id);
        t.state = state;
        let now = env.tick();
        env.runner.save_ticket(&mut t, now).unwrap();
        let now = env.tick();
        let e = env.runner.request_park(&id, None, now).unwrap_err();
        assert_eq!(
            e.downcast_ref::<dispatch::UsageError>()
                .map(ToString::to_string),
            Some(format!("ticket {id} is {word}"))
        );
    }
    // The command maps it to 64; `park` is offline, so no Switchboard
    // is needed.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_dispatch"))
        .args(["park", &id])
        .env("DISPATCH_DATA_DIR", &env.data.root)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stderr).trim(),
        format!("ticket {id} is closed")
    );
}

#[test]
fn closing_during_checks_kills_them_before_the_attempt_reads_cancelled() {
    let mut env = Env::new();
    let (id, key) = at_running_checks(&mut env);
    env.repo.lock().unwrap().stubborn_checks.push(key.clone());
    close_by_record(&env, &id);
    env.step();
    assert!(matches!(env.ticket(&id).state, TicketState::Closing { .. }));
    assert!(
        first_implement(&env, &id).is_open(),
        "not cancelled while it runs"
    );
    assert_eq!(env.repo.lock().unwrap().killed_checks, vec![key.clone()]);
    env.repo.lock().unwrap().checks.retain(|c| c.key != key);
    env.step();
    assert!(
        matches!(&implement_state(&env, &id), AttemptState::Cancelled { reason } if reason.starts_with("the ticket closed: ") && !reason.contains("still running")),
        "{:?}",
        implement_state(&env, &id)
    );
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Closed { .. }), "{t:#?}");
}

#[test]
fn a_check_that_ignores_the_kill_does_not_hold_the_park_past_the_limit() {
    let mut env = Env::new();
    let (id, key) = at_running_checks(&mut env);
    env.repo.lock().unwrap().stubborn_checks.push(key.clone());
    park_by_hand(&mut env, &id);
    env.step();
    assert!(matches!(env.ticket(&id).state, TicketState::Parking { .. }));
    assert!(env.repo.lock().unwrap().escalated_checks.is_empty());
    env.wait(STOP_LIMIT_MS);
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Parked { .. }), "{t:#?}");
    assert!(
        matches!(&implement_state(&env, &id), AttemptState::Cancelled { reason } if reason.contains("still running") && reason.contains(&key)),
        "{:?}",
        implement_state(&env, &id)
    );
    assert_eq!(env.repo.lock().unwrap().escalated_checks, vec![key]);
}

#[test]
fn parking_after_a_restart_reads_a_lost_check_as_gone() {
    let mut env = Env::new();
    let (id, _) = at_running_checks(&mut env);
    env.repo.lock().unwrap().checks.clear();
    env.restart();
    park_by_hand(&mut env, &id);
    env.step();
    assert!(matches!(
        implement_state(&env, &id),
        AttemptState::Cancelled { .. }
    ));
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Parked { .. }), "{t:#?}");
    assert!(
        env.repo.lock().unwrap().checks.is_empty(),
        "nothing started again"
    );
}

/// The runner dies but the check it started keeps running in its own
/// group, and a new runner comes up over the same records.
fn orphan_and_restart(env: &mut Env, key: &str) {
    {
        let mut repo = env.repo.lock().unwrap();
        repo.checks.retain(|c| c.key != key);
        repo.orphans.insert(key.to_owned());
    }
    env.restart();
}

fn started_under(env: &Env, key: &str) -> usize {
    env.repo
        .lock()
        .unwrap()
        .checks
        .iter()
        .filter(|c| c.key == key)
        .count()
}

fn orphan_events(env: &Env, id: &str) -> usize {
    events_of(&env.data, id)
        .iter()
        .filter(|k| *k == "check-orphan-killed")
        .count()
}

#[test]
fn an_orphaned_check_is_stopped_before_the_checks_start_again() {
    let mut env = Env::new();
    let (id, key) = at_running_checks(&mut env);
    let group = first_implement(&env, &id).gate.unwrap().group.unwrap();
    orphan_and_restart(&mut env, &key);
    for _ in 0..3 {
        env.step();
    }
    assert_eq!(env.repo.lock().unwrap().adopted, vec![key.clone()]);
    assert_eq!(started_under(&env, &key), 0, "nothing starts beside it");
    assert!(first_implement(&env, &id).is_open());
    env.repo.lock().unwrap().orphans.remove(&key);
    env.step();
    assert_eq!(started_under(&env, &key), 1);
    let a = first_implement(&env, &id);
    let gate = a.gate.unwrap();
    assert_eq!((gate.head.as_str(), gate.exit), ("base0000", None));
    assert!(gate.group.is_some_and(|g| g != group));
    assert_eq!(a.orphans_killed.len(), 1);
    assert_eq!(
        (
            a.orphans_killed[0].pgid,
            &a.orphans_killed[0].leader_started
        ),
        (group.pgid, &group.leader_started)
    );
    assert_eq!(orphan_events(&env, &id), 1);
    assert!(env.repo.lock().unwrap().escalated_checks.is_empty());
}

#[test]
fn an_orphan_that_ignores_term_is_killed_past_the_limit() {
    let mut env = Env::new();
    let (id, key) = at_running_checks(&mut env);
    orphan_and_restart(&mut env, &key);
    env.step();
    env.step();
    assert!(env.repo.lock().unwrap().escalated_checks.is_empty());
    assert_eq!(started_under(&env, &key), 0);
    env.wait(STOP_LIMIT_MS);
    env.step();
    assert_eq!(env.repo.lock().unwrap().escalated_checks, vec![key.clone()]);
    assert_eq!(started_under(&env, &key), 1);
    env.step();
    assert_eq!(started_under(&env, &key), 1, "started once");
    assert_eq!(first_implement(&env, &id).orphans_killed.len(), 1);
}

#[test]
fn a_stale_orphan_group_is_cleared() {
    let mut env = Env::new();
    let (id, key) = at_running_checks(&mut env);
    let group = first_implement(&env, &id).gate.unwrap().group.unwrap();
    env.repo.lock().unwrap().checks.clear();
    env.restart();
    env.step();
    assert_eq!(started_under(&env, &key), 1);
    let a = first_implement(&env, &id);
    assert!(a.gate.unwrap().group.is_some_and(|g| g != group));
    assert!(a.orphans_killed.is_empty());
    assert!(env.repo.lock().unwrap().adopted.is_empty());
    assert_eq!(orphan_events(&env, &id), 0);
}

#[test]
fn a_second_orphan_under_a_reused_pgid_gets_its_own_kill() {
    let mut env = Env::new();
    let (id, key) = at_running_checks(&mut env);
    let first = first_implement(&env, &id).gate.unwrap().group.unwrap();
    orphan_and_restart(&mut env, &key);
    env.step();
    {
        let mut repo = env.repo.lock().unwrap();
        repo.orphans.remove(&key);
        // The kernel hands the emptied group's id to the next check.
        repo.next_pgid = first.pgid;
    }
    env.step();
    let second = first_implement(&env, &id).gate.unwrap().group.unwrap();
    assert_eq!(second.pgid, first.pgid);
    assert_ne!(second.leader_started, first.leader_started);
    // Long after the first kill, the new check is orphaned too.
    env.wait(STOP_LIMIT_MS);
    orphan_and_restart(&mut env, &key);
    env.step();
    assert!(
        env.repo.lock().unwrap().escalated_checks.is_empty(),
        "a TERM and the full grace, not a SIGKILL"
    );
    let a = first_implement(&env, &id);
    assert_eq!(a.orphans_killed.len(), 2);
    assert!(a.orphans_killed.iter().all(|o| o.pgid == first.pgid));
    assert_eq!(a.orphans_killed[1].leader_started, second.leader_started);
    assert_eq!(orphan_events(&env, &id), 2);
    assert_eq!(started_under(&env, &key), 0);
}

#[test]
fn parking_after_a_restart_waits_on_an_orphaned_check() {
    let mut env = Env::new();
    let (id, key) = at_running_checks(&mut env);
    orphan_and_restart(&mut env, &key);
    park_by_hand(&mut env, &id);
    env.step();
    env.step();
    assert!(
        first_implement(&env, &id).is_open(),
        "not cancelled while it runs"
    );
    assert!(matches!(env.ticket(&id).state, TicketState::Parking { .. }));
    env.repo.lock().unwrap().orphans.remove(&key);
    env.step();
    assert!(
        matches!(&implement_state(&env, &id), AttemptState::Cancelled { reason } if !reason.contains("still running")),
        "{:?}",
        implement_state(&env, &id)
    );
    assert_eq!(first_implement(&env, &id).orphans_killed.len(), 1);
    assert_eq!(env.repo.lock().unwrap().adopted, vec![key.clone()]);
    assert!(matches!(env.ticket(&id).state, TicketState::Parked { .. }));
    assert_eq!(started_under(&env, &key), 0, "nothing started again");
}

#[test]
fn a_leaderless_orphan_with_a_fresh_kill_is_killed_again() {
    let mut env = Env::new();
    let (id, key) = at_running_checks(&mut env);
    orphan_and_restart(&mut env, &key);
    env.step();
    let at_ms = first_implement(&env, &id).orphans_killed[0].at_ms;
    // The leader died of the TERM; a member that ignored it runs on.
    env.repo.lock().unwrap().leaderless.insert(key.clone());
    env.restart();
    env.step();
    {
        let repo = env.repo.lock().unwrap();
        assert_eq!(repo.adopted, vec![key.clone(), key.clone()]);
        assert_eq!(repo.vouched, vec![key.clone()]);
        assert!(repo.escalated_checks.is_empty());
    }
    assert_eq!(first_implement(&env, &id).orphans_killed.len(), 1);
    assert_eq!(orphan_events(&env, &id), 1);
    assert_eq!(started_under(&env, &key), 0);
    // SIGKILL at the first kill's time plus the limit, not a fresh limit.
    env.now = at_ms + STOP_LIMIT_MS;
    env.step();
    assert_eq!(env.repo.lock().unwrap().escalated_checks, vec![key.clone()]);
    assert_eq!(started_under(&env, &key), 1);
    env.step();
    assert_eq!(started_under(&env, &key), 1, "started once");
}

#[test]
fn a_leaderless_orphan_past_the_limit_is_left_alone() {
    let mut env = Env::new();
    let (id, key) = at_running_checks(&mut env);
    orphan_and_restart(&mut env, &key);
    env.step();
    env.wait(STOP_LIMIT_MS);
    env.repo.lock().unwrap().leaderless.insert(key.clone());
    env.restart();
    env.step();
    {
        let repo = env.repo.lock().unwrap();
        assert_eq!(repo.adopted, vec![key.clone()]);
        assert!(repo.vouched.is_empty());
        assert!(repo.orphans.contains(&key), "its members run on");
        assert!(repo.left_alone.contains(&key));
        assert!(repo.escalated_checks.is_empty());
    }
    assert_eq!(started_under(&env, &key), 1);
    let a = first_implement(&env, &id);
    assert_eq!(a.orphans_killed.len(), 1);
    assert_eq!(orphan_events(&env, &id), 1);
    env.step();
    assert_eq!(started_under(&env, &key), 1, "started once");
}

#[test]
fn a_review_rounds_orphaned_checks_are_stopped_before_they_start_again() {
    let (mut env, id, _) = findings_asked_and_fixed();
    let t = env.ticket(&id);
    let round = &review_attempt(&t).rounds[0];
    let fixer = round.implementer.clone().unwrap();
    let response = round.response.clone().unwrap();
    env.repo
        .lock()
        .unwrap()
        .heads
        .insert(t.lanes[0].worktree.clone(), "fix00001".into());
    env.finish(
        &fixer,
        &response,
        "- r1/style-1: fixed\n- r1/style-2: fixed\n- r1/lint-1: fixed\n",
    );
    env.steps_until(&id, "the checks after the fix", |t, _| {
        review_attempt(t).gate.is_some()
    });
    let key = checks_key(&env.ticket(&id), 1);
    orphan_and_restart(&mut env, &key);
    for _ in 0..3 {
        env.step();
    }
    assert_eq!(env.repo.lock().unwrap().adopted, vec![key.clone()]);
    assert_eq!(started_under(&env, &key), 0);
    env.repo.lock().unwrap().orphans.remove(&key);
    env.step();
    assert_eq!(started_under(&env, &key), 1);
    let a = review_attempt(&env.ticket(&id));
    assert!(
        a.gate
            .is_some_and(|g| g.head == "fix00001" && g.group.is_some())
    );
    assert_eq!(a.orphans_killed.len(), 1);
    assert_eq!(orphan_events(&env, &id), 1);
}

#[test]
fn closing_during_a_review_round_kills_its_command_reviewers() {
    let mut env = Env::new();
    let id = at_review(&mut env);
    let lint = lint_key(&env.ticket(&id), 1);
    assert!(
        env.repo
            .lock()
            .unwrap()
            .checks
            .iter()
            .any(|c| c.key == lint)
    );
    close_by_record(&env, &id);
    env.step();
    assert!(env.repo.lock().unwrap().killed_checks.contains(&lint));
    let t = env.ticket(&id);
    assert!(
        matches!(&review_attempt(&t).state, AttemptState::Cancelled { reason } if reason.starts_with("the ticket closed: ")),
        "{:?}",
        review_attempt(&t).state
    );
    assert!(matches!(t.state, TicketState::Closed { .. }), "{t:#?}");
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
repo = "git@example.com:example-org/orchard-workspace.git"
worktrees = "{worktrees}"
space = "Dispatch · Orchard"

[source]
kind = "github"
repo = "example-org/orchard-workspace"
label = "dispatch"
lane_hints = { "area:backend" = "backend", "area:frontend" = "frontend" }

[[lanes]]
name = "backend"
path = "orchard-backend"
repo = "git@example.com:example-org/orchard-backend.git"
base = "main"
setup = ["uv", "sync"]

[[lanes]]
name = "frontend"
path = "orchard-frontend"
repo = "git@example.com:example-org/orchard-frontend.git"
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
                taken_by: None,
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
    workspace_env_with(WORKSPACE, labels)
}

/// A ticket taken from `text`, a workspace pipeline with `{worktrees}`
/// still in it.
fn workspace_env_with(text: &str, labels: &[&str]) -> (Env, String) {
    let mut env = Env::new();
    let text = text.replace("{worktrees}", &env.worktrees.display().to_string());
    std::fs::write(env.data.pipeline("Orchard"), text).unwrap();
    let now = env.tick();
    let id = env
        .runner
        .take(
            "Orchard",
            &std::fs::read_to_string(env.data.pipeline("Orchard")).unwrap(),
            SourceSnapshot {
                kind: "github".into(),
                identity: "example-org/orchard-workspace#42".into(),
                number: Some(42),
                title: "Asset report column missing".into(),
                body: String::new(),
                url: None,
                labels: labels.iter().map(|l| (*l).to_owned()).collect(),
                taken_at_ms: now,
                taken_by: None,
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

/// The workspace with a third lane, and investigate and plan prompts
/// that show `{lanes}` and `{lanes.all}`; the brackets make an
/// assertion match the whole list, not a prefix of it.
fn three_lane_workspace() -> String {
    WORKSPACE
        .replace(
            "[operators.investigator]",
            r#"[[lanes]]
name = "admin"
path = "orchard-admin"
repo = "git@example.com:example-org/orchard-admin.git"
base = "main"

[operators.investigator]"#,
        )
        .replace(
            "Write to {notes}.",
            "Lanes [{lanes}] of [{lanes.all}]. Write to {notes}.",
        )
        .replace(
            "Plan in {worktree}",
            "Plan [{lanes}] of [{lanes.all}] in {worktree}",
        )
}

#[test]
fn lanes_is_the_chosen_lanes_and_lanes_all_every_lane_of_the_pipeline() {
    let (mut env, id) = workspace_env_with(&three_lane_workspace(), &["area:frontend"]);
    let now = env.tick();
    env.runner.step_project("Orchard", now).unwrap();
    assert!(
        env.data.repo_dir("Orchard@admin").exists(),
        "the third clone"
    );
    let prompt = last_prompt_of(&env, "investigator");
    assert!(
        prompt.contains("Lanes [backend, frontend, admin] of [backend, frontend, admin]."),
        "before the choice every lane: {prompt}"
    );
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
    let chosen: Vec<&str> = t
        .lanes
        .iter()
        .filter(|l| l.chosen)
        .map(|l| l.name.as_str())
        .collect();
    assert_eq!(chosen, ["frontend"]);
    assert_eq!(t.attempts_of("plan").count(), 1, "{t:#?}");
    let prompt = last_prompt_of(&env, "planner");
    assert!(
        prompt.contains("Plan [frontend] of [backend, frontend, admin] in"),
        "{prompt}"
    );
}

#[test]
fn lanes_follows_the_pipeline_order_not_the_answer_order() {
    let (mut env, id) = workspace_env_with(&three_lane_workspace(), &["type:bug"]);
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
    let now = env.tick();
    env.runner
        .decide(&id, &pending[0].id, "admin, backend", None, now)
        .unwrap();
    for _ in 0..8 {
        let now = env.tick();
        env.runner.step_project("Orchard", now).unwrap();
        if env.sb().sessions_named("planner").len() == 2 {
            break;
        }
    }
    assert_eq!(env.ticket(&id).attempts_of("plan").count(), 2);
    let prompts: Vec<String> = env
        .sb()
        .calls
        .iter()
        .filter_map(|r| match &r.body {
            Body::SessionNew {
                prompt, name: n, ..
            } if n == "planner" => prompt.clone(),
            _ => None,
        })
        .collect();
    assert_eq!(prompts.len(), 2, "{prompts:#?}");
    for prompt in &prompts {
        assert!(
            prompt.contains("Plan [backend, admin] of [backend, frontend, admin]"),
            "{prompt}"
        );
    }
}

#[test]
fn a_single_lane_pipeline_renders_its_lane_in_both() {
    let mut env = Env::new();
    // Before the take: the ticket freezes its own copy of the file.
    let path = env.data.pipeline(PROJECT);
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace(
            "Write to {notes}.",
            "Lanes [{lanes}] of [{lanes.all}]. Write to {notes}.",
        )
        .replace(
            "plan #{issue.number} to {plan}",
            "plan [{lanes}] of [{lanes.all}] #{issue.number} to {plan}",
        );
    std::fs::write(&path, text).unwrap();
    through_plan(&mut env, 7);
    for name in ["investigator", "planner"] {
        let prompt = last_prompt_of(&env, name);
        assert!(prompt.contains("[repo] of [repo]"), "{name}: {prompt}");
    }
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
    with_policy(env, t, &format!("max_reruns = {n}"));
}

/// One more `[policy]` line in the project's pipeline and in the
/// ticket's copy.
fn with_policy(env: &Env, t: &Ticket, line: &str) {
    for path in [env.data.pipeline(PROJECT), t.pipeline_file.clone()] {
        let text = std::fs::read_to_string(&path).unwrap().replace(
            "waiting_on_me = 3\n",
            &format!("waiting_on_me = 3\n{line}\n"),
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

#[test]
fn a_resume_reruns_the_cancelled_lane_and_asks_about_the_failed_one() {
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
    env.sb()
        .remove(plan_of(&t, "backend").session.as_ref().unwrap());
    env.steps_until(&id, "the backend plan failing", |t, _| {
        rerun_for(t, "backend").is_some()
    });
    assert!(plan_of(&env.ticket(&id), "frontend").is_open());
    let now = env.tick();
    env.runner
        .request_park(&id, Some("stopped for now"), now)
        .unwrap();
    env.steps_until(&id, "parked", |t, _| {
        matches!(t.state, TicketState::Parked { .. })
    });
    let now = env.tick();
    env.runner.resume(&id, now).unwrap();
    env.steps_until(&id, "the frontend planner again", |t, _| {
        let a = plan_of(t, "frontend");
        a.n > 2 && a.session.is_some()
    });
    let t = env.ticket(&id);
    assert!(
        matches!(plan_of(&t, "backend").state, AttemptState::Failed { .. }),
        "the failed lane is not rerun without an answer"
    );
    let asked = rerun_for(&t, "backend").expect("the failed lane is asked about");
    assert!(asked.question.contains("failed"), "{}", asked.question);
    assert_eq!(
        t.pending_decisions().len(),
        1,
        "{:#?}",
        t.pending_decisions()
    );
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
    let (id, implementer) = before_review(env);
    review_starts(env, &id, &implementer);
    id
}

/// `at_review` with the plan the reviewers are pointed at replaced by
/// `text`, or removed when there is none.
fn at_review_with_plan(env: &mut Env, text: Option<&str>) -> String {
    let (id, implementer) = before_review(env);
    let plan = env.ticket(&id).input("plan").cloned().unwrap();
    match text {
        Some(text) => std::fs::write(&plan, text).unwrap(),
        None => std::fs::remove_file(&plan).unwrap(),
    }
    review_starts(env, &id, &implementer);
    id
}

/// The first half of `at_review`: the implementer running on a lane
/// whose base is `root0000`.
fn before_review(env: &mut Env) -> (String, String) {
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
    at_implement(env)
}

/// The second half of `at_review`: the implementer stops, its checks
/// pass and the reviewers start.
fn review_starts(env: &mut Env, id: &str, implementer: &str) {
    let id = id.to_owned();
    implementer_stops(env, &id, implementer);
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

/// A command sibling that outlives the round's TERM is waited for, not
/// polled: its kill took it from the runner, and a poll would fail it
/// as lost to a restart that never happened, as the retired agent
/// sibling would be failed as exited.
#[test]
fn a_failed_rounds_stubborn_command_sibling_is_not_read_as_lost() {
    let mut env = Env::new();
    env.with_review_stage("ask");
    let path = env.data.pipeline(PROJECT);
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace(
            "[operators.rebaser]\n",
            "[operators.test]\nkind = \"command\"\nargv = [\"sh\", \"-c\", \"test\"]\n\n[operators.rebaser]\n",
        )
        .replace(
            "reviewers = [\"style\", \"lint\"]",
            "reviewers = [\"style\", \"lint\", \"test\"]",
        );
    std::fs::write(&path, text).unwrap();
    let id = at_review(&mut env);
    let test_key = lint_key(&env.ticket(&id), 1).replace("/lint", "/test");
    env.repo
        .lock()
        .unwrap()
        .stubborn_checks
        .push(test_key.clone());
    lint_exits(&mut env, &id, 1, 2, "");
    for _ in 0..3 {
        env.step();
    }
    assert_eq!(
        env.repo
            .lock()
            .unwrap()
            .killed_checks
            .iter()
            .filter(|k| **k == test_key)
            .count(),
        1
    );
    let t = env.ticket(&id);
    assert!(review_attempt(&t).is_open(), "waits on the sibling");
    assert_eq!(reviewer(&t, 1, "test").result, None);
    env.repo
        .lock()
        .unwrap()
        .checks
        .retain(|c| c.key != test_key);
    env.steps_until(&id, "the rerun question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    let a = review_attempt(&env.ticket(&id));
    assert!(
        matches!(&a.state, AttemptState::Failed { reason } if reason.contains("lint: exited 2") && !reason.contains("lost") && !reason.contains("style")),
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
/// implementer leaving it dirty past the commit wait is nudged once, and
/// a second dirty stop past the wait fails its round.
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
    env.steps_until(&id, "the dirty clock", |t, _| {
        review_attempt(t).rounds[0].dirty_since_ms.is_some()
    });
    env.wait(DIRTY_WAIT_MS - 2_000);
    env.step();
    assert!(
        env.pending(&id).is_empty(),
        "no question while the tree may still be committed"
    );
    env.wait(2_000);
    env.steps_until(&id, "the nudge", |_, sb| !sb.sent.is_empty());
    let implementer = review_attempt(&env.ticket(&id)).rounds[0]
        .implementer
        .clone()
        .unwrap();
    assert_eq!(
        env.sb().sent,
        vec![(implementer.clone(), NUDGE_TEXT.into())]
    );
    env.step();
    assert!(env.pending(&id).is_empty(), "a nudge, not a question");
    // It stops again with the tree still dirty: the wait again, then the
    // question.
    let now = env.tick();
    env.sb().stop(&implementer, now);
    env.steps_until(&id, "the dirty clock again", |t, _| {
        review_attempt(t).rounds[0].dirty_since_ms.is_some()
    });
    env.wait(DIRTY_WAIT_MS);
    env.steps_until(&id, "the rerun question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    let a = review_attempt(&env.ticket(&id));
    assert!(
        matches!(&a.state, AttemptState::Failed { reason } if reason.contains("left the tree") && reason.contains("after 1 nudge")),
        "{:?}",
        a.state
    );
    assert_eq!(a.rounds[0].nudges.len(), 1);
    assert_eq!(env.sb().sent.len(), 1);
}

/// A fixer nudged about its dirty tree whose pane then goes fails its
/// round on the tree, saying it was nudged.
#[test]
fn a_nudged_fixer_whose_pane_goes_fails_on_its_dirty_tree() {
    let mut env = Env::new();
    env.with_review_stage("auto");
    let id = at_review(&mut env);
    let tree = env.ticket(&id).lanes[0].worktree.clone();
    lint_exits(&mut env, &id, 1, 0, "");
    style_says(&mut env, &id, 1, "- a point\n");
    env.steps_until(&id, "the implementer", |t, _| {
        review_attempt(t).rounds[0].implementer.is_some()
    });
    let round = review_attempt(&env.ticket(&id)).rounds[0].clone();
    let implementer = round.implementer.unwrap();
    env.repo.lock().unwrap().dirty.push(tree);
    env.finish(
        &implementer,
        &round.response.unwrap(),
        "- r1/style-1: fixed\n",
    );
    env.steps_until(&id, "the dirty clock", |t, _| {
        review_attempt(t).rounds[0].dirty_since_ms.is_some()
    });
    env.wait(DIRTY_WAIT_MS);
    env.steps_until(&id, "the nudge", |_, sb| !sb.sent.is_empty());
    env.sb().vanish(&implementer);
    env.steps_until(&id, "the rerun question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    let a = review_attempt(&env.ticket(&id));
    assert!(
        matches!(&a.state, AttemptState::Failed { reason } if reason.contains("left the tree")
            && reason.contains("after 1 nudge; the session ended with no stop after the last nudge")),
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
    let since = review_attempt(&t).rounds[0].dirty_since_ms;
    assert!(since.is_some());
    // A restarted runner keeps the clock the first dirty pass started.
    env.restart();
    env.step();
    let t = env.ticket(&id);
    assert_eq!(review_attempt(&t).rounds[0].dirty_since_ms, since);
    assert!(t.pending_decisions().is_empty(), "{t:#?}");
    env.repo.lock().unwrap().dirty.clear();
    env.steps_until(&id, "the round fixed", |t, _| {
        review_attempt(t).rounds[0].head_after.is_some()
    });
    assert!(env.pending(&id).is_empty());
    assert!(env.sb().sent.is_empty());
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

#[test]
fn an_orphaned_command_reviewer_is_stopped_before_the_rounds_reviewers_start_again() {
    let mut env = Env::new();
    let id = at_review(&mut env);
    let t = env.ticket(&id);
    let key = lint_key(&t, 1);
    let group = reviewer(&t, 1, "lint").group.expect("the reviewer's group");
    let head = review_attempt(&t).rounds[0].head.clone();
    orphan_and_restart(&mut env, &key);
    for _ in 0..3 {
        env.step();
    }
    assert_eq!(env.repo.lock().unwrap().adopted, vec![key.clone()]);
    let t = env.ticket(&id);
    assert_eq!(reviewer(&t, 1, "lint").result, None);
    assert!(!t.pending_decisions().iter().any(|d| d.name == "rerun"));
    let a = review_attempt(&t);
    assert!(a.is_open());
    assert_eq!(a.orphans_killed.len(), 1);
    let o = &a.orphans_killed[0];
    assert_eq!(
        (o.reviewer.as_deref(), o.pgid, &o.leader_started, &o.head),
        (Some("lint"), group.pgid, &group.leader_started, &head)
    );
    assert_eq!(orphan_events(&env, &id), 1);
    env.repo.lock().unwrap().orphans.remove(&key);
    env.steps_until(&id, "the rerun question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    let a = review_attempt(&env.ticket(&id));
    assert!(
        matches!(&a.state, AttemptState::Failed { reason } if reason.contains("lint: lost") && !reason.contains("still running")),
        "{:?}",
        a.state
    );
    let d = env
        .pending(&id)
        .into_iter()
        .find(|d| d.name == "rerun")
        .unwrap();
    answer(&mut env, &id, &d, "rerun");
    env.steps_until(&id, "the new attempt's reviewers", |t, _| {
        review_attempt(t).n == a.n + 1
            && review_attempt(t).rounds.first().is_some_and(|r| {
                r.reviewers
                    .iter()
                    .all(|x| x.session.is_some() || x.launched)
            })
    });
    let t = env.ticket(&id);
    assert_eq!(started_under(&env, &lint_key(&t, 1)), 1);
    assert_eq!(started_under(&env, &key), 0, "the lost one never again");
}

#[test]
fn an_orphaned_command_reviewer_that_ignores_term_is_killed_past_the_limit() {
    let mut env = Env::new();
    let id = at_review(&mut env);
    let key = lint_key(&env.ticket(&id), 1);
    orphan_and_restart(&mut env, &key);
    env.step();
    env.step();
    assert!(env.repo.lock().unwrap().escalated_checks.is_empty());
    env.wait(STOP_LIMIT_MS);
    env.steps_until(&id, "the rerun question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    assert_eq!(env.repo.lock().unwrap().escalated_checks, vec![key.clone()]);
    let a = review_attempt(&env.ticket(&id));
    assert!(
        matches!(&a.state, AttemptState::Failed { reason } if reason.contains("lint: lost") && reason.contains("its reviewer lint") && reason.contains("still running after")),
        "{:?}",
        a.state
    );
}

#[test]
fn parking_after_a_restart_waits_on_an_orphaned_command_reviewer() {
    let mut env = Env::new();
    let id = at_review(&mut env);
    let key = lint_key(&env.ticket(&id), 1);
    orphan_and_restart(&mut env, &key);
    park_by_hand(&mut env, &id);
    env.step();
    env.step();
    let t = env.ticket(&id);
    assert!(review_attempt(&t).is_open(), "not cancelled while it runs");
    assert!(matches!(t.state, TicketState::Parking { .. }));
    assert_eq!(env.repo.lock().unwrap().adopted, vec![key.clone()]);
    env.repo.lock().unwrap().orphans.remove(&key);
    env.step();
    let t = env.ticket(&id);
    assert!(
        matches!(&review_attempt(&t).state, AttemptState::Cancelled { reason } if !reason.contains("still running")),
        "{:?}",
        review_attempt(&t).state
    );
    assert_eq!(review_attempt(&t).orphans_killed.len(), 1);
    assert!(
        matches!(t.state, TicketState::Parked { .. }),
        "{:?}",
        t.state
    );
}

// --- review rounds carry forward

/// `review_pipeline` with room for three rounds and the fix passes run
/// without asking.
fn with_three_rounds(env: &Env) {
    let path = env.data.pipeline(PROJECT);
    let text = review_pipeline(&env.data.root.join("wt"), "auto").replace("cap = 2\n", "cap = 4\n");
    std::fs::write(path, text).unwrap();
}

/// Waits for round `n` of the latest review attempt to have every
/// reviewer started.
fn round_started(env: &mut Env, id: &str, n: usize) {
    env.steps_until(id, "the round's reviewers", |t, _| {
        t.attempts_of("review-code")
            .last()
            .and_then(|a| a.rounds.get(n - 1))
            .is_some_and(|r| {
                r.reviewers
                    .iter()
                    .all(|x| x.session.is_some() || x.launched)
            })
    });
}

/// Round `n`'s implementer (started without asking) commits `head`
/// and answers with `response`; the checks after it exit `code`.
fn fix_pass(env: &mut Env, id: &str, n: u32, head: &str, response: &str, code: i32) {
    let i = n as usize - 1;
    env.steps_until(id, "the implementer", |t, _| {
        review_attempt(t)
            .rounds
            .get(i)
            .is_some_and(|r| r.implementer.is_some())
    });
    let t = env.ticket(id);
    let round = review_attempt(&t).rounds[i].clone();
    env.repo
        .lock()
        .unwrap()
        .heads
        .insert(t.lanes[0].worktree.clone(), head.into());
    env.finish(
        &round.implementer.unwrap(),
        &round.response.unwrap(),
        response,
    );
    env.steps_until(id, "the checks", |t, _| review_attempt(t).gate.is_some());
    let t = env.ticket(id);
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(checks_key(&t, n), code);
}

/// The pending `rerun` question answered `rerun` with `note`; then the
/// new attempt's first round started.
fn rerun_with(env: &mut Env, id: &str, note: Option<&str>) {
    env.steps_until(id, "the rerun question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    let before = env.ticket(id).attempts_of("review-code").count();
    let d = env
        .pending(id)
        .into_iter()
        .find(|d| d.name == "rerun")
        .unwrap();
    let now = env.tick();
    env.runner.decide(id, &d.id, "rerun", note, now).unwrap();
    env.steps_until(id, "the new attempt", |t, _| {
        t.attempts_of("review-code").count() == before + 1
    });
    round_started(env, id, 1);
}

/// Three rounds and a failure: lint raises two points, both disputed;
/// style withdraws one and keeps the other, disputed again; lint
/// exits 2 in round three. The rerun question is pending.
fn three_rounds_then_a_failure(env: &mut Env) -> String {
    with_three_rounds(env);
    three_rounds_failing(env)
}

/// `three_rounds_then_a_failure` on the pipeline the test already wrote.
fn three_rounds_failing(env: &mut Env) -> String {
    let id = at_review(env);
    lint_exits(
        env,
        &id,
        1,
        1,
        "- src/a.rs:3: unused import\n- src/b.rs:9: dead code\n",
    );
    style_says(env, &id, 1, "No findings.");
    fix_pass(
        env,
        &id,
        1,
        "fix00001",
        "- r1/lint-1: disputed used by a macro\n- r1/lint-2: disputed kept for the test\n",
        0,
    );
    round_started(env, &id, 2);
    lint_exits(env, &id, 2, 0, "");
    style_says(
        env,
        &id,
        2,
        "- withdraw r1/lint-1\n- keep r1/lint-2: still dead\n",
    );
    fix_pass(
        env,
        &id,
        2,
        "fix00002",
        "- r1/lint-2: disputed the test reads it\n",
        0,
    );
    round_started(env, &id, 3);
    lint_exits(env, &id, 3, 2, "");
    env.steps_until(&id, "the rerun question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    id
}

/// A rerun after a failed round carries the earlier attempt's last
/// gathered state: its settled points are named so they are not raised
/// again, its open points are open coming into round one under ids
/// qualified with the attempt, and the reviewers read only the change
/// since.
#[test]
fn a_rerun_review_carries_the_old_attempts_settled_and_open_points() {
    let mut env = Env::new();
    let id = three_rounds_then_a_failure(&mut env);
    let question = env
        .pending(&id)
        .into_iter()
        .find(|d| d.name == "rerun")
        .unwrap()
        .question;
    assert!(question.contains("start over"), "{question}");
    rerun_with(&mut env, &id, None);
    let t = env.ticket(&id);
    let a = review_attempt(&t);
    assert_eq!(a.n, 2);
    assert_eq!(a.carried_from, Some(("review-code".to_owned(), 1)));
    let prompt = last_prompt_of(&env, "style");
    assert!(
        prompt.contains("reviewed this branch at fix00001 through round 2"),
        "{prompt}"
    );
    assert!(prompt.contains("git diff fix00001 fix00002"), "{prompt}");
    let open_at = prompt.find("These are still open").unwrap();
    let settled = prompt
        .find("- a1/r1/lint-1: src/a.rs:3: unused import")
        .unwrap();
    let open = prompt
        .rfind("- a1/r1/lint-2: src/b.rs:9: dead code")
        .unwrap();
    assert!(settled < open_at && open_at < open, "{prompt}");
    style_says(&mut env, &id, 1, "- src/c.rs: a new point\n");
    lint_exits(&mut env, &id, 1, 0, "");
    env.steps_until(&id, "round one gathered", |t, _| {
        review_attempt(t).rounds[0].feedback.is_some()
    });
    let a = review_attempt(&env.ticket(&id));
    assert_eq!(a.rounds[0].open_points, 2);
    let feedback = std::fs::read_to_string(a.rounds[0].feedback.clone().unwrap()).unwrap();
    let still = feedback.find("## Still open from earlier rounds").unwrap();
    let kept = feedback
        .find("- a1/r1/lint-2: src/b.rs:9: dead code")
        .unwrap();
    assert!(still < kept, "{feedback}");
    assert!(
        feedback.contains("- r1/style-1 (style): src/c.rs: a new point"),
        "{feedback}"
    );
    assert!(!feedback.contains("a1/r1/lint-1"), "{feedback}");
}

/// A point the failed attempt answered as fixed, with no round after
/// to read the fix, is carried open with its fix to be checked rather
/// than settled.
#[test]
fn a_rerun_after_a_failed_fix_carries_the_fixed_point_open() {
    let mut env = Env::new();
    with_three_rounds(&env);
    let id = at_review(&mut env);
    lint_exits(&mut env, &id, 1, 1, "src/a.rs:3: unused import\n");
    style_says(&mut env, &id, 1, "No findings.");
    fix_pass(
        &mut env,
        &id,
        1,
        "fix00001",
        "- r1/lint-1: fixed removed\n",
        1,
    );
    rerun_with(&mut env, &id, None);
    let prompt = last_prompt_of(&env, "style");
    let open_at = prompt.find("These are still open").unwrap();
    let point = prompt
        .rfind("- a1/r1/lint-1: src/a.rs:3: unused import (answered fixed; check the fix)")
        .unwrap();
    assert!(open_at < point, "{prompt}");
    assert_eq!(prompt.matches("a1/r1/lint-1").count(), 1, "{prompt}");
}

/// A rerun whose note says "start over" reviews the whole branch: no
/// carry, and the note moves onto the attempt and reaches its first fix
/// pass.
#[test]
fn a_rerun_noted_start_over_reviews_the_whole_branch() {
    let mut env = Env::new();
    let id = three_rounds_then_a_failure(&mut env);
    rerun_with(&mut env, &id, Some("start over please"));
    let t = env.ticket(&id);
    let a = review_attempt(&t);
    assert_eq!(a.n, 2);
    assert_eq!(a.carried_from, None);
    assert_eq!(a.rework.as_deref(), Some("start over please"));
    assert!(t.rework.is_empty(), "{:?}", t.rework);
    let prompt = last_prompt_of(&env, "style");
    assert!(!prompt.contains("earlier attempt"), "{prompt}");
    style_says(&mut env, &id, 1, "- src/c.rs: a new point\n");
    lint_exits(&mut env, &id, 1, 0, "");
    env.steps_until(&id, "the implementer", |t, _| {
        review_attempt(t).rounds[0].implementer.is_some()
    });
    let prompt = last_prompt_of(&env, "implementer");
    assert!(prompt.ends_with("start over please"), "{prompt}");
}

/// A note given to an attempt that failed before any fixer saw it is
/// passed to the next attempt; one a fixer was given is not passed on,
/// even though an older attempt still holds it unspent.
#[test]
fn a_note_survives_a_review_that_fails_before_its_fix_pass() {
    let mut env = Env::new();
    with_three_rounds(&env);
    let id = at_review(&mut env);
    lint_exits(&mut env, &id, 1, 2, "");
    rerun_with(&mut env, &id, Some("rename tmp"));
    let t = env.ticket(&id);
    assert_eq!(review_attempt(&t).rework.as_deref(), Some("rename tmp"));
    assert!(t.rework.is_empty());
    lint_exits(&mut env, &id, 1, 2, "");
    rerun_with(&mut env, &id, None);
    let t = env.ticket(&id);
    assert_eq!(review_attempt(&t).n, 3);
    assert_eq!(review_attempt(&t).rework.as_deref(), Some("rename tmp"));
    lint_exits(&mut env, &id, 1, 1, "src/a.rs:3: unused import\n");
    style_says(&mut env, &id, 1, "No findings.");
    env.steps_until(&id, "the implementer", |t, _| {
        review_attempt(t).rounds[0].implementer.is_some()
    });
    let prompt = last_prompt_of(&env, "implementer");
    assert!(prompt.ends_with("rename tmp"), "{prompt}");
    fix_pass(
        &mut env,
        &id,
        1,
        "fix00001",
        "- r1/lint-1: fixed removed\n",
        1,
    );
    rerun_with(&mut env, &id, None);
    let a = review_attempt(&env.ticket(&id));
    assert_eq!(a.n, 4);
    assert_eq!(a.rework, None, "the attempt before it spent the note");
}

/// A note on an attempt that converges at round one leaves the ticket
/// with it, so nothing stays sent back.
#[test]
fn a_review_that_converges_at_round_one_does_not_keep_its_note() {
    let mut env = Env::new();
    with_three_rounds(&env);
    let id = at_review(&mut env);
    lint_exits(&mut env, &id, 1, 2, "");
    rerun_with(&mut env, &id, Some("look at the docs too"));
    lint_exits(&mut env, &id, 1, 0, "");
    style_says(&mut env, &id, 1, "No findings.");
    env.steps_until(&id, "the stage completing", |t, _| {
        review_attempt(t).state == AttemptState::Complete
    });
    let t = env.ticket(&id);
    assert!(t.rework.is_empty(), "{:?}", t.rework);
    assert_eq!(
        review_attempt(&t).rework.as_deref(),
        Some("look at the docs too")
    );
    env.step();
    assert_eq!(env.ticket(&id).stage, 6, "on to inspect");
}

/// The acceptance: a third round with only a style point converges at
/// the head the reviewers read, the point is left to the merge in the
/// round file and in the summary, and the checks run at that head.
#[test]
fn a_style_only_round_three_converges_and_leaves_the_point_to_the_merge() {
    let mut env = Env::new();
    with_three_rounds(&env);
    let id = at_review(&mut env);
    lint_exits(&mut env, &id, 1, 1, "src/a.rs:3: unused import\n");
    style_says(&mut env, &id, 1, "- style: rename tmp\n");
    fix_pass(
        &mut env,
        &id,
        1,
        "fix00001",
        "- r1/lint-1: fixed removed\n- r1/style-1: fixed renamed\n",
        0,
    );
    round_started(&mut env, &id, 2);
    lint_exits(&mut env, &id, 2, 1, "src/b.rs:9: dead code\n");
    style_says(&mut env, &id, 2, "- style: comment wording\n");
    fix_pass(
        &mut env,
        &id,
        2,
        "fix00002",
        "- r2/lint-1: fixed removed\n- r2/style-1: fixed reworded\n",
        0,
    );
    round_started(&mut env, &id, 3);
    lint_exits(&mut env, &id, 3, 0, "");
    style_says(&mut env, &id, 3, "- style: one more wording\n");
    env.steps_until(&id, "the final checks", |t, _| {
        review_attempt(t).gate.is_some()
    });
    let t = env.ticket(&id);
    let a = review_attempt(&t);
    assert_eq!(a.rounds[2].state, RoundState::Converged);
    assert_eq!(a.rounds[2].open_points, 0);
    assert!(
        a.rounds[2].implementer.is_none(),
        "no fix pass no one would read"
    );
    assert_eq!(a.gate.as_ref().unwrap().head, "fix00002");
    let feedback = std::fs::read_to_string(a.rounds[2].feedback.clone().unwrap()).unwrap();
    assert!(feedback.contains("No open point."), "{feedback}");
    let left = feedback.find("## Left to the merge").unwrap();
    let point = feedback
        .find("- r3/style-1 (style): style: one more wording")
        .unwrap();
    assert!(left < point, "{feedback}");
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(checks_key(&t, 3), 0);
    env.steps_until(&id, "the stage completing", |t, _| {
        review_attempt(t).state == AttemptState::Complete
    });
    let a = review_attempt(&env.ticket(&id));
    let summary = std::fs::read_to_string(&a.artifacts["summary"]).unwrap();
    assert!(
        summary.contains("Converged at `fix00002` in round 3."),
        "{summary}"
    );
    let left = summary.find("## Left to the merge").unwrap();
    let point = summary
        .find("- r3/style-1 (style): style: one more wording")
        .unwrap();
    let after = summary.find("## Found but not done").unwrap();
    assert!(left < point && point < after, "{summary}");
    // `show` points at the summary and at the last round's findings.
    let t = env.ticket(&id);
    let paths = dispatch::serve::ticket_paths(&t, env.runner.pipeline_of(&t).ok().as_ref());
    assert_eq!(paths.review_summary.as_ref(), Some(&a.artifacts["summary"]));
    assert_eq!(paths.round_file, a.rounds[2].feedback);
}

/// Two rounds of lint and style points, each fixed, then round three
/// started: the setup of the round-three tests.
fn two_fixed_rounds(env: &mut Env, id: &str) {
    lint_exits(env, id, 1, 1, "src/a.rs:3: unused import\n");
    style_says(env, id, 1, "- style: rename tmp\n");
    fix_pass(
        env,
        id,
        1,
        "fix00001",
        "- r1/lint-1: fixed removed\n- r1/style-1: fixed renamed\n",
        0,
    );
    round_started(env, id, 2);
    lint_exits(env, id, 2, 1, "src/b.rs:9: dead code\n");
    style_says(env, id, 2, "- style: comment wording\n");
    fix_pass(
        env,
        id,
        2,
        "fix00002",
        "- r2/lint-1: fixed removed\n- r2/style-1: fixed reworded\n",
        0,
    );
    round_started(env, id, 3);
    lint_exits(env, id, 3, 0, "");
}

/// Runs a converged round three's checks to the stage's end and returns
/// its summary.
fn summary_after_checks(env: &mut Env, id: &str) -> String {
    let t = env.ticket(id);
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(checks_key(&t, 3), 0);
    env.steps_until(id, "the stage completing", |t, _| {
        review_attempt(t).state == AttemptState::Complete
    });
    let a = review_attempt(&env.ticket(id));
    std::fs::read_to_string(&a.artifacts["summary"]).unwrap()
}

/// The acceptance: a style reviewer's line that every point left is
/// wording is a note under `## Reviewer notes`, not a point, so the
/// round leaves the one real point to the merge.
#[test]
fn a_wording_declaration_is_a_note_and_the_round_leaves_one_point() {
    let mut env = Env::new();
    with_three_rounds(&env);
    let id = at_review(&mut env);
    two_fixed_rounds(&mut env, &id);
    style_says(
        &mut env,
        &id,
        3,
        "- Every point left is wording; the round can close on it.\n- style: one more wording\n",
    );
    env.steps_until(&id, "the final checks", |t, _| {
        review_attempt(t).gate.is_some()
    });
    let t = env.ticket(&id);
    let a = review_attempt(&t);
    assert_eq!(a.rounds[2].state, RoundState::Converged);
    assert_eq!(a.rounds[2].open_points, 0);
    let feedback = std::fs::read_to_string(a.rounds[2].feedback.clone().unwrap()).unwrap();
    let left = feedback.find("## Left to the merge").unwrap();
    let point = feedback
        .find("- r3/style-1 (style): style: one more wording")
        .unwrap();
    let notes = feedback.find("## Reviewer notes").unwrap();
    let note = feedback
        .find("- style: Every point left is wording; the round can close on it.")
        .unwrap();
    assert!(left < point && point < notes && notes < note, "{feedback}");
    assert!(!feedback.contains("r3/style-2"), "{feedback}");
    let summary = summary_after_checks(&mut env, &id);
    let left = summary.find("## Left to the merge").unwrap();
    let after = summary.find("## Found but not done").unwrap();
    assert_eq!(
        summary[left..after].trim(),
        "## Left to the merge\n\n- r3/style-1 (style): style: one more wording",
        "{summary}"
    );
}

/// A round whose only style feedback is the declaration has nothing
/// left: it converges with the sentinel and the note.
#[test]
fn a_round_whose_only_style_feedback_is_the_declaration_converges_with_nothing_left() {
    let mut env = Env::new();
    with_three_rounds(&env);
    let id = at_review(&mut env);
    two_fixed_rounds(&mut env, &id);
    style_says(
        &mut env,
        &id,
        3,
        "Every point left is wording; the round can close on it.\n",
    );
    env.steps_until(&id, "the final checks", |t, _| {
        review_attempt(t).gate.is_some()
    });
    let t = env.ticket(&id);
    let a = review_attempt(&t);
    assert_eq!(a.rounds[2].state, RoundState::Converged);
    assert_eq!(a.rounds[2].open_points, 0);
    let feedback = std::fs::read_to_string(a.rounds[2].feedback.clone().unwrap()).unwrap();
    assert!(!feedback.contains("## Left to the merge"), "{feedback}");
    assert!(feedback.contains("## Reviewer notes"), "{feedback}");
    assert!(
        feedback.contains("- style: Every point left is wording; the round can close on it."),
        "{feedback}"
    );
    let summary = summary_after_checks(&mut env, &id);
    let left = summary.find("## Left to the merge").unwrap();
    let after = summary.find("## Found but not done").unwrap();
    assert_eq!(
        summary[left..after].trim(),
        "## Left to the merge\n\nNone.",
        "{summary}"
    );
}

/// Before `style_rounds`, style points go to the fixer like any other.
#[test]
fn style_points_in_round_one_go_to_the_fixer() {
    let mut env = Env::new();
    with_three_rounds(&env);
    let id = at_review(&mut env);
    lint_exits(&mut env, &id, 1, 0, "");
    style_says(&mut env, &id, 1, "- style: rename tmp\n");
    env.steps_until(&id, "the implementer", |t, _| {
        review_attempt(t).rounds[0].implementer.is_some()
    });
    assert_eq!(review_attempt(&env.ticket(&id)).rounds[0].open_points, 1);
}

/// `style_rounds` is read from the project's live pipeline file, not
/// the ticket's copy.
#[test]
fn style_rounds_is_read_from_the_live_pipeline() {
    let mut env = Env::new();
    with_three_rounds(&env);
    let id = at_review(&mut env);
    let path = env.data.pipeline(PROJECT);
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        text.replace("cap = 4\n", "cap = 4\nstyle_rounds = 1\n"),
    )
    .unwrap();
    lint_exits(&mut env, &id, 1, 0, "");
    style_says(&mut env, &id, 1, "- style: rename tmp\n");
    env.steps_until(&id, "round one gathered", |t, _| {
        review_attempt(t).rounds[0].feedback.is_some()
    });
    let a = review_attempt(&env.ticket(&id));
    assert_eq!(a.rounds[0].state, RoundState::Converged);
    assert!(a.rounds[0].implementer.is_none());
}

/// The plan's decisions section reaches the reviewers as settled; a
/// point that contests it holds nothing open and is listed as found but
/// not done, in the round file and in the summary.
#[test]
fn the_plans_decisions_reach_the_reviewer_as_settled() {
    let mut env = Env::new();
    let id = at_review_with_plan(
        &mut env,
        Some("## 2. Decisions\n- D1: keep X because Y\n## Tests\n- t1\n"),
    );
    let prompt = last_prompt_of(&env, "style");
    assert!(prompt.contains("settled these decisions"), "{prompt}");
    assert!(prompt.contains("D1: keep X because Y"), "{prompt}");
    assert!(prompt.contains("- decided:"), "{prompt}");
    assert!(prompt.contains("style: "), "{prompt}");
    assert!(!prompt.contains("- t1"), "{prompt}");
    style_says(&mut env, &id, 1, "- decided: D1 should be Z\n");
    lint_exits(&mut env, &id, 1, 0, "");
    env.steps_until(&id, "the stage completing", |t, _| {
        review_attempt(t).state == AttemptState::Complete
    });
    let a = review_attempt(&env.ticket(&id));
    assert_eq!(a.rounds[0].state, RoundState::Converged);
    let feedback = std::fs::read_to_string(a.rounds[0].feedback.clone().unwrap()).unwrap();
    let section = feedback.find("## Found but not done").unwrap();
    let point = feedback
        .find("- r1/style-1 (style): decided: D1 should be Z")
        .unwrap();
    assert!(section < point, "{feedback}");
    let summary = std::fs::read_to_string(&a.artifacts["summary"]).unwrap();
    let section = summary.find("## Found but not done").unwrap();
    let point = summary.find("decided: D1 should be Z").unwrap();
    assert!(section < point, "{summary}");
}

/// A plan with no decisions section is said to have none.
#[test]
fn a_plan_without_decisions_says_so() {
    let mut env = Env::new();
    at_review_with_plan(&mut env, Some("# Plan\n## Steps\n- s1\n"));
    let prompt = last_prompt_of(&env, "style");
    assert!(prompt.contains("lists no decisions"), "{prompt}");
}

/// A plan file that cannot be read says nothing about decisions.
#[test]
fn a_plan_that_cannot_be_read_adds_nothing() {
    let mut env = Env::new();
    at_review_with_plan(&mut env, None);
    let prompt = last_prompt_of(&env, "style");
    assert!(!prompt.contains("settled these decisions"), "{prompt}");
    assert!(!prompt.contains("lists no decisions"), "{prompt}");
}

/// The implementer running on a lane cut from `root0000` with the tree
/// at `head`; then the base moves to `main0002`, one commit ahead, and
/// the implementer stops. With `conflict` the rebase at `review-code`
/// stops on a conflict.
fn base_moves_before_review(env: &mut Env, head: &str, conflict: bool) -> (String, String) {
    env.with_review_stage("auto");
    let (id, implementer) = before_review(env);
    let tree = env.ticket(&id).lanes[0].worktree.clone();
    {
        let mut repo = env.repo.lock().unwrap();
        repo.heads.insert(tree.clone(), head.into());
        repo.bases
            .insert(env.data.repo_dir(PROJECT), "main0002".into());
        repo.behind.insert(tree.clone(), 1);
        if conflict {
            repo.rebase_conflicts.push(tree);
        }
    }
    (id, implementer)
}

/// The branch had commits when the base moved, so it was rebased: the
/// first review after it is told to check the rebase, and a rerun that
/// reads the same base is not told again.
#[test]
fn a_review_after_a_rebase_with_commits_checks_the_rebase() {
    let mut env = Env::new();
    let (id, implementer) = base_moves_before_review(&mut env, "impl0001", false);
    review_starts(&mut env, &id, &implementer);
    let t = env.ticket(&id);
    let moved = t.lanes[0].refreshed.clone().unwrap();
    assert_eq!(
        (moved.from.as_str(), moved.to.as_str()),
        ("root0000", "main0002")
    );
    assert!(moved.commits);
    assert_eq!(moved.notes, None);
    assert_eq!(review_attempt(&t).rounds[0].base, "main0002");
    let prompt = last_prompt_of(&env, "style");
    assert!(prompt.contains("git log root0000..main0002"), "{prompt}");
    assert!(
        prompt.contains("both sides of every conflicted hunk"),
        "{prompt}"
    );
    // Round one gathers findings; its fixer vanishes, so the attempt fails.
    lint_exits(&mut env, &id, 1, 1, "src/a.rs:3: unused import\n");
    style_says(&mut env, &id, 1, "No findings.");
    env.steps_until(&id, "the implementer", |t, _| {
        review_attempt(t).rounds[0].implementer.is_some()
    });
    let fixer = review_attempt(&env.ticket(&id)).rounds[0]
        .implementer
        .clone()
        .unwrap();
    env.sb().vanish(&fixer);
    rerun_with(&mut env, &id, None);
    let prompt = last_prompt_of(&env, "style");
    assert!(
        !prompt.contains("both sides of every conflicted hunk"),
        "{prompt}"
    );
}

/// A lane cut before its base was recorded reads the fork point as the
/// base it moved from, so a branch with commits still has its rebase
/// checked.
#[test]
fn a_rebase_of_a_lane_without_a_recorded_base_is_checked() {
    let mut env = Env::new();
    let (id, implementer) = base_moves_before_review(&mut env, "impl0001", false);
    let mut t = env.ticket(&id);
    t.lanes[0].base_sha = None;
    dispatch::store::write_ticket(&env.data.ticket_file(&id), &t).unwrap();
    env.repo
        .lock()
        .unwrap()
        .bases
        .insert(t.lanes[0].worktree.clone(), "root0000".into());
    review_starts(&mut env, &id, &implementer);
    let moved = env.ticket(&id).lanes[0].refreshed.clone().unwrap();
    assert_eq!(
        (moved.from.as_str(), moved.to.as_str()),
        ("root0000", "main0002")
    );
    assert!(moved.commits);
    let prompt = last_prompt_of(&env, "style");
    assert!(prompt.contains("git log root0000..main0002"), "{prompt}");
}

/// The fork point of a lane with no recorded base is read before its
/// rebaser runs; after it, the fork point would be the new base itself.
#[test]
fn a_rebaser_on_a_lane_without_a_recorded_base_keeps_its_fork_point() {
    let mut env = Env::new();
    let (id, implementer) = base_moves_before_review(&mut env, "impl0001", true);
    let mut t = env.ticket(&id);
    t.lanes[0].base_sha = None;
    dispatch::store::write_ticket(&env.data.ticket_file(&id), &t).unwrap();
    let tree = t.lanes[0].worktree.clone();
    env.repo
        .lock()
        .unwrap()
        .bases
        .insert(tree.clone(), "root0000".into());
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
        .insert(format!("{id}/implement/1"), 0);
    env.steps_until(&id, "the rebaser", |t, _| {
        t.attempts_of(dispatch::scheduler::REFRESH)
            .any(|a| a.session.is_some())
    });
    // The rebaser moved the branch: its fork point is now the new base.
    env.repo
        .lock()
        .unwrap()
        .bases
        .insert(tree, "main0002".into());
    {
        let mut repo = env.repo.lock().unwrap();
        repo.rebase_conflicts.clear();
        repo.behind.clear();
    }
    let rebase = env
        .ticket(&id)
        .attempts_of(dispatch::scheduler::REFRESH)
        .last()
        .unwrap()
        .clone();
    env.finish(
        &rebase.session.clone().unwrap(),
        &rebase.artifacts["notes"].clone(),
        "# rebased\nkept both sides",
    );
    round_started(&mut env, &id, 1);
    let moved = env.ticket(&id).lanes[0].refreshed.clone().unwrap();
    assert_eq!(
        (moved.from.as_str(), moved.to.as_str()),
        ("root0000", "main0002")
    );
    assert!(moved.commits);
    let prompt = last_prompt_of(&env, "style");
    assert!(prompt.contains("git log root0000..main0002"), "{prompt}");
}

/// A lane with no recorded base, not behind, whose fork point is the
/// base never moved: the base is recorded and nothing is checked.
#[test]
fn a_lane_without_a_recorded_base_that_never_moved_records_it() {
    let mut env = Env::new();
    let (id, implementer) = base_moves_before_review(&mut env, "impl0001", false);
    let mut t = env.ticket(&id);
    t.lanes[0].base_sha = None;
    dispatch::store::write_ticket(&env.data.ticket_file(&id), &t).unwrap();
    {
        let mut repo = env.repo.lock().unwrap();
        repo.behind.clear();
        repo.bases
            .insert(t.lanes[0].worktree.clone(), "main0002".into());
    }
    review_starts(&mut env, &id, &implementer);
    let t = env.ticket(&id);
    assert_eq!(t.lanes[0].refreshed, None);
    assert_eq!(t.lanes[0].base_sha.as_deref(), Some("main0002"));
    let prompt = last_prompt_of(&env, "style");
    assert!(
        !prompt.contains("both sides of every conflicted hunk"),
        "{prompt}"
    );
}

/// A lane with no recorded base, not behind, whose fork point cannot be
/// read has an unknown old base: its rebase is checked, naming no range.
#[test]
fn a_rebase_from_an_unknown_base_is_checked_without_a_range() {
    let mut env = Env::new();
    let (id, implementer) = base_moves_before_review(&mut env, "impl0001", false);
    let mut t = env.ticket(&id);
    t.lanes[0].base_sha = None;
    dispatch::store::write_ticket(&env.data.ticket_file(&id), &t).unwrap();
    {
        let mut repo = env.repo.lock().unwrap();
        repo.behind.clear();
        repo.no_merge_base.push(t.lanes[0].worktree.clone());
    }
    review_starts(&mut env, &id, &implementer);
    let moved = env.ticket(&id).lanes[0].refreshed.clone().unwrap();
    assert_eq!((moved.from.as_str(), moved.to.as_str()), ("", "main0002"));
    assert!(moved.commits);
    let prompt = last_prompt_of(&env, "style");
    assert!(
        prompt.contains("rebased onto main0002 from a base that was not recorded"),
        "{prompt}"
    );
    assert!(!prompt.contains("git log"), "{prompt}");
}

/// A lane whose fork point cannot be read while it is behind (history
/// unrelated to its base) and whose rebaser then replays it onto the
/// base is checked from an unknown base, with the rebaser's notes, even
/// though its fork point now reads as the base.
#[test]
fn a_rebaser_from_an_unknown_base_has_its_work_checked() {
    let mut env = Env::new();
    let (id, implementer) = base_moves_before_review(&mut env, "impl0001", true);
    let mut t = env.ticket(&id);
    t.lanes[0].base_sha = None;
    dispatch::store::write_ticket(&env.data.ticket_file(&id), &t).unwrap();
    let tree = t.lanes[0].worktree.clone();
    env.repo.lock().unwrap().no_merge_base.push(tree.clone());
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
        .insert(format!("{id}/implement/1"), 0);
    env.steps_until(&id, "the rebaser", |t, _| {
        t.attempts_of(dispatch::scheduler::REFRESH)
            .any(|a| a.session.is_some())
    });
    {
        let mut repo = env.repo.lock().unwrap();
        repo.no_merge_base.clear();
        repo.bases.insert(tree, "main0002".into());
        repo.rebase_conflicts.clear();
        repo.behind.clear();
    }
    let rebase = env
        .ticket(&id)
        .attempts_of(dispatch::scheduler::REFRESH)
        .last()
        .unwrap()
        .clone();
    let notes = rebase.artifacts["notes"].clone();
    env.finish(
        &rebase.session.clone().unwrap(),
        &notes,
        "# rebased\nkept both sides",
    );
    round_started(&mut env, &id, 1);
    let moved = env.ticket(&id).lanes[0].refreshed.clone().unwrap();
    assert_eq!((moved.from.as_str(), moved.to.as_str()), ("", "main0002"));
    assert!(moved.commits);
    assert_eq!(moved.notes.as_ref(), Some(&notes));
    let prompt = last_prompt_of(&env, "style");
    assert!(
        prompt.contains("rebased onto main0002 from a base that was not recorded"),
        "{prompt}"
    );
}

/// A branch with no commits of its own is moved, not rebased: no check.
#[test]
fn a_review_after_a_clean_move_has_no_rebase_check() {
    let mut env = Env::new();
    let (id, implementer) = base_moves_before_review(&mut env, "root0000", false);
    review_starts(&mut env, &id, &implementer);
    let t = env.ticket(&id);
    assert!(!t.lanes[0].refreshed.clone().unwrap().commits);
    let prompt = last_prompt_of(&env, "style");
    assert!(!prompt.contains("rebased onto it"), "{prompt}");
}

/// The rebaser resolved the conflict that brought the branch up to its
/// base at `review-code`; then stops: the review starts once the
/// branch is read again.
fn rebaser_resolves(env: &mut Env, id: &str, implementer: &str) {
    implementer_stops(env, id, implementer);
    env.steps_until(id, "the checks starting", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.gate.is_some())
    });
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(format!("{id}/implement/1"), 0);
    env.steps_until(id, "the rebaser", |t, _| {
        t.attempts_of(dispatch::scheduler::REFRESH)
            .any(|a| a.session.is_some())
    });
    let t = env.ticket(id);
    let rebase = t
        .attempts_of(dispatch::scheduler::REFRESH)
        .last()
        .unwrap()
        .clone();
    {
        let mut repo = env.repo.lock().unwrap();
        repo.rebase_conflicts.clear();
        repo.behind.clear();
    }
    env.finish(
        &rebase.session.clone().unwrap(),
        &rebase.artifacts["notes"].clone(),
        "# rebased\nkept both sides of the scheduler change",
    );
    round_started(env, id, 1);
}

/// A rebase a rebaser finished is checked with its notes attached; one
/// whose lane last moved before moves were timed gets the check but not
/// the notes, since the record cannot say which move the rebaser served.
#[test]
fn a_review_after_a_rebaser_attaches_its_notes() {
    let mut env = Env::new();
    let (id, implementer) = base_moves_before_review(&mut env, "impl0001", true);
    rebaser_resolves(&mut env, &id, &implementer);
    let t = env.ticket(&id);
    let notes = t
        .attempts_of(dispatch::scheduler::REFRESH)
        .last()
        .unwrap()
        .artifacts["notes"]
        .clone();
    let moved = t.lanes[0].refreshed.clone().unwrap();
    assert!(moved.commits);
    assert_eq!(moved.notes.as_ref(), Some(&notes));
    let prompt = last_prompt_of(&env, "style");
    assert!(
        prompt.contains("both sides of every conflicted hunk"),
        "{prompt}"
    );
    assert!(
        prompt.contains(&format!("The rebaser's notes are at {}.", notes.display())),
        "{prompt}"
    );

    let mut env = Env::new();
    let (id, implementer) = base_moves_before_review(&mut env, "impl0001", true);
    let mut t = env.ticket(&id);
    t.lanes[0].refreshed = Some(dispatch::ticket::Refreshed {
        from: "old00000".into(),
        to: "root0000".into(),
        commits: false,
        notes: None,
        at_ms: 0,
        conflict: None,
        after: None,
    });
    dispatch::store::write_ticket(&env.data.ticket_file(&id), &t).unwrap();
    rebaser_resolves(&mut env, &id, &implementer);
    let moved = env.ticket(&id).lanes[0].refreshed.clone().unwrap();
    assert!(moved.commits);
    assert_eq!(moved.notes, None);
    let prompt = last_prompt_of(&env, "style");
    assert!(
        prompt.contains("both sides of every conflicted hunk"),
        "{prompt}"
    );
    assert!(!prompt.contains("The rebaser's notes"), "{prompt}");
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

/// The repo lane lives on the tree's branch, so its own refresh moves
/// the tree; the tree is not brought up a second time beside it.
#[test]
fn a_tree_with_a_chosen_lane_on_its_branch_is_brought_up_only_by_the_lane() {
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
    assert!(t.lanes[0].refreshed.is_some());
    assert_eq!(t.tree_refreshed, None);
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

// --- the review of a conflict's resolution after the last code review

/// The code review pipeline with a `pr` agent stage after `review-code`
/// and a `resolver` as the policy's `resolution_reviewer`.
fn resolution_pipeline(worktrees: &std::path::Path) -> String {
    review_pipeline(worktrees, "auto")
        .replace(
            "[operators.rebaser]\n",
            "[operators.resolver]\nkind = \"claude\"\n\n[operators.rebaser]\n",
        )
        .replace(
            "[[stages]]\nname = \"inspect\"\n",
            "[[stages]]\nname = \"pr\"\noperator = \"implementer\"\ncontext = \"each\"\nwrites = [\"notes\"]\nprompt = \"Open a pull request for {branch}.\"\n\n[[stages]]\nname = \"inspect\"\n",
        )
        .replace("[policy]\n", "[policy]\nresolution_reviewer = \"resolver\"\n")
}

impl Env {
    fn with_resolution_stage(&self) {
        let worktrees = self.data.root.join("wt");
        std::fs::write(self.data.pipeline(PROJECT), resolution_pipeline(&worktrees)).unwrap();
    }

    /// The pipeline the test wrote, with no rebaser: a conflict is a
    /// question.
    fn without_rebaser(&self) {
        let path = self.data.pipeline(PROJECT);
        let text = std::fs::read_to_string(&path).unwrap();
        let line = "rebaser = \"rebaser\"\n";
        assert!(text.contains(line));
        std::fs::write(path, text.replace(line, "")).unwrap();
    }
}

/// A ticket whose `review-code` converged at `base0000` over
/// `root0000`, with main moved to `main0002` under it before `pr`
/// begins and the branch one behind. With `conflict` the rebase stops,
/// on `pick0001`; without, it leaves `rebased1`.
fn reviewed_before_main_moves(env: &mut Env, conflict: bool) -> (String, PathBuf) {
    if !std::fs::read_to_string(env.data.pipeline(PROJECT))
        .unwrap()
        .contains("resolution_reviewer")
    {
        env.with_resolution_stage();
    }
    let (id, implementer) = before_review(env);
    review_starts(env, &id, &implementer);
    let tree = env.ticket(&id).lanes[0].worktree.clone();
    {
        let mut repo = env.repo.lock().unwrap();
        repo.bases
            .insert(env.data.repo_dir(PROJECT), "main0002".into());
        repo.behind.insert(tree.clone(), 1);
        if conflict {
            repo.rebase_conflicts.push(tree.clone());
            repo.conflicting
                .insert(tree.clone(), vec!["pick0001".into()]);
        } else {
            repo.rebase_heads.insert(tree.clone(), "rebased1".into());
        }
    }
    lint_exits(env, &id, 1, 0, "all fine\n");
    style_says(env, &id, 1, "No findings.");
    env.steps_until(&id, "review-code complete", |t, _| {
        review_attempt(t).state == AttemptState::Complete
    });
    (id, tree)
}

/// `reviewed_before_main_moves` with a conflict, up to the rebaser
/// started at `pr`; its attempt.
fn rebaser_at_pr(env: &mut Env) -> (String, PathBuf, Attempt) {
    let (id, tree) = reviewed_before_main_moves(env, true);
    env.steps_until(&id, "the rebaser", |t, _| {
        t.attempts_of(REFRESH).any(|a| a.session.is_some())
    });
    let rebase = env.ticket(&id).attempts_of(REFRESH).last().unwrap().clone();
    (id, tree, rebase)
}

/// The rebaser resolves the conflict at `resolv01` and stops; then the
/// resolution reviewer starts.
fn resolution_review_started(env: &mut Env, id: &str, tree: &std::path::Path, rebase: &Attempt) {
    {
        let mut repo = env.repo.lock().unwrap();
        repo.rebase_conflicts.clear();
        repo.behind.clear();
        repo.heads.insert(tree.to_path_buf(), "resolv01".into());
    }
    env.finish(
        &rebase.session.clone().unwrap(),
        &rebase.artifacts["notes"].clone(),
        "# rebased\nkept both sides of the queue change",
    );
    resolver_started(env, id);
}

fn resolver_started(env: &mut Env, id: &str) {
    env.steps_until(id, "the resolution reviewer", |t, _| {
        t.attempts_of(RESOLUTION).last().is_some_and(|a| {
            a.rounds
                .last()
                .is_some_and(|r| r.reviewers.iter().all(|x| x.session.is_some()))
        })
    });
}

fn resolution_attempt(t: &Ticket) -> Attempt {
    t.attempts_of(RESOLUTION).last().unwrap().clone()
}

/// The resolution reviewer writes `text` and stops.
fn resolver_says(env: &mut Env, id: &str, text: &str) {
    let r = resolution_attempt(&env.ticket(id)).rounds[0].reviewers[0].clone();
    env.finish(&r.session.unwrap(), &r.feedback, text);
}

/// How many sessions were started under `name`.
fn launches_of(env: &Env, name: &str) -> usize {
    env.sb()
        .calls
        .iter()
        .filter(|r| matches!(&r.body, Body::SessionNew { name: n, .. } if n == name))
        .count()
}

fn pr_started(env: &mut Env, id: &str) {
    env.steps_until(id, "the pr agent", |t, _| {
        t.attempts_of("pr").next().is_some()
    });
}

/// Main moved after the review and the rebase at `pr` was clean: the
/// branch is brought up, nothing is reviewed, and `pr` runs.
#[test]
fn a_clean_refresh_at_pr_launches_no_reviewer() {
    let mut env = Env::new();
    let (id, _) = reviewed_before_main_moves(&mut env, false);
    pr_started(&mut env, &id);
    let t = env.ticket(&id);
    let moved = t.lanes[0].refreshed.clone().unwrap();
    assert_eq!(moved.to, "main0002");
    assert_eq!(moved.conflict, None);
    assert_eq!(moved.after.as_deref(), Some("rebased1"));
    assert_eq!(t.lanes[0].conflict, None);
    assert!(t.attempts_of(RESOLUTION).next().is_none());
    assert_eq!(launches_of(&env, "resolver"), 0);
    assert!(
        refreshed_text(&env, &id).ends_with(", rebased cleanly"),
        "{}",
        refreshed_text(&env, &id)
    );
}

/// The rebase at `pr` conflicts: the reviewed head and the commits that
/// conflict are recorded before the rebaser starts; once it resolves
/// the conflict, one reviewer reads only the resolution, and its clean
/// answer completes the pass with nothing run, then `pr` runs.
#[test]
fn a_conflicting_refresh_at_pr_is_reviewed_before_the_pr_opens() {
    let mut env = Env::new();
    let (id, tree, rebase) = rebaser_at_pr(&mut env);
    let t = env.ticket(&id);
    let conflict = t.lanes[0].conflict.clone().expect("recorded first");
    assert_eq!(
        (
            conflict.before.as_str(),
            conflict.from.as_str(),
            conflict.to.as_str()
        ),
        ("base0000", "root0000", "main0002")
    );
    assert_eq!(conflict.commits, vec!["pick0001".to_owned()]);
    assert_eq!(conflict.stage, 6, "seen at pr");
    assert!(t.attempts_of("pr").next().is_none());

    resolution_review_started(&mut env, &id, &tree, &rebase);
    let t = env.ticket(&id);
    assert_eq!(t.lanes[0].conflict, None, "moved to the bring-up");
    let moved = t.lanes[0].refreshed.clone().unwrap();
    assert_eq!(moved.conflict.map(|c| c.before), Some("base0000".into()));
    assert_eq!(moved.after.as_deref(), Some("resolv01"));
    let a = resolution_attempt(&t);
    assert_eq!((a.kind, a.context.as_str()), (AttemptKind::Review, "repo"));
    assert_eq!(a.carried_from, None);
    let round = &a.rounds[0];
    assert_eq!(
        (round.base.as_str(), round.head.as_str()),
        ("main0002", "resolv01")
    );
    let names: Vec<&str> = round.reviewers.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["resolver"]);
    assert_eq!(launches_of(&env, "resolver"), 1);
    assert!(t.attempts_of("pr").next().is_none(), "pr waits for it");
    let prompt = last_prompt_of(&env, "resolver");
    assert!(
        prompt.contains("git range-diff root0000..base0000 main0002..resolv01"),
        "{prompt}"
    );
    assert!(
        prompt.contains("was reviewed at base0000 over root0000, then rebased onto main0002"),
        "{prompt}"
    );
    assert!(prompt.contains("1 commit (pick0001)"), "{prompt}");
    assert!(
        prompt.contains(&format!(
            "The rebaser's notes are at {}.",
            rebase.artifacts["notes"].display()
        )),
        "{prompt}"
    );
    assert!(prompt.contains("No findings."), "{prompt}");

    let checks_before = env.repo.lock().unwrap().checks.len();
    resolver_says(&mut env, &id, "No findings.");
    pr_started(&mut env, &id);
    let a = resolution_attempt(&env.ticket(&id));
    assert_eq!(a.state, AttemptState::Complete);
    assert_eq!(a.head.as_deref(), Some("resolv01"));
    assert_eq!(a.rounds.len(), 1);
    assert_eq!(a.rounds[0].state, RoundState::Converged);
    assert!(a.rounds[0].implementer.is_none());
    assert!(a.gate.is_none(), "nothing changed, so no checks ran");
    assert_eq!(env.repo.lock().unwrap().checks.len(), checks_before);
    assert!(a.artifacts.contains_key("summary"));
}

/// The pass up to its findings: one point, and the question.
fn resolution_findings(env: &mut Env) -> (String, PathBuf) {
    let (id, tree, rebase) = rebaser_at_pr(env);
    resolution_review_started(env, &id, &tree, &rebase);
    resolver_says(
        env,
        &id,
        "- src/queue.rs: the base's guard on an empty queue was dropped\n",
    );
    env.steps_until(&id, "the resolution question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == RESOLUTION)
    });
    (id, tree)
}

fn decide_resolution(env: &mut Env, id: &str, answer: &str) {
    let d = env
        .pending(id)
        .into_iter()
        .find(|d| d.name == RESOLUTION)
        .unwrap();
    let now = env.tick();
    env.runner.decide(id, &d.id, answer, None, now).unwrap();
}

/// The resolution fixer commits `fix00001` and answers; the checks run
/// at it and pass.
fn resolution_fixed(env: &mut Env, id: &str, tree: &std::path::Path) {
    env.steps_until(id, "the fixer", |t, _| {
        resolution_attempt(t).rounds[0].implementer.is_some()
    });
    let round = resolution_attempt(&env.ticket(id)).rounds[0].clone();
    env.repo
        .lock()
        .unwrap()
        .heads
        .insert(tree.to_path_buf(), "fix00001".into());
    env.finish(
        &round.implementer.unwrap(),
        &round.response.unwrap(),
        "- r1/resolver-1: fixed restored the guard\n",
    );
    env.steps_until(id, "the checks", |t, _| {
        resolution_attempt(t).gate.is_some()
    });
    let t = env.ticket(id);
    let key = format!("{id}/{RESOLUTION}/{}/r1/checks", resolution_attempt(&t).n);
    env.repo.lock().unwrap().check_exits.insert(key, 0);
}

/// A point is a question of its own even with `review-code` on auto: a
/// fix is one fixer and the checks, with no second reviewer, and accept
/// completes at the reviewed head with nothing run.
#[test]
fn a_resolution_pass_with_points_asks_fix_accept_park() {
    let mut env = Env::new();
    let (id, tree) = resolution_findings(&mut env);
    let pending = env.pending(&id);
    assert_eq!(pending.len(), 1, "{pending:?}");
    let d = &pending[0];
    assert_eq!(d.options, vec!["fix", "accept", "park"]);
    assert_eq!(d.attempt, Some((RESOLUTION.to_owned(), 1)));
    assert!(d.question.contains("1 point(s)"), "{}", d.question);
    decide_resolution(&mut env, &id, "fix");
    resolution_fixed(&mut env, &id, &tree);
    pr_started(&mut env, &id);
    let a = resolution_attempt(&env.ticket(&id));
    assert_eq!(a.state, AttemptState::Complete);
    assert_eq!(a.head.as_deref(), Some("fix00001"));
    assert_eq!(a.rounds.len(), 1, "no second round");
    assert_eq!(a.rounds[0].state, RoundState::Fixed);
    assert_eq!(a.gate.as_ref().and_then(|g| g.exit), Some(0));
    assert_eq!(launches_of(&env, "resolver"), 1);
    let summary = std::fs::read_to_string(&a.artifacts["summary"]).unwrap();
    assert!(summary.contains("Fixed at `fix00001`"), "{summary}");

    let mut env = Env::new();
    let (id, _) = resolution_findings(&mut env);
    let checks_before = env.repo.lock().unwrap().checks.len();
    decide_resolution(&mut env, &id, "accept");
    pr_started(&mut env, &id);
    let a = resolution_attempt(&env.ticket(&id));
    assert_eq!(a.state, AttemptState::Complete);
    assert_eq!(a.head.as_deref(), Some("resolv01"));
    assert_eq!(a.rounds[0].state, RoundState::Accepted);
    assert!(a.gate.is_none());
    assert_eq!(env.repo.lock().unwrap().checks.len(), checks_before);
}

/// A conflict brought up at `implement` is read by `review-code`'s
/// first round, and the bring-up it left on the lane, still there at
/// `pr` with the base unmoved, starts no second review.
#[test]
fn a_conflict_before_review_code_gets_no_resolution_pass() {
    let mut env = Env::new();
    env.with_resolution_stage();
    env.repo
        .lock()
        .unwrap()
        .bases
        .insert(env.data.repo_dir(PROJECT), "root0000".into());
    let id = at_finalize(&mut env);
    let tree = env.ticket(&id).lanes[0].worktree.clone();
    {
        let mut repo = env.repo.lock().unwrap();
        repo.bases
            .insert(env.data.repo_dir(PROJECT), "main0002".into());
        repo.behind.insert(tree.clone(), 1);
        repo.rebase_conflicts.push(tree.clone());
    }
    let decision = env.pending(&id)[0].id.clone();
    let now = env.tick();
    env.runner
        .decide(&id, &decision, "finalize", None, now)
        .unwrap();
    env.steps_until(&id, "the rebaser", |t, _| {
        t.attempts_of(REFRESH).any(|a| a.session.is_some())
    });
    let rebase = env.ticket(&id).attempts_of(REFRESH).last().unwrap().clone();
    {
        let mut repo = env.repo.lock().unwrap();
        repo.rebase_conflicts.clear();
        repo.behind.clear();
        repo.heads.insert(tree.clone(), "resolv01".into());
    }
    env.finish(
        &rebase.session.clone().unwrap(),
        &rebase.artifacts["notes"].clone(),
        "# rebased",
    );
    env.steps_until(&id, "the implementer", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(Attempt::is_open)
    });
    let t = env.ticket(&id);
    let moved = t.lanes[0].refreshed.clone().unwrap();
    assert_eq!(moved.conflict.map(|c| c.stage), Some(4), "at implement");
    review_starts(&mut env, &id, &session_of(&t, "implement"));
    let prompt = last_prompt_of(&env, "style");
    assert!(
        prompt.contains("both sides of every conflicted hunk"),
        "{prompt}"
    );
    lint_exits(&mut env, &id, 1, 0, "all fine\n");
    style_says(&mut env, &id, 1, "No findings.");
    pr_started(&mut env, &id);
    let t = env.ticket(&id);
    assert!(t.lanes[0].refreshed.as_ref().unwrap().conflict.is_some());
    assert!(t.attempts_of(RESOLUTION).next().is_none());
    assert_eq!(launches_of(&env, "resolver"), 0);
}

/// A branch already on the remote keeps its commits when the fix
/// completes the pass: the rewrite is recorded as skipped, and `pr`
/// runs.
#[test]
fn a_resolution_fix_on_a_published_branch_keeps_its_commits() {
    let mut env = Env::new();
    env.with_resolution_stage();
    with_commits(&env, "fold");
    let (id, tree) = resolution_findings(&mut env);
    let mut t = env.ticket(&id);
    // An earlier refresh pushed the branch.
    t.lanes[0].pushed = Some(PushedHead {
        head: "base0000".into(),
        at_ms: 1_000,
    });
    env.runner.save_ticket(&mut t, env.now).unwrap();
    decide_resolution(&mut env, &id, "fix");
    resolution_fixed(&mut env, &id, &tree);
    pr_started(&mut env, &id);
    let a = resolution_attempt(&env.ticket(&id));
    assert_eq!(a.state, AttemptState::Complete);
    assert_eq!(a.head.as_deref(), Some("fix00001"));
    let r = a.rewrite.clone().unwrap();
    assert_eq!(r.skipped.as_deref(), Some("the branch is published"));
    assert!(env.repo.lock().unwrap().replayed.is_empty());
}

/// A reviewer that stops without writing fails the pass: the rerun
/// question says what a rerun does, with no "start over", and nothing
/// is launched while it waits; a rerun reviews afresh.
#[test]
fn a_failed_resolution_pass_asks_rerun_without_carry_wording() {
    let mut env = Env::new();
    let (id, tree, rebase) = rebaser_at_pr(&mut env);
    resolution_review_started(&mut env, &id, &tree, &rebase);
    let r = resolution_attempt(&env.ticket(&id)).rounds[0].reviewers[0].clone();
    let now = env.now;
    env.sb().stop(&r.session.unwrap(), now);
    env.idle_past_grace();
    env.steps_until(&id, "the rerun question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    let d = env
        .pending(&id)
        .into_iter()
        .find(|d| d.name == "rerun")
        .unwrap();
    assert_eq!(d.attempt, Some((RESOLUTION.to_owned(), 1)));
    assert!(
        d.question
            .contains("A rerun reviews the same resolution again with a fresh reviewer."),
        "{}",
        d.question
    );
    assert!(!d.question.contains("start over"), "{}", d.question);
    for _ in 0..3 {
        env.step();
    }
    assert_eq!(launches_of(&env, "resolver"), 1);
    assert!(env.ticket(&id).attempts_of("pr").next().is_none());
    let now = env.tick();
    env.runner.decide(&id, &d.id, "rerun", None, now).unwrap();
    env.steps_until(&id, "the second pass", |t, _| {
        t.attempts_of(RESOLUTION).count() == 2
    });
    resolver_started(&mut env, &id);
    assert_eq!(launches_of(&env, "resolver"), 2);
    assert_eq!(resolution_attempt(&env.ticket(&id)).carried_from, None);
}

/// With no rebaser the conflict is a question; a hand rebase and
/// `recheck` bring the branch up, and the same pass reviews it, with no
/// rebaser's notes.
#[test]
fn a_hand_rebase_after_the_refresh_question_is_reviewed() {
    let mut env = Env::new();
    env.with_resolution_stage();
    env.without_rebaser();
    let (id, tree) = reviewed_before_main_moves(&mut env, true);
    env.steps_until(&id, "the refresh question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == REFRESH)
    });
    assert!(env.ticket(&id).lanes[0].conflict.is_some());
    {
        let mut repo = env.repo.lock().unwrap();
        repo.rebase_conflicts.clear();
        repo.behind.clear();
        repo.heads.insert(tree, "hand0001".into());
    }
    let d = env.pending(&id)[0].clone();
    let now = env.tick();
    env.runner.decide(&id, &d.id, "recheck", None, now).unwrap();
    resolver_started(&mut env, &id);
    let prompt = last_prompt_of(&env, "resolver");
    assert!(
        prompt.contains("git range-diff root0000..base0000 main0002..hand0001"),
        "{prompt}"
    );
    assert!(!prompt.contains("rebaser's notes"), "{prompt}");
    let text = refreshed_text(&env, &id);
    assert!(
        text.ends_with(", rebased by hand (adopted), conflicts in 1 commit"),
        "{text}"
    );
}

/// A rebaser that leaves the branch alone, then a base that moves again
/// and rebases cleanly: nothing resolved the conflict, so there is
/// nothing to review, and the lane's record of it is gone.
#[test]
fn a_clean_rebase_after_an_aborted_rebaser_drops_the_conflict() {
    let mut env = Env::new();
    let (id, tree, rebase) = rebaser_at_pr(&mut env);
    {
        let mut repo = env.repo.lock().unwrap();
        repo.bases
            .insert(env.data.repo_dir(PROJECT), "main0003".into());
        repo.rebase_conflicts.clear();
        repo.rebase_heads.insert(tree, "rebased3".into());
    }
    env.finish(
        &rebase.session.clone().unwrap(),
        &rebase.artifacts["notes"].clone(),
        "# left alone\nthe intent of the conflict was unclear",
    );
    pr_started(&mut env, &id);
    let t = env.ticket(&id);
    assert_eq!(t.lanes[0].conflict, None);
    let moved = t.lanes[0].refreshed.clone().unwrap();
    assert_eq!(moved.to, "main0003");
    assert_eq!(moved.conflict, None);
    assert_eq!(moved.after.as_deref(), Some("rebased3"));
    assert!(t.attempts_of(RESOLUTION).next().is_none());
}

/// A conflict at `review-code` parked on instead of resolved: the
/// resume reads the lane again and asks again, and once the owner
/// leaves work in the tree and answers `recheck`, the stage goes ahead
/// on the branch as it stands and drops the conflict, so a clean rebase
/// at `pr` has resolved nothing and starts no reviewer.
#[test]
fn a_conflict_parked_on_is_dropped_when_the_stage_goes_ahead() {
    let mut env = Env::new();
    env.with_resolution_stage();
    env.without_rebaser();
    let (id, implementer) = before_review(&mut env);
    let tree = env.ticket(&id).lanes[0].worktree.clone();
    {
        let mut repo = env.repo.lock().unwrap();
        repo.bases
            .insert(env.data.repo_dir(PROJECT), "main0002".into());
        repo.behind.insert(tree.clone(), 1);
        repo.rebase_conflicts.push(tree.clone());
    }
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
        .insert(format!("{id}/implement/1"), 0);
    env.steps_until(&id, "the refresh question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == REFRESH)
    });
    assert!(env.ticket(&id).lanes[0].conflict.is_some());
    let d = env.pending(&id)[0].clone();
    let now = env.tick();
    env.runner.decide(&id, &d.id, "park", None, now).unwrap();
    env.steps_until(&id, "parked", |t, _| {
        matches!(t.state, TicketState::Parked { .. })
    });
    let now = env.tick();
    env.runner.resume(&id, now).unwrap();
    env.steps_until(&id, "the refresh question again", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|x| x.name == REFRESH && x.id != d.id)
    });
    assert!(env.ticket(&id).attempts_of("review-code").next().is_none());
    env.repo.lock().unwrap().dirty.push(tree.clone());
    let again = env.pending(&id)[0].clone();
    let now = env.tick();
    env.runner
        .decide(&id, &again.id, "recheck", None, now)
        .unwrap();
    // The stage goes ahead on the tree as it stands, which the review
    // round refuses while it is dirty; the owner cleans it and reruns.
    env.steps_until(&id, "the not-clean rerun question", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|x| x.name == "rerun" && x.stage == "review-code")
    });
    assert_eq!(env.ticket(&id).lanes[0].conflict, None);
    env.repo.lock().unwrap().dirty.clear();
    let rerun = env.pending(&id)[0].clone();
    let now = env.tick();
    env.runner
        .decide(&id, &rerun.id, "rerun", None, now)
        .unwrap();
    env.steps_until(&id, "the review round", |t, _| {
        t.attempts_of("review-code").last().is_some_and(|a| {
            a.rounds.last().is_some_and(|r| {
                r.reviewers
                    .iter()
                    .all(|x| x.session.is_some() || x.launched)
            })
        })
    });
    assert_eq!(env.ticket(&id).lanes[0].conflict, None);
    {
        let mut repo = env.repo.lock().unwrap();
        repo.bases
            .insert(env.data.repo_dir(PROJECT), "main0003".into());
        repo.rebase_conflicts.clear();
        repo.rebase_heads.insert(tree, "rebased3".into());
    }
    lint_exits(&mut env, &id, 1, 0, "all fine\n");
    style_says(&mut env, &id, 1, "No findings.");
    pr_started(&mut env, &id);
    let t = env.ticket(&id);
    let moved = t.lanes[0].refreshed.clone().unwrap();
    assert_eq!(moved.to, "main0003");
    assert_eq!(moved.conflict, None);
    assert!(t.attempts_of(RESOLUTION).next().is_none());
    assert_eq!(launches_of(&env, "resolver"), 0);
}

/// A park while the rebaser at `pr` runs, after it rewrote the branch:
/// the resumed stage reads the lane again, so the conflict reaches the
/// bring-up and is reviewed before the PR opens.
#[test]
fn a_park_mid_rebaser_keeps_the_conflict_for_review() {
    let mut env = Env::new();
    let (id, tree, _) = rebaser_at_pr(&mut env);
    {
        let mut repo = env.repo.lock().unwrap();
        repo.rebase_conflicts.clear();
        repo.behind.clear();
        repo.heads.insert(tree, "resolv01".into());
    }
    park_by_hand(&mut env, &id);
    env.steps_until(&id, "parked", |t, _| {
        matches!(t.state, TicketState::Parked { .. })
    });
    let now = env.tick();
    env.runner.resume(&id, now).unwrap();
    resolver_started(&mut env, &id);
    let t = env.ticket(&id);
    let moved = t.lanes[0].refreshed.clone().unwrap();
    assert_eq!(moved.conflict.map(|c| c.before), Some("base0000".into()));
    assert_eq!(moved.after.as_deref(), Some("resolv01"));
    assert!(t.attempts_of("pr").next().is_none(), "pr waits for it");
    let text = refreshed_text(&env, &id);
    assert!(
        text.ends_with(", rebased after the rebaser stopped, conflicts in 1 commit"),
        "{text}"
    );
}

/// A record from before a rebaser's rerun question held the stage: one
/// left pending about a superseded rebaser, past a stage already
/// refreshed, does not freeze it; the resolution and `pr` go on.
#[test]
fn a_stale_rebaser_rerun_does_not_hold_a_refreshed_stage() {
    let mut env = Env::new();
    let (id, tree, rebase) = rebaser_at_pr(&mut env);
    resolution_review_started(&mut env, &id, &tree, &rebase);
    let mut t = env.ticket(&id);
    assert_eq!(t.refreshed_stage, Some(t.stage));
    let mut stale = t.decisions.last().unwrap().clone();
    stale.id = "stale-rerun".into();
    stale.stage = REFRESH.into();
    stale.name = "rerun".into();
    stale.attempt = Some((REFRESH.into(), rebase.n));
    stale.state = DecisionState::Pending;
    t.decisions.push(stale);
    let now = env.tick();
    env.runner.save_ticket(&mut t, now).unwrap();
    resolver_says(&mut env, &id, "No findings.");
    pr_started(&mut env, &id);
    assert!(
        env.pending(&id).iter().any(|d| d.id == "stale-rerun"),
        "nothing withdraws it"
    );
}

/// A restart while the reviewer runs finds it on the record and polls
/// it; nothing is launched again.
#[test]
fn a_restart_mid_resolution_reattaches_the_reviewer() {
    let mut env = Env::new();
    let (id, tree, rebase) = rebaser_at_pr(&mut env);
    resolution_review_started(&mut env, &id, &tree, &rebase);
    env.restart();
    for _ in 0..3 {
        env.step();
    }
    assert_eq!(launches_of(&env, "resolver"), 1);
    assert!(resolution_attempt(&env.ticket(&id)).is_open());
    resolver_says(&mut env, &id, "No findings.");
    pr_started(&mut env, &id);
    assert_eq!(
        resolution_attempt(&env.ticket(&id)).state,
        AttemptState::Complete
    );
    assert_eq!(launches_of(&env, "resolver"), 1);
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
    let kinds = events_of(&env.data, &id);
    assert_in_order(&kinds, &["refreshed", "pushed"]);
    assert_eq!(
        kinds.iter().filter(|k| *k == "pushed").count(),
        1,
        "{kinds:?}"
    );
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
    let text = refreshed_text(&env, &id);
    assert!(
        text.contains(", rebased by the rebaser, conflicts in "),
        "{text}"
    );
}

/// Two refreshes in one lane, each after main moved, with an agent
/// stage between them that records no head: the second lease is on the
/// head the first refresh pushed, not the older one the implement
/// checks recorded, so `ready` reads the PR with no question.
#[test]
fn a_second_refresh_leases_on_the_head_the_first_one_pushed() {
    let mut env = Env::new();
    let path = env.data.pipeline(PROJECT);
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        text.replace(
            "[[stages]]\nname = \"ready\"",
            "[[stages]]\nname = \"polish\"\noperator = \"implementer\"\ncontext = \"each\"\nwrites = [\"notes\"]\nprompt = \"Polish {branch}.\"\n\n[[stages]]\nname = \"ready\"",
        ),
    )
    .unwrap();
    let (id, tree) = main_moved_before_ready(&mut env, "rebased2", 1);
    env.inspect(&id, "proceed", None);
    env.steps_until(&id, "the polisher", |t, _| {
        t.attempts_of("polish").last().is_some_and(Attempt::is_open)
    });
    let t = env.ticket(&id);
    let branch = t.lanes[0].branch.clone();
    assert_eq!(
        env.repo.lock().unwrap().pushed,
        vec![(
            tree.clone(),
            "origin".into(),
            branch.clone(),
            "base0000".into()
        )]
    );
    let clone = env.runner.data.repo_dir(PROJECT);
    {
        let mut repo = env.repo.lock().unwrap();
        repo.bases.insert(clone, "main0003".into());
        repo.behind.insert(tree.clone(), 1);
        repo.rebase_heads.insert(tree.clone(), "rebased2".into());
    }
    env.finish(
        &session_of(&t, "polish"),
        &artifact_of(&t, "polish", "notes"),
        "# polished\nnothing to commit",
    );
    env.steps_until(&id, "the ready attempt done", |t, _| {
        t.attempts_of("ready").last().is_some_and(|a| !a.is_open())
    });
    let t = env.ticket(&id);
    assert_eq!(
        env.repo.lock().unwrap().pushed,
        vec![
            (
                tree.clone(),
                "origin".into(),
                branch.clone(),
                "base0000".into()
            ),
            (tree, "origin".into(), branch, "rebased1".into()),
        ]
    );
    assert_eq!(t.lanes[0].base_sha.as_deref(), Some("main0003"));
    assert!(t.pending_decisions().is_empty(), "{t:#?}");
    assert_eq!(
        t.attempts_of("ready").last().unwrap().state,
        AttemptState::Complete
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

/// `rebaser_at_pr`, then the rebaser stops with the tree mid-rebase at
/// `replay01` and no notes, and is failed for it; the ticket at its
/// rerun question, with the lane as it was before the rebaser.
fn rebaser_stopped_mid_rebase(env: &mut Env) -> (String, PathBuf, LaneRecord) {
    let (id, tree, rebase) = rebaser_at_pr(env);
    let lane = env.ticket(&id).lanes[0].clone();
    assert!(lane.conflict.is_some());
    {
        let mut repo = env.repo.lock().unwrap();
        repo.rebase_conflicts.clear();
        repo.behind.clear();
        repo.mid_rebase.push(tree.clone());
        repo.heads.insert(tree.clone(), "replay01".into());
    }
    let t = env.ticket(&id);
    assert_ne!(t.refreshed_stage, Some(t.stage), "held while it runs");
    let now = env.now;
    env.sb().stop(rebase.session.as_deref().unwrap(), now);
    env.idle_past_grace();
    env.steps_until(&id, "the rebaser's rerun question", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == "rerun" && d.stage == REFRESH)
    });
    (id, tree, lane)
}

/// The lane untouched and no `pr` attempt after a few passes.
fn held_at_pr(env: &mut Env, id: &str, lane: &LaneRecord, pushes: usize) {
    for _ in 0..4 {
        env.step();
    }
    let t = env.ticket(id);
    assert!(t.attempts_of("pr").next().is_none(), "{t:#?}");
    assert_ne!(t.refreshed_stage, Some(t.stage));
    assert_eq!(t.lanes[0].base_sha, lane.base_sha);
    assert_eq!(t.lanes[0].refreshed, lane.refreshed);
    assert_eq!(t.lanes[0].conflict, lane.conflict);
    assert_eq!(env.repo.lock().unwrap().pushed.len(), pushes);
}

/// A rebaser that stops with the rebase stopped part way: its rerun
/// question says so and holds `pr`, and the lane is not read as
/// brought up from the detached head; a `rerun` while the tree is
/// still mid-rebase asks the `refresh` question instead of launching.
#[test]
fn a_rebaser_that_stops_mid_rebase_holds_the_stage() {
    let mut env = Env::new();
    let (id, _, lane) = rebaser_stopped_mid_rebase(&mut env);
    let pushes = env.repo.lock().unwrap().pushed.len();
    let rerun = env.pending(&id)[0].clone();
    assert_eq!(env.pending(&id).len(), 1);
    assert!(
        rerun.question.contains("mid-rebase at replay01"),
        "{}",
        rerun.question
    );
    held_at_pr(&mut env, &id, &lane, pushes);
    let now = env.tick();
    env.runner
        .decide(&id, &rerun.id, "rerun", None, now)
        .unwrap();
    env.steps_until(&id, "the mid-rebase question", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == REFRESH && d.question.contains("mid-rebase"))
    });
    held_at_pr(&mut env, &id, &lane, pushes);
    assert_eq!(env.ticket(&id).attempts_of(REFRESH).count(), 1);
}

/// The owner finishes the stopped rebase by hand and answers `rerun`:
/// the lane is read again and brought up from the branch, and the
/// resolution is reviewed; no second rebaser.
#[test]
fn a_hand_finished_rebase_after_a_stopped_rebaser_is_brought_up() {
    let mut env = Env::new();
    let (id, tree, _) = rebaser_stopped_mid_rebase(&mut env);
    {
        let mut repo = env.repo.lock().unwrap();
        repo.mid_rebase.clear();
        repo.heads.insert(tree, "resolv01".into());
    }
    let rerun = env.pending(&id)[0].clone();
    let now = env.tick();
    env.runner
        .decide(&id, &rerun.id, "rerun", None, now)
        .unwrap();
    resolver_started(&mut env, &id);
    let t = env.ticket(&id);
    assert_eq!(t.lanes[0].base_sha.as_deref(), Some("main0002"));
    let moved = t.lanes[0].refreshed.clone().unwrap();
    assert_eq!(moved.after.as_deref(), Some("resolv01"));
    assert!(moved.conflict.is_some());
    assert_eq!(t.attempts_of(REFRESH).count(), 1);
    let text = refreshed_text(&env, &id);
    assert!(
        text.contains(", rebased by hand (adopted), conflicts in "),
        "{text}"
    );
}

/// A worktree whose `HEAD` is detached off its branch is never read as
/// the branch: the stage waits on a `refresh` question and nothing is
/// rebased or recorded.
#[test]
fn a_detached_worktree_is_held_not_brought_up() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let tree = env.ticket(&id).lanes[0].worktree.clone();
    let clone = env.runner.data.repo_dir(PROJECT);
    {
        let mut repo = env.repo.lock().unwrap();
        repo.bases.insert(clone, "main0002".into());
        repo.detached.push(tree);
    }
    let decision = env.pending(&id)[0].id.clone();
    let now = env.tick();
    env.runner
        .decide(&id, &decision, "finalize", None, now)
        .unwrap();
    env.steps_until(&id, "the refresh question", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == REFRESH && d.question.contains("not on"))
    });
    for _ in 0..4 {
        env.step();
    }
    let t = env.ticket(&id);
    assert!(t.attempts_of("implement").next().is_none(), "{t:#?}");
    assert!(env.repo.lock().unwrap().rebased.is_empty());
    assert_eq!(t.lanes[0].base_sha.as_deref(), Some("base0000"));
}

/// Parks `id` by answering `park` to `decision`, then resumes it.
fn park_and_resume(env: &mut Env, id: &str, decision: &str) {
    let now = env.tick();
    env.runner.decide(id, decision, "park", None, now).unwrap();
    env.steps_until(id, "parked", |t, _| {
        matches!(t.state, TicketState::Parked { .. })
    });
    let now = env.tick();
    env.runner.resume(id, now).unwrap();
}

/// After a park and resume the held lane is read again: still
/// mid-rebase, it asks again and `pr` still waits.
fn asks_again_after_resume(
    env: &mut Env,
    id: &str,
    before: &str,
    lane: &LaneRecord,
    pushes: usize,
) {
    env.steps_until(id, "the mid-rebase question again", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == REFRESH && d.id != before && d.question.contains("mid-rebase"))
    });
    held_at_pr(env, id, lane, pushes);
    assert_eq!(env.ticket(id).attempts_of(REFRESH).count(), 1);
}

/// A park on the mid-rebase question withdraws it; the resume reads
/// the lane again rather than running `pr` in the half-rebased tree.
#[test]
fn a_park_on_the_mid_rebase_question_asks_again_on_resume() {
    let mut env = Env::new();
    let (id, _, lane) = rebaser_stopped_mid_rebase(&mut env);
    let pushes = env.repo.lock().unwrap().pushed.len();
    let rerun = env.pending(&id)[0].clone();
    let now = env.tick();
    env.runner
        .decide(&id, &rerun.id, "rerun", None, now)
        .unwrap();
    env.steps_until(&id, "the mid-rebase question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == REFRESH)
    });
    let asked = env.pending(&id)[0].clone();
    park_and_resume(&mut env, &id, &asked.id);
    asks_again_after_resume(&mut env, &id, &asked.id, &lane, pushes);
}

/// A park on a stopped rebaser's rerun question: the resume asks the
/// mid-rebase question rather than running `pr`.
#[test]
fn a_park_on_a_stopped_rebasers_rerun_asks_again_on_resume() {
    let mut env = Env::new();
    let (id, _, lane) = rebaser_stopped_mid_rebase(&mut env);
    let pushes = env.repo.lock().unwrap().pushed.len();
    let rerun = env.pending(&id)[0].clone();
    park_and_resume(&mut env, &id, &rerun.id);
    asks_again_after_resume(&mut env, &id, &rerun.id, &lane, pushes);
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

// --- a retake after close: the kept branches of the closed ticket are
// in the clones under the same name; one with nothing beyond its base is
// deleted, one with commits is asked about.

fn retake_branch() -> String {
    dispatch::git::branch_name(42, "Asset report column missing")
}

/// The workspace ticket closed by hand, `edit` applied to the fake
/// repository (a kept branch moved), and the same issue taken again:
/// the new ticket's id.
fn close_and_retake(env: &mut Env, id: &str, edit: impl FnOnce(&mut FakeRepo)) -> String {
    let now = env.tick();
    let t = env.runner.close_by_hand(id, None, now).unwrap();
    assert!(matches!(t.state, TicketState::Closed { .. }), "{t:#?}");
    edit(&mut env.repo.lock().unwrap());
    let now = env.tick();
    env.runner
        .take(
            "Orchard",
            &std::fs::read_to_string(env.data.pipeline("Orchard")).unwrap(),
            SourceSnapshot {
                kind: "github".into(),
                identity: "example-org/orchard-workspace#42".into(),
                number: Some(42),
                title: "Asset report column missing".into(),
                body: String::new(),
                url: None,
                labels: vec!["area:backend".into()],
                taken_at_ms: now,
                taken_by: None,
                pull_requests: Vec::new(),
            },
            now,
        )
        .unwrap()
        .id
}

/// A kept branch given `n` commits beyond its base, at `head`.
fn move_branch(repo: &mut FakeRepo, clone: PathBuf, head: &str, n: u64) {
    let entry = repo
        .branches
        .get_mut(&(clone, retake_branch()))
        .expect("the close kept the branch");
    *entry = (head.into(), n);
}

/// The retaken ticket held at its `branch` question: the one pending
/// decision, checked to be that.
fn branch_decision(env: &Env, id: &str) -> Decision {
    let pending = env.pending(id);
    assert_eq!(pending.len(), 1, "{pending:#?}");
    let d = pending[0].clone();
    assert_eq!((d.stage.as_str(), d.name.as_str()), ("cut", "branch"));
    assert_eq!(d.options, ["reuse", "fresh", "park"]);
    d
}

#[test]
fn a_retake_deletes_an_unmoved_kept_branch_and_cuts() {
    let (mut env, id) = workspace_env(&["area:backend"]);
    at_rerun(&mut env, &id);
    let tree_clone = env.data.repo_dir("Orchard");
    let backend = env.data.repo_dir("Orchard@backend");
    let frontend = env.data.repo_dir("Orchard@frontend");
    let branch = retake_branch();
    let new = close_and_retake(&mut env, &id, |_| {});
    let closed = env.ticket(&id);
    let p = env.runner.pipeline_of(&closed).unwrap();
    assert_eq!(
        dispatch::scheduler::kept_branches(&closed, &p, &env.data),
        [
            (branch.clone(), backend.clone()),
            (branch.clone(), frontend.clone()),
            (branch.clone(), tree_clone.clone()),
        ]
    );
    env.step();
    let repo = env.repo.lock().unwrap();
    assert_eq!(
        repo.deleted_branches,
        [
            (tree_clone, branch.clone()),
            (backend, branch.clone()),
            (frontend, branch.clone()),
        ]
    );
    let new_tree = env.worktrees.join(&new);
    assert_eq!(
        repo.worktrees
            .iter()
            .filter(|(_, d, _, _)| d.starts_with(&new_tree))
            .count(),
        3
    );
    drop(repo);
    let t = env.ticket(&new);
    assert!(t.active(), "{t:#?}");
    assert_eq!(t.tree.as_deref(), Some(new_tree.as_path()));
    assert_eq!(t.lanes.len(), 2);
}

#[test]
fn a_retake_asks_about_a_kept_branch_with_commits() {
    let (mut env, id) = workspace_env(&["area:backend"]);
    at_rerun(&mut env, &id);
    let frontend = env.data.repo_dir("Orchard@frontend");
    let branch = retake_branch();
    let investigators = env.sb().sessions_named("investigator").len();
    let new = close_and_retake(&mut env, &id, |r| {
        move_branch(r, frontend.clone(), "old00001", 1);
    });
    env.step();
    let d = branch_decision(&env, &new);
    for part in [
        branch.as_str(),
        "lane frontend",
        &frontend.display().to_string(),
        "1 commit,",
    ] {
        assert!(d.question.contains(part), "{part}: {}", d.question);
    }
    assert!(!d.question.contains("backend"), "{}", d.question);
    assert!(!d.question.contains("the tree"), "{}", d.question);
    let t = env.ticket(&new);
    assert!(t.active() && t.tree.is_none(), "{t:#?}");
    assert_eq!(
        env.sb().sessions_named("investigator").len(),
        investigators,
        "nothing ran"
    );
    let fetched = env.repo.lock().unwrap().fetched.len();
    env.step();
    assert_eq!(
        env.repo.lock().unwrap().fetched.len(),
        fetched,
        "a held cut does not fetch"
    );

    let now = env.tick();
    env.runner.decide(&new, &d.id, "reuse", None, now).unwrap();
    env.step();
    let dir = env.worktrees.join(&new).join("orchard-frontend");
    let repo = env.repo.lock().unwrap();
    assert!(
        repo.worktrees.contains(&(
            frontend.clone(),
            dir.clone(),
            branch.clone(),
            branch.clone()
        )),
        "{:#?}",
        repo.worktrees
    );
    assert_eq!(repo.heads[&dir], "old00001");
    assert!(repo.renamed_branches.is_empty());
    drop(repo);
    let t = env.ticket(&new);
    assert_eq!(t.lanes.len(), 2, "{t:#?}");
    assert!(
        t.attempts_of("investigate").count() == 1,
        "the ticket goes on to investigate: {t:#?}"
    );
}

#[test]
fn fresh_renames_the_kept_branch_and_cuts_a_new_one() {
    let (mut env, id) = workspace_env(&["area:backend"]);
    at_rerun(&mut env, &id);
    let frontend = env.data.repo_dir("Orchard@frontend");
    let branch = retake_branch();
    let closed = format!("{branch}.closed-19700101");
    let new = close_and_retake(&mut env, &id, |r| {
        move_branch(r, frontend.clone(), "old00001", 1);
        // Closed and renamed once already today.
        r.branches
            .insert((frontend.clone(), closed.clone()), ("older001".into(), 2));
    });
    env.step();
    let d = branch_decision(&env, &new);
    assert!(
        d.question.contains(&format!("{closed}-2")),
        "{}",
        d.question
    );
    let now = env.tick();
    env.runner.decide(&new, &d.id, "fresh", None, now).unwrap();
    env.step();
    let dir = env.worktrees.join(&new).join("orchard-frontend");
    let repo = env.repo.lock().unwrap();
    assert_eq!(
        repo.renamed_branches,
        [(frontend.clone(), branch.clone(), format!("{closed}-2"))]
    );
    assert!(
        repo.worktrees.contains(&(
            frontend.clone(),
            dir.clone(),
            branch.clone(),
            "origin/dev".into()
        )),
        "{:#?}",
        repo.worktrees
    );
    assert_eq!(repo.heads[&dir], "base0000");
    assert_eq!(
        repo.branches[&(frontend.clone(), format!("{closed}-2"))],
        ("old00001".into(), 1),
        "the earlier work is kept under the new name"
    );
    drop(repo);
    assert_eq!(env.ticket(&new).lanes.len(), 2);
}

#[test]
fn a_retake_with_one_moved_lane_of_three_deletes_the_others_and_names_only_it() {
    let (mut env, id) = workspace_env(&["area:backend"]);
    at_rerun(&mut env, &id);
    let tree_clone = env.data.repo_dir("Orchard");
    let backend = env.data.repo_dir("Orchard@backend");
    let frontend = env.data.repo_dir("Orchard@frontend");
    let branch = retake_branch();
    let new = close_and_retake(&mut env, &id, |r| {
        move_branch(r, backend.clone(), "old00002", 3);
    });
    env.step();
    let d = branch_decision(&env, &new);
    assert_eq!(
        env.repo.lock().unwrap().deleted_branches,
        [
            (tree_clone, branch.clone()),
            (frontend.clone(), branch.clone())
        ]
    );
    assert!(
        d.question.contains("lane backend, 3 commits,"),
        "{}",
        d.question
    );
    assert!(!d.question.contains("frontend"), "{}", d.question);
    assert!(!d.question.contains("the tree"), "{}", d.question);
    let now = env.tick();
    env.runner.decide(&new, &d.id, "reuse", None, now).unwrap();
    env.step();
    let t = env.ticket(&new);
    assert!(t.tree.is_some(), "{t:#?}");
    let names: Vec<&str> = t.lanes.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["backend", "frontend"]);
    let backend_dir = env.worktrees.join(&new).join("orchard-backend");
    assert_eq!(env.repo.lock().unwrap().heads[&backend_dir], "old00002");
}

#[test]
fn park_at_the_branch_question_parks_and_a_resume_asks_again() {
    let (mut env, id) = workspace_env(&["area:backend"]);
    at_rerun(&mut env, &id);
    let frontend = env.data.repo_dir("Orchard@frontend");
    let new = close_and_retake(&mut env, &id, |r| {
        move_branch(r, frontend.clone(), "old00001", 1);
    });
    env.step();
    let first = branch_decision(&env, &new);
    let now = env.tick();
    env.runner
        .decide(&new, &first.id, "park", None, now)
        .unwrap();
    env.step();
    let t = env.ticket(&new);
    assert!(
        matches!(&t.state, TicketState::Parked { reason } if reason == "parked by hand at decision branch"),
        "{t:#?}"
    );
    assert!(t.tree.is_none());
    let now = env.tick();
    env.runner.resume(&new, now).unwrap();
    env.step();
    let again = branch_decision(&env, &new);
    assert_ne!(again.id, first.id);
    let t = env.ticket(&new);
    let old = t.decisions.iter().find(|d| d.id == first.id).unwrap();
    assert!(
        matches!(&old.state, dispatch::ticket::DecisionState::Answered { answer, .. } if answer == "park"),
        "{old:#?}"
    );
    assert!(t.tree.is_none());
}

#[test]
fn a_reuse_whose_cut_fails_is_spent_and_a_resume_asks_again() {
    let (mut env, id) = workspace_env(&["area:backend"]);
    at_rerun(&mut env, &id);
    let tree_clone = env.data.repo_dir("Orchard");
    let frontend = env.data.repo_dir("Orchard@frontend");
    let branch = retake_branch();
    // A lane failing to go keeps the closed ticket's trees, with the
    // branches checked out in them.
    let kept = env.worktrees.join(&id).join("orchard-frontend");
    env.repo.lock().unwrap().fail_remove = Some(kept);
    let new = close_and_retake(&mut env, &id, |r| {
        move_branch(r, tree_clone.clone(), "old00003", 1);
        move_branch(r, frontend.clone(), "old00001", 1);
    });
    env.step();
    let first = branch_decision(&env, &new);
    let now = env.tick();
    env.runner
        .decide(&new, &first.id, "reuse", None, now)
        .unwrap();
    env.step();
    let t = env.ticket(&new);
    assert!(
        matches!(&t.state, TicketState::Parked { reason } if reason.contains("could not cut") && reason.contains("already used by worktree")),
        "{t:#?}"
    );
    let old = t.decisions.iter().find(|d| d.id == first.id).unwrap();
    assert_eq!(old.state, dispatch::ticket::DecisionState::Cancelled);

    let now = env.tick();
    env.runner.resume(&new, now).unwrap();
    env.step();
    let again = branch_decision(&env, &new);
    assert_ne!(again.id, first.id);
    let now = env.tick();
    env.runner
        .decide(&new, &again.id, "fresh", None, now)
        .unwrap();
    env.step();
    let t = env.ticket(&new);
    assert!(t.active(), "{t:#?}");
    assert_eq!(t.lanes.len(), 2, "{t:#?}");
    let used = t.decisions.iter().find(|d| d.id == again.id).unwrap();
    assert!(used.unacted_answer().is_none(), "{used:#?}");
    assert_eq!(
        env.repo.lock().unwrap().renamed_branches,
        [
            (
                tree_clone,
                branch.clone(),
                format!("{branch}.closed-19700101")
            ),
            (
                frontend,
                branch.clone(),
                format!("{branch}.closed-19700101")
            ),
        ]
    );
}

#[test]
fn a_kept_branch_that_cannot_be_read_parks_the_retake() {
    let (mut env, id) = workspace_env(&["area:backend"]);
    at_rerun(&mut env, &id);
    let new = close_and_retake(&mut env, &id, |r| {
        r.fail_ahead = Some("fatal: ambiguous argument 'origin/dev..'".into());
    });
    env.step();
    let t = env.ticket(&new);
    assert!(
        matches!(&t.state, TicketState::Parked { reason } if reason.contains("could not clear the kept branch") && reason.contains("ambiguous argument")),
        "{t:#?}"
    );
    assert!(t.tree.is_none());
}

#[test]
fn fresh_renames_every_moved_branch_to_the_one_name_the_question_gave() {
    let (mut env, id) = workspace_env(&["area:backend"]);
    at_rerun(&mut env, &id);
    let backend = env.data.repo_dir("Orchard@backend");
    let frontend = env.data.repo_dir("Orchard@frontend");
    let branch = retake_branch();
    let closed = format!("{branch}.closed-19700101");
    let new = close_and_retake(&mut env, &id, |r| {
        move_branch(r, backend.clone(), "old00002", 2);
        move_branch(r, frontend.clone(), "old00001", 1);
        // Only the second clone was closed and renamed once already today.
        r.branches
            .insert((frontend.clone(), closed.clone()), ("older001".into(), 2));
    });
    env.step();
    let d = branch_decision(&env, &new);
    let to = format!("{closed}-2");
    assert!(d.question.contains(&to), "{}", d.question);
    let now = env.tick();
    env.runner.decide(&new, &d.id, "fresh", None, now).unwrap();
    env.step();
    assert_eq!(
        env.repo.lock().unwrap().renamed_branches,
        [
            (backend, branch.clone(), to.clone()),
            (frontend, branch.clone(), to),
        ]
    );
    assert_eq!(env.ticket(&new).lanes.len(), 2);
}

#[test]
fn a_reused_lane_records_its_fork_point_as_its_base() {
    let (mut env, id) = workspace_env(&["area:backend"]);
    at_rerun(&mut env, &id);
    let frontend = env.data.repo_dir("Orchard@frontend");
    let new = close_and_retake(&mut env, &id, |r| {
        move_branch(r, frontend.clone(), "old00001", 1);
        r.fork_points.insert(frontend.clone(), "fork0001".into());
    });
    env.step();
    let d = branch_decision(&env, &new);
    let now = env.tick();
    env.runner.decide(&new, &d.id, "reuse", None, now).unwrap();
    env.step();
    let t = env.ticket(&new);
    let base = |name: &str| {
        t.lanes
            .iter()
            .find(|l| l.name == name)
            .and_then(|l| l.base_sha.clone())
    };
    assert_eq!(base("frontend").as_deref(), Some("fork0001"));
    assert_eq!(base("backend").as_deref(), Some("base0000"));
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
    let (id, tree) = closed_with_a_kept_tree(&mut env);
    let now = env.tick();
    let t = env.runner.close_by_hand(&id, None, now).unwrap();
    assert!(t.close.tree_removed && t.close.trees_kept.is_none());
    assert!(matches!(&t.state, TicketState::Closed { reason } if reason == "every stage is done"));
    assert_eq!(
        env.repo.lock().unwrap().removed,
        [(env.data.repo_dir(PROJECT), tree)]
    );
}

/// A ticket driven to its merge decision with clean checks and an open
/// PR.
fn at_merge_decision(env: &mut Env) -> String {
    let (id, implementer) = at_implement(env);
    implementer_stops(env, &id, &implementer);
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
    id
}

// --- a base that moves into a conflict while the ticket waits at merge

/// The ticket at its merge decision, then its lane's base moves to
/// `main0002` with the tree one behind; a merge with it conflicts in
/// `files`, and a rebase leaves `rebased1`.
fn base_moves_at_merge(env: &mut Env, files: &[&str]) -> (String, PathBuf) {
    let id = at_merge_decision(env);
    let tree = env.ticket(&id).lanes[0].worktree.clone();
    base_moves(env, &tree, files);
    (id, tree)
}

/// The base of the project's clone moves to `main0002`, with `tree` one
/// behind it; a merge with it conflicts in `files`, and a rebase leaves
/// `rebased1`.
fn base_moves(env: &Env, tree: &std::path::Path, files: &[&str]) {
    let clone = env.data.repo_dir(PROJECT);
    let mut repo = env.repo.lock().unwrap();
    repo.bases.insert(clone, "main0002".into());
    repo.behind.insert(tree.to_path_buf(), 1);
    repo.rebase_heads
        .insert(tree.to_path_buf(), "rebased1".into());
    if !files.is_empty() {
        repo.conflicting_files.insert(
            tree.to_path_buf(),
            files.iter().map(|f| (*f).to_owned()).collect(),
        );
    }
}

/// Whether the ticket stands at the stage named `stage`.
fn stands_at(t: &Ticket, stage: &str) -> bool {
    dispatch::events::stage_names(t)
        .get(t.stage)
        .is_some_and(|s| s == stage)
}

fn count_of(kinds: &[String], kind: &str) -> usize {
    kinds.iter().filter(|k| *k == kind).count()
}

/// Polls past the PR's poll interval, `n` times.
fn polls(env: &mut Env, n: usize) {
    for _ in 0..n {
        env.wait(PR_POLL_MS);
        env.step();
    }
}

/// The base moved under a ticket at `merge` and a merge with it
/// conflicts: the ticket goes back to `ready`, whose refresh rebases the
/// branch and pushes it once with the lease, whose reading of the checks
/// waits and then passes, and the ticket stands at `merge` again with a
/// new decision.
#[test]
fn a_conflicting_base_move_at_merge_is_refreshed_through_ready() {
    let mut env = Env::new();
    let (id, tree) = base_moves_at_merge(&mut env, &["src/a.rs"]);
    let first = pending_named(&env, &id, "merge").unwrap();
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the trip back", |t, _| stands_at(t, "ready"));
    let t = env.ticket(&id);
    assert_eq!(t.refreshed_stage, None);
    let left = t.attempts_of("merge").last().unwrap();
    assert!(
        matches!(&left.state, AttemptState::Cancelled { reason }
            if reason == "sent back from merge: the base moved and conflicts in src/a.rs"),
        "{left:?}"
    );
    assert!(
        t.attempts_of("ready")
            .all(|a| a.state != AttemptState::Complete),
        "ready reads the checks again"
    );
    assert_eq!(
        t.decisions.iter().find(|d| d.id == first.id).unwrap().state,
        DecisionState::Cancelled
    );
    // The provider sees the push.
    env.pr_is(&id, "rebased1", "open", Checks::Pending);
    env.steps_until(&id, "the checks pending", |t, _| {
        t.attempts_of("ready")
            .last()
            .and_then(|a| a.pr.as_ref())
            .is_some_and(|p| p.checks == "pending")
    });
    let branch = env.ticket(&id).lanes[0].branch.clone();
    assert_eq!(
        env.repo.lock().unwrap().pushed,
        vec![(tree, "origin".into(), branch, "base0000".into())]
    );
    env.pr_is(&id, "rebased1", "open", Checks::Passed);
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the merge decision again", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "merge")
    });
    let t = env.ticket(&id);
    assert_eq!(t.lanes[0].base_sha.as_deref(), Some("main0002"));
    let d = pending_named(&env, &id, "merge").unwrap();
    assert_ne!(d.id, first.id);
    assert_eq!(d.options, vec!["recheck", "park"]);
    assert!(
        d.question.contains("is open at rebased1; merge it there"),
        "{}",
        d.question
    );
    polls(&mut env, 3);
    assert_eq!(env.repo.lock().unwrap().pushed.len(), 1, "pushed once");
    assert!(stands_at(&env.ticket(&id), "merge"), "no second trip");
    let kinds = events_of(&env.data, &id);
    assert_in_order(
        &kinds,
        &[
            "sent-back",
            "refreshed",
            "pushed",
            "pr",
            "pr-checks",
            "stage",
        ],
    );
    assert_eq!(count_of(&kinds, "sent-back"), 1, "{kinds:?}");
}

/// A base that moved without a conflict is left alone at `merge`: no
/// trip back, no push, and the same merge decision.
#[test]
fn a_clean_base_move_at_merge_leaves_the_branch_alone() {
    let mut env = Env::new();
    let (id, _) = base_moves_at_merge(&mut env, &[]);
    let first = pending_named(&env, &id, "merge").unwrap();
    polls(&mut env, 3);
    let t = env.ticket(&id);
    assert!(stands_at(&t, "merge"));
    assert_eq!(t.lanes[0].base_sha.as_deref(), Some("base0000"));
    assert!(env.repo.lock().unwrap().pushed.is_empty());
    assert!(env.repo.lock().unwrap().rebased.is_empty());
    assert_eq!(pending_named(&env, &id, "merge").unwrap().id, first.id);
    assert_eq!(count_of(&events_of(&env.data, &id), "sent-back"), 0);
}

/// The trip back meets a rebase that conflicts and a policy with no
/// rebaser: `ready` asks its `refresh` question, naming the file.
#[test]
fn a_base_move_at_merge_the_rebaser_cannot_resolve_asks_refresh_naming_the_file() {
    let mut env = Env::new();
    let path = env.data.pipeline(PROJECT);
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("rebaser = \"rebaser\"\n", "");
    std::fs::write(&path, text).unwrap();
    let (id, tree) = base_moves_at_merge(&mut env, &["src/a.rs"]);
    env.repo.lock().unwrap().rebase_conflicts.push(tree);
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the refresh question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == REFRESH)
    });
    let t = env.ticket(&id);
    assert!(stands_at(&t, "ready"));
    let d = pending_named(&env, &id, REFRESH).unwrap();
    assert!(
        d.question
            .contains("a rebase onto it conflicts in src/a.rs; the policy names no rebaser"),
        "{}",
        d.question
    );
    assert!(env.sb().cloned.is_empty(), "no rebaser");
    assert!(env.repo.lock().unwrap().pushed.is_empty());
}

/// A tree with work in it is not sent back, which would only meet a
/// refresh that leaves it alone: the merge decision names the tree and
/// the file, a `recheck` while it is still not clean says so again, and
/// once it is clean `recheck` sends the ticket back and it is brought
/// up.
#[test]
fn a_dirty_tree_at_merge_asks_instead_of_looping() {
    let mut env = Env::new();
    let (id, tree) = base_moves_at_merge(&mut env, &["src/a.rs"]);
    env.repo.lock().unwrap().dirty.push(tree.clone());
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the conflict named", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == "merge" && d.question.contains("not clean"))
    });
    let d = pending_named(&env, &id, "merge").unwrap();
    assert!(
        d.question.contains(&format!(
            "the base moved and conflicts in src/a.rs, and it was not sent back through ready: the tree of lane repo at {} is not clean. Fix it there, then answer recheck.",
            tree.display()
        )),
        "{}",
        d.question
    );
    assert_eq!(d.options, vec!["recheck", "park"]);
    polls(&mut env, 3);
    assert!(stands_at(&env.ticket(&id), "merge"));
    assert_eq!(pending_named(&env, &id, "merge").unwrap().id, d.id);
    answer(&mut env, &id, &d, "recheck");
    env.step();
    let t = env.ticket(&id);
    assert!(stands_at(&t, "merge"), "still not clean");
    let again = pending_named(&env, &id, "merge").unwrap();
    assert!(
        again.question.contains("recheck was answered") && again.question.contains("is not clean"),
        "{}",
        again.question
    );
    assert_eq!(count_of(&events_of(&env.data, &id), "sent-back"), 0);
    assert!(env.sb().cloned.is_empty(), "no rebaser");
    assert!(env.repo.lock().unwrap().pushed.is_empty());
    env.repo.lock().unwrap().dirty.clear();
    answer(&mut env, &id, &again, "recheck");
    env.steps_until(&id, "the branch brought up", |t, _| {
        t.lanes[0].base_sha.as_deref() == Some("main0002")
    });
    assert!(stands_at(&env.ticket(&id), "ready"));
    assert_eq!(env.repo.lock().unwrap().pushed.len(), 1);
}

/// The same with the provider also reporting the PR conflicting: no
/// trip back, so no rebaser is started into the tree at `ready`.
#[test]
fn a_dirty_tree_with_a_conflicting_pr_at_merge_starts_no_rebaser() {
    let mut env = Env::new();
    let (id, tree) = base_moves_at_merge(&mut env, &["src/a.rs"]);
    env.repo.lock().unwrap().dirty.push(tree);
    env.pr_is_with(&id, "base0000", "open", Checks::Passed, Some("conflicting"));
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the conflict named", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == "merge" && d.question.contains("not clean"))
    });
    polls(&mut env, 3);
    assert!(stands_at(&env.ticket(&id), "merge"));
    assert!(env.sb().cloned.is_empty(), "no rebaser");
    assert_eq!(count_of(&events_of(&env.data, &id), "sent-back"), 0);
    // Without the probe, the provider's word is named.
    env.repo.lock().unwrap().conflicting_files.clear();
    polls(&mut env, 1);
    let d = pending_named(&env, &id, "merge").unwrap();
    assert!(
        d.question.contains("the provider reports it conflicting"),
        "{}",
        d.question
    );
}

/// The provider says conflicting at `merge` but cannot say at `ready`
/// (GitHub computes it lazily), and the base never moved: the trip back
/// changes nothing, so the second reading asks, naming the head the
/// trip left, and no second trip follows.
#[test]
fn a_trip_back_that_leaves_the_head_asks_once() {
    let mut env = Env::new();
    let id = at_merge_decision(&mut env);
    env.pr_is_with(&id, "base0000", "open", Checks::Passed, Some("conflicting"));
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the trip back", |t, _| stands_at(t, "ready"));
    env.pr_is_with(&id, "base0000", "open", Checks::Passed, None);
    env.steps_until(&id, "merge again", |t, _| stands_at(t, "merge"));
    env.pr_is_with(&id, "base0000", "open", Checks::Passed, Some("conflicting"));
    env.steps_until(&id, "the merge decision naming the head", |t, _| {
        t.pending_decisions().iter().any(|d| {
            d.name == "merge"
                && d.question
                    .contains("the last trip back through ready left the branch at base0000")
        })
    });
    polls(&mut env, 3);
    assert!(stands_at(&env.ticket(&id), "merge"));
    assert_eq!(count_of(&events_of(&env.data, &id), "sent-back"), 1);
    assert!(env.sb().cloned.is_empty(), "no rebaser");
}

/// A trip back on which `ready`'s rebaser moved the head (no refresh
/// recorded on the lane): a second conflicting reading at `merge` sends
/// the ticket back again rather than asking.
#[test]
fn a_second_base_move_after_a_rebaser_push_sends_back_again() {
    let mut env = Env::new();
    let id = at_merge_decision(&mut env);
    let tree = env.ticket(&id).lanes[0].worktree.clone();
    env.pr_is_with(&id, "base0000", "open", Checks::Passed, Some("conflicting"));
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the rebaser", |t, _| {
        t.attempts_of("ready")
            .any(|a| a.kind == AttemptKind::Agent && a.session.is_some())
    });
    let t = env.ticket(&id);
    let rebaser = session_of(&t, "ready");
    let notes = artifact_of(&t, "ready", "notes");
    env.repo
        .lock()
        .unwrap()
        .heads
        .insert(tree, "rebased1".into());
    env.pr_is_with(&id, "rebased1", "open", Checks::Passed, Some("clean"));
    env.finish(&rebaser, &notes, "# rebased");
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "merge again", |t, _| {
        stands_at(t, "merge") && t.pending_decisions().iter().any(|d| d.name == "merge")
    });
    assert!(env.ticket(&id).lanes[0].refreshed.is_none());
    env.pr_is_with(&id, "rebased1", "open", Checks::Passed, Some("conflicting"));
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the second trip back", |t, _| stands_at(t, "ready"));
    assert_eq!(count_of(&events_of(&env.data, &id), "sent-back"), 2);
}

/// A PR merged by hand while the conflict is reported still closes the
/// ticket: the watch polls through its own question.
#[test]
fn a_pr_merged_while_the_conflict_is_reported_closes_the_ticket() {
    let mut env = Env::new();
    let (id, tree) = base_moves_at_merge(&mut env, &["src/a.rs"]);
    env.repo.lock().unwrap().dirty.push(tree);
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the conflict named", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == "merge" && d.question.contains("not clean"))
    });
    env.pr_is(&id, "base0000", "merged", Checks::Passed);
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the ticket closing", |t, _| !t.active());
    let t = env.ticket(&id);
    assert!(
        matches!(&t.state, TicketState::Closed { .. }),
        "{:?}",
        t.state
    );
    let merge: Vec<_> = t.decisions.iter().filter(|d| d.name == "merge").collect();
    assert!(
        merge.iter().any(|d| matches!(&d.state,
            DecisionState::Answered { answer, by, .. } if answer == "merged" && by == "dispatch")),
        "{merge:?}"
    );
    assert!(merge.iter().all(|d| !d.pending()));
}

/// A fetch that fails while the merge decision names a conflict leaves
/// that decision pending, rather than asking the plain question under a
/// new id and the conflict again once the fetch works.
#[test]
fn a_failed_probe_keeps_the_pending_merge_decision() {
    let mut env = Env::new();
    let (id, tree) = base_moves_at_merge(&mut env, &["src/a.rs"]);
    env.repo.lock().unwrap().dirty.push(tree);
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the conflict named", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == "merge" && d.question.contains("not clean"))
    });
    let d = pending_named(&env, &id, "merge").unwrap();
    env.repo.lock().unwrap().fail_fetch = Some("fatal: unable to access".into());
    polls(&mut env, 2);
    env.repo.lock().unwrap().fail_fetch = None;
    polls(&mut env, 1);
    assert_eq!(pending_named(&env, &id, "merge").unwrap().id, d.id);
    let t = env.ticket(&id);
    assert_eq!(t.decisions.iter().filter(|d| d.name == "merge").count(), 2);
}

/// The provider reports the PR conflicting while the base cannot be
/// fetched: the reading is sure, so the pending question is asked again
/// naming the conflict, but the ticket is not sent back to a refresh that
/// would fail the same fetch.
#[test]
fn a_conflict_the_provider_reports_is_named_while_the_fetch_fails() {
    let mut env = Env::new();
    let (id, _) = base_moves_at_merge(&mut env, &[]);
    let first = pending_named(&env, &id, "merge").unwrap();
    env.repo.lock().unwrap().fail_fetch = Some("fatal: unable to access".into());
    env.pr_is_with(&id, "base0000", "open", Checks::Passed, Some("conflicting"));
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the conflict named", |t, _| {
        t.pending_decisions().iter().any(|d| {
            d.name == "merge"
                && d.question.contains("the provider reports it conflicting")
                && d.question.contains("could not be fetched or probed")
        })
    });
    polls(&mut env, 2);
    assert_ne!(pending_named(&env, &id, "merge").unwrap().id, first.id);
    assert!(stands_at(&env.ticket(&id), "merge"));
    assert_eq!(count_of(&events_of(&env.data, &id), "sent-back"), 0);
}

/// The owner merges by hand and answers `recheck` before the watch reads
/// the merge: the PR is read again rather than the branch sent back, so
/// the merged branch is neither rebased nor pushed and the ticket closes.
#[test]
fn a_recheck_after_a_hand_merge_closes_without_a_trip_back() {
    let mut env = Env::new();
    let (id, _) = base_moves_at_merge(&mut env, &[]);
    env.pr_is(&id, "base0000", "merged", Checks::Passed);
    answer_named(&mut env, &id, "merge", "recheck");
    env.steps_until(&id, "the ticket closing", |t, _| !t.active());
    assert_eq!(count_of(&events_of(&env.data, &id), "sent-back"), 0);
    assert!(env.repo.lock().unwrap().rebased.is_empty());
    assert!(env.repo.lock().unwrap().pushed.is_empty());
}

/// A `merge` stage in the `root` context reads no lane of its own, so
/// only the provider's `conflicting` sends it back: the lane trees the
/// refresh at `ready` would bring up are checked all the same, and a
/// dirty one asks instead of starting a rebaser into it.
#[test]
fn a_dirty_tree_under_a_root_merge_asks_instead_of_looping() {
    let mut env = Env::new();
    let path = env.data.pipeline(PROJECT);
    let text = std::fs::read_to_string(&path).unwrap().replace(
        "name = \"merge\"\ncontext = \"each\"",
        "name = \"merge\"\ncontext = \"root\"",
    );
    std::fs::write(&path, text).unwrap();
    let id = at_merge_decision(&mut env);
    let tree = env.ticket(&id).lanes[0].worktree.clone();
    env.repo.lock().unwrap().dirty.push(tree.clone());
    env.pr_is_with(&id, "base0000", "open", Checks::Passed, Some("conflicting"));
    env.wait(PR_POLL_MS);
    let named = format!("the tree of lane repo at {} is not clean", tree.display());
    env.steps_until(&id, "the dirty tree named", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == "merge" && d.question.contains(&named))
    });
    polls(&mut env, 2);
    assert!(stands_at(&env.ticket(&id), "merge"));
    assert_eq!(count_of(&events_of(&env.data, &id), "sent-back"), 0);
    assert!(env.sb().cloned.is_empty(), "no rebaser");
}

/// The test pipeline with `stage` (TOML) between `ready` and `merge`.
fn with_stage_before_merge(env: &Env, stage: &str) {
    let path = env.data.pipeline(PROJECT);
    let text = std::fs::read_to_string(&path).unwrap().replace(
        "[[stages]]\nname = \"merge\"\n",
        &format!("{stage}\n[[stages]]\nname = \"merge\"\n"),
    );
    std::fs::write(&path, text).unwrap();
}

/// A human gate between `ready` and `merge` passed at the old head: the
/// trip back cancels its pass, so it asks again and its answer binds the
/// rebased head.
#[test]
fn a_gate_between_ready_and_merge_judges_the_new_head() {
    let mut env = Env::new();
    with_stage_before_merge(
        &env,
        "[[stages]]\nname = \"look\"\ncontext = \"each\"\ngate = { kind = \"human\", decision = \"look\" }\n",
    );
    let id = at_ready(&mut env);
    env.pr_is(&id, "base0000", "open", Checks::Passed);
    env.recheck(&id);
    env.steps_until(&id, "the look", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "look")
    });
    answer_named(&mut env, &id, "look", "proceed");
    env.steps_until(&id, "the merge decision", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "merge")
    });
    let tree = env.ticket(&id).lanes[0].worktree.clone();
    base_moves(&env, &tree, &["src/a.rs"]);
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the trip back", |t, _| stands_at(t, "ready"));
    let t = env.ticket(&id);
    let look = t.attempts_of("look").last().unwrap();
    assert!(
        matches!(&look.state, AttemptState::Cancelled { reason } if reason.starts_with("sent back from merge: ")),
        "{look:?}"
    );
    env.pr_is(&id, "rebased1", "open", Checks::Passed);
    env.steps_until(&id, "the look again", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "look")
    });
    answer_named(&mut env, &id, "look", "proceed");
    env.steps_until(&id, "the merge decision again", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "merge")
    });
    let t = env.ticket(&id);
    let look = t.attempts_of("look").last().unwrap();
    assert_eq!(look.state, AttemptState::Complete);
    assert_eq!(look.head.as_deref(), Some("rebased1"));
}

/// An agent stage between `ready` and `merge` would run again on a trip
/// back, which costs a run nobody approved: the merge decision names it
/// instead.
#[test]
fn an_agent_stage_between_ready_and_merge_holds_the_trip_back() {
    let mut env = Env::new();
    with_stage_before_merge(
        &env,
        "[[stages]]\nname = \"polish\"\noperator = \"implementer\"\ncontext = \"each\"\nwrites = [\"notes\"]\nprompt = \"Polish {{branch}}.\"\n",
    );
    let id = at_ready(&mut env);
    env.pr_is(&id, "base0000", "open", Checks::Passed);
    env.recheck(&id);
    env.steps_until(&id, "the polisher", |t, _| {
        t.attempts_of("polish").any(|a| a.session.is_some())
    });
    let t = env.ticket(&id);
    let polisher = session_of(&t, "polish");
    let notes = artifact_of(&t, "polish", "notes");
    env.finish(&polisher, &notes, "# polished");
    env.steps_until(&id, "the merge decision", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "merge")
    });
    let tree = env.ticket(&id).lanes[0].worktree.clone();
    base_moves(&env, &tree, &["src/a.rs"]);
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the conflict named", |t, _| {
        t.pending_decisions().iter().any(|d| {
            d.name == "merge"
                && d.question
                    .contains("polish lies between and would run again")
        })
    });
    polls(&mut env, 2);
    assert!(stands_at(&env.ticket(&id), "merge"));
    assert_eq!(count_of(&events_of(&env.data, &id), "sent-back"), 0);
}

/// `recheck` answered on the merge decision with no conflict: the ticket
/// goes back through `ready`, which reads the checks again, and stands
/// at `merge` again.
#[test]
fn merge_recheck_sends_the_ticket_back_through_ready() {
    let mut env = Env::new();
    let id = at_merge_decision(&mut env);
    answer_named(&mut env, &id, "merge", "recheck");
    env.steps_until(&id, "the trip back", |t, _| stands_at(t, "ready"));
    env.steps_until(&id, "the merge decision again", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "merge")
    });
    let t = env.ticket(&id);
    let ready: Vec<_> = t
        .attempts_of("ready")
        .filter(|a| a.kind == AttemptKind::GateOnly)
        .collect();
    assert_eq!(ready.len(), 2, "{ready:?}");
    assert!(matches!(&ready[0].state, AttemptState::Cancelled { reason }
            if reason == "sent back from merge: recheck answered"));
    assert_eq!(ready[1].state, AttemptState::Complete);
    assert_eq!(count_of(&events_of(&env.data, &id), "sent-back"), 1);
    assert!(env.repo.lock().unwrap().pushed.is_empty());
}

/// Dispatch restarts right after the trip back was saved: the new
/// runner refreshes at `ready` and pushes once.
#[test]
fn a_restart_after_the_send_back_refreshes_once() {
    let mut env = Env::new();
    let (id, _) = base_moves_at_merge(&mut env, &["src/a.rs"]);
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the trip back", |t, _| stands_at(t, "ready"));
    env.restart();
    env.pr_is(&id, "rebased1", "open", Checks::Passed);
    env.steps_until(&id, "the merge decision again", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "merge")
    });
    polls(&mut env, 2);
    assert_eq!(env.repo.lock().unwrap().pushed.len(), 1);
    assert_eq!(count_of(&events_of(&env.data, &id), "sent-back"), 1);
}

/// A provider that reports a short head (Bitbucket does): the push
/// after the trip back is leased on the full head `ready` read, not the
/// short one the merge watch recorded.
#[test]
fn the_lease_after_a_trip_back_uses_the_full_head() {
    let mut env = Env::new();
    let id = at_ready(&mut env);
    env.pr_is(&id, "base000", "open", Checks::Passed);
    env.recheck(&id);
    env.steps_until(&id, "the merge decision", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "merge")
    });
    let tree = env.ticket(&id).lanes[0].worktree.clone();
    base_moves(&env, &tree, &["src/a.rs"]);
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the trip back", |t, _| stands_at(t, "ready"));
    env.pr_is(&id, "rebased", "open", Checks::Passed);
    env.steps_until(&id, "the merge decision again", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "merge")
    });
    let pushed = env.repo.lock().unwrap().pushed.clone();
    assert_eq!(pushed.len(), 1);
    assert_eq!(pushed[0].3, "base0000");
}

/// A ticket closed past its last stage with its tree kept because it
/// was dirty, and the tree since cleaned: a hand close retries it.
fn closed_with_a_kept_tree(env: &mut Env) -> (String, PathBuf) {
    let id = at_merge_decision(env);
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
    (id, tree)
}

// --- a close's tree removal runs with the writer lock let go

/// Every `worktree_remove` from now on stops at the returned gate.
fn gate_removals(env: &Env) -> Arc<Gate> {
    let gate = Arc::new(Gate::default());
    env.repo
        .lock()
        .unwrap()
        .gates
        .insert("worktree_remove", Arc::clone(&gate));
    gate
}

/// What another `dispatch` process needs to build its own runner over
/// the same records, Switchboard and repository.
#[derive(Clone)]
struct Cli {
    data: DataDir,
    sb: Arc<Mutex<FakeSwitchboard>>,
    repo: Arc<Mutex<FakeRepo>>,
}

impl Cli {
    fn of(env: &Env) -> Self {
        Self {
            data: env.data.clone(),
            sb: Arc::clone(&env.sb),
            repo: Arc::clone(&env.repo),
        }
    }

    fn runner(&self) -> Runner {
        Runner::new(
            self.data.clone(),
            Box::new(SharedPort(Arc::clone(&self.sb))),
            Box::new(Arc::clone(&self.repo)),
        )
    }
}

/// Issue 8 taken by another process once a removal is stopped at
/// `gate`, which it then lets go: the thread returns the new ticket's id
/// and how long the take took.
fn take_in_the_gap(
    gate: &Arc<Gate>,
    cli: Cli,
    now: u64,
) -> std::thread::JoinHandle<(String, std::time::Duration)> {
    let gate = Arc::clone(gate);
    std::thread::spawn(move || {
        gate.wait_entered();
        let started = std::time::Instant::now();
        let b = take_issue(&mut cli.runner(), &cli.data, 8, now + 500).unwrap();
        let took = started.elapsed();
        gate.release();
        (b.id, took)
    })
}

/// A ticket at its `rerun` decision with nothing running, closed by
/// record: the next pass finishes the close.
fn closing_with_nothing_running(env: &mut Env) -> String {
    let id = env.take(7).id;
    at_rerun(env, &id);
    close_by_record(env, &id);
    id
}

#[test]
fn a_take_during_a_long_tree_removal_returns_at_once_and_runs_next_pass() {
    let mut env = Env::new();
    let a = closing_with_nothing_running(&mut env);
    let gate = gate_removals(&env);
    let cli = Cli::of(&env);
    let taker = take_in_the_gap(&gate, cli, env.now);
    env.step();
    let (b, took) = taker.join().unwrap();
    assert!(took < std::time::Duration::from_secs(1), "took {took:?}");
    let t = env.ticket(&a);
    assert!(matches!(t.state, TicketState::Closed { .. }), "{t:#?}");
    let ps = env.runner.load_project(PROJECT).unwrap();
    assert!(!ps.closing.contains(&a), "{ps:#?}");
    assert_eq!(ps.queue, vec![b.clone()]);
    env.step();
    let investigator = session_of(&env.ticket(&b), "investigate");
    assert!(env.sb().sessions.iter().any(|s| s.id == investigator));
}

#[test]
fn a_take_while_the_last_stage_closes_the_ticket_is_kept() {
    let mut env = Env::new();
    let id = at_merge_decision(&mut env);
    env.pr_is(&id, "base0000", "merged", Checks::Passed);
    env.wait(PR_POLL_MS);
    let gate = gate_removals(&env);
    let cli = Cli::of(&env);
    let taker = take_in_the_gap(&gate, cli, env.now);
    env.steps_until(&id, "the ticket closing", |t, _| !t.active());
    let (b, took) = taker.join().unwrap();
    assert!(took < std::time::Duration::from_secs(1), "took {took:?}");
    let t = env.ticket(&id);
    assert!(
        matches!(&t.state, TicketState::Closed { reason } if reason == "every stage is done"),
        "{t:#?}"
    );
    assert!(t.close.tree_removed);
    let ps = env.runner.load_project(PROJECT).unwrap();
    assert!(ps.queue.contains(&b), "the take was lost: {ps:#?}");
    assert!(!ps.closing.contains(&id));
}

#[test]
fn a_decision_on_a_ticket_whose_trees_are_being_removed_is_refused() {
    let mut env = Env::new();
    let id = env.take(7).id;
    at_rerun(&mut env, &id);
    let decision = env.pending(&id)[0].id.clone();
    close_by_record(&env, &id);
    let gate = gate_removals(&env);
    let cli = Cli::of(&env);
    let now = env.now;
    let decider = {
        let gate = Arc::clone(&gate);
        let id = id.clone();
        std::thread::spawn(move || {
            gate.wait_entered();
            let runner = cli.runner();
            let before = runner.load_ticket(&id).unwrap();
            let refused = runner.decide(&id, &decision, "rerun", None, now + 500);
            let after = runner.load_ticket(&id).unwrap();
            gate.release();
            (refused.map(|d| d.id), before, after)
        })
    };
    env.step();
    let (refused, before, after) = decider.join().unwrap();
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(before, after, "the refused decide wrote the record");
    assert!(matches!(env.ticket(&id).state, TicketState::Closed { .. }));
}

#[test]
fn a_pass_during_a_hand_close_leaves_the_removal_to_it() {
    let mut env = Env::new();
    let id = env.take(7).id;
    at_rerun(&mut env, &id);
    let gate = gate_removals(&env);
    let cli = Cli::of(&env);
    let now = env.tick();
    let closer = {
        let id = id.clone();
        std::thread::spawn(move || cli.runner().close_by_hand(&id, None, now))
    };
    gate.wait_entered();
    let calls = env.sb().calls.len();
    env.step();
    assert_eq!(env.sb().calls.len(), calls, "the pass sent something");
    assert!(env.repo.lock().unwrap().removed.is_empty());
    assert!(matches!(env.ticket(&id).state, TicketState::Closing { .. }));
    gate.release();
    let t = closer.join().unwrap().unwrap();
    assert!(matches!(t.state, TicketState::Closed { .. }), "{t:#?}");
    let removed = env.repo.lock().unwrap().removed.clone();
    let mut once = removed.clone();
    once.sort();
    once.dedup();
    assert_eq!(removed.len(), once.len(), "{removed:?}");
    assert!(!removed.is_empty());
}

#[test]
fn a_retry_in_progress_keeps_its_trees_retryable() {
    let mut env = Env::new();
    let (id, tree) = closed_with_a_kept_tree(&mut env);
    let gate = gate_removals(&env);
    let cli = Cli::of(&env);
    let now = env.tick();
    let retry = {
        let id = id.clone();
        std::thread::spawn(move || cli.runner().close_by_hand(&id, None, now))
    };
    gate.wait_entered();
    // What a retry killed here would leave: still marked, still
    // retryable by the next `dispatch close`.
    let t = env.ticket(&id);
    assert!(
        t.close.trees_kept.is_some() && t.trees_retryable(),
        "{t:#?}"
    );
    gate.release();
    let t = retry.join().unwrap().unwrap();
    assert!(t.close.trees_kept.is_none() && t.close.tree_removed);
    assert_eq!(env.ticket(&id).close.trees_kept, None);
    assert_eq!(
        env.repo.lock().unwrap().removed,
        [(env.data.repo_dir(PROJECT), tree)]
    );
}

#[test]
fn a_second_retry_during_a_retry_is_turned_away() {
    let mut env = Env::new();
    let (id, tree) = closed_with_a_kept_tree(&mut env);
    let gate = gate_removals(&env);
    let cli = Cli::of(&env);
    let now = env.tick();
    let retry = {
        let id = id.clone();
        std::thread::spawn(move || cli.runner().close_by_hand(&id, None, now))
    };
    gate.wait_entered();
    let now = env.tick();
    let e = env.runner.close_by_hand(&id, None, now).unwrap_err();
    assert!(
        format!("{e:#}").contains("its trees are being removed by another dispatch"),
        "{e:#}"
    );
    gate.release();
    let t = retry.join().unwrap().unwrap();
    assert!(t.close.trees_kept.is_none());
    assert_eq!(
        env.repo.lock().unwrap().removed,
        [(env.data.repo_dir(PROJECT), tree)]
    );
}

// --- a transaction and a command from the terminal

#[test]
fn a_decision_and_a_step_edit_in_one_transaction_both_land() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let decision = env.pending(&id)[0].id.clone();
    let cli = Cli::of(&env);
    let now = env.tick();
    // The decide can only run once the transaction ends, whenever its
    // thread is scheduled: the step's edit is saved under the lock
    // first, and the decide reads it back before writing.
    let (ticket_id, decision_id) = (id.clone(), decision.clone());
    let answered = env
        .runner
        .transaction(|r| {
            let mut stale = r.load_ticket(&ticket_id)?;
            let decider = std::thread::spawn(move || {
                cli.runner()
                    .decide(&ticket_id, &decision_id, "finalize", None, now + 1)
                    .map(|d| d.id)
            });
            stale.source.title.push_str(" (edited by the step)");
            r.save_ticket(&mut stale, now)?;
            Ok(decider)
        })
        .unwrap()
        .join()
        .unwrap()
        .unwrap();
    assert_eq!(answered, decision);
    let t = env.ticket(&id);
    assert!(t.source.title.ends_with(" (edited by the step)"), "{t:#?}");
    let d = t.decisions.iter().find(|d| d.id == decision).unwrap();
    assert!(
        matches!(d.state, dispatch::ticket::DecisionState::Answered { .. }),
        "{d:#?}"
    );
}

// --- a code review stage leaves clean commits

/// `commits = "<mode>"` added to the review stage of the pipeline the
/// test already wrote.
fn with_commits(env: &Env, mode: &str) {
    let path = env.data.pipeline(PROJECT);
    let text = std::fs::read_to_string(&path).unwrap();
    let stage = "\nimplementer = \"implementer\"\n";
    assert!(text.contains(stage));
    let text = text.replace(stage, &format!("{stage}commits = \"{mode}\"\n"));
    std::fs::write(path, text).unwrap();
}

/// The branch the fake reports for the lane: the implementer's two
/// commits, then a fixup the first fix pass made and a plain fix the
/// second made.
fn seed_commits(env: &Env, id: &str) -> PathBuf {
    let tree = env.ticket(id).lanes[0].worktree.clone();
    let c = |sha: &str, message: &str| Commit {
        sha: sha.to_owned(),
        parents: 1,
        message: message.to_owned(),
    };
    env.repo.lock().unwrap().commits.insert(
        tree.clone(),
        vec![
            c("impl0001", "A"),
            c("base0000", "B"),
            c("fix00001", "fixup! A"),
            c("fix00002", "plain fix"),
        ],
    );
    tree
}

/// From the first round: two fix passes (`fix00001`, `fix00002`) and a
/// third round with no findings, up to its checks starting.
fn two_fixes_then_the_final_checks(env: &mut Env, id: &str) {
    lint_exits(env, id, 1, 1, "src/a.rs:3: unused import\n");
    style_says(env, id, 1, "No findings.");
    fix_pass(env, id, 1, "fix00001", "- r1/lint-1: fixed removed\n", 0);
    round_started(env, id, 2);
    lint_exits(env, id, 2, 1, "src/b.rs:9: dead code\n");
    style_says(env, id, 2, "No findings.");
    fix_pass(env, id, 2, "fix00002", "- r2/lint-1: fixed removed\n", 0);
    round_started(env, id, 3);
    lint_exits(env, id, 3, 0, "");
    style_says(env, id, 3, "No findings.");
    env.steps_until(id, "the final checks", |t, _| {
        review_attempt(t).gate.is_some()
    });
}

/// A ticket whose review stage has `commits = mode` (none when `None`)
/// at its final checks, with the branch seeded.
fn at_final_checks(env: &mut Env, mode: Option<&str>) -> (String, PathBuf) {
    with_three_rounds(env);
    if let Some(mode) = mode {
        with_commits(env, mode);
    }
    let id = at_review(env);
    let tree = seed_commits(env, &id);
    two_fixes_then_the_final_checks(env, &id);
    (id, tree)
}

/// The final checks pass; the attempt ends one way or the other.
fn final_checks_pass(env: &mut Env, id: &str) -> Attempt {
    let t = env.ticket(id);
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(checks_key(&t, 3), 0);
    env.steps_until(id, "the attempt ending", |t, _| {
        !review_attempt(t).is_open()
    });
    review_attempt(&env.ticket(id))
}

fn group(picks: &[&str], message: &str) -> Group {
    Group {
        picks: picks.iter().map(|p| (*p).to_owned()).collect(),
        message: message.to_owned(),
        author_of: picks[0].to_owned(),
    }
}

/// The rewrite without its time.
fn rewrite_of(a: &Attempt) -> Option<Rewrite> {
    a.rewrite.clone().map(|r| Rewrite { at_ms: 0, ..r })
}

/// The acceptance: the fixup folds into the commit it names, the plain
/// fix into the tip the second round reviewed; the branch moves once,
/// to a head with the same tree, and the checks are not run again.
#[test]
fn fold_completes_with_the_fix_rounds_folded_and_the_checks_run_once() {
    let mut env = Env::new();
    let (id, tree) = at_final_checks(&mut env, Some("fold"));
    let checks_before = env.repo.lock().unwrap().checks.len();
    let a = final_checks_pass(&mut env, &id);
    assert_eq!(a.state, AttemptState::Complete);
    let t = env.ticket(&id);
    {
        let repo = env.repo.lock().unwrap();
        assert_eq!(
            repo.replayed,
            [(
                tree.clone(),
                "root0000".to_owned(),
                vec![
                    group(&["impl0001", "fix00001"], "A"),
                    group(&["base0000", "fix00002"], "B"),
                ]
            )]
        );
        assert_eq!(
            repo.head_sets,
            [(tree.clone(), "fold0001".to_owned(), "fix00002".to_owned())]
        );
        assert_eq!(repo.heads[&tree], "fold0001");
        assert_eq!(
            repo.checks.len(),
            checks_before,
            "no checks after the rewrite"
        );
        let key = checks_key(&t, 3);
        assert_eq!(repo.checks.iter().filter(|c| c.key == key).count(), 1);
    }
    assert_eq!(a.head.as_deref(), Some("fold0001"));
    assert_eq!(
        a.gate.as_ref().unwrap().head,
        "fix00002",
        "where the checks ran"
    );
    assert_eq!(
        rewrite_of(&a),
        Some(Rewrite {
            mode: Commits::Fold,
            before: "fix00002".into(),
            after: Some("fold0001".into()),
            from: 4,
            to: 2,
            skipped: None,
            stale: Vec::new(),
            message: None,
            at_ms: 0,
        })
    );
    let summary = std::fs::read_to_string(&a.artifacts["summary"]).unwrap();
    assert!(
        summary.contains("Commits folded from 4 to 2: `fix00002` → `fold0001`."),
        "{summary}"
    );
}

#[test]
fn one_completes_with_one_commit() {
    let mut env = Env::new();
    let (id, tree) = at_final_checks(&mut env, Some("one"));
    let a = final_checks_pass(&mut env, &id);
    assert_eq!(a.state, AttemptState::Complete);
    assert_eq!(
        env.repo.lock().unwrap().replayed,
        [(
            tree,
            "root0000".to_owned(),
            vec![group(
                &["impl0001", "base0000", "fix00001", "fix00002"],
                "A"
            )]
        )]
    );
    let r = a.rewrite.clone().unwrap();
    assert_eq!((r.mode, r.from, r.to), (Commits::One, 4, 1));
    assert_eq!(a.head.as_deref(), Some("fold0001"));
    let summary = std::fs::read_to_string(&a.artifacts["summary"]).unwrap();
    assert!(
        summary.contains("Squashed 4 commits to one: `fix00002` → `fold0001`."),
        "{summary}"
    );
}

#[test]
fn keep_or_no_key_leaves_history_alone() {
    for mode in [None, Some("keep")] {
        let mut env = Env::new();
        let (id, _) = at_final_checks(&mut env, mode);
        let a = final_checks_pass(&mut env, &id);
        assert_eq!(a.state, AttemptState::Complete, "{mode:?}");
        let repo = env.repo.lock().unwrap();
        assert!(repo.replayed.is_empty(), "{mode:?}");
        assert!(repo.head_sets.is_empty(), "{mode:?}");
        assert_eq!(a.rewrite, None, "{mode:?}");
        assert_eq!(a.head.as_deref(), Some("fix00002"), "{mode:?}");
    }
}

/// The attempt failed into a `rerun` question offering `options`, with
/// the branch at the head the checks passed at.
fn failed_with_the_branch_at_the_reviewed_head(
    env: &Env,
    id: &str,
    a: &Attempt,
    tree: &std::path::Path,
    options: &[&str],
) {
    assert!(
        matches!(a.state, AttemptState::Failed { .. }),
        "{:?}",
        a.state
    );
    assert_eq!(env.repo.lock().unwrap().heads[tree], "fix00002");
    let d = env
        .pending(id)
        .into_iter()
        .find(|d| d.name == "rerun")
        .unwrap();
    assert_eq!(d.options, options);
}

#[test]
fn a_rewritten_tree_that_differs_fails_with_the_branch_unmoved() {
    let mut env = Env::new();
    let (id, tree) = at_final_checks(&mut env, Some("fold"));
    env.repo
        .lock()
        .unwrap()
        .trees
        .insert("fold0001".into(), "tree-bad".into());
    let a = final_checks_pass(&mut env, &id);
    failed_with_the_branch_at_the_reviewed_head(&env, &id, &a, &tree, &["rerun", "keep", "park"]);
    assert!(env.repo.lock().unwrap().head_sets.is_empty());
    let r = a.rewrite.clone().unwrap();
    assert_eq!(
        (r.before.as_str(), r.after.as_deref()),
        ("fix00002", Some("fold0001"))
    );
    let AttemptState::Failed { reason } = &a.state else {
        unreachable!()
    };
    assert!(
        reason.contains("the rewritten head fold0001 has tree tree-bad, not tree0000 as fix00002 has; the branch stays at fix00002"),
        "{reason}"
    );
}

#[test]
fn a_tree_dirty_after_the_move_is_moved_back_and_fails() {
    let mut env = Env::new();
    let (id, tree) = at_final_checks(&mut env, Some("fold"));
    env.repo.lock().unwrap().dirty_on_set.push(tree.clone());
    let a = final_checks_pass(&mut env, &id);
    failed_with_the_branch_at_the_reviewed_head(&env, &id, &a, &tree, &["rerun", "keep", "park"]);
    assert_eq!(
        env.repo.lock().unwrap().head_sets,
        [
            (tree.clone(), "fold0001".to_owned(), "fix00002".to_owned()),
            (tree.clone(), "fix00002".to_owned(), "fold0001".to_owned()),
        ]
    );
    let AttemptState::Failed { reason } = &a.state else {
        unreachable!()
    };
    assert!(
        reason.ends_with("the branch is back at fix00002"),
        "{reason}"
    );
}

/// The pending `rerun` question, answered `answer`.
fn answer_rerun(env: &mut Env, id: &str, answer: &str) -> Decision {
    let d = env
        .pending(id)
        .into_iter()
        .find(|d| d.name == "rerun")
        .unwrap_or_else(|| panic!("no rerun question: {:#?}", env.ticket(id)));
    let now = env.tick();
    env.runner.decide(id, &d.id, answer, None, now).unwrap();
    d
}

/// A fold at its final checks whose replay conflicts: the checks pass
/// at `fix00002` and the attempt fails into `rerun | keep | park`.
fn a_fold_that_conflicts(env: &mut Env) -> (String, PathBuf) {
    let (id, tree) = at_final_checks(env, Some("fold"));
    env.repo.lock().unwrap().replay_conflicts.push(tree.clone());
    let a = final_checks_pass(env, &id);
    failed_with_the_branch_at_the_reviewed_head(env, &id, &a, &tree, &["rerun", "keep", "park"]);
    (id, tree)
}

/// The attempt completed at `fix00002` with its history kept by hand
/// and nothing run or moved since `checks`, and the ticket moved on to
/// `inspect` with the branch still there.
fn kept_by_hand_at_the_reviewed_head(
    env: &mut Env,
    id: &str,
    tree: &std::path::Path,
    checks: usize,
) {
    env.steps_until(id, "the next stage", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "inspect")
    });
    let t = env.ticket(id);
    assert_eq!(t.state, TicketState::Active, "{:?}", t.state);
    assert_eq!(t.attempts_of("review-code").count(), 1, "no rerun");
    let a = review_attempt(&t);
    assert_eq!(a.state, AttemptState::Complete, "{:?}", a.state);
    assert_eq!(a.head.as_deref(), Some("fix00002"));
    let r = rewrite_of(&a).unwrap();
    assert_eq!(
        r.skipped.as_deref(),
        Some("the user kept them after the rewrite failed")
    );
    assert_eq!(
        (r.before.as_str(), r.after.as_deref()),
        ("fix00002", Some("fix00002"))
    );
    let summary = std::fs::read_to_string(&a.artifacts["summary"]).unwrap();
    assert!(
        summary.contains("Commits kept: the user kept them after the rewrite failed."),
        "{summary}"
    );
    let repo = env.repo.lock().unwrap();
    assert_eq!(repo.heads[tree], "fix00002");
    assert_eq!(repo.checks.len(), checks, "no checks run again");
}

#[test]
fn a_fold_whose_replay_conflicts_is_kept_by_hand() {
    let mut env = Env::new();
    let (id, tree) = a_fold_that_conflicts(&mut env);
    assert!(env.repo.lock().unwrap().head_sets.is_empty());
    let checks = env.repo.lock().unwrap().checks.len();
    let d = answer_rerun(&mut env, &id, "keep");
    for o in ["`rerun`", "`keep`", "`park`", "fix00002"] {
        assert!(d.question.contains(o), "{o}: {}", d.question);
    }
    kept_by_hand_at_the_reviewed_head(&mut env, &id, &tree, checks);
    assert!(env.repo.lock().unwrap().head_sets.is_empty());
}

#[test]
fn keep_after_a_differing_tree_completes_at_the_reviewed_head() {
    let mut env = Env::new();
    let (id, tree) = at_final_checks(&mut env, Some("fold"));
    env.repo
        .lock()
        .unwrap()
        .trees
        .insert("fold0001".into(), "tree-bad".into());
    let a = final_checks_pass(&mut env, &id);
    assert_eq!(a.rewrite.unwrap().after.as_deref(), Some("fold0001"));
    let checks = env.repo.lock().unwrap().checks.len();
    answer_rerun(&mut env, &id, "keep");
    kept_by_hand_at_the_reviewed_head(&mut env, &id, &tree, checks);
}

#[test]
fn keep_after_a_move_back_completes_once_the_tree_is_cleaned() {
    let mut env = Env::new();
    let (id, tree) = at_final_checks(&mut env, Some("fold"));
    env.repo.lock().unwrap().dirty_on_set.push(tree.clone());
    final_checks_pass(&mut env, &id);
    let checks = env.repo.lock().unwrap().checks.len();
    env.repo.lock().unwrap().dirty.retain(|d| *d != tree);
    answer_rerun(&mut env, &id, "keep");
    kept_by_hand_at_the_reviewed_head(&mut env, &id, &tree, checks);
}

#[test]
fn keep_after_a_move_back_on_a_tree_left_dirty_is_asked_again_and_kept_once_cleaned() {
    let mut env = Env::new();
    let (id, tree) = at_final_checks(&mut env, Some("fold"));
    env.repo.lock().unwrap().dirty_on_set.push(tree.clone());
    final_checks_pass(&mut env, &id);
    let checks = env.repo.lock().unwrap().checks.len();
    let kept = answer_rerun(&mut env, &id, "keep");
    env.steps_until(&id, "a fresh question", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == "rerun" && d.id != kept.id)
    });
    let a = review_attempt(&env.ticket(&id));
    failed_with_the_branch_at_the_reviewed_head(&env, &id, &a, &tree, &["rerun", "keep", "park"]);
    assert!(a.head.is_none(), "nothing completed");
    assert!(!a.artifacts.contains_key("summary"));
    env.repo.lock().unwrap().dirty.retain(|d| *d != tree);
    answer_rerun(&mut env, &id, "keep");
    kept_by_hand_at_the_reviewed_head(&mut env, &id, &tree, checks);
}

#[test]
fn keep_with_the_branch_moved_since_parks() {
    let mut env = Env::new();
    let (id, tree) = a_fold_that_conflicts(&mut env);
    env.repo
        .lock()
        .unwrap()
        .heads
        .insert(tree.clone(), "other001".into());
    answer_rerun(&mut env, &id, "keep");
    env.steps_until(&id, "the park", |t, _| {
        matches!(t.state, TicketState::Parked { .. })
    });
    let t = env.ticket(&id);
    let TicketState::Parked { reason, .. } = &t.state else {
        unreachable!()
    };
    assert!(
        reason.contains("fix00002") && reason.contains("other001"),
        "{reason}"
    );
    assert!(matches!(
        review_attempt(&t).state,
        AttemptState::Failed { .. }
    ));
}

#[test]
fn keep_on_a_dirty_tree_is_asked_again() {
    let mut env = Env::new();
    let (id, tree) = a_fold_that_conflicts(&mut env);
    env.repo.lock().unwrap().dirty.push(tree.clone());
    let kept = answer_rerun(&mut env, &id, "keep");
    env.steps_until(&id, "a fresh question", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == "rerun" && d.id != kept.id)
    });
    let a = review_attempt(&env.ticket(&id));
    failed_with_the_branch_at_the_reviewed_head(&env, &id, &a, &tree, &["rerun", "keep", "park"]);
    let AttemptState::Failed { reason } = &a.state else {
        unreachable!()
    };
    assert!(reason.contains("is not clean"), "{reason}");
}

#[test]
fn a_parked_rewrite_failure_offers_keep_on_resume() {
    let mut env = Env::new();
    let (id, _) = a_fold_that_conflicts(&mut env);
    let parked_from = answer_rerun(&mut env, &id, "park");
    env.steps_until(&id, "the park", |t, _| {
        matches!(t.state, TicketState::Parked { .. })
    });
    assert!(review_attempt(&env.ticket(&id)).failed_at_rewrite);
    let now = env.tick();
    env.runner.resume(&id, now).unwrap();
    env.steps_until(&id, "the question again", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == "rerun" && d.id != parked_from.id)
    });
    let d = env
        .pending(&id)
        .into_iter()
        .find(|d| d.name == "rerun")
        .unwrap();
    assert_eq!(d.options, vec!["rerun", "keep", "park"]);
    assert!(
        d.question
            .contains("`keep` completes the stage at fix00002"),
        "{}",
        d.question
    );
}

#[test]
fn keep_answered_before_a_restart_acts_once() {
    let mut env = Env::new();
    let (id, tree) = a_fold_that_conflicts(&mut env);
    let checks = env.repo.lock().unwrap().checks.len();
    answer_rerun(&mut env, &id, "keep");
    env.restart();
    kept_by_hand_at_the_reviewed_head(&mut env, &id, &tree, checks);
    let ended = review_attempt(&env.ticket(&id)).ended_ms;
    env.restart();
    env.step();
    env.step();
    let t = env.ticket(&id);
    assert_eq!(review_attempt(&t).ended_ms, ended, "completed once");
    assert_eq!(t.attempts_of("review-code").count(), 1);
    assert!(
        t.decisions
            .iter()
            .all(|d| !d.pending() || d.name != "rerun")
    );
}

/// Converged in round one with `implement`'s checks reused, so the
/// attempt has no gate of its own; its fold fails all the same, and
/// `keep` completes it on the reused checks.
#[test]
fn keep_after_reused_checks_completes_without_running_them() {
    let mut env = Env::new();
    env.with_review_stage("ask");
    with_commits(&env, "fold");
    let id = at_review(&mut env);
    let tree = env.ticket(&id).lanes[0].worktree.clone();
    {
        let mut repo = env.repo.lock().unwrap();
        repo.commits.insert(
            tree.clone(),
            vec![
                Commit {
                    sha: "impl0001".into(),
                    parents: 1,
                    message: "A".into(),
                },
                Commit {
                    sha: "base0000".into(),
                    parents: 1,
                    message: "fixup! A".into(),
                },
            ],
        );
        repo.replay_conflicts.push(tree.clone());
    }
    lint_exits(&mut env, &id, 1, 0, "");
    style_says(&mut env, &id, 1, "No findings.");
    env.steps_until(&id, "the attempt ending", |t, _| {
        !review_attempt(t).is_open()
    });
    let a = review_attempt(&env.ticket(&id));
    assert!(a.gate.is_none(), "implement's checks reused");
    assert!(
        matches!(a.state, AttemptState::Failed { .. }),
        "{:?}",
        a.state
    );
    assert_eq!(env.repo.lock().unwrap().replayed.len(), 1);
    let d = env
        .pending(&id)
        .into_iter()
        .find(|d| d.name == "rerun")
        .unwrap();
    assert_eq!(d.options, vec!["rerun", "keep", "park"]);
    let checks = env.repo.lock().unwrap().checks.len();
    answer_rerun(&mut env, &id, "keep");
    env.steps_until(&id, "the next stage", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "inspect")
    });
    let t = env.ticket(&id);
    let a = review_attempt(&t);
    assert_eq!(a.state, AttemptState::Complete, "{:?}", a.state);
    assert_eq!(a.head.as_deref(), Some("base0000"));
    assert_eq!(
        rewrite_of(&a).unwrap().skipped.as_deref(),
        Some("the user kept them after the rewrite failed")
    );
    let repo = env.repo.lock().unwrap();
    assert_eq!(repo.checks.len(), checks, "no checks run");
    assert_eq!(repo.heads[&tree], "base0000");
}

/// A completed fold attempt written back as a restart would find it
/// had the runner stopped after the intent was saved; the fake's head
/// set to `head`, then the restart.
fn restarted_mid_rewrite(env: &mut Env, id: &str, head: &str, tree: &std::path::Path) {
    let mut t = env.ticket(id);
    let a = t
        .attempts
        .iter_mut()
        .rev()
        .find(|a| a.stage == "review-code")
        .unwrap();
    a.state = AttemptState::Running;
    a.head = None;
    a.ended_ms = None;
    if let Some(r) = &mut a.rewrite {
        r.after = None;
    }
    // Back at `review-code`, in case the pass that completed it also
    // moved on.
    t.stage = 5;
    env.runner.save_ticket(&mut t, env.now).unwrap();
    env.repo
        .lock()
        .unwrap()
        .heads
        .insert(tree.to_path_buf(), head.to_owned());
    env.restart();
}

#[test]
fn a_restart_after_the_move_adopts_the_head() {
    let mut env = Env::new();
    let (id, tree) = at_final_checks(&mut env, Some("fold"));
    final_checks_pass(&mut env, &id);
    restarted_mid_rewrite(&mut env, &id, "fold0001", &tree);
    env.steps_until(&id, "the stage completing", |t, _| {
        review_attempt(t).state == AttemptState::Complete
    });
    let t = env.ticket(&id);
    assert_eq!(t.state, TicketState::Active);
    let a = review_attempt(&t);
    assert_eq!(a.head.as_deref(), Some("fold0001"));
    assert_eq!(a.rewrite.unwrap().after.as_deref(), Some("fold0001"));
    let repo = env.repo.lock().unwrap();
    assert_eq!(repo.replayed.len(), 1, "no second replay");
    assert_eq!(repo.head_sets.len(), 1);
}

#[test]
fn a_restart_before_the_move_replays_again() {
    let mut env = Env::new();
    let (id, tree) = at_final_checks(&mut env, Some("fold"));
    final_checks_pass(&mut env, &id);
    restarted_mid_rewrite(&mut env, &id, "fix00002", &tree);
    env.steps_until(&id, "the stage completing", |t, _| {
        review_attempt(t).state == AttemptState::Complete
    });
    let t = env.ticket(&id);
    assert_eq!(t.state, TicketState::Active);
    assert_eq!(review_attempt(&t).head.as_deref(), Some("fold0001"));
    let repo = env.repo.lock().unwrap();
    assert_eq!(repo.replayed.len(), 2, "replayed once more");
    assert_eq!(repo.heads[&tree], "fold0001");
}

/// Both normal paths refuse a dirty tree at the checks (with `check`
/// among the answers); the rewrite's own check is reached after a
/// restart, and fails without touching history.
#[test]
fn a_dirty_tree_at_completion_fails_without_touching_history() {
    let mut env = Env::new();
    let (id, tree) = at_final_checks(&mut env, Some("fold"));
    final_checks_pass(&mut env, &id);
    restarted_mid_rewrite(&mut env, &id, "fix00002", &tree);
    env.repo.lock().unwrap().dirty.push(tree.clone());
    env.steps_until(&id, "the attempt failing", |t, _| {
        !review_attempt(t).is_open()
    });
    let a = review_attempt(&env.ticket(&id));
    failed_with_the_branch_at_the_reviewed_head(&env, &id, &a, &tree, &["rerun", "park"]);
    let AttemptState::Failed { reason } = &a.state else {
        unreachable!()
    };
    assert!(
        reason.contains("is not clean when its commits would be rewritten"),
        "{reason}"
    );
    let repo = env.repo.lock().unwrap();
    assert_eq!(repo.replayed.len(), 1, "only the first completion's");
    assert_eq!(repo.head_sets.len(), 1);
}

/// The case the record cannot see: the first attempt's rewrite failed,
/// an agent then pushed the branch (the remote's copy holds its
/// commits, and nothing on the record says so), and the rerun converges.
/// It leaves the history alone.
#[test]
fn a_branch_pushed_by_an_agent_is_not_rewritten_by_a_rerun() {
    let mut env = Env::new();
    let (id, tree) = at_final_checks(&mut env, Some("fold"));
    env.repo
        .lock()
        .unwrap()
        .trees
        .insert("fold0001".into(), "tree-bad".into());
    let a = final_checks_pass(&mut env, &id);
    assert!(
        matches!(a.state, AttemptState::Failed { .. }),
        "{:?}",
        a.state
    );
    env.repo
        .lock()
        .unwrap()
        .published
        .push((tree.clone(), "root0000".into(), "fix00002".into()));
    rerun_with(&mut env, &id, None);
    lint_exits(&mut env, &id, 1, 0, "");
    style_says(&mut env, &id, 1, "No findings.");
    env.steps_until(&id, "the final checks", |t, _| {
        review_attempt(t).gate.is_some()
    });
    let t = env.ticket(&id);
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(checks_key(&t, 1), 0);
    env.steps_until(&id, "the rerun ending", |t, _| !review_attempt(t).is_open());
    let t = env.ticket(&id);
    let a = review_attempt(&t);
    assert_eq!(a.state, AttemptState::Complete, "{:?}", a.state);
    assert_eq!(a.n, 2);
    assert!(t.attempts_of("ready").next().is_none());
    assert!(t.lanes[0].pushed.is_none());
    assert!(t.attempts.iter().all(|a| a.pr.is_none()));
    let r = a.rewrite.clone().unwrap();
    assert_eq!(r.skipped.as_deref(), Some("the branch is published"));
    assert_eq!(
        (r.before.as_str(), r.after.as_deref()),
        ("fix00002", Some("fix00002"))
    );
    assert_eq!(a.head.as_deref(), Some("fix00002"));
    let repo = env.repo.lock().unwrap();
    assert_eq!(repo.replayed.len(), 1, "only the first attempt's");
    assert!(repo.head_sets.is_empty());
    drop(repo);
    let summary = std::fs::read_to_string(&a.artifacts["summary"]).unwrap();
    assert!(
        summary.contains("Commits kept: the branch is published."),
        "{summary}"
    );
}

/// The attempt folds its fixes and `inspect` sends the lane back. The
/// implementer adds a commit; the review stage's completed attempt
/// stands (a send-back reaches only an agent stage), so `inspect` asks
/// again and the folded history, pushed or not, is not rewritten.
#[test]
fn a_branch_pushed_by_an_agent_is_not_rewritten_after_a_send_back() {
    let mut env = Env::new();
    let (id, tree) = at_final_checks(&mut env, Some("fold"));
    let a = final_checks_pass(&mut env, &id);
    assert_eq!(a.state, AttemptState::Complete, "{:?}", a.state);
    assert_eq!(a.head.as_deref(), Some("fold0001"));
    env.inspect(&id, "rerun", Some("name the error"));
    env.steps_until(&id, "a second implementer", |t, _| {
        t.attempts_of("implement").count() == 2
    });
    env.repo
        .lock()
        .unwrap()
        .heads
        .insert(tree.clone(), "impl0002".into());
    let second = session_of(&env.ticket(&id), "implement");
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
        t.attempts_of("inspect").count() == 2
            && t.pending_decisions().iter().any(|d| d.name == "inspect")
    });
    let t = env.ticket(&id);
    assert_eq!(t.attempts_of("review-code").count(), 1);
    assert_eq!(review_attempt(&t), a, "the completed attempt stands");
    let repo = env.repo.lock().unwrap();
    assert_eq!(repo.replayed.len(), 1, "only the first completion's");
    assert_eq!(repo.head_sets.len(), 1, "only the first completion's");
    assert_eq!(repo.heads[&tree], "impl0002");
}

#[test]
fn a_published_branch_is_not_rewritten() {
    let mut env = Env::new();
    with_three_rounds(&env);
    with_commits(&env, "fold");
    let id = at_review(&mut env);
    seed_commits(&env, &id);
    let mut t = env.ticket(&id);
    t.lanes[0].pushed = Some(PushedHead {
        head: "base0000".into(),
        at_ms: env.now,
    });
    env.runner.save_ticket(&mut t, env.now).unwrap();
    two_fixes_then_the_final_checks(&mut env, &id);
    let a = final_checks_pass(&mut env, &id);
    assert_eq!(a.state, AttemptState::Complete);
    let r = a.rewrite.clone().unwrap();
    assert_eq!(r.skipped.as_deref(), Some("the branch is published"));
    assert_eq!(a.head.as_deref(), Some("fix00002"));
    assert!(env.repo.lock().unwrap().replayed.is_empty());
}

/// A rerun carries the failed attempt's state; its fix rounds fold too,
/// since they are fixes all the same.
#[test]
fn a_carried_rerun_folds_the_earlier_attempts_fixes() {
    let mut env = Env::new();
    with_three_rounds(&env);
    with_commits(&env, "fold");
    let id = three_rounds_failing(&mut env);
    let tree = seed_commits(&env, &id);
    rerun_with(&mut env, &id, None);
    lint_exits(&mut env, &id, 1, 0, "");
    style_says(&mut env, &id, 1, "- withdraw a1/r1/lint-2\n");
    env.steps_until(&id, "the final checks", |t, _| {
        review_attempt(t).gate.is_some()
    });
    let t = env.ticket(&id);
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(checks_key(&t, 1), 0);
    env.steps_until(&id, "the attempt ending", |t, _| {
        !review_attempt(t).is_open()
    });
    let a = review_attempt(&env.ticket(&id));
    assert_eq!(a.state, AttemptState::Complete, "{:?}", a.state);
    assert_eq!(a.n, 2);
    assert!(
        a.rounds.iter().all(|r| r.head_after.is_none()),
        "no fix in this attempt"
    );
    assert_eq!(
        env.repo.lock().unwrap().replayed,
        [(
            tree,
            "root0000".to_owned(),
            vec![
                group(&["impl0001", "fix00001"], "A"),
                group(&["base0000", "fix00002"], "B"),
            ]
        )]
    );
    assert_eq!(a.head.as_deref(), Some("fold0001"));
}

/// Converged in round one at the implementer's head with its checks
/// reused: nothing to fold, so nothing is replayed.
#[test]
fn a_review_converging_at_round_one_with_fold_rewrites_nothing() {
    let mut env = Env::new();
    env.with_review_stage("ask");
    with_commits(&env, "fold");
    let id = at_review(&mut env);
    let tree = env.ticket(&id).lanes[0].worktree.clone();
    env.repo.lock().unwrap().commits.insert(
        tree,
        vec![
            Commit {
                sha: "impl0001".into(),
                parents: 1,
                message: "A".into(),
            },
            Commit {
                sha: "base0000".into(),
                parents: 1,
                message: "B".into(),
            },
        ],
    );
    lint_exits(&mut env, &id, 1, 0, "");
    style_says(&mut env, &id, 1, "No findings.");
    env.steps_until(&id, "the stage completing", |t, _| {
        review_attempt(t).state == AttemptState::Complete
    });
    let a = review_attempt(&env.ticket(&id));
    assert!(a.gate.is_none(), "implement's checks reused");
    assert_eq!(
        rewrite_of(&a),
        Some(Rewrite {
            mode: Commits::Fold,
            before: "base0000".into(),
            after: Some("base0000".into()),
            from: 2,
            to: 2,
            skipped: None,
            stale: Vec::new(),
            message: None,
            at_ms: 0,
        })
    );
    assert_eq!(a.head.as_deref(), Some("base0000"));
    let repo = env.repo.lock().unwrap();
    assert!(repo.replayed.is_empty());
    assert!(repo.head_sets.is_empty());
}

#[test]
fn a_park_and_a_resume_are_logged() {
    let (env, id, _) = parked_and_resumed();
    assert_in_order(
        &events_of(&env.data, &id),
        &["answered", "parking", "parked", "resumed"],
    );
}

/// `wait` over a runner a test steps between its looks: `pause` runs
/// one pass and moves the fake clock on.
fn wait_stepping(
    env: &mut Env,
    id: &str,
    what: dispatch::events::For,
    deadline: u64,
) -> dispatch::events::Waited {
    let data = env.data.clone();
    let env = std::cell::RefCell::new(env);
    dispatch::events::wait(
        &data,
        id,
        what,
        None,
        Some(deadline),
        &mut || env.borrow().now,
        &mut || {
            let mut env = env.borrow_mut();
            env.step();
            env.wait(250);
        },
    )
    .unwrap()
}

#[test]
fn wait_returns_the_decision_the_next_pass_raises() {
    let mut env = Env::new();
    let id = at_review_run(&mut env);
    let run = env.sb().runs[0].id.clone();
    env.sb().run_mut(&run).state = RunState::Converged;
    assert!(env.pending(&id).is_empty(), "nothing asked yet");
    let deadline = env.now + 10_000;
    let waited = wait_stepping(&mut env, &id, dispatch::events::For::Decision, deadline);
    let dispatch::events::Waited::Matched(e) = waited else {
        panic!("{waited:?}");
    };
    let finalize = env.pending(&id)[0].clone();
    assert_eq!(finalize.name, "finalize");
    assert_eq!(e.kind, dispatch::events::Kind::Decision);
    assert_eq!(e.decision.as_deref(), Some(finalize.id.as_str()));
    assert!(e.seq > 0, "read from the log");
    assert!(
        e.text.starts_with("finalize: The review of plan"),
        "{}",
        e.text
    );

    // Asked again with it pending, it answers from the record at once.
    let again = wait_stepping(&mut env, &id, dispatch::events::For::Decision, 0);
    assert_eq!(again, dispatch::events::Waited::Matched(e));
}

#[test]
fn wait_times_out_on_an_idle_ticket_and_ends_on_a_parked_one() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let deadline = env.now + 1_000;
    let waited = wait_stepping(&mut env, &id, dispatch::events::For::Stage, deadline);
    assert_eq!(waited, dispatch::events::Waited::TimedOut);

    let d = env.pending(&id)[0].id.clone();
    let now = env.tick();
    env.runner.decide(&id, &d, "park", None, now).unwrap();
    env.steps_until(&id, "parked", |t, _| {
        matches!(t.state, TicketState::Parked { .. })
    });
    let deadline = env.now + 1_000;
    let waited = wait_stepping(&mut env, &id, dispatch::events::For::Decision, deadline);
    let dispatch::events::Waited::Ended(e) = waited else {
        panic!("{waited:?}");
    };
    assert_eq!(e.kind, dispatch::events::Kind::Parked);
    // Waiting for anything on a parked ticket ends at once, as any other
    // wait does.
    let deadline = env.now + 1_000;
    let waited = wait_stepping(&mut env, &id, dispatch::events::For::Any, deadline);
    let dispatch::events::Waited::Ended(e) = waited else {
        panic!("{waited:?}");
    };
    assert_eq!(e.kind, dispatch::events::Kind::Parked);
}

#[test]
fn show_points_at_the_plan_the_notes_and_the_pr() {
    let mut env = Env::new();
    let id = at_ready(&mut env);
    env.pr_is(&id, "base0000", "open", Checks::Pending);
    env.recheck(&id);
    env.steps_until(&id, "the PR bound", |t, _| {
        t.attempts_of("ready")
            .last()
            .is_some_and(|a| a.pr.is_some())
    });
    let t = env.ticket(&id);
    let pipeline = env.runner.pipeline_of(&t).ok();
    let mut view = dispatch::serve::ticket_view(&t, pipeline.as_ref());
    assert_eq!(view.paths, dispatch_control::PathsView::default());
    view.paths = dispatch::serve::ticket_paths(&t, pipeline.as_ref());
    assert_eq!(
        view.paths.pr_url.as_deref(),
        Some("https://github.com/msull/switchboard/pull/7")
    );
    assert_eq!(view.paths.pr_head.as_deref(), Some("base0000"));
    assert_eq!(view.paths.plan, t.input("plan").cloned());
    assert!(view.paths.plan.is_some());
    assert_eq!(view.paths.notes, t.input("notes").cloned());
    let review = t.attempts_of("review").last().unwrap();
    assert_eq!(
        view.paths.round_file,
        None,
        "the reviewer wrote no round file beside {}",
        review.artifacts["plan"].display()
    );
    let lane = &view.lanes[0];
    assert_eq!(lane.base_sha.as_deref(), Some("base0000"));
    assert!(lane.head.is_some());
    let json = serde_json::to_string_pretty(&view).unwrap();
    let back: dispatch_control::TicketView = serde_json::from_str(&json).unwrap();
    assert_eq!(back, view);
}

#[test]
fn tail_reads_the_running_agents_screen_and_nothing_once_it_is_done() {
    let mut env = Env::new();
    let (id, implementer) = at_implement(&mut env);
    env.sb()
        .screens
        .insert(implementer.clone(), "$ cargo test\nok".into());
    let t = env.ticket(&id);
    let screens = env.runner.screens(&t, 20).unwrap();
    assert_eq!(
        screens,
        vec![(
            "implement/repo agent".to_owned(),
            "$ cargo test\nok".to_owned()
        )]
    );
    let asked = env.sb().calls.iter().any(|r| {
        matches!(&r.body, Body::SessionScreen { session, lines: Some(20) } if *session == implementer)
    });
    assert!(asked);

    env.sb().screen_unknown = true;
    let err = env.runner.screens(&t, 20).unwrap_err().to_string();
    assert!(err.contains("update the app"), "{err}");
    env.sb().screen_unknown = false;

    implementer_stops(&mut env, &id, &implementer);
    env.steps_until(&id, "the checks starting", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.gate.is_some())
    });
    let key = format!("{id}/implement/1");
    env.repo.lock().unwrap().check_exits.insert(key, 0);
    env.steps_until(&id, "the attempt complete", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.state == AttemptState::Complete)
    });
    let t = env.ticket(&id);
    assert!(env.runner.screens(&t, 20).unwrap().is_empty());
}

// --- a folded message that names what the review removed

/// The implementation commit's message, as the fake reports it from
/// now on.
fn seed_message(env: &Env, tree: &std::path::Path, message: &str) {
    message.clone_into(&mut env.repo.lock().unwrap().commits.get_mut(tree).unwrap()[0].message);
}

/// A fold at its final checks whose first commit says it adds
/// `old_name`, which the tree no longer has: the checks pass and the
/// `message` question is asked.
fn a_stale_fold(env: &mut Env) -> (String, PathBuf) {
    let (id, tree) = at_final_checks(env, Some("fold"));
    seed_message(env, &tree, "A\n\nAdds `old_name`.");
    env.repo
        .lock()
        .unwrap()
        .absent_names
        .insert(tree.clone(), vec!["old_name".into()]);
    let t = env.ticket(&id);
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(checks_key(&t, 3), 0);
    env.steps_until(&id, "the message question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "message")
    });
    (id, tree)
}

/// The texts of the ticket's events in the log, in order.
fn event_texts(env: &Env, id: &str) -> Vec<String> {
    dispatch::events::read_since(&dispatch::events::log_path(&env.data), 0)
        .unwrap()
        .into_iter()
        .filter(|e| e.ticket == id)
        .map(|e| e.text)
        .collect()
}

/// The text of `id`'s latest `refreshed` event.
fn refreshed_text(env: &Env, id: &str) -> String {
    dispatch::events::read_since(&dispatch::events::log_path(&env.data), 0)
        .unwrap()
        .into_iter()
        .rfind(|e| e.ticket == id && e.kind == dispatch::events::Kind::Refreshed)
        .map(|e| e.text)
        .unwrap_or_default()
}

/// The pending `message` question, answered `answer`.
fn answer_message(env: &mut Env, id: &str, answer: &str) -> Decision {
    let d = env
        .pending(id)
        .into_iter()
        .find(|d| d.name == "message")
        .unwrap_or_else(|| panic!("no message question: {:#?}", env.ticket(id)));
    let now = env.tick();
    env.runner.decide(id, &d.id, answer, None, now).unwrap();
    d
}

/// The folded history the fake reports from now on: the commit the
/// fixup folded into, with its stale message, and the tip.
fn folded_history(env: &Env, tree: &std::path::Path) {
    let c = |sha: &str, message: &str| Commit {
        sha: sha.to_owned(),
        parents: 1,
        message: message.to_owned(),
    };
    env.repo.lock().unwrap().commits.insert(
        tree.to_path_buf(),
        vec![c("folda001", "A\n\nAdds `old_name`."), c("fold0001", "B")],
    );
}

fn message_fix(t: &Ticket) -> dispatch::ticket::MessageFix {
    review_attempt(t)
        .rewrite
        .and_then(|r| r.message)
        .unwrap_or_else(|| panic!("no message record: {t:#?}"))
}

/// `rewrite` answered on a stale fold, the folded history seeded and
/// the rewriter running: its session and its output file.
fn a_rewriter_running(env: &mut Env) -> (String, PathBuf, String, PathBuf) {
    let (id, tree) = a_stale_fold(env);
    folded_history(env, &tree);
    answer_message(env, &id, "rewrite");
    env.steps_until(&id, "the rewriter", |t, _| {
        review_attempt(t)
            .rewrite
            .and_then(|r| r.message)
            .is_some_and(|m| m.session.is_some())
    });
    let t = env.ticket(&id);
    let session = message_fix(&t).session.unwrap();
    let out = review_attempt(&t).artifacts["message/folda00"].clone();
    (id, tree, session, out)
}

/// The issue's first test: the fold lands, and its message names a
/// name neither the commit nor the tree has, so the attempt stays open
/// with the branch at the folded head and asks.
#[test]
fn a_folded_message_naming_a_removed_name_asks() {
    let mut env = Env::new();
    let (id, tree) = a_stale_fold(&mut env);
    let t = env.ticket(&id);
    let a = review_attempt(&t);
    assert!(a.is_open(), "{:?}", a.state);
    assert_eq!(env.repo.lock().unwrap().heads[&tree], "fold0001");
    let d = env
        .pending(&id)
        .into_iter()
        .find(|d| d.name == "message")
        .unwrap();
    assert_eq!(d.options, ["rewrite", "accept", "park"]);
    assert!(d.question.contains("`old_name`"), "{}", d.question);
    assert!(d.question.contains("fold000~1"), "{}", d.question);
    let r = a.rewrite.unwrap();
    assert_eq!(r.after.as_deref(), Some("fold0001"));
    assert_eq!(
        r.stale,
        [dispatch::ticket::StaleMessage {
            index: 0,
            subject: "A".into(),
            names: vec!["old_name".into()],
        }]
    );
    assert!(
        t.attempts_of("inspect").next().is_none(),
        "nothing advanced"
    );
    let events = event_texts(&env, &id);
    assert!(
        events
            .iter()
            .any(|e| e == "folded message names `old_name`, which the tree does not have"),
        "{events:#?}"
    );
}

#[test]
fn a_folded_message_whose_names_all_exist_completes() {
    let mut env = Env::new();
    let (id, tree) = at_final_checks(&mut env, Some("fold"));
    seed_message(&env, &tree, "A\n\nAdds `old_name`.");
    let a = final_checks_pass(&mut env, &id);
    assert_eq!(a.state, AttemptState::Complete, "{:?}", a.state);
    assert_eq!(a.head.as_deref(), Some("fold0001"));
    assert!(a.rewrite.unwrap().stale.is_empty());
    assert!(
        env.ticket(&id)
            .decisions
            .iter()
            .all(|d| d.name != "message"),
        "nothing asked"
    );
}

/// The issue's third test: `rewrite` rewords the folded commit one for
/// one, the tree unchanged, and nothing is pushed.
#[test]
fn rewrite_rewords_the_folded_commit_and_keeps_the_tree() {
    let mut env = Env::new();
    let (id, tree, session, out) = a_rewriter_running(&mut env);
    let t = env.ticket(&id);
    let dir = out.parent().unwrap().to_path_buf();
    assert!(dir.ends_with("message"), "{}", dir.display());
    assert!(review_attempt(&t).artifacts["message/input"].is_file());
    let launch = env
        .sb()
        .calls
        .iter()
        .rev()
        .find_map(|r| match &r.body {
            Body::SessionNew { notes, launch, .. } if notes.contains("message rewriter") => {
                Some(launch.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(
        launch,
        switchboard_control::Launch::Argv(vec![
            "--allowedTools".into(),
            format!("Edit(//{}/**)", dir.display())
        ]),
        "the rewriter may write its files outside the tree"
    );
    assert!(
        t.ledger
            .iter()
            .any(|o| o.intent == "message" && o.kind == "session.new")
    );
    let prompt = last_prompt_of(&env, "implementer");
    assert!(prompt.contains("input.md"), "{prompt}");
    assert!(prompt.contains("Do not commit"), "{prompt}");
    env.repo
        .lock()
        .unwrap()
        .replay_heads
        .insert(tree.clone(), "word0001".into());
    env.finish(&session, &out, "A\n\nAdds `new_name`.\n");
    env.steps_until(&id, "the attempt completing", |t, _| {
        !review_attempt(t).is_open()
    });
    let t = env.ticket(&id);
    let a = review_attempt(&t);
    assert_eq!(a.state, AttemptState::Complete, "{:?}", a.state);
    assert_eq!(a.head.as_deref(), Some("word0001"));
    let m = message_fix(&t);
    assert_eq!(m.to.as_deref(), Some("word0001"));
    assert_eq!(m.from, "fold0001");
    assert_eq!(a.rewrite.unwrap().after.as_deref(), Some("word0001"));
    {
        let repo = env.repo.lock().unwrap();
        assert_eq!(repo.replayed.len(), 2);
        assert_eq!(
            repo.replayed[1],
            (
                tree.clone(),
                "root0000".to_owned(),
                vec![
                    group(&["folda001"], "A\n\nAdds `new_name`."),
                    group(&["fold0001"], "B"),
                ]
            )
        );
        assert_eq!(
            repo.head_sets.last().unwrap(),
            &(tree.clone(), "word0001".to_owned(), "fold0001".to_owned())
        );
        assert!(repo.pushed.is_empty(), "nothing pushed");
    }
    assert!(env.sb().killed.contains(&session), "the rewriter killed");
    let summary = std::fs::read_to_string(&a.artifacts["summary"]).unwrap();
    assert!(
        summary.contains("The folded message named `old_name`, which neither the commit nor the tree has; rewritten, fold000 → word000."),
        "{summary}"
    );
    let events = event_texts(&env, &id);
    assert!(
        events
            .iter()
            .any(|e| e == "message rewritten fold000 → word000"),
        "{events:#?}"
    );
}

/// A rewording that lands and still names the removed name asks again;
/// `accept` keeps the reworded message the branch now carries, and says
/// so rather than that the folded one was kept.
#[test]
fn accept_after_a_rewording_that_still_names_it_keeps_the_rewording() {
    let mut env = Env::new();
    let (id, tree, session, out) = a_rewriter_running(&mut env);
    env.repo
        .lock()
        .unwrap()
        .replay_heads
        .insert(tree.clone(), "word0001".into());
    env.finish(&session, &out, "A\n\nStill adds `old_name`.\n");
    let d = answer_message(&mut env, &id, "accept");
    assert_eq!(d.options, ["accept", "park"]);
    assert!(
        d.question.contains("still names `old_name`"),
        "{}",
        d.question
    );
    env.steps_until(&id, "the attempt completing", |t, _| {
        !review_attempt(t).is_open()
    });
    let t = env.ticket(&id);
    let a = review_attempt(&t);
    assert_eq!(a.state, AttemptState::Complete, "{:?}", a.state);
    assert_eq!(a.head.as_deref(), Some("word0001"));
    assert_eq!(env.repo.lock().unwrap().heads[&tree], "word0001");
    let summary = std::fs::read_to_string(&a.artifacts["summary"]).unwrap();
    assert!(
        summary.contains("rewritten, fold000 → word000, but the rewritten message still names `old_name`; kept as rewritten."),
        "{summary}"
    );
    let events = event_texts(&env, &id);
    assert!(
        events.iter().any(|e| e == "message kept as rewritten"),
        "{events:#?}"
    );
}

#[test]
fn accept_keeps_the_folded_message() {
    let mut env = Env::new();
    let (id, tree) = a_stale_fold(&mut env);
    answer_message(&mut env, &id, "accept");
    env.steps_until(&id, "the attempt completing", |t, _| {
        !review_attempt(t).is_open()
    });
    let t = env.ticket(&id);
    let a = review_attempt(&t);
    assert_eq!(a.state, AttemptState::Complete, "{:?}", a.state);
    assert_eq!(a.head.as_deref(), Some("fold0001"));
    assert_eq!(message_fix(&t).answer, "accept");
    assert_eq!(env.repo.lock().unwrap().heads[&tree], "fold0001");
    assert_eq!(env.repo.lock().unwrap().replayed.len(), 1);
    let summary = std::fs::read_to_string(&a.artifacts["summary"]).unwrap();
    assert!(
        summary.contains("The folded message named `old_name`, which neither the commit nor the tree has; kept as written."),
        "{summary}"
    );
    assert!(
        event_texts(&env, &id)
            .iter()
            .any(|e| e == "message kept as written")
    );
}

#[test]
fn a_message_rewrite_that_fails_asks_accept_or_park() {
    let mut env = Env::new();
    let (id, tree, session, _) = a_rewriter_running(&mut env);
    let first = env
        .ticket(&id)
        .decisions
        .iter()
        .find(|d| d.name == "message")
        .unwrap()
        .id
        .clone();
    let now = env.now;
    env.sb().stop(&session, now);
    env.idle_past_grace();
    env.steps_until(&id, "the question again", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == "message" && d.id != first)
    });
    let t = env.ticket(&id);
    assert!(review_attempt(&t).is_open());
    let failed = message_fix(&t).failed.unwrap();
    assert!(failed.contains("without writing"), "{failed}");
    assert!(env.sb().killed.contains(&session), "the rewriter killed");
    let d = answer_message(&mut env, &id, "accept");
    assert_eq!(d.options, ["accept", "park"]);
    assert!(d.question.contains("without writing"), "{}", d.question);
    env.steps_until(&id, "the attempt completing", |t, _| {
        !review_attempt(t).is_open()
    });
    let t = env.ticket(&id);
    let a = review_attempt(&t);
    assert_eq!(a.state, AttemptState::Complete, "{:?}", a.state);
    assert_eq!(a.head.as_deref(), Some("fold0001"));
    let repo = env.repo.lock().unwrap();
    assert_eq!(repo.heads[&tree], "fold0001");
    assert_eq!(repo.replayed.len(), 1, "nothing replayed for the messages");
}

/// A completed rewording written back as a restart would find it had
/// the runner stopped after `moving` was saved; the fake's head set to
/// `head`, then the restart.
fn restarted_mid_rewording(env: &mut Env, id: &str, head: &str, tree: &std::path::Path) {
    let mut t = env.ticket(id);
    let a = t
        .attempts
        .iter_mut()
        .rev()
        .find(|a| a.stage == "review-code")
        .unwrap();
    a.state = AttemptState::Running;
    a.head = None;
    a.ended_ms = None;
    let r = a.rewrite.as_mut().unwrap();
    r.after = Some("fold0001".into());
    let m = r.message.as_mut().unwrap();
    m.to = None;
    m.moving = true;
    t.stage = 5;
    env.runner.save_ticket(&mut t, env.now).unwrap();
    env.repo
        .lock()
        .unwrap()
        .heads
        .insert(tree.to_path_buf(), head.to_owned());
    env.restart();
}

/// The rewording ran to completion; the steps that follow.
fn reworded(env: &mut Env) -> (String, PathBuf) {
    let (id, tree, session, out) = a_rewriter_running(env);
    env.repo
        .lock()
        .unwrap()
        .replay_heads
        .insert(tree.clone(), "word0001".into());
    env.finish(&session, &out, "A\n\nAdds `new_name`.\n");
    env.steps_until(&id, "the attempt completing", |t, _| {
        !review_attempt(t).is_open()
    });
    (id, tree)
}

#[test]
fn a_restart_after_the_message_move_completes_at_the_reworded_head() {
    let mut env = Env::new();
    let (id, tree) = reworded(&mut env);
    restarted_mid_rewording(&mut env, &id, "word0001", &tree);
    env.steps_until(&id, "the stage completing", |t, _| {
        review_attempt(t).state == AttemptState::Complete
    });
    let t = env.ticket(&id);
    let a = review_attempt(&t);
    assert_eq!(a.head.as_deref(), Some("word0001"));
    assert_eq!(message_fix(&t).to.as_deref(), Some("word0001"));
    let repo = env.repo.lock().unwrap();
    assert_eq!(repo.replayed.len(), 2, "no third replay");
}

#[test]
fn a_restart_before_the_message_move_swaps_again() {
    let mut env = Env::new();
    let (id, tree) = reworded(&mut env);
    restarted_mid_rewording(&mut env, &id, "fold0001", &tree);
    env.steps_until(&id, "the stage completing", |t, _| {
        review_attempt(t).state == AttemptState::Complete
    });
    let t = env.ticket(&id);
    assert_eq!(review_attempt(&t).head.as_deref(), Some("word0001"));
    let repo = env.repo.lock().unwrap();
    assert_eq!(repo.replayed.len(), 3, "replayed once more");
    assert_eq!(repo.replayed[2].2, repo.replayed[1].2);
    assert_eq!(repo.heads[&tree], "word0001");
}

#[test]
fn a_rewriter_reply_lost_is_found_again_and_nothing_launches_twice() {
    let mut env = Env::new();
    let (id, tree) = a_stale_fold(&mut env);
    folded_history(&env, &tree);
    answer_message(&mut env, &id, "rewrite");
    let before = env.sb().kinds_called("session.new");
    env.sb().drop_reply_for = Some("session.new".into());
    env.step();
    let m = message_fix(&env.ticket(&id));
    assert!(m.launched && m.session.is_none(), "{m:#?}");
    env.step();
    assert_eq!(env.sb().kinds_called("session.new"), before + 1);
    env.restart();
    let session = message_fix(&env.ticket(&id)).session;
    assert!(session.is_some(), "the reply applied by recovery");
    env.step();
    env.step();
    assert_eq!(env.sb().kinds_called("session.new"), before + 1);
    assert!(env.ticket(&id).processes.contains(&session.unwrap()));
}

#[test]
fn a_restart_with_the_message_question_pending_asks_nothing_new() {
    let mut env = Env::new();
    let (id, _) = a_stale_fold(&mut env);
    env.restart();
    env.step();
    env.step();
    let asked: Vec<Decision> = env
        .ticket(&id)
        .decisions
        .into_iter()
        .filter(|d| d.name == "message")
        .collect();
    assert_eq!(asked.len(), 1, "{asked:#?}");
    assert!(asked[0].pending());
}

#[test]
fn a_published_branch_an_identity_fold_and_keep_ask_nothing() {
    // Published: the history is left alone, and so are its messages.
    let mut env = Env::new();
    let (id, tree) = at_final_checks(&mut env, Some("fold"));
    seed_message(&env, &tree, "A\n\nAdds `old_name`.");
    {
        let mut repo = env.repo.lock().unwrap();
        repo.absent_names
            .insert(tree.clone(), vec!["old_name".into()]);
        repo.published
            .push((tree.clone(), "root0000".into(), "fix00002".into()));
    }
    let a = final_checks_pass(&mut env, &id);
    assert_eq!(a.state, AttemptState::Complete, "{:?}", a.state);
    assert!(a.rewrite.unwrap().stale.is_empty());
    // Identity: nothing folds.
    let mut env = Env::new();
    let (id, tree) = at_final_checks(&mut env, Some("fold"));
    {
        let mut repo = env.repo.lock().unwrap();
        repo.commits.insert(
            tree.clone(),
            vec![Commit {
                sha: "impl0001".into(),
                parents: 1,
                message: "A\n\nAdds `old_name`.".into(),
            }],
        );
        repo.absent_names
            .insert(tree.clone(), vec!["old_name".into()]);
    }
    let a = final_checks_pass(&mut env, &id);
    assert_eq!(a.state, AttemptState::Complete, "{:?}", a.state);
    assert!(env.repo.lock().unwrap().replayed.is_empty());
    // Keep: no rewrite, no check.
    let mut env = Env::new();
    let (id, tree) = at_final_checks(&mut env, Some("keep"));
    seed_message(&env, &tree, "A\n\nAdds `old_name`.");
    env.repo
        .lock()
        .unwrap()
        .absent_names
        .insert(tree.clone(), vec!["old_name".into()]);
    let a = final_checks_pass(&mut env, &id);
    assert_eq!(a.state, AttemptState::Complete, "{:?}", a.state);
    assert_eq!(a.rewrite, None);
    assert!(
        env.ticket(&id)
            .decisions
            .iter()
            .all(|d| d.name != "message")
    );
}

#[test]
fn a_message_answer_after_the_branch_moved_parks() {
    let mut env = Env::new();
    let (id, tree) = a_stale_fold(&mut env);
    env.repo
        .lock()
        .unwrap()
        .heads
        .insert(tree.clone(), "other001".into());
    answer_message(&mut env, &id, "accept");
    env.steps_until(&id, "the park", |t, _| {
        matches!(t.state, TicketState::Parked { .. })
    });
    let TicketState::Parked { reason, .. } = env.ticket(&id).state else {
        unreachable!()
    };
    assert!(
        reason.contains("fold0001") && reason.contains("other001"),
        "{reason}"
    );
}

/// The fixer followed its guidance and wrote a `squash!` body about
/// the rename: the folded message carries both, and nothing is asked.
#[test]
fn a_squash_fix_that_renames_the_name_asks_nothing() {
    let mut env = Env::new();
    let (id, tree) = at_final_checks(&mut env, Some("fold"));
    {
        let mut repo = env.repo.lock().unwrap();
        let commits = repo.commits.get_mut(&tree).unwrap();
        commits[0].message = "A\n\nAdds `old_name`.".into();
        commits[2].message = "squash! A\n\nRenames `old_name` to `new_name`.".into();
        repo.absent_names
            .insert(tree.clone(), vec!["old_name".into()]);
    }
    let a = final_checks_pass(&mut env, &id);
    assert_eq!(a.state, AttemptState::Complete, "{:?}", a.state);
    assert_eq!(a.head.as_deref(), Some("fold0001"));
    assert!(a.rewrite.unwrap().stale.is_empty());
    assert!(
        env.ticket(&id)
            .decisions
            .iter()
            .all(|d| d.name != "message"),
        "nothing asked"
    );
}

#[test]
fn the_message_dial_at_auto_keeps_the_message_without_asking() {
    let mut env = Env::new();
    with_three_rounds(&env);
    with_commits(&env, "fold");
    // The pipeline is read as the ticket is taken.
    let path = env.data.pipeline(PROJECT);
    let text = std::fs::read_to_string(&path).unwrap();
    let text = text.replace(
        "finalize = \"ask\",",
        "finalize = \"ask\", message = \"auto\",",
    );
    assert!(text.contains("message = \"auto\""));
    std::fs::write(path, text).unwrap();
    let id = at_review(&mut env);
    let tree = seed_commits(&env, &id);
    two_fixes_then_the_final_checks(&mut env, &id);
    seed_message(&env, &tree, "A\n\nAdds `old_name`.");
    env.repo
        .lock()
        .unwrap()
        .absent_names
        .insert(tree.clone(), vec!["old_name".into()]);
    let a = final_checks_pass(&mut env, &id);
    assert_eq!(a.state, AttemptState::Complete, "{:?}", a.state);
    assert_eq!(a.rewrite.unwrap().message.unwrap().answer, "accept");
    assert!(
        env.ticket(&id)
            .decisions
            .iter()
            .all(|d| d.name != "message")
    );
}

// --- restart

/// The project's live pipeline file with `from` replaced by `to`; every
/// ticket's copy is left as it was.
fn live_edit(env: &Env, from: &str, to: &str) {
    let path = env.data.pipeline(PROJECT);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(from), "{from} is not in the live file");
    std::fs::write(path, text.replacen(from, to, 1)).unwrap();
}

const SETUP: &str = r#"setup = ["cargo", "fetch", "--locked"]"#;
const FIXED_SETUP: &str = r#"setup = ["cargo", "fetch"]"#;
const GATE_ARGV: &str = r#"argv = ["sh", "-c", "cargo test"]"#;
const FIXED_ARGV: &str = r#"argv = ["sh", "-c", "cargo nextest run"]"#;

fn restart_at(env: &mut Env, id: &str, stage: Option<&str>) -> Ticket {
    let now = env.tick();
    env.runner.restart(id, stage, now).unwrap()
}

fn restart_refused(env: &mut Env, id: &str, stage: Option<&str>) -> String {
    let now = env.tick();
    let e = env.runner.restart(id, stage, now).unwrap_err();
    format!("{e:#}")
}

/// The one pending `rerun` decision about `stage`'s attempt `n`.
fn rerun_about(env: &Env, id: &str, stage: &str, n: u32) -> Decision {
    env.pending(id)
        .into_iter()
        .find(|d| d.name == "rerun" && d.attempt == Some((stage.to_owned(), n)))
        .unwrap_or_else(|| {
            panic!(
                "no rerun question about {stage}/{n}: {:#?}",
                env.pending(id)
            )
        })
}

fn answer(env: &mut Env, id: &str, d: &Decision, with: &str) {
    let now = env.tick();
    env.runner.decide(id, &d.id, with, None, now).unwrap();
}

/// Runs of `argv`, confined or not.
fn setups_run(env: &Env, argv: &[&str]) -> usize {
    let repo = env.repo.lock().unwrap();
    let plain = repo.ran.iter().filter(|(_, a)| a == argv).count();
    let confined = repo
        .ran_confined
        .iter()
        .filter(|(_, a, _)| a == argv)
        .count();
    plain + confined
}

/// `implement`'s checks started after the implementer stopped; the key
/// they run under.
fn implement_checking(env: &mut Env) -> (String, String) {
    let (id, implementer) = at_implement(env);
    implementer_stops(env, &id, &implementer);
    env.steps_until(&id, "the checks starting", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.gate.is_some())
    });
    let key = format!("{id}/implement/1");
    (id, key)
}

/// `implement`'s checks exited 127; the next start of the same checks
/// runs until a test says otherwise.
fn implement_checks_127(env: &mut Env) -> String {
    let (id, key) = implement_checking(env);
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(key.clone(), 127);
    env.steps_until(&id, "the failure", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| matches!(a.state, AttemptState::Failed { .. }))
    });
    env.repo.lock().unwrap().check_exits.remove(&key);
    id
}

/// The 127 the issue was about: the pipeline is fixed, the ticket
/// restarted at its stage, and `check` runs the new setup and the new
/// gate on the same attempt, with no agent and no bring-up.
#[test]
fn a_restart_at_the_current_stage_runs_the_fixed_setup_and_gate_on_check() {
    let mut env = Env::new();
    let id = implement_checks_127(&mut env);
    let d = rerun_about(&env, &id, "implement", 1);
    assert_eq!(d.options, vec!["rerun", "check", "park"]);
    assert!(d.question.contains("dispatch restart"), "{}", d.question);
    live_edit(&env, SETUP, FIXED_SETUP);
    live_edit(&env, GATE_ARGV, FIXED_ARGV);
    let first = env.ticket(&id).pipeline_file.clone();
    let first_text = std::fs::read_to_string(&first).unwrap();
    let (fetched, rebased) = {
        let repo = env.repo.lock().unwrap();
        (repo.fetched.len(), repo.rebased.len())
    };
    let sessions = env.sb().sessions.len();
    let t = restart_at(&mut env, &id, None);
    assert!(t.active(), "{:?}", t.state);
    assert_eq!(t.pipeline_file, first.with_file_name("pipeline.2.toml"));
    assert_eq!(std::fs::read_to_string(&first).unwrap(), first_text);
    assert!(
        std::fs::read_to_string(&t.pipeline_file)
            .unwrap()
            .contains("nextest")
    );
    let r = &t.restarts[0];
    assert_eq!((r.from.as_str(), r.to.as_str()), ("implement", "implement"));
    assert_eq!(r.setup_again, vec!["repo"]);
    assert!(r.reset.is_empty() && r.discarded.is_empty());
    assert!(!t.lanes[0].setup_done);
    assert_eq!(t.refreshed_stage, Some(t.stage), "no bring-up on re-entry");
    assert_eq!(t.restart, None);
    let kinds = events_of(&env.data, &id);
    assert_eq!(
        kinds.last().map(String::as_str),
        Some("restarted"),
        "{kinds:?}"
    );
    env.step();
    let d = rerun_about(&env, &id, "implement", 1);
    assert_eq!(d.options, vec!["rerun", "check", "park"]);
    assert_eq!(
        setups_run(&env, &["cargo", "fetch"]),
        0,
        "not before the answer"
    );
    answer(&mut env, &id, &d, "check");
    env.steps_until(&id, "the new checks", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.is_open() && a.gate.is_some())
    });
    assert_eq!(setups_run(&env, &["cargo", "fetch"]), 1);
    let repo = env.repo.lock().unwrap();
    assert_eq!(
        repo.checks.last().unwrap().argv,
        vec!["sh", "-c", "cargo nextest run"]
    );
    assert_eq!(
        (repo.fetched.len(), repo.rebased.len()),
        (fetched, rebased),
        "no bring-up between the restart and the checks"
    );
    drop(repo);
    assert_eq!(env.sb().sessions.len(), sessions, "no agent launched");
    let t = env.ticket(&id);
    assert_eq!(t.attempts_of("implement").count(), 1, "the same attempt");
    assert!(t.lanes[0].setup_done);
}

/// A code review's checks exit 127 after a fix: the fixed setup runs
/// before the review's checks start again on `check`.
#[test]
fn a_restart_at_a_review_stage_runs_the_fixed_setup_before_its_checks() {
    let (mut env, id, _) = findings_asked_and_fixed();
    let t = env.ticket(&id);
    let round = &review_attempt(&t).rounds[0];
    let fixer = round.implementer.clone().unwrap();
    let response = round.response.clone().unwrap();
    env.repo
        .lock()
        .unwrap()
        .heads
        .insert(t.lanes[0].worktree.clone(), "fix00001".into());
    env.finish(
        &fixer,
        &response,
        "- r1/style-1: fixed renamed it\n- r1/style-2: fixed added the line\n- r1/lint-1: fixed removed it\n",
    );
    env.steps_until(&id, "the checks after the fix", |t, _| {
        review_attempt(t).gate.is_some()
    });
    let key = checks_key(&env.ticket(&id), 1);
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(key.clone(), 127);
    env.steps_until(&id, "the review failing at its checks", |t, _| {
        matches!(review_attempt(t).state, AttemptState::Failed { .. })
    });
    env.repo.lock().unwrap().check_exits.remove(&key);
    let n = review_attempt(&env.ticket(&id)).n;
    assert_eq!(
        rerun_about(&env, &id, "review-code", n).options,
        vec!["rerun", "check", "park"]
    );
    live_edit(&env, SETUP, FIXED_SETUP);
    assert!(restart_at(&mut env, &id, None).active());
    env.step();
    let d = rerun_about(&env, &id, "review-code", n);
    assert_eq!(d.options, vec!["rerun", "check", "park"]);
    let checks_before = env.repo.lock().unwrap().checks.len();
    answer(&mut env, &id, &d, "check");
    env.steps_until(&id, "the review's checks again", |t, _| {
        let a = review_attempt(t);
        a.is_open() && a.gate.is_some()
    });
    assert_eq!(setups_run(&env, &["cargo", "fetch"]), 1);
    let repo = env.repo.lock().unwrap();
    assert_eq!(repo.checks.len(), checks_before + 1);
    assert_eq!(repo.checks.last().unwrap().key, key);
}

/// The fixed setup still fails: the checks fail with `check` offered
/// again, and the ticket stays active rather than parking.
#[test]
fn a_setup_that_fails_on_check_fails_the_checks_and_keeps_check() {
    let mut env = Env::new();
    let id = implement_checks_127(&mut env);
    live_edit(&env, SETUP, FIXED_SETUP);
    restart_at(&mut env, &id, None);
    env.step();
    let d = rerun_about(&env, &id, "implement", 1);
    env.repo.lock().unwrap().fail_run = Some("cargo: no such command".into());
    answer(&mut env, &id, &d, "check");
    env.steps_until(&id, "the checks failing on the setup", |t, _| {
        t.pending_decisions().iter().any(|x| x.name == "rerun")
    });
    let t = env.ticket(&id);
    assert!(t.active(), "{:?}", t.state);
    let a = t.attempts_of("implement").last().unwrap();
    let AttemptState::Failed { reason } = &a.state else {
        panic!("{a:#?}")
    };
    assert!(reason.contains("setup failed"), "{reason}");
    assert!(!t.lanes[0].setup_done);
    let d = rerun_about(&env, &id, "implement", 1);
    assert_eq!(d.options, vec!["rerun", "check", "park"]);
    // Fixed for real this time: `check` runs it and starts the checks.
    answer(&mut env, &id, &d, "check");
    env.steps_until(&id, "the checks", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.is_open() && a.gate.is_some())
    });
    assert!(env.ticket(&id).lanes[0].setup_done);
}

/// Checks the park stops mid-run: the attempt is cancelled, and its
/// question offers `check`, which runs them again with no agent.
#[test]
fn a_cancelled_attempt_mid_check_is_offered_check() {
    let mut env = Env::new();
    let (id, key) = implement_checking(&mut env);
    let started = env
        .ticket(&id)
        .attempts_of("implement")
        .last()
        .unwrap()
        .gate
        .clone()
        .unwrap()
        .started_ms;
    let t = restart_at(&mut env, &id, None);
    assert!(t.active(), "{:?}", t.state);
    let a = t.attempts_of("implement").last().unwrap();
    assert!(
        matches!(&a.state, AttemptState::Cancelled { .. }) && a.failed_at_checks,
        "{a:#?}"
    );
    env.step();
    let d = rerun_about(&env, &id, "implement", 1);
    assert_eq!(d.options, vec!["rerun", "check", "park"]);
    answer(&mut env, &id, &d, "check");
    env.steps_until(&id, "the checks again", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.gate.is_some() && a.is_open())
    });
    {
        // The fake forgets a killed check, so the one listed is the new one.
        let repo = env.repo.lock().unwrap();
        assert_eq!(repo.killed_checks, vec![key.clone()]);
        assert_eq!(repo.checks.len(), 1);
        assert_eq!(repo.checks[0].key, key);
    }
    let gate = env
        .ticket(&id)
        .attempts_of("implement")
        .last()
        .unwrap()
        .gate
        .clone()
        .unwrap();
    assert!(gate.started_ms > started, "a new run of the checks");
    assert_eq!(env.sb().sessions_named("implementer").len(), 1);
}

/// Checks that ignore TERM hold the restart: the ticket stays parking
/// with the intent until they are gone, then the restart applies.
#[test]
fn a_restart_waits_for_running_checks_then_applies() {
    let mut env = Env::new();
    let (id, key) = implement_checking(&mut env);
    env.repo.lock().unwrap().stubborn_checks.push(key);
    let t = restart_at(&mut env, &id, None);
    assert!(
        matches!(t.state, TicketState::Parking { .. }),
        "{:?}",
        t.state
    );
    assert!(t.restart.is_some());
    env.step();
    env.step();
    let t = env.ticket(&id);
    assert!(
        matches!(t.state, TicketState::Parking { .. }),
        "{:?}",
        t.state
    );
    assert!(t.restart.is_some() && t.restarts.is_empty());
    env.wait(STOP_LIMIT_MS);
    env.steps_until(&id, "the restart applied", |t, _| t.active());
    let t = env.ticket(&id);
    assert_eq!(t.restarts.len(), 1);
    assert_eq!(t.restart, None);
}

/// A supervisor's restart parks as the supervisor: the `Parking` state
/// and its event say who, until the restart applies.
#[test]
fn a_supervisors_restart_parks_as_the_supervisor() {
    let mut env = Env::new();
    let (id, key) = implement_checking(&mut env);
    env.repo.lock().unwrap().stubborn_checks.push(key);
    env.runner.actor = Some(dispatch::scheduler::BY_SUPERVISOR.into());
    let t = restart_at(&mut env, &id, None);
    env.runner.actor = None;
    assert!(
        matches!(t.state, TicketState::Parking { .. }),
        "{:?}",
        t.state
    );
    assert_eq!(
        t.state_by.as_deref(),
        Some(dispatch::scheduler::BY_SUPERVISOR)
    );
    let events = dispatch::events::read_since(&dispatch::events::log_path(&env.data), 0).unwrap();
    let parking = events
        .iter()
        .find(|e| e.ticket == id && e.kind == dispatch::events::Kind::Parking)
        .expect("a parking event");
    assert_eq!(
        parking.actor.as_deref(),
        Some(dispatch::scheduler::BY_SUPERVISOR)
    );
    env.wait(STOP_LIMIT_MS);
    env.steps_until(&id, "the restart applied", |t, _| t.active());
    let t = env.ticket(&id);
    assert_eq!(t.restarts.len(), 1);
    assert_eq!(t.state_by, None);
}

/// A runner that restarts while a restart waits on its checks carries
/// it on from the record.
#[test]
fn a_runner_restart_mid_restart_carries_it_on() {
    let mut env = Env::new();
    let (id, key) = implement_checking(&mut env);
    env.repo.lock().unwrap().stubborn_checks.push(key);
    restart_at(&mut env, &id, None);
    env.restart();
    env.step();
    assert!(env.ticket(&id).restart.is_some());
    env.wait(STOP_LIMIT_MS);
    env.steps_until(&id, "the restart applied", |t, _| t.active());
    assert_eq!(env.ticket(&id).restarts.len(), 1);
}

/// At a gate-only stage the watch opens again under the new copy, with
/// nothing launched.
#[test]
fn a_plain_restart_at_a_gate_only_stage_opens_the_gate_again() {
    let mut env = Env::new();
    let id = at_ready(&mut env);
    let sessions = env.sb().sessions.len();
    let t = restart_at(&mut env, &id, None);
    assert!(t.active(), "{:?}", t.state);
    let ready = t.attempts_of("ready").last().unwrap();
    assert!(matches!(ready.state, AttemptState::Cancelled { .. }));
    env.steps_until(&id, "a new ready attempt", |t, _| {
        t.attempts_of("ready").last().is_some_and(|a| a.n == 2)
    });
    let t = env.ticket(&id);
    let a = t.attempts_of("ready").last().unwrap();
    assert_eq!(a.kind, AttemptKind::GateOnly);
    assert!(a.is_open());
    assert_eq!(env.sb().sessions.len(), sessions, "nothing launched");
}

/// The failures of the copy a restart replaced do not count against the
/// new one's `max_reruns`.
#[test]
fn failures_under_the_old_copy_do_not_count_toward_max_reruns() {
    let mut env = Env::new();
    let (id, key) = implement_checking(&mut env);
    with_max_reruns(&env, &env.ticket(&id), 1);
    env.repo.lock().unwrap().check_exits.insert(key, 1);
    env.steps_until(&id, "the first failure", |t, _| {
        !t.pending_decisions().is_empty()
    });
    restart_at(&mut env, &id, None);
    env.step();
    let d = rerun_about(&env, &id, "implement", 1);
    answer(&mut env, &id, &d, "rerun");
    env.steps_until(&id, "a second implementer", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.n == 2 && a.is_open())
    });
    let second = session_of(&env.ticket(&id), "implement");
    implementer_stops(&mut env, &id, &second);
    env.steps_until(&id, "the second checks", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.gate.is_some())
    });
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(format!("{id}/implement/2"), 1);
    env.steps_until(&id, "the second failure", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| matches!(a.state, AttemptState::Failed { .. }))
    });
    let t = env.ticket(&id);
    assert!(t.active(), "asked, not parked: {:?}", t.state);
    rerun_about(&env, &id, "implement", 2);
}

/// An attempt carried into the new copy by `check` counts when it fails
/// there: its failure and a rerun's make two, past a `max_reruns` of one.
#[test]
fn a_checked_attempt_failing_under_the_new_copy_counts_toward_max_reruns() {
    let mut env = Env::new();
    let (id, key) = implement_checking(&mut env);
    with_max_reruns(&env, &env.ticket(&id), 1);
    env.repo.lock().unwrap().check_exits.insert(key, 1);
    env.steps_until(&id, "the first failure", |t, _| {
        !t.pending_decisions().is_empty()
    });
    restart_at(&mut env, &id, None);
    env.step();
    let d = rerun_about(&env, &id, "implement", 1);
    answer(&mut env, &id, &d, "check");
    env.steps_until(&id, "the checks failing again", |t, _| {
        t.pending_decisions().iter().any(|q| q.id != d.id)
    });
    let d = rerun_about(&env, &id, "implement", 1);
    answer(&mut env, &id, &d, "rerun");
    env.steps_until(&id, "a second implementer", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.n == 2 && a.is_open())
    });
    let second = session_of(&env.ticket(&id), "implement");
    implementer_stops(&mut env, &id, &second);
    env.steps_until(&id, "the second checks", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.gate.is_some())
    });
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(format!("{id}/implement/2"), 1);
    env.steps_until(&id, "parked past max_reruns", |t, _| {
        matches!(t.state, TicketState::Parked { .. })
    });
    let t = env.ticket(&id);
    assert!(
        matches!(&t.state, TicketState::Parked { reason } if reason.contains("failed 2 times")),
        "{:?}",
        t.state
    );
}

/// Refusals: a closed ticket, a stage after the current one, and a live
/// file that names a stage the ticket never ran before the target.
#[test]
fn a_restart_is_refused_for_a_closed_ticket() {
    let mut env = Env::new();
    let id = env.take(30).id;
    let now = env.tick();
    env.runner.close_by_hand(&id, None, now).unwrap();
    let e = restart_refused(&mut env, &id, None);
    assert!(e.contains("retake"), "{e}");
}

/// A ticket still parking for its own reasons is refused: the park
/// sequence owns it until it reads parked.
#[test]
fn a_restart_is_refused_while_still_parking() {
    let mut env = Env::new();
    let (id, _) = at_implement(&mut env);
    let mut t = env.ticket(&id);
    t.state = TicketState::Parking {
        reason: "by hand".into(),
    };
    env.runner.save_ticket(&mut t, env.now).unwrap();
    let before = env.ticket(&id);
    let e = restart_refused(&mut env, &id, None);
    assert!(e.contains("still parking"), "{e}");
    assert_eq!(env.ticket(&id), before, "nothing written");
}

#[test]
fn a_restart_is_refused_for_a_stage_after_the_current_one() {
    let mut env = Env::new();
    let (id, _) = at_implement(&mut env);
    let before = env.ticket(&id);
    let e = restart_refused(&mut env, &id, Some("inspect"));
    assert!(e.contains("comes after"), "{e}");
    let e = restart_refused(&mut env, &id, Some("nowhere"));
    assert!(e.contains("not a stage"), "{e}");
    live_edit(
        &env,
        "[[stages]]\nname = \"implement\"",
        "[[stages]]\nname = \"design\"\noperator = \"planner\"\ncontext = \"each\"\nwrites = [\"design\"]\nprompt = \"Design to {design}.\"\n\n[[stages]]\nname = \"implement\"",
    );
    let e = restart_refused(&mut env, &id, None);
    assert!(e.contains("adds design before implement"), "{e}");
    assert_eq!(env.ticket(&id), before, "nothing written");
}

/// A live file that renames the current stage: the stage in its place is
/// live-only and the stage after it is later than the current one, so
/// the refusal names the current stage as missing, not as passed.
#[test]
fn a_restart_at_a_renamed_current_stage_says_the_live_file_lacks_it() {
    let mut env = Env::new();
    let (id, _) = at_implement(&mut env);
    let before = env.ticket(&id);
    live_edit(
        &env,
        "[[stages]]\nname = \"implement\"",
        "[[stages]]\nname = \"build\"",
    );
    let e = restart_refused(&mut env, &id, Some("build"));
    assert!(
        e.contains("the live pipeline has no stage implement"),
        "{e}"
    );
    assert!(!e.contains("comes after"), "{e}");
    assert_eq!(env.ticket(&id), before, "nothing written");
}

/// Two commits the fake reports beyond `dir`'s base.
fn seed_moved(env: &Env, dir: &std::path::Path) {
    let c = |sha: &str| Commit {
        sha: sha.into(),
        parents: 1,
        message: "work".into(),
    };
    env.repo
        .lock()
        .unwrap()
        .commits
        .insert(dir.to_path_buf(), vec![c("work0001"), c("work0002")]);
}

/// A ticket taken before entries were recorded: `at_finalize` with
/// `entered` cleared.
fn at_finalize_without_entries(env: &mut Env) -> String {
    let id = at_finalize(env);
    let mut t = env.ticket(&id);
    t.entered.clear();
    dispatch::store::write_ticket(&env.data.ticket_file(&id), &t).unwrap();
    id
}

/// No head is recorded for `plan`, but no branch has moved from its
/// base: the restart goes ahead without a reset, records the heads as
/// it enters `plan`, and the rerun runs the edited plan prompt.
#[test]
fn a_ranged_restart_without_heads_proceeds_when_no_branch_moved() {
    let mut env = Env::new();
    let id = at_finalize_without_entries(&mut env);
    let chosen: Vec<bool> = env.ticket(&id).lanes.iter().map(|l| l.chosen).collect();
    live_edit(
        &env,
        "Using {inputs.notes}, plan",
        "Using {inputs.notes}, carefully plan",
    );
    let planners = env.sb().sessions_named("planner").len();
    let t = restart_at(&mut env, &id, Some("plan"));
    assert!(t.active(), "{:?}", t.state);
    assert_eq!(dispatch::events::stage_names(&t)[t.stage], "plan");
    assert!(t.pipeline_file.ends_with("pipeline.2.toml"));
    for a in t.attempts_of("plan") {
        assert!(
            matches!(&a.state, AttemptState::Cancelled { reason } if reason == "discarded by restart at plan"),
            "{a:#?}"
        );
    }
    assert!(
        t.attempts_of("review")
            .all(|a| a.state != AttemptState::Complete)
    );
    assert!(
        t.decisions
            .iter()
            .filter(|d| d.name == "finalize")
            .all(|d| d.state == DecisionState::Cancelled)
    );
    assert!(
        t.decisions
            .iter()
            .all(|d| d.name != "lanes" || matches!(d.state, DecisionState::Answered { .. })),
        "lanes is not asked again"
    );
    assert_eq!(t.lanes.iter().map(|l| l.chosen).collect::<Vec<_>>(), chosen);
    assert!(t.restarts[0].reset.is_empty());
    assert!(env.repo.lock().unwrap().resets.is_empty());
    let tree = t.tree.clone().unwrap();
    let entry = t.entered.last().unwrap();
    assert_eq!(entry.stage, "plan");
    assert_eq!(entry.heads["root"], env.repo.lock().unwrap().heads[&tree]);
    env.step();
    let d = rerun_about(&env, &id, "plan", 1);
    answer(&mut env, &id, &d, "rerun");
    env.steps_until(&id, "a second planner", |t, _| {
        t.attempts_of("plan").any(Attempt::is_open)
    });
    assert_eq!(env.sb().sessions_named("planner").len(), planners + 1);
    assert!(
        last_prompt_of(&env, "planner").contains("carefully plan"),
        "{}",
        last_prompt_of(&env, "planner")
    );
}

/// No head is recorded for `plan`, and an agent after it may have
/// moved the tree: uncommitted changes and commits beyond the base are
/// each refused by name, and nothing is written.
#[test]
fn a_ranged_restart_without_heads_is_refused_when_the_tree_moved() {
    let mut env = Env::new();
    let id = at_finalize_without_entries(&mut env);
    let before = env.ticket(&id);
    let tree = before.tree.clone().unwrap();
    env.repo
        .lock()
        .unwrap()
        .changes
        .insert(tree.clone(), vec![PathBuf::from("src/lib.rs")]);
    let e = restart_refused(&mut env, &id, Some("plan"));
    assert!(e.contains("root has uncommitted changes in"), "{e}");
    assert!(e.contains("src/lib.rs"), "{e}");
    assert!(!e.contains("has moved"), "{e}");
    assert_eq!(env.ticket(&id), before, "nothing written");
    env.repo.lock().unwrap().changes.clear();
    seed_moved(&env, &tree);
    let e = restart_refused(&mut env, &id, Some("plan"));
    assert!(e.contains("no head is recorded for plan"), "{e}");
    assert!(e.contains("root has moved from its base"), "{e}");
    assert_eq!(env.ticket(&id), before, "nothing written");
}

/// Without entries, a lane with a repository of its own that moved
/// from its base is named in the refusal; the tree, still at its base,
/// is not.
#[test]
fn a_ranged_restart_is_refused_when_a_lane_repo_moved_without_heads() {
    let mut env = Env::new();
    let text = two_lane_pipeline(&env.worktrees);
    let id = two_lanes(&mut env, &text);
    let lanes = two_lanes_checking(&mut env, &id);
    {
        let mut repo = env.repo.lock().unwrap();
        for (_, _, key) in &lanes {
            repo.check_exits.insert(key.clone(), 0);
        }
    }
    env.steps_until(&id, "inspect", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "inspect")
    });
    let mut t = env.ticket(&id);
    t.entered.clear();
    dispatch::store::write_ticket(&env.data.ticket_file(&id), &t).unwrap();
    let docs = t.lanes[1].worktree.clone();
    seed_moved(&env, &docs);
    let before = env.ticket(&id);
    let e = restart_refused(&mut env, &id, Some("plan"));
    assert!(e.contains("docs has moved from its base"), "{e}");
    assert!(!e.contains("root has moved"), "{e}");
    assert_eq!(env.ticket(&id), before, "nothing written");
}

/// `implement` failed at its checks, and the live file adds `design`
/// before it: a failed agent may have committed, so the tree is read,
/// and the restart goes ahead only once it has nothing beyond its base.
#[test]
fn a_live_only_restart_after_a_failed_agent_reads_whether_the_tree_moved() {
    let mut env = Env::new();
    let id = implement_checks_127(&mut env);
    let mut t = env.ticket(&id);
    t.entered.clear();
    dispatch::store::write_ticket(&env.data.ticket_file(&id), &t).unwrap();
    live_edit(
        &env,
        "[[stages]]\nname = \"implement\"",
        "[[stages]]\nname = \"design\"\noperator = \"planner\"\ncontext = \"each\"\nwrites = [\"design\"]\nprompt = \"Design to {design}.\"\n\n[[stages]]\nname = \"implement\"",
    );
    let tree = t.tree.clone().unwrap();
    seed_moved(&env, &tree);
    let before = env.ticket(&id);
    let e = restart_refused(&mut env, &id, Some("design"));
    assert!(e.contains("no head is recorded for design"), "{e}");
    assert!(e.contains("root has moved from its base"), "{e}");
    assert_eq!(env.ticket(&id), before, "nothing written");
    env.repo.lock().unwrap().commits.clear();
    let t = restart_at(&mut env, &id, Some("design"));
    assert!(t.active(), "{:?}", t.state);
    assert_eq!(dispatch::events::stage_names(&t)[t.stage], "design");
    let r = &t.restarts[0];
    assert_eq!((r.from.as_str(), r.to.as_str()), ("implement", "design"));
    assert!(r.reset.is_empty(), "{r:#?}");
    assert!(
        r.discarded.is_empty(),
        "the failed attempt was never complete"
    );
}

/// `at_inspect` with the implementer's commit at `work0001`: the tree
/// moved during `implement`.
fn at_inspect_with_work(env: &mut Env) -> String {
    let (id, implementer) = at_implement(env);
    let tree = env.ticket(&id).tree.clone().unwrap();
    env.repo
        .lock()
        .unwrap()
        .heads
        .insert(tree, "work0001".into());
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

/// A bring-up after the entry moved the lane's base; the restart back
/// to the stage puts the base and the bring-up record back as they were
/// at entry, and leaves what the remote holds alone.
#[test]
fn a_ranged_restart_restores_base_and_refreshed_from_the_entry() {
    let mut env = Env::new();
    let id = at_inspect_with_work(&mut env);
    let mut t = env.ticket(&id);
    let entry = t
        .entered
        .iter()
        .rev()
        .find(|e| e.stage == "implement")
        .cloned()
        .unwrap();
    let at_entry = entry.lanes["repo"].clone();
    let pushed = PushedHead {
        head: "work0001".into(),
        at_ms: 5,
    };
    t.lanes[0].base_sha = Some("moved001".into());
    t.lanes[0].refreshed = Some(dispatch::ticket::Refreshed {
        from: "base0000".into(),
        to: "moved001".into(),
        commits: true,
        notes: None,
        at_ms: 5,
        conflict: None,
        after: Some("work0001".into()),
    });
    t.lanes[0].pushed = Some(pushed.clone());
    dispatch::store::write_ticket(&env.data.ticket_file(&id), &t).unwrap();
    let t = restart_at(&mut env, &id, Some("implement"));
    assert!(t.active(), "{:?}", t.state);
    assert_eq!(t.lanes[0].base_sha, at_entry.base_sha);
    assert_eq!(t.lanes[0].refreshed, at_entry.refreshed);
    assert_eq!(t.lanes[0].pushed, Some(pushed), "what the remote holds");
    assert_eq!(entry.heads["root"], "base0000");
    assert_eq!(
        env.repo.lock().unwrap().heads[t.tree.as_ref().unwrap()],
        "base0000"
    );
}

/// A send-back moves the stage and no branch, so the entry for
/// `implement` is still the head before it first ran.
#[test]
fn a_ranged_restart_after_a_send_back_resets_to_the_head_before_the_stage_first_ran() {
    let mut env = Env::new();
    let id = at_inspect_with_work(&mut env);
    env.inspect(&id, "rerun", Some("tighten it"));
    env.steps_until(&id, "a second implementer", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| a.n == 2 && a.is_open())
    });
    let tree = env.ticket(&id).tree.clone().unwrap();
    env.repo
        .lock()
        .unwrap()
        .heads
        .insert(tree.clone(), "work0002".into());
    let second = session_of(&env.ticket(&id), "implement");
    implementer_stops(&mut env, &id, &second);
    env.steps_until(&id, "the second checks", |t, _| {
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
    let t = restart_at(&mut env, &id, Some("implement"));
    assert!(t.active(), "{:?}", t.state);
    let reset = &t.restarts[0].reset;
    assert_eq!(reset.len(), 1);
    assert_eq!(
        (
            reset[0].key.as_str(),
            reset[0].from.as_str(),
            reset[0].to.as_str()
        ),
        ("root", "work0002", "base0000")
    );
    assert_eq!(env.repo.lock().unwrap().heads[&tree], "base0000");
}

/// Git refuses the reset: the restart holds, parked with git's reason
/// and its intent kept; a resume is refused while it is held, and the
/// restart again finishes it.
#[test]
fn a_reset_refused_by_git_keeps_the_intent_and_resume_refuses() {
    let mut env = Env::new();
    let id = at_inspect_with_work(&mut env);
    env.repo.lock().unwrap().fail_reset =
        Some("error: Entry 'src/a.rs' not uptodate. Cannot merge.".into());
    let t = restart_at(&mut env, &id, Some("implement"));
    let TicketState::Parked { reason } = &t.state else {
        panic!("{:?}", t.state)
    };
    assert!(
        reason.starts_with("restart at implement held: ") && reason.contains("not uptodate"),
        "{reason}"
    );
    assert!(t.restart.is_some() && t.restarts.is_empty());
    let now = env.tick();
    let Err(e) = env.runner.resume(&id, now) else {
        panic!("a resume with a restart held")
    };
    assert!(format!("{e:#}").contains("part done"), "{e:#}");
    // A pass leaves a held restart alone.
    env.step();
    assert!(matches!(env.ticket(&id).state, TicketState::Parked { .. }));
    let t = restart_at(&mut env, &id, Some("implement"));
    assert!(t.active(), "{:?}", t.state);
    assert_eq!(t.restarts[0].reset.len(), 1);
}

/// A copy with a stage gone before the current one: the recorded
/// conflicts move with their stage names, one at the stage gone is
/// dropped, and no resolution review starts.
#[test]
fn a_restart_keeps_and_remaps_conflict_stages() {
    let mut env = Env::new();
    let id = at_inspect(&mut env);
    let mut t = env.ticket(&id);
    assert_eq!(t.stage, 5);
    let conflict = |stage: usize| dispatch::ticket::RefreshConflict {
        before: "head0001".into(),
        from: "base0000".into(),
        to: "main0002".into(),
        commits: vec![],
        stage,
        at_ms: 0,
    };
    // `lanes` is stage 1, `implement` stage 4.
    t.lanes[0].conflict = Some(conflict(1));
    t.lanes[0].refreshed = Some(dispatch::ticket::Refreshed {
        from: "base0000".into(),
        to: "main0002".into(),
        commits: true,
        notes: None,
        at_ms: 5,
        conflict: Some(conflict(4)),
        after: None,
    });
    dispatch::store::write_ticket(&env.data.ticket_file(&id), &t).unwrap();
    live_edit(
        &env,
        "[[stages]]\nname = \"lanes\"\ngate = { kind = \"human\", decision = \"lanes\" }\n\n",
        "",
    );
    let t = restart_at(&mut env, &id, None);
    assert!(t.active(), "{:?}", t.state);
    assert_eq!(t.stage, 4, "inspect, one stage earlier in the new copy");
    assert_eq!(t.refreshed_stage, Some(4));
    assert_eq!(t.lanes[0].conflict, None, "its stage is gone");
    let moved = t.lanes[0].refreshed.as_ref().unwrap();
    assert_eq!(moved.conflict.as_ref().map(|c| c.stage), Some(3));
    env.step();
    env.step();
    assert!(env.ticket(&id).attempts_of(RESOLUTION).next().is_none());
}

/// A two-lane pipeline: the project's repository at `.`, a docs
/// repository of its own at `docs`, each with a setup; the lanes
/// question, a plan and an implement stage per lane, and a look.
fn two_lane_pipeline(worktrees: &std::path::Path) -> String {
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

[[lanes]]
name = "docs"
path = "docs"
repo = "git@github.com:msull/docs.git"
setup = ["mdbook", "build"]

[operators.planner]
kind = "claude"

[operators.implementer]
kind = "claude"

[[stages]]
name = "lanes"
gate = {{ kind = "human", decision = "lanes" }}

[[stages]]
name = "plan"
operator = "planner"
context = "each"
writes = ["plan"]
prompt = "Plan to {{plan}} on {{branch}}."

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

[policy]
slots = 4
waiting_on_me = 6
decisions = {{ lanes = "ask" }}
"#,
        worktrees = worktrees.display()
    )
}

/// Every open attempt of `stage` finished with its artifact.
fn finish_all(env: &mut Env, id: &str, stage: &str, artifact: &str) {
    let t = env.ticket(id);
    let open: Vec<Attempt> = t
        .attempts_of(stage)
        .filter(|a| a.is_open())
        .cloned()
        .collect();
    for a in open {
        env.finish(
            a.session.as_ref().unwrap(),
            &a.artifacts[artifact],
            "# done",
        );
    }
}

/// A two-lane ticket with both lanes chosen, both plans done and both
/// implementers running.
fn two_lanes(env: &mut Env, pipeline_text: &str) -> String {
    std::fs::write(env.data.pipeline(PROJECT), pipeline_text).unwrap();
    let id = env.take(21).id;
    env.steps_until(&id, "the lanes question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "lanes")
    });
    let d = env
        .pending(&id)
        .into_iter()
        .find(|d| d.name == "lanes")
        .unwrap();
    answer(env, &id, &d, "repo,docs");
    env.steps_until(&id, "both planners", |t, _| {
        t.attempts_of("plan").filter(|a| a.is_open()).count() == 2
    });
    finish_all(env, &id, "plan", "plan");
    env.steps_until(&id, "both implementers", |t, _| {
        t.attempts_of("implement").filter(|a| a.is_open()).count() == 2
    });
    id
}

/// Two lanes plan, and the `repo` lane's branch is behind its base with
/// a rebase that conflicts: the rebaser is given the `repo` lane's plan,
/// though the `docs` lane's plan finished later.
#[test]
fn a_lanes_rebaser_is_given_its_own_lanes_plan() {
    let mut env = Env::new();
    let text = two_lane_pipeline(&env.worktrees)
        .replace(
            "[operators.implementer]\n",
            "[operators.rebaser]\nkind = \"claude\"\n\n[operators.implementer]\n",
        )
        .replace("[policy]\n", "[policy]\nrebaser = \"rebaser\"\n");
    std::fs::write(env.data.pipeline(PROJECT), text).unwrap();
    let id = env.take(21).id;
    env.steps_until(&id, "the lanes question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "lanes")
    });
    let d = env
        .pending(&id)
        .into_iter()
        .find(|d| d.name == "lanes")
        .unwrap();
    answer(&mut env, &id, &d, "repo,docs");
    env.steps_until(&id, "both planners", |t, _| {
        t.attempts_of("plan").filter(|a| a.is_open()).count() == 2
    });
    let t = env.ticket(&id);
    let tree = t
        .lanes
        .iter()
        .find(|l| l.name == "repo")
        .unwrap()
        .worktree
        .clone();
    {
        let mut repo = env.repo.lock().unwrap();
        repo.bases
            .insert(env.runner.data.repo_dir(PROJECT), "main0002".into());
        repo.behind.insert(tree.clone(), 2);
        repo.rebase_conflicts.push(tree);
    }
    let plan_of = |lane: &str| {
        t.attempts_of("plan")
            .find(|a| a.context == lane)
            .unwrap()
            .clone()
    };
    let (repo_plan, docs_plan) = (plan_of("repo"), plan_of("docs"));
    let place = |a: &Attempt| t.attempts.iter().position(|x| x == a).unwrap();
    assert!(
        place(&docs_plan) > place(&repo_plan),
        "docs's plan is the newest, so a lane-blind read hands it to repo"
    );
    for a in [&repo_plan, &docs_plan] {
        env.finish(a.session.as_ref().unwrap(), &a.artifacts["plan"], "# plan");
    }
    env.steps_until(&id, "the rebaser", |t, _| {
        t.attempts_of(dispatch::scheduler::REFRESH)
            .any(|a| a.session.is_some())
    });
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
    let path = |a: &Attempt| a.artifacts["plan"].display().to_string();
    assert!(
        prompt.contains(&format!("(the plan is at {})", path(&repo_plan))),
        "{prompt}"
    );
    assert!(!prompt.contains(&path(&docs_plan)), "{prompt}");
}

/// The tree is brought up by a stage before the lanes are chosen: the
/// lane at `.`, cut on the tree's branch but not yet chosen, moves its
/// base with it, so choosing it later brings up nothing a second time.
#[test]
fn a_tree_brought_up_before_the_lanes_question_moves_the_unchosen_lanes_base() {
    let mut env = Env::new();
    let text = two_lane_pipeline(&env.worktrees)
        .replace(
            "[operators.planner]\n",
            "[operators.investigator]\nkind = \"claude\"\n\n[operators.planner]\n",
        )
        .replace(
            "[[stages]]\nname = \"lanes\"",
            "[[stages]]\nname = \"investigate\"\noperator = \"investigator\"\ncontext = \"root\"\nwrites = [\"notes\"]\nprompt = \"Look into it; notes to {{notes}}.\"\n\n[[stages]]\nname = \"survey\"\noperator = \"investigator\"\ncontext = \"root\"\nwrites = [\"notes\"]\nprompt = \"Survey it; notes to {{notes}}.\"\n\n[[stages]]\nname = \"lanes\"",
        );
    std::fs::write(env.data.pipeline(PROJECT), text).unwrap();
    let id = env.take(21).id;
    env.steps_until(&id, "the investigator", |t, _| {
        t.attempts_of("investigate").any(|a| a.session.is_some())
    });
    let t = env.ticket(&id);
    let tree = t.tree.clone().unwrap();
    let cut_base = t.lanes[0].base_sha.clone();
    {
        let mut repo = env.repo.lock().unwrap();
        repo.bases
            .insert(env.runner.data.repo_dir(PROJECT), "main0002".into());
        repo.behind.insert(tree.clone(), 2);
        repo.rebase_heads.insert(tree.clone(), "main0002".into());
    }
    finish_all(&mut env, &id, "investigate", "notes");
    env.steps_until(&id, "the survey", |t, _| {
        t.attempts_of("survey").any(|a| a.session.is_some())
    });
    let t = env.ticket(&id);
    assert!(t.tree_refreshed.is_some(), "{t:#?}");
    let repo_lane = &t.lanes[0];
    assert_eq!(repo_lane.name, "repo");
    assert!(!repo_lane.chosen);
    assert_ne!(repo_lane.base_sha, cut_base);
    assert_eq!(repo_lane.base_sha.as_deref(), Some("main0002"));

    finish_all(&mut env, &id, "survey", "notes");
    env.steps_until(&id, "the lanes question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "lanes")
    });
    let d = env
        .pending(&id)
        .into_iter()
        .find(|d| d.name == "lanes")
        .unwrap();
    answer(&mut env, &id, &d, "repo,docs");
    env.steps_until(&id, "both planners", |t, _| {
        t.attempts_of("plan").filter(|a| a.is_open()).count() == 2
    });
    let t = env.ticket(&id);
    assert_eq!(t.lanes[0].refreshed, None, "{t:#?}");
    let events = refreshed_events(&env, &id);
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(events[0].1.starts_with("root from"), "{}", events[0].1);
}

/// The survey's entry could not read the tree's head, so a restart from
/// the lanes question to the survey leaves the tree at its brought-up
/// head, and the unchosen lane at `.` keeps the base the bring-up moved
/// it to rather than its base at the survey's entry.
#[test]
fn a_restart_without_the_trees_entry_head_keeps_the_repo_less_lanes_base() {
    let mut env = Env::new();
    let text = two_lane_pipeline(&env.worktrees)
        .replace(
            "[operators.planner]\n",
            "[operators.investigator]\nkind = \"claude\"\n\n[operators.planner]\n",
        )
        .replace(
            "[[stages]]\nname = \"lanes\"",
            "[[stages]]\nname = \"investigate\"\noperator = \"investigator\"\ncontext = \"root\"\nwrites = [\"notes\"]\nprompt = \"Look into it; notes to {{notes}}.\"\n\n[[stages]]\nname = \"survey\"\noperator = \"investigator\"\ncontext = \"root\"\nwrites = [\"notes\"]\nprompt = \"Survey it; notes to {{notes}}.\"\n\n[[stages]]\nname = \"lanes\"",
        );
    std::fs::write(env.data.pipeline(PROJECT), text).unwrap();
    let id = env.take(21).id;
    env.steps_until(&id, "the investigator", |t, _| {
        t.attempts_of("investigate").any(|a| a.session.is_some())
    });
    let tree = env.ticket(&id).tree.clone().unwrap();
    {
        let mut repo = env.repo.lock().unwrap();
        repo.bases
            .insert(env.runner.data.repo_dir(PROJECT), "main0002".into());
        repo.behind.insert(tree.clone(), 2);
        repo.rebase_heads.insert(tree.clone(), "main0002".into());
    }
    finish_all(&mut env, &id, "investigate", "notes");
    env.steps_until(&id, "the survey", |t, _| {
        t.attempts_of("survey").any(|a| a.session.is_some())
    });
    finish_all(&mut env, &id, "survey", "notes");
    env.steps_until(&id, "the lanes question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "lanes")
    });
    let mut t = env.ticket(&id);
    let up = t.tree_refreshed.clone().expect("the bring-up is recorded");
    assert_eq!(t.lanes[0].base_sha.as_deref(), Some("main0002"), "{t:#?}");
    for e in t.entered.iter_mut().filter(|e| e.stage == "survey") {
        e.heads.remove("root");
    }
    dispatch::store::write_ticket(&env.data.ticket_file(&id), &t).unwrap();
    let t = restart_at(&mut env, &id, Some("survey"));
    assert!(
        t.restarts[0].reset.iter().all(|h| h.key != "root"),
        "{t:#?}"
    );
    assert_eq!(t.tree_refreshed, Some(up));
    assert_eq!(t.lanes[0].name, "repo");
    assert_eq!(t.lanes[0].base_sha.as_deref(), Some("main0002"), "{t:#?}");
}

/// Both implementers stop and their checks start; each lane's attempt
/// number and checks key.
fn two_lanes_checking(env: &mut Env, id: &str) -> Vec<(String, u32, String)> {
    finish_all(env, id, "implement", "notes");
    env.steps_until(id, "both checks", |t, _| {
        t.attempts_of("implement")
            .filter(|a| a.gate.is_some())
            .count()
            == 2
    });
    env.ticket(id)
        .attempts_of("implement")
        .map(|a| (a.context.clone(), a.n, format!("{id}/implement/{}", a.n)))
        .collect()
}

/// Both lanes' pull requests as the provider reports them: open, green,
/// at `base0000`, with `docs` reported as `docs_mergeable`.
fn two_lane_prs(env: &Env, id: &str, docs_mergeable: Option<&str>) {
    let t = env.ticket(id);
    let mut prs = env.prs.lock().unwrap();
    prs.prs.clear();
    prs.checks.clear();
    for (n, lane, repo) in [(7, "repo", "msull/switchboard"), (8, "docs", "msull/docs")] {
        let branch = t
            .lanes
            .iter()
            .find(|l| l.name == lane)
            .unwrap()
            .branch
            .clone();
        prs.prs.push((
            repo.into(),
            branch,
            PullRequest {
                number: n,
                url: format!("https://github.com/{repo}/pull/{n}"),
                head: "base0000".into(),
                state: "open".into(),
                mergeable: if lane == "docs" {
                    docs_mergeable.map(str::to_owned)
                } else {
                    None
                },
                branch: String::new(),
                base: String::new(),
                title: String::new(),
            },
        ));
        prs.checks.push((repo.into(), n, Checks::Passed));
    }
}

/// A two-lane ticket with `ready` and `merge` stages and a rebaser,
/// standing at both lanes' merge decisions.
fn two_lanes_at_merge(env: &mut Env) -> String {
    let text = two_lane_pipeline(&env.worktrees)
        .replace(
            "[operators.implementer]\n",
            "[operators.rebaser]\nkind = \"claude\"\n\n[operators.implementer]\n",
        )
        .replace(
            "[policy]\n",
            "[[stages]]\nname = \"ready\"\ncontext = \"each\"\ngate = { kind = \"external\", check = \"pr-checks\" }\n\n[[stages]]\nname = \"merge\"\ncontext = \"each\"\ngate = { kind = \"external\", check = \"pr-merged\", decision = \"merge\" }\n\n[policy]\nrebaser = \"rebaser\"\n",
        );
    let id = two_lanes(env, &text);
    let lanes = two_lanes_checking(env, &id);
    for (_, _, key) in &lanes {
        env.repo.lock().unwrap().check_exits.insert(key.clone(), 0);
    }
    two_lane_prs(env, &id, None);
    env.steps_until(&id, "both looks", |t, _| {
        t.pending_decisions()
            .iter()
            .filter(|d| d.name == "inspect")
            .count()
            == 2
    });
    for d in env.pending(&id) {
        answer(env, &id, &d, "proceed");
    }
    env.steps_until(&id, "both merge decisions", |t, _| {
        t.pending_decisions()
            .iter()
            .filter(|d| d.name == "merge")
            .count()
            == 2
    });
    id
}

/// Two lanes at `merge`. The `repo` lane's tree is clean and its base
/// moved into a conflict; the `docs` lane's tree is not clean and its PR
/// reads conflicting. A trip back moves every lane, so neither goes
/// back while `docs` is not clean, and both merge decisions name it;
/// once it is clean, both go back to `ready` together.
#[test]
fn a_dirty_second_lane_holds_the_send_back_of_a_clean_one() {
    let mut env = Env::new();
    let id = two_lanes_at_merge(&mut env);
    let t = env.ticket(&id);
    let tree = |lane: &str| {
        t.lanes
            .iter()
            .find(|l| l.name == lane)
            .unwrap()
            .worktree
            .clone()
    };
    let (repo_tree, docs_tree) = (tree("repo"), tree("docs"));
    base_moves(&env, &repo_tree, &["src/a.rs"]);
    env.repo.lock().unwrap().dirty.push(docs_tree.clone());
    two_lane_prs(&env, &id, Some("conflicting"));
    let names_docs = |t: &Ticket| {
        let merge: Vec<_> = t
            .pending_decisions()
            .into_iter()
            .filter(|d| d.name == "merge")
            .collect();
        merge.len() == 2
            && merge.iter().all(|d| {
                d.question.contains(&format!(
                    "the tree of lane docs at {} is not clean",
                    docs_tree.display()
                ))
            })
    };
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "both decisions naming docs", |t, _| names_docs(t));
    let repo_decision = env
        .pending(&id)
        .into_iter()
        .find(|d| d.name == "merge" && d.question.starts_with("merge (repo)"))
        .unwrap();
    assert!(
        repo_decision
            .question
            .contains("the base moved and conflicts in src/a.rs"),
        "{}",
        repo_decision.question
    );
    answer(&mut env, &id, &repo_decision, "recheck");
    env.step();
    polls(&mut env, 2);
    let t = env.ticket(&id);
    assert!(stands_at(&t, "merge"));
    assert_eq!(count_of(&events_of(&env.data, &id), "sent-back"), 0);
    assert!(env.sb().cloned.is_empty(), "no rebaser in either lane");
    assert!(env.repo.lock().unwrap().pushed.is_empty());
    env.repo.lock().unwrap().dirty.clear();
    polls(&mut env, 1);
    env.steps_until(&id, "the trip back", |t, _| stands_at(t, "ready"));
    let t = env.ticket(&id);
    for lane in ["repo", "docs"] {
        let a = t
            .attempts_of("merge")
            .filter(|a| a.context == lane)
            .last()
            .unwrap();
        assert!(
            matches!(&a.state, AttemptState::Cancelled { reason } if reason.starts_with("sent back from merge: ")),
            "{lane}: {a:?}"
        );
    }
    assert_eq!(count_of(&events_of(&env.data, &id), "sent-back"), 1);
}

/// Two lanes at `merge`, the `docs` lane's PR merged by hand, then the
/// `repo` lane's base moved into a conflict: only `repo` goes back
/// through `ready`. `docs` keeps its passes, its tree (left dirty) does
/// not hold the trip, and its branch, behind a base that now holds it,
/// is neither rebased nor pushed.
#[test]
fn a_merged_lane_stays_merged_when_the_other_lane_is_sent_back() {
    let mut env = Env::new();
    let id = two_lanes_at_merge(&mut env);
    let t = env.ticket(&id);
    let tree = |lane: &str| {
        t.lanes
            .iter()
            .find(|l| l.name == lane)
            .unwrap()
            .worktree
            .clone()
    };
    let (repo_tree, docs_tree) = (tree("repo"), tree("docs"));
    let last = |t: &Ticket, stage: &str, lane: &str| {
        t.attempts_of(stage)
            .filter(|a| a.context == lane && a.kind == AttemptKind::GateOnly)
            .last()
            .unwrap()
            .state
            .clone()
    };
    env.prs
        .lock()
        .unwrap()
        .prs
        .iter_mut()
        .find(|(_, _, pr)| pr.number == 8)
        .unwrap()
        .2
        .state = "merged".into();
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "docs merged", |t, _| {
        last(t, "merge", "docs") == AttemptState::Complete
    });
    {
        let mut repo = env.repo.lock().unwrap();
        repo.bases
            .insert(env.data.lane_repo_dir(PROJECT, "docs"), "docs0002".into());
        repo.behind.insert(docs_tree.clone(), 1);
        repo.rebase_conflicts.push(docs_tree.clone());
        repo.dirty.push(docs_tree);
    }
    base_moves(&env, &repo_tree, &["src/a.rs"]);
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the trip back", |t, _| stands_at(t, "ready"));
    let t = env.ticket(&id);
    assert_eq!(last(&t, "merge", "docs"), AttemptState::Complete);
    assert_eq!(last(&t, "ready", "docs"), AttemptState::Complete);
    env.steps_until(&id, "repo brought up", |t, _| {
        t.lanes
            .iter()
            .any(|l| l.name == "repo" && l.base_sha.as_deref() == Some("main0002"))
    });
    let repo = env.repo.lock().unwrap();
    assert!(
        repo.rebased.iter().all(|(dir, _)| *dir == repo_tree),
        "{:?}",
        repo.rebased
    );
    assert!(repo.pushed.iter().all(|(dir, ..)| *dir == repo_tree));
    drop(repo);
    assert!(env.sb().cloned.is_empty(), "no rebaser for docs");
    assert!(env.pending(&id).iter().all(|d| d.name != "refresh"));
}

/// One lane passed its checks and one failed at them: a plain restart
/// discards the pass too, so both lanes meet the new gate, each asked
/// with `check`, and neither launches an agent.
#[test]
fn a_plain_restart_of_an_each_stage_discards_the_lanes_that_passed() {
    let mut env = Env::new();
    let text = two_lane_pipeline(&env.worktrees);
    let id = two_lanes(&mut env, &text);
    let lanes = two_lanes_checking(&mut env, &id);
    {
        let mut repo = env.repo.lock().unwrap();
        repo.check_exits.insert(lanes[0].2.clone(), 0);
        repo.check_exits.insert(lanes[1].2.clone(), 1);
    }
    env.steps_until(&id, "one pass and one failure", |t, _| {
        !t.pending_decisions().is_empty()
    });
    {
        let mut repo = env.repo.lock().unwrap();
        repo.check_exits.clear();
    }
    live_edit(&env, GATE_ARGV, FIXED_ARGV);
    let implementers = env.sb().sessions_named("implementer").len();
    let t = restart_at(&mut env, &id, None);
    assert!(t.active(), "{:?}", t.state);
    assert_eq!(
        t.restarts[0].discarded,
        vec![("implement".to_owned(), lanes[0].1)]
    );
    env.step();
    let checks_before = env.repo.lock().unwrap().checks.len();
    for (_, n, _) in &lanes {
        let d = rerun_about(&env, &id, "implement", *n);
        assert_eq!(d.options, vec!["rerun", "check", "park"]);
        answer(&mut env, &id, &d, "check");
    }
    env.steps_until(&id, "both checks again", |t, _| {
        t.attempts_of("implement")
            .filter(|a| a.is_open() && a.gate.is_some())
            .count()
            == 2
    });
    let repo = env.repo.lock().unwrap();
    assert_eq!(repo.checks.len(), checks_before + 2);
    for c in &repo.checks[checks_before..] {
        assert_eq!(c.argv, vec!["sh", "-c", "cargo nextest run"]);
    }
    drop(repo);
    assert_eq!(env.sb().sessions_named("implementer").len(), implementers);
}

/// Only the lane whose setup changed runs it again.
#[test]
fn a_restart_reruns_setup_only_where_it_changed() {
    let mut env = Env::new();
    let text = two_lane_pipeline(&env.worktrees);
    let id = two_lanes(&mut env, &text);
    let t = env.ticket(&id);
    assert!(t.lanes.iter().all(|l| l.setup_done));
    live_edit(
        &env,
        r#"setup = ["mdbook", "build"]"#,
        r#"setup = ["mdbook", "build", "--strict"]"#,
    );
    let t = restart_at(&mut env, &id, None);
    assert_eq!(t.restarts[0].setup_again, vec!["docs"]);
    let done: Vec<(&str, bool)> = t
        .lanes
        .iter()
        .map(|l| (l.name.as_str(), l.setup_done))
        .collect();
    assert_eq!(done, vec![("repo", true), ("docs", false)]);
}

/// Back to `plan` from `inspect`: the ticket's tree and the docs lane's
/// own branch are reset to their heads as the ticket entered `plan`,
/// `plan` and `implement` are discarded, `plan` asks before it runs,
/// and the next pass brings the branches up again.
#[test]
fn a_restart_at_an_earlier_stage_resets_each_lane_to_its_entry_head() {
    let mut env = Env::new();
    let text = two_lane_pipeline(&env.worktrees);
    let id = two_lanes(&mut env, &text);
    let t = env.ticket(&id);
    let plan_entry = t
        .entered
        .iter()
        .rev()
        .find(|e| e.stage == "plan")
        .cloned()
        .unwrap();
    let tree = t.tree.clone().unwrap();
    let docs = t.lanes[1].worktree.clone();
    {
        let mut repo = env.repo.lock().unwrap();
        repo.heads.insert(tree.clone(), "work0001".into());
        repo.heads.insert(docs.clone(), "docs0001".into());
    }
    let lanes = two_lanes_checking(&mut env, &id);
    {
        let mut repo = env.repo.lock().unwrap();
        for (_, _, key) in &lanes {
            repo.check_exits.insert(key.clone(), 0);
        }
    }
    env.steps_until(&id, "inspect", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "inspect")
    });
    let planners = env.sb().sessions_named("planner").len();
    let t = restart_at(&mut env, &id, Some("plan"));
    assert!(t.active(), "{:?}", t.state);
    assert_eq!(t.stage, 1);
    assert_eq!(t.refreshed_stage, None, "brought up on re-entry");
    {
        let repo = env.repo.lock().unwrap();
        assert_eq!(repo.heads[&tree], plan_entry.heads["root"]);
        assert_eq!(repo.heads[&docs], plan_entry.heads["docs"]);
    }
    let keys: Vec<&str> = t.restarts[0].reset.iter().map(|h| h.key.as_str()).collect();
    assert_eq!(keys, vec!["root", "docs"]);
    for a in t.attempts_of("plan").chain(t.attempts_of("implement")) {
        assert!(
            matches!(&a.state, AttemptState::Cancelled { reason } if reason == "discarded by restart at plan"),
            "{a:#?}"
        );
    }
    assert!(t.pending_decisions().is_empty());
    let entry = t.entered.last().unwrap();
    assert_eq!(entry.stage, "plan");
    assert_eq!(entry.heads["root"], plan_entry.heads["root"]);
    env.step();
    let t = env.ticket(&id);
    for a in t.attempts_of("plan") {
        let d = rerun_about(&env, &id, "plan", a.n);
        assert_eq!(d.options, vec!["rerun", "park"]);
    }
    assert_eq!(
        env.sb().sessions_named("planner").len(),
        planners,
        "nothing launched"
    );
}

/// Back to `lanes`: the answer is withdrawn, every lane is chosen
/// again, and the question is asked afresh.
#[test]
fn a_restart_before_lanes_asks_lanes_again() {
    let mut env = Env::new();
    let text = two_lane_pipeline(&env.worktrees);
    std::fs::write(env.data.pipeline(PROJECT), &text).unwrap();
    let id = env.take(22).id;
    env.steps_until(&id, "the lanes question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "lanes")
    });
    let first = env
        .pending(&id)
        .into_iter()
        .find(|d| d.name == "lanes")
        .unwrap();
    answer(&mut env, &id, &first, "repo");
    env.steps_until(&id, "the planner", |t, _| {
        t.attempts_of("plan").next().is_some()
    });
    let t = env.ticket(&id);
    assert_eq!(t.lanes.iter().filter(|l| l.chosen).count(), 1);
    assert_eq!(
        t.entered[0].stage, "lanes",
        "the first stage's entry, at the cut"
    );
    let t = restart_at(&mut env, &id, Some("lanes"));
    assert!(t.active(), "{:?}", t.state);
    assert!(t.lanes.iter().all(|l| l.chosen));
    assert!(matches!(
        t.decisions.iter().find(|d| d.id == first.id).unwrap().state,
        DecisionState::Cancelled
    ));
    env.steps_until(&id, "the lanes question again", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "lanes")
    });
}

#[test]
fn a_restart_is_refused_when_the_live_file_drops_a_lane() {
    let mut env = Env::new();
    let text = two_lane_pipeline(&env.worktrees);
    let id = two_lanes(&mut env, &text);
    live_edit(
        &env,
        "[[lanes]]\nname = \"docs\"\npath = \"docs\"\nrepo = \"git@github.com:msull/docs.git\"\nsetup = [\"mdbook\", \"build\"]\n",
        "",
    );
    let e = restart_refused(&mut env, &id, None);
    assert!(e.contains("drops lane docs"), "{e}");
    std::fs::write(
        env.data.pipeline(PROJECT),
        text.replace("path = \"docs\"", "path = \"documentation\""),
    )
    .unwrap();
    let e = restart_refused(&mut env, &id, None);
    assert!(e.contains("changes lane docs"), "{e}");
}

/// Someone else's branches are never reset: a ticket from pull requests
/// restarts only at its current stage.
#[test]
fn a_restart_is_refused_ranged_on_a_pull_request_ticket() {
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
    for _ in 0..2 {
        env.inspect(&id, "proceed", None);
    }
    env.steps_until(&id, "the merge stage", |t, _| t.stage == 1);
    let e = restart_refused(&mut env, &id, Some("inspect"));
    assert!(e.contains("someone else's branches"), "{e}");
    let t = restart_at(&mut env, &id, None);
    assert!(t.active(), "{:?}", t.state);
    assert!(t.pipeline_file.ends_with("pipeline.2.toml"));
}

/// A pull-request ticket cannot take a ranged restart, so the refusal
/// for a stage added before its current one does not point at one.
#[test]
fn a_restart_at_the_current_stage_of_a_pull_request_ticket_does_not_point_at_a_ranged_restart() {
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
    for _ in 0..2 {
        env.inspect(&id, "proceed", None);
    }
    env.steps_until(&id, "the merge stage", |t, _| t.stage == 1);
    let text = pr_pipeline(&worktrees).replacen(
        "[[stages]]\nname = \"merge\"",
        "[[stages]]\nname = \"triage\"\ncontext = \"each\"\ngate = { kind = \"human\", decision = \"triage\" }\n\n[[stages]]\nname = \"merge\"",
        1,
    );
    std::fs::write(env.data.pr_pipeline(PROJECT), text).unwrap();
    let before = env.ticket(&id);
    let e = restart_refused(&mut env, &id, None);
    assert!(e.contains("adds triage before merge"), "{e}");
    assert!(!e.contains("restart it at"), "{e}");
    assert_eq!(env.ticket(&id), before, "nothing written");
}

/// An `each` code review with one lane finished and one failed at its
/// checks: the finished lane is asked `rerun | park` (its fold and
/// summary are done; checks that pass would rewrite it again), the
/// failed one `rerun | check | park`, and the restart rewrites nothing.
#[test]
fn a_plain_restart_of_a_review_stage_offers_a_finished_lane_rerun_or_park() {
    let mut env = Env::new();
    let text = two_lane_pipeline(&env.worktrees)
        .replace(
            "[operators.implementer]\n",
            "[operators.lint]\nkind = \"command\"\nargv = [\"sh\", \"-c\", \"lint\"]\n\n[operators.implementer]\n",
        )
        .replace(
            "[[stages]]\nname = \"inspect\"\n",
            "[[stages]]\nname = \"review-code\"\ncontext = \"each\"\nreviewers = [\"lint\"]\nimplementer = \"implementer\"\ncap = 2\ngate = { kind = \"command\", like = \"implement\" }\n\n[[stages]]\nname = \"inspect\"\n",
        );
    let id = two_lanes(&mut env, &text);
    let lanes = two_lanes_checking(&mut env, &id);
    {
        let mut repo = env.repo.lock().unwrap();
        for (_, _, key) in &lanes {
            repo.check_exits.insert(key.clone(), 0);
        }
    }
    env.steps_until(&id, "both reviews", |t, _| {
        t.attempts_of("review-code")
            .filter(|a| {
                a.rounds
                    .last()
                    .is_some_and(|r| r.reviewers.iter().all(|x| x.launched))
            })
            .count()
            == 2
    });
    let t = env.ticket(&id);
    for a in t.attempts_of("review-code") {
        let r = &a.rounds[0].reviewers[0];
        std::fs::write(&r.feedback, "No findings.\n").unwrap();
        env.repo
            .lock()
            .unwrap()
            .check_exits
            .insert(format!("{id}/review-code/{}/r1/lint", a.n), 0);
    }
    env.steps_until(&id, "both rounds converged", |t, _| {
        t.attempts_of("review-code")
            .filter(|a| a.rounds[0].state == RoundState::Converged)
            .count()
            == 2
    });
    let docs = env.ticket(&id).lanes[1].worktree.clone();
    env.repo.lock().unwrap().dirty.push(docs);
    env.steps_until(&id, "one complete, one failed at its checks", |t, _| {
        let states: Vec<&AttemptState> = t.attempts_of("review-code").map(|a| &a.state).collect();
        states.contains(&&AttemptState::Complete)
            && states
                .iter()
                .any(|s| matches!(s, AttemptState::Failed { .. }))
    });
    env.repo.lock().unwrap().dirty.clear();
    let t = env.ticket(&id);
    let done = t
        .attempts_of("review-code")
        .find(|a| a.state == AttemptState::Complete)
        .unwrap()
        .n;
    let failed = t
        .attempts_of("review-code")
        .find(|a| matches!(a.state, AttemptState::Failed { .. }))
        .unwrap()
        .n;
    let (replays, sets) = {
        let repo = env.repo.lock().unwrap();
        (repo.replayed.len(), repo.head_sets.len())
    };
    let t = restart_at(&mut env, &id, None);
    assert!(t.active(), "{:?}", t.state);
    env.step();
    assert_eq!(
        rerun_about(&env, &id, "review-code", done).options,
        vec!["rerun", "park"]
    );
    assert_eq!(
        rerun_about(&env, &id, "review-code", failed).options,
        vec!["rerun", "check", "park"]
    );
    let repo = env.repo.lock().unwrap();
    assert_eq!((repo.replayed.len(), repo.head_sets.len()), (replays, sets));
}

// --- deploy, try and tried: a gate-only command, a held resource, and
// the lanes a stage serves.

/// `WORKSPACE`'s lanes with the back half of a pipeline that deploys:
/// a deploy in the backend lane, a tester with the frontend served, a
/// confirmation, all holding `my-dev`, then an agent stage that holds
/// nothing.
const BACK_HALF: &str = r#"
version = 1

[project]
name = "Orchard"
repo = "git@example.com:example-org/orchard-workspace.git"
worktrees = "{worktrees}"
space = "Dispatch · Orchard"

[source]
kind = "github"
repo = "example-org/orchard-workspace"
label = "dispatch"
lane_hints = { "area:backend" = "backend", "area:frontend" = "frontend" }

[[lanes]]
name = "backend"
path = "orchard-backend"
repo = "git@example.com:example-org/orchard-backend.git"
base = "main"
setup = ["uv", "sync"]

[[lanes]]
name = "frontend"
path = "orchard-frontend"
repo = "git@example.com:example-org/orchard-frontend.git"
base = "dev"
setup = ["npm", "ci"]
serve = { argv = ["npm", "start"], env = { BROWSER = "none", PORT = "{port}" }, url = "http://localhost:{port}", ready = { http = "/", within_secs = 120 } }

[[resources]]
name = "my-dev"

[operators.tester]
kind = "claude"

[operators.implementer]
kind = "claude"

[[stages]]
name = "lanes"
gate = { kind = "human", decision = "lanes" }

[[stages]]
name = "deploy"
context = "lane:backend"
needs = ["my-dev"]
gate = { kind = "command", in = "lane:backend", argv = ["sh", "-c", "inv deploy"] }

[[stages]]
name = "try"
operator = "tester"
context = "joined"
needs = ["my-dev"]
services = ["frontend"]
before = { frontend = ["npm", "run", "link-env"] }
writes = ["notes"]
prompt = "my-dev runs backend {inputs.deploy.commit}; the frontend is at {services.frontend}. Report to {notes}."

[[stages]]
name = "tried"
needs = ["my-dev"]
gate = { kind = "human", decision = "tried", confirm = true }

[[stages]]
name = "after"
operator = "implementer"
context = "each"
writes = ["notes"]
prompt = "Carry on with {branch}; notes to {notes}."

[policy]
slots = 2
waiting_on_me = 3
ports = [3100, 3101]
decisions = { lanes = "auto" }
trust_folders = true
"#;

const BOTH: &[&str] = &["area:backend", "area:frontend"];

fn back_half_env(labels: &[&str]) -> (Env, String) {
    let mut env = Env::new();
    let text = BACK_HALF.replace("{worktrees}", &env.worktrees.display().to_string());
    std::fs::write(env.data.pipeline("Orchard"), text).unwrap();
    let id = take_orchard(&mut env, 42, labels);
    (env, id)
}

/// Another Orchard ticket under the project's pipeline as it is now.
fn take_orchard(env: &mut Env, number: u64, labels: &[&str]) -> String {
    let now = env.tick();
    env.runner
        .take(
            "Orchard",
            &std::fs::read_to_string(env.data.pipeline("Orchard")).unwrap(),
            SourceSnapshot {
                kind: "github".into(),
                identity: format!("example-org/orchard-workspace#{number}"),
                number: Some(number),
                title: format!("Orchard ticket {number}"),
                body: String::new(),
                url: None,
                labels: labels.iter().map(|l| (*l).to_owned()).collect(),
                taken_at_ms: now,
                pull_requests: Vec::new(),
                taken_by: None,
            },
            now,
        )
        .unwrap()
        .id
}

const STAGES: [&str; 5] = ["lanes", "deploy", "try", "tried", "after"];

/// The name of the ticket's stage in its own pipeline copy, so the
/// helpers serve every back-half fixture; `STAGES` for a copy that
/// `break_copy` made unreadable.
fn stage_name(t: &Ticket) -> String {
    let mut names = dispatch::events::stage_names(t);
    if names.is_empty() {
        names = STAGES.map(str::to_owned).to_vec();
    }
    names
        .get(t.stage)
        .cloned()
        .unwrap_or_else(|| "done".to_owned())
}

fn at_stage(env: &mut Env, id: &str, stage: &str) {
    env.steps_until(id, stage, |t, _| stage_name(t) == stage);
}

fn lane_tree(env: &Env, id: &str, lane: &str) -> PathBuf {
    env.worktrees.join(id).join(format!("orchard-{lane}"))
}

fn deploy_key(id: &str, n: u32) -> String {
    format!("{id}/deploy/{n}")
}

fn before_key(id: &str, n: u32) -> String {
    format!("{id}/try/before:frontend:{n}")
}

fn started(env: &Env, key: &str) -> usize {
    env.repo
        .lock()
        .unwrap()
        .checks
        .iter()
        .filter(|c| c.key == key)
        .count()
}

fn exits(env: &Env, key: &str, code: i32) {
    env.repo
        .lock()
        .unwrap()
        .check_exits
        .insert(key.to_owned(), code);
}

fn holds(t: &Ticket) -> Vec<String> {
    t.holds.iter().map(|h| h.resource.clone()).collect()
}

/// The ticket's deploy started: its hold taken, its command running.
fn deploying(env: &mut Env, id: &str) {
    let key = deploy_key(id, 1);
    for _ in 0..8 {
        if started(env, &key) > 0 {
            return;
        }
        env.step();
    }
    panic!("never deployed: {:#?}", env.ticket(id));
}

/// The deploy exits 0 and the ticket stands at `try`.
fn deployed(env: &mut Env, id: &str) {
    deploying(env, id);
    exits(env, &deploy_key(id, 1), 0);
    at_stage(env, id, "try");
}

/// Service sessions made with `session.new`.
fn service_launches(env: &Env) -> Vec<switchboard_control::Request> {
    env.sb()
        .calls
        .iter()
        .filter(|r| {
            matches!(
                &r.body,
                Body::SessionNew {
                    session_kind: SessionKind::Service,
                    ..
                }
            )
        })
        .cloned()
        .collect()
}

/// The frontend's `before` exits 0, its service launches on a port and
/// answers, and the tester starts.
fn served(env: &mut Env, id: &str) {
    serving(env, id, 1);
    env.steps_until(id, "the tester", |t, _| {
        t.attempts_of("try").next().is_some()
    });
}

/// The frontend's service record `n`: its `before` exits 0, it launches
/// on a port, and that port answers until the record reads `Ready`.
fn serving(env: &mut Env, id: &str, n: u32) {
    let key = before_key(id, n);
    for _ in 0..6 {
        if started(env, &key) > 0 {
            break;
        }
        env.step();
    }
    exits(env, &key, 0);
    env.steps_until(id, "the service launched", |t, _| {
        t.services.iter().any(|s| s.n == n && s.session.is_some())
    });
    let port = env
        .ticket(id)
        .services
        .iter()
        .find(|s| s.n == n)
        .and_then(|s| s.port)
        .unwrap();
    env.repo.lock().unwrap().answering.insert(port);
    env.steps_until(id, "the service ready", |t, _| {
        t.services
            .iter()
            .any(|s| s.n == n && s.state == ServiceState::Ready)
    });
}

/// The tester writes its notes and stops; the ticket stands at `tried`.
fn tester_done(env: &mut Env, id: &str) {
    let t = env.ticket(id);
    let tester = session_of(&t, "try");
    env.finish(&tester, &artifact_of(&t, "try", "notes"), "it works");
    at_stage(env, id, "tried");
}

/// The `tried` confirmation answered.
fn tried(env: &mut Env, id: &str, answer: &str) {
    env.steps_until(id, "the tried question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "tried")
    });
    let d = env
        .pending(id)
        .into_iter()
        .find(|d| d.name == "tried")
        .unwrap();
    let now = env.tick();
    env.runner.decide(id, &d.id, answer, None, now).unwrap();
}

/// A ticket through `try` and its tester, waiting at `tried`.
fn at_tried(env: &mut Env, id: &str) {
    deployed(env, id);
    served(env, id);
    tester_done(env, id);
}

/// The ticket parks through `step_one`'s own path: its pipeline copy
/// cannot be read.
fn break_copy(env: &Env, id: &str) {
    std::fs::write(&env.ticket(id).pipeline_file, "not a pipeline").unwrap();
}

fn pending_named(env: &Env, id: &str, name: &str) -> Option<Decision> {
    env.pending(id).into_iter().find(|d| d.name == name)
}

fn answer_named(env: &mut Env, id: &str, name: &str, with: &str) {
    let d = pending_named(env, id, name).unwrap_or_else(|| panic!("no {name} pending"));
    let now = env.tick();
    env.runner.decide(id, &d.id, with, None, now).unwrap();
}

fn is_parked(t: &Ticket) -> bool {
    matches!(t.state, TicketState::Parked { .. })
}

fn is_parking(t: &Ticket) -> bool {
    matches!(t.state, TicketState::Parking { .. })
}

#[test]
fn a_deploy_runs_once_at_the_lanes_clean_head_and_the_tester_is_told_the_commit() {
    let (mut env, id) = back_half_env(BOTH);
    at_stage(&mut env, &id, "deploy");
    let backend = lane_tree(&env, &id, "backend");
    env.repo
        .lock()
        .unwrap()
        .heads
        .insert(backend.clone(), "dep10001".into());
    deploying(&mut env, &id);
    let t = env.ticket(&id);
    assert_eq!(holds(&t), ["my-dev"]);
    let deploys: Vec<&Attempt> = t.attempts_of("deploy").collect();
    assert_eq!(deploys.len(), 1, "{t:#?}");
    assert_eq!(deploys[0].context, "backend");
    assert_eq!(deploys[0].kind, AttemptKind::GateOnly);
    assert!(env.sb().sessions.is_empty(), "a deploy launches nothing");
    {
        let repo = env.repo.lock().unwrap();
        let check = repo
            .checks
            .iter()
            .find(|c| c.key == deploy_key(&id, 1))
            .unwrap();
        assert_eq!(check.dir, backend);
        assert_eq!(check.argv, ["sh", "-c", "inv deploy"]);
        assert!(check.log.ends_with("deploy/1/backend/checks.log"));
        assert!(
            check
                .env
                .contains(&("DISPATCH_STAGE".to_owned(), "deploy".to_owned()))
        );
        assert!(
            check
                .env
                .contains(&("DISPATCH_HEAD".to_owned(), "dep10001".to_owned()))
        );
    }
    exits(&env, &deploy_key(&id, 1), 0);
    at_stage(&mut env, &id, "try");
    let t = env.ticket(&id);
    assert_eq!(
        t.attempts_of("deploy").next().unwrap().head.as_deref(),
        Some("dep10001")
    );
    served(&mut env, &id);
    let prompt = last_prompt_of(&env, "tester");
    assert!(prompt.contains("my-dev runs backend dep10001"), "{prompt}");
    assert_eq!(started(&env, &deploy_key(&id, 1)), 1, "run once");
}

#[test]
fn a_deploy_for_a_lane_not_chosen_is_skipped_and_reads_as_skipped() {
    let (mut env, id) = back_half_env(&["area:frontend"]);
    at_stage(&mut env, &id, "try");
    let t = env.ticket(&id);
    assert_eq!(t.attempts_of("deploy").count(), 0, "{t:#?}");
    assert!(t.pending_decisions().is_empty());
    assert!(
        !env.repo
            .lock()
            .unwrap()
            .checks
            .iter()
            .any(|c| c.key.starts_with(&format!("{id}/deploy"))),
        "no deploy command ran"
    );
    served(&mut env, &id);
    let prompt = last_prompt_of(&env, "tester");
    assert!(
        prompt.contains("backend unknown (deploy skipped)"),
        "{prompt}"
    );
    tester_done(&mut env, &id);
    env.steps_until(&id, "the tried question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "tried")
    });
    let question = pending_named(&env, &id, "tried").unwrap().question;
    assert!(
        question.contains("Deployed: deploy skipped (no backend lane)"),
        "{question}"
    );
    assert!(
        question.contains("Served: frontend http://localhost:3100"),
        "{question}"
    );
}

/// The Orchard live file with an agent stage `deploy-check` added
/// before `deploy`.
fn live_deploy_check(env: &Env) {
    let path = env.data.pipeline("Orchard");
    let text = std::fs::read_to_string(&path).unwrap();
    let from = "[[stages]]\nname = \"deploy\"";
    assert!(text.contains(from));
    let to = "[[stages]]\nname = \"deploy-check\"\noperator = \"tester\"\ncontext = \"lane:backend\"\nwrites = [\"notes\"]\nprompt = \"Check the deploy; notes to {notes}.\"\n\n[[stages]]\nname = \"deploy\"";
    std::fs::write(path, text.replacen(from, to, 1)).unwrap();
}

/// A ticket parked at a failed `deploy`, taken before entries were
/// recorded, whose lanes moved during the stages before it.
fn parked_at_deploy_with_moved_lanes() -> (Env, String) {
    let (mut env, id) = back_half_env(BOTH);
    deploying(&mut env, &id);
    exits(&env, &deploy_key(&id, 1), 1);
    env.steps_until(&id, "the rerun question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    answer_named(&mut env, &id, "rerun", "park");
    env.step();
    let mut t = env.ticket(&id);
    assert!(is_parked(&t), "{t:#?}");
    t.entered.clear();
    dispatch::store::write_ticket(&env.data.ticket_file(&id), &t).unwrap();
    for lane in ["backend", "frontend"] {
        let dir = lane_tree(&env, &id, lane);
        env.repo
            .lock()
            .unwrap()
            .heads
            .insert(dir.clone(), "work0001".into());
        seed_moved(&env, &dir);
    }
    live_deploy_check(&env);
    (env, id)
}

/// The live file adds an agent stage before the failed `deploy`: the
/// ticket is put at it with nothing discarded and no branch reset, since
/// the lanes moved before the target and only a gate-only attempt
/// follows it.
#[test]
fn a_restart_can_target_a_stage_only_the_live_file_has() {
    let (mut env, id) = parked_at_deploy_with_moved_lanes();
    let t = restart_at(&mut env, &id, Some("deploy-check"));
    assert!(t.active(), "{:?}", t.state);
    assert_eq!(dispatch::events::stage_names(&t)[t.stage], "deploy-check");
    let r = &t.restarts[0];
    assert_eq!((r.from.as_str(), r.to.as_str()), ("deploy", "deploy-check"));
    assert!(r.discarded.is_empty() && r.reset.is_empty(), "{r:#?}");
    {
        let repo = env.repo.lock().unwrap();
        for lane in ["backend", "frontend"] {
            assert_eq!(repo.heads[&lane_tree(&env, &id, lane)], "work0001");
        }
        assert!(repo.resets.is_empty());
    }
    env.steps_until(&id, "the deploy-check tester", |t, _| {
        t.attempts_of("deploy-check").any(Attempt::is_open)
    });
    let sb = env.sb();
    let tester = sb.sessions_named("tester");
    assert_eq!(tester.last().unwrap().cwd, lane_tree(&env, &id, "backend"));
}

/// A restart at the current stage names the stage the live file adds
/// before it, and the restart that would run it.
#[test]
fn a_restart_at_the_current_stage_names_the_stage_the_live_file_adds() {
    let (mut env, id) = parked_at_deploy_with_moved_lanes();
    let before = env.ticket(&id);
    let e = restart_refused(&mut env, &id, None);
    assert!(e.contains("adds deploy-check before deploy"), "{e}");
    assert!(e.contains("restart it at deploy-check to run it"), "{e}");
    assert_eq!(env.ticket(&id), before, "nothing written");
}

#[test]
fn a_failed_deploy_asks_rerun_or_park_and_a_resume_does_not_deploy_again_unasked() {
    let (mut env, id) = back_half_env(BOTH);
    deploying(&mut env, &id);
    exits(&env, &deploy_key(&id, 1), 1);
    env.steps_until(&id, "the rerun question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    let d = pending_named(&env, &id, "rerun").unwrap();
    assert_eq!(d.options, ["rerun", "park"], "no check without an agent");
    assert!(d.question.contains("checks exited 1"), "{}", d.question);
    answer_named(&mut env, &id, "rerun", "park");
    env.step();
    let t = env.ticket(&id);
    assert!(is_parked(&t), "{t:#?}");
    assert!(t.holds.is_empty(), "a parked ticket holds nothing");
    let now = env.tick();
    env.runner.resume(&id, now).unwrap();
    env.steps_until(&id, "the question again", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    env.step();
    assert_eq!(started(&env, &deploy_key(&id, 1)), 1);
    assert_eq!(started(&env, &deploy_key(&id, 2)), 0, "not again unasked");
    assert_eq!(holds(&env.ticket(&id)), ["my-dev"], "taken again on resume");
    answer_named(&mut env, &id, "rerun", "rerun");
    for _ in 0..3 {
        env.step();
    }
    assert_eq!(started(&env, &deploy_key(&id, 2)), 1);
    let t = env.ticket(&id);
    let logs: Vec<PathBuf> = t
        .attempts_of("deploy")
        .filter_map(|a| a.gate.as_ref().map(|g| g.log.clone()))
        .collect();
    assert_eq!(logs.len(), 2);
    assert_ne!(logs[0], logs[1], "each run has its own log");
}

#[test]
fn a_resume_past_the_deploy_asks_to_deploy_again_before_anything_reads_it() {
    let (mut env, id) = back_half_env(BOTH);
    at_tried(&mut env, &id);
    tried(&mut env, &id, "park");
    env.step();
    assert!(is_parked(&env.ticket(&id)));
    assert!(env.ticket(&id).holds.is_empty());
    let now = env.tick();
    env.runner.resume(&id, now).unwrap();
    let deploy_rerun = |t: &Ticket| {
        t.pending_decisions()
            .into_iter()
            .find(|d| d.name == "rerun" && d.stage == "deploy")
            .cloned()
    };
    env.steps_until(&id, "the deploy question", |t, _| deploy_rerun(t).is_some());
    let t = env.ticket(&id);
    assert_eq!(stage_name(&t), "deploy");
    assert_eq!(holds(&t), ["my-dev"]);
    let d = deploy_rerun(&t).unwrap();
    assert!(d.question.contains("my-dev was let go"), "{}", d.question);
    assert_eq!(started(&env, &deploy_key(&id, 2)), 0, "not again unasked");
    let now = env.tick();
    env.runner.decide(&id, &d.id, "rerun", None, now).unwrap();
    for _ in 0..3 {
        env.step();
    }
    assert_eq!(started(&env, &deploy_key(&id, 2)), 1);
    exits(&env, &deploy_key(&id, 2), 0);
    at_stage(&mut env, &id, "try");
    tested_again(&mut env, &id);
}

#[test]
fn a_resume_with_the_deploy_skipped_serves_again_and_asks_before_the_tester() {
    let (mut env, id) = back_half_env(&["area:frontend"]);
    at_stage(&mut env, &id, "try");
    served(&mut env, &id);
    tester_done(&mut env, &id);
    tried(&mut env, &id, "park");
    env.step();
    assert!(is_parked(&env.ticket(&id)));
    let now = env.tick();
    env.runner.resume(&id, now).unwrap();
    at_stage(&mut env, &id, "try");
    tested_again(&mut env, &id);
}

#[test]
fn a_resume_past_a_human_gate_in_the_range_asks_it_again() {
    let mut env = Env::new();
    let text = BACK_HALF
        .replace("{worktrees}", &env.worktrees.display().to_string())
        .replace(
            "[[stages]]\nname = \"tried\"",
            "[[stages]]\nname = \"look\"\nneeds = [\"my-dev\"]\ngate = { kind = \"human\", decision = \"look\" }\n\n[[stages]]\nname = \"tried\"",
        );
    std::fs::write(env.data.pipeline("Orchard"), text).unwrap();
    let id = take_orchard(&mut env, 42, BOTH);
    deployed(&mut env, &id);
    served(&mut env, &id);
    let t = env.ticket(&id);
    env.finish(
        &session_of(&t, "try"),
        &artifact_of(&t, "try", "notes"),
        "it works",
    );
    env.steps_until(&id, "the look question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "look")
    });
    answer_named(&mut env, &id, "look", "proceed");
    tried(&mut env, &id, "park");
    env.step();
    assert!(is_parked(&env.ticket(&id)));
    let now = env.tick();
    env.runner.resume(&id, now).unwrap();
    env.steps_until(&id, "the deploy question", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == "rerun" && d.stage == "deploy")
    });
    answer_named(&mut env, &id, "rerun", "rerun");
    for _ in 0..3 {
        env.step();
    }
    exits(&env, &deploy_key(&id, 2), 0);
    at_stage(&mut env, &id, "try");
    serving(&mut env, &id, 2);
    env.steps_until(&id, "the tester question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    answer_named(&mut env, &id, "rerun", "rerun");
    env.steps_until(&id, "the second tester", |t, _| {
        t.attempts_of("try").count() == 2
    });
    let t = env.ticket(&id);
    env.finish(
        &session_of(&t, "try"),
        &artifact_of(&t, "try", "notes"),
        "still works",
    );
    env.steps_until(&id, "a question after the tester", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == "look" || d.name == "tried")
    });
    let t = env.ticket(&id);
    assert!(
        t.pending_decisions().iter().any(|d| d.name == "look"),
        "the earlier look was about the old deploy: {:#?}",
        t.pending_decisions()
    );
    let looks: Vec<&Attempt> = t.attempts_of("look").collect();
    assert_eq!(looks.len(), 2, "{looks:#?}");
    assert!(
        matches!(&looks[0].state, AttemptState::Cancelled { reason } if reason.contains("my-dev was let go")),
        "{looks:#?}"
    );
}

/// Back at `try` after a resume: the frontend comes up again before the
/// tester's cancelled attempt is asked about, nothing runs until the
/// answer, and `tried` then says what is served.
fn tested_again(env: &mut Env, id: &str) {
    let rerun = |t: &Ticket| {
        t.pending_decisions()
            .into_iter()
            .find(|d| d.name == "rerun" && d.stage == "try")
            .cloned()
    };
    env.steps_until(id, "the frontend's before", |t, _| {
        t.services
            .iter()
            .any(|s| s.n == 2 && s.state == ServiceState::Before)
    });
    env.step();
    env.step();
    let t = env.ticket(id);
    assert!(
        t.services
            .iter()
            .any(|s| s.n == 2 && s.state == ServiceState::Before),
        "{t:#?}"
    );
    assert!(rerun(&t).is_none(), "asked before the frontend is up");
    serving(env, id, 2);
    env.steps_until(id, "the tester question", |t, _| rerun(t).is_some());
    let t = env.ticket(id);
    assert_eq!(stage_name(&t), "try", "{t:#?}");
    assert_eq!(t.attempts_of("try").count(), 1, "not again unasked");
    let d = rerun(&t).unwrap();
    assert!(d.question.contains("my-dev was let go"), "{}", d.question);
    let now = env.tick();
    env.runner.decide(id, &d.id, "rerun", None, now).unwrap();
    env.steps_until(id, "the second tester", |t, _| {
        t.attempts_of("try").count() == 2
    });
    tester_done(env, id);
    env.steps_until(id, "the tried question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "tried")
    });
    let question = pending_named(env, id, "tried").unwrap().question;
    assert!(question.contains("Served: frontend"), "{question}");
}

#[test]
fn a_deploy_lost_to_a_runner_restart_is_a_question_not_a_rerun() {
    let (mut env, id) = back_half_env(BOTH);
    deploying(&mut env, &id);
    // The runner restarts; its child went with it.
    env.repo
        .lock()
        .unwrap()
        .checks
        .retain(|c| c.key != deploy_key(&id, 1));
    env.restart();
    env.steps_until(&id, "the rerun question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    let t = env.ticket(&id);
    let a = t.attempts_of("deploy").next().unwrap();
    assert!(
        matches!(&a.state, AttemptState::Failed { reason } if reason.contains("it may have run")),
        "{a:#?}"
    );
    assert_eq!(t.attempts_of("deploy").count(), 1);
    assert_eq!(started(&env, &deploy_key(&id, 1)), 0, "not started again");
    assert_eq!(holds(&t), ["my-dev"], "the hold stays with the question");
}

#[test]
fn parking_during_a_deploy_waits_for_it_to_exit_then_releases_the_hold() {
    let (mut env, id) = back_half_env(BOTH);
    deploying(&mut env, &id);
    break_copy(&env, &id);
    env.step();
    env.step();
    let t = env.ticket(&id);
    assert!(is_parking(&t), "{t:#?}");
    assert_eq!(holds(&t), ["my-dev"], "held while the deploy runs");
    assert!(
        !env.repo
            .lock()
            .unwrap()
            .killed_checks
            .contains(&deploy_key(&id, 1)),
        "a deploy is never killed halfway"
    );
    exits(&env, &deploy_key(&id, 1), 0);
    env.step();
    let t = env.ticket(&id);
    assert!(is_parked(&t), "{t:#?}");
    assert!(t.holds.is_empty());
    let a = t.attempts_of("deploy").next().unwrap();
    assert_eq!(a.gate.as_ref().unwrap().exit, Some(0), "its exit is kept");
    assert!(matches!(a.state, AttemptState::Cancelled { .. }));
}

/// Nothing was ever sent to the deploy's group: no kill, no adoption,
/// no escalation, no orphan event.
fn never_signalled(env: &Env, id: &str) {
    let key = deploy_key(id, 1);
    let repo = env.repo.lock().unwrap();
    assert!(
        !repo.killed_checks.contains(&key),
        "{:?}",
        repo.killed_checks
    );
    assert!(!repo.adopted.contains(&key), "{:?}", repo.adopted);
    assert!(
        !repo.escalated_checks.contains(&key),
        "{:?}",
        repo.escalated_checks
    );
    drop(repo);
    assert_eq!(orphan_events(env, id), 0);
}

/// The deploy's group a previous runner left has emptied.
fn lost_deploy_exits(env: &Env, id: &str) {
    env.repo.lock().unwrap().orphans.remove(&deploy_key(id, 1));
}

fn deploy_attempt(t: &Ticket) -> Attempt {
    t.attempts_of("deploy").next().unwrap().clone()
}

fn stuck_questions(env: &Env, id: &str) -> Vec<Decision> {
    env.pending(id)
        .into_iter()
        .filter(|d| d.name == "stuck")
        .collect()
}

/// Parking waits, holding `my-dev`, while the lost deploy's group runs.
fn parking_waits_on_the_lost_deploy(env: &Env, id: &str) {
    let t = env.ticket(id);
    assert!(is_parking(&t), "{t:#?}");
    assert_eq!(holds(&t), ["my-dev"], "held while the lost deploy runs");
    never_signalled(env, id);
}

/// The lost deploy gone, the park ends: nothing held, the attempt
/// cancelled with its group forgotten.
fn parked_after_the_lost_deploy(env: &Env, id: &str) {
    let t = env.ticket(id);
    assert!(is_parked(&t), "{t:#?}");
    assert!(t.holds.is_empty());
    let a = deploy_attempt(&t);
    assert!(matches!(a.state, AttemptState::Cancelled { .. }), "{a:#?}");
    let gate = a.gate.unwrap();
    assert_eq!(gate.group, None);
    assert_eq!(gate.lost_since_ms, None);
    assert_eq!(gate.exit, None, "its exit stays unknown");
}

#[test]
fn parking_after_a_restart_waits_for_a_lost_deploy_and_never_signals_it() {
    let (mut env, id) = back_half_env(BOTH);
    deploying(&mut env, &id);
    orphan_and_restart(&mut env, &deploy_key(&id, 1));
    park_by_hand(&mut env, &id);
    env.step();
    parking_waits_on_the_lost_deploy(&env, &id);
    let since = deploy_attempt(&env.ticket(&id)).gate.unwrap().lost_since_ms;
    assert!(since.is_some(), "the wait is on the record");
    env.step();
    parking_waits_on_the_lost_deploy(&env, &id);
    lost_deploy_exits(&env, &id);
    env.step();
    parked_after_the_lost_deploy(&env, &id);
    never_signalled(&env, &id);
}

#[test]
fn a_restart_while_parking_on_a_deploy_keeps_waiting() {
    let (mut env, id) = back_half_env(BOTH);
    deploying(&mut env, &id);
    break_copy(&env, &id);
    env.step();
    env.step();
    assert!(is_parking(&env.ticket(&id)));
    orphan_and_restart(&mut env, &deploy_key(&id, 1));
    env.step();
    env.step();
    parking_waits_on_the_lost_deploy(&env, &id);
    lost_deploy_exits(&env, &id);
    env.step();
    parked_after_the_lost_deploy(&env, &id);
    never_signalled(&env, &id);
}

#[test]
fn a_lost_deploy_still_running_at_the_stop_limit_asks_stuck() {
    let (mut env, id) = back_half_env(BOTH);
    deploying(&mut env, &id);
    orphan_and_restart(&mut env, &deploy_key(&id, 1));
    park_by_hand(&mut env, &id);
    env.step();
    assert!(
        stuck_questions(&env, &id).is_empty(),
        "not before the limit"
    );
    env.wait(STOP_LIMIT_MS);
    env.step();
    env.step();
    let stuck = stuck_questions(&env, &id);
    assert_eq!(stuck.len(), 1, "{stuck:#?}");
    assert!(stuck[0].question.contains("lost to a runner restart"));
    parking_waits_on_the_lost_deploy(&env, &id);
    answer(&mut env, &id, &stuck[0], "wait");
    env.step();
    env.step();
    assert!(
        stuck_questions(&env, &id).is_empty(),
        "wait gives it longer"
    );
    parking_waits_on_the_lost_deploy(&env, &id);
    env.wait(STOP_LIMIT_MS);
    env.step();
    let stuck = stuck_questions(&env, &id);
    assert_eq!(stuck.len(), 1, "asked again after another limit");
    answer(&mut env, &id, &stuck[0], "released");
    env.step();
    parked_after_the_lost_deploy(&env, &id);
    never_signalled(&env, &id);
}

#[test]
fn a_lost_deploy_that_exits_withdraws_its_stuck() {
    let (mut env, id) = back_half_env(BOTH);
    deploying(&mut env, &id);
    orphan_and_restart(&mut env, &deploy_key(&id, 1));
    park_by_hand(&mut env, &id);
    env.step();
    env.wait(STOP_LIMIT_MS);
    env.step();
    assert_eq!(stuck_questions(&env, &id).len(), 1);
    lost_deploy_exits(&env, &id);
    env.step();
    parked_after_the_lost_deploy(&env, &id);
    let t = env.ticket(&id);
    let stuck = t.decisions.iter().find(|d| d.name == "stuck").unwrap();
    assert_eq!(stuck.state, DecisionState::Cancelled);
    never_signalled(&env, &id);
}

/// The deploy lost to a restart while its group keeps running, failed
/// with "it may have run" and the `rerun` question pending.
fn at_lost_deploy_rerun(env: &mut Env, id: &str) -> Decision {
    deploying(env, id);
    orphan_and_restart(env, &deploy_key(id, 1));
    env.steps_until(id, "the rerun question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    pending_named(env, id, "rerun").unwrap()
}

#[test]
fn a_failed_lost_deploy_still_running_holds_the_park_and_the_rerun() {
    let (mut env, id) = back_half_env(BOTH);
    let rerun = at_lost_deploy_rerun(&mut env, &id);
    answer(&mut env, &id, &rerun, "rerun");
    for _ in 0..3 {
        env.step();
    }
    let t = env.ticket(&id);
    assert_eq!(started(&env, &deploy_key(&id, 2)), 0, "{t:#?}");
    assert_eq!(t.attempts_of("deploy").count(), 1);
    let d = t.decisions.iter().find(|d| d.id == rerun.id).unwrap();
    assert!(d.unacted_answer().is_some(), "{d:#?}");
    never_signalled(&env, &id);
    lost_deploy_exits(&env, &id);
    for _ in 0..3 {
        env.step();
    }
    assert_eq!(started(&env, &deploy_key(&id, 2)), 1);
    never_signalled(&env, &id);

    // Still running at the stop limit, the rerun asks `stuck` once, and
    // `released` lets the second deploy start with the group untouched.
    let (mut env, id) = back_half_env(BOTH);
    let rerun = at_lost_deploy_rerun(&mut env, &id);
    answer(&mut env, &id, &rerun, "rerun");
    env.step();
    env.wait(STOP_LIMIT_MS);
    env.step();
    env.step();
    let stuck = stuck_questions(&env, &id);
    assert_eq!(stuck.len(), 1, "{stuck:#?}");
    assert_eq!(started(&env, &deploy_key(&id, 2)), 0);
    answer(&mut env, &id, &stuck[0], "released");
    for _ in 0..3 {
        env.step();
    }
    assert!(
        env.repo
            .lock()
            .unwrap()
            .orphans
            .contains(&deploy_key(&id, 1))
    );
    assert_eq!(started(&env, &deploy_key(&id, 2)), 1);
    let t = env.ticket(&id);
    let acted = |d: &Decision| matches!(d.state, DecisionState::Answered { acted: true, .. });
    assert!(acted(
        t.decisions.iter().find(|d| d.id == stuck[0].id).unwrap()
    ));
    assert!(acted(
        t.decisions.iter().find(|d| d.id == rerun.id).unwrap()
    ));
    assert_eq!(deploy_attempt(&t).gate.unwrap().group, None);
    never_signalled(&env, &id);

    // A park from the failed state waits the same way.
    let (mut env, id) = back_half_env(BOTH);
    at_lost_deploy_rerun(&mut env, &id);
    park_by_hand(&mut env, &id);
    env.step();
    env.step();
    parking_waits_on_the_lost_deploy(&env, &id);
    lost_deploy_exits(&env, &id);
    env.step();
    let t = env.ticket(&id);
    assert!(is_parked(&t), "{t:#?}");
    assert!(t.holds.is_empty());
    assert_eq!(deploy_attempt(&t).gate.unwrap().group, None);
}

/// A close waits while the lost deploy's group runs: the trees, the
/// hold and the record's progress all stay, and nothing is signalled.
fn closing_waits_on_the_lost_deploy(env: &Env, id: &str) {
    let t = env.ticket(id);
    assert!(matches!(t.state, TicketState::Closing { .. }), "{t:#?}");
    assert!(!t.close.tree_removed, "{:#?}", t.close);
    assert!(env.repo.lock().unwrap().removed.is_empty());
    assert_eq!(holds(&t), ["my-dev"], "held while the lost deploy runs");
    never_signalled(env, id);
}

fn closed_after_the_lost_deploy(env: &mut Env, id: &str) {
    lost_deploy_exits(env, id);
    env.steps_until(id, "closed", |t, _| {
        matches!(t.state, TicketState::Closed { .. })
    });
    let t = env.ticket(id);
    assert!(t.close.tree_removed);
    assert!(!env.repo.lock().unwrap().removed.is_empty());
    assert!(t.holds.is_empty());
    never_signalled(env, id);
}

/// The back half with a gate-only `creds` stage before `deploy`, in
/// `my-dev`'s range, whose secret the deploy may still be using.
fn creds_env() -> (Env, String) {
    let mut env = Env::new();
    let text = BACK_HALF
        .replace("{worktrees}", &env.worktrees.display().to_string())
        .replace(
            "[[stages]]\nname = \"deploy\"\n",
            "[[stages]]\nname = \"creds\"\ncontext = \"lane:backend\"\nneeds = [\"my-dev\"]\nwrites = [{ name = \"creds\", secret = true }]\ngate = { kind = \"command\", in = \"lane:backend\", argv = [\"sh\", \"-c\", \"inv creds\"] }\n\n[[stages]]\nname = \"deploy\"\n",
        );
    std::fs::write(env.data.pipeline("Orchard"), text).unwrap();
    let id = take_orchard(&mut env, 42, BOTH);
    (env, id)
}

/// `creds` writes its secret and exits 0: the path it wrote.
fn creds_written(env: &mut Env, id: &str) -> PathBuf {
    let key = format!("{id}/creds/1");
    for _ in 0..8 {
        if started(env, &key) > 0 {
            break;
        }
        env.step();
    }
    let path = {
        let repo = env.repo.lock().unwrap();
        let check = repo.checks.iter().find(|c| c.key == key).unwrap();
        check
            .env
            .iter()
            .find(|(k, _)| k == "DISPATCH_WRITES_CREDS")
            .map(|(_, v)| PathBuf::from(v))
            .unwrap()
    };
    std::fs::write(&path, TOKEN).unwrap();
    exits(env, &key, 0);
    path
}

fn creds_attempt(t: &Ticket) -> Attempt {
    t.attempts_of("creds").last().unwrap().clone()
}

#[test]
fn a_close_at_the_rerun_question_waits_for_a_lost_deploy_still_running() {
    let (mut env, id) = creds_env();
    let creds = creds_written(&mut env, &id);
    at_lost_deploy_rerun(&mut env, &id);
    let now = env.tick();
    env.runner.close_by_hand(&id, None, now).unwrap();
    for _ in 0..2 {
        closing_waits_on_the_lost_deploy(&env, &id);
        assert!(creds.exists(), "the secret stays while the deploy runs");
        assert!(creds_attempt(&env.ticket(&id)).forgotten.is_empty());
        env.step();
    }
    closed_after_the_lost_deploy(&mut env, &id);
    assert!(!creds.exists());
    assert_eq!(
        creds_attempt(&env.ticket(&id)).forgotten["creds"].why,
        "closed"
    );
}

#[test]
fn a_close_from_parked_waits_for_a_lost_deploy_still_running() {
    let (mut env, id) = back_half_env(BOTH);
    at_lost_deploy_rerun(&mut env, &id);
    // A record an older build parked while the group still ran: its
    // question withdrawn, its hold let go.
    let mut t = env.ticket(&id);
    t.state = TicketState::Parked {
        reason: "parked by hand".into(),
    };
    for d in &mut t.decisions {
        d.state = DecisionState::Cancelled;
    }
    t.holds.clear();
    let now = env.tick();
    env.runner.save_ticket(&mut t, now).unwrap();
    let a = deploy_attempt(&env.ticket(&id));
    assert!(a.gate.as_ref().unwrap().group.is_some() && !a.is_open());
    let now = env.tick();
    env.runner.close_by_hand(&id, None, now).unwrap();
    for _ in 0..2 {
        let t = env.ticket(&id);
        assert!(matches!(t.state, TicketState::Closing { .. }), "{t:#?}");
        assert!(!t.close.tree_removed);
        assert!(env.repo.lock().unwrap().removed.is_empty());
        never_signalled(&env, &id);
        env.step();
    }
    lost_deploy_exits(&env, &id);
    env.steps_until(&id, "closed", |t, _| {
        matches!(t.state, TicketState::Closed { .. })
    });
    assert!(!env.repo.lock().unwrap().removed.is_empty());
}

#[test]
fn a_second_ticket_waits_for_the_resource_without_a_question_or_a_slot_then_takes_it_when_tried_ends()
 {
    let (mut env, a) = back_half_env(BOTH);
    let b = take_orchard(&mut env, 43, BOTH);
    deploying(&mut env, &a);
    for _ in 0..3 {
        env.step();
    }
    let tb = env.ticket(&b);
    assert_eq!(stage_name(&tb), "deploy");
    assert!(tb.holds.is_empty() && tb.attempts_of("deploy").next().is_none());
    assert!(tb.pending_decisions().is_empty(), "waiting asks nothing");
    let status = dispatch::serve::status(&env.runner).unwrap();
    let view = status.tickets.iter().find(|v| v.id == b).unwrap();
    assert_eq!(
        view.waiting_for.as_deref(),
        Some(format!("my-dev, held by {a} (#42) from deploy").as_str())
    );
    let orchard = status
        .projects
        .iter()
        .find(|p| p.name == "Orchard")
        .unwrap();
    assert_eq!(orchard.running, 1, "the waiting ticket costs no slot");
    exits(&env, &deploy_key(&a, 1), 0);
    at_stage(&mut env, &a, "try");
    served(&mut env, &a);
    tester_done(&mut env, &a);
    assert!(env.ticket(&b).holds.is_empty(), "held through tried");
    tried(&mut env, &a, "done");
    env.steps_until(&b, "the second ticket's hold", |t, _| !t.holds.is_empty());
    let ta = env.ticket(&a);
    assert!(ta.holds.is_empty());
    let session = ta.services[0].session.clone().unwrap();
    assert_eq!(ta.services[0].state, ServiceState::Stopped);
    assert!(env.sb().removed_by_port.contains(&session));
    assert!(!ta.processes.contains(&session));
    deploying(&mut env, &b);
}

#[test]
fn a_ticket_queued_for_the_stack_waits_through_a_tried_rerun() {
    let (mut env, a) = back_half_env(BOTH);
    let b = take_orchard(&mut env, 43, BOTH);
    at_tried(&mut env, &a);
    tried_rerun(&mut env, &a, "again");
    for _ in 0..3 {
        env.step();
        assert!(env.ticket(&b).holds.is_empty(), "held through the rerun");
    }
    restarting(&mut env, &a, 2);
    answering_again(&mut env, &a, 2);
    assert!(env.ticket(&b).holds.is_empty());
    tester_done(&mut env, &a);
    assert!(env.ticket(&b).holds.is_empty(), "held through tried");
    tried(&mut env, &a, "done");
    env.steps_until(&b, "the second ticket's hold", |t, _| !t.holds.is_empty());
    assert!(env.ticket(&a).holds.is_empty());
    deploying(&mut env, &b);
}

// --- two runs: the implementer holds `my-dev` too, lets go for the
// look, and the deploy takes it again.

/// `BACK_HALF` with an `implement` stage holding `my-dev` after `lanes`
/// and a `look` that holds nothing, standing in for a code review: two
/// runs of `my-dev`, `implement` and `deploy`..`tried`.
const IMPLEMENT_STAGE: &str = "[[stages]]\nname = \"implement\"\noperator = \"implementer\"\ncontext = \"each\"\nneeds = [\"my-dev\"]\nwrites = [\"notes\"]\nprompt = \"Implement the {lane} part on {branch}; notes to {notes}.\"\n\n[[stages]]\nname = \"look\"\ngate = { kind = \"human\", decision = \"look\" }\n\n[[stages]]\nname = \"deploy\"\n";

fn two_runs_text(env: &Env) -> String {
    let text = BACK_HALF
        .replace("{worktrees}", &env.worktrees.display().to_string())
        .replace("[[stages]]\nname = \"deploy\"\n", IMPLEMENT_STAGE);
    assert!(text.contains("name = \"implement\""));
    text
}

fn two_runs_env() -> (Env, String) {
    let mut env = Env::new();
    let text = two_runs_text(&env);
    std::fs::write(env.data.pipeline("Orchard"), text).unwrap();
    let id = take_orchard(&mut env, 42, BOTH);
    (env, id)
}

/// Each lane's implementer launched, or still complete from before a
/// send-back of the other: the ticket holds `my-dev`.
fn implementing(env: &mut Env, id: &str) {
    env.steps_until(id, "the implementers", |t, _| {
        let latest = |lane: &str| {
            t.attempts_of("implement")
                .filter(|a| a.context == lane)
                .last()
        };
        t.attempts_of("implement").any(Attempt::is_open)
            && ["backend", "frontend"].iter().all(|l| {
                latest(l).is_some_and(|a| a.is_open() || a.state == AttemptState::Complete)
            })
    });
}

/// Every open `implement` attempt finishes, one per lane, and the
/// ticket stands at `look` with its question asked, past the pass that
/// let `my-dev` go.
fn implemented(env: &mut Env, id: &str) {
    implementing(env, id);
    let t = env.ticket(id);
    let open: Vec<Attempt> = t
        .attempts_of("implement")
        .filter(|a| a.is_open())
        .cloned()
        .collect();
    for a in open {
        env.finish(
            a.session.as_deref().unwrap(),
            &a.artifacts["notes"],
            "# notes\ndone",
        );
    }
    env.steps_until(id, "the look question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "look")
    });
}

fn implement_states(t: &Ticket) -> Vec<AttemptState> {
    t.attempts_of("implement")
        .map(|a| a.state.clone())
        .collect()
}

fn waiting_text(env: &Env, id: &str) -> Option<String> {
    let status = dispatch::serve::status(&env.runner).unwrap();
    status
        .tickets
        .iter()
        .find(|v| v.id == id)
        .unwrap()
        .waiting_for
        .clone()
}

#[test]
fn a_second_ticket_waits_at_implement_and_the_first_waits_again_at_deploy() {
    let (mut env, a) = two_runs_env();
    implementing(&mut env, &a);
    let b = take_orchard(&mut env, 43, BOTH);
    for _ in 0..4 {
        env.step();
    }
    let ta = env.ticket(&a);
    assert_eq!(holds(&ta), ["my-dev"]);
    assert_eq!(ta.holds[0].stage, "implement");
    let tb = env.ticket(&b);
    assert_eq!(stage_name(&tb), "implement");
    assert!(tb.holds.is_empty());
    assert!(tb.attempts_of("implement").next().is_none(), "{tb:#?}");
    assert!(tb.attempts_of(REFRESH).next().is_none());
    assert_ne!(
        tb.refreshed_stage,
        Some(tb.stage),
        "nothing brought up while waiting"
    );
    assert!(tb.pending_decisions().is_empty(), "waiting asks nothing");
    assert_eq!(
        waiting_text(&env, &b).as_deref(),
        Some(format!("my-dev, held by {a} (#42) from implement").as_str())
    );
    let status = dispatch::serve::status(&env.runner).unwrap();
    let orchard = status
        .projects
        .iter()
        .find(|p| p.name == "Orchard")
        .unwrap();
    assert_eq!(orchard.running, 1, "the waiting ticket costs no slot");

    implemented(&mut env, &a);
    assert!(
        env.ticket(&a).holds.is_empty(),
        "let go at the end of the run"
    );
    implementing(&mut env, &b);
    assert_eq!(env.ticket(&b).holds[0].stage, "implement");

    answer_named(&mut env, &a, "look", "proceed");
    at_stage(&mut env, &a, "deploy");
    for _ in 0..3 {
        env.step();
    }
    let ta = env.ticket(&a);
    assert!(ta.holds.is_empty());
    assert_eq!(started(&env, &deploy_key(&a, 1)), 0, "waits for the hold");
    assert_eq!(stage_name(&ta), "deploy", "not sent back to implement");
    assert!(
        implement_states(&ta)
            .iter()
            .all(|s| *s == AttemptState::Complete),
        "{ta:#?}"
    );
    assert_eq!(
        waiting_text(&env, &a).as_deref(),
        Some(format!("my-dev, held by {b} (#43) from implement").as_str())
    );

    implemented(&mut env, &b);
    deploying(&mut env, &a);
    let ta = env.ticket(&a);
    assert_eq!(holds(&ta), ["my-dev"]);
    assert_eq!(ta.holds[0].stage, "deploy");
    assert!(
        implement_states(&ta)
            .iter()
            .all(|s| *s == AttemptState::Complete),
        "{ta:#?}"
    );
}

/// The implement-only operator's own allow rules (its deploy and its
/// dev server) go on the command line ahead of the rule that lets it
/// write its notes, each behind its own `--allowedTools`.
#[test]
fn the_implementers_own_allowed_tools_go_beside_its_write_rule() {
    let mut env = Env::new();
    let text = two_runs_text(&env)
        .replace(
            "name = \"implement\"\noperator = \"implementer\"",
            "name = \"implement\"\noperator = \"deployer\"",
        )
        .replace(
            "[operators.implementer]\n",
            "[operators.deployer]\nkind = \"claude\"\nargs = [\"--allowedTools\", \"Bash(inv deploy:*)\", \"--allowedTools\", \"Bash(npm start:*)\"]\n\n[operators.implementer]\n",
        );
    std::fs::write(env.data.pipeline("Orchard"), text).unwrap();
    let id = take_orchard(&mut env, 42, BOTH);
    implementing(&mut env, &id);
    let t = env.ticket(&id);
    let notes = t.attempts_of("implement").next().unwrap().artifacts["notes"].clone();
    let dir = notes.parent().unwrap().display().to_string();
    let argvs: Vec<Vec<String>> = env
        .sb()
        .calls
        .iter()
        .filter_map(|r| match &r.body {
            Body::SessionNew {
                name,
                launch: switchboard_control::Launch::Argv(a),
                ..
            } if name == "deployer" => Some(a.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(argvs.len(), 2, "one per lane");
    let backend = argvs
        .iter()
        .find(|a| a.iter().any(|x| x.contains(&dir)))
        .unwrap();
    assert_eq!(
        backend,
        &[
            "--allowedTools".to_owned(),
            "Bash(inv deploy:*)".into(),
            "--allowedTools".into(),
            "Bash(npm start:*)".into(),
            "--allowedTools".into(),
            format!("Edit(//{dir}/**)"),
        ]
    );
}

#[test]
fn parking_at_implement_lets_go_and_a_resume_asks_rerun_there() {
    let (mut env, id) = two_runs_env();
    implementing(&mut env, &id);
    let now = env.tick();
    env.runner.request_park(&id, Some("for now"), now).unwrap();
    env.steps_until(&id, "parked", |t, _| is_parked(t));
    let t = env.ticket(&id);
    assert!(t.holds.is_empty());
    assert!(
        implement_states(&t)
            .iter()
            .all(|s| matches!(s, AttemptState::Cancelled { .. })),
        "{t:#?}"
    );
    let now = env.tick();
    let resumed = env.runner.resume(&id, now).unwrap();
    let reruns: Vec<&str> = resumed.reruns.iter().map(|a| a.stage.as_str()).collect();
    assert_eq!(reruns, ["implement", "implement"], "one per lane");
    env.steps_until(&id, "the second implementers", |t, _| {
        t.attempts_of("implement").filter(|a| a.is_open()).count() == 2
    });
    let t = env.ticket(&id);
    assert_eq!(stage_name(&t), "implement");
    assert_eq!(holds(&t), ["my-dev"]);
    assert_eq!(t.holds[0].stage, "implement");
    assert!(
        t.attempts_of("lanes")
            .all(|a| a.state == AttemptState::Complete)
    );
}

/// `ticket`'s backend lane is one commit behind a base that moved to
/// `base`, with commits of its own; with `conflict` its rebase stops.
fn backend_base_moves(env: &Env, ticket: &str, base: &str, conflict: bool) -> PathBuf {
    let tree = lane_tree(env, ticket, "backend");
    let mut repo = env.repo.lock().unwrap();
    repo.heads.insert(tree.clone(), "impl0001".into());
    repo.bases
        .insert(env.data.lane_repo_dir("Orchard", "backend"), base.into());
    repo.behind.insert(tree.clone(), 1);
    if conflict {
        repo.rebase_conflicts.push(tree.clone());
    }
    tree
}

fn rebases_of(env: &Env, tree: &PathBuf) -> usize {
    env.repo
        .lock()
        .unwrap()
        .rebased
        .iter()
        .filter(|(d, _)| d == tree)
        .count()
}

/// A ticket waiting at `implement` for `my-dev` while its backend's
/// base moves: the first ticket and the second, and the backend tree.
fn waiting_while_the_base_moves(conflict: bool) -> (Env, String, String, PathBuf) {
    let (mut env, a) = two_runs_env();
    implementing(&mut env, &a);
    let b = take_orchard(&mut env, 43, BOTH);
    env.steps_until(&b, "the second ticket at implement", |t, _| {
        stage_name(t) == "implement"
    });
    let tree = backend_base_moves(&env, &b, "main0002", conflict);
    let base_before = env
        .ticket(&b)
        .lanes
        .iter()
        .find(|l| l.name == "backend")
        .unwrap()
        .base_sha
        .clone();
    for _ in 0..4 {
        env.step();
    }
    let tb = env.ticket(&b);
    assert_eq!(rebases_of(&env, &tree), 0, "nothing moves while it waits");
    assert_ne!(tb.refreshed_stage, Some(tb.stage));
    assert!(tb.attempts_of(REFRESH).next().is_none());
    assert!(tb.pending_decisions().is_empty(), "{tb:#?}");
    assert_eq!(
        tb.lanes
            .iter()
            .find(|l| l.name == "backend")
            .unwrap()
            .base_sha,
        base_before
    );
    (env, a, b, tree)
}

#[test]
fn implement_brings_its_lanes_up_once_it_holds_the_stack() {
    let (mut env, a, b, tree) = waiting_while_the_base_moves(false);
    implemented(&mut env, &a);
    implementing(&mut env, &b);
    let tb = env.ticket(&b);
    assert_eq!(holds(&tb), ["my-dev"]);
    assert_eq!(rebases_of(&env, &tree), 1);
    assert_eq!(tb.refreshed_stage, Some(tb.stage));
    assert_eq!(stage_name(&tb), "implement");
    let backend = tb.lanes.iter().find(|l| l.name == "backend").unwrap();
    assert_eq!(backend.base_sha.as_deref(), Some("main0002"));
    let prompt = env
        .sb()
        .calls
        .iter()
        .rev()
        .find_map(|r| match &r.body {
            Body::SessionNew { prompt, .. }
                if prompt
                    .as_deref()
                    .is_some_and(|p| p.contains("Implement the backend part")) =>
            {
                prompt.clone()
            }
            _ => None,
        })
        .unwrap();
    assert!(prompt.contains("the base moved from"), "{prompt}");

    // Past the first stage of its run, `try` reads what was deployed and
    // is not moved under it.
    implemented(&mut env, &b);
    answer_named(&mut env, &b, "look", "proceed");
    backend_base_moves(&env, &b, "main0003", false);
    deployed(&mut env, &b);
    for _ in 0..3 {
        env.step();
    }
    assert_eq!(stage_name(&env.ticket(&b)), "try");
    assert_eq!(rebases_of(&env, &tree), 1, "try does not rebase");
}

/// The fixture names no rebaser, so a conflicting rebase is a `refresh`
/// question; a rebaser would start at the same point.
#[test]
fn a_conflict_at_implement_is_asked_only_once_the_stack_is_held() {
    let (mut env, a, b, _) = waiting_while_the_base_moves(true);
    implemented(&mut env, &a);
    env.steps_until(&b, "the refresh question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == REFRESH)
    });
    let tb = env.ticket(&b);
    assert_eq!(holds(&tb), ["my-dev"]);
    assert!(tb.attempts_of("implement").next().is_none());
}

#[test]
fn a_send_back_into_implement_keeps_the_hold_and_names_implement() {
    let mut env = Env::new();
    let text = two_runs_text(&env).replace(
        "[[stages]]\nname = \"try\"\n",
        "[[stages]]\nname = \"checked\"\ncontext = \"lane:backend\"\nneeds = [\"my-dev\"]\ngate = { kind = \"human\", decision = \"checked\" }\n\n[[stages]]\nname = \"try\"\n",
    );
    std::fs::write(env.data.pipeline("Orchard"), text).unwrap();
    let a = take_orchard(&mut env, 42, BOTH);
    implemented(&mut env, &a);
    answer_named(&mut env, &a, "look", "proceed");
    deploying(&mut env, &a);
    exits(&env, &deploy_key(&a, 1), 0);
    env.steps_until(&a, "the checked question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "checked")
    });
    assert_eq!(env.ticket(&a).holds[0].stage, "deploy");
    let b = take_orchard(&mut env, 43, BOTH);
    env.steps_until(&b, "the second ticket at implement", |t, _| {
        stage_name(t) == "implement"
    });
    assert_eq!(
        waiting_text(&env, &b).as_deref(),
        Some(format!("my-dev, held by {a} (#42) from deploy").as_str())
    );
    answer_named(&mut env, &a, "checked", "rerun");
    env.step();
    let ta = env.ticket(&a);
    assert_eq!(stage_name(&ta), "implement");
    assert_eq!(holds(&ta), ["my-dev"], "kept, not dropped");
    assert_eq!(ta.holds[0].stage, "implement");
    assert!(env.ticket(&b).holds.is_empty());
    assert_eq!(
        waiting_text(&env, &b).as_deref(),
        Some(format!("my-dev, held by {a} (#42) from implement").as_str())
    );
    // The backend reruns; the frontend's result stands, and `look`
    // passed before, so leaving `implement` lets go for `deploy`.
    implementing(&mut env, &a);
    let open = env
        .ticket(&a)
        .attempts_of("implement")
        .find(|x| x.is_open())
        .cloned()
        .unwrap();
    assert_eq!(open.context, "backend");
    env.finish(
        open.session.as_deref().unwrap(),
        &open.artifacts["notes"],
        "# notes\nagain",
    );
    env.steps_until(&b, "the second ticket's hold", |t, _| !t.holds.is_empty());
    assert!(env.ticket(&a).holds.is_empty());
    implementing(&mut env, &b);
}

#[test]
fn a_review_stage_holding_a_resource_runs_on_in_a_ticket_already_taken() {
    let (mut env, id) = two_runs_env();
    implementing(&mut env, &id);
    let copy = env.ticket(&id).pipeline_file;
    let old = std::fs::read_to_string(&copy).unwrap();
    let held_review = old.replace(
        "[[stages]]\nname = \"look\"\ngate = { kind = \"human\", decision = \"look\" }\n",
        "[[stages]]\nname = \"look\"\ncontext = \"each\"\nneeds = [\"my-dev\"]\nreviewers = [\"tester\"]\nimplementer = \"implementer\"\ngate = { kind = \"command\", argv = [\"sh\", \"-c\", \"check\"] }\n\n[[stages]]\nname = \"between\"\ngate = { kind = \"human\", decision = \"between\" }\n",
    );
    assert_ne!(held_review, old);
    std::fs::write(&copy, &held_review).unwrap();
    for _ in 0..3 {
        env.step();
    }
    let t = env.ticket(&id);
    assert!(t.active(), "{:?}", t.state);
    assert_eq!(stage_name(&t), "implement");

    let now = env.tick();
    let source = env.ticket(&id).source;
    let e = env
        .runner
        .take("Orchard", &held_review, source, now)
        .unwrap_err();
    assert!(
        format!("{e:#}").contains("\"look\" is a code review stage and may not hold \"my-dev\""),
        "{e:#}"
    );
    std::fs::write(env.data.pipeline("Orchard"), &held_review).unwrap();
    let before = env.ticket(&id);
    let e = restart_refused(&mut env, &id, None);
    assert!(e.contains("is a code review stage and may not hold"), "{e}");
    assert_eq!(env.ticket(&id), before, "nothing written");
}

#[test]
fn a_resume_at_tried_goes_back_to_deploy_not_implement() {
    let (mut env, id) = two_runs_env();
    implemented(&mut env, &id);
    answer_named(&mut env, &id, "look", "proceed");
    at_tried(&mut env, &id);
    tried(&mut env, &id, "park");
    env.step();
    assert!(is_parked(&env.ticket(&id)));
    let now = env.tick();
    env.runner.resume(&id, now).unwrap();
    env.steps_until(&id, "the deploy question", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == "rerun" && d.stage == "deploy")
    });
    let t = env.ticket(&id);
    assert_eq!(stage_name(&t), "deploy");
    assert_eq!(holds(&t), ["my-dev"]);
    // Retaken where the ticket stood, then rewound within the run.
    assert_eq!(t.holds[0].stage, "tried");
    assert!(
        implement_states(&t)
            .iter()
            .all(|s| *s == AttemptState::Complete),
        "{t:#?}"
    );
    assert!(
        t.attempts_of("look")
            .all(|a| a.state == AttemptState::Complete)
    );
}

#[test]
fn parking_and_closing_release_the_hold_after_the_service_is_gone() {
    let (mut env, id) = back_half_env(BOTH);
    deployed(&mut env, &id);
    served(&mut env, &id);
    let session = env.ticket(&id).services[0].session.clone().unwrap();
    // The service survives its first kill, and then leaves its port
    // taken (a forked dev server that outlived the pane).
    env.sb().fail_next = Some("session.kill".into());
    env.repo.lock().unwrap().busy_ports.insert(3100);
    break_copy(&env, &id);
    env.step();
    let t = env.ticket(&id);
    assert!(is_parking(&t), "{t:#?}");
    assert_eq!(holds(&t), ["my-dev"]);
    env.step();
    assert_ne!(env.sb().session(&session).liveness, Liveness::Running);
    let t = env.ticket(&id);
    assert!(is_parking(&t), "the port is still taken: {t:#?}");
    assert_eq!(holds(&t), ["my-dev"]);
    assert!(
        env.sb().removed_by_port.is_empty(),
        "not removed while held"
    );
    env.repo.lock().unwrap().busy_ports.clear();
    env.step();
    let t = env.ticket(&id);
    assert!(is_parked(&t), "{t:#?}");
    assert!(t.holds.is_empty());
    assert_eq!(t.services[0].state, ServiceState::Stopped);
    assert_eq!(env.sb().removed_by_port, [session]);

    // A close at `tried`, the frontend still served.
    let (mut env, id) = back_half_env(BOTH);
    at_tried(&mut env, &id);
    let session = env.ticket(&id).services[0].session.clone().unwrap();
    env.repo.lock().unwrap().busy_ports.insert(3100);
    let now = env.tick();
    let t = env.runner.close_by_hand(&id, None, now).unwrap();
    assert!(matches!(t.state, TicketState::Closing { .. }), "{t:#?}");
    assert_eq!(holds(&t), ["my-dev"]);
    env.step();
    assert!(matches!(env.ticket(&id).state, TicketState::Closing { .. }));
    env.repo.lock().unwrap().busy_ports.clear();
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Closed { .. }), "{t:#?}");
    assert!(t.holds.is_empty());
    assert_eq!(env.sb().removed_by_port, [session]);
}

#[test]
fn a_refresh_question_after_tried_does_not_keep_the_hold() {
    let (mut env, a) = back_half_env(BOTH);
    let b = take_orchard(&mut env, 43, BOTH);
    at_tried(&mut env, &a);
    // The backend's base moves and the rebase conflicts; the pipeline
    // names no rebaser, so `after` asks.
    {
        let mut repo = env.repo.lock().unwrap();
        let clone = env.data.lane_repo_dir("Orchard", "backend");
        repo.bases.insert(clone, "main0002".into());
        let tree = lane_tree(&env, &a, "backend");
        repo.behind.insert(tree.clone(), 1);
        repo.rebase_conflicts.push(tree);
    }
    tried(&mut env, &a, "done");
    env.steps_until(&b, "the second ticket's hold", |t, _| !t.holds.is_empty());
    let ta = env.ticket(&a);
    assert_eq!(stage_name(&ta), "after");
    assert!(
        ta.pending_decisions().iter().any(|d| d.name == "refresh"),
        "{ta:#?}"
    );
    assert!(ta.holds.is_empty());
    assert_eq!(ta.services[0].state, ServiceState::Stopped);
}

/// The tree, the clone it is a worktree of, and its branch.
fn orchard_tree(env: &Env, id: &str) -> (PathBuf, PathBuf, String) {
    let tree = env.ticket(id).tree.unwrap();
    let clone = env.data.repo_dir("Orchard");
    let branch = env
        .repo
        .lock()
        .unwrap()
        .worktrees
        .iter()
        .find(|(_, d, ..)| *d == tree)
        .map(|(_, _, b, _)| b.clone())
        .unwrap();
    (tree, clone, branch)
}

/// The project's base moves two commits past the ticket's tree.
fn move_orchard_base(env: &Env, id: &str) -> PathBuf {
    let (tree, clone, _) = orchard_tree(env, id);
    let mut repo = env.repo.lock().unwrap();
    repo.bases.insert(clone, "main0002".into());
    repo.behind.insert(tree.clone(), 2);
    repo.rebase_heads.insert(tree.clone(), "main0002".into());
    tree
}

/// Every rebase the fake git ran, as (directory, onto).
fn env_rebased(env: &Env) -> Vec<(PathBuf, String)> {
    env.repo.lock().unwrap().rebased.clone()
}

/// The `refreshed` events of `id` in the log, as (stage, text).
fn refreshed_events(env: &Env, id: &str) -> Vec<(String, String)> {
    dispatch::events::read_since(&dispatch::events::log_path(&env.data), 0)
        .unwrap()
        .into_iter()
        .filter(|e| e.ticket == id && e.kind == dispatch::events::Kind::Refreshed)
        .map(|e| (e.stage, e.text))
        .collect()
}

/// The joined tester is past the lane skip rule (it follows the deploy
/// in a `needs` run and serves a lane), but the tree is not a lane: it
/// is brought up to the moved base before the services and the tester
/// start, the lanes stay where they were deployed, and the bring-up is
/// a `refreshed` event naming `root`.
#[test]
fn a_joined_tester_runs_on_a_tree_brought_up_to_the_moved_base() {
    let (mut env, id) = back_half_env(BOTH);
    deploying(&mut env, &id);
    let tree = move_orchard_base(&env, &id);
    exits(&env, &deploy_key(&id, 1), 0);
    at_stage(&mut env, &id, "try");
    for _ in 0..12 {
        if !env_rebased(&env).is_empty() {
            break;
        }
        env.step();
    }
    assert_eq!(
        env_rebased(&env),
        vec![(tree.clone(), "origin/main".to_owned())]
    );
    assert!(env.ticket(&id).attempts_of("try").next().is_none());
    served(&mut env, &id);
    let t = env.ticket(&id);
    let up = t.tree_refreshed.clone().expect("the bring-up is recorded");
    assert_eq!(
        (
            up.from.as_str(),
            up.to.as_str(),
            up.commits,
            up.after.as_deref()
        ),
        ("base0000", "main0002", false, Some("main0002"))
    );
    assert_eq!(
        env_rebased(&env),
        vec![(tree, "origin/main".to_owned())],
        "the lanes are not rebased"
    );
    let events = refreshed_events(&env, &id);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].0, "try");
    assert!(
        events[0].1.starts_with("root from base000 to main000"),
        "{}",
        events[0].1
    );
}

/// An uncommitted file in the tree leaves it where it is and the stage
/// runs; a change inside a nested lane, as the tree or the lane's own
/// worktree reports it, is the lane's, not the tree's.
#[test]
fn a_tree_with_an_uncommitted_file_is_not_moved() {
    let (mut env, id) = back_half_env(BOTH);
    deploying(&mut env, &id);
    let tree = move_orchard_base(&env, &id);
    env.repo
        .lock()
        .unwrap()
        .changes
        .insert(tree.clone(), vec!["tools/new.sh".into()]);
    exits(&env, &deploy_key(&id, 1), 0);
    at_stage(&mut env, &id, "try");
    served(&mut env, &id);
    assert!(env_rebased(&env).is_empty());
    assert_eq!(env.ticket(&id).tree_refreshed, None);

    let (mut env, id) = back_half_env(BOTH);
    deploying(&mut env, &id);
    let tree = move_orchard_base(&env, &id);
    {
        let backend = lane_tree(&env, &id, "backend");
        let mut repo = env.repo.lock().unwrap();
        repo.changes
            .insert(tree.clone(), vec!["orchard-backend/x.py".into()]);
        repo.changes.insert(backend, vec!["x.py".into()]);
    }
    exits(&env, &deploy_key(&id, 1), 0);
    at_stage(&mut env, &id, "try");
    served(&mut env, &id);
    assert_eq!(env_rebased(&env), vec![(tree, "origin/main".to_owned())]);
    assert!(env.ticket(&id).tree_refreshed.is_some());
}

/// A tree with commits of its own whose rebase conflicts is left where
/// it was: no rebaser, no question, and the tester still runs. Without
/// the conflict the same tree is rebased and recorded as having
/// commits.
#[test]
fn a_tree_with_commits_of_its_own_that_conflicts_is_left_and_the_stage_runs() {
    for conflicts in [true, false] {
        let (mut env, id) = back_half_env(BOTH);
        deploying(&mut env, &id);
        let tree = move_orchard_base(&env, &id);
        {
            let (_, clone, branch) = orchard_tree(&env, &id);
            let mut repo = env.repo.lock().unwrap();
            repo.branches.get_mut(&(clone, branch)).unwrap().1 = 1;
            if conflicts {
                repo.rebase_conflicts.push(tree.clone());
            }
        }
        exits(&env, &deploy_key(&id, 1), 0);
        at_stage(&mut env, &id, "try");
        served(&mut env, &id);
        let t = env.ticket(&id);
        assert_eq!(env_rebased(&env), vec![(tree, "origin/main".to_owned())]);
        assert!(t.pending_decisions().is_empty(), "{t:#?}");
        assert!(t.attempts_of(dispatch::scheduler::REFRESH).next().is_none());
        if conflicts {
            assert_eq!(t.tree_refreshed, None);
        } else {
            assert!(t.tree_refreshed.unwrap().commits);
        }
    }
}

/// A restart to the tester puts the tree back at the head it entered
/// with and the record back to the bring-up it had then; on re-entry
/// the tree is brought up again and logged again.
#[test]
fn a_restart_to_the_tester_resets_the_tree_and_brings_it_up_again() {
    let (mut env, id) = back_half_env(BOTH);
    deploying(&mut env, &id);
    let tree = move_orchard_base(&env, &id);
    let before = env.repo.lock().unwrap().heads[&tree].clone();
    exits(&env, &deploy_key(&id, 1), 0);
    at_stage(&mut env, &id, "try");
    served(&mut env, &id);
    tester_done(&mut env, &id);
    let t = restart_at(&mut env, &id, Some("try"));
    let reset: Vec<(&str, &str, &str)> = t.restarts[0]
        .reset
        .iter()
        .map(|h| (h.key.as_str(), h.from.as_str(), h.to.as_str()))
        .collect();
    assert_eq!(reset, vec![("root", "main0002", before.as_str())]);
    assert_eq!(t.tree_refreshed, None, "as it was at the tester's entry");
    assert_eq!(refreshed_events(&env, &id).len(), 1);

    env.repo.lock().unwrap().behind.insert(tree.clone(), 2);
    for _ in 0..40 {
        let t = env.ticket(&id);
        if t.attempts_of("try")
            .any(|a| a.n == 2 && a.session.is_some())
        {
            break;
        }
        if let Some(d) = t
            .pending_decisions()
            .into_iter()
            .find(|d| d.name == "rerun")
            .cloned()
        {
            answer(&mut env, &id, &d, "rerun");
        }
        let n = t.services.iter().map(|s| s.n).max().unwrap_or(0);
        exits(&env, &before_key(&id, n), 0);
        if let Some(port) = t.services.iter().find(|s| s.n == n).and_then(|s| s.port) {
            env.repo.lock().unwrap().answering.insert(port);
        }
        env.step();
    }
    let t = env.ticket(&id);
    assert!(
        t.attempts_of("try")
            .any(|a| a.n == 2 && a.session.is_some()),
        "{t:#?}"
    );
    let repo = env.repo.lock().unwrap();
    assert_eq!(
        repo.rebased,
        vec![
            (tree.clone(), "origin/main".to_owned()),
            (tree.clone(), "origin/main".to_owned())
        ]
    );
    assert_eq!(repo.heads[&tree], "main0002");
    drop(repo);
    assert!(t.tree_refreshed.is_some());
    assert_eq!(refreshed_events(&env, &id).len(), 2);
}

/// The tester's entry could not read the tree's head, so a restart to
/// the tester leaves the tree where it is: its bring-up stays on the
/// record, since that still describes the head it has.
#[test]
fn a_restart_without_the_trees_entry_head_keeps_its_bring_up() {
    let (mut env, id) = back_half_env(BOTH);
    deploying(&mut env, &id);
    move_orchard_base(&env, &id);
    exits(&env, &deploy_key(&id, 1), 0);
    at_stage(&mut env, &id, "try");
    served(&mut env, &id);
    tester_done(&mut env, &id);
    let mut t = env.ticket(&id);
    let up = t.tree_refreshed.clone().expect("the bring-up is recorded");
    for e in t.entered.iter_mut().filter(|e| e.stage == "try") {
        e.heads.remove("root");
    }
    dispatch::store::write_ticket(&env.data.ticket_file(&id), &t).unwrap();
    let t = restart_at(&mut env, &id, Some("try"));
    assert!(
        t.restarts[0].reset.iter().all(|h| h.key != "root"),
        "{t:#?}"
    );
    assert_eq!(t.tree_refreshed, Some(up));
}

#[test]
fn a_stage_needing_a_lane_parks_as_not_built() {
    let mut env = Env::new();
    let text = BACK_HALF
        .replace("{worktrees}", &env.worktrees.display().to_string())
        .replace(
            "context = \"lane:backend\"\nneeds = [\"my-dev\"]",
            "context = \"lane:backend\"\nneeds = [\"backend\"]",
        );
    std::fs::write(env.data.pipeline("Orchard"), text).unwrap();
    let id = take_orchard(&mut env, 42, BOTH);
    env.steps_until(&id, "parked", |t, _| is_parked(t));
    let t = env.ticket(&id);
    assert!(
        matches!(&t.state, TicketState::Parked { reason }
            if reason == "stage deploy needs lane backend (an in-place hold), which is not built in this slice"),
        "{t:#?}"
    );
    assert!(t.attempts_of("deploy").next().is_none());
}

#[test]
fn a_hold_survives_a_runner_restart() {
    let (mut env, a) = back_half_env(BOTH);
    let b = take_orchard(&mut env, 43, BOTH);
    deploying(&mut env, &a);
    env.restart();
    for _ in 0..3 {
        env.step();
    }
    assert_eq!(holds(&env.ticket(&a)), ["my-dev"]);
    let tb = env.ticket(&b);
    assert!(tb.holds.is_empty(), "{tb:#?}");
    assert!(tb.attempts_of("deploy").next().is_none());
    let status = dispatch::serve::status(&env.runner).unwrap();
    let view = status.tickets.iter().find(|v| v.id == b).unwrap();
    assert!(view.waiting_for.is_some());
}

#[test]
fn a_ticket_holding_a_resource_costs_a_slot_and_status_agrees() {
    let (mut env, a) = back_half_env(BOTH);
    let live = std::fs::read_to_string(env.data.pipeline("Orchard"))
        .unwrap()
        .replace("slots = 2", "slots = 1");
    std::fs::write(env.data.pipeline("Orchard"), live).unwrap();
    at_tried(&mut env, &a);
    // A frontend-only ticket skips the deploy and would start its
    // tester, but the slot is the holder's.
    let b = take_orchard(&mut env, 43, &["area:frontend"]);
    for _ in 0..6 {
        env.step();
    }
    let tb = env.ticket(&b);
    assert_eq!(stage_name(&tb), "try", "{tb:#?}");
    assert!(tb.holds.is_empty() && tb.services.is_empty());
    assert_eq!(
        env.sb().sessions_named("tester").len(),
        1,
        "only the holder's"
    );
    let status = dispatch::serve::status(&env.runner).unwrap();
    let orchard = status
        .projects
        .iter()
        .find(|p| p.name == "Orchard")
        .unwrap();
    assert_eq!(orchard.running, 1);
    assert_eq!(orchard.slots, 1);
}

#[test]
fn services_start_after_before_on_a_free_port_and_the_tester_is_told_the_url() {
    let (mut env, id) = back_half_env(BOTH);
    deployed(&mut env, &id);
    for _ in 0..2 {
        env.step();
    }
    let frontend = lane_tree(&env, &id, "frontend");
    {
        let repo = env.repo.lock().unwrap();
        let check = repo
            .checks
            .iter()
            .find(|c| c.key == before_key(&id, 1))
            .unwrap();
        assert_eq!(check.dir, frontend);
        assert_eq!(check.argv, ["npm", "run", "link-env"]);
        assert!(check.log.ends_with("try/services/frontend/1/before.log"));
    }
    assert!(service_launches(&env).is_empty(), "before runs first");
    exits(&env, &before_key(&id, 1), 0);
    env.step();
    let launches = service_launches(&env);
    assert_eq!(launches.len(), 1);
    let Body::SessionNew {
        name, cwd, launch, ..
    } = &launches[0].body
    else {
        unreachable!()
    };
    assert_eq!(name, "frontend :3100");
    assert_eq!(cwd, &frontend);
    let switchboard_control::Launch::Argv(argv) = launch else {
        panic!("{launch:?}")
    };
    assert_eq!(
        argv[1..],
        [
            "-lc",
            "exec \"$@\"",
            "dispatch-service",
            "env",
            "BROWSER=none",
            "PORT=3100",
            "npm",
            "start"
        ]
    );
    for _ in 0..3 {
        env.step();
    }
    assert!(
        env.sb().sessions_named("tester").is_empty(),
        "no tester until it answers"
    );
    let t = env.ticket(&id);
    assert_eq!(t.services[0].state, ServiceState::Starting);
    assert!(
        t.processes
            .contains(t.services[0].session.as_ref().unwrap())
    );
    env.repo.lock().unwrap().answering.insert(3100);
    env.steps_until(&id, "the tester", |t, _| {
        t.attempts_of("try").next().is_some()
    });
    let prompt = last_prompt_of(&env, "tester");
    assert!(
        prompt.contains("the frontend is at http://localhost:3100"),
        "{prompt}"
    );
    let status = dispatch::serve::status(&env.runner).unwrap();
    let view = status.tickets.iter().find(|v| v.id == id).unwrap();
    assert_eq!(view.holds, ["my-dev"]);
    assert_eq!(view.services[0].state, "ready");
    assert_eq!(
        view.services[0].url.as_deref(),
        Some("http://localhost:3100")
    );
}

#[test]
fn a_lane_not_cut_is_not_served_and_reads_so() {
    let (mut env, id) = back_half_env(&["area:backend"]);
    deployed(&mut env, &id);
    env.steps_until(&id, "the tester", |t, _| {
        t.attempts_of("try").next().is_some()
    });
    assert!(service_launches(&env).is_empty());
    assert!(env.ticket(&id).services.is_empty());
    let prompt = last_prompt_of(&env, "tester");
    assert!(
        prompt.contains("the frontend is at not served (no frontend lane)"),
        "{prompt}"
    );
}

#[test]
fn a_busy_port_is_skipped_and_no_free_port_is_a_question() {
    let (mut env, id) = back_half_env(BOTH);
    env.repo.lock().unwrap().busy_ports.insert(3100);
    deployed(&mut env, &id);
    served(&mut env, &id);
    let t = env.ticket(&id);
    assert_eq!(t.services[0].port, Some(3101));
    assert!(last_prompt_of(&env, "tester").contains("http://localhost:3101"));

    let (mut env, id) = back_half_env(BOTH);
    env.repo.lock().unwrap().busy_ports.extend([3100, 3101]);
    deployed(&mut env, &id);
    exits(&env, &before_key(&id, 1), 0);
    env.steps_until(&id, "the service question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "service")
    });
    let d = pending_named(&env, &id, "service").unwrap();
    assert_eq!(d.options, ["retry", "park"]);
    assert!(
        d.question.contains("no free port in 3100-3101"),
        "{}",
        d.question
    );
    assert!(service_launches(&env).is_empty());
    assert!(env.sb().sessions_named("tester").is_empty());
}

#[test]
fn a_before_that_fails_is_a_question_before_the_tester_and_retry_runs_it_again() {
    let (mut env, id) = back_half_env(BOTH);
    deployed(&mut env, &id);
    for _ in 0..2 {
        env.step();
    }
    exits(&env, &before_key(&id, 1), 1);
    env.steps_until(&id, "the service question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "service")
    });
    let d = pending_named(&env, &id, "service").unwrap();
    assert!(d.question.contains("before exited 1"), "{}", d.question);
    assert!(env.sb().sessions_named("tester").is_empty());
    answer_named(&mut env, &id, "service", "retry");
    for _ in 0..2 {
        env.step();
    }
    assert_eq!(started(&env, &before_key(&id, 2)), 1);
    let t = env.ticket(&id);
    assert_eq!(t.services.len(), 2);
    assert_eq!(t.services[0].state, ServiceState::Stopped);
    assert_eq!(t.services[1].n, 2);
    assert_eq!(t.services[1].state, ServiceState::Before, "{t:#?}");
    assert!(t.pending_decisions().is_empty());
}

#[test]
fn parking_and_closing_wait_for_a_running_before() {
    let (mut env, id) = back_half_env(BOTH);
    deployed(&mut env, &id);
    for _ in 0..2 {
        env.step();
    }
    assert_eq!(started(&env, &before_key(&id, 1)), 1);
    break_copy(&env, &id);
    for _ in 0..3 {
        env.step();
    }
    let t = env.ticket(&id);
    assert!(is_parking(&t), "{t:#?}");
    assert_eq!(holds(&t), ["my-dev"]);
    assert!(env.repo.lock().unwrap().killed_checks.is_empty());
    exits(&env, &before_key(&id, 1), 0);
    env.step();
    let t = env.ticket(&id);
    assert!(is_parked(&t), "{t:#?}");
    assert!(t.holds.is_empty());
    assert_eq!(t.services[0].before.as_ref().unwrap().exit, Some(0));
    assert!(service_launches(&env).is_empty(), "nothing launched after");

    let (mut env, id) = back_half_env(BOTH);
    deployed(&mut env, &id);
    for _ in 0..2 {
        env.step();
    }
    let now = env.tick();
    env.runner.close_by_hand(&id, None, now).unwrap();
    for _ in 0..3 {
        env.step();
    }
    assert!(matches!(env.ticket(&id).state, TicketState::Closing { .. }));
    exits(&env, &before_key(&id, 1), 0);
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Closed { .. }), "{t:#?}");
    assert!(t.holds.is_empty());
}

fn stuck_pending(env: &Env, id: &str) -> Vec<Decision> {
    env.pending(id)
        .into_iter()
        .filter(|d| d.name == "stuck")
        .collect()
}

#[test]
fn a_park_held_by_a_busy_port_asks_stuck_and_released_parks_it() {
    use dispatch::services::STOP_LIMIT_MS;
    let (mut env, id) = back_half_env(BOTH);
    deployed(&mut env, &id);
    served(&mut env, &id);
    let session = env.ticket(&id).services[0].session.clone().unwrap();
    env.repo.lock().unwrap().busy_ports.insert(3100);
    break_copy(&env, &id);
    env.step();
    env.wait(STOP_LIMIT_MS - 10_000);
    env.step();
    let t = env.ticket(&id);
    assert!(is_parking(&t));
    assert!(stuck_pending(&env, &id).is_empty(), "not before the limit");
    env.wait(10_000);
    env.step();
    let stuck = stuck_pending(&env, &id);
    assert_eq!(stuck.len(), 1);
    assert_eq!(stuck[0].options, ["wait", "released"]);
    assert!(
        stuck[0].question.contains("port 3100"),
        "{}",
        stuck[0].question
    );
    assert!(stuck[0].question.contains("lsof -i :3100"));
    let status = dispatch::serve::status(&env.runner).unwrap();
    let view = status.tickets.iter().find(|v| v.id == id).unwrap();
    let d = view.decisions.iter().find(|d| d.id == stuck[0].id).unwrap();
    assert_eq!(d.state, "pending");
    env.step();
    env.step();
    assert_eq!(stuck_pending(&env, &id).len(), 1, "parking keeps it");
    // `wait`: answered between passes, acted on by the next one.
    answer_named(&mut env, &id, "stuck", "wait");
    env.step();
    let first = stuck[0].id.clone();
    assert!(
        matches!(
            decision_state(&env.ticket(&id), &first),
            dispatch::ticket::DecisionState::Answered { acted: true, .. }
        ),
        "{:?}",
        decision_state(&env.ticket(&id), &first)
    );
    env.wait(STOP_LIMIT_MS / 2);
    env.step();
    assert!(
        stuck_pending(&env, &id).is_empty(),
        "the clock started again"
    );
    env.wait(STOP_LIMIT_MS / 2);
    env.step();
    let again = stuck_pending(&env, &id);
    assert_eq!(again.len(), 1);
    assert_ne!(again[0].id, first);
    assert!(is_parking(&env.ticket(&id)));
    answer_named(&mut env, &id, "stuck", "released");
    env.step();
    let t = env.ticket(&id);
    assert!(
        matches!(
            decision_state(&t, &again[0].id),
            dispatch::ticket::DecisionState::Answered { acted: true, .. }
        ),
        "{t:#?}"
    );
    assert!(is_parked(&t), "{t:#?}");
    assert!(t.holds.is_empty());
    let released = t.services[0].released.clone().unwrap();
    assert!(
        released.starts_with("port 3100 is still taken"),
        "{released}"
    );
    assert!(env.sb().removed_by_port.contains(&session));
}

#[test]
fn a_park_withdraws_a_stuck_question_asked_while_active_and_asks_afresh() {
    use dispatch::services::STOP_LIMIT_MS;
    let (mut env, id) = back_half_env(BOTH);
    at_tried(&mut env, &id);
    env.repo.lock().unwrap().busy_ports.insert(3100);
    tried(&mut env, &id, "done");
    at_stage(&mut env, &id, "after");
    env.step();
    env.wait(STOP_LIMIT_MS);
    env.step();
    let stuck = stuck_pending(&env, &id);
    assert_eq!(stuck.len(), 1, "{:#?}", env.ticket(&id));
    assert!(env.ticket(&id).active());
    break_copy(&env, &id);
    env.step();
    let t = env.ticket(&id);
    assert!(is_parking(&t), "{t:#?}");
    assert_eq!(
        decision_state(&t, &stuck[0].id),
        dispatch::ticket::DecisionState::Cancelled,
        "the intent to park withdraws it"
    );
    env.wait(STOP_LIMIT_MS);
    env.step();
    let again = stuck_pending(&env, &id);
    assert_eq!(again.len(), 1);
    assert_ne!(again[0].id, stuck[0].id);
    assert!(is_parking(&env.ticket(&id)));
}

#[test]
fn a_close_held_by_a_hung_before_asks_stuck_and_can_be_answered_while_closing() {
    use dispatch::services::STOP_LIMIT_MS;
    let (mut env, id) = back_half_env(BOTH);
    deployed(&mut env, &id);
    for _ in 0..2 {
        env.step();
    }
    let now = env.tick();
    env.runner.close_by_hand(&id, None, now).unwrap();
    env.step();
    env.wait(STOP_LIMIT_MS);
    env.step();
    let stuck = stuck_pending(&env, &id);
    assert_eq!(stuck.len(), 1);
    assert!(stuck[0].question.contains(&before_key(&id, 1)));
    assert!(matches!(env.ticket(&id).state, TicketState::Closing { .. }));
    answer_named(&mut env, &id, "stuck", "released");
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(t.state, TicketState::Closed { .. }), "{t:#?}");
    assert!(
        env.repo
            .lock()
            .unwrap()
            .killed_checks
            .contains(&before_key(&id, 1))
    );
    assert!(t.holds.is_empty());
}

#[test]
fn a_leaving_stop_past_the_limit_keeps_the_hold_until_answered() {
    use dispatch::services::STOP_LIMIT_MS;
    let (mut env, a) = back_half_env(BOTH);
    let b = take_orchard(&mut env, 43, BOTH);
    at_tried(&mut env, &a);
    env.repo.lock().unwrap().busy_ports.insert(3100);
    tried(&mut env, &a, "done");
    at_stage(&mut env, &a, "after");
    env.step();
    env.wait(STOP_LIMIT_MS);
    env.step();
    assert_eq!(stuck_pending(&env, &a).len(), 1);
    assert_eq!(holds(&env.ticket(&a)), ["my-dev"]);
    assert!(env.ticket(&b).holds.is_empty());
    assert!(
        env.sb().sessions_named("implementer").is_empty(),
        "nothing of `after` starts while the stop waits"
    );
    answer_named(&mut env, &a, "stuck", "released");
    env.steps_until(&b, "the second ticket's hold", |t, _| !t.holds.is_empty());
    assert!(env.ticket(&a).holds.is_empty());
}

#[test]
fn a_stop_that_ends_on_its_own_after_stuck_withdraws_the_question() {
    use dispatch::services::STOP_LIMIT_MS;
    // Leaving the range while active.
    let (mut env, id) = back_half_env(BOTH);
    at_tried(&mut env, &id);
    env.repo.lock().unwrap().busy_ports.insert(3100);
    tried(&mut env, &id, "done");
    at_stage(&mut env, &id, "after");
    env.step();
    env.wait(STOP_LIMIT_MS);
    env.step();
    let stuck = stuck_pending(&env, &id);
    assert_eq!(stuck.len(), 1);
    env.repo.lock().unwrap().busy_ports.clear();
    env.step();
    let t = env.ticket(&id);
    assert_eq!(
        decision_state(&t, &stuck[0].id),
        dispatch::ticket::DecisionState::Cancelled
    );
    assert!(t.pending_decisions().is_empty(), "{t:#?}");
    assert!(t.holds.is_empty());
    assert_eq!(t.services[0].state, ServiceState::Stopped);
    assert_eq!(t.services[0].released, None, "the runner confirmed it");

    // Parking.
    let (mut env, id) = back_half_env(BOTH);
    deployed(&mut env, &id);
    served(&mut env, &id);
    env.repo.lock().unwrap().busy_ports.insert(3100);
    break_copy(&env, &id);
    env.step();
    env.wait(STOP_LIMIT_MS);
    env.step();
    assert_eq!(stuck_pending(&env, &id).len(), 1);
    env.repo.lock().unwrap().busy_ports.clear();
    env.step();
    let t = env.ticket(&id);
    assert!(is_parked(&t), "{t:#?}");
    assert!(
        t.pending_decisions().is_empty(),
        "parked with nothing to act on"
    );
}

/// The back half with the backend served beside the frontend at `try`.
fn two_served_env() -> (Env, String) {
    let mut env = Env::new();
    let text = BACK_HALF
        .replace("{worktrees}", &env.worktrees.display().to_string())
        .replace(
            "setup = [\"uv\", \"sync\"]",
            "setup = [\"uv\", \"sync\"]\nserve = { argv = [\"uv\", \"run\", \"serve\"], env = { PORT = \"{port}\" }, url = \"http://localhost:{port}\", ready = { http = \"/\", within_secs = 120 } }",
        )
        .replace(
            "services = [\"frontend\"]",
            "services = [\"frontend\", \"backend\"]",
        );
    std::fs::write(env.data.pipeline("Orchard"), text).unwrap();
    let id = take_orchard(&mut env, 42, BOTH);
    (env, id)
}

fn services_of<'a>(t: &'a Ticket, lane: &str) -> Vec<&'a dispatch::ticket::ServiceRecord> {
    t.services.iter().filter(|s| s.lane == lane).collect()
}

#[test]
fn a_retry_leaves_a_failed_service_whose_stop_is_not_confirmed_to_its_stop() {
    use dispatch::services::STOP_LIMIT_MS;
    let (mut env, id) = two_served_env();
    deployed(&mut env, &id);
    for _ in 0..2 {
        env.step();
    }
    let backend = services_of(&env.ticket(&id), "backend")[0].clone();
    assert!(backend.session.is_some(), "{backend:#?}");
    let port = backend.port.unwrap();
    exits(&env, &before_key(&id, 1), 1);
    env.steps_until(&id, "the service question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "service")
    });
    let d = pending_named(&env, &id, "service").unwrap();
    assert!(!d.question.contains("backend"), "{}", d.question);
    // The backend's probe never answers, and once killed its port stays
    // taken.
    env.repo.lock().unwrap().busy_ports.insert(port);
    env.wait(121_000);
    env.step();
    let t = env.ticket(&id);
    assert!(matches!(
        services_of(&t, "backend")[0].state,
        ServiceState::Failed { .. }
    ));
    answer_named(&mut env, &id, "service", "retry");
    env.step();
    let t = env.ticket(&id);
    let backends = services_of(&t, "backend");
    assert_eq!(backends.len(), 1, "no second beside it: {t:#?}");
    assert!(matches!(backends[0].state, ServiceState::Failed { .. }));
    assert!(t.processes.contains(backends[0].session.as_ref().unwrap()));
    let frontends = services_of(&t, "frontend");
    assert_eq!(frontends[0].state, ServiceState::Stopped);
    assert_eq!(frontends.len(), 2, "the confirmed one starts again");
    assert!(env.sb().removed_by_port.is_empty());
    env.wait(STOP_LIMIT_MS / 2);
    env.repo.lock().unwrap().busy_ports.clear();
    env.steps_until(&id, "the backend asked about", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == "service" && d.question.contains("backend"))
    });
    let t = env.ticket(&id);
    assert_eq!(services_of(&t, "backend").len(), 1);
    assert_eq!(env.sb().removed_by_port, [backend.session.unwrap()]);
    answer_named(&mut env, &id, "service", "retry");
    env.step();
    env.step();
    let t = env.ticket(&id);
    let backends = services_of(&t, "backend");
    assert_eq!(backends[0].state, ServiceState::Stopped);
    assert_eq!(backends.len(), 2, "{t:#?}");
}

#[test]
fn a_service_saved_but_never_sent_is_sent_once() {
    let (mut env, id) = back_half_env(BOTH);
    deployed(&mut env, &id);
    for _ in 0..2 {
        env.step();
    }
    exits(&env, &before_key(&id, 1), 0);
    env.step();
    assert_eq!(service_launches(&env).len(), 1);
    // As if the runner died between saving `Starting` and writing the
    // request: no ledger entry, no session on the record.
    let mut t = env.ticket(&id);
    let intent = t.services[0].intent();
    let made = t.services[0].session.take().unwrap();
    t.services[0].op = None;
    t.ledger.retain(|o| o.intent != intent);
    t.processes.retain(|s| s != &made);
    env.runner.save_ticket(&mut t, env.now).unwrap();
    for _ in 0..4 {
        env.step();
    }
    assert_eq!(service_launches(&env).len(), 2, "sent once more, only once");
    let t = env.ticket(&id);
    assert!(t.services[0].session.is_some());
    assert_ne!(t.services[0].session.as_ref(), Some(&made));
}

#[test]
fn a_probe_that_never_answers_is_a_question_and_the_service_is_stopped() {
    let (mut env, id) = back_half_env(BOTH);
    deployed(&mut env, &id);
    for _ in 0..2 {
        env.step();
    }
    exits(&env, &before_key(&id, 1), 0);
    env.step();
    let session = env.ticket(&id).services[0].session.clone().unwrap();
    env.wait(121_000);
    env.steps_until(&id, "the service question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "service")
    });
    let d = pending_named(&env, &id, "service").unwrap();
    assert!(
        d.question
            .contains("http://localhost:3100 did not answer within 120s"),
        "{}",
        d.question
    );
    assert!(env.sb().killed.contains(&session));
    assert!(env.sb().removed_by_port.contains(&session));
    assert!(env.sb().sessions_named("tester").is_empty());
    answer_named(&mut env, &id, "service", "retry");
    for _ in 0..2 {
        env.step();
    }
    assert_eq!(started(&env, &before_key(&id, 2)), 1);
}

#[test]
fn services_stop_when_tried_ends_and_on_park() {
    let (mut env, id) = back_half_env(BOTH);
    at_tried(&mut env, &id);
    let session = env.ticket(&id).services[0].session.clone().unwrap();
    assert_eq!(holds(&env.ticket(&id)), ["my-dev"], "held through tried");
    tried(&mut env, &id, "done");
    env.steps_until(&id, "released", |t, _| t.holds.is_empty());
    let t = env.ticket(&id);
    assert_eq!(t.services[0].state, ServiceState::Stopped);
    assert!(env.sb().killed.contains(&session));
    assert_eq!(env.sb().removed_by_port, [session]);
    env.steps_until(&id, "the agent after", |t, _| {
        t.attempts_of("after").next().is_some()
    });

    let (mut env, id) = back_half_env(BOTH);
    at_tried(&mut env, &id);
    let session = env.ticket(&id).services[0].session.clone().unwrap();
    tried(&mut env, &id, "park");
    env.step();
    let t = env.ticket(&id);
    assert!(is_parked(&t), "{t:#?}");
    assert_eq!(t.services[0].state, ServiceState::Stopped);
    assert_eq!(env.sb().removed_by_port, [session]);
    assert!(t.holds.is_empty());
}

/// `tried` answered `rerun` with a note.
fn tried_rerun(env: &mut Env, id: &str, note: &str) {
    env.steps_until(id, "the tried question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "tried")
    });
    let d = pending_named(env, id, "tried").unwrap();
    let now = env.tick();
    env.runner
        .decide(id, &d.id, "rerun", Some(note), now)
        .unwrap();
}

/// After a `tried` rerun: the frontend's record `n` made, its `before`
/// run, and its service launched but not yet answering.
fn restarting(env: &mut Env, id: &str, n: u32) {
    env.repo.lock().unwrap().answering.clear();
    env.steps_until(id, "the record made again", |t, _| {
        t.services.iter().any(|s| s.n == n)
    });
    for _ in 0..6 {
        if started(env, &before_key(id, n)) > 0 {
            break;
        }
        env.step();
    }
    exits(env, &before_key(id, n), 0);
    env.steps_until(id, "the service launched again", |t, _| {
        t.services.iter().any(|s| s.n == n && s.session.is_some())
    });
}

/// The service record `n` answers on its port and the second tester
/// launches.
fn answering_again(env: &mut Env, id: &str, n: u32) {
    let port = env
        .ticket(id)
        .services
        .iter()
        .find(|s| s.n == n)
        .and_then(|s| s.port)
        .unwrap();
    env.repo.lock().unwrap().answering.insert(port);
    env.steps_until(id, "the second tester", |t, _| {
        t.attempts_of("try").count() == 2
    });
}

#[test]
fn a_tried_rerun_runs_the_tester_again_with_services_restarted_and_keeps_the_hold() {
    let (mut env, id) = back_half_env(BOTH);
    deployed(&mut env, &id);
    served(&mut env, &id);
    let t = env.ticket(&id);
    let tester = session_of(&t, "try");
    let notes = artifact_of(&t, "try", "notes");
    env.finish(
        &tester,
        &notes,
        "\nResult: nothing could be tested\n\nThe page 500s.",
    );
    at_stage(&mut env, &id, "tried");
    env.steps_until(&id, "the tried question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "tried")
    });
    let asked = pending_named(&env, &id, "tried").unwrap();
    assert_eq!(asked.options, ["done", "rerun", "park"]);
    assert!(
        asked.question.contains(&format!(
            "Notes (try): {}\n  Result: nothing could be tested",
            notes.display()
        )),
        "{}",
        asked.question
    );
    let first = env.ticket(&id).services[0].clone();
    let session = first.session.clone().unwrap();
    tried_rerun(&mut env, &id, "the page 500s; the env link was stale");
    env.step();
    let t = env.ticket(&id);
    assert_eq!(stage_name(&t), "try", "{:#?}", t.decisions);
    assert_eq!(holds(&t), ["my-dev"]);
    restarting(&mut env, &id, 2);
    let t = env.ticket(&id);
    assert_eq!(holds(&t), ["my-dev"], "held through the restart");
    assert!(env.sb().killed.contains(&session));
    assert!(env.sb().removed_by_port.contains(&session));
    assert_eq!(t.services[0].state, ServiceState::Stopped);
    assert_eq!(started(&env, &before_key(&id, 2)), 1, "before ran again");
    assert_eq!(service_launches(&env).len(), 2);
    assert_eq!(
        launches_of(&env, "tester"),
        1,
        "not before the service answers"
    );
    answering_again(&mut env, &id, 2);
    let t = env.ticket(&id);
    assert_eq!(holds(&t), ["my-dev"]);
    assert!(
        t.services
            .iter()
            .any(|s| s.n == 2 && s.state == ServiceState::Ready)
    );
    assert_eq!(launches_of(&env, "tester"), 2);
    let prompt = last_prompt_of(&env, "tester");
    assert!(
        prompt.contains("the page 500s; the env link was stale"),
        "{prompt}"
    );
    assert!(
        prompt.contains(&format!(
            "(the previous attempt's notes are at {})",
            notes.display()
        )),
        "{prompt}"
    );
    assert!(notes.exists(), "attempt 1's notes stay");
    // Nothing is deployed again.
    let deploys: Vec<&Attempt> = t.attempts_of("deploy").collect();
    assert_eq!(deploys.len(), 1);
    assert_eq!(deploys[0].state, AttemptState::Complete);
    assert!(deploys[0].head.is_some());
    assert_eq!(started(&env, &deploy_key(&id, 2)), 0);
    tester_done(&mut env, &id);
    env.steps_until(&id, "a second tried question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "tried")
    });
    assert_ne!(pending_named(&env, &id, "tried").unwrap().id, asked.id);
    let events = dispatch::events::read_since(&dispatch::events::log_path(&env.data), 0).unwrap();
    assert!(
        events.iter().any(|e| e.ticket == id
            && e.kind == dispatch::events::Kind::AttemptStarted
            && e.stage == "try"
            && e.text.contains("#2")),
        "{events:#?}"
    );
}

#[test]
fn a_tried_rerun_cut_off_by_a_runner_restart_launches_once() {
    let (mut env, id) = back_half_env(BOTH);
    at_tried(&mut env, &id);
    tried_rerun(&mut env, &id, "again");
    env.step();
    let t = env.ticket(&id);
    assert!(
        t.services[0].stopping_ms.is_some(),
        "marked for the restart: {t:#?}"
    );
    env.restart();
    restarting(&mut env, &id, 2);
    answering_again(&mut env, &id, 2);
    for _ in 0..3 {
        env.step();
    }
    assert_eq!(service_launches(&env).len(), 2, "one launch per record");
    assert_eq!(started(&env, &before_key(&id, 2)), 1);
    assert_eq!(launches_of(&env, "tester"), 2);
    assert_eq!(holds(&env.ticket(&id)), ["my-dev"]);
}

#[test]
fn a_tried_rerun_restart_past_the_stop_limit_asks_stuck() {
    use dispatch::services::STOP_LIMIT_MS;
    let (mut env, id) = back_half_env(BOTH);
    at_tried(&mut env, &id);
    env.repo.lock().unwrap().busy_ports.insert(3100);
    tried_rerun(&mut env, &id, "again");
    env.step();
    env.step();
    assert!(stuck_pending(&env, &id).is_empty(), "not before the limit");
    env.wait(STOP_LIMIT_MS);
    env.step();
    let stuck = stuck_pending(&env, &id);
    assert_eq!(stuck.len(), 1, "{:#?}", env.ticket(&id));
    assert!(
        stuck[0].question.contains("port 3100"),
        "{}",
        stuck[0].question
    );
    for _ in 0..3 {
        env.step();
    }
    let t = env.ticket(&id);
    assert_eq!(launches_of(&env, "tester"), 1, "no tester against it");
    assert_eq!(t.services.len(), 1, "no second record beside it");
    assert_eq!(holds(&t), ["my-dev"]);
}

#[test]
fn a_lost_service_reply_is_found_again_not_started_twice() {
    let (mut env, id) = back_half_env(BOTH);
    deployed(&mut env, &id);
    for _ in 0..2 {
        env.step();
    }
    exits(&env, &before_key(&id, 1), 0);
    env.sb().drop_reply_for = Some("session.new".into());
    env.step();
    let t = env.ticket(&id);
    assert!(t.services[0].session.is_none(), "the reply was lost");
    env.restart();
    for _ in 0..3 {
        env.step();
    }
    assert_eq!(service_launches(&env).len(), 1, "never started twice");
    let t = env.ticket(&id);
    let made = env.sb().sessions_named("frontend :3100")[0].id.clone();
    assert_eq!(t.services[0].session.as_ref(), Some(&made));
    assert!(t.processes.contains(&made));
    assert!(t.services[0].op.is_some());
}

/// A ticket waiting on a decision is not written again while it waits:
/// its record, `updated_ms` included, stays as it was.
#[test]
fn a_ticket_waiting_on_a_decision_keeps_its_record_and_updated_time() {
    let mut env = Env::new();
    let id = at_finalize(&mut env);
    let waiting = env.ticket(&id);
    for _ in 0..5 {
        env.step();
    }
    assert_eq!(env.ticket(&id), waiting);
}

/// A pull request read again moves its `checked_ms`, so the poll
/// interval still holds, and not the ticket's `updated_ms`, which moves
/// only once the checks change.
#[test]
fn a_pr_poll_moves_its_checked_time_and_not_the_tickets() {
    let mut env = Env::new();
    let id = at_ready(&mut env);
    env.pr_is(&id, "base0000", "open", Checks::Pending);
    env.recheck(&id);
    env.step();
    env.step();
    let checked = |t: &Ticket| {
        t.attempts_of("ready")
            .last()
            .and_then(|a| a.pr.as_ref())
            .unwrap()
            .checked_ms
    };
    let before = env.ticket(&id);
    let looked = env.prs.lock().unwrap().looked;
    env.wait(PR_POLL_MS);
    env.step();
    env.step();
    let polled = env.ticket(&id);
    assert_eq!(env.prs.lock().unwrap().looked, looked + 1, "once a minute");
    assert!(checked(&polled) > checked(&before));
    assert_eq!(polled.updated_ms, before.updated_ms);
    env.pr_is(&id, "base0000", "open", Checks::Passed);
    env.wait(PR_POLL_MS);
    env.step();
    let passed = env.ticket(&id);
    assert!(passed.updated_ms > before.updated_ms);
    assert_eq!(passed.updated_ms, env.now);
}

/// An agent's artifact settling is poll bookkeeping: `updated_ms` stays
/// put through the settle passes and moves when the attempt moves on to
/// its checks.
#[test]
fn an_artifact_settling_moves_the_updated_time_only_when_the_attempt_does() {
    let mut env = Env::new();
    let (id, implementer) = at_implement(&mut env);
    let t = env.ticket(&id);
    std::fs::write(artifact_of(&t, "implement", "notes"), "# done\nchanged").unwrap();
    let now = env.now;
    env.sb().stop(&implementer, now);
    let mut prev = env.ticket(&id);
    let (mut quiet, mut moved) = (0, 0);
    for _ in 0..=SETTLE_POLLS {
        env.step();
        let cur = env.ticket(&id);
        if dispatch::store::same_but_polls(&prev, &cur) {
            assert_eq!(cur.updated_ms, prev.updated_ms);
            if cur != prev {
                quiet += 1;
            }
        } else {
            assert_eq!(cur.updated_ms, env.now);
            moved += 1;
        }
        prev = cur;
    }
    assert!(quiet > 0, "a settle pass wrote poll bookkeeping alone");
    let a = prev.attempts_of("implement").last().unwrap();
    assert!(a.gate.is_some(), "the checks started: {a:#?}");
    assert!(moved > 0, "the stop and the checks starting moved it");
}

// --- secret artifacts: a gate-only stage that writes them, kept only
// through the hold

/// The back half with a gate-only `try-setup` after `deploy` that writes
/// a secret `personas` file for the tester.
const SECRET_STAGES: [&str; 6] = ["lanes", "deploy", "try-setup", "try", "tried", "after"];

/// What the fake command "writes": the test stands in for it.
const TOKEN: &str = "tok-SEKRET-0451";

fn secret_env() -> (Env, String) {
    let mut env = Env::new();
    let text = BACK_HALF
        .replace("{worktrees}", &env.worktrees.display().to_string())
        .replace(
            "[[stages]]\nname = \"try\"\n",
            "[[stages]]\nname = \"try-setup\"\ncontext = \"lane:backend\"\nneeds = [\"my-dev\"]\nwrites = [{ name = \"personas\", secret = true }]\ngate = { kind = \"command\", in = \"lane:backend\", argv = [\"sh\", \"-c\", \"inv personas\"] }\n\n[[stages]]\nname = \"try\"\n",
        )
        .replace(
            "Report to {notes}.",
            "Personas at {inputs.personas} and {inputs.try-setup.personas}. Report to {notes}.",
        );
    std::fs::write(env.data.pipeline("Orchard"), text).unwrap();
    let id = take_orchard(&mut env, 42, BOTH);
    (env, id)
}

fn secret_stage(t: &Ticket) -> &'static str {
    SECRET_STAGES.get(t.stage).copied().unwrap_or("done")
}

fn setup_key(id: &str, n: u32) -> String {
    format!("{id}/try-setup/{n}")
}

/// The deploy done and try-setup's command started: the path it was
/// told to write `personas` to.
fn setup_started(env: &mut Env, id: &str) -> PathBuf {
    deploying(env, id);
    exits(env, &deploy_key(id, 1), 0);
    let key = setup_key(id, 1);
    for _ in 0..8 {
        if started(env, &key) > 0 {
            break;
        }
        env.step();
    }
    let repo = env.repo.lock().unwrap();
    let check = repo
        .checks
        .iter()
        .find(|c| c.key == key)
        .unwrap_or_else(|| panic!("try-setup never started"));
    let path = check
        .env
        .iter()
        .find(|(k, _)| k == "DISPATCH_WRITES_PERSONAS")
        .map_or_else(
            || panic!("no DISPATCH_WRITES_PERSONAS: {:?}", check.env),
            |(_, v)| PathBuf::from(v),
        );
    assert!(
        !check.env.iter().any(|(k, _)| k == "DISPATCH_WRITES_CHECKS"),
        "{:?}",
        check.env
    );
    path
}

/// try-setup writes the file and exits 0; the ticket stands at `try`.
fn set_up(env: &mut Env, id: &str) -> PathBuf {
    let path = setup_started(env, id);
    std::fs::write(&path, TOKEN).unwrap();
    exits(env, &setup_key(id, 1), 0);
    env.steps_until(id, "try", |t, _| secret_stage(t) == "try");
    path
}

/// Through `try`'s tester to the `tried` question.
fn secret_at_tried(env: &mut Env, id: &str) -> PathBuf {
    let path = set_up(env, id);
    served(env, id);
    let t = env.ticket(id);
    let tester = session_of(&t, "try");
    env.finish(&tester, &artifact_of(&t, "try", "notes"), "it works");
    env.steps_until(id, "tried", |t, _| secret_stage(t) == "tried");
    path
}

/// The `session.new` requests for sessions named `name`.
fn session_news(
    env: &Env,
    name: &str,
) -> Vec<(String, std::collections::BTreeMap<String, String>)> {
    env.sb()
        .calls
        .iter()
        .filter_map(|r| match &r.body {
            Body::SessionNew {
                name: n,
                prompt,
                env,
                ..
            } if n == name => Some((prompt.clone().unwrap_or_default(), env.clone())),
            _ => None,
        })
        .collect()
}

/// The `forgotten` events of the ticket, as their text.
fn forgotten_events(env: &Env, id: &str) -> Vec<String> {
    dispatch::events::read_since(&dispatch::events::log_path(&env.data), 0)
        .unwrap()
        .into_iter()
        .filter(|e| e.ticket == id && e.kind == dispatch::events::Kind::Forgotten)
        .map(|e| e.text)
        .collect()
}

fn setup_attempt(t: &Ticket) -> Attempt {
    t.attempts_of("try-setup").last().unwrap().clone()
}

#[test]
fn a_gate_only_stage_writes_an_artifact_the_tester_is_given_by_path_and_environment() {
    let (mut env, id) = secret_env();
    let path = set_up(&mut env, &id);
    let a = setup_attempt(&env.ticket(&id));
    assert_eq!(a.state, AttemptState::Complete);
    assert_eq!(a.artifacts.get("personas"), Some(&path));
    assert_eq!(a.secret.iter().collect::<Vec<_>>(), ["personas"]);
    served(&mut env, &id);
    let testers = session_news(&env, "tester");
    assert_eq!(testers.len(), 1);
    let (prompt, vars) = &testers[0];
    let shown = path.display().to_string();
    assert!(
        prompt.contains(&format!("Personas at {shown} and {shown}.")),
        "{prompt}"
    );
    assert_eq!(vars.get("DISPATCH_INPUT_PERSONAS"), Some(&shown));
    assert_eq!(vars.get("DISPATCH_INPUT_TRY_SETUP_PERSONAS"), Some(&shown));
    assert_eq!(vars.len(), 2, "{vars:?}");
    assert!(!prompt.contains(TOKEN));
}

#[test]
fn a_gate_only_stage_that_exits_0_without_its_artifact_fails_and_asks_rerun() {
    let (mut env, id) = secret_env();
    setup_started(&mut env, &id);
    exits(&env, &setup_key(&id, 1), 0);
    env.steps_until(&id, "the rerun question", |t, _| {
        t.pending_decisions().iter().any(|d| d.name == "rerun")
    });
    let a = setup_attempt(&env.ticket(&id));
    assert!(
        matches!(&a.state, AttemptState::Failed { reason } if reason.contains("without writing personas")),
        "{a:#?}"
    );
    env.step();
    let a = setup_attempt(&env.ticket(&id));
    assert_eq!(a.forgotten["personas"].why, "attempt failed");
}

#[test]
fn a_secret_artifact_is_private_and_never_read_into_a_view_a_report_or_the_log() {
    use std::os::unix::fs::PermissionsExt;
    let (mut env, id) = secret_env();
    let path = set_up(&mut env, &id);
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let dir_mode = std::fs::metadata(path.parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(dir_mode, 0o700);
    let t = env.ticket(&id);
    let pipeline = env.runner.pipeline_of(&t).ok();
    let mut view = dispatch::serve::ticket_view(&t, pipeline.as_ref());
    view.paths = dispatch::serve::ticket_paths(&t, pipeline.as_ref());
    let attempt = view
        .attempts
        .iter()
        .find(|a| a.stage == "try-setup")
        .unwrap();
    assert_eq!(attempt.secret, ["personas"]);
    let shown = serde_json::to_string(&view).unwrap();
    assert!(!shown.contains(TOKEN));
    let report = dispatch::report::of(
        &t,
        pipeline.as_ref(),
        &dispatch::events::stage_names(&t),
        &|p| std::fs::read_to_string(p).ok(),
        &dispatch::report::plan_round_numbers,
        None,
        env.now,
    );
    assert!(!serde_json::to_string(&report).unwrap().contains(TOKEN));
    let log = std::fs::read_to_string(dispatch::events::log_path(&env.data)).unwrap();
    assert!(!log.contains(TOKEN));
    let mut handler = dispatch::serve::Handler {
        runner: Runner::new(
            env.data.clone(),
            Box::new(SharedPort(Arc::clone(&env.sb))),
            Box::new(Arc::clone(&env.repo)),
        ),
        issues: Box::new(dispatch::github::FakeIssues::default()),
    };
    let reply = handler.handle(
        &dispatch_control::Request::new("t", dispatch_control::Body::Ticket { id: id.clone() }),
        env.now,
    );
    assert!(
        matches!(reply, dispatch_control::Reply::Ticket(_)),
        "{reply:?}"
    );
    assert!(!serde_json::to_string(&reply).unwrap().contains(TOKEN));
    let reply = handler.handle(
        &dispatch_control::Request::new(
            "a",
            dispatch_control::Body::Artifact {
                ticket: id.clone(),
                path: path.clone(),
            },
        ),
        env.now,
    );
    let text = serde_json::to_string(&reply).unwrap();
    assert!(text.contains("personas is secret"), "{text}");
    assert!(!text.contains(TOKEN));
}

#[test]
fn a_secret_is_deleted_when_tried_releases_the_hold() {
    let (mut env, id) = secret_env();
    let path = secret_at_tried(&mut env, &id);
    assert!(path.is_file(), "kept through tried");
    tried(&mut env, &id, "done");
    env.steps_until(&id, "the hold released", |t, _| t.holds.is_empty());
    assert!(!path.exists());
    let t = env.ticket(&id);
    let a = setup_attempt(&t);
    assert_eq!(a.forgotten["personas"].why, "my-dev released");
    assert_eq!(a.artifacts.get("personas"), Some(&path), "still listed");
    assert_eq!(
        forgotten_events(&env, &id),
        ["personas deleted: my-dev released"]
    );
    assert_eq!(
        t.input("personas"),
        None,
        "a later prompt gets no dead path"
    );
    let after = session_news(&env, "implementer");
    assert!(!after.is_empty());
    assert!(after.iter().all(|(_, vars)| vars.is_empty()), "{after:?}");
}

#[test]
fn a_secret_is_deleted_when_the_ticket_parks() {
    let (mut env, id) = secret_env();
    let path = set_up(&mut env, &id);
    break_copy(&env, &id);
    env.steps_until(&id, "parked", |t, _| is_parked(t));
    assert!(!path.exists());
    assert_eq!(
        setup_attempt(&env.ticket(&id)).forgotten["personas"].why,
        "parked"
    );
    assert_eq!(forgotten_events(&env, &id), ["personas deleted: parked"]);
}

#[test]
fn a_restart_inside_the_secrets_hold_deletes_it_and_writes_it_again() {
    let (mut env, id) = secret_env();
    let path = secret_at_tried(&mut env, &id);
    restart_at(&mut env, &id, Some("try"));
    env.steps_until(&id, "the restart applied", |t, _| {
        !is_parking(t) && t.restart.is_none()
    });
    assert!(!path.exists());
    let a = setup_attempt(&env.ticket(&id));
    assert_eq!(a.forgotten["personas"].why, "parked");
    assert_eq!(forgotten_events(&env, &id), ["personas deleted: parked"]);
    // The hold the secret was made under was let go, so retaking it at
    // `try` sends the ticket back to write the secret again.
    env.steps_until(&id, "the deploy question", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == "rerun" && d.stage == "deploy")
    });
    let d = rerun_about(&env, &id, "deploy", 1);
    assert!(d.question.contains("my-dev was let go"), "{}", d.question);
    answer(&mut env, &id, &d, "rerun");
    for _ in 0..8 {
        if started(&env, &deploy_key(&id, 2)) > 0 {
            break;
        }
        env.step();
    }
    assert_eq!(started(&env, &deploy_key(&id, 2)), 1, "the second deploy");
    exits(&env, &deploy_key(&id, 2), 0);
    env.steps_until(&id, "the try-setup question", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == "rerun" && d.stage == "try-setup")
    });
    let d = rerun_about(&env, &id, "try-setup", 1);
    answer(&mut env, &id, &d, "rerun");
    for _ in 0..8 {
        if started(&env, &setup_key(&id, 2)) > 0 {
            break;
        }
        env.step();
    }
    assert_eq!(started(&env, &setup_key(&id, 2)), 1, "try-setup again");
    let again = setup_attempt(&env.ticket(&id)).artifacts["personas"].clone();
    assert_ne!(again, path);
    std::fs::write(&again, TOKEN).unwrap();
    exits(&env, &setup_key(&id, 2), 0);
    env.steps_until(&id, "try", |t, _| secret_stage(t) == "try");
    let before = session_news(&env, "tester").len();
    serving(&mut env, &id, 2);
    env.steps_until(&id, "the restart's question", |t, _| {
        t.pending_decisions()
            .iter()
            .any(|d| d.name == "rerun" && d.stage == "try")
    });
    let d = rerun_about(&env, &id, "try", 1);
    answer(&mut env, &id, &d, "rerun");
    env.steps_until(&id, "the new tester", |t, _| {
        t.attempts_of("try").any(Attempt::is_open)
    });
    let testers = session_news(&env, "tester");
    assert_eq!(testers.len(), before + 1, "a new tester");
    let (prompt, vars) = testers.last().unwrap();
    let shown = again.display().to_string();
    assert!(
        prompt.contains(&format!("Personas at {shown} and {shown}.")),
        "{prompt}"
    );
    assert_eq!(vars.get("DISPATCH_INPUT_PERSONAS"), Some(&shown));
}

#[test]
fn a_secret_is_deleted_when_the_ticket_closes() {
    let (mut env, id) = secret_env();
    let path = secret_at_tried(&mut env, &id);
    let now = env.tick();
    env.runner.close_by_hand(&id, None, now).unwrap();
    env.steps_until(&id, "closed", |t, _| {
        matches!(t.state, TicketState::Closed { .. })
    });
    assert!(!path.exists());
    assert_eq!(
        setup_attempt(&env.ticket(&id)).forgotten["personas"].why,
        "closed"
    );
    assert_eq!(forgotten_events(&env, &id), ["personas deleted: closed"]);
}

#[test]
fn a_secret_is_never_swept_while_its_command_runs() {
    let (mut env, id) = secret_env();
    let path = setup_started(&mut env, &id);
    std::fs::write(&path, TOKEN).unwrap();
    for _ in 0..4 {
        env.step();
    }
    assert!(path.is_file());
    let a = setup_attempt(&env.ticket(&id));
    assert!(a.is_open() && a.forgotten.is_empty(), "{a:#?}");
    exits(&env, &setup_key(&id, 1), 0);
    env.steps_until(&id, "try", |t, _| secret_stage(t) == "try");
    assert_eq!(
        setup_attempt(&env.ticket(&id)).state,
        AttemptState::Complete
    );
    assert!(path.is_file());
}

#[test]
fn a_park_asked_while_the_secrets_command_runs_deletes_it_once_cancelled() {
    let (mut env, id) = secret_env();
    let path = setup_started(&mut env, &id);
    std::fs::write(&path, TOKEN).unwrap();
    break_copy(&env, &id);
    env.step();
    env.step();
    let t = env.ticket(&id);
    assert!(is_parking(&t), "{t:#?}");
    assert!(setup_attempt(&t).forgotten.is_empty());
    assert!(path.is_file());
    exits(&env, &setup_key(&id, 1), 0);
    env.steps_until(&id, "parked", |t, _| is_parked(t));
    let a = setup_attempt(&env.ticket(&id));
    assert!(matches!(a.state, AttemptState::Cancelled { .. }), "{a:#?}");
    assert!(!path.exists());
    assert_eq!(a.forgotten["personas"].why, "attempt cancelled");
    assert_eq!(
        forgotten_events(&env, &id),
        ["personas deleted: attempt cancelled"]
    );
}

// --- a joined code review over per-lane plans: the reviewer and the
// fixer are given every lane's plan.

/// `WORKSPACE`'s lanes, planned and implemented per lane, then one
/// code review of them all.
const JOINED_REVIEW: &str = r#"
version = 1

[project]
name = "Orchard"
repo = "git@example.com:example-org/orchard-workspace.git"
worktrees = "{worktrees}"
space = "Dispatch · Orchard"

[source]
kind = "github"
repo = "example-org/orchard-workspace"
label = "dispatch"
lane_hints = { "area:backend" = "backend", "area:frontend" = "frontend" }

[[lanes]]
name = "backend"
path = "orchard-backend"
repo = "git@example.com:example-org/orchard-backend.git"
base = "main"
setup = ["uv", "sync"]

[[lanes]]
name = "frontend"
path = "orchard-frontend"
repo = "git@example.com:example-org/orchard-frontend.git"
base = "dev"
setup = ["npm", "ci"]

[operators.planner]
kind = "claude"

[operators.implementer]
kind = "claude"

[operators.style]
kind = "claude"

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
name = "implement"
operator = "implementer"
context = "each"
writes = ["notes"]
prompt = "Implement the {lane} part on {branch}; notes to {notes}."
gate = { kind = "command", argv = ["sh", "-c", "check"] }

[[stages]]
name = "review-code"
context = "joined"
reviewers = ["style"]
implementer = "implementer"
gate = { kind = "command", like = "implement" }
review_prompt = "Against {plan}."
fix_prompt = "Per {plan}."

[policy]
slots = 2
waiting_on_me = 3
decisions = { lanes = "auto", review-code = "auto" }
trust_folders = true
"#;

#[test]
fn a_joined_review_over_lane_plans_names_every_plan() {
    let mut env = Env::new();
    let text = JOINED_REVIEW.replace("{worktrees}", &env.worktrees.display().to_string());
    std::fs::write(env.data.pipeline("Orchard"), text).unwrap();
    let id = take_orchard(&mut env, 42, BOTH);
    env.steps_until(&id, "the planners", |t, _| {
        t.attempts_of("plan")
            .filter(|a| a.session.is_some() && a.is_open())
            .count()
            == 2
    });
    let t = env.ticket(&id);
    let planners: Vec<Attempt> = t.attempts_of("plan").cloned().collect();
    for a in planners {
        env.finish(
            a.session.as_deref().unwrap(),
            &a.artifacts["plan"],
            &format!("# Plan\n\n## Decisions\n- Keep {}.\n", a.context),
        );
    }
    implementing(&mut env, &id);
    let t = env.ticket(&id);
    let open: Vec<Attempt> = t
        .attempts_of("implement")
        .filter(|a| a.is_open())
        .cloned()
        .collect();
    for a in open {
        env.finish(
            a.session.as_deref().unwrap(),
            &a.artifacts["notes"],
            "# notes\ndone",
        );
    }
    env.steps_until(&id, "each lane's checks", |t, _| {
        t.attempts_of("implement")
            .filter(|a| a.gate.is_some())
            .count()
            == 2
    });
    let t = env.ticket(&id);
    for a in t.attempts_of("implement") {
        env.repo
            .lock()
            .unwrap()
            .check_exits
            .insert(format!("{id}/implement/{}", a.n), 0);
    }
    env.steps_until(&id, "the joined review round", |t, _| {
        t.attempts_of("review-code").last().is_some_and(|a| {
            a.context == "joined"
                && a.rounds
                    .first()
                    .is_some_and(|r| r.reviewers.iter().all(|x| x.session.is_some()))
        })
    });
    let t = env.ticket(&id);
    let plan = |lane: &str| {
        t.attempts_of("plan")
            .find(|a| a.context == lane)
            .unwrap()
            .artifacts["plan"]
            .display()
            .to_string()
    };
    let listed = format!(
        "{} (backend), {} (frontend)",
        plan("backend"),
        plan("frontend")
    );
    let prompt = last_prompt_of(&env, "style");
    assert!(prompt.contains(&format!("Against {listed}.")), "{prompt}");
    let (backend, frontend) = (
        prompt.find("Keep backend.").expect(&prompt),
        prompt.find("Keep frontend.").expect(&prompt),
    );
    assert!(backend < frontend, "{prompt}");
    style_says(&mut env, &id, 1, "- src/a.rs: unused import\n");
    env.steps_until(&id, "the fixer", |t, _| {
        t.attempts_of("review-code")
            .last()
            .is_some_and(|a| a.rounds[0].implementer.is_some())
    });
    let fix = last_prompt_of(&env, "implementer");
    assert!(fix.contains(&format!("Per {listed}.")), "{fix}");
}

/// The implementer granted `aws-dev` and `npm`, and the implement stage
/// `npm` and `deploy`, with `switchboard-env` at a known path; the
/// path returned.
fn with_env_sets(env: &mut Env) -> PathBuf {
    let path = env.data.pipeline(PROJECT);
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace(
            "[operators.implementer]\nkind = \"claude\"\n",
            "[operators.implementer]\nkind = \"claude\"\nenv = [\"aws-dev\", \"npm\"]\n",
        )
        .replace(
            "prompt = \"Implement {inputs.plan}",
            "env = [\"npm\", \"deploy\"]\nprompt = \"Implement {inputs.plan}",
        );
    std::fs::write(path, text).unwrap();
    let bin = env.worktrees.join("bin").join("switchboard-env");
    env.runner.env_bin = Some(bin.clone());
    bin
}

/// The implement stage's `SessionNew`, as Dispatch sent it.
fn implement_launch(env: &Env) -> Body {
    env.sb()
        .calls
        .iter()
        .find_map(|r| match &r.body {
            b @ Body::SessionNew { name, .. } if name == "implementer" => Some(b.clone()),
            _ => None,
        })
        .expect("the implementer was launched")
}

#[test]
fn an_agent_with_env_gets_its_operators_sets_then_its_stages_and_the_sentence() {
    let mut env = Env::new();
    let bin = with_env_sets(&mut env);
    at_implement(&mut env);
    let Body::SessionNew {
        env_sets,
        prompt,
        launch,
        ..
    } = implement_launch(&env)
    else {
        unreachable!()
    };
    assert_eq!(env_sets, vec!["aws-dev", "npm", "deploy"]);
    let prompt = prompt.unwrap();
    let sentence = format!(
        "Commands that need credentials run through `{} exec -- <command>`; never look for credential files or profiles.",
        bin.display()
    );
    assert!(prompt.ends_with(&sentence), "{prompt}");
    let switchboard_control::Launch::Argv(argv) = launch else {
        panic!("the implementer launches with its write flags: {launch:?}");
    };
    assert!(
        !argv.iter().any(|a| a.contains("Bash(switchboard-env")),
        "no allow rule: {argv:?}"
    );
    // Stages without env launch as before.
    let investigate = env
        .sb()
        .calls
        .iter()
        .find_map(|r| match &r.body {
            Body::SessionNew {
                name,
                env_sets,
                prompt,
                ..
            } if name == "investigator" => Some((env_sets.clone(), prompt.clone().unwrap())),
            _ => None,
        })
        .unwrap();
    assert!(investigate.0.is_empty());
    assert!(
        !investigate.1.contains("switchboard-env"),
        "{}",
        investigate.1
    );
}

#[test]
fn the_env_sentence_quotes_a_path_with_a_space() {
    let bin = PathBuf::from("/Applications/My Apps/Switchboard.app/Contents/MacOS/switchboard-env");
    assert_eq!(
        dispatch::scheduler::env_sentence(&bin),
        "Commands that need credentials run through `'/Applications/My Apps/Switchboard.app/Contents/MacOS/switchboard-env' exec -- <command>`; never look for credential files or profiles."
    );
}

#[test]
fn an_agent_with_env_and_no_switchboard_env_fails_its_attempt_unlaunched() {
    let mut env = Env::new();
    with_env_sets(&mut env);
    env.runner.env_bin = None;
    let id = at_finalize(&mut env);
    let decision = env.pending(&id)[0].id.clone();
    let now = env.tick();
    env.runner
        .decide(&id, &decision, "finalize", None, now)
        .unwrap();
    env.steps_until(&id, "the implement attempt failing", |t, _| {
        t.attempts_of("implement")
            .next()
            .is_some_and(|a| matches!(a.state, AttemptState::Failed { .. }))
    });
    let t = env.ticket(&id);
    let a = t.attempts_of("implement").next().unwrap();
    assert_eq!(
        a.state,
        AttemptState::Failed {
            reason: "switchboard-env not found beside dispatch".into()
        }
    );
    assert!(
        !env.sb().calls.iter().any(|r| matches!(
            &r.body,
            Body::SessionNew { name, .. } if name == "implementer"
        )),
        "nothing launched"
    );
}

#[test]
fn a_clone_carries_its_own_operators_sets() {
    let mut env = Env::new();
    let path = env.data.pipeline(PROJECT);
    let text = std::fs::read_to_string(&path).unwrap().replace(
        "[operators.rebaser]\nkind = \"claude\"\n",
        "[operators.rebaser]\nkind = \"claude\"\nenv = [\"git-push\"]\n",
    );
    std::fs::write(path, text).unwrap();
    env.runner.env_bin = Some(PathBuf::from("/opt/sb/switchboard-env"));
    let id = at_ready(&mut env);
    env.at_merge(&id);
    env.pr_is_with(&id, "base0000", "open", Checks::Passed, Some("conflicting"));
    env.wait(PR_POLL_MS);
    env.steps_until(&id, "the rebaser", |t, _| {
        t.attempts_of("ready")
            .any(|a| a.kind == AttemptKind::Agent && a.session.is_some())
    });
    let (sets, prompt) = env
        .sb()
        .calls
        .iter()
        .find_map(|r| match &r.body {
            Body::SessionClone {
                name,
                env_sets,
                prompt,
                ..
            } if name == "rebaser" => Some((env_sets.clone(), prompt.clone())),
            _ => None,
        })
        .unwrap();
    assert_eq!(sets, vec!["git-push"]);
    assert!(
        prompt.contains("`/opt/sb/switchboard-env exec -- <command>`"),
        "{prompt}"
    );
}

#[test]
fn a_command_gate_with_env_runs_under_switchboard_env_with_the_runners_token() {
    let mut env = Env::new();
    let bin = with_env_sets(&mut env);
    env.runner.credentials = Some(dispatch::scheduler::RunnerCredentials {
        record: "runner-record".into(),
        token: "runner-token".into(),
    });
    let (id, implementer) = at_implement(&mut env);
    implementer_stops(&mut env, &id, &implementer);
    let check = implement_check(&mut env, &id);
    assert_eq!(
        check.outer,
        vec![bin.display().to_string(), "exec".into(), "--".into()]
    );
    assert_eq!(check.argv, vec!["sh", "-c", "cargo test"]);
    assert!(
        check.env.contains(&(
            "SWITCHBOARD_RECORD_TOKEN".to_owned(),
            "runner-token".to_owned()
        )) && check.env.contains(&(
            "SWITCHBOARD_RECORD_ID".to_owned(),
            "runner-record".to_owned()
        )),
        "{:?}",
        check.env
    );
}

#[test]
fn a_command_gate_with_env_on_a_runner_without_a_record_fails_with_one_line() {
    let mut env = Env::new();
    with_env_sets(&mut env);
    let (id, implementer) = at_implement(&mut env);
    implementer_stops(&mut env, &id, &implementer);
    env.steps_until(&id, "the checks failing", |t, _| {
        t.attempts_of("implement")
            .last()
            .is_some_and(|a| matches!(a.state, AttemptState::Failed { .. }))
    });
    let t = env.ticket(&id);
    let a = t.attempts_of("implement").last().unwrap();
    assert_eq!(
        a.state,
        AttemptState::Failed {
            reason: "stage implement needs credentials (env), but this runner has no Switchboard record: start it from the Dispatch overview".into()
        }
    );
    assert!(env.repo.lock().unwrap().checks.is_empty(), "nothing ran");
}

#[test]
fn a_command_gate_without_env_gets_no_wrapper_and_no_token() {
    let mut env = Env::new();
    env.runner.env_bin = Some(PathBuf::from("/opt/sb/switchboard-env"));
    env.runner.credentials = Some(dispatch::scheduler::RunnerCredentials {
        record: "runner-record".into(),
        token: "runner-token".into(),
    });
    let (id, implementer) = at_implement(&mut env);
    implementer_stops(&mut env, &id, &implementer);
    let check = implement_check(&mut env, &id);
    assert!(check.outer.is_empty(), "{:?}", check.outer);
    assert!(
        !check
            .env
            .iter()
            .any(|(k, _)| k == "SWITCHBOARD_RECORD_TOKEN"),
        "{:?}",
        check.env
    );
    let Body::SessionNew {
        env_sets, prompt, ..
    } = implement_launch(&env)
    else {
        unreachable!()
    };
    assert!(env_sets.is_empty());
    assert!(!prompt.unwrap().contains("switchboard-env"));
}
