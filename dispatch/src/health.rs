//! The runner's health: what its calls to Switchboard and the PR
//! providers did, kept in memory by the runner and written after every
//! pass to `<data>/runner.json`, and `check`, which reads that file and
//! asks both sockets whether they answer. Not a record: the file is a
//! status, replaced whole.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::store::{DataDir, atomic_write, read_ticket};
use crate::ticket::Attempt;

/// The status file's name in the data directory.
pub const RUNNER_FILE: &str = "runner.json";

/// How long a failure stays listed.
pub const FAILURE_KEEP_MS: u64 = 3_600_000;
/// How long `check` waits for each socket unless told otherwise.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(2);
/// How long an open attempt may go unstepped before `check` calls it
/// stale, unless told otherwise.
pub const STALE_MS: u64 = 30_000;

/// One call that failed: when, for which ticket, and the error's text
/// (never a request body).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Failure {
    /// When the call failed.
    pub at_ms: u64,
    /// The ticket it was made for, when there was one.
    pub ticket: Option<String>,
    /// The error's text.
    pub what: String,
}

/// Calls to Switchboard's control port.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PortHealth {
    /// When a call last answered.
    pub last_ok_ms: Option<u64>,
    /// How long that call took.
    pub last_latency_ms: Option<u64>,
    /// The failures of the last `FAILURE_KEEP_MS`.
    pub failures: Vec<Failure>,
}

/// Calls to a pull request provider.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderHealth {
    /// When a call last answered.
    pub last_ok_ms: Option<u64>,
    /// The failures of the last `FAILURE_KEEP_MS`.
    pub failures: Vec<Failure>,
}

/// One ticket: the last time the runner got on with it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TicketHealth {
    /// When a step or a call for it last went through.
    pub last_ok_ms: Option<u64>,
}

/// `runner.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunnerFile {
    /// The runner's process, which `check` asks is alive.
    pub pid: u32,
    /// When the runner started.
    pub started_ms: u64,
    /// When the last pass ended.
    pub last_pass_ms: u64,
    /// How long it took.
    pub last_pass_took_ms: u64,
    /// Calls to Switchboard.
    pub port: PortHealth,
    /// Calls to the pull request providers.
    pub gh: ProviderHealth,
    /// Each ticket the runner has got on with, by id.
    pub tickets: BTreeMap<String, TicketHealth>,
}

/// What the runner has seen of its calls since it started. Times are the
/// wall clock's, since `health` compares them with its own.
#[derive(Debug, Default)]
pub struct Health {
    /// The ticket being stepped or recovered, which a call made without
    /// one in hand (a query) is put down to.
    pub current: Option<String>,
    /// Calls to Switchboard.
    pub port: PortHealth,
    /// Calls to the pull request providers.
    pub gh: ProviderHealth,
    /// Each ticket the runner has got on with, by id.
    pub tickets: BTreeMap<String, TicketHealth>,
}

fn wall_ms() -> u64 {
    crate::epoch_ms(SystemTime::now())
}

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

impl Health {
    fn ticket_of(&self, ticket: Option<&str>) -> Option<String> {
        ticket.map(str::to_owned).or_else(|| self.current.clone())
    }

    /// A call to Switchboard that started at `started`, and how it went;
    /// `ticket` is `None` for the current one.
    pub fn port_call<T>(
        &mut self,
        ticket: Option<&str>,
        started: Instant,
        result: &std::io::Result<T>,
    ) {
        let at_ms = wall_ms();
        let ticket = self.ticket_of(ticket);
        match result {
            Ok(_) => {
                self.port.last_ok_ms = Some(at_ms);
                self.port.last_latency_ms = Some(ms(started.elapsed()));
                if let Some(id) = ticket {
                    self.tickets.entry(id).or_default().last_ok_ms = Some(at_ms);
                }
            }
            Err(e) => self.port.failures.push(Failure {
                at_ms,
                ticket,
                what: format!("{:?}: {e}", e.kind()),
            }),
        }
    }

    /// A call to a pull request provider, put down to the current
    /// ticket.
    pub fn gh_call<T>(&mut self, result: &Result<T>) {
        let at_ms = wall_ms();
        let ticket = self.current.clone();
        match result {
            Ok(_) => {
                self.gh.last_ok_ms = Some(at_ms);
                if let Some(id) = ticket {
                    self.tickets.entry(id).or_default().last_ok_ms = Some(at_ms);
                }
            }
            Err(e) => self.gh.failures.push(Failure {
                at_ms,
                ticket,
                what: format!("{e:#}"),
            }),
        }
    }

    /// A ticket's step finished without an error.
    pub fn stepped(&mut self, ticket: &str) {
        self.tickets
            .entry(ticket.to_owned())
            .or_default()
            .last_ok_ms = Some(wall_ms());
    }

    /// Failures older than `FAILURE_KEEP_MS` before `now_ms` dropped.
    fn prune(&mut self, now_ms: u64) {
        let keep = |f: &Failure| now_ms.saturating_sub(f.at_ms) <= FAILURE_KEEP_MS;
        self.port.failures.retain(keep);
        self.gh.failures.retain(keep);
    }

    /// The status file as it stands after a pass. The failures older
    /// than `FAILURE_KEEP_MS` are dropped from the runner's own lists
    /// first, so a runner that lives for days keeps only the last hour.
    pub fn file(&mut self, pid: u32, started_ms: u64, now_ms: u64, took_ms: u64) -> RunnerFile {
        self.prune(now_ms);
        RunnerFile {
            pid,
            started_ms,
            last_pass_ms: now_ms,
            last_pass_took_ms: took_ms,
            port: self.port.clone(),
            gh: self.gh.clone(),
            tickets: self.tickets.clone(),
        }
    }

    /// Write `runner.json`, replacing it whole. Not under the writer
    /// lock: only the one runner writes it.
    pub fn write(
        &mut self,
        data: &DataDir,
        pid: u32,
        started_ms: u64,
        now_ms: u64,
        took_ms: u64,
    ) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.file(pid, started_ms, now_ms, took_ms))?;
        atomic_write(&data.root.join(RUNNER_FILE), &bytes)
    }
}

/// What `check` found: one line per finding for a person, and whether
/// everything is well.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checked {
    /// One finding per line.
    pub lines: Vec<String>,
    /// Nothing found wrong.
    pub ok: bool,
}

/// Whether process `pid` is alive, by `ps`: std has no signal call, and
/// the crate allows no `unsafe`.
#[must_use]
pub fn alive(pid: u32) -> bool {
    std::process::Command::new("ps")
        .args(["-p", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// One query on Switchboard's socket, timed.
fn switchboard_answers(socket: &Path, timeout: Duration) -> std::result::Result<u64, String> {
    let started = Instant::now();
    let mut client = switchboard_control::Client::connect_with_timeout(socket, timeout)
        .map_err(|e| e.to_string())?;
    let reply = client
        .call(&switchboard_control::Request::new(
            "health",
            switchboard_control::Body::Waiting,
        ))
        .map_err(|e| e.to_string())?;
    match reply {
        switchboard_control::Reply::Waiting { .. } => Ok(ms(started.elapsed())),
        other => Err(format!("answered {other:?}")),
    }
}

/// One `status` on Dispatch's own socket, timed.
fn dispatch_answers(socket: &Path, timeout: Duration) -> std::result::Result<u64, String> {
    let started = Instant::now();
    let mut client = dispatch_control::Client::connect_with_timeout(socket, timeout)
        .map_err(|e| e.to_string())?;
    let reply = client
        .call(&dispatch_control::Request::new(
            "health",
            dispatch_control::Body::Status,
        ))
        .map_err(|e| e.to_string())?;
    match reply {
        dispatch_control::Reply::Status(_) => Ok(ms(started.elapsed())),
        dispatch_control::Reply::Failed { reason } => Err(reason),
        _ => Err("answered something other than a status".into()),
    }
}

/// Seconds ago, for a person.
fn ago(now_ms: u64, at_ms: u64) -> String {
    format!("{}s ago", now_ms.saturating_sub(at_ms) / 1000)
}

/// Failures that share a ticket and an error, folded into one line.
struct Group {
    count: usize,
    first_ms: u64,
    last_ms: u64,
}

/// The lines for one kind of failure (`what` is `port` or `gh`). With
/// `verbose`, one per failure. Otherwise failures with the same ticket
/// and error fold into one line with a count and the last and first
/// time; a failure that happened once reads as it does with `verbose`.
/// Either way the newest trouble comes last.
fn failure_lines(what: &str, failures: &[Failure], now_ms: u64, verbose: bool) -> Vec<String> {
    let single = |at_ms: u64, ticket: Option<&str>, text: &str| {
        format!(
            "{what} failure {} {}: {text}",
            ago(now_ms, at_ms),
            ticket.unwrap_or("-")
        )
    };
    if verbose {
        return failures
            .iter()
            .map(|x| single(x.at_ms, x.ticket.as_deref(), &x.what))
            .collect();
    }
    let mut groups: BTreeMap<(Option<&str>, &str), Group> = BTreeMap::new();
    for x in failures {
        let g = groups
            .entry((x.ticket.as_deref(), x.what.as_str()))
            .or_insert(Group {
                count: 0,
                first_ms: x.at_ms,
                last_ms: x.at_ms,
            });
        g.count += 1;
        g.first_ms = g.first_ms.min(x.at_ms);
        g.last_ms = g.last_ms.max(x.at_ms);
    }
    let mut groups: Vec<_> = groups.into_iter().collect();
    // Stable, so equal last times keep the map's key order.
    groups.sort_by_key(|(_, g)| g.last_ms);
    groups
        .into_iter()
        .map(|((ticket, text), g)| {
            if g.count == 1 {
                single(g.last_ms, ticket, text)
            } else {
                format!(
                    "{what} failure ×{} {}: {text}, last {}, first {}",
                    g.count,
                    ticket.unwrap_or("-"),
                    ago(now_ms, g.last_ms),
                    ago(now_ms, g.first_ms)
                )
            }
        })
        .collect()
}

/// Whether the runner is up and getting on: its status file fresh and
/// its process alive, both sockets answering within `timeout`, no port
/// failure since its last success, and no active ticket with an open
/// attempt left unstepped for longer than `stale_ms`. It never takes
/// `runner.lock`, which could make a starting runner exit. Failures
/// that repeat for one ticket with one error are a single line with a
/// count unless `verbose`, which lists each.
#[must_use]
pub fn check(
    data: &DataDir,
    switchboard_socket: &Path,
    timeout: Duration,
    stale_ms: u64,
    now_ms: u64,
    verbose: bool,
) -> Checked {
    let mut lines = Vec::new();
    let mut ok = true;
    let file: Option<RunnerFile> = std::fs::read(data.root.join(RUNNER_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    match &file {
        None => {
            ok = false;
            lines.push(format!(
                "runner: no {} in {}; is `dispatch run` up?",
                RUNNER_FILE,
                data.root.display()
            ));
        }
        Some(f) => {
            let live = alive(f.pid);
            let fresh = now_ms.saturating_sub(f.last_pass_ms) <= stale_ms;
            ok &= live && fresh;
            lines.push(format!(
                "runner: pid {} {}, last pass {} took {} ms{}",
                f.pid,
                if live { "alive" } else { "not running" },
                ago(now_ms, f.last_pass_ms),
                f.last_pass_took_ms,
                if fresh { "" } else { " (stale)" }
            ));
        }
    }
    match switchboard_answers(switchboard_socket, timeout) {
        Ok(latency) => lines.push(format!("switchboard: answered in {latency} ms")),
        Err(e) => {
            ok = false;
            lines.push(format!(
                "switchboard: {} did not answer: {e}",
                switchboard_socket.display()
            ));
        }
    }
    let own = data.root.join(dispatch_control::SOCKET_FILE);
    match dispatch_answers(&own, timeout) {
        Ok(latency) => lines.push(format!("dispatch: answered in {latency} ms")),
        Err(e) => {
            ok = false;
            lines.push(format!("dispatch: {} did not answer: {e}", own.display()));
        }
    }
    let Some(f) = file else {
        return Checked { lines, ok };
    };
    let recent = |fs: &[Failure]| -> Vec<Failure> {
        fs.iter()
            .filter(|x| now_ms.saturating_sub(x.at_ms) <= FAILURE_KEEP_MS)
            .cloned()
            .collect()
    };
    for (what, failures) in [
        ("port", recent(&f.port.failures)),
        ("gh", recent(&f.gh.failures)),
    ] {
        lines.extend(failure_lines(what, &failures, now_ms, verbose));
    }
    let last_failure = f.port.failures.iter().map(|x| x.at_ms).max();
    if last_failure.is_some_and(|at| f.port.last_ok_ms.is_none_or(|ok_ms| at > ok_ms)) {
        ok = false;
        lines.push("port: the last call to Switchboard failed".into());
    }
    for path in data.ticket_files().unwrap_or_default() {
        let Ok(t) = read_ticket(&path) else {
            continue;
        };
        if !t.active() || !t.attempts.iter().any(Attempt::is_open) {
            continue;
        }
        let last = f.tickets.get(&t.id).and_then(|h| h.last_ok_ms);
        if last.is_none_or(|at| now_ms.saturating_sub(at) > stale_ms) {
            ok = false;
            lines.push(format!(
                "ticket {} stale: {}",
                t.id,
                last.map_or("never stepped by this runner".to_owned(), |at| format!(
                    "last stepped {}",
                    ago(now_ms, at)
                ))
            ));
        }
    }
    Checked { lines, ok }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    use super::*;

    #[test]
    fn an_empty_data_directory_is_not_healthy() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let checked = check(
            &data,
            &dir.path().join("control.sock"),
            Duration::from_secs(1),
            30_000,
            wall_ms(),
            false,
        );
        assert!(!checked.ok);
        assert!(
            checked.lines[0].contains("no runner.json"),
            "{:?}",
            checked.lines
        );
        assert!(checked.lines.iter().any(|l| l.starts_with("switchboard:")));
    }

    #[test]
    fn a_socket_that_never_answers_times_out_and_one_that_does_is_timed() {
        let dir = tempfile::Builder::new()
            .prefix("dh")
            .tempdir_in("/tmp")
            .unwrap();
        let data = DataDir::new(dir.path());
        let silent = dir.path().join("silent.sock");
        let listener = UnixListener::bind(&silent).unwrap();
        let held = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            // Held open without a reply until the client gives up.
            std::thread::sleep(Duration::from_millis(1500));
            drop(stream);
        });
        let started = Instant::now();
        let checked = check(
            &data,
            &silent,
            Duration::from_secs(1),
            30_000,
            wall_ms(),
            false,
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(!checked.ok);
        let line = checked
            .lines
            .iter()
            .find(|l| l.starts_with("switchboard:"))
            .unwrap();
        assert!(line.contains("did not answer"), "{line}");
        held.join().unwrap();

        let answering = dir.path().join("answer.sock");
        let listener = UnixListener::bind(&answering).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let mut w = stream;
            writeln!(w, r#"{{"reply":"waiting","sessions":[]}}"#).unwrap();
        });
        let checked = check(
            &data,
            &answering,
            Duration::from_secs(1),
            30_000,
            wall_ms(),
            false,
        );
        server.join().unwrap();
        assert!(
            checked
                .lines
                .iter()
                .any(|l| l.starts_with("switchboard: answered in ")),
            "{:?}",
            checked.lines
        );
    }

    fn failures_for_folding() -> Vec<Failure> {
        let failure = |at_ms, ticket: Option<&str>, what: &str| Failure {
            at_ms,
            ticket: ticket.map(Into::into),
            what: what.into(),
        };
        vec![
            failure(10_000, Some("t1"), "NotFound"),
            failure(20_000, Some("t2"), "timed out"),
            failure(30_000, Some("t1"), "NotFound"),
            failure(40_000, Some("t1"), "refused"),
            failure(50_000, Some("t1"), "NotFound"),
            failure(60_000, Some("t2"), "timed out"),
            failure(70_000, None, "refused"),
        ]
    }

    #[test]
    fn repeated_failures_fold_into_one_line_per_ticket_and_error() {
        let lines = failure_lines("port", &failures_for_folding(), 100_000, false);
        assert_eq!(
            lines,
            [
                "port failure 60s ago t1: refused",
                "port failure ×3 t1: NotFound, last 50s ago, first 90s ago",
                "port failure ×2 t2: timed out, last 40s ago, first 80s ago",
                "port failure 30s ago -: refused",
            ]
        );
    }

    #[test]
    fn verbose_lists_every_failure() {
        let failures = failures_for_folding();
        let lines = failure_lines("gh", &failures, 100_000, true);
        assert_eq!(lines.len(), failures.len());
        assert_eq!(lines[0], "gh failure 90s ago t1: NotFound");
        assert_eq!(lines[6], "gh failure 30s ago -: refused");
    }

    #[test]
    fn failures_older_than_an_hour_are_dropped() {
        let mut h = Health::default();
        let failure = |at_ms| Failure {
            at_ms,
            ticket: Some("t1".into()),
            what: "WouldBlock: timed out".into(),
        };
        h.port.failures = vec![failure(1_000), failure(FAILURE_KEEP_MS + 5_000)];
        h.gh.failures = vec![failure(1_000)];
        let file = h.file(7, 1, FAILURE_KEEP_MS + 10_000, 3);
        assert_eq!(file.port.failures, [failure(FAILURE_KEEP_MS + 5_000)]);
        assert!(file.gh.failures.is_empty());
        // The runner's own lists are pruned, not only the file's.
        assert_eq!(h.port.failures, file.port.failures);
        assert!(h.gh.failures.is_empty());
    }

    #[test]
    fn a_call_is_put_down_to_the_current_ticket() {
        let mut h = Health {
            current: Some("t1".into()),
            ..Health::default()
        };
        h.port_call(None, Instant::now(), &Ok::<(), std::io::Error>(()));
        assert!(h.tickets["t1"].last_ok_ms.is_some());
        h.port_call(
            Some("t2"),
            Instant::now(),
            &Err::<(), _>(std::io::Error::new(std::io::ErrorKind::WouldBlock, "slow")),
        );
        assert_eq!(h.port.failures[0].ticket.as_deref(), Some("t2"));
        assert!(h.port.failures[0].what.starts_with("WouldBlock"));
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        h.write(&data, 7, 1, 2, 3).unwrap();
        let back: RunnerFile =
            serde_json::from_slice(&std::fs::read(dir.path().join(RUNNER_FILE)).unwrap()).unwrap();
        assert_eq!((back.pid, back.last_pass_took_ms), (7, 3));
        assert_eq!(back.port.failures.len(), 1);
    }
}
