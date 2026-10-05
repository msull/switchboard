//! `dispatch`: take a ticket, run the scheduler, answer decisions, look.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use dispatch::USAGE;
use dispatch::epoch_ms;
use dispatch::events::{self, Event, For, Kind, Waited, short};
use dispatch::git::GitCli;
use dispatch::github::Gh;
use dispatch::port::SocketPort;
use dispatch::report::{self, TicketReport};
use dispatch::scheduler::{Runner, kept_branches};
use dispatch::serve::{Handler, Server, take_issue, take_pull_requests};
use dispatch::serve::{ticket_paths, ticket_view};
use dispatch::store::DataDir;
use dispatch::ticket::{DecisionState, TicketState};
use std::io::Write as _;

/// `EX_USAGE`: the command line was wrong. `wait` gives 2 and 3 their
/// own meanings, so a usage error is never mistaken for them.
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

fn runner() -> Result<Runner> {
    Ok(Runner::new(
        DataDir::from_env()?,
        Box::new(SocketPort::from_env()?),
        Box::new(GitCli::default()),
    ))
}

/// A runner for the commands that never talk to Switchboard.
fn offline_runner() -> Result<Runner> {
    Ok(Runner::new(
        DataDir::from_env()?,
        Box::new(NoPort),
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
        ["close", ticket] => close(ticket, None),
        ["close", ticket, "--reason", reason] => close(ticket, Some(reason)),
        ["worktrees", rest @ ..] => worktrees(rest),
        ["events", rest @ ..] => events(rest),
        ["wait", rest @ ..] => wait(rest),
        ["show", rest @ ..] => show(rest),
        ["report", rest @ ..] => report(rest),
        ["tail", rest @ ..] => tail(rest),
        ["health", rest @ ..] => health(rest),
        _ => usage(),
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
    for t in tickets {
        let p = runner.pipeline_of(&t).ok();
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
    let runner = offline_runner()?;
    let t = runner.resume(ticket, now_ms())?;
    say!(
        "{} {} {} active again",
        t.id,
        t.source.label(),
        t.source.title
    );
    Ok(())
}

/// Close a ticket through Switchboard, as the runner would: its
/// processes are read back and its session unmarked, so this takes the
/// real port, not `NoPort`.
fn close(ticket: &str, reason: Option<&str>) -> Result<()> {
    let mut runner = runner()?;
    let t = runner.close_by_hand(ticket, reason, now_ms())?;
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

/// `hh:mm:ss` of `ms` since the epoch, in the local zone: the reader is
/// a person or an agent on this machine, and a UTC clock next to a local
/// one (the shell's `date`, a log file) misleads twice a day. Falls back
/// to UTC only when the moment is out of range.
fn clock(ms: u64) -> String {
    use chrono::{DateTime, Local, TimeZone as _};
    let secs = i64::try_from(ms / 1000).unwrap_or(i64::MAX);
    if let Some(utc) = DateTime::from_timestamp(secs, 0) {
        return Local
            .from_utc_datetime(&utc.naive_utc())
            .format("%H:%M:%S")
            .to_string();
    }
    let s = (ms / 1000) % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
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

/// An event as a line: as stored with `--json`, else
/// `seq  hh:mm:ss  ticket  stage  kind  text`.
fn event_line(e: &Event, json: bool) -> String {
    if json {
        return e.to_line();
    }
    let text = if e.kind == Kind::Void {
        let seqs: Vec<String> = e.voids.iter().map(u64::to_string).collect();
        format!("withdraws {}: {}", seqs.join(", "), e.text)
    } else {
        e.text.clone()
    };
    format!(
        "{}  {}  {}  {}  {}  {text}",
        e.seq,
        clock(e.at_ms),
        e.ticket,
        e.stage,
        e.kind.as_str()
    )
}

fn events(args: &[&str]) -> Result<()> {
    let f = Flags::parse(
        args,
        &["--since", "--ticket", "--project"],
        &["--follow", "--json"],
    );
    if !f.rest.is_empty() {
        usage();
    }
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
            say!("{}", event_line(e, json));
        }
    }
    if !f.on("--follow") {
        return Ok(());
    }
    // Followed, a withdrawn event may already be printed when its void
    // comes; the void's line says which seqs to drop.
    let mut follow = events::follow(&path, last);
    loop {
        for e in follow.next_batch()? {
            if keep(&e) {
                say!("{}", event_line(&e, json));
            }
        }
        std::thread::sleep(Duration::from_millis(events::FOLLOW_POLL_MS));
    }
}

fn wait(args: &[&str]) -> Result<()> {
    let f = Flags::parse(args, &["--for", "--timeout"], &["--json"]);
    let ticket = f.one();
    let what = match f.value("--for") {
        None => For::Any,
        Some(word) => For::parse(word).unwrap_or_else(|| {
            eprintln!("--for takes decision, stage, pr, closed or any");
            usage()
        }),
    };
    let deadline = f.number("--timeout").map(|s| now_ms() + s * 1000);
    let json = f.on("--json");
    let data = DataDir::from_env()?;
    let waited = events::wait(&data, ticket, what, deadline, &mut now_ms, &mut || {
        std::thread::sleep(Duration::from_millis(events::FOLLOW_POLL_MS));
    })?;
    match waited {
        Waited::Matched(e) => {
            say!("{}", event_line(&e, json));
            if !json
                && e.kind == Kind::Decision
                && let Some(d) = &e.decision
            {
                say!("    dispatch decide {ticket} {d} <answer> [--note <text>]");
            }
            Ok(())
        }
        Waited::Ended(e) => {
            say!("{}", event_line(&e, json));
            std::process::exit(EXIT_ENDED);
        }
        Waited::TimedOut => {
            if json {
                say!(r#"{{"timed_out":true}}"#);
            } else {
                say!("timed out");
            }
            std::process::exit(EXIT_TIMED_OUT);
        }
    }
}

fn show(args: &[&str]) -> Result<()> {
    let f = Flags::parse(args, &[], &["--json"]);
    let runner = offline_runner()?;
    let t = runner.load_ticket(f.one())?;
    let mut view = ticket_view(&t, runner.pipeline_of(&t).ok().as_ref());
    view.paths = ticket_paths(&t);
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
    say!("state {}", t.state.label());
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
            conflict_clause(&view, l)
        );
    }
    print_attempts(&view)?;
    print_pending(&view)?;
    let p = &view.paths;
    say!("files:");
    for (name, path) in [
        ("plan", &p.plan),
        ("round", &p.round_file),
        ("review summary", &p.review_summary),
        ("notes", &p.notes),
    ] {
        if let Some(path) = path {
            say!("  {name}: {}", path.display());
        }
    }
    if let Some(url) = &p.pr_url {
        say!("  PR: {url} at {}", p.pr_head.as_deref().map_or("-", short));
    }
    Ok(())
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

/// `, rebased with conflicts in N commits, reviewed` after a lane whose
/// last bring-up resolved a conflict, worded as the ticket page words it.
fn conflict_clause(view: &dispatch_control::TicketView, l: &dispatch_control::LaneView) -> String {
    view.lane_conflict(l)
        .map_or_else(String::new, |c| format!(", {c}"))
}

/// `, nudged N times` after an attempt, round or stage line, worded as
/// the page words it, or nothing.
fn nudged_clause(n: usize) -> String {
    dispatch_control::nudged(n).map_or_else(String::new, |n| format!(", {n}"))
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
            "  {} [{}] {}\n    options: {}{}\n    dispatch decide {} {} <answer> [--note <text>]",
            d.id,
            d.stage,
            d.question,
            d.options.join(" | "),
            d.recommendation
                .as_ref()
                .map_or(String::new(), |r| format!(" (suggested: {r})")),
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
    let range = report::pr_range(t).and_then(|(lane, base, head)| {
        let p = runner.pipeline_of(t).ok()?;
        let dir = runner.lane_clone(&p, p.lane(&lane)?);
        runner.git.range_size(&dir, &base, &head).ok()
    });
    report::of(
        t,
        &names,
        &|p| std::fs::read_to_string(p).ok(),
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
    if let (Some(lines), Some(bytes)) = (r.plan_lines, r.plan_bytes) {
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
    say!("  fix passes: {}", r.fix_passes);
    say!("  rebases: at least {}", r.rebases);
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
    let f = Flags::parse(args, &["--timeout", "--stale"], &["--json"]);
    if !f.rest.is_empty() {
        usage();
    }
    let timeout = Duration::from_secs(f.number("--timeout").unwrap_or(2));
    let stale_ms = f.number("--stale").unwrap_or(30) * 1000;
    let data = DataDir::from_env()?;
    let socket = SocketPort::from_env()?.path().to_path_buf();
    let checked = dispatch::health::check(&data, &socket, timeout, stale_ms, now_ms());
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
