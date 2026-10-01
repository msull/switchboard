//! `dispatch`: take a ticket, run the scheduler, answer decisions, look.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use dispatch::epoch_ms;
use dispatch::git::GitCli;
use dispatch::github::Gh;
use dispatch::port::SocketPort;
use dispatch::scheduler::Runner;
use dispatch::serve::{Handler, Server, take_issue, take_pull_requests};
use dispatch::store::DataDir;
use dispatch::ticket::{DecisionState, TicketState};
use std::io::Write as _;

const USAGE: &str = "usage:
  dispatch take <project> <issue-number>   make a ticket from an issue and queue it
  dispatch take <project> pr <lane>/<n>... a ticket from someone's pull requests, one per lane,
                                           on <project>.pr.toml; the lane may be left off with one lane
  dispatch run [--once]                    drive every ticket (once, or until stopped)
  dispatch decide <ticket> <decision> <answer> [--note <text>]
  dispatch decisions                       what waits on you
  dispatch status                          every ticket, its stage and state
  dispatch queue <project> [<ticket>...]   show, or reorder, a project's queue
  dispatch resume <ticket>                 a parked ticket back to active
  dispatch worktrees [<path>] [--migrate]  where tickets' trees go (default ~/.dispatch/worktrees);
                                           with a path, set it; --migrate moves idle tickets' trees there

Data: $DISPATCH_DATA_DIR (default ~/Library/Application Support/Dispatch).
Switchboard: $SWITCHBOARD_DATA_DIR/control.sock (default Switchboard's).
While `run` is up it serves the same commands on <data>/dispatch.sock.";

/// Print a line, and end quietly when the reader has gone: a pipe into
/// `head` or `grep -m` closes stdout early, which is not an error.
macro_rules! say {
    ($($arg:tt)*) => {{
        let mut out = std::io::stdout().lock();
        if let Err(e) = writeln!(out, $($arg)*) {
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                std::process::exit(0);
            }
            return Err(e.into());
        }
    }};
}

fn now_ms() -> u64 {
    epoch_ms(SystemTime::now())
}

fn runner() -> Result<Runner> {
    Ok(Runner::new(
        DataDir::from_env()?,
        Box::new(SocketPort::from_env()?),
        Box::new(GitCli::default()),
    ))
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("dispatch=info"))
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["take", project, "pr", specs @ ..] => take_prs(project, specs),
        ["take", project, issue] => take(project, issue),
        ["run"] => run(false),
        ["run", "--once"] => run(true),
        ["decide", ticket, decision, answer] => decide(ticket, decision, answer, None),
        ["decide", ticket, decision, answer, "--note", note] => {
            decide(ticket, decision, answer, Some(note))
        }
        ["decisions"] => decisions(),
        ["status"] => status(),
        ["queue", project, rest @ ..] => queue(project, rest),
        ["resume", ticket] => resume(ticket),
        ["worktrees", rest @ ..] => worktrees(rest),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

fn take_prs(project: &str, specs: &[&str]) -> Result<()> {
    let mut runner = runner()?;
    let ticket = take_pull_requests(&mut runner, project, specs, now_ms())?;
    say!(
        "{} {} ({})",
        ticket.id,
        ticket.source.title,
        ticket
            .source
            .pull_requests
            .iter()
            .map(|pr| format!("{} PR #{}", pr.lane, pr.number))
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(())
}

fn take(project: &str, issue: &str) -> Result<()> {
    let mut runner = runner()?;
    let ticket = take_issue(&mut runner, &Gh, project, issue, now_ms())?;
    say!(
        "{} {} {}",
        ticket.id,
        ticket.source.label(),
        ticket.source.title
    );
    Ok(())
}

fn run(once: bool) -> Result<()> {
    let mut runner = runner()?;
    // One runner per data directory; a second exits here.
    let _owner = runner.data.claim_runner()?;
    // The port answers on its own runner over the same records; the
    // writer lock keeps the two apart.
    let handler = Handler {
        runner: self::runner()?,
        issues: Box::new(Gh),
    };
    let _port = Server::bind(&runner.data, handler)?;
    runner.recover(now_ms())?;
    loop {
        runner.step_all(now_ms())?;
        if once {
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn decide(ticket: &str, decision: &str, answer: &str, note: Option<&str>) -> Result<()> {
    let runner = Runner::new(
        DataDir::from_env()?,
        Box::new(NoPort),
        Box::new(GitCli::default()),
    );
    let d = runner.decide(ticket, decision, answer, note, now_ms())?;
    say!(
        "{ticket} {}: {answer} (the runner acts on it on its next pass)",
        d.id
    );
    Ok(())
}

fn worktrees(args: &[&str]) -> Result<()> {
    let migrate = args.contains(&"--migrate");
    let path: Vec<&str> = args.iter().copied().filter(|a| *a != "--migrate").collect();
    let path = match path[..] {
        [] => None,
        [p] => Some(PathBuf::from(p)),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };
    let mut runner = self::runner()?;
    let view = runner.set_worktrees(path, migrate, now_ms())?;
    say!("worktrees: {}", view.root.display());
    for id in &view.moved {
        say!("  moved {id}");
    }
    for (id, why) in &view.skipped {
        say!("  left {id}: {why}");
    }
    Ok(())
}

fn decisions() -> Result<()> {
    let runner = Runner::new(
        DataDir::from_env()?,
        Box::new(NoPort),
        Box::new(GitCli::default()),
    );
    let mut any = false;
    for t in runner.tickets()? {
        for d in t.pending_decisions() {
            any = true;
            say!(
                "{} {} [{}] {}\n    options: {}{}\n    dispatch decide {} {} <answer>",
                t.id,
                d.id,
                d.stage,
                d.question,
                d.options.join(" | "),
                d.recommendation
                    .as_ref()
                    .map_or(String::new(), |r| format!(" (suggested: {r})")),
                t.id,
                d.id
            );
        }
    }
    if !any {
        say!("nothing waits on you");
    }
    Ok(())
}

fn status() -> Result<()> {
    let runner = Runner::new(
        DataDir::from_env()?,
        Box::new(NoPort),
        Box::new(GitCli::default()),
    );
    let tickets = runner.tickets()?;
    if tickets.is_empty() {
        say!("no tickets");
    }
    for p in dispatch::serve::status(&runner)?.projects {
        say!(
            "{}: {} of {} slots in use, {} of {} decisions waiting{}",
            p.name,
            p.running,
            p.slots,
            p.pending,
            p.waiting_on_me,
            p.held()
                .map_or(String::new(), |why| format!(" · nothing new starts: {why}"))
        );
    }
    for t in tickets {
        let p = runner.pipeline_of(&t).ok();
        let stage = p
            .as_ref()
            .and_then(|p| p.stages.get(t.stage))
            .map_or("done".to_owned(), |s| s.name.clone());
        let standing = match &t.state {
            TicketState::Active => "active".to_owned(),
            TicketState::Parking { reason } => format!("parking: {reason}"),
            TicketState::Parked { reason } => format!("parked: {reason}"),
            TicketState::Closed { reason } => format!("closed: {reason}"),
        };
        let last = t.attempts.last().map_or(String::new(), |a| {
            format!(
                " · {}/{} #{} {}",
                a.stage,
                a.context,
                a.n,
                match &a.state {
                    dispatch::ticket::AttemptState::Starting => "starting".to_owned(),
                    dispatch::ticket::AttemptState::Running => "running".to_owned(),
                    dispatch::ticket::AttemptState::Complete => "complete".to_owned(),
                    dispatch::ticket::AttemptState::Failed { reason } =>
                        format!("failed: {reason}"),
                    dispatch::ticket::AttemptState::Cancelled { reason } =>
                        format!("cancelled: {reason}"),
                }
            )
        });
        let pending = t.pending_decisions().len();
        say!(
            "{} {} {} {} · stage {stage} · {standing}{last}{}",
            t.id,
            t.project,
            t.source.label(),
            t.source.title,
            if pending > 0 {
                format!(" · {pending} decision(s) pending")
            } else {
                String::new()
            }
        );
        for d in &t.decisions {
            if let DecisionState::Answered { answer, acted, .. } = &d.state
                && !acted
            {
                say!("    {} answered {answer}, not yet acted on", d.id);
            }
        }
    }
    Ok(())
}

fn resume(ticket: &str) -> Result<()> {
    let runner = Runner::new(
        DataDir::from_env()?,
        Box::new(NoPort),
        Box::new(GitCli::default()),
    );
    let t = runner.resume(ticket, now_ms())?;
    say!(
        "{} {} {} active again",
        t.id,
        t.source.label(),
        t.source.title
    );
    Ok(())
}

fn queue(project: &str, order: &[&str]) -> Result<()> {
    let mut runner = Runner::new(
        DataDir::from_env()?,
        Box::new(NoPort),
        Box::new(GitCli::default()),
    );
    let ps = if order.is_empty() {
        runner.load_project(project)?
    } else {
        runner.reorder_queue(project, order)?
    };
    for (i, id) in ps.queue.iter().enumerate() {
        let title = runner
            .load_ticket(id)
            .map(|t| format!("{} {}", t.source.label(), t.source.title))
            .unwrap_or_default();
        say!("{} {id} {title}", i + 1);
    }
    Ok(())
}

/// The commands that never talk to Switchboard.
struct NoPort;

impl dispatch::port::Port for NoPort {
    fn call(
        &mut self,
        _: &switchboard_control::Request,
    ) -> std::io::Result<switchboard_control::Reply> {
        Err(std::io::Error::other(
            "this command does not talk to Switchboard",
        ))
    }
}
