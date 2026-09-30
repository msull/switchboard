//! A Switchboard in memory that answers the control port's contract the
//! way the app does: replies logged and repeated for the same op, `find`
//! from the log first, records that carry their op. Knobs stand in for
//! the crash windows: a reply dropped after acting, a launch that never
//! reported, a record removed in the window.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use dispatch::git::{FakeRepo, Repo};
use dispatch::port::Port;
use switchboard_control::{
    Body, Definition, Found, Liveness, Made, OpStatus, Pin, ProjectView, RecordKind, Reply,
    Request, RunState, RunView, SessionKind, SessionView, SetView, SpaceView,
};

#[derive(Debug, Clone)]
pub struct LogLine {
    pub op: String,
    pub kind: String,
    pub ids: Vec<String>,
    pub reply: Option<Reply>,
}

#[derive(Debug, Default)]
pub struct FakeSwitchboard {
    pub spaces: Vec<SpaceView>,
    pub projects: Vec<ProjectView>,
    pub sessions: Vec<SessionView>,
    pub runs: Vec<RunView>,
    pub sets: Vec<SetView>,
    pub definitions: Vec<Definition>,
    pub log: Vec<LogLine>,
    /// Removed in the window: gone from the records, still in the log.
    pub removed: Vec<String>,
    /// Records saved with their launch pending: the app died in between.
    pub interrupted: Vec<String>,
    /// Every request seen, in order.
    pub calls: Vec<Request>,
    pub killed: Vec<String>,
    pub notes: BTreeMap<String, String>,
    pub waiting: BTreeMap<String, (bool, String)>,
    /// Sessions whose trust question was answered, in order.
    pub trusted: Vec<String>,
    /// Sessions the app has a transcript for (a review can clone them).
    pub resumable: Vec<String>,
    // --- knobs
    /// The next launch of this kind fails outright (no records).
    pub fail_next: Option<String>,
    /// Act and log the reply, then fail the socket: the reply was lost.
    pub drop_reply_for: Option<String>,
    /// The next session query is answered with this failure instead of
    /// the session (the app too busy to answer, say).
    pub fail_session_query: Option<String>,
    /// Save the records and the request line, then die: no reply line,
    /// no pane, `op.status` interrupted.
    pub die_launching: Option<String>,
    /// The next creation of this kind is reported in progress until the
    /// knob is cleared.
    pub in_progress_for: Option<String>,
    /// Replies held back while `in_progress_for` is set.
    pub pending_reply: Vec<(String, Reply)>,
    counter: u64,
}

impl FakeSwitchboard {
    pub fn new() -> Self {
        Self {
            spaces: vec![SpaceView {
                id: "space-default".into(),
                name: "Default".into(),
                op: None,
            }],
            ..Self::default()
        }
    }

    fn id(&mut self, prefix: &str) -> String {
        self.counter += 1;
        format!("{prefix}-{}", self.counter)
    }

    pub fn session(&self, id: &str) -> &SessionView {
        self.sessions
            .iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("no session {id}"))
    }

    pub fn session_mut(&mut self, id: &str) -> &mut SessionView {
        self.sessions
            .iter_mut()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("no session {id}"))
    }

    pub fn sessions_named(&self, name: &str) -> Vec<&SessionView> {
        self.sessions.iter().filter(|s| s.name == name).collect()
    }

    /// The agent finished its turn.
    pub fn stop(&mut self, id: &str, at_ms: u64) {
        let s = self.session_mut(id);
        s.last_stop_at_ms = Some(at_ms);
        s.card = "idle".into();
    }

    /// The pane is gone, no stop reported.
    pub fn vanish(&mut self, id: &str) {
        let s = self.session_mut(id);
        s.liveness = Liveness::Missing;
        s.card = "not running".into();
    }

    /// Removed in the window.
    pub fn remove(&mut self, id: &str) {
        self.sessions.retain(|s| s.id != id);
        self.removed.push(id.to_owned());
    }

    pub fn run_mut(&mut self, id: &str) -> &mut RunView {
        self.runs
            .iter_mut()
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!("no run {id}"))
    }

    pub fn kinds_called(&self, kind: &str) -> usize {
        self.calls.iter().filter(|r| r.body.kind() == kind).count()
    }

    fn replied(&self, op: &str) -> Option<Reply> {
        self.log
            .iter()
            .find(|l| l.op == op)
            .and_then(|l| l.reply.clone())
    }

    fn new_session(
        &mut self,
        project: &str,
        name: &str,
        kind: SessionKind,
        cwd: &std::path::Path,
        notes: &str,
        op: &str,
    ) -> String {
        let id = self.id("s");
        self.sessions.push(SessionView {
            id: id.clone(),
            project: project.into(),
            name: name.into(),
            kind,
            cwd: cwd.to_path_buf(),
            notes: notes.into(),
            liveness: Liveness::Running,
            card: "working".into(),
            last_exit: None,
            last_stop_at_ms: None,
            quiet_secs: Some(0),
            waiting: false,
            waiting_reason: None,
            trust_question: false,
            resume_id: Some(format!("resume-{id}")),
            op: Some(op.into()),
        });
        self.resumable.push(id.clone());
        id
    }

    // One arm per command mirrors Switchboard's own table; splitting it
    // would only hide the mirror.
    #[allow(clippy::too_many_lines)]
    fn command(&mut self, op: &str, body: &Body) -> Reply {
        let kind = body.kind();
        if self.fail_next.as_deref() == Some(kind.as_str()) {
            self.fail_next = None;
            return Reply::failed(format!("{kind} refused by the test"));
        }
        let (made, launched): (Vec<Made>, bool) = match body {
            Body::SpaceNew { name } => {
                let id = self.id("space");
                self.spaces.push(SpaceView {
                    id: id.clone(),
                    name: name.clone(),
                    op: Some(op.into()),
                });
                (
                    vec![Made {
                        kind: RecordKind::Space,
                        id,
                    }],
                    false,
                )
            }
            Body::ProjectAdd { space, name, root } => {
                if !self.spaces.iter().any(|s| &s.id == space) {
                    return Reply::failed("no such space");
                }
                let id = self.id("p");
                self.projects.push(ProjectView {
                    id: id.clone(),
                    name: name.clone(),
                    root: root.clone(),
                    space: space.clone(),
                    op: Some(op.into()),
                });
                (
                    vec![Made {
                        kind: RecordKind::Project,
                        id,
                    }],
                    false,
                )
            }
            Body::SessionNew {
                project,
                name,
                session_kind,
                cwd,
                notes,
                ..
            } => {
                if !self.projects.iter().any(|p| &p.id == project) {
                    return Reply::failed("cannot start a session: unknown project");
                }
                let id = self.new_session(project, name, *session_kind, cwd, notes, op);
                (
                    vec![Made {
                        kind: RecordKind::Session,
                        id,
                    }],
                    true,
                )
            }
            Body::SetNew { space, name } => {
                let id = self.id("set");
                self.sets.push(SetView {
                    id: id.clone(),
                    name: name.clone(),
                    space: space.clone(),
                    items: vec![],
                    op: Some(op.into()),
                });
                (
                    vec![Made {
                        kind: RecordKind::Set,
                        id,
                    }],
                    false,
                )
            }
            Body::SetSync { set, items } => {
                for (i, a) in items.iter().enumerate() {
                    for b in &items[..i] {
                        if overlaps(a, b) {
                            return Reply::failed(format!("card {i} overlaps an earlier card"));
                        }
                    }
                }
                match self.sets.iter_mut().find(|s| &s.id == set) {
                    Some(s) => s.items.clone_from(items),
                    None => return Reply::failed("no such working set"),
                }
                (vec![], false)
            }
            Body::DefinitionInstall { definition } => {
                self.definitions.retain(|d| d.name != definition.name);
                self.definitions.push(definition.clone());
                (vec![], false)
            }
            Body::WorkflowStart {
                source,
                plan,
                definition,
                reviewer_cwd,
                reviewer_args: _,
            } => {
                let Some(src) = self.sessions.iter().find(|s| &s.id == source).cloned() else {
                    return Reply::failed("no such session");
                };
                if !self.resumable.contains(source) {
                    return Reply::failed(format!(
                        "{} cannot be the planner: only a Claude Code session with a transcript can be cloned",
                        src.name
                    ));
                }
                let Some(def) = self.definitions.iter().find(|d| &d.name == definition) else {
                    return Reply::failed(format!("no workflow definition called {definition}"));
                };
                let reviewer_kind = match def.reviewer {
                    switchboard_control::AgentKind::Claude => SessionKind::Claude,
                    switchboard_control::AgentKind::Codex => SessionKind::Codex,
                };
                let cwd = reviewer_cwd.clone().unwrap_or_else(|| src.cwd.clone());
                let reviewer =
                    self.new_session(&src.project, "plan review", reviewer_kind, &cwd, "", op);
                let planner = self.new_session(
                    &src.project,
                    "planner planner",
                    SessionKind::Claude,
                    &src.cwd,
                    "",
                    op,
                );
                let id = self.id("run");
                self.runs.push(RunView {
                    id: id.clone(),
                    project: src.project.clone(),
                    definition: definition.clone(),
                    state: RunState::AwaitingFeedback,
                    round: 1,
                    cap: 4,
                    source: source.clone(),
                    plan: plan.clone(),
                    reviewer: reviewer.clone(),
                    planner: Some(planner),
                    op: Some(op.into()),
                });
                (
                    vec![
                        Made {
                            kind: RecordKind::Run,
                            id,
                        },
                        Made {
                            kind: RecordKind::Session,
                            id: reviewer,
                        },
                    ],
                    true,
                )
            }
            Body::SessionKill { session } => {
                if let Some(s) = self.sessions.iter_mut().find(|s| &s.id == session) {
                    s.liveness = Liveness::Exited { code: None };
                    s.card = "exited".into();
                }
                self.killed.push(session.clone());
                (vec![], false)
            }
            Body::SessionNotes { session, text } => {
                self.notes.insert(session.clone(), text.clone());
                (vec![], false)
            }
            Body::SessionMove { session, project } => {
                if let Some(s) = self.sessions.iter_mut().find(|s| &s.id == session) {
                    s.project.clone_from(project);
                }
                (vec![], false)
            }
            Body::ProjectRename { project, name } => {
                if let Some(p) = self.projects.iter_mut().find(|p| &p.id == project) {
                    p.name.clone_from(name);
                }
                (vec![], false)
            }
            Body::ProjectRoot { project, root } => {
                if let Some(p) = self.projects.iter_mut().find(|p| &p.id == project) {
                    p.root.clone_from(root);
                }
                (vec![], false)
            }
            Body::SessionWaiting {
                session,
                on,
                reason,
            } => {
                if let Some(s) = self.sessions.iter_mut().find(|s| &s.id == session) {
                    s.waiting = *on;
                    s.waiting_reason = on.then(|| reason.clone());
                }
                self.waiting.insert(session.clone(), (*on, reason.clone()));
                (vec![], false)
            }
            Body::SessionTrust { session } => {
                if let Some(s) = self.sessions.iter_mut().find(|s| &s.id == session) {
                    s.trust_question = false;
                }
                self.trusted.push(session.clone());
                (vec![], false)
            }
            Body::WorkflowFinalize { run } => {
                self.run_mut(run).state = RunState::Finalized;
                (vec![], false)
            }
            Body::WorkflowPause { run } => {
                self.run_mut(run).state = RunState::Paused {
                    reason: "paused by you".into(),
                };
                (vec![], false)
            }
            Body::WorkflowContinue { run } => {
                self.run_mut(run).state = RunState::AwaitingFeedback;
                (vec![], false)
            }
            other => return Reply::failed(format!("{} is not built in the fake", other.kind())),
        };
        let reply = if launched {
            Reply::Launched { made: made.clone() }
        } else {
            Reply::Persisted { made: made.clone() }
        };
        if body.class() == switchboard_control::Class::Creation {
            self.log.push(LogLine {
                op: op.into(),
                kind: kind.clone(),
                ids: made.iter().map(|m| m.id.clone()).collect(),
                reply: None,
            });
        }
        if self.die_launching.as_deref() == Some(kind.as_str()) {
            // Saved, logged, never launched, never answered.
            self.die_launching = None;
            for m in &made {
                if m.kind == RecordKind::Session
                    && let Some(s) = self.sessions.iter_mut().find(|s| s.id == m.id)
                {
                    s.liveness = Liveness::Missing;
                    s.card = "not running".into();
                }
                self.interrupted.push(m.id.clone());
            }
            return Reply::failed("__die__");
        }
        if let Some(line) = self.log.iter_mut().find(|l| l.op == op) {
            line.reply = Some(reply.clone());
        }
        reply
    }

    #[allow(clippy::too_many_lines)]
    fn query(&mut self, body: &Body) -> Reply {
        match body {
            Body::Spaces => Reply::Spaces {
                spaces: self.spaces.clone(),
            },
            Body::Projects { space } => Reply::Projects {
                projects: self
                    .projects
                    .iter()
                    .filter(|p| space.as_ref().is_none_or(|s| &p.space == s))
                    .cloned()
                    .collect(),
            },
            Body::Sets { space } => Reply::Sets {
                sets: self
                    .sets
                    .iter()
                    .filter(|s| &s.space == space)
                    .cloned()
                    .collect(),
            },
            Body::Sessions { project } => Reply::Sessions {
                sessions: self
                    .sessions
                    .iter()
                    .filter(|s| &s.project == project)
                    .cloned()
                    .collect(),
            },
            Body::Session { session } => match self.fail_session_query.take() {
                Some(reason) => Reply::failed(reason),
                None => match self.sessions.iter().find(|s| &s.id == session) {
                    Some(s) => Reply::Session { session: s.clone() },
                    None => Reply::failed("no such session"),
                },
            },
            Body::Waiting => Reply::Waiting {
                sessions: self
                    .sessions
                    .iter()
                    .filter(|s| s.waiting)
                    .cloned()
                    .collect(),
            },
            Body::Workflow { run } => match self.runs.iter().find(|r| &r.id == run) {
                Some(r) => Reply::Workflow { run: r.clone() },
                None => Reply::failed("no such run"),
            },
            Body::Workflows { project } => Reply::Workflows {
                runs: self
                    .runs
                    .iter()
                    .filter(|r| &r.project == project)
                    .cloned()
                    .collect(),
            },
            Body::Find { operation } => {
                let mut records = Vec::new();
                for line in self.log.iter().filter(|l| &l.op == operation) {
                    for (n, id) in line.ids.iter().enumerate() {
                        let kind = match (line.kind.as_str(), n) {
                            ("project.add", _) => RecordKind::Project,
                            ("space.new", _) => RecordKind::Space,
                            ("set.new", _) => RecordKind::Set,
                            ("workflow.start", 0) => RecordKind::Run,
                            _ => RecordKind::Session,
                        };
                        let session = self.sessions.iter().find(|s| &s.id == id).cloned();
                        let run = self.runs.iter().find(|r| &r.id == id).cloned();
                        let present = session.is_some()
                            || run.is_some()
                            || self.projects.iter().any(|p| &p.id == id)
                            || self.spaces.iter().any(|s| &s.id == id)
                            || self.sets.iter().any(|s| &s.id == id);
                        records.push(Found {
                            kind,
                            id: id.clone(),
                            removed: !present,
                            session,
                            run,
                        });
                    }
                }
                Reply::Found { records }
            }
            Body::OpStatus { operation } => {
                let line = self.log.iter().find(|l| &l.op == operation);
                let interrupted =
                    line.is_some_and(|l| l.ids.iter().any(|id| self.interrupted.contains(id)));
                let held = self.pending_reply.iter().find(|(o, _)| o == operation);
                let status = match line {
                    _ if interrupted => OpStatus::Interrupted,
                    Some(l)
                        if held.is_some()
                            && self.in_progress_for.as_deref() == Some(l.kind.as_str()) =>
                    {
                        OpStatus::InProgress
                    }
                    Some(LogLine { reply: Some(r), .. }) => OpStatus::Done {
                        reply: Box::new(r.clone()),
                    },
                    Some(_) if held.is_some() => OpStatus::Done {
                        reply: Box::new(held.unwrap().1.clone()),
                    },
                    Some(_) => OpStatus::Interrupted,
                    None => OpStatus::Unknown,
                };
                Reply::OpStatus { status }
            }
            _ => Reply::failed("not a query"),
        }
    }
}

fn overlaps(a: &Pin, b: &Pin) -> bool {
    a.rect.x < b.rect.x + b.rect.w
        && b.rect.x < a.rect.x + a.rect.w
        && a.rect.y < b.rect.y + b.rect.h
        && b.rect.y < a.rect.y + a.rect.h
}

/// The runner's handle: the fake behind a lock the test also holds.
#[derive(Clone)]
pub struct SharedPort(pub Arc<Mutex<FakeSwitchboard>>);

impl Port for SharedPort {
    fn call(&mut self, request: &Request) -> io::Result<Reply> {
        let mut sb = self.0.lock().unwrap();
        sb.calls.push(request.clone());
        if !request.body.is_command() {
            return Ok(sb.query(&request.body));
        }
        if let Some(reply) = sb.replied(&request.op) {
            return Ok(reply);
        }
        let kind = request.body.kind();
        let in_progress = sb.in_progress_for.as_deref() == Some(kind.as_str());
        let reply = sb.command(&request.op, &request.body);
        if matches!(&reply, Reply::Failed { reason } if reason == "__die__") {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "the control socket closed before replying",
            ));
        }
        if in_progress {
            // Logged and being run; the socket dies before the reply.
            if let Some(line) = sb.log.iter_mut().find(|l| l.op == request.op) {
                line.reply = None;
            }
            sb.pending_reply.push((request.op.clone(), reply));
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "the control socket closed before replying",
            ));
        }
        if sb.drop_reply_for.as_deref() == Some(kind.as_str()) {
            sb.drop_reply_for = None;
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "the control socket closed before replying",
            ));
        }
        Ok(reply)
    }
}

/// A fake repository shared with the test, so a runner can be replaced
/// (a restart) over the same heads and checks.
pub struct SharedRepo(pub Arc<Mutex<FakeRepo>>);

impl Repo for SharedRepo {
    fn ensure_clone(&mut self, url: &str, dir: &std::path::Path) -> anyhow::Result<()> {
        self.0.lock().unwrap().ensure_clone(url, dir)
    }
    fn fetch(&mut self, dir: &std::path::Path, remote: &str) -> anyhow::Result<()> {
        self.0.lock().unwrap().fetch(dir, remote)
    }
    fn worktree_add(
        &mut self,
        repo: &std::path::Path,
        dir: &std::path::Path,
        branch: &str,
        base: &str,
    ) -> anyhow::Result<()> {
        self.0.lock().unwrap().worktree_add(repo, dir, branch, base)
    }
    fn is_worktree_of(
        &self,
        repo: &std::path::Path,
        dir: &std::path::Path,
        branch: &str,
    ) -> anyhow::Result<bool> {
        self.0.lock().unwrap().is_worktree_of(repo, dir, branch)
    }
    fn head(&self, dir: &std::path::Path) -> anyhow::Result<String> {
        self.0.lock().unwrap().head(dir)
    }
    fn is_clean(&self, dir: &std::path::Path) -> anyhow::Result<bool> {
        self.0.lock().unwrap().is_clean(dir)
    }
    fn remote_url(&self, dir: &std::path::Path) -> anyhow::Result<Option<String>> {
        self.0.lock().unwrap().remote_url(dir)
    }
    fn summary(&self, dir: &std::path::Path, base: &str) -> anyhow::Result<String> {
        self.0.lock().unwrap().summary(dir, base)
    }
    fn worktree_move(
        &mut self,
        repo: &std::path::Path,
        from: &std::path::Path,
        to: &std::path::Path,
    ) -> anyhow::Result<()> {
        self.0.lock().unwrap().worktree_move(repo, from, to)
    }
    fn worktree_repair(
        &mut self,
        repo: &std::path::Path,
        dir: &std::path::Path,
    ) -> anyhow::Result<()> {
        self.0.lock().unwrap().worktree_repair(repo, dir)
    }
    fn run(
        &mut self,
        dir: &std::path::Path,
        argv: &[String],
        env: &[(String, String)],
    ) -> anyhow::Result<()> {
        self.0.lock().unwrap().run(dir, argv, env)
    }
    fn start_check(
        &mut self,
        key: &str,
        dir: &std::path::Path,
        argv: &[String],
        env: &[(String, String)],
        log: &std::path::Path,
    ) -> anyhow::Result<()> {
        self.0.lock().unwrap().start_check(key, dir, argv, env, log)
    }
    fn poll_check(&mut self, key: &str) -> Option<anyhow::Result<i32>> {
        self.0.lock().unwrap().poll_check(key)
    }
}
