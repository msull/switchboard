//! The tools: each takes `host`, the name a host was paired under
//! (optional while only one is paired), and maps to the host's remote
//! port. There is no raw `send` and no shell: what an agent may type
//! into another agent goes through the owner channel, headed as its
//! own, and what it may run is what the owner made or approved.

use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::config::{self, Host};
use crate::events::{self, Waited};
use crate::http;
use crate::server::Server;

/// `wait`'s default and largest timeouts, in seconds: under the minute
/// a desktop client allows a call, and under the 30-minute idle limit.
pub const WAIT_DEFAULT_SECS: u64 = 45;
/// `wait`'s largest timeout, in seconds: 25 minutes, so a call ends
/// before the client's 30-minute idle limit with time to spare.
pub const WAIT_MAX_SECS: u64 = 1500;
/// `run` and `ask_supervisor`'s default wait, in seconds.
pub const RUN_DEFAULT_SECS: u64 = 45;
/// How often a wait reads again when no event came.
const POLL: Duration = Duration::from_secs(2);
/// How often a long wait reports progress, when the client asked.
const PROGRESS_EVERY: Duration = Duration::from_secs(15);

/// One tool call: the server, its cancel flag, and the client's
/// progress token if it sent one.
pub struct Call<'a> {
    pub server: &'a Server,
    pub cancel: &'a AtomicBool,
    pub progress: Option<Value>,
}

type Outcome = Result<Value, String>;

/// Every tool: name, description, and its arguments' properties and
/// required names (`host` is added to each).
#[allow(clippy::too_many_lines)] // one row per tool
fn table() -> Vec<(&'static str, &'static str, Value, Vec<&'static str>)> {
    let s = |d: &str| json!({ "type": "string", "description": d });
    let n = |d: &str| json!({ "type": "integer", "description": d });
    let b = |d: &str| json!({ "type": "boolean", "description": d });
    vec![
        (
            "hosts",
            "The hosts this server is paired with: name, address, scope, and whether one needs pairing again.",
            json!({}),
            vec![],
        ),
        (
            "projects",
            "The host's projects and their ids.",
            json!({ "all": b("Include Dispatch's own worker projects") }),
            vec![],
        ),
        (
            "sessions",
            "A project's sessions (agents, shells, commands, services) and their state.",
            json!({ "project": s("Project id or name") }),
            vec!["project"],
        ),
        (
            "session",
            "One session's record and state.",
            json!({ "session": s("Session id") }),
            vec!["session"],
        ),
        (
            "conversation",
            "The last turns of an agent session's conversation, masked.",
            json!({ "session": s("Session id"), "turns": n("How many turns, from the last (default 5)") }),
            vec!["session"],
        ),
        (
            "last_response",
            "An agent session's last answer.",
            json!({ "session": s("Session id") }),
            vec!["session"],
        ),
        (
            "screen",
            "The last lines of a running session's pane, masked.",
            json!({ "session": s("Session id"), "lines": n("Lines (default 40)") }),
            vec!["session"],
        ),
        (
            "waiting",
            "The sessions waiting on the owner.",
            json!({}),
            vec![],
        ),
        (
            "tickets",
            "Dispatch's tickets on the host, with their standing.",
            json!({}),
            vec![],
        ),
        (
            "ticket",
            "One Dispatch ticket: summary, body, attempts, lanes, decisions.",
            json!({ "ticket": s("Ticket id") }),
            vec!["ticket"],
        ),
        (
            "ticket_events",
            "A ticket's event log.",
            json!({ "ticket": s("Ticket id") }),
            vec!["ticket"],
        ),
        (
            "ticket_file",
            "One file of a ticket's attempt, by the key the ticket lists.",
            json!({ "ticket": s("Ticket id"), "key": s("File key") }),
            vec!["ticket", "key"],
        ),
        (
            "brief",
            "A Dispatch project's brief: supervisor, tickets, what waits on the owner, recent events, worktrees, hand-off.",
            json!({ "project": s("Dispatch project name, as tickets carry it") }),
            vec!["project"],
        ),
        (
            "run_output",
            "The output of a command session's run, masked; the last run unless `n` is given.",
            json!({ "session": s("Session id"), "n": n("Run number"), "lines": n("Lines (default 200)") }),
            vec!["session"],
        ),
        (
            "wait",
            "Block until a session, a ticket or a project changes on the host, or the timeout passes. Loop on it for longer waits.",
            json!({ "session": s("Session id"), "ticket": s("Ticket id"), "project": s("Project id"), "timeout": n("Seconds (default 45, at most 1500)") }),
            vec![],
        ),
        (
            "new_session",
            "Open a session in a project: claude, codex or shell, in the root or a directory under it.",
            json!({ "project": s("Project id or name"), "name": s("Session name"), "kind": s("claude, codex or shell"), "cwd": s("Directory under the project root"), "prompt": s("An agent's first message") }),
            vec!["project", "name", "kind"],
        ),
        (
            "run",
            "Run a project's command entry (one the owner made or approved) and return its exit code and output, or that it still runs at the timeout.",
            json!({ "project": s("Project id or name"), "entry": s("The command session's name"), "timeout": n("Seconds to wait (default 45)") }),
            vec!["project", "entry"],
        ),
        (
            "decide",
            "Answer a ticket's pending decision; held to what the project's supervisor table allows an agent.",
            json!({ "ticket": s("Ticket id"), "decision": s("Decision id"), "answer": s("The option, or options joined by commas"), "note": s("A note, where the option needs one") }),
            vec!["ticket", "decision", "answer"],
        ),
        (
            "close_ticket",
            "Close a ticket: its worktrees removed, its branch and records kept.",
            json!({ "ticket": s("Ticket id") }),
            vec!["ticket"],
        ),
        (
            "ask_supervisor",
            "Message a Dispatch project's supervisor session through the owner channel. It is typed into the supervisor's pane when it is idle, headed as from this agent, not the owner. With `wait`, returns the supervisor's answer.",
            json!({ "project": s("Dispatch project name"), "text": s("The message"), "wait": b("Wait for the answer"), "timeout": n("Seconds to wait (default 45)") }),
            vec!["project", "text"],
        ),
        (
            "dispatch",
            "Dispatch verbs with no tool of their own: `report` a ticket, or `park` it.",
            json!({ "verb": s("report or park"), "ticket": s("Ticket id"), "reason": s("Why it is parked") }),
            vec!["verb", "ticket"],
        ),
        (
            "resume",
            "Resume an agent session's conversation. Spends a paid turn: set `confirm` only when the owner said to resume this session.",
            json!({ "session": s("Session id"), "confirm": b("The owner said to resume it") }),
            vec!["session", "confirm"],
        ),
        (
            "add_project",
            "Add a project on the host, rooted at a directory inside a folder of the host user's home.",
            json!({ "name": s("Project name"), "root": s("Absolute path on the host"), "space": s("Space id (default: the host's current)") }),
            vec!["name", "root"],
        ),
    ]
}

/// The tools as `tools/list` lists them.
#[must_use]
pub fn list() -> Vec<Value> {
    table()
        .into_iter()
        .map(|(name, description, props, required)| {
            let mut props = props;
            props["host"] = json!({
                "type": "string",
                "description": "The host's name, as `hosts` lists it; may be left out while one is paired",
            });
            json!({
                "name": name,
                "description": description,
                "inputSchema": { "type": "object", "properties": props, "required": required },
            })
        })
        .collect()
}

/// Whether `name` is a tool.
#[must_use]
pub fn known(name: &str) -> bool {
    table().iter().any(|(n, ..)| *n == name)
}

fn arg<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args[name].as_str().map(str::trim).filter(|s| !s.is_empty())
}

fn need<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    arg(args, name).ok_or_else(|| format!("`{name}` is required"))
}

fn number(args: &Value, name: &str) -> Option<u64> {
    args[name]
        .as_u64()
        .or_else(|| args[name].as_str().and_then(|s| s.parse().ok()))
}

/// A value that goes into a path as one segment; anything that would
/// change the path's shape is refused rather than encoded.
fn segment(value: &str) -> Result<&str, String> {
    if value.is_empty()
        || value
            .chars()
            .any(|c| matches!(c, '/' | '?' | '#' | '%' | ' ') || c.is_control())
    {
        return Err(format!("{value:?} cannot be named in a path"));
    }
    Ok(value)
}

fn op() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Run tool `name`.
#[allow(clippy::too_many_lines)] // one arm per tool
pub fn run(call: &Call, name: &str, args: &Value) -> Outcome {
    if name == "hosts" {
        return hosts(call);
    }
    let host = pick_host(call, arg(args, "host"))?;
    let c = Conn { call, host };
    match name {
        "projects" => c.get(if args["all"].as_bool() == Some(true) {
            "/api/v1/projects?scope=all"
        } else {
            "/api/v1/projects"
        }),
        "sessions" => {
            let p = c.project_id(need(args, "project")?)?;
            c.get(&format!("/api/v1/projects/{p}/sessions"))
        }
        "session" => c.get(&format!(
            "/api/v1/sessions/{}",
            segment(need(args, "session")?)?
        )),
        "conversation" => {
            let turns = number(args, "turns").unwrap_or(5).clamp(1, 100);
            c.get(&format!(
                "/api/v1/sessions/{}/conversation?from=last&limit={turns}",
                segment(need(args, "session")?)?
            ))
        }
        "last_response" => c.get(&format!(
            "/api/v1/sessions/{}/last-response",
            segment(need(args, "session")?)?
        )),
        "screen" => {
            let lines = number(args, "lines").unwrap_or(40);
            c.get(&format!(
                "/api/v1/sessions/{}/screen?lines={lines}",
                segment(need(args, "session")?)?
            ))
        }
        "waiting" => c.get("/api/v1/waiting"),
        "tickets" => c.get("/api/v1/dispatch/tickets"),
        "ticket" => c.get(&format!(
            "/api/v1/dispatch/tickets/{}",
            segment(need(args, "ticket")?)?
        )),
        "ticket_events" => c.get(&format!(
            "/api/v1/dispatch/tickets/{}/events",
            segment(need(args, "ticket")?)?
        )),
        "ticket_file" => c.get(&format!(
            "/api/v1/dispatch/tickets/{}/files/{}",
            segment(need(args, "ticket")?)?,
            segment(need(args, "key")?)?
        )),
        "brief" => {
            c.owner_channel()?;
            c.get(&format!(
                "/api/v1/dispatch/projects/{}/brief",
                segment(need(args, "project")?)?
            ))
        }
        "run_output" => run_output(&c, args),
        "wait" => wait(&c, args),
        "new_session" => {
            let p = c.project_id(need(args, "project")?)?;
            let mut body = json!({
                "op": op(),
                "name": need(args, "name")?,
                "kind": need(args, "kind")?,
            });
            for field in ["cwd", "prompt"] {
                if let Some(v) = arg(args, field) {
                    body[field] = json!(v);
                }
            }
            c.post(&format!("/api/v1/projects/{p}/sessions"), &body)
        }
        "run" => run_entry(&c, args),
        "decide" => {
            c.agent_device()?;
            let mut body = json!({
                "decision": need(args, "decision")?,
                "answer": need(args, "answer")?,
            });
            if let Some(note) = arg(args, "note") {
                body["note"] = json!(note);
            }
            c.post(
                &format!(
                    "/api/v1/dispatch/tickets/{}/decide",
                    segment(need(args, "ticket")?)?
                ),
                &body,
            )
        }
        "close_ticket" => {
            c.agent_device()?;
            c.post(
                &format!(
                    "/api/v1/dispatch/tickets/{}/close",
                    segment(need(args, "ticket")?)?
                ),
                &json!({}),
            )
        }
        "ask_supervisor" => ask_supervisor(&c, args),
        "dispatch" => {
            c.owner_channel()?;
            let ticket = segment(need(args, "ticket")?)?;
            match need(args, "verb")? {
                "report" => c.get(&format!("/api/v1/dispatch/tickets/{ticket}/report")),
                "park" => {
                    let mut body = json!({});
                    if let Some(reason) = arg(args, "reason") {
                        body["reason"] = json!(reason);
                    }
                    c.post(&format!("/api/v1/dispatch/tickets/{ticket}/park"), &body)
                }
                other => Err(format!(
                    "verb {other:?}: this tool takes report or park; `brief`, `ticket` and `decide` are tools of their own"
                )),
            }
        }
        "resume" => {
            let session = segment(need(args, "session")?)?;
            if args["confirm"].as_bool() != Some(true) {
                return Err("resume spends a paid turn and was not sent: set `confirm: true` only when the owner has said to resume this session".into());
            }
            c.post(
                &format!("/api/v1/sessions/{session}/resume"),
                &json!({ "op": op() }),
            )
        }
        "add_project" => {
            c.owner_channel()?;
            let mut body = json!({
                "op": op(),
                "name": need(args, "name")?,
                "root": need(args, "root")?,
            });
            if let Some(space) = arg(args, "space") {
                body["space"] = json!(space);
            }
            c.post("/api/v1/projects", &body)
        }
        _ => Err(format!("no tool {name}")),
    }
}

/// The paired hosts with no token.
fn hosts(call: &Call) -> Outcome {
    let hosts = config::load(&call.server.dir)?;
    Ok(json!({
        "hosts": hosts.hosts.iter().map(|h| json!({
            "name": h.name,
            "addresses": h.addresses,
            "server_id": h.server_id,
            "scope": h.scope,
            "paired_ms": h.paired_ms,
            "needs_pairing": h.needs_pairing,
        })).collect::<Vec<_>>(),
    }))
}

fn pick_host(call: &Call, name: Option<&str>) -> Result<Host, String> {
    let hosts = config::load(&call.server.dir)?;
    let host = match name {
        Some(name) => hosts.get(name).cloned().ok_or_else(|| {
            format!(
                "no host {name:?} is paired; `hosts` lists them, `switchboard-mcp pair` adds one"
            )
        })?,
        None => match hosts.hosts.as_slice() {
            [one] => one.clone(),
            [] => return Err("no host is paired; run `switchboard-mcp pair` first".into()),
            _ => return Err("several hosts are paired; name one with `host`".into()),
        },
    };
    if host.needs_pairing {
        return Err(repair(&host.name));
    }
    Ok(host)
}

fn repair(name: &str) -> String {
    format!(
        "host {name} needs pairing again (its key changed or it no longer knows this device): open a control pairing window with Agent ticked on the host and run `switchboard-mcp pair {name} --address <host:port> --spki <key> --code <code>`"
    )
}

/// One host for one call.
struct Conn<'a> {
    call: &'a Call<'a>,
    host: Host,
}

impl Conn<'_> {
    fn exchange(&self, method: &str, path: &str, body: Option<&Value>) -> Outcome {
        let reply = http::request(
            &self.host.addresses,
            &self.host.spki_sha256,
            method,
            path,
            Some(&self.host.token),
            body,
        );
        match reply {
            Err(http::Failure::Pin) => Err(self.needs_pairing()),
            Err(http::Failure::Io(e)) => Err(format!("host {}: {e}", self.host.name)),
            Ok(r) if r.status == 401 => Err(self.needs_pairing()),
            Ok(r) if r.ok() => Ok(r.body().clone()),
            Ok(r) => {
                let (code, message) = r.error();
                Err(format!("{code} ({}): {message}", r.status))
            }
        }
    }

    fn needs_pairing(&self) -> String {
        if let Err(e) =
            config::mark_needs_pairing(&self.call.server.dir, &self.host.name, &self.host.token)
        {
            eprintln!("switchboard-mcp: {e}");
        }
        repair(&self.host.name)
    }

    fn get(&self, path: &str) -> Outcome {
        self.exchange("GET", path, None)
    }

    fn post(&self, path: &str, body: &Value) -> Outcome {
        self.exchange("POST", path, Some(body))
    }

    /// Refuse a host from before the owner channel and its routes.
    fn owner_channel(&self) -> Result<(), String> {
        let cached = self
            .call
            .server
            .info
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&self.host.name)
            .cloned();
        let info = if let Some(info) = cached {
            info
        } else {
            let info = self.get("/api/v1/info")?;
            self.call
                .server
                .info
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(self.host.name.clone(), info.clone());
            info
        };
        let has = info["capabilities"]
            .as_array()
            .is_some_and(|c| c.iter().any(|c| c == "owner_channel"));
        if has {
            Ok(())
        } else {
            Err(format!(
                "host too old: {} runs Switchboard {}, which has no owner channel, brief, runs or new projects on its port; update it",
                self.host.name,
                info["version"].as_str().unwrap_or("of an earlier version")
            ))
        }
    }

    /// Refuse a decision or a close unless the host knows this device as
    /// an agent. Against a host without the owner channel, or a device
    /// paired without Agent ticked, the host would take the call as the
    /// owner's own, with no `by` and so no supervisor `decides` table.
    fn agent_device(&self) -> Result<(), String> {
        self.owner_channel()?;
        if self.get("/api/v1/me")?["agent"].as_bool() == Some(true) {
            Ok(())
        } else {
            Err(format!(
                "this device is not paired as an agent on {}: its decisions and closes would be taken as the owner's. Open a control pairing window with Agent ticked and pair again",
                self.host.name
            ))
        }
    }

    /// A project's id from its id or its name.
    fn project_id(&self, project: &str) -> Result<String, String> {
        if uuid::Uuid::parse_str(project).is_ok() {
            return Ok(project.to_owned());
        }
        let listed = self.get("/api/v1/projects?scope=all")?;
        let projects = listed["projects"].as_array().cloned().unwrap_or_default();
        let found: Vec<&Value> = projects.iter().filter(|p| p["name"] == project).collect();
        match found.as_slice() {
            [one] => one["id"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "a project without an id".to_owned()),
            [] => Err(format!(
                "no project named {project:?} on {}",
                self.host.name
            )),
            _ => Err(format!(
                "several projects are named {project:?}; give its id"
            )),
        }
    }

    fn feed(&self) -> Result<std::sync::Arc<events::Feed>, String> {
        match self.call.server.hub.feed(&self.host) {
            Ok(feed) => Ok(feed),
            Err(http::Failure::Pin) => Err(self.needs_pairing()),
            Err(http::Failure::Io(e)) if e == "unauthorized" => Err(self.needs_pairing()),
            Err(http::Failure::Io(e)) => Err(format!("host {}: {e}", self.host.name)),
        }
    }

    /// Wait on `feed` for a frame `matches`, at most `step`, reporting
    /// progress when the client asked for it. Whether one came.
    fn wake(
        &self,
        feed: &events::Feed,
        since: &mut u64,
        until: Instant,
        matches: &dyn Fn(&Value) -> bool,
    ) -> Result<Option<Value>, String> {
        let start = Instant::now();
        let mut last_progress = start;
        loop {
            let next = until.min(Instant::now() + PROGRESS_EVERY);
            match feed.wait_for(since, next, self.call.cancel, matches) {
                Waited::Matched(frame) => return Ok(Some(frame)),
                Waited::Cancelled => return Err("cancelled".into()),
                Waited::Closed(why) => {
                    return Err(format!(
                        "the event stream of {} closed: {why}",
                        self.host.name
                    ));
                }
                Waited::TimedOut if Instant::now() >= until => return Ok(None),
                Waited::TimedOut => {
                    if let Some(token) = &self.call.progress
                        && last_progress.elapsed() >= PROGRESS_EVERY
                    {
                        last_progress = Instant::now();
                        self.call.server.send(&json!({
                            "jsonrpc": "2.0",
                            "method": "notifications/progress",
                            "params": {
                                "progressToken": token,
                                "progress": start.elapsed().as_secs(),
                                "message": "waiting",
                            },
                        }));
                    }
                }
            }
        }
    }
}

fn seconds(args: &Value, default: u64) -> Duration {
    Duration::from_secs(
        number(args, "timeout")
            .unwrap_or(default)
            .clamp(1, WAIT_MAX_SECS),
    )
}

/// `wait`: one target, a change to it or the timeout.
fn wait(c: &Conn, args: &Value) -> Outcome {
    let until = Instant::now() + seconds(args, WAIT_DEFAULT_SECS);
    let feed = c.feed()?;
    let mut since = feed.mark();
    if let Some(session) = arg(args, "session") {
        let session = segment(session)?.to_owned();
        let frame = c.wake(&feed, &mut since, until, &|f| {
            events::is_change(f, &["session", "runs", "conversation"], &session)
        })?;
        return Ok(changed(frame));
    }
    if let Some(project) = arg(args, "project") {
        let project = c.project_id(project)?;
        let frame = c.wake(&feed, &mut since, until, &|f| {
            events::is_change(f, &["project", "sessions"], &project)
        })?;
        return Ok(changed(frame));
    }
    let ticket = segment(
        need(args, "ticket")
            .map_err(|_| "`wait` takes one of `session`, `ticket` or `project`".to_owned())?,
    )?;
    // The tickets are one collection on the port: every move of it is
    // read against the ticket's own time.
    let before = updated_ms(&c.get(&format!("/api/v1/dispatch/tickets/{ticket}"))?);
    loop {
        let frame = c.wake(&feed, &mut since, until, &|f| {
            events::is_change(f, &["tickets"], "")
        })?;
        let Some(frame) = frame else {
            return Ok(changed(None));
        };
        let now = c.get(&format!("/api/v1/dispatch/tickets/{ticket}"))?;
        if frame["type"] == "resync_required" || updated_ms(&now) != before {
            return Ok(json!({ "changed": true, "reason": "ticket", "ticket": now }));
        }
    }
}

fn updated_ms(ticket: &Value) -> Option<u64> {
    ticket["updated_ms"]
        .as_u64()
        .or_else(|| ticket["ticket"]["updated_ms"].as_u64())
}

fn changed(frame: Option<Value>) -> Value {
    match frame {
        Some(f) => json!({
            "changed": true,
            "reason": format!(
                "{} {} {}",
                f["type"].as_str().unwrap_or(""),
                f["resource"]["kind"].as_str().unwrap_or(""),
                f["resource"]["id"].as_str().unwrap_or("")
            ).trim().to_owned(),
        }),
        None => json!({ "changed": false }),
    }
}

/// The runs of `session`, as the port lists them.
fn runs(c: &Conn, session: &str) -> Result<Vec<Value>, String> {
    Ok(c.get(&format!("/api/v1/sessions/{session}/runs"))?["runs"]
        .as_array()
        .cloned()
        .unwrap_or_default())
}

fn run_output(c: &Conn, args: &Value) -> Outcome {
    let session = segment(need(args, "session")?)?;
    let n = match number(args, "n") {
        Some(n) => n,
        None => runs(c, session)?
            .last()
            .and_then(|r| r["n"].as_u64())
            .ok_or_else(|| "the session has no run".to_owned())?,
    };
    let lines = number(args, "lines").unwrap_or(200);
    c.get(&format!(
        "/api/v1/sessions/{session}/runs/{n}/output?lines={lines}"
    ))
}

/// `run`: start a new run of the project's command entry, then wait
/// for it to end.
fn run_entry(c: &Conn, args: &Value) -> Outcome {
    c.owner_channel()?;
    let until = Instant::now() + seconds(args, RUN_DEFAULT_SECS);
    let project = c.project_id(need(args, "project")?)?;
    let entry = need(args, "entry")?;
    let sessions = c.get(&format!("/api/v1/projects/{project}/sessions?scope=all"))?;
    let listed = sessions["sessions"].as_array().cloned().unwrap_or_default();
    let found: Vec<&Value> = listed.iter().filter(|s| s["name"] == entry).collect();
    let session = match found.as_slice() {
        [one] => one["id"].as_str().unwrap_or_default().to_owned(),
        [] => {
            return Err(format!(
                "the project has no session named {entry:?}; `sessions` lists them"
            ));
        }
        _ => return Err(format!("several sessions are named {entry:?}")),
    };
    let session = segment(&session)?.to_owned();
    let feed = c.feed()?;
    let mut since = feed.mark();
    let started = c.post(
        &format!("/api/v1/sessions/{session}/run"),
        &json!({ "op": op() }),
    )?;
    let n = started["run"]
        .as_u64()
        .ok_or_else(|| format!("the host started no run: {started}"))?;
    loop {
        let run = runs(c, &session)?
            .into_iter()
            .find(|r| r["n"].as_u64() == Some(n));
        if let Some(run) = run.filter(|r| !r["ended_ms"].is_null()) {
            let output = c.get(&format!(
                "/api/v1/sessions/{session}/runs/{n}/output?lines=200"
            ))?;
            return Ok(json!({
                "session": session,
                "run": n,
                "exit": run["exit"],
                "output": output["text"],
                "truncated": output["truncated"],
            }));
        }
        if Instant::now() >= until {
            return Ok(json!({ "running": true, "session": session, "run": n }));
        }
        let step = until.min(Instant::now() + POLL);
        c.wake(&feed, &mut since, step, &|f| {
            events::is_change(f, &["runs", "session"], &session)
        })?;
    }
}

/// `ask_supervisor`: queue the message, and with `wait` read where it
/// stands until the supervisor has answered or the timeout passes.
fn ask_supervisor(c: &Conn, args: &Value) -> Outcome {
    c.owner_channel()?;
    let project = segment(need(args, "project")?)?;
    let text = need(args, "text")?;
    let until = Instant::now() + seconds(args, RUN_DEFAULT_SECS);
    let feed = if args["wait"].as_bool() == Some(true) {
        Some(c.feed()?)
    } else {
        None
    };
    let mut since = feed.as_ref().map_or(0, |f| f.mark());
    let path = format!("/api/v1/dispatch/projects/{project}/supervisor/ask");
    let asked = c.post(&path, &json!({ "op": op(), "text": text }))?;
    let Some(feed) = feed else {
        return Ok(asked);
    };
    let id = segment(asked["id"].as_str().unwrap_or_default())?.to_owned();
    loop {
        let status = c.get(&format!("{path}/{id}"))?;
        if status["state"] == "answered" || Instant::now() >= until {
            return Ok(status);
        }
        let step = until.min(Instant::now() + POLL);
        c.wake(&feed, &mut since, step, &|f| {
            matches!(
                f["resource"]["kind"].as_str(),
                Some("session" | "conversation" | "tickets")
            )
        })?;
    }
}
