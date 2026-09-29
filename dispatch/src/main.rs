//! `dispatch`: take a ticket, run the scheduler, answer decisions, look.

use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail};
use dispatch::epoch_ms;
use dispatch::git::GitCli;
use dispatch::github::{Gh, Issues};
use dispatch::pipeline::{Pipeline, Source};
use dispatch::port::SocketPort;
use dispatch::scheduler::Runner;
use dispatch::store::DataDir;
use dispatch::ticket::{DecisionState, TicketState};

const USAGE: &str = "usage:
  dispatch take <project> <issue-number>   make a ticket from an issue and queue it
  dispatch run [--once]                    drive every ticket (once, or until stopped)
  dispatch decide <ticket> <decision> <answer> [--note <text>]
  dispatch decisions                       what waits on you
  dispatch status                          every ticket, its stage and state
  dispatch queue <project> [<ticket>...]   show, or reorder, a project's queue

Data: $DISPATCH_DATA_DIR (default ~/Library/Application Support/Dispatch).
Switchboard: $SWITCHBOARD_DATA_DIR/control.sock (default Switchboard's).";

fn now_ms() -> u64 {
    epoch_ms(SystemTime::now())
}

fn runner() -> Result<Runner> {
    Ok(Runner::new(
        DataDir::from_env()?,
        Box::new(SocketPort::from_env()?),
        Box::new(GitCli),
    ))
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("dispatch=info"))
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
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
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

fn take(project: &str, issue: &str) -> Result<()> {
    let mut runner = runner()?;
    let path = runner.data.pipeline(project);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("no pipeline for {project} at {}", path.display()))?;
    let pipeline = Pipeline::parse(&text)?;
    let Source::Github { repo, .. } = &pipeline.source else {
        bail!("only a GitHub source is taken from the command line in this slice");
    };
    let number: u64 = issue
        .trim_start_matches('#')
        .parse()
        .context("an issue number")?;
    let source = Gh.fetch(repo, number, now_ms())?;
    let ticket = runner.take(project, &text, source, now_ms())?;
    println!("{} #{} {}", ticket.id, number, ticket.source.title);
    Ok(())
}

fn run(once: bool) -> Result<()> {
    let mut runner = runner()?;
    // One runner per data directory; a second exits here.
    let _owner = runner.data.claim_runner()?;
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
    let runner = Runner::new(DataDir::from_env()?, Box::new(NoPort), Box::new(GitCli));
    let d = runner.decide(ticket, decision, answer, note, now_ms())?;
    println!(
        "{ticket} {}: {answer} (the runner acts on it on its next pass)",
        d.id
    );
    Ok(())
}

fn decisions() -> Result<()> {
    let runner = Runner::new(DataDir::from_env()?, Box::new(NoPort), Box::new(GitCli));
    let mut any = false;
    for t in runner.tickets()? {
        for d in t.pending_decisions() {
            any = true;
            println!(
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
        println!("nothing waits on you");
    }
    Ok(())
}

fn status() -> Result<()> {
    let runner = Runner::new(DataDir::from_env()?, Box::new(NoPort), Box::new(GitCli));
    let tickets = runner.tickets()?;
    if tickets.is_empty() {
        println!("no tickets");
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
        println!(
            "{} {} #{} {} · stage {stage} · {standing}{last}{}",
            t.id,
            t.project,
            t.source.number.unwrap_or(0),
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
                println!("    {} answered {answer}, not yet acted on", d.id);
            }
        }
    }
    Ok(())
}

fn queue(project: &str, order: &[&str]) -> Result<()> {
    let mut runner = Runner::new(DataDir::from_env()?, Box::new(NoPort), Box::new(GitCli));
    let ps = if order.is_empty() {
        runner.load_project(project)?
    } else {
        runner.reorder_queue(project, order)?
    };
    for (i, id) in ps.queue.iter().enumerate() {
        let title = runner
            .load_ticket(id)
            .map(|t| format!("#{} {}", t.source.number.unwrap_or(0), t.source.title))
            .unwrap_or_default();
        println!("{} {id} {title}", i + 1);
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
