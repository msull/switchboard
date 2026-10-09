//! `dispatch`: take a ticket, run the scheduler, answer decisions, look.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use dispatch::epoch_ms;
use dispatch::events::{self, Burst, Event, For, Kind, base_name, clock, short};
use dispatch::git::{GitCli, yyyymmdd};
use dispatch::github::Gh;
use dispatch::health;
use dispatch::pipeline::{DeployWait, Lane};
use dispatch::port::SocketPort;
use dispatch::report::{self, TicketReport};
use dispatch::scheduler::{BY_SUPERVISOR, Runner, attempt_label, kept_branches};
use dispatch::serve::{Handler, Server, take_issue, take_pull_requests};
use dispatch::serve::{ticket_paths, ticket_view};
use dispatch::store::{DataDir, read_ticket};
use dispatch::subscribe;
use dispatch::supervisor::{self, Actor};
use dispatch::ticket::{DecisionState, REFUSALS_KEPT, Refusal, ServiceState, Ticket, TicketState};
use dispatch::{USAGE, UsageError};
use std::io::Write as _;
use switchboard_control::RunnerVerb;

/// `EX_USAGE`: the command line was wrong, or the command is one the
/// ticket's state never allows (`park` on a closing or closed ticket, a
/// `UsageError`). `wait` gives 2 and 3 their own meanings, so a usage
/// error is never mistaken for them.
const EXIT_USAGE: i32 = 64;

/// Exit codes of `wait` past its match.
const EXIT_TIMED_OUT: i32 = 2;
const EXIT_ENDED: i32 = 3;

fn usage() -> ! {
    eprintln!("{USAGE}");
    std::process::exit(EXIT_USAGE);
}

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

/// Who runs this command, worked out once in `command`: finding it reads
/// every project's record.
static ACTOR: OnceLock<Actor> = OnceLock::new();

/// This command's actor; the owner until `command` has set it.
fn actor() -> &'static Actor {
    static OWNER: Actor = Actor::Owner;
    ACTOR.get().unwrap_or(&OWNER)
}

fn runner() -> Result<Runner> {
    Ok(with_actor(Runner::new(
        DataDir::from_env()?,
        Box::new(SocketPort::from_env()?),
        Box::new(GitCli::default()),
    )))
}

/// A runner for the commands that never talk to Switchboard.
fn offline_runner() -> Result<Runner> {
    Ok(with_actor(Runner::new(
        DataDir::from_env()?,
        Box::new(NoPort),
        Box::new(GitCli::default()),
    )))
}

/// The runner stamped with who runs this command (a supervisor session
/// or the owner), where `switchboard-env` is, and the Switchboard record
/// and launch token of the pane it runs in.
fn with_actor(mut runner: Runner) -> Runner {
    runner.actor = actor().by();
    runner.env_bin = dispatch::scheduler::env_bin_beside_exe();
    runner.credentials = dispatch::scheduler::RunnerCredentials::from_env();
    runner
}

/// The command a printed `decide` line starts with. A supervisor's only
/// permission matches the full path, so its lines carry that path; the
/// owner types `dispatch`.
fn decide_command() -> String {
    match actor() {
        Actor::Owner => "dispatch decide".to_owned(),
        Actor::Supervisor(_) => std::env::current_exe().map_or_else(
            |_| "dispatch decide".to_owned(),
            |exe| format!("{} decide", exe.display()),
        ),
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("dispatch=info"))
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = command(&args);
    if let Err(e) = &result
        && let Some(usage) = e.downcast_ref::<UsageError>()
    {
        eprintln!("{usage}");
        std::process::exit(EXIT_USAGE);
    }
    result
}

fn command(args: &[&str]) -> Result<()> {
    // A supervisor session may run only some commands, on its own
    // project; the owner runs anything.
    let data = DataDir::from_env()?;
    let actor = supervisor::actor(&data)?;
    if let Err(e) = supervisor::permit(ACTOR.get_or_init(|| actor), args, &data) {
        if let Some(refused) = e.downcast_ref::<supervisor::CapabilityRefused>()
            && let Err(save) = save_refusal(refused)
        {
            // The refusal is the answer; a record that will not take it
            // is only logged.
            log::warn!("{}: refusal not saved: {save:#}", refused.project);
        }
        return Err(e);
    }
    match args {
        ["take", project, "pr", specs @ ..] => take_prs(project, specs),
        ["take", project, issue] => take(project, issue),
        ["run"] => run(false),
        ["run", "--once"] => run(true),
        ["runner", "stop"] => runner_verb(RunnerVerb::Stop),
        ["runner", "start"] => runner_verb(RunnerVerb::Start),
        ["runner", "restart"] => runner_verb(RunnerVerb::Restart),
        ["decide", ticket, decision, answer] => decide(ticket, decision, answer, None),
        ["decide", ticket, decision, answer, "--note", note] => {
            decide(ticket, decision, answer, Some(note))
        }
        ["decide", ticket, decision, answer, "--file", path] => {
            let note = offline_runner()?.note_from_file(ticket, Path::new(path))?;
            decide(ticket, decision, answer, Some(&note))
        }
        ["decisions"] => decisions(),
        ["status"] => status(),
        ["queue", project, rest @ ..] => queue(project, rest),
        ["park", ticket] => park(ticket, None),
        ["park", ticket, "--reason", reason] => park(ticket, Some(reason)),
        ["resume", ticket] => resume(ticket, true),
        ["resume", ticket, "--no-rerun"] => resume(ticket, false),
        ["close", ticket] => close(ticket, None, false),
        ["close", ticket, "--reason", reason] => close(ticket, Some(reason), false),
        ["close", ticket, "--drop-evidence"] => close(ticket, None, true),
        ["close", ticket, "--reason", reason, "--drop-evidence"]
        | ["close", ticket, "--drop-evidence", "--reason", reason] => {
            close(ticket, Some(reason), true)
        }
        ["evidence", ticket, stage @ ..] if stage.len() <= 1 => {
            evidence(ticket, stage.first().copied())
        }
        ["restart", ticket, stage @ ..] if stage.len() <= 1 => {
            restart(ticket, stage.first().copied(), None)
        }
        [
            "restart",
            ticket,
            stage @ ..,
            flag @ ("--note" | "--file"),
            value,
        ] if stage.len() <= 1 => {
            let note = note_arg(ticket, flag, value)?;
            restart(ticket, stage.first().copied(), Some(&note))
        }
        ["worktrees", rest @ ..] => worktrees(rest),
        ["events", rest @ ..] => events(rest),
        ["wait", rest @ ..] => wait(rest),
        ["show", rest @ ..] => show(rest),
        ["report", rest @ ..] => report(rest),
        ["tail", rest @ ..] => tail(rest),
        ["health", rest @ ..] => health(rest),
        ["brief", rest @ ..] => brief(rest),
        ["subscribe", rest @ ..] => subscribe(rest),
        ["unsubscribe", ticket] => unsubscribe(ticket),
        ["subscriptions", project] => subscriptions(project),
        ["supervisor", rest @ ..] => supervise(rest),
        _ => usage(),
    }
}

/// A command refused for want of a capability, kept on the project's
/// record so the owner sees what the supervisor tried, and logged.
fn save_refusal(refused: &supervisor::CapabilityRefused) -> Result<()> {
    log::warn!(
        "{}: the supervisor was refused `dispatch {}`",
        refused.project,
        refused.command()
    );
    let mut runner = offline_runner()?;
    runner.transaction(|r| {
        let mut ps = r.load_project(&refused.project)?;
        let kept = &mut ps.supervisor.refusals;
        kept.push(Refusal {
            by: BY_SUPERVISOR.to_owned(),
            answer: refused.command(),
            at_ms: now_ms(),
        });
        let over = kept.len().saturating_sub(REFUSALS_KEPT);
        kept.drain(..over);
        r.save_project(&ps)
    })
}

/// `dispatch runner stop|start|restart`: asked of the app, then waited
/// for in `runner.json`.
fn runner_verb(verb: RunnerVerb) -> Result<()> {
    let data = DataDir::from_env()?;
    let socket = SocketPort::from_env()?.path().to_path_buf();
    let line = dispatch::runner_cmd::run(&data, &socket, verb, Duration::from_secs(60), now_ms)?;
    say!("{line}");
    Ok(())
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
    let pid = std::process::id();
    let started_ms = now_ms();
    let pass = Instant::now();
    runner.recover(now_ms())?;
    write_health(&runner, pid, started_ms, pass);
    loop {
        let pass = Instant::now();
        runner.step_all(now_ms())?;
        write_health(&runner, pid, started_ms, pass);
        if once {
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// `runner.json` after a pass that began at `pass`; a failure to write
/// it is logged, never the end of the runner.
fn write_health(runner: &Runner, pid: u32, started_ms: u64, pass: Instant) {
    let took = u64::try_from(pass.elapsed().as_millis()).unwrap_or(u64::MAX);
    if let Err(e) = runner
        .health
        .borrow_mut()
        .write(&runner.data, pid, started_ms, now_ms(), took)
    {
        log::warn!("runner.json: {e:#}");
    }
}

fn decide(ticket: &str, decision: &str, answer: &str, note: Option<&str>) -> Result<()> {
    let runner = offline_runner()?;
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
        _ => usage(),
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
    let runner = offline_runner()?;
    let mut any = false;
    for t in runner.tickets()? {
        for d in t.waiting_on_you() {
            any = true;
            say!(
                "{} {} [{}] {}\n    options: {}{}\n    {} {} {} <answer>",
                t.id,
                d.id,
                d.stage,
                d.question,
                d.options.join(" | "),
                d.recommendation
                    .as_ref()
                    .map_or(String::new(), |r| format!(" (suggested: {r})")),
                decide_command(),
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
    let runner = offline_runner()?;
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
    for t in &tickets {
        let p = runner.pipeline_of(t).ok();
        let stage = p
            .as_ref()
            .and_then(|p| p.stages.get(t.stage))
            .map_or("done".to_owned(), |s| s.name.clone());
        let standing = t.state.label();
        let last = t.attempts.last().map_or(String::new(), |a| {
            format!(
                " · {}/{} #{} {}",
                a.stage,
                a.context,
                a.n,
                match dispatch::serve::attempt_state(&a.state) {
                    (word, None) => word.to_owned(),
                    (word, Some(reason)) => format!("{word}: {reason}"),
                }
            )
        });
        let pending = t.waiting_on_you().len();
        let holding = if let Some(why) = p
            .as_ref()
            .and_then(|p| dispatch::services::waiting_for(t, p, &tickets))
        {
            format!(" · waiting for {why}")
        } else if t.holds.is_empty() {
            String::new()
        } else {
            let held: Vec<&str> = t.holds.iter().map(|h| h.resource.as_str()).collect();
            format!(" · holds {}", held.join(", "))
        };
        say!(
            "{} {} {} {} · stage {stage} · {standing}{last}{holding}{}",
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
        for s in t
            .services
            .iter()
            .filter(|s| s.state != ServiceState::Stopped)
        {
            say!(
                "    {} {} {}",
                s.lane,
                s.url.as_deref().unwrap_or("(no port yet)"),
                s.state.label()
            );
        }
        for d in &t.decisions {
            if let DecisionState::Answered {
                answer, acted, by, ..
            } = &d.state
                && !acted
            {
                say!("    {} answered {answer} by {by}, not yet acted on", d.id);
            }
        }
    }
    Ok(())
}

/// Write the parking intent; the runner, whose children the checks
/// are, does the stopping on its next pass.
fn park(ticket: &str, reason: Option<&str>) -> Result<()> {
    let runner = offline_runner()?;
    let t = runner.request_park(ticket, reason, now_ms())?;
    let reason = match &t.state {
        TicketState::Parking { reason } => reason.as_str(),
        _ => "",
    };
    say!(
        "{} {} {} parking: {reason} (the runner finishes it on its next pass)",
        t.id,
        t.source.label(),
        t.source.title
    );
    Ok(())
}

fn resume(ticket: &str, rerun: bool) -> Result<()> {
    let runner = offline_runner()?;
    let now = now_ms();
    let resumed = if rerun {
        runner.resume(ticket, now)?
    } else {
        runner.resume_asking(ticket, now)?
    };
    let t = &resumed.ticket;
    say!(
        "{} {} {} active again",
        t.id,
        t.source.label(),
        t.source.title
    );
    for a in &resumed.reruns {
        say!("  rerunning {}", attempt_label(a));
    }
    if let Some(why) = &resumed.no_reruns {
        say!("  nothing reruns: {why}");
    }
    Ok(())
}

/// The note a trailing `--note <text>` or `--file <path>` gives: the
/// text itself, or the file read under the ticket's rules for notes.
fn note_arg(ticket: &str, flag: &str, value: &str) -> Result<String> {
    if flag == "--file" {
        offline_runner()?.note_from_file(ticket, Path::new(value))
    } else {
        Ok(value.to_owned())
    }
}

/// Restart a ticket through Switchboard, as a park would run: its
/// processes are read back as gone before anything moves, so this takes
/// the real port.
fn restart(ticket: &str, stage: Option<&str>, note: Option<&str>) -> Result<()> {
    let mut runner = runner()?;
    let t = runner.restart(ticket, stage, note, now_ms())?;
    let standing = if matches!(t.state, TicketState::Parking { .. }) {
        "restarting (the runner finishes it on its next pass)".to_owned()
    } else {
        t.state.label()
    };
    say!(
        "{} {} {} {standing}",
        t.id,
        t.source.label(),
        t.source.title
    );
    if t.restart.is_some() {
        return Ok(());
    }
    let Some(r) = t.restarts.last() else {
        return Ok(());
    };
    say!("  at {} from {}, under {}", r.to, r.from, r.after.display());
    if let Some(line) = r
        .note
        .as_deref()
        .and_then(|n| n.lines().find(|l| !l.trim().is_empty()))
    {
        say!("  note: {}", line.trim());
    }
    for (stage, n) in &r.discarded {
        say!("  discarded {stage}/{n}");
    }
    for h in &r.reset {
        say!("  reset {h}");
    }
    for lane in &r.setup_again {
        say!("  setup runs again in lane {lane}");
    }
    if !r.reset.is_empty() {
        for lane in t.lanes.iter().filter(|l| l.pushed.is_some()) {
            say!(
                "  lane {} was pushed: its PR is force-updated at the next push",
                lane.name
            );
        }
    }
    Ok(())
}

/// Close a ticket through Switchboard, as the runner would: its
/// processes are read back and its session unmarked, so this takes the
/// real port, not `NoPort`.
fn close(ticket: &str, reason: Option<&str>, drop_evidence: bool) -> Result<()> {
    let mut runner = runner()?;
    let t = runner.close_by_hand(ticket, reason, drop_evidence, now_ms())?;
    let mut standing = t.state.label();
    if matches!(t.state, TicketState::Closing { .. }) {
        standing.push_str(" (the runner finishes it on its next pass)");
    }
    say!(
        "{} {} {} {standing}",
        t.id,
        t.source.label(),
        t.source.title
    );
    for lane in t.lanes.iter().filter(|l| l.removed) {
        if t.tree.as_ref() != Some(&lane.worktree) {
            say!("  removed lane {}: {}", lane.name, lane.worktree.display());
        }
    }
    if let Some(tree) = &t.tree
        && t.close.tree_removed
    {
        say!("  removed {}", tree.display());
    }
    if let Some(why) = &t.close.trees_kept {
        say!("  kept: {why}");
    }
    if drop_evidence {
        if t.close.drop_evidence && matches!(t.state, TicketState::Closing { .. }) {
            say!("  evidence: dropped as the close finishes");
        } else {
            // Counted over every removed directory, by this close or an
            // earlier sweep, so the line states what is gone, not who
            // removed it.
            let gone: usize = t
                .attempts
                .iter()
                .filter_map(|a| a.evidence.as_ref())
                .filter(|e| e.swept.is_some())
                .map(|e| e.files.len())
                .sum();
            say!("  evidence removed: {gone} files");
        }
    }
    // The close has happened; a pipeline that no longer reads only
    // leaves the branches unlisted.
    if let Ok(p) = runner.pipeline_of(&t) {
        for (branch, clone) in kept_branches(&t, &p, &runner.data) {
            say!("  branch kept: {branch} in {}", clone.display());
        }
    }
    Ok(())
}

fn queue(project: &str, order: &[&str]) -> Result<()> {
    let mut runner = offline_runner()?;
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
    for id in &ps.closing {
        let line = runner.load_ticket(id).map_or_else(
            |_| String::new(),
            |t| {
                format!(
                    "{} {} {}",
                    t.source.label(),
                    t.source.title,
                    t.state.label()
                )
            },
        );
        say!("- {id} {line}");
    }
    Ok(())
}

/// A command's flags: the values and switches it knows, and the words
/// left over. An unknown flag or one without its value is a usage
/// error.
struct Flags<'a> {
    values: Vec<(&'a str, &'a str)>,
    switches: Vec<&'a str>,
    rest: Vec<&'a str>,
}

impl<'a> Flags<'a> {
    fn parse(args: &[&'a str], values: &[&str], switches: &[&str]) -> Self {
        let mut out = Flags {
            values: Vec::new(),
            switches: Vec::new(),
            rest: Vec::new(),
        };
        let mut words = args.iter();
        while let Some(&word) = words.next() {
            if values.contains(&word) {
                let Some(&value) = words.next() else {
                    eprintln!("{word} takes a value");
                    usage();
                };
                out.values.push((word, value));
            } else if switches.contains(&word) {
                out.switches.push(word);
            } else if word.starts_with("--") {
                eprintln!("unknown flag {word}");
                usage();
            } else {
                out.rest.push(word);
            }
        }
        out
    }

    /// The last value given for `flag`.
    fn value(&self, flag: &str) -> Option<&'a str> {
        self.values
            .iter()
            .rev()
            .find(|(f, _)| *f == flag)
            .map(|(_, v)| *v)
    }

    /// Every value given for a flag that may repeat.
    fn all(&self, flag: &str) -> Vec<&'a str> {
        self.values
            .iter()
            .filter(|(f, _)| *f == flag)
            .map(|(_, v)| *v)
            .collect()
    }

    fn on(&self, switch: &str) -> bool {
        self.switches.contains(&switch)
    }

    /// `flag`'s value as a number, or a usage error.
    fn number(&self, flag: &str) -> Option<u64> {
        self.value(flag).map(|v| {
            v.parse().unwrap_or_else(|_| {
                eprintln!("{flag} takes a number, not {v:?}");
                usage()
            })
        })
    }

    /// The one word a command takes, or a usage error.
    fn one(&self) -> &'a str {
        match self.rest[..] {
            [word] => word,
            _ => usage(),
        }
    }
}

/// Why an open attempt's agent is not yet failed for its missing
/// artifact: `  root held: subagent general-purpose, wakeup at
/// 2026-10-08 15:40 (until 2026-10-08 16:10)`.
fn held_line(context: &str, h: &dispatch_control::HeldView) -> String {
    let mut pending = h.pending.clone();
    // The earliest one-shot wakeup is one of the plain `wakeup` entries.
    if let (Some(at), Some(first)) = (
        h.wakeup_at_ms,
        pending.iter_mut().find(|p| p.as_str() == "wakeup"),
    ) {
        *first = format!("wakeup at {}", local_time(at));
    }
    format!(
        "  {context} held: {} (until {})",
        pending.join(", "),
        local_time(h.until_ms)
    )
}

/// `YYYY-MM-DD hh:mm` of `ms` since the epoch, in the local zone, as
/// `clock` reads it; for a moment that may be days back.
fn local_time(ms: u64) -> String {
    use chrono::{DateTime, Local, TimeZone as _};
    let secs = i64::try_from(ms / 1000).unwrap_or(i64::MAX);
    DateTime::from_timestamp(secs, 0).map_or_else(
        || format!("{} {}", date_of(ms), clock(ms)),
        |utc| {
            Local
                .from_utc_datetime(&utc.naive_utc())
                .format("%Y-%m-%d %H:%M")
                .to_string()
        },
    )
}

/// A duration for a person: `2h05m`, `4m10s`, `12s`.
fn span(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}h{:02}m", s / 3600, (s / 60) % 60)
    } else if s >= 60 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}

fn events(args: &[&str]) -> Result<()> {
    let f = Flags::parse(
        args,
        &["--since", "--ticket", "--project", "--timeout"],
        &["--follow", "--json"],
    );
    if !f.rest.is_empty() || (f.value("--timeout").is_some() && !f.on("--follow")) {
        usage();
    }
    let deadline = f.number("--timeout").map(|s| now_ms() + s * 1000);
    let mut printed = false;
    // A follow with no cursor starts at the tail: the reader wants what
    // happens next, not the whole history again. A plain listing or an
    // explicit --since replays from 0 or from the cursor.
    let since = match f.number("--since") {
        Some(n) => n,
        None if f.on("--follow") => events::last_seq(&events::log_path(&DataDir::from_env()?))?,
        None => 0,
    };
    let tickets = f.all("--ticket");
    let project = f.value("--project");
    let json = f.on("--json");
    let keep = |e: &Event| {
        (tickets.is_empty() || tickets.contains(&e.ticket.as_str()))
            && project.is_none_or(|p| p == e.project)
    };
    let path = events::log_path(&DataDir::from_env()?);
    let all = events::read_since(&path, since)?;
    let gone = events::withdrawn(&all);
    let mut last = since;
    for e in &all {
        last = last.max(e.seq);
        if keep(e) && !gone.contains(&e.seq) {
            say!("{}", events::line(e, json));
            printed = true;
        }
    }
    if !f.on("--follow") {
        return Ok(());
    }
    // Followed, a withdrawn event may already be printed when its void
    // comes; the void's line says which seqs to drop.
    // With a timeout it ends as `wait` does: 0 as soon as a batch
    // printed something, so the watcher acts on it now, and 2 when
    // nothing came in the time.
    let mut follow = events::follow(&path, last);
    loop {
        for e in follow.next_batch()? {
            if keep(&e) {
                say!("{}", events::line(&e, json));
                printed = true;
            }
        }
        if deadline.is_some() && printed {
            return Ok(());
        }
        if deadline.is_some_and(|d| now_ms() >= d) {
            std::process::exit(EXIT_TIMED_OUT);
        }
        std::thread::sleep(Duration::from_millis(events::FOLLOW_POLL_MS));
    }
}

fn wait(args: &[&str]) -> Result<()> {
    let f = Flags::parse(args, &["--for", "--timeout", "--since"], &["--json"]);
    let ticket = f.one();
    let what = match f.value("--for") {
        None => For::Any,
        Some(word) => For::parse(word).unwrap_or_else(|| {
            eprintln!("--for takes decision, stage, pr, closed, move or any");
            usage()
        }),
    };
    let deadline = f.number("--timeout").map(|s| now_ms() + s * 1000);
    let json = f.on("--json");
    let data = DataDir::from_env()?;
    let since = f.number("--since");
    let burst = events::wait_burst(
        &data,
        ticket,
        what,
        since,
        deadline,
        &mut now_ms,
        &mut || {
            std::thread::sleep(Duration::from_millis(events::FOLLOW_POLL_MS));
        },
    )?;
    match burst {
        Burst::Lines(lines) => {
            for e in &lines {
                say!("{}", events::line(e, json));
            }
            // Only the last line can be a decision: one ends the burst.
            let Some(e) = lines.last() else {
                return Ok(());
            };
            if !json && e.kind == Kind::Decision {
                // Read again: a decision answered since its event (by a
                // resume, by Dispatch, from another terminal) has no
                // answer left to give.
                let t = read_ticket(&data.ticket_file(ticket))?;
                if let Some(hint) = decide_hint(e, &t) {
                    say!("{hint}");
                }
            }
            Ok(())
        }
        Burst::Ended { lines, end } => {
            for e in &lines {
                say!("{}", events::line(e, json));
            }
            say!("{}", events::line(&end, json));
            std::process::exit(EXIT_ENDED);
        }
        Burst::TimedOut => {
            if json {
                say!(r#"{{"timed_out":true}}"#);
            } else {
                say!("timed out");
            }
            std::process::exit(EXIT_TIMED_OUT);
        }
    }
}

/// `events::decide_hint` as this binary runs `decide`.
fn decide_hint(e: &Event, t: &Ticket) -> Option<String> {
    events::decide_hint(e, t, &decide_command())
}

/// `show`'s lines for a document a per-lane stage writes once per lane.
fn print_lane_files(name: &str, files: &[dispatch_control::LaneFile]) -> Result<()> {
    for f in files {
        let lane = f
            .lane
            .as_ref()
            .map_or_else(String::new, |l| format!(" ({l})"));
        say!("  {name}{lane}: {}", f.path.display());
        if f.reviewing {
            let round = f.round.map_or_else(String::new, |n| format!(", round {n}"));
            say!("    (reviewed copy{round}, review open)");
        }
    }
    Ok(())
}

fn show(args: &[&str]) -> Result<()> {
    let f = Flags::parse(args, &[], &["--json"]);
    let runner = offline_runner()?;
    let t = runner.load_ticket(f.one())?;
    let pipeline = runner.pipeline_of(&t).ok();
    let mut view = ticket_view(&t, pipeline.as_ref());
    view.paths = ticket_paths(&t, pipeline.as_ref());
    if f.on("--json") {
        say!("{}", serde_json::to_string_pretty(&view)?);
        return Ok(());
    }
    let current = view.stages.get(view.stage).map_or("done", String::as_str);
    say!(
        "{} {} {} {}",
        view.id,
        view.project,
        t.source.label(),
        view.title
    );
    say!("stage {current} ({}/{})", view.stage + 1, view.stages.len());
    if !view.stages.is_empty() {
        say!("stages: {}", stages_line(&view));
    }
    say!("state {}", t.state.label());
    for r in &view.restarts {
        say!(
            "restarted {} at {} from {} ({} → {})",
            date_of(r.at_ms),
            r.to,
            r.from,
            base_name(&r.before),
            base_name(&r.after)
        );
    }
    if !view.lanes.is_empty() {
        say!("lanes:");
    }
    for l in &view.lanes {
        let sha = |s: &Option<String>| s.as_deref().map_or("-".to_owned(), |s| short(s).to_owned());
        say!(
            "  {} {} {} base {} head {} pushed {}{}{}",
            l.name,
            l.branch,
            l.worktree.display(),
            sha(&l.base_sha),
            sha(&l.head),
            sha(&l.pushed_head),
            if l.removed { " (removed)" } else { "" },
            brought_up_clause(&view, l)
        );
    }
    if let Some(order) = merge_order_line(&view, pipeline.as_ref()) {
        say!("merge order: {order}");
    }
    for a in view.attempts.iter().filter(|a| a.ended_ms.is_none()) {
        if let (Some(waits), Some(since)) = (&a.waits, a.waits_since_ms) {
            say!(
                "  {} waits for {waits} since {}",
                a.context,
                local_time(since)
            );
        }
        for h in &a.held {
            say!("{}", held_line(&a.context, h));
        }
    }
    print_attempts(&view)?;
    print_answered(&t)?;
    print_pending(&view)?;
    let p = &view.paths;
    say!("files:");
    print_lane_files("plan", &p.plan_files)?;
    for (name, path) in [
        ("round", &p.round_file),
        ("review summary", &p.review_summary),
    ] {
        if let Some(path) = path {
            say!("  {name}: {}", path.display());
        }
    }
    for r in &p.plan_rounds {
        let owner = if r.by.is_some() { " (owner)" } else { "" };
        say!("  plan round {}{owner}: {}", r.n, r.feedback.display());
    }
    print_lane_files("notes", &p.notes_files)?;
    if let Some(url) = &p.pr_url {
        say!("  PR: {url} at {}", p.pr_head.as_deref().map_or("-", short));
    }
    Ok(())
}

/// A ticket's evidence files, by attempt: each listed file's absolute
/// path, what was kept but not listed, and when a directory was swept.
fn evidence(ticket: &str, stage: Option<&str>) -> Result<()> {
    let runner = offline_runner()?;
    let t = runner.load_ticket(ticket)?;
    let mut any = false;
    for a in &t.attempts {
        let Some(ev) = &a.evidence else { continue };
        if stage.is_some_and(|s| s != a.stage) {
            continue;
        }
        any = true;
        say!("{}/{} #{}:", a.stage, a.context, a.n);
        if let Some(swept) = &ev.swept {
            say!("  swept {} ({})", date_of(swept.at_ms), swept.why);
            continue;
        }
        for f in &ev.files {
            say!("  {}", ev.dir.join(&f.rel).display());
        }
        if !ev.over_cap.is_empty() {
            say!("  kept, not listed: {}", ev.over_cap.join(", "));
        }
    }
    if !any {
        say!("no evidence");
    }
    Ok(())
}

/// The stages in order, an external gate's check after its name:
/// `plan, implement, ready (pr-checks), merge (pr-merged)`.
fn stages_line(view: &dispatch_control::TicketView) -> String {
    view.stages
        .iter()
        .enumerate()
        .map(
            |(i, name)| match view.stage_checks.get(i).cloned().flatten() {
                Some(check) => format!("{name} ({check})"),
                None => name.clone(),
            },
        )
        .collect::<Vec<_>>()
        .join(", ")
}

/// The chosen lanes in the order their merges are held to, when any of
/// them waits on another: `backend, then frontend (after backend's base
/// pipeline)`.
fn merge_order_line(
    view: &dispatch_control::TicketView,
    p: Option<&dispatch::pipeline::Pipeline>,
) -> Option<String> {
    let p = p?;
    let chosen: Vec<&str> = view
        .lanes
        .iter()
        .filter(|l| l.chosen)
        .map(|l| l.name.as_str())
        .collect();
    let deps = |name: &str| {
        p.lane(name)
            .map_or_else(Vec::new, |l| l.merge_after_among(&chosen))
    };
    if chosen.iter().all(|name| deps(name).is_empty()) {
        return None;
    }
    let parts: Vec<String> = p
        .merge_order(&chosen)
        .into_iter()
        .map(|name| {
            let of = deps(name)
                .iter()
                .map(|d| format!("{d}'s"))
                .collect::<Vec<_>>()
                .join(" and ");
            if of.is_empty() {
                return name.to_owned();
            }
            match p.lane(name).map(Lane::deploy_wait) {
                Some(DeployWait::Run) => format!("{name} (after {of} base pipeline)"),
                Some(DeployWait::Step(step)) => {
                    format!("{name} (after {of} base pipeline step {step:?})")
                }
                Some(DeployWait::Merge) | None => name.to_owned(),
            }
        })
        .collect();
    Some(parts.join(", then "))
}

/// `YYYY-MM-DD` of `ms` since the epoch, in UTC.
fn date_of(ms: u64) -> String {
    let d = yyyymmdd(ms);
    format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..])
}

/// Attempts grouped by stage, each with its PR and review rounds.
fn print_attempts(view: &dispatch_control::TicketView) -> Result<()> {
    let mut stage_seen: Option<&str> = None;
    for a in &view.attempts {
        if stage_seen != Some(a.stage.as_str()) {
            say!("{}:", a.stage);
            stage_seen = Some(&a.stage);
        }
        let state = match &a.reason {
            Some(why) => format!("{}: {why}", a.state),
            None => a.state.clone(),
        };
        let gate = a
            .checks
            .as_ref()
            .map_or(String::new(), |c| format!(" gate-head {}", short(&c.head)));
        let rewrite = a.rewrite.as_ref().map_or(String::new(), |r| {
            let message = if r.stale.is_empty() {
                String::new()
            } else {
                format!(" message {}", r.message.as_deref().unwrap_or("asked"))
            };
            format!(" rewrite {} {}→{}{message}", r.mode, r.from, r.to)
        });
        say!(
            "  #{} {} {state} head {}{gate}{rewrite}{}",
            a.n,
            a.context,
            a.head.as_deref().map_or("-", short),
            nudged_clause(a.nudges.len())
        );
        if let Some(at) = a.evidence_swept_ms {
            say!("    evidence: swept {}", date_of(at));
        } else if !a.evidence.is_empty() {
            say!("    evidence: {} files", a.evidence.len());
        }
        if let Some(pr) = &a.pr {
            say!(
                "    PR #{} {} head {} checks {}",
                pr.number,
                pr.url,
                short(&pr.head),
                pr.checks
            );
        }
        for r in &a.rounds {
            say!(
                "    r{} {} open {}{}",
                r.n,
                r.state,
                r.open_points,
                nudged_clause(r.nudges.len())
            );
        }
    }
    Ok(())
}

/// `, rebased cleanly` or `, rebased by the rebaser, conflicts in N
/// commits, reviewed` after a lane that was brought up, in the words of
/// its `refreshed` event, or nothing.
fn brought_up_clause(
    view: &dispatch_control::TicketView,
    l: &dispatch_control::LaneView,
) -> String {
    view.lane_brought_up(l)
        .map_or_else(String::new, |c| format!(", {c}"))
}

/// `, nudged N times` after an attempt, round or stage line, worded as
/// the page words it, or nothing.
fn nudged_clause(n: usize) -> String {
    dispatch_control::nudged(n).map_or_else(String::new, |n| format!(", {n}"))
}

/// The answered decisions with who answered, and every answer a
/// supervisor gave that was refused.
fn print_answered(t: &Ticket) -> Result<()> {
    let answered: Vec<_> = t
        .decisions
        .iter()
        .filter(|d| matches!(d.state, DecisionState::Answered { .. }))
        .collect();
    if !answered.is_empty() {
        say!("answered:");
    }
    for d in answered {
        if let DecisionState::Answered {
            answer, by, note, ..
        } = &d.state
        {
            let note = note.as_ref().map_or(String::new(), |n| format!(" — {n}"));
            say!(
                "  {} [{}] {}: answered {answer} by {by}{note}",
                d.id,
                d.stage,
                d.name
            );
        }
    }
    let refused: Vec<_> = t
        .decisions
        .iter()
        .flat_map(|d| d.refusals.iter().map(move |r| (d, r)))
        .collect();
    if !refused.is_empty() {
        say!("refused:");
    }
    for (d, r) in refused {
        say!(
            "  {} [{}] {}: the {} asked {} at {}; the owner answers",
            d.id,
            d.stage,
            d.name,
            r.by,
            r.answer,
            clock(r.at_ms)
        );
    }
    Ok(())
}

/// The decisions that wait on the user, with the exact line that
/// answers each.
fn print_pending(view: &dispatch_control::TicketView) -> Result<()> {
    let pending: Vec<_> = view
        .decisions
        .iter()
        .filter(|d| d.state == "pending")
        .collect();
    if !pending.is_empty() {
        say!("waiting on you:");
    }
    for d in pending {
        say!(
            "  {} [{}] {}\n    options: {}{}\n    {} {} {} <answer> [--note <text> | --file <path>]",
            d.id,
            d.stage,
            d.question,
            d.options.join(" | "),
            d.recommendation
                .as_ref()
                .map_or(String::new(), |r| format!(" (suggested: {r})")),
            decide_command(),
            view.id,
            d.id
        );
    }
    Ok(())
}

/// A ticket's report, with git asked for the size of its PR's range in
/// Dispatch's clone of the lane's repository.
fn report_of(runner: &Runner, t: &dispatch::ticket::Ticket) -> TicketReport {
    let names = events::stage_names(t);
    let pipeline = runner.pipeline_of(t).ok();
    let range = report::pr_range(t).and_then(|(lane, base, head)| {
        let p = pipeline.as_ref()?;
        let dir = runner.lane_clone(p, p.lane(&lane)?);
        runner.git.range_size(&dir, &base, &head).ok()
    });
    report::of(
        t,
        pipeline.as_ref(),
        &names,
        &|p| std::fs::read_to_string(p).ok(),
        &report::plan_round_numbers,
        range,
        now_ms(),
    )
}

fn report(args: &[&str]) -> Result<()> {
    let f = Flags::parse(args, &["--project", "--since"], &["--json"]);
    let json = f.on("--json");
    let runner = offline_runner()?;
    match (&f.rest[..], f.value("--project")) {
        ([ticket], None) => {
            if f.value("--since").is_some() {
                usage();
            }
            let r = report_of(&runner, &runner.load_ticket(ticket)?);
            if json {
                say!("{}", serde_json::to_string_pretty(&r)?);
            } else {
                print_report(&r)?;
            }
        }
        ([], Some(project)) => {
            let since = match f.value("--since") {
                None => 0,
                Some(day) => report::parse_date(day).unwrap_or_else(|| {
                    eprintln!("--since takes a date as YYYY-MM-DD");
                    usage()
                }),
            };
            let mut tickets: Vec<_> = runner
                .tickets()?
                .into_iter()
                .filter(|t| t.project == project && t.created_ms >= since)
                .collect();
            tickets.sort_by_key(|t| t.created_ms);
            let rows: Vec<TicketReport> = tickets.iter().map(|t| report_of(&runner, t)).collect();
            let total = report::total(&rows);
            if json {
                say!(
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({ "tickets": rows, "total": total })
                    )?
                );
                return Ok(());
            }
            say!("ticket    time      plan  code  fixes  rebases  commits  title");
            let row = |id: &str, r: &TicketReport, title: &str| -> Result<()> {
                say!(
                    "{:<9} {:<9} {:<5} {:<5} {:<6} {:<8} {:<8} {}",
                    id,
                    span(r.stages.iter().map(|s| s.ms).sum()),
                    r.plan_points,
                    format!(
                        "{}{}",
                        r.code_points,
                        if r.code_incomplete { "+" } else { "" }
                    ),
                    r.fix_passes,
                    r.rebases,
                    r.commits.map_or("-".to_owned(), |c| c.to_string()),
                    title
                );
                Ok(())
            };
            for r in &rows {
                row(&r.id, r, &r.title)?;
            }
            row("total", &total, &format!("{} ticket(s)", total.tickets))?;
        }
        _ => usage(),
    }
    Ok(())
}

/// `N by you, M by supervisor`, then anyone else who answered.
fn answered_line(by: &std::collections::BTreeMap<String, u32>) -> String {
    let count = |who: &str| by.get(who).copied().unwrap_or(0);
    let mut parts = vec![
        format!("{} by you", count(dispatch::scheduler::BY_HAND)),
        format!(
            "{} by supervisor",
            count(dispatch::scheduler::BY_SUPERVISOR)
        ),
    ];
    for (who, n) in by {
        if who != dispatch::scheduler::BY_HAND && who != dispatch::scheduler::BY_SUPERVISOR {
            parts.push(format!("{n} by {who}"));
        }
    }
    parts.join(", ")
}

fn print_report(r: &TicketReport) -> Result<()> {
    say!("{} {} ({})", r.id, r.title, r.state);
    for s in &r.stages {
        if s.attempts == 0 {
            continue;
        }
        let nudges = nudged_clause(s.nudges as usize);
        say!(
            "  {}: {} over {} attempt(s), waiting on you {}{nudges}",
            s.stage,
            span(s.ms),
            s.attempts,
            span(s.waiting_ms)
        );
    }
    let per_lane: Vec<_> = r
        .plans
        .iter()
        .filter_map(|x| Some((x.lane.as_deref()?, x)))
        .collect();
    if !per_lane.is_empty() {
        for (lane, x) in per_lane {
            say!("  plan ({lane}): {} lines, {} bytes", x.lines, x.bytes);
        }
    } else if let (Some(lines), Some(bytes)) = (r.plan_lines, r.plan_bytes) {
        say!("  plan: {lines} lines, {bytes} bytes");
    } else {
        say!("  plan: none");
    }
    let rounds = |rs: &[report::RoundReport]| -> String {
        rs.iter()
            .map(|x| match x.new {
                Some(new) => format!("r{} new {new} open {}", x.n, x.open),
                None => format!("r{} open {}", x.n, x.open),
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    say!(
        "  plan review: {} round(s), {} point(s)",
        r.plan_rounds,
        r.plan_points
    );
    for p in &r.plan_reviews {
        say!("    {} #{}: {}", p.stage, p.attempt, rounds(&p.rounds));
    }
    let by: Vec<String> = r
        .code_points_by_reviewer
        .iter()
        .map(|(who, n)| format!("{who} {n}"))
        .collect();
    say!(
        "  code review: {} round(s), {} point(s){}{}",
        r.code_rounds,
        r.code_points,
        if by.is_empty() {
            String::new()
        } else {
            format!(" ({})", by.join(", "))
        },
        if r.code_incomplete {
            ", incomplete: a round's file is missing"
        } else {
            ""
        }
    );
    for c in &r.code_reviews {
        say!("    {} #{}: {}", c.stage, c.attempt, rounds(&c.rounds));
    }
    say!("  answered: {}", answered_line(&r.answered_by));
    say!("  fix passes: {}", r.fix_passes);
    say!("  rebases: at least {}", r.rebases);
    if r.evidence_files > 0 {
        say!(
            "  evidence: {} files, {} bytes",
            r.evidence_files,
            r.evidence_bytes
        );
    }
    match &r.pr_url {
        None => say!("  PR: none"),
        Some(url) => {
            let size = match (r.range, r.commits) {
                (Some(g), _) => format!(
                    "{} commit(s), {} file(s), +{} -{}",
                    g.commits, g.files, g.insertions, g.deletions
                ),
                (None, Some(c)) => format!("{c} commit(s) (from the rewrite), diff unknown"),
                (None, None) => "size unknown".to_owned(),
            };
            say!("  PR: {url}, {size}");
        }
    }
    say!("  cost and turns: not recorded");
    Ok(())
}

/// What a ticket's running agents show. Asks Switchboard, so this takes
/// the real port.
fn tail(args: &[&str]) -> Result<()> {
    let f = Flags::parse(args, &["--lines"], &[]);
    let ticket = f.one();
    let lines = f
        .number("--lines")
        .map_or(40, |n| u32::try_from(n).unwrap_or(u32::MAX));
    let mut runner = runner()?;
    let t = runner.load_ticket(ticket)?;
    let screens = runner.screens(&t, lines)?;
    if screens.is_empty() {
        say!("no agent running");
    }
    for (label, text) in screens {
        say!("== {label}");
        say!("{text}");
    }
    Ok(())
}

fn health(args: &[&str]) -> Result<()> {
    let f = Flags::parse(args, &["--timeout", "--stale"], &["--json", "--verbose"]);
    if !f.rest.is_empty() {
        usage();
    }
    let timeout = f
        .number("--timeout")
        .map_or(health::CHECK_TIMEOUT, Duration::from_secs);
    let stale_ms = f.number("--stale").map_or(health::STALE_MS, |s| s * 1000);
    let data = DataDir::from_env()?;
    let socket = SocketPort::from_env()?.path().to_path_buf();
    let checked = health::check(
        &data,
        &socket,
        timeout,
        stale_ms,
        now_ms(),
        f.on("--verbose"),
    );
    if f.on("--json") {
        say!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "ok": checked.ok,
                "lines": checked.lines,
            }))?
        );
    } else {
        for line in &checked.lines {
            say!("{line}");
        }
        say!("{}", if checked.ok { "ok" } else { "not ok" });
    }
    if !checked.ok {
        std::process::exit(1);
    }
    Ok(())
}

/// `dispatch brief <project>`: what a supervisor reads first. Read-only.
fn brief(args: &[&str]) -> Result<()> {
    let f = Flags::parse(args, &[], &[]);
    let project = f.one();
    let runner = offline_runner()?;
    let now = now_ms();
    match runner.supervisor_standing(project) {
        Ok(s) => say!("{}", supervisor_line(project, &s, now)),
        Err(e) => say!("supervisor: {e:#}"),
    }
    let tickets: Vec<Ticket> = runner
        .tickets()?
        .into_iter()
        .filter(|t| t.project == project && !matches!(t.state, TicketState::Closed { .. }))
        .collect();
    say!("\ntickets:");
    if tickets.is_empty() {
        say!("  none");
    }
    for t in &tickets {
        let stage = runner
            .pipeline_of(t)
            .ok()
            .and_then(|p| p.stages.get(t.stage).map(|s| s.name.clone()))
            .unwrap_or_else(|| "done".to_owned());
        say!(
            "  {} {} {} · stage {stage} · {}",
            t.id,
            t.source.label(),
            t.source.title,
            t.state.label()
        );
    }
    say!("\nwaiting on you:");
    let mut any = false;
    for t in &tickets {
        for d in t.waiting_on_you() {
            any = true;
            say!(
                "  {} {} [{}] {}: {}\n    options: {}\n    {} {} {} <answer> [--note <text> | --file <path>]",
                t.id,
                d.id,
                d.stage,
                d.name,
                d.question,
                d.options.join(" | "),
                decide_command(),
                t.id,
                d.id
            );
        }
    }
    if !any {
        say!("  nothing");
    }
    let log = events::log_path(&runner.data);
    let all = events::read_since(&log, 0)?;
    let gone = events::withdrawn(&all);
    let mine: Vec<&Event> = all
        .iter()
        .filter(|e| e.project == project && e.kind != Kind::Void && !gone.contains(&e.seq))
        .collect();
    say!("\nlast events:");
    for e in &mine[mine.len().saturating_sub(20)..] {
        say!("  {}", events::line(e, false));
    }
    say!("  follow from seq {}", events::last_seq(&log)?);
    let ps = runner.load_project(project)?;
    if !ps.supervisor.subscriptions.is_empty() {
        say!("\nsubscriptions (the runner types these into your pane when you are idle):");
    }
    for sub in &ps.supervisor.subscriptions {
        let lines = subscribe::undelivered(&runner.data, sub)?;
        say!(
            "  {} from seq {}: {} undelivered",
            sub.ticket,
            sub.since,
            lines.len()
        );
        for e in &lines[lines.len().saturating_sub(10)..] {
            say!("    {}", events::line(e, false));
        }
    }
    say!("\nopen worktrees:");
    let mut trees = false;
    for t in &tickets {
        for lane in t.lanes.iter().filter(|l| !l.removed) {
            trees = true;
            say!("  {} {}: {}", t.id, lane.name, lane.worktree.display());
        }
    }
    if !trees {
        say!("  none");
    }
    let handoff = runner.data.supervisor_dir(project).join("handoff.md");
    say!("\nhand-off ({}):", handoff.display());
    if let Ok(text) = std::fs::read_to_string(&handoff) {
        say!("{}", text.trim_end());
    } else {
        say!("(no hand-off yet)");
    }
    Ok(())
}

/// `dispatch subscribe <ticket> [--for move] [--since <seq>]`.
fn subscribe(args: &[&str]) -> Result<()> {
    let f = Flags::parse(args, &["--for", "--since"], &[]);
    let ticket = f.one();
    let what = f.value("--for").unwrap_or("move");
    let by = match actor() {
        Actor::Owner => "owner".to_owned(),
        Actor::Supervisor(_) => {
            std::env::var("SWITCHBOARD_RECORD_ID").unwrap_or_else(|_| BY_SUPERVISOR.to_owned())
        }
    };
    let mut runner = offline_runner()?;
    let sub = runner.subscribe(ticket, what, f.number("--since"), &by, now_ms())?;
    say!(
        "subscribed to {} from seq {}; the runner types its moves into the supervisor's pane",
        sub.ticket,
        sub.since
    );
    Ok(())
}

fn unsubscribe(ticket: &str) -> Result<()> {
    if offline_runner()?.unsubscribe(ticket)? {
        say!("unsubscribed from {ticket}");
    } else {
        say!("{ticket} was not subscribed");
    }
    Ok(())
}

/// Each subscription with its cursor and what has not been delivered,
/// then a delivery in flight and why deliveries wait.
fn subscriptions(project: &str) -> Result<()> {
    let runner = offline_runner()?;
    let ps = runner.load_project(project)?;
    let sup = &ps.supervisor;
    if sup.subscriptions.is_empty() {
        say!("no subscriptions");
    }
    for sub in &sup.subscriptions {
        let undelivered = subscribe::undelivered(&runner.data, sub)?.len();
        say!(
            "{} --for {} since {}: {undelivered} undelivered (by {}, {})",
            sub.ticket,
            sub.what,
            sub.since,
            sub.by,
            local_time(sub.at_ms)
        );
    }
    if let Some(d) = &sup.delivery {
        say!(
            "in flight: {} to {} since {}",
            d.op,
            d.session,
            local_time(d.at_ms)
        );
    }
    if let Some(why) = &sup.delivery_waits {
        say!("waits: {why} (since {})", local_time(sup.delivery_waits_ms));
    }
    Ok(())
}

/// The supervisor in one line: its session, age and seed.
fn supervisor_line(project: &str, s: &supervisor::Standing, now: u64) -> String {
    if s.table.is_none() {
        return format!("supervisor: none (no [supervisor] table in pipelines/{project}.toml)");
    }
    let Some(c) = &s.ps.supervisor.current else {
        return format!("supervisor: none; `dispatch supervisor {project} --fresh` starts one");
    };
    let seed = if s.stale {
        "stale: `[supervisor]` changed, `--fresh` to reseed"
    } else {
        "current"
    };
    format!(
        "supervisor: session {}, {} old, seed {} ({seed})",
        c.session,
        span(now.saturating_sub(c.created_ms)),
        c.seed_hash
    )
}

/// `dispatch supervisor <project> [--fresh [--setup] | --resume | --kill
/// [--reason <text>]]`. The read is offline; the rest talk to
/// Switchboard.
fn supervise(args: &[&str]) -> Result<()> {
    let f = Flags::parse(
        args,
        &["--reason"],
        &["--fresh", "--setup", "--resume", "--kill"],
    );
    let project = f.one();
    let chosen = ["--fresh", "--resume", "--kill"]
        .iter()
        .filter(|s| f.on(s))
        .count();
    if chosen > 1
        || (f.on("--setup") && !f.on("--fresh"))
        || (f.value("--reason").is_some() && !f.on("--kill"))
    {
        usage();
    }
    let now = now_ms();
    if f.on("--fresh") {
        let mut runner = runner()?;
        let c = runner.supervisor_fresh(project, f.on("--setup"), "fresh", now)?;
        say!("{project}: supervisor session {} started", c.session);
        say!("open it from the Dispatch page in Switchboard");
        return Ok(());
    }
    if f.on("--resume") {
        let mut runner = runner()?;
        let session = runner.supervisor_resume(project)?;
        // Switchboard answers before its transcript check: one with no
        // transcript is then marked not resumable and stays cold.
        say!("{project}: resume of supervisor session {session} asked");
        say!("the Dispatch page shows whether it came back");
        return Ok(());
    }
    if f.on("--kill") {
        let mut runner = runner()?;
        let why = f.value("--reason").unwrap_or("killed by hand");
        runner.supervisor_kill(project, why, now)?;
        say!("{project}: supervisor killed: {why}");
        return Ok(());
    }
    let runner = offline_runner()?;
    let s = runner.supervisor_standing(project)?;
    say!("{}", supervisor_line(project, &s, now));
    say!("workspace {}", s.workspace.display());
    say!("hand-off {}", s.handoff.display());
    say!("replaced {} supervisor(s)", s.ps.supervisor.past.len());
    if let Some(e) = &s.ps.supervisor.error {
        say!("error: {e}");
    }
    for refusal in &s.ps.supervisor.refusals {
        say!(
            "refused: {} at {}",
            refusal.answer,
            local_time(refusal.at_ms)
        );
    }
    if s.ps.supervisor.intent.is_some() {
        say!("a fresh one starts on the runner's next pass");
    }
    if s.ps.supervisor.current.is_some() {
        say!("open it from the Dispatch page in Switchboard");
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

#[cfg(test)]
mod tests {
    use super::*;
    use dispatch::ticket::{Decision, DecisionKind};

    fn ticket() -> Ticket {
        serde_json::from_value(serde_json::json!({
            "id": "t1",
            "project": "p",
            "source": {
                "kind": "github",
                "identity": "o/r#7",
                "number": 7,
                "title": "a title",
                "body": "",
                "url": null,
                "labels": [],
                "taken_at_ms": 0
            },
            "pipeline_fingerprint": "",
            "pipeline_file": "",
            "lanes": [],
            "stage": 0,
            "attempts": [],
            "decisions": [],
            "ledger": [],
            "processes": [],
            "root_project": null,
            "state": "active",
            "created_ms": 0,
            "updated_ms": 0
        }))
        .unwrap()
    }

    fn rerun(state: DecisionState) -> Decision {
        Decision {
            id: "d1".into(),
            stage: "implement".into(),
            name: "rerun".into(),
            kind: DecisionKind::Permission,
            question: "Run it again?".into(),
            options: vec!["rerun".into(), "park".into()],
            recommendation: None,
            attempt: Some(("implement".into(), 1)),
            state,
            made_ms: 3,
            refusals: Vec::new(),
        }
    }

    fn answered(by: &str) -> DecisionState {
        DecisionState::Answered {
            answer: "rerun".into(),
            note: None,
            by: by.into(),
            at_ms: 4,
            acted: false,
        }
    }

    #[test]
    fn the_decide_hint_is_only_for_a_decision_that_still_waits() {
        let mut t = ticket();
        t.decisions.push(rerun(DecisionState::Pending));
        let event = Event::from_decision(&t, &t.decisions[0]);
        assert_eq!(
            decide_hint(&event, &t).as_deref(),
            Some("    dispatch decide t1 d1 <answer> [--note <text> | --file <path>]")
        );
        for by in [dispatch::scheduler::BY_RESUME, dispatch::scheduler::BY_HAND] {
            t.decisions[0].state = answered(by);
            assert_eq!(decide_hint(&event, &t), None, "answered by {by}");
        }
    }
}
