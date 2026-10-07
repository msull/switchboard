//! `dispatch runner stop | start | restart`: the runner the app runs,
//! stopped or started through Switchboard's control port as its buttons
//! do, then watched through `runner.json` until the change shows. A stop
//! or restart is refused while a ticket runs a deploy, since the kill
//! would cut its command short.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use switchboard_control::{Body, Client, Reply, Request, RunnerVerb};

use crate::health::{CHECK_TIMEOUT, RUNNER_FILE, RunnerFile, STALE_MS, alive};
use crate::pipeline::Pipeline;
use crate::store::{DataDir, read_ticket};
use crate::ticket::Ticket;

/// How often the wait reads `runner.json`.
const POLL: Duration = Duration::from_millis(500);

/// The first ticket with an open attempt of a gate-only command stage (a
/// deploy), as `(ticket id, stage)`: the attempt counts before its
/// command starts as well as while it runs. The stage is looked up in
/// the ticket's own pipeline copy, which `pipeline_of` reads; a copy
/// that will not read is an error naming the ticket, since a ticket
/// that cannot be told apart from a deploy is not safe to cut short.
pub fn mid_apply(
    tickets: &[Ticket],
    pipeline_of: impl Fn(&Ticket) -> Result<Pipeline>,
) -> Result<Option<(String, String)>> {
    for t in tickets {
        let open: Vec<&str> = t
            .attempts
            .iter()
            .filter(|a| a.is_open())
            .map(|a| a.stage.as_str())
            .collect();
        if open.is_empty() {
            continue;
        }
        let pipeline = pipeline_of(t).with_context(|| {
            format!(
                "ticket {} has an open attempt and its pipeline copy will not read",
                t.id
            )
        })?;
        let deploy = open.into_iter().find(|name| {
            pipeline
                .stages
                .iter()
                .any(|s| s.name == *name && s.is_command_stage())
        });
        if let Some(stage) = deploy {
            return Ok(Some((t.id.clone(), stage.to_owned())));
        }
    }
    Ok(None)
}

/// `runner.json` as it is now, if it reads.
fn runner_file(data: &DataDir) -> Option<RunnerFile> {
    std::fs::read(data.root.join(RUNNER_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

/// Ask the app at `socket` to stop, start or restart its runner, and wait
/// up to `wait` for `runner.json` to show it; `clock` reads the wall
/// clock in milliseconds since the epoch. The writer lock is held from
/// the deploy check until the app replies, so no pass opens a deploy in
/// between, and let go before the wait, since the new runner takes it
/// while it recovers.
///
/// A stop is done when the pid `runner.json` held is gone. A start is
/// done on a pass written after the request by a pid that is alive: a
/// runner already up writes one within a second, a killed one never
/// does, and one still recovering writes its first when it is done. A
/// restart is done when a runner started after the request is alive,
/// since the old one may still pass once before its kill lands.
pub fn run(
    data: &DataDir,
    socket: &Path,
    verb: RunnerVerb,
    wait: Duration,
    clock: impl Fn() -> u64,
) -> Result<String> {
    let asked_ms = clock();
    let (before, reply) = data.with_lock(|| {
        if verb != RunnerVerb::Start {
            let tickets = data
                .ticket_files()?
                .iter()
                .map(|p| read_ticket(p))
                .collect::<Result<Vec<_>>>()?;
            if let Some((id, stage)) = mid_apply(&tickets, pipeline_of)? {
                bail!(
                    "ticket {id} is running `{stage}`; {} the runner when it ends",
                    verb.word()
                );
            }
        }
        let before = runner_file(data);
        let op = format!("runner-{}-{}", verb.word(), uuid::Uuid::new_v4());
        let mut client = Client::connect(socket)
            .with_context(|| format!("Switchboard's control socket at {}", socket.display()))?;
        let reply = client
            .call(&Request::new(op, Body::DispatchRunner { action: verb }))
            .context("ask Switchboard")?;
        Ok((before, reply))
    })?;
    match reply {
        Reply::Failed { reason } if reason.contains("dispatch.runner") => {
            bail!("this Switchboard is older than `dispatch runner`; restart the app once")
        }
        Reply::Failed { reason } => bail!("{reason}"),
        _ => {}
    }
    let old = before.map(|f| f.pid).filter(|pid| *pid != 0 && alive(*pid));
    let up = |since: fn(&RunnerFile) -> u64| {
        runner_file(data)
            .filter(|f| since(f) >= asked_ms && f.pid != 0 && alive(f.pid))
            .map(|f| format!("runner up: pid {}", f.pid))
    };
    let started = Instant::now();
    loop {
        let done = match (verb, old) {
            (RunnerVerb::Stop, Some(pid)) if alive(pid) => None,
            (RunnerVerb::Stop, Some(pid)) => Some(format!("runner stopped (pid {pid})")),
            (RunnerVerb::Stop, None) => Some("runner stopped".to_owned()),
            (RunnerVerb::Start, _) => up(|f| f.last_pass_ms),
            (RunnerVerb::Restart, _) => up(|f| f.started_ms),
        };
        if let Some(line) = done {
            return Ok(line);
        }
        if started.elapsed() >= wait {
            let checked =
                crate::health::check(data, socket, CHECK_TIMEOUT, STALE_MS, clock(), false);
            bail!(
                "the runner did not {} within {}s:\n{}\nsee the Dispatch page in Switchboard",
                verb.word(),
                wait.as_secs(),
                checked.lines.join("\n")
            );
        }
        std::thread::sleep(POLL);
    }
}

/// A ticket's own pipeline copy, read and parsed.
fn pipeline_of(t: &Ticket) -> Result<Pipeline> {
    let text = std::fs::read_to_string(&t.pipeline_file)
        .with_context(|| format!("read {}", t.pipeline_file.display()))?;
    Pipeline::parse(&text)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;

    use super::*;
    use crate::scheduler::new_attempt;
    use crate::ticket::{
        AttemptKind, AttemptState, CloseProgress, GateRun, SourceSnapshot, TicketState,
    };

    const DEPLOYING: &str = r#"
version = 1

[project]
name = "orchard"
root = "/r"
space = "Dispatch"

[source]
kind = "github"
repo = "o/r"
label = "dispatch"

[operators.implementer]
kind = "claude"

[[lanes]]
name = "repo"
path = "."

[[stages]]
name = "implement"
operator = "implementer"
context = "root"
writes = ["notes"]
prompt = "Do it; notes to {notes}."
gate = { kind = "command", argv = ["true"], in = "lane" }

[[stages]]
name = "deploy"
gate = { kind = "command", argv = ["true"], in = "lane" }
"#;

    /// A command gate started, with `exit` once it ended.
    fn gate(exit: Option<i32>) -> GateRun {
        GateRun {
            head: "h".into(),
            argv: vec!["true".into()],
            log: "/l".into(),
            started_ms: 1,
            exit,
            group: None,
            lost_since_ms: None,
        }
    }

    fn ticket(stage: &str, at: AttemptState, gate: Option<GateRun>) -> Ticket {
        let mut a = new_attempt(
            stage,
            1,
            "repo",
            AttemptKind::GateOnly,
            at,
            BTreeMap::new(),
            1,
        );
        a.gate = gate;
        Ticket {
            version: 0,
            id: "t1".into(),
            project: "orchard".into(),
            source: SourceSnapshot {
                kind: "github".into(),
                identity: "o/r#7".into(),
                pull_requests: Vec::new(),
                number: Some(7),
                title: "a title".into(),
                body: String::new(),
                url: None,
                labels: vec![],
                taken_at_ms: 0,
                taken_by: None,
            },
            pipeline_fingerprint: String::new(),
            pipeline_file: PathBuf::new(),
            lanes: vec![],
            tree: None,
            stage: 0,
            attempts: vec![a],
            decisions: vec![],
            ledger: vec![],
            processes: vec![],
            root_project: None,
            rework: BTreeMap::new(),
            refreshed_stage: None,
            tree_refreshed: None,
            state: TicketState::Active,
            state_by: None,
            close: CloseProgress::default(),
            restarts: Vec::new(),
            restart: None,
            entered: Vec::new(),
            holds: Vec::new(),
            services: Vec::new(),
            created_ms: 0,
            updated_ms: 0,
        }
    }

    fn deploying() -> Pipeline {
        Pipeline::parse(DEPLOYING).unwrap()
    }

    #[test]
    fn a_deploy_open_before_or_while_its_command_runs_is_mid_apply() {
        let want = Some(("t1".to_owned(), "deploy".to_owned()));
        let waiting = ticket("deploy", AttemptState::Starting, None);
        assert_eq!(mid_apply(&[waiting], |_| Ok(deploying())).unwrap(), want);
        let running = ticket("deploy", AttemptState::Running, Some(gate(None)));
        assert_eq!(mid_apply(&[running], |_| Ok(deploying())).unwrap(), want);
    }

    #[test]
    fn an_agent_stage_and_its_check_and_a_finished_deploy_are_not() {
        let agent = ticket("implement", AttemptState::Running, None);
        let checking = ticket("implement", AttemptState::Running, Some(gate(None)));
        let done = ticket("deploy", AttemptState::Complete, Some(gate(Some(0))));
        assert_eq!(
            mid_apply(&[agent, checking, done], |_| Ok(deploying())).unwrap(),
            None
        );
    }

    #[test]
    fn an_open_attempt_whose_pipeline_copy_will_not_read_refuses() {
        let open = ticket("deploy", AttemptState::Running, Some(gate(None)));
        let e = mid_apply(&[open], pipeline_of).unwrap_err();
        assert!(format!("{e:#}").contains("ticket t1"), "{e:#}");
        let done = ticket("deploy", AttemptState::Complete, Some(gate(Some(0))));
        assert_eq!(mid_apply(&[done], pipeline_of).unwrap(), None);
    }

    /// A data directory and an app on a socket beside it that answers
    /// one `verb` request, then runs `after` on the data directory.
    fn app(
        verb: RunnerVerb,
        after: impl FnOnce(&DataDir) + Send + 'static,
    ) -> (
        tempfile::TempDir,
        DataDir,
        PathBuf,
        std::thread::JoinHandle<()>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path().join("d"));
        std::fs::create_dir_all(&data.root).unwrap();
        let socket = dir.path().join("s.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let app_data = DataDir::new(data.root.clone());
        let app = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let req = Request::parse(line.trim_end()).unwrap();
            assert_eq!(req.body, Body::DispatchRunner { action: verb });
            let mut w = stream;
            w.write_all(Reply::Persisted { made: vec![] }.to_line().as_bytes())
                .unwrap();
            after(&app_data);
        });
        (dir, data, socket, app)
    }

    fn write_runner(data: &DataDir, file: &RunnerFile) {
        std::fs::write(
            data.root.join(RUNNER_FILE),
            serde_json::to_vec(file).unwrap(),
        )
        .unwrap();
    }

    const ASKED_MS: u64 = 1_000;

    #[test]
    fn the_lock_is_let_go_before_the_wait_for_the_new_runner() {
        let (_dir, data, socket, app) = app(RunnerVerb::Restart, |data| {
            // The new runner recovers under the writer lock before it
            // writes `runner.json`.
            data.with_lock(|| {
                let file = RunnerFile {
                    pid: std::process::id(),
                    started_ms: ASKED_MS + 1_000,
                    ..RunnerFile::default()
                };
                write_runner(data, &file);
                Ok(())
            })
            .unwrap();
        });
        let line = run(
            &data,
            &socket,
            RunnerVerb::Restart,
            Duration::from_secs(5),
            || ASKED_MS,
        )
        .unwrap();
        assert_eq!(line, format!("runner up: pid {}", std::process::id()));
        app.join().unwrap();
    }

    #[test]
    fn a_start_while_the_old_runner_dies_waits_for_a_pass_after_it() {
        // The killed runner is still alive but passes no more.
        let mut dying = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let (_dir, data, socket, app) = app(RunnerVerb::Start, |_| {});
        let old = RunnerFile {
            pid: dying.id(),
            started_ms: 10,
            last_pass_ms: ASKED_MS - 1,
            ..RunnerFile::default()
        };
        write_runner(&data, &old);
        let e = run(
            &data,
            &socket,
            RunnerVerb::Start,
            Duration::from_secs(1),
            || ASKED_MS,
        )
        .unwrap_err();
        dying.kill().unwrap();
        dying.wait().unwrap();
        app.join().unwrap();
        assert!(
            e.to_string().starts_with("the runner did not start"),
            "{e:#}"
        );
    }

    #[test]
    fn a_start_while_a_runner_recovers_is_done_on_its_first_pass() {
        let mut gone = std::process::Command::new("true").spawn().unwrap();
        gone.wait().unwrap();
        let (_dir, data, socket, app) = app(RunnerVerb::Start, |data| {
            // Launched before the request, so started before it too.
            let file = RunnerFile {
                pid: std::process::id(),
                started_ms: ASKED_MS - 500,
                last_pass_ms: ASKED_MS + 2_000,
                ..RunnerFile::default()
            };
            write_runner(data, &file);
        });
        let last = RunnerFile {
            pid: gone.id(),
            started_ms: 10,
            last_pass_ms: 20,
            ..RunnerFile::default()
        };
        write_runner(&data, &last);
        let line = run(
            &data,
            &socket,
            RunnerVerb::Start,
            Duration::from_secs(5),
            || ASKED_MS,
        )
        .unwrap();
        assert_eq!(line, format!("runner up: pid {}", std::process::id()));
        app.join().unwrap();
    }
}
