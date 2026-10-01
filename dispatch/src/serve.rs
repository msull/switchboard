//! Dispatch's own port: `<data dir>/dispatch.sock`, served while
//! `dispatch run` is up. A client (Switchboard's Dispatch page, a
//! remote shell) reads tickets as views and does what the command line
//! does: answer a decision, reorder a queue, take an issue. Nothing here
//! bypasses the scheduler: every command goes through the same `Runner`
//! methods the CLI calls, under the same writer lock, so the runner's
//! loop and a request never interleave inside a record.

use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use anyhow::{Context, Result, bail};
use dispatch_control::{
    AttemptView, Body, DecisionView, LaneView, ProjectView, Reply, Request, SOCKET_FILE, Status,
    TicketView,
};

use crate::epoch_ms;
use crate::github::Issues;
use crate::pipeline::{Pipeline, Source};
use crate::scheduler::Runner;
use crate::store::DataDir;
use crate::ticket::{
    AttemptKind, AttemptState, Decision, DecisionState, PullRequestSource, SourceSnapshot, Ticket,
    TicketState,
};

/// A Unix socket path may be at most 104 bytes on macOS.
const MAX_SOCKET_PATH: usize = 100;

/// What a request needs: the runner (its data directory, its way to
/// Switchboard and git) and the issue source, shared with every
/// connection under one lock.
pub struct Handler {
    pub runner: Runner,
    pub issues: Box<dyn Issues>,
}

impl Handler {
    /// Answer one request. Never panics on a bad one; the reply says.
    pub fn handle(&mut self, request: &Request, now_ms: u64) -> Reply {
        match self.answer(&request.body, now_ms) {
            Ok(reply) => reply,
            Err(e) => Reply::failed(format!("{e:#}")),
        }
    }

    fn answer(&mut self, body: &Body, now_ms: u64) -> Result<Reply> {
        Ok(match body {
            Body::Status => Reply::Status(status(&self.runner)?),
            Body::Ticket { id } => {
                let t = self.runner.load_ticket(id)?;
                Reply::Ticket(self.view(&t))
            }
            Body::Artifact { ticket, path } => {
                let dir = self.runner.data.ticket_dir(ticket);
                let (dir, file) = (
                    dir.canonicalize().unwrap_or(dir),
                    path.canonicalize().unwrap_or_else(|_| path.clone()),
                );
                if !file.starts_with(&dir) {
                    bail!("{} is not a file of ticket {ticket}", path.display());
                }
                let text = fs::read_to_string(&file)
                    .with_context(|| format!("read {}", file.display()))?;
                Reply::Artifact { text }
            }
            Body::Decide {
                ticket,
                decision,
                answer,
                note,
            } => {
                let d = self
                    .runner
                    .decide(ticket, decision, answer, note.as_deref(), now_ms)?;
                Reply::Decided(decision_view(ticket, &d))
            }
            Body::Queue { project, order } => {
                let ps = if order.is_empty() {
                    self.runner.load_project(project)?
                } else {
                    let order: Vec<&str> = order.iter().map(String::as_str).collect();
                    self.runner.reorder_queue(project, &order)?
                };
                Reply::Queue { order: ps.queue }
            }
            Body::Worktrees { path, migrate } => {
                Reply::Worktrees(self.runner.set_worktrees(path.clone(), *migrate, now_ms)?)
            }
            Body::Resume { ticket } => {
                let t = self.runner.resume(ticket, now_ms)?;
                Reply::Ticket(self.view(&t))
            }
            Body::Close { ticket, reason } => {
                let t = self
                    .runner
                    .close_by_hand(ticket, reason.as_deref(), now_ms)?;
                Reply::Ticket(self.view(&t))
            }
            Body::Take { project, issue } => {
                let t = take_issue(&mut self.runner, &*self.issues, project, issue, now_ms)?;
                Reply::Taken(self.view(&t))
            }
        })
    }

    fn view(&self, t: &Ticket) -> TicketView {
        let stages = self
            .runner
            .pipeline_of(t)
            .map(|p| p.stages.iter().map(|s| s.name.clone()).collect())
            .unwrap_or_default();
        ticket_view(t, stages)
    }
}

/// Make a ticket from the project's source, as `dispatch take` does.
pub fn take_issue(
    runner: &mut Runner,
    issues: &dyn Issues,
    project: &str,
    issue: &str,
    now_ms: u64,
) -> Result<Ticket> {
    let path = runner.data.pipeline(project);
    let text = fs::read_to_string(&path)
        .with_context(|| format!("no pipeline for {project} at {}", path.display()))?;
    let pipeline = Pipeline::parse(&text)?;
    let Source::Github { repo, .. } = &pipeline.source else {
        bail!("only a GitHub source is taken by number");
    };
    let number: u64 = issue
        .trim_start_matches('#')
        .parse()
        .context("an issue number")?;
    let source = issues.fetch(repo, number, now_ms)?;
    runner.take(project, &text, source, now_ms)
}

/// Make a ticket from someone else's pull requests, one per lane, on
/// the project's pull-request pipeline. A spec is `<lane>/<number>`,
/// or `<number>` alone where the pipeline has one lane.
pub fn take_pull_requests(
    runner: &mut Runner,
    project: &str,
    specs: &[&str],
    now_ms: u64,
) -> Result<Ticket> {
    let path = runner.data.pr_pipeline(project);
    let text = fs::read_to_string(&path).with_context(|| {
        format!(
            "no pull-request pipeline for {project} at {}",
            path.display()
        )
    })?;
    let pipeline = Pipeline::parse(&text)?;
    if pipeline.source != Source::PullRequest {
        bail!(
            "{} is not a pull-request pipeline: its source is not kind = \"pull-request\"",
            path.display()
        );
    }
    let lanes: Vec<&str> = pipeline.lanes.iter().map(|l| l.name.as_str()).collect();
    let mut prs: Vec<PullRequestSource> = Vec::new();
    for spec in specs {
        let pr = pull_request_source(runner, &pipeline, &lanes, spec)?;
        if prs.iter().any(|x| x.lane == pr.lane) {
            bail!("lane {} is named twice", pr.lane);
        }
        if pipeline.lane(&pr.lane).is_some_and(|l| l.repo.is_none())
            && let Some(other) = prs
                .iter()
                .find(|x| pipeline.lane(&x.lane).is_some_and(|l| l.repo.is_none()))
        {
            bail!(
                "lanes {} and {} share the project's repository, so one pull request covers both",
                other.lane,
                pr.lane
            );
        }
        prs.push(pr);
    }
    let Some(first) = prs.first() else {
        bail!("name at least one pull request as <lane>/<number>");
    };
    let identity = prs
        .iter()
        .map(|pr| format!("{}:{}!{}", pr.provider, pr.repo, pr.number))
        .collect::<Vec<_>>()
        .join("+");
    let body = prs
        .iter()
        .map(|pr| format!("{}: PR #{} {} ({})", pr.lane, pr.number, pr.title, pr.url))
        .collect::<Vec<_>>()
        .join("\n");
    let source = SourceSnapshot {
        kind: "pull-request".into(),
        identity,
        number: Some(first.number),
        title: first.title.clone(),
        body,
        url: Some(first.url.clone()),
        labels: Vec::new(),
        taken_at_ms: now_ms,
        pull_requests: prs,
    };
    runner.take(project, &text, source, now_ms)
}

/// One `<remote>:<lane>/<n>` (or `<lane>/<n>`, or `<n>` with one lane)
/// read from its provider.
fn pull_request_source(
    runner: &Runner,
    pipeline: &Pipeline,
    lanes: &[&str],
    spec: &str,
) -> Result<PullRequestSource> {
    // `<remote>:<lane>/<n>` takes the PR from a named mirror.
    let (remote_name, spec_rest) = match spec.split_once(':') {
        Some((remote, rest)) => (Some(remote), rest),
        None => (None, spec),
    };
    let (lane, number) = match spec_rest.split_once('/') {
        Some((lane, number)) => (lane.to_owned(), number),
        None if lanes.len() == 1 => (lanes[0].to_owned(), spec_rest),
        None => bail!(
            "name the lane as <lane>/<number>; the lanes are {}",
            lanes.join(", ")
        ),
    };
    let number: u64 = number
        .trim_start_matches('#')
        .parse()
        .with_context(|| format!("a pull request number in {spec:?}"))?;
    let def = pipeline
        .lane(&lane)
        .with_context(|| format!("no lane {lane:?}; the lanes are {}", lanes.join(", ")))?;
    let default_remote = if def.repo.is_some() {
        pipeline.lane_remote(def)
    } else {
        pipeline.project.remote.as_str()
    };
    let remote_name = remote_name.unwrap_or(default_remote).to_owned();
    let remote = pipeline
        .remote_url(Some(def), &remote_name)
        .with_context(|| {
            format!("lane {lane} has no remote {remote_name:?} to read a pull request from")
        })?;
    let provider = crate::scheduler::guess_provider(&remote).to_owned();
    let repo = match provider.as_str() {
        "github" => crate::github::github_repo(&remote),
        "bitbucket" => crate::bitbucket::bitbucket_repo(&remote),
        _ => None,
    }
    .with_context(|| format!("lane {lane}: {remote:?} is not on a provider Dispatch reads"))?;
    let pr = runner.prs_for(&provider).by_number(&repo, number)?;
    if pr.state != "open" {
        bail!("PR #{number} in {repo} is {}", pr.state);
    }
    // GitHub serves every PR's head as a pull ref, from whichever
    // fork; Bitbucket has the branch itself.
    let local = if provider == "github" {
        format!("pr/{number}")
    } else {
        pr.branch.clone()
    };
    Ok(PullRequestSource {
        lane,
        provider,
        repo,
        number,
        url: pr.url,
        branch: pr.branch,
        base: pr.base,
        remote: remote_name,
        local,
        head: pr.head,
        title: pr.title,
    })
}

/// A code review attempt's rounds, for the page.
fn round_views(a: &crate::ticket::Attempt) -> Vec<dispatch_control::ReviewRoundView> {
    a.rounds
        .iter()
        .map(|r| dispatch_control::ReviewRoundView {
            n: r.n,
            base: r.base.clone(),
            head: r.head.clone(),
            state: match &r.state {
                crate::ticket::RoundState::Reviewing => "reviewing".to_owned(),
                crate::ticket::RoundState::Converged => "converged".to_owned(),
                crate::ticket::RoundState::Findings => "findings".to_owned(),
                crate::ticket::RoundState::Fixing => "fixing".to_owned(),
                crate::ticket::RoundState::Fixed => "fixed".to_owned(),
                crate::ticket::RoundState::Accepted => "accepted".to_owned(),
                crate::ticket::RoundState::Failed { reason } => {
                    format!("failed: {reason}")
                }
            },
            open_points: r.open_points,
            head_after: r.head_after.clone(),
            reviewers: r
                .reviewers
                .iter()
                .map(|x| {
                    (
                        x.name.clone(),
                        match &x.result {
                            None if x.session.is_some() || x.launched => "running".to_owned(),
                            None => "starting".to_owned(),
                            Some(crate::ticket::ReviewerResult::Clean) => "clean".to_owned(),
                            Some(crate::ticket::ReviewerResult::Findings) => "findings".to_owned(),
                            Some(crate::ticket::ReviewerResult::Failed { reason }) => {
                                format!("failed: {reason}")
                            }
                        },
                    )
                })
                .collect(),
        })
        .collect()
}

/// Every project's queue and every ticket.
pub fn status(runner: &Runner) -> Result<Status> {
    let records = runner.tickets()?;
    let mut projects = Vec::new();
    for name in runner.projects()? {
        let ps = runner.load_project(&name)?;
        // The limits come from the project's current pipeline file; the
        // counts are the scheduler's: an active ticket with an open
        // attempt holds a slot, and every pending decision counts.
        let policy = fs::read_to_string(runner.data.pipeline(&name))
            .ok()
            .and_then(|text| Pipeline::parse(&text).ok())
            .map(|p| p.policy)
            .unwrap_or_default();
        let mine = records.iter().filter(|t| t.project == name);
        let running = mine
            .clone()
            .filter(|t| t.active() && t.attempts.iter().any(crate::scheduler::costs_slot))
            .count();
        let pending = mine.map(|t| t.pending_decisions().len()).sum::<usize>();
        projects.push(ProjectView {
            name,
            queue: ps.queue,
            slots: policy.slots,
            waiting_on_me: policy.waiting_on_me,
            running: u32::try_from(running).unwrap_or(u32::MAX),
            pending: u32::try_from(pending).unwrap_or(u32::MAX),
            min_free_gb: policy.min_free_gb,
            free_gb: runner.free_gb(),
        });
    }
    let mut tickets = Vec::new();
    for t in records {
        let stages = runner
            .pipeline_of(&t)
            .map(|p| p.stages.iter().map(|s| s.name.clone()).collect())
            .unwrap_or_default();
        tickets.push(ticket_view(&t, stages));
    }
    tickets.sort_by_key(|t| std::cmp::Reverse(t.updated_ms));
    Ok(Status {
        data_dir: runner.data.root.clone(),
        worktrees: runner.data.worktrees_dir(),
        projects,
        tickets,
    })
}

/// The record as a reader sees it.
#[must_use]
pub fn ticket_view(t: &Ticket, stages: Vec<String>) -> TicketView {
    let (state, reason) = match &t.state {
        TicketState::Active => ("active", None),
        TicketState::Parking { reason } => ("parking", Some(reason.clone())),
        TicketState::Parked { reason } => ("parked", Some(reason.clone())),
        TicketState::Closing { reason } => ("closing", Some(reason.clone())),
        TicketState::Closed { reason } => ("closed", Some(reason.clone())),
    };
    TicketView {
        id: t.id.clone(),
        project: t.project.clone(),
        kind: t.source.kind.clone(),
        number: t.source.number,
        title: t.source.title.clone(),
        body: t.source.body.clone(),
        url: t.source.url.clone(),
        labels: t.source.labels.clone(),
        state: state.into(),
        reason,
        stages,
        stage: t.stage,
        tree: t.tree.clone(),
        tree_removed: t.close.tree_removed,
        trees_kept: t.close.trees_kept.clone(),
        lanes: t
            .lanes
            .iter()
            .map(|l| LaneView {
                name: l.name.clone(),
                worktree: l.worktree.clone(),
                branch: l.branch.clone(),
                chosen: l.chosen,
                setup_done: l.setup_done,
                removed: l.removed,
            })
            .collect(),
        attempts: t
            .attempts
            .iter()
            .map(|a| {
                let (state, reason) = match &a.state {
                    AttemptState::Starting => ("starting", None),
                    AttemptState::Running => ("running", None),
                    AttemptState::Complete => ("complete", None),
                    AttemptState::Failed { reason } => ("failed", Some(reason.clone())),
                    AttemptState::Cancelled { reason } => ("cancelled", Some(reason.clone())),
                };
                AttemptView {
                    stage: a.stage.clone(),
                    n: a.n,
                    context: a.context.clone(),
                    kind: match a.kind {
                        AttemptKind::Agent => "agent",
                        AttemptKind::Workflow => "workflow",
                        AttemptKind::GateOnly => "gate-only",
                        AttemptKind::Review => "review",
                    }
                    .into(),
                    rounds: round_views(a),
                    state: state.into(),
                    reason,
                    session: a.session.clone(),
                    run: a.run.clone(),
                    artifacts: a
                        .artifacts
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    head: a.head.clone(),
                    checks: a.gate.as_ref().map(|g| dispatch_control::ChecksView {
                        head: g.head.clone(),
                        exit: g.exit,
                        log: g.log.clone(),
                    }),
                    pr: a.pr.as_ref().map(|pr| dispatch_control::PullRequestView {
                        provider: pr.provider.clone(),
                        repo: pr.repo.clone(),
                        number: pr.number,
                        url: pr.url.clone(),
                        head: pr.head.clone(),
                        checks: pr.checks.clone(),
                    }),
                    started_ms: a.started_ms,
                    ended_ms: a.ended_ms,
                }
            })
            .collect(),
        decisions: t
            .decisions
            .iter()
            .map(|d| decision_view(&t.id, d))
            .collect(),
        root_project: t.root_project.clone(),
        current_session: t.current_session().cloned(),
        created_ms: t.created_ms,
        updated_ms: t.updated_ms,
    }
}

fn decision_view(ticket: &str, d: &Decision) -> DecisionView {
    let (state, answer, note) = match &d.state {
        DecisionState::Pending => ("pending", None, None),
        DecisionState::Answered {
            answer,
            note,
            acted,
            ..
        } => (
            if *acted { "acted" } else { "answered" },
            Some(answer.clone()),
            note.clone(),
        ),
        DecisionState::Cancelled => ("cancelled", None, None),
    };
    DecisionView {
        id: d.id.clone(),
        ticket: ticket.into(),
        stage: d.stage.clone(),
        name: d.name.clone(),
        question: d.question.clone(),
        options: d.options.clone(),
        recommendation: d.recommendation.clone(),
        multiple: d.name == "lanes",
        state: state.into(),
        answer,
        note,
        made_ms: d.made_ms,
    }
}

/// The socket, owner-only, with one thread accepting and one per
/// connection; each request is answered under the handler's lock.
/// Dropping it closes the socket and removes the file.
pub struct Server {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    pub fn bind(data: &DataDir, handler: Handler) -> io::Result<Self> {
        fs::create_dir_all(&data.root)?;
        let path = data.root.join(SOCKET_FILE);
        if path.as_os_str().len() > MAX_SOCKET_PATH {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "socket path {} is longer than {MAX_SOCKET_PATH} bytes; use a shorter data directory",
                    path.display()
                ),
            ));
        }
        // Only the runner that holds `runner.lock` gets here, so a file
        // left by a dead runner is stale.
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let handler = Arc::new(Mutex::new(handler));
        let thread = {
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("dispatch-port".into())
                .spawn(move || {
                    for stream in listener.incoming() {
                        if stop.load(Ordering::SeqCst) {
                            break;
                        }
                        let Ok(stream) = stream else { continue };
                        let handler = Arc::clone(&handler);
                        let _ = std::thread::Builder::new()
                            .name("dispatch-port-conn".into())
                            .spawn(move || serve_connection(stream, &handler));
                    }
                })?
        };
        Ok(Self {
            path,
            stop,
            thread: Some(thread),
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(&self.path);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = fs::remove_file(&self.path);
    }
}

fn serve_connection(stream: UnixStream, handler: &Mutex<Handler>) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match Request::parse(&line) {
            Err(e) => Reply::failed(format!("bad request: {e}")),
            Ok(request) => {
                let now = epoch_ms(std::time::SystemTime::now());
                match handler.lock() {
                    Ok(mut h) => h.handle(&request, now),
                    Err(_) => Reply::failed("the runner's handler is poisoned"),
                }
            }
        };
        if writer.write_all(reply.to_line().as_bytes()).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use dispatch_control::Client;

    use super::*;
    use crate::git::FakeRepo;
    use crate::github::FakeIssues;
    use crate::port::Port;

    struct NoPort;
    impl Port for NoPort {
        fn call(
            &mut self,
            _: &switchboard_control::Request,
        ) -> io::Result<switchboard_control::Reply> {
            Err(io::Error::other("no Switchboard in this test"))
        }
    }

    const PIPELINE: &str = r#"
version = 1
[project]
name = "P"
root = "/tmp/p"
space = "S"
[source]
kind = "github"
repo = "o/r"
label = "dispatch"
[[lanes]]
name = "repo"
path = "."
[operators.a]
kind = "claude"
[[stages]]
name = "investigate"
operator = "a"
context = "root"
writes = ["notes"]
prompt = "go {notes}"
[policy]
slots = 1
"#;

    fn handler(dir: &Path) -> Handler {
        let data = DataDir::new(dir.join("d"));
        fs::create_dir_all(data.root.join("pipelines")).unwrap();
        fs::write(data.pipeline("P"), PIPELINE).unwrap();
        Handler {
            runner: Runner::new(data, Box::new(NoPort), Box::new(FakeRepo::default())),
            issues: Box::new(FakeIssues {
                issues: vec![("o/r".into(), 7, "Seven".into(), "body".into())],
            }),
        }
    }

    #[test]
    fn the_port_takes_reads_and_decides_like_the_command_line() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = handler(dir.path());
        let taken = h.handle(
            &Request::new(
                "1",
                Body::Take {
                    project: "P".into(),
                    issue: "#7".into(),
                },
            ),
            1_000,
        );
        let Reply::Taken(t) = taken else {
            panic!("{taken:?}")
        };
        assert_eq!(
            (t.number, t.stages.len(), t.state.as_str()),
            (Some(7), 1, "active")
        );
        let Reply::Status(status) = h.handle(&Request::new("2", Body::Status), 2_000) else {
            panic!("a status")
        };
        assert_eq!(status.projects[0].queue, vec![t.id.clone()]);
        assert_eq!(
            (status.projects[0].slots, status.projects[0].running),
            (1, 0),
            "the pipeline's limit and nothing running yet"
        );
        assert_eq!(status.tickets[0].title, "Seven");
        assert_eq!(status.data_dir, h.runner.data.root);
        // A file outside the ticket's directory is refused by path.
        let outside = h.handle(
            &Request::new(
                "3",
                Body::Artifact {
                    ticket: t.id.clone(),
                    path: h.runner.data.pipeline("P"),
                },
            ),
            3_000,
        );
        assert!(matches!(outside, Reply::Failed { .. }), "{outside:?}");
        let file = h.runner.data.ticket_dir(&t.id).join("notes.md");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "# notes").unwrap();
        assert_eq!(
            h.handle(
                &Request::new(
                    "4",
                    Body::Artifact {
                        ticket: t.id.clone(),
                        path: file,
                    },
                ),
                4_000,
            ),
            Reply::Artifact {
                text: "# notes".into()
            }
        );
        let no_such = h.handle(
            &Request::new(
                "5",
                Body::Decide {
                    ticket: t.id.clone(),
                    decision: "d9".into(),
                    answer: "yes".into(),
                    note: None,
                },
            ),
            5_000,
        );
        assert!(matches!(no_such, Reply::Failed { .. }));
    }

    /// A close through the port: the ticket's tree removed and the
    /// closed view in the reply; a second close refused with the reason.
    #[test]
    fn the_port_closes_a_ticket_and_refuses_a_second_close() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = handler(dir.path());
        // A pipeline that cuts its own trees, with the tree cut by hand:
        // stepping would need a Switchboard this test does not have.
        let text = PIPELINE.replace("root = \"/tmp/p\"", "repo = \"git@example.com:o/r.git\"");
        fs::write(h.runner.data.pipeline("P"), text).unwrap();
        let Reply::Taken(view) = h.handle(
            &Request::new(
                "1",
                Body::Take {
                    project: "P".into(),
                    issue: "7".into(),
                },
            ),
            1_000,
        ) else {
            panic!("taken")
        };
        let tree = dir.path().join("wt").join(&view.id);
        fs::create_dir_all(&tree).unwrap();
        let mut t = h.runner.load_ticket(&view.id).unwrap();
        t.tree = Some(tree.clone());
        h.runner.save_ticket(&mut t, 1_500).unwrap();
        let close = Body::Close {
            ticket: view.id.clone(),
            reason: Some("done elsewhere".into()),
        };
        let reply = h.handle(&Request::new("2", close.clone()), 2_000);
        let Reply::Ticket(closed) = reply else {
            panic!("{reply:?}")
        };
        assert_eq!(
            (closed.state.as_str(), closed.reason.as_deref()),
            ("closed", Some("done elsewhere"))
        );
        assert!(closed.tree_removed && closed.trees_kept.is_none());
        assert!(!tree.exists());
        let again = h.handle(&Request::new("3", close), 3_000);
        assert!(
            matches!(&again, Reply::Failed { reason } if reason.contains("already closed")),
            "{again:?}"
        );
    }

    #[test]
    fn the_socket_answers_a_client_and_leaves_no_file_behind() {
        let dir = std::env::temp_dir().join(format!("dsv-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let h = handler(&dir);
        let data = DataDir::new(h.runner.data.root.clone());
        let server = Server::bind(&data, h).unwrap();
        let mut client = Client::connect(server.path()).unwrap();
        let reply = client.call(&Request::new("s", Body::Status)).unwrap();
        assert!(matches!(reply, Reply::Status(_)), "{reply:?}");
        let path = server.path().to_path_buf();
        drop(server);
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
