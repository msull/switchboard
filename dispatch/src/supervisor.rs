//! A project's supervisor: one long-lived Claude Code session, made from
//! the live pipeline's `[supervisor]` table, that watches the project's
//! tickets and answers the decisions its `decides` lists. The pure parts
//! are its seed, its launch flags, the hash that says when the seed is
//! stale, and the rule for which commands a supervisor may run; the
//! `impl Runner` makes, replaces, kills and resumes the session.
//!
//! Who runs a command is read from `SWITCHBOARD_RECORD_ID`, which the
//! supervisor's Bash tool inherits from its pane. That guards against a
//! supervisor's mistakes, not against a hostile agent, which can unset
//! any variable.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use switchboard_control::{self as wire, Body, Reply, Request};

use crate::pipeline::{OperatorKind, Pipeline, Supervisor};
use crate::scheduler::{BY_SUPERVISOR, Runner, SocketDown};
use crate::store::{DataDir, atomic_write, read_project, read_ticket, shell_unsafe};
use crate::ticket::{Operation, PastSupervisor, ProjectState, SupervisorIntent, SupervisorRecord};

/// The commands a supervisor works with, for its seed. `{exe}` is the
/// full path of the `dispatch` it must type, since its allow rule
/// matches that path. A test holds every verb named here to `USAGE`.
pub const GUIDE_ESSENTIALS: &str = "\
- `{exe} brief <project>`: the project at a glance: tickets, what waits, \
the last events with the seq to follow from, open worktrees, the hand-off.
- `{exe} wait <ticket> --for move --since <seq> --timeout 540`: your watch \
on one ticket, always as a background call: it returns on the ticket's \
next stage change, decision, pull request line, park or close after \
`<seq>`, with every such line that follows within two seconds (exit 0; \
exit 3 for a park or close), or after 540 seconds with nothing (exit 2; \
arm it again). `<seq>` is the `follow from seq` that `brief` printed the \
first time, the seq of the last event line the watch printed after a \
return, and the same seq again after `timed out`, so nothing between \
two watches is lost, and a decision left pending does not come back. \
Give the call a 600000 ms tool timeout. Never in the foreground, never \
in a shell loop.
- `{exe} events --project <project> --since <seq> --follow --timeout 540`: \
the same for the whole project, for the Monitor tool or a background \
call: it exits 0 as soon as it prints events (note the last seq and start \
again from it) and 2 when nothing happened. Never run `--follow` without \
`--timeout`.
- `{exe} show <ticket>`: one ticket: stage, lanes, attempts, decisions, \
files, and the exact `decide` line for each pending decision.
- `{exe} report <ticket>`: how a ticket went.
- `{exe} decide <ticket> <decision> <answer> --note <why>`: answer a \
decision you are allowed to; say why in the note. An answer to any other \
decision is refused and logged.
- `{exe} park <ticket> --reason <why>` stops a ticket's work; \
`{exe} resume <ticket> --no-rerun` brings a parked one back with every \
rerun left a question.
";

/// The prompt the session starts with: a path, like every prompt sent
/// through `session.new`.
fn first_prompt(seed: &Path) -> String {
    format!("Read {} and do what it says.", seed.display())
}

/// What one decision's answers do, in the seed's words.
fn decision_words(name: &str) -> &'static str {
    match name {
        "finalize" => "`finalize` ends a plan review with the plan as it stands",
        "paused" => "`continue` lets a paused review run another round",
        "rerun" => {
            "`rerun` runs a failed or cancelled attempt again and spends an agent run; \
             `check` runs only its checks again; `keep` keeps a rewritten history"
        }
        "pr" => "`recheck` looks for the attempt's pull request again",
        "branch" => {
            "`reuse` checks out an earlier ticket's branch as it is; `fresh` renames it and cuts a new one"
        }
        "lanes" => "the lanes the ticket works in, joined by commas",
        "refresh" => "`recheck` brings the lanes up to their bases again",
        "merge" => {
            "`recheck` sends the ticket back through the `pr-checks` stage before it to bring \
             the branch up and read the checks again; `park` stops it; you never merge"
        }
        _ => "answer from the options the question lists",
    }
}

/// The seed: everything the session needs to start, written to
/// `seed.md`. Pure: the caller gives the paths.
#[must_use]
pub fn seed(
    project: &str,
    sup: &Supervisor,
    exe: &Path,
    handoff: &Path,
    workspace: &Path,
) -> String {
    let exe_text = exe.display().to_string();
    let mut out = String::new();
    let _ = write!(
        out,
        "# Supervisor of {project}\n\n{guidance}\n\n\
         You supervise Dispatch's tickets for the project `{project}`. Your working \
         directory is {workspace}.\n\n\
         Dispatch's command is `{exe_text}`. Type every command with that full path: your \
         permission to run it matches the path, and nothing else.\n\n\
         ## How you work\n\n\
         The owner talks to you in this session and hands you work; you take no initiative \
         of your own. You do not take issues, file issues, answer decisions, merge, or edit \
         the repository unless the owner asked for that in this session. When they hand \
         you a ticket, you drive it through every stage: you answer its decisions from the \
         list below with a note that says why, you handle its pull request as the Pull \
         requests section says, and you tell the owner in a line or two at each stage \
         change and the moment it needs them.\n\n\
         When you start: run `{exe_text} brief {project}` first; compare what it says with \
         the hand-off at {handoff} and say in a few lines what changed; check \
         `{exe_text} health`; for every ticket that is in flight, arm a background watch \
         (below) from the `follow from seq` that brief printed, never from a seq in the \
         hand-off; then stop and wait for the owner. Do not fill the wait with reading or \
         polling.\n\n\
         Watching never blocks this session. A watch is a background call: run \
         `{exe_text} wait <ticket> --for move --since <seq> --timeout 540` with your Bash tool's \
         run-in-background option (or the Monitor tool over `events --follow`), so its \
         result arrives as a notification while you stay free to talk. Never run `wait` or \
         `events --follow` in the foreground, and never wrap either in a shell loop. When \
         a watch returns, read what it says, act only on tickets the owner handed you, and \
         arm it again from the seq of the last event line it printed (from the same seq \
         after a timeout) if that ticket is still in flight.\n\n\
         Keep {handoff} current as you work: what you watch, what you answered and why, \
         what is left. The next supervisor starts from it. Write it with your Edit or Write \
         tool, which your permissions allow; never through a shell script or a heredoc, \
         which the permission classifier refuses as instruction poisoning.\n\n",
        guidance = sup.guidance.trim(),
        workspace = workspace.display(),
        handoff = handoff.display(),
    );
    if !sup.read.is_empty() {
        out.push_str("## Read first\n\n");
        for path in &sup.read {
            let _ = writeln!(out, "- {}", workspace.join(path).display());
        }
        out.push('\n');
    }
    out.push_str("## Decisions you answer\n\n");
    if sup.decides.is_empty() {
        out.push_str("None.\n");
    }
    for name in &sup.decides {
        let _ = writeln!(out, "- `{name}`: {}", decision_words(name));
    }
    let runner = sup.may.iter().any(|m| m == "runner");
    let restart = sup.may.iter().any(|m| m == "restart");
    let _ = write!(
        out,
        "\nEvery other decision is the owner's: say so and move on. Tell the owner they \
         answer it {OWNER_ROUTES}. A `dispatch` command the owner types in this pane, a `!` \
         line included, runs as you and is refused the same way, so never suggest it, and \
         never change your environment to get round the rule. A `merge` question is \
         answered `recheck` or `park`, never merged by you: Dispatch resolves it when the \
         provider reports the merge. \
         You may not {restart_a_ticket}move the worktrees, {run_the_runner}, resume with \
         reruns unless `rerun` is yours, or replace yourself.\n\n",
        restart_a_ticket = if restart { "" } else { "restart a ticket, " },
        run_the_runner = if runner {
            "start a runner with `dispatch run`"
        } else {
            "run the runner"
        },
    );
    out.push_str(if sup.merges {
        "## Pull requests\n\nThe pull request itself is yours to merge: once Dispatch logs \
         `pr-checks passed`, read the pull request's body and commit message and check that \
         they carry no client name and no attribution line, then merge it with \
         `gh pr merge <n> --merge` and pull main in this workspace. Dispatch closes the \
         ticket when it sees the merge. A failed check is yours to look at; a fix by hand \
         goes on the ticket's branch, amended into its one commit and pushed with a lease, \
         then answer `recheck`.\n\n"
    } else {
        "## Pull requests\n\nYou never merge a pull request. When Dispatch logs \
         `pr-checks passed`, read the pull request's body and commit message, say to the \
         owner that it is green and whether the body is clean, and stop: the owner merges.\n\n"
    });
    capability_sections(&mut out, runner, restart, &exe_text);
    out.push_str("## Commands\n\n");
    out.push_str(&GUIDE_ESSENTIALS.replace("{exe}", &exe_text));
    out
}

/// The seed's sections for what the table's `may` gives, each drawn only
/// when it is given.
fn capability_sections(out: &mut String, runner: bool, restart: bool, exe_text: &str) {
    if runner {
        let _ = write!(
            out,
            "## The runner\n\nAfter you merge a change that touches `dispatch/`, pull main, \
             rebundle as the guidance says, run `{exe_text} runner restart`, then \
             `{exe_text} health`, and tell the owner the new pid. A restart refused because a \
             deploy is running is retried when that stage ends, never forced. If `runner \
             start` or `restart` fails, report it to the owner; never run `dispatch run` \
             yourself.\n\n",
        );
    }
    if restart {
        let _ = write!(
            out,
            "## Restarts\n\nYou may restart one of this project's tickets with \
             `{exe_text} restart <ticket> [<stage>]`, and only when the owner named that ticket \
             for a restart, or when the ticket was handed to you to take to done and its pull \
             request at `merge` cannot merge because it conflicts. A restart of a ticket that \
             is running a deploy is refused; retry it when that stage ends, never ask the owner \
             to force it. In your report, say what you restarted, at which stage, and why.\n\n",
        );
    }
}

/// The hash of what the owner controls about the seed: the table as
/// TOML, the project's name, and `GUIDE_ESSENTIALS`. Never the rendered
/// seed, which holds paths that differ between builds and machines.
#[must_use]
pub fn seed_hash(project: &str, sup: &Supervisor) -> String {
    let table = toml::to_string(sup).unwrap_or_default();
    Pipeline::fingerprint(&format!("{table}\n{project}\n{GUIDE_ESSENTIALS}"))
}

/// The session's flags: a model when set, an allow rule for the
/// `dispatch` executable, a read rule and a write rule for the
/// supervisor directory, and with `merges` the `gh pr` and `git pull`
/// rules. No settings file: `--settings` already carries Switchboard's
/// hooks.
#[must_use]
pub fn launch_flags(sup: &Supervisor, exe: &Path, dir: &Path) -> Vec<String> {
    let mut flags = Vec::new();
    if let Some(model) = &sup.model {
        flags.extend(["--model".to_owned(), model.clone()]);
    }
    flags.extend([
        "--allowedTools".to_owned(),
        format!("Bash({}:*)", exe.display()),
        "--allowedTools".to_owned(),
        format!("Read(//{}/**)", dir.display()),
    ]);
    flags.extend(OperatorKind::Claude.write_flags(dir));
    if sup.merges {
        for rule in MERGE_RULES {
            flags.extend(["--allowedTools".to_owned(), (*rule).to_owned()]);
        }
    }
    flags
}

/// The commands a merging supervisor runs on a pull request: read it,
/// read its checks, merge it, and bring its workspace up to main. Nothing
/// that pushes or rewrites a branch.
pub const MERGE_RULES: &[&str] = &[
    "Bash(gh pr view:*)",
    "Bash(gh pr checks:*)",
    "Bash(gh pr diff:*)",
    "Bash(gh pr merge:*)",
    "Bash(git pull:*)",
    "Bash(git log:*)",
];

/// The model a launch's flags name.
fn model_of(body: Option<&Body>) -> Option<String> {
    let Some(Body::SessionNew {
        launch: wire::Launch::Argv(argv),
        ..
    }) = body
    else {
        return None;
    };
    argv.iter()
        .position(|a| a == "--model")
        .and_then(|i| argv.get(i + 1).cloned())
}

/// The heading a rotation puts over the old hand-off.
const ROTATED_HEADING: &str = "## From the session of ";

/// `handoff.md`'s text after a rotation: the old text under one heading
/// naming the session it came from. The headings earlier rotations left
/// on top are replaced rather than stacked, since each kept
/// `handoff.<stamp>.md` already records them; the same heading further
/// down, below the session's own text, is left alone. Text that is empty
/// once those are gone gets no heading.
#[must_use]
pub fn rotated_handoff(old: &str, from_ms: u64) -> String {
    let mut rest = old;
    while let Some(line) = rest.split_inclusive('\n').next() {
        if !(line.trim().is_empty() || line.starts_with(ROTATED_HEADING)) {
            break;
        }
        rest = &rest[line.len()..];
    }
    if rest.trim().is_empty() {
        return rest.to_owned();
    }
    format!(
        "{ROTATED_HEADING}{}\n\n{rest}",
        stamp(from_ms, "%Y-%m-%d %H:%M UTC")
    )
}

fn stamp(ms: u64, format: &str) -> String {
    i64::try_from(ms)
        .ok()
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map_or_else(|| ms.to_string(), |t| t.format(format).to_string())
}

/// Who runs a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    Owner,
    /// A supervisor session, current or past, of this project.
    Supervisor(String),
}

impl Actor {
    /// What a record stamps for this actor: `None` for the owner.
    #[must_use]
    pub fn by(&self) -> Option<String> {
        match self {
            Self::Owner => None,
            Self::Supervisor(_) => Some(BY_SUPERVISOR.to_owned()),
        }
    }
}

/// The actor of this process: `SWITCHBOARD_RECORD_ID` matched against
/// every project's supervisors.
pub fn actor(data: &DataDir) -> Result<Actor> {
    let id = std::env::var("SWITCHBOARD_RECORD_ID").ok();
    actor_of(data, id.as_deref())
}

/// The actor for a Switchboard record id; no id, or one no project's
/// supervisor ever had, is the owner.
pub fn actor_of(data: &DataDir, record: Option<&str>) -> Result<Actor> {
    let Some(record) = record.filter(|r| !r.is_empty()) else {
        return Ok(Actor::Owner);
    };
    let dir = data.root.join("projects");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(Actor::Owner);
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    for path in files {
        match read_project(&path) {
            Ok(ps) if ps.supervisor.knows(record) => return Ok(Actor::Supervisor(ps.name)),
            Ok(_) => {}
            Err(e) => log::warn!("{}: {e:#}", path.display()),
        }
    }
    Ok(Actor::Owner)
}

/// What a supervisor may do with a verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// Read-only: anything.
    Allowed,
    /// Its first word is a project: the supervisor's own only.
    OwnProject,
    /// Its first word is a ticket: one of the supervisor's own project.
    OwnTicket,
    /// Never.
    Refused,
    /// `resume`: own tickets, and with reruns only when `rerun` is in
    /// `decides`.
    Resume,
    /// `worktrees`: the plain read only.
    Worktrees,
    /// `supervisor`: the plain read only.
    Supervisor,
    /// `runner`: only when the table's `may` lists `runner`.
    Runner,
    /// `restart`: own tickets, only when the table's `may` lists
    /// `restart`.
    Restart,
}

/// Every verb and what a supervisor may do with it. A verb not here is
/// refused; a test holds this to `USAGE`.
pub const SUPERVISOR_VERBS: &[(&str, Rule)] = &[
    ("take", Rule::OwnProject),
    ("queue", Rule::OwnProject),
    ("decide", Rule::OwnTicket),
    ("park", Rule::OwnTicket),
    ("close", Rule::OwnTicket),
    ("resume", Rule::Resume),
    ("restart", Rule::Restart),
    ("run", Rule::Refused),
    ("runner", Rule::Runner),
    ("worktrees", Rule::Worktrees),
    ("supervisor", Rule::Supervisor),
    ("decisions", Rule::Allowed),
    ("status", Rule::Allowed),
    ("events", Rule::Allowed),
    ("wait", Rule::Allowed),
    ("show", Rule::Allowed),
    ("report", Rule::Allowed),
    ("tail", Rule::Allowed),
    ("health", Rule::Allowed),
    ("brief", Rule::Allowed),
];

/// The rule for `verb`, if it has one.
#[must_use]
pub fn rule(verb: &str) -> Option<Rule> {
    SUPERVISOR_VERBS
        .iter()
        .find(|(v, _)| *v == verb)
        .map(|(_, r)| *r)
}

/// Where the owner answers a decision the supervisor may not: said by
/// the refusal and by the seed, so the two never drift.
pub const OWNER_ROUTES: &str = "with the decision's buttons on the ticket page \
    in Switchboard, or with `dispatch decide` from any shell but a supervisor's \
    pane (a Switchboard shell session or another terminal)";

/// The decisions a project's live `[supervisor]` table lets its
/// supervisor answer; empty without one.
#[must_use]
pub fn decides(data: &DataDir, project: &str) -> Vec<String> {
    live_pipeline(data, project)
        .ok()
        .and_then(|p| p.supervisor)
        .map(|s| s.decides)
        .unwrap_or_default()
}

/// The capabilities a project's live `[supervisor]` table gives its
/// supervisor beyond decisions; empty without one.
#[must_use]
pub fn may(data: &DataDir, project: &str) -> Vec<String> {
    live_pipeline(data, project)
        .ok()
        .and_then(|p| p.supervisor)
        .map(|s| s.may)
        .unwrap_or_default()
}

/// Every capability a `[supervisor]` table's `may` can name.
pub const CAPABILITIES: &[&str] = &["runner", "restart"];

/// A command refused because the supervisor's table does not give it
/// the capability. Unlike the other refusals it is saved on the
/// project's record, so the owner sees it was tried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRefused {
    /// The supervisor's project.
    pub project: String,
    /// The capability the command needs, which is also its verb after
    /// `dispatch`: `runner` or `restart`.
    pub capability: String,
    /// What the command asks of it: `restart`, or `<ticket> [<stage>]`.
    pub action: String,
}

impl CapabilityRefused {
    /// The command as typed after `dispatch`: `runner restart` or
    /// `restart 314cb7a1 ready`.
    #[must_use]
    pub fn command(&self) -> String {
        format!("{} {}", self.capability, self.action)
    }
}

impl std::fmt::Display for CapabilityRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the supervisor may not run `dispatch {}` unless its table's `may` lists `{}`; \
             the owner does",
            self.command(),
            self.capability
        )
    }
}

impl std::error::Error for CapabilityRefused {}

/// The project's live pipeline, read and parsed.
pub fn live_pipeline(data: &DataDir, project: &str) -> Result<Pipeline> {
    let path = data.pipeline(project);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("no pipeline for {project} at {}", path.display()))?;
    Pipeline::parse(&text)
}

/// Whether `actor` may run `args`; the owner always may. An error is the
/// refusal, said to the supervisor.
pub fn permit(actor: &Actor, args: &[&str], data: &DataDir) -> Result<()> {
    let Actor::Supervisor(own) = actor else {
        return Ok(());
    };
    let Some(&verb) = args.first() else {
        return Ok(());
    };
    let refused = |form: &str| {
        anyhow::anyhow!("the supervisor may not run `dispatch {form}`; the owner does")
    };
    let own_project = |project: &str| -> Result<()> {
        if project == own {
            Ok(())
        } else {
            bail!("the supervisor of {own} may not act on {project}")
        }
    };
    // A ticket that cannot be read is left to the command to report.
    let own_ticket = |ticket: &str| -> Result<()> {
        match read_ticket(&data.ticket_file(ticket)) {
            Ok(t) if t.project != *own => bail!(
                "the supervisor of {own} may not act on {}'s tickets",
                t.project
            ),
            _ => Ok(()),
        }
    };
    match rule(verb) {
        None | Some(Rule::Refused) => Err(refused(verb)),
        Some(Rule::Allowed) => Ok(()),
        Some(Rule::OwnProject) => args.get(1).map_or(Ok(()), |p| own_project(p)),
        Some(Rule::OwnTicket) => args.get(1).map_or(Ok(()), |t| own_ticket(t)),
        Some(Rule::Resume) => {
            if let Some(t) = args.get(1) {
                own_ticket(t)?;
            }
            if args.contains(&"--no-rerun") || decides(data, own).iter().any(|d| d == "rerun") {
                Ok(())
            } else {
                bail!("the supervisor may not resume with reruns; `--no-rerun`, or the owner does")
            }
        }
        Some(Rule::Runner) => match args {
            [capability, action @ ("stop" | "start" | "restart")]
                if !may(data, own).iter().any(|m| m == capability) =>
            {
                Err(CapabilityRefused {
                    project: own.clone(),
                    capability: (*capability).to_owned(),
                    action: (*action).to_owned(),
                }
                .into())
            }
            // Anything else is allowed or a usage error, which the
            // command reports without a refusal to save.
            _ => Ok(()),
        },
        Some(Rule::Restart) => match args {
            [capability, ticket, stage @ ..] if stage.len() <= 1 => {
                own_ticket(ticket)?;
                if may(data, own).iter().any(|m| m == capability) {
                    Ok(())
                } else {
                    Err(CapabilityRefused {
                        project: own.clone(),
                        capability: (*capability).to_owned(),
                        action: args[1..].join(" "),
                    }
                    .into())
                }
            }
            // A malformed restart is the usage error's, never saved.
            _ => Ok(()),
        },
        Some(Rule::Worktrees) if args.len() == 1 => Ok(()),
        Some(Rule::Supervisor) if args.len() == 2 => Ok(()),
        Some(Rule::Worktrees | Rule::Supervisor) => Err(refused(&args.join(" "))),
    }
}

/// What `dispatch supervisor` shows of a project's supervisor.
#[derive(Debug)]
pub struct Standing {
    /// The project's record, with its `supervisor`.
    pub ps: ProjectState,
    /// The live table, when the pipeline has one.
    pub table: Option<Supervisor>,
    /// The current session's seed no longer matches the table.
    pub stale: bool,
    /// Where the supervisor works: the recorded workspace, else where
    /// the first fresh would set it up.
    pub workspace: PathBuf,
    /// The hand-off file the session keeps.
    pub handoff: PathBuf,
}

impl Runner {
    /// Where a project's supervisor works unless its record says: the
    /// root a ticket's trees use, under `supervisor-<project>`.
    #[must_use]
    pub fn supervisor_workspace(&self, p: &Pipeline) -> PathBuf {
        self.worktree_root(p)
            .join(format!("supervisor-{}", p.project.name))
    }

    /// The live pipeline of `project`, which must have a `[supervisor]`.
    fn supervised(&self, project: &str) -> Result<(Pipeline, Supervisor)> {
        let p = live_pipeline(&self.data, project)?;
        let Some(sup) = p.supervisor.clone() else {
            bail!("no [supervisor] table in pipelines/{project}.toml");
        };
        Ok((p, sup))
    }

    /// The project's supervisor as `dispatch supervisor` shows it.
    pub fn supervisor_standing(&self, project: &str) -> Result<Standing> {
        let ps = self.load_project(project)?;
        let p = live_pipeline(&self.data, project)?;
        let stale = p
            .supervisor
            .as_ref()
            .is_some_and(|sup| ps.supervisor.seed_stale(project, sup));
        let workspace = ps
            .supervisor
            .workspace
            .clone()
            .unwrap_or_else(|| self.supervisor_workspace(&p));
        let table = p.supervisor;
        Ok(Standing {
            handoff: self.data.supervisor_dir(project).join("handoff.md"),
            ps,
            table,
            stale,
            workspace,
        })
    }

    /// The port's `supervisor-fresh`: the intent written for the
    /// runner's next pass, since the work asks the caller's own control
    /// socket.
    pub fn request_supervisor_fresh(&mut self, project: &str) -> Result<ProjectState> {
        self.supervised(project)?;
        self.transaction(|r| {
            let mut ps = r.load_project(project)?;
            ps.supervisor.intent = Some(SupervisorIntent::Fresh);
            r.save_project(&ps)?;
            Ok(ps)
        })
    }

    /// Carry out a project's intent, cleared first so a failing one is
    /// not tried again every second; the failure is the record's `error`.
    pub(crate) fn supervisor_intent(&mut self, project: &str, now_ms: u64) -> Result<()> {
        let intent = self.transaction(|r| {
            let mut ps = r.load_project(project)?;
            let intent = ps.supervisor.intent.take();
            if intent.is_some() {
                r.save_project(&ps)?;
            }
            Ok(intent)
        })?;
        match intent {
            Some(SupervisorIntent::Fresh) => {
                self.supervisor_fresh(project, false, "fresh", now_ms)?;
            }
            None => {}
        }
        Ok(())
    }

    /// A new supervisor session, replacing the current one. The workspace
    /// is set up first, so a failed setup leaves the current session as
    /// it was.
    pub fn supervisor_fresh(
        &mut self,
        project: &str,
        setup: bool,
        why: &str,
        now_ms: u64,
    ) -> Result<SupervisorRecord> {
        self.transaction(|r| {
            let result = r.fresh_locked(project, setup, why, now_ms);
            r.note_error(project, result.as_ref().err())?;
            result
        })
    }

    /// Kill the current session and keep it in `past` with `kill: <why>`.
    pub fn supervisor_kill(&mut self, project: &str, why: &str, now_ms: u64) -> Result<()> {
        self.transaction(|r| {
            let result = r.kill_locked(project, why, now_ms);
            r.note_error(project, result.as_ref().err())?;
            result
        })
    }

    /// Resume the current session's conversation through Switchboard. It
    /// never starts a fresh one: Switchboard refuses one it cannot resume.
    pub fn supervisor_resume(&mut self, project: &str) -> Result<String> {
        self.transaction(|r| {
            let result = r.resume_locked(project);
            r.note_error(project, result.as_ref().err())?;
            result
        })
    }

    /// The last failure on the record, or none after a success.
    fn note_error(&mut self, project: &str, e: Option<&anyhow::Error>) -> Result<()> {
        let mut ps = self.load_project(project)?;
        let error = e.map(|e| format!("{e:#}"));
        if ps.supervisor.error != error {
            ps.supervisor.error = error;
            self.save_project(&ps)?;
        }
        Ok(())
    }

    fn fresh_locked(
        &mut self,
        project: &str,
        setup: bool,
        why: &str,
        now_ms: u64,
    ) -> Result<SupervisorRecord> {
        let (p, sup) = self.supervised(project)?;
        let mut ps = self.load_project(project)?;
        if ps.supervisor.op.as_ref().is_some_and(Operation::unresolved) {
            self.recover_supervisor(&mut ps)?;
            if ps.supervisor.op.as_ref().is_some_and(Operation::unresolved) {
                bail!("a request for {project}'s supervisor is still in flight; try again shortly");
            }
        }
        let workspace = ps
            .supervisor
            .workspace
            .clone()
            .unwrap_or_else(|| self.supervisor_workspace(&p));
        if let Some(why) = shell_unsafe(&workspace) {
            bail!("supervisor workspace: {why}; set another with `dispatch worktrees <path>`");
        }
        if setup || !workspace.exists() {
            self.set_up(&p, &sup, &workspace)?;
        }
        if ps.supervisor.workspace.is_none() {
            ps.supervisor.workspace = Some(workspace.clone());
            self.save_project(&ps)?;
        }
        let dir = self.data.supervisor_dir(project);
        let exe = std::env::current_exe().context("the dispatch executable's path")?;
        let seed_path = dir.join("seed.md");
        let handoff = dir.join("handoff.md");
        atomic_write(
            &seed_path,
            seed(project, &sup, &exe, &handoff, &workspace).as_bytes(),
        )?;
        if ps.supervisor.current.is_some() {
            self.retire(&mut ps, why, now_ms)?;
        }
        let hash = seed_hash(project, &sup);
        let flags = launch_flags(&sup, &exe, &dir);
        let name = format!("Supervisor · {project}");
        let space = self.supervisor_space(&mut ps, &p, now_ms)?;
        let mut session = None;
        // A Switchboard project removed by hand is made again, once.
        for _ in 0..2 {
            let sb_project = self.supervisor_project(&mut ps, &space, &name, &workspace, now_ms)?;
            let reply = self.send_supervisor(
                &mut ps,
                &format!("session:{hash}"),
                Body::SessionNew {
                    project: sb_project,
                    name: name.clone(),
                    session_kind: wire::SessionKind::Claude,
                    cwd: workspace.clone(),
                    launch: wire::Launch::Argv(flags.clone()),
                    prompt: Some(first_prompt(&seed_path)),
                    notes: format!("Dispatch supervisor of {project}"),
                    env: BTreeMap::new(),
                    env_sets: Vec::new(),
                },
                now_ms,
            )?;
            match reply {
                Reply::Failed { reason }
                    if reason.contains("unknown project") || reason.contains("no such project") =>
                {
                    ps.supervisor.project = None;
                    self.save_project(&ps)?;
                }
                Reply::Failed { reason } => bail!("session.new: {reason}"),
                _ => {
                    session.clone_from(&ps.supervisor.current);
                    break;
                }
            }
        }
        let current = session.context("session.new made no session")?;
        log::info!("{project}: supervisor {} started", current.session);
        Ok(current)
    }

    /// The Switchboard project the supervisor lives in, rooted at the
    /// workspace: made once and reused.
    fn supervisor_project(
        &mut self,
        ps: &mut ProjectState,
        space: &str,
        name: &str,
        workspace: &Path,
        now_ms: u64,
    ) -> Result<String> {
        if let Some(id) = &ps.supervisor.project {
            return Ok(id.clone());
        }
        let reply = self.send_supervisor(
            ps,
            "project",
            Body::ProjectAdd {
                space: space.to_owned(),
                name: name.to_owned(),
                root: workspace.to_path_buf(),
            },
            now_ms,
        )?;
        ps.supervisor
            .project
            .clone()
            .with_context(|| format!("project.add: {reply:?}"))
    }

    /// The workspace made and its setup run: the table's argvs, else a
    /// clone of the project's repository, else nothing. A directory this
    /// call made is removed again when a command fails.
    fn set_up(&mut self, p: &Pipeline, sup: &Supervisor, workspace: &Path) -> Result<()> {
        let created = !workspace.exists();
        std::fs::create_dir_all(workspace)
            .with_context(|| format!("create {}", workspace.display()))?;
        let mut argvs = sup.setup.argvs();
        if argvs.is_empty()
            && let Some(repo) = &p.project.repo
        {
            argvs.push(vec!["git".into(), "clone".into(), repo.clone(), ".".into()]);
        }
        for argv in argvs {
            if let Err(e) = self.git.run(workspace, &argv, &[]) {
                if created {
                    let _ = std::fs::remove_dir_all(workspace);
                }
                bail!("supervisor setup `{}` failed: {e:#}", argv.join(" "));
            }
        }
        Ok(())
    }

    /// The current session killed and moved to `past`, and the hand-off
    /// rotated under a heading naming it.
    fn retire(&mut self, ps: &mut ProjectState, why: &str, now_ms: u64) -> Result<()> {
        let Some(current) = ps.supervisor.current.take() else {
            return Ok(());
        };
        self.kill_session(&current.session)?;
        ps.supervisor.past.push(PastSupervisor {
            session: current.session.clone(),
            seed_hash: current.seed_hash.clone(),
            created_ms: current.created_ms,
            replaced_ms: now_ms,
            why: why.to_owned(),
        });
        self.save_project(ps)?;
        let dir = self.data.supervisor_dir(&ps.name);
        let handoff = dir.join("handoff.md");
        if let Ok(old) = std::fs::read_to_string(&handoff) {
            let kept = dir.join(format!("handoff.{}.md", stamp(now_ms, "%Y%m%d-%H%M%S")));
            std::fs::rename(&handoff, &kept).with_context(|| format!("keep {}", kept.display()))?;
            atomic_write(
                &handoff,
                rotated_handoff(&old, current.created_ms).as_bytes(),
            )?;
        }
        Ok(())
    }

    /// `session.kill`, idempotent: a session already gone is killed.
    fn kill_session(&mut self, session: &str) -> Result<()> {
        let request = Request::new(
            format!("sup-kill-{}", uuid::Uuid::new_v4().simple()),
            Body::SessionKill {
                session: session.to_owned(),
            },
        );
        self.call(None, &request)
            .map_err(SocketDown::from)
            .context("session.kill: the control socket failed")?;
        Ok(())
    }

    fn kill_locked(&mut self, project: &str, why: &str, now_ms: u64) -> Result<()> {
        let mut ps = self.load_project(project)?;
        if ps.supervisor.current.is_none() {
            bail!("{project} has no supervisor to kill");
        }
        self.retire(&mut ps, &format!("kill: {why}"), now_ms)
    }

    fn resume_locked(&mut self, project: &str) -> Result<String> {
        let ps = self.load_project(project)?;
        let Some(current) = ps.supervisor.current.clone() else {
            bail!(
                "{project} has no supervisor; `dispatch supervisor {project} --fresh` starts one"
            );
        };
        if !ps.supervisor.workspace.as_ref().is_some_and(|w| w.exists()) {
            bail!(
                "{project}'s supervisor workspace is missing; `dispatch supervisor {project} --fresh` sets it up"
            );
        }
        let request = Request::new(
            format!("sup-resume-{}", uuid::Uuid::new_v4().simple()),
            Body::SessionResume {
                session: current.session.clone(),
            },
        );
        let reply = self
            .call(None, &request)
            .map_err(SocketDown::from)
            .context("session.resume: the control socket failed")?;
        if let Reply::Failed { reason } = reply {
            bail!("session.resume: {reason}");
        }
        Ok(current.session)
    }

    /// The Switchboard workspace the pipeline names, found or made
    /// through the supervisor's own ledger.
    fn supervisor_space(
        &mut self,
        ps: &mut ProjectState,
        p: &Pipeline,
        now_ms: u64,
    ) -> Result<String> {
        if let Some(space) = self.known_space(ps, p)? {
            return Ok(space);
        }
        let reply = self.send_supervisor(
            ps,
            "space",
            Body::SpaceNew {
                name: p.project.space.clone(),
            },
            now_ms,
        )?;
        ps.space
            .clone()
            .with_context(|| format!("space.new: {reply:?}"))
    }

    /// One creation for the supervisor, written to the project's record
    /// before it is sent and its reply after, as a ticket's ledger is.
    fn send_supervisor(
        &mut self,
        ps: &mut ProjectState,
        intent: &str,
        body: Body,
        now_ms: u64,
    ) -> Result<Reply> {
        let op = format!(
            "sup-{}-{}",
            ps.name,
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        ps.supervisor.op = Some(Operation::new(op.clone(), &body, None, intent, now_ms));
        self.save_project(ps)?;
        let result = self.call(None, &Request::new(op, body));
        if let Some(entry) = ps.supervisor.op.as_mut() {
            match &result {
                Ok(reply) => entry.reply = Some(reply.clone()),
                Err(e) => entry.error = Some(e.to_string()),
            }
        }
        if let Ok(reply) = &result {
            apply_supervisor_reply(ps, reply);
        }
        self.save_project(ps)?;
        result
            .map_err(SocketDown::from)
            .with_context(|| format!("{intent}: the control socket failed"))
    }
}

/// A supervisor request's reply applied to the project's record by the
/// op's intent: the space, the Switchboard project, or the session as
/// `current`, which also settles a pending fresh intent.
pub(crate) fn apply_supervisor_reply(ps: &mut ProjectState, reply: &Reply) {
    let Some(op) = ps.supervisor.op.clone() else {
        return;
    };
    let made = reply.made();
    let first = |kind: wire::RecordKind| made.iter().find(|m| m.kind == kind).map(|m| m.id.clone());
    match op.intent.as_str() {
        "space" => {
            if let Some(id) = first(wire::RecordKind::Space) {
                ps.space = Some(id);
            }
        }
        "project" => {
            if let Some(id) = first(wire::RecordKind::Project) {
                ps.supervisor.project = Some(id);
            }
        }
        intent => {
            if let Some(hash) = intent.strip_prefix("session:")
                && let Some(id) = first(wire::RecordKind::Session)
            {
                ps.supervisor.current = Some(SupervisorRecord {
                    session: id,
                    seed_hash: hash.to_owned(),
                    created_ms: op.sent_ms,
                    model: model_of(op.body.as_ref()),
                });
                // A fresh asked for through the port is this one: the
                // runner must not replace it on its next pass.
                ps.supervisor.intent = None;
                ps.supervisor.error = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::SupervisorSetup;

    fn table() -> Supervisor {
        Supervisor {
            guidance: "Keep the queue moving.".into(),
            read: vec!["CLAUDE.md".into(), "docs/design.md".into()],
            setup: SupervisorSetup::default(),
            model: Some("haiku".into()),
            decides: vec!["finalize".into(), "rerun".into()],
            merges: false,
            may: Vec::new(),
        }
    }

    fn seed_of(sup: &Supervisor) -> String {
        seed(
            "orchard",
            sup,
            Path::new("/opt/bin/dispatch"),
            Path::new("/data/projects/orchard/supervisor/handoff.md"),
            Path::new("/trees/supervisor-orchard"),
        )
    }

    #[test]
    fn a_supervisor_that_may_use_the_runner_is_told_to_restart_it_after_a_merge() {
        let mut sup = table();
        let before = seed_of(&sup);
        assert!(before.contains("run the runner"));
        assert!(!before.contains("## The runner"));
        // A table that never had the key reads the same as one with it empty.
        let parsed: Supervisor = toml::from_str(&toml::to_string(&sup).unwrap()).unwrap();
        assert_eq!(seed_of(&parsed), before);
        assert_eq!(seed_hash("orchard", &parsed), seed_hash("orchard", &sup));
        sup.may = vec!["runner".into()];
        let s = seed_of(&sup);
        assert!(s.contains("`/opt/bin/dispatch runner restart`"), "{s}");
        assert!(s.contains("start a runner with `dispatch run`"), "{s}");
        assert!(!s.contains("run the runner"), "{s}");
        assert!(s.find("## The runner") < s.find("## Commands"));
        assert_ne!(seed_hash("orchard", &sup), seed_hash("orchard", &table()));
    }

    #[test]
    fn a_supervisor_that_may_restart_is_told_when() {
        let mut sup = table();
        let before = seed_of(&sup);
        assert!(before.contains("You may not restart a ticket"), "{before}");
        assert!(!before.contains("## Restarts"));
        sup.may = vec!["restart".into()];
        let s = seed_of(&sup);
        assert!(
            s.contains("`/opt/bin/dispatch restart <ticket> [<stage>]`"),
            "{s}"
        );
        assert!(s.contains("say what you restarted"), "{s}");
        assert!(!s.contains("You may not restart a ticket"), "{s}");
        assert!(s.contains("You may not move the worktrees"), "{s}");
        assert!(s.find("## Restarts") < s.find("## Commands"));
        assert_ne!(seed_hash("orchard", &sup), seed_hash("orchard", &table()));
    }

    #[test]
    fn the_runner_is_refused_to_a_supervisor_unless_its_table_says_may() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let write = |may: &str| {
            let text = format!(
                "version = 1\n\n[project]\nname = \"orchard\"\nroot = \"/r\"\n\
                 space = \"Dispatch\"\n\n[source]\nkind = \"github\"\nrepo = \"o/r\"\n\
                 label = \"dispatch\"\n\n[[lanes]]\nname = \"repo\"\npath = \".\"\n\n\
                 [[stages]]\nname = \"inspect\"\n\
                 gate = {{ kind = \"human\", decision = \"inspect\" }}\n\n\
                 [supervisor]\nguidance = \"g\"\n{may}\n"
            );
            std::fs::create_dir_all(dir.path().join("pipelines")).unwrap();
            std::fs::write(data.pipeline("orchard"), text).unwrap();
        };
        let sup = Actor::Supervisor("orchard".into());
        let args = ["runner", "restart"];
        write("");
        let e = permit(&sup, &args, &data).unwrap_err();
        let refused = e
            .downcast_ref::<CapabilityRefused>()
            .expect("a capability refusal");
        assert_eq!(refused.command(), "runner restart");
        assert_eq!(
            e.to_string(),
            "the supervisor may not run `dispatch runner restart` unless its table's `may` \
             lists `runner`; the owner does"
        );
        // A malformed command is left to the usage error, never saved.
        permit(&sup, &["runner"], &data).unwrap();
        permit(&sup, &["runner", "bogus"], &data).unwrap();
        permit(&Actor::Owner, &args, &data).unwrap();
        write("may = [\"runner\"]");
        assert_eq!(may(&data, "orchard"), ["runner"]);
        permit(&sup, &args, &data).unwrap();
        // `dispatch run` stays the owner's whatever the table says.
        assert!(permit(&sup, &["run"], &data).is_err());
    }

    #[test]
    fn the_seed_holds_the_guidance_the_reads_the_decisions_and_the_paths() {
        let s = seed(
            "orchard",
            &table(),
            Path::new("/opt/bin/dispatch"),
            Path::new("/data/projects/orchard/supervisor/handoff.md"),
            Path::new("/trees/supervisor-orchard"),
        );
        for want in [
            "Keep the queue moving.",
            "/trees/supervisor-orchard/CLAUDE.md",
            "/trees/supervisor-orchard/docs/design.md",
            "- `finalize`: `finalize` ends a plan review",
            "- `rerun`: `rerun` runs a failed",
            "Every other decision is the owner's: say so and move on.",
            OWNER_ROUTES,
            "a `!` line included, runs as you",
            "never change your environment",
            "answered `recheck` or `park`",
            "run `/opt/bin/dispatch brief orchard` first",
            "/data/projects/orchard/supervisor/handoff.md current",
            "`/opt/bin/dispatch events --project <project>",
            "`/opt/bin/dispatch wait <ticket> --for move --since <seq> --timeout 540`",
            "from the seq of the last event line it printed",
            "same seq after a timeout",
            "from the `follow from seq` that brief printed, never from a seq in the hand-off",
            "You never merge a pull request.",
            "you take no initiative",
            "Watching never blocks this session.",
            "Write it with your Edit or Write",
        ] {
            assert!(s.contains(want), "the seed lacks {want:?}:\n{s}");
        }
        assert!(!s.contains("{exe}"));
        // Naming the variable would show the supervisor how to pass as the owner.
        assert!(!s.contains("SWITCHBOARD_RECORD_ID"));
        assert!(!s.contains("--timeout 100"));
        assert!(!s.contains("--for any --timeout 540"));
        assert!(!s.contains("--for any --since"));
    }

    #[test]
    fn a_merging_supervisor_is_told_to_merge_and_allowed_gh_pr() {
        let mut sup = table();
        sup.merges = true;
        let s = seed(
            "orchard",
            &sup,
            Path::new("/opt/bin/dispatch"),
            Path::new("/data/projects/orchard/supervisor/handoff.md"),
            Path::new("/trees/supervisor-orchard"),
        );
        assert!(s.contains("`gh pr merge <n> --merge`"), "{s}");
        assert!(!s.contains("You never merge a pull request."));
        assert!(s.contains("answered `recheck` or `park`"));
        let flags = launch_flags(&sup, Path::new("/opt/bin/dispatch"), Path::new("/d/s"));
        for rule in MERGE_RULES {
            assert!(flags.contains(&(*rule).to_owned()), "missing {rule}");
        }
        assert_ne!(seed_hash("orchard", &sup), seed_hash("orchard", &table()));
    }

    #[test]
    fn the_essentials_name_only_verbs_in_the_usage() {
        let verbs: Vec<&str> = crate::USAGE
            .lines()
            .filter_map(|l| l.trim_start().strip_prefix("dispatch "))
            .filter_map(|rest| rest.split_whitespace().next())
            .collect();
        let named: Vec<&str> = GUIDE_ESSENTIALS
            .split("{exe} ")
            .skip(1)
            .filter_map(|rest| rest.split_whitespace().next())
            .collect();
        assert!(named.len() >= 6, "{named:?}");
        for verb in named {
            assert!(verbs.contains(&verb), "`{verb}` is not in the usage");
        }
    }

    #[test]
    fn every_verb_in_the_usage_has_a_rule_for_a_supervisor() {
        for line in crate::USAGE.lines() {
            let Some(rest) = line.trim_start().strip_prefix("dispatch ") else {
                continue;
            };
            let verb = rest.split_whitespace().next().unwrap();
            assert!(rule(verb).is_some(), "place `{verb}` in SUPERVISOR_VERBS");
        }
        assert_eq!(rule("frob"), None);
    }

    #[test]
    fn the_launch_flags_allow_dispatch_and_the_supervisor_directory_only() {
        let flags = launch_flags(&table(), Path::new("/opt/bin/dispatch"), Path::new("/d/s"));
        assert_eq!(
            flags,
            [
                "--model",
                "haiku",
                "--allowedTools",
                "Bash(/opt/bin/dispatch:*)",
                "--allowedTools",
                "Read(///d/s/**)",
                "--allowedTools",
                "Edit(///d/s/**)",
            ]
        );
        assert!(!flags.iter().any(|f| f.contains("--settings")));
    }

    #[test]
    fn the_hash_follows_the_owners_inputs_and_not_the_paths() {
        let base = seed_hash("orchard", &table());
        let mut guidance = table();
        guidance.guidance.push_str(" Quietly.");
        let mut decides = table();
        decides.decides.push("pr".into());
        assert_ne!(seed_hash("orchard", &guidance), base);
        assert_ne!(seed_hash("orchard", &decides), base);
        assert_ne!(seed_hash("grove", &table()), base);
        // The rendered seeds differ by path; the hash does not see them.
        let a = seed(
            "orchard",
            &table(),
            Path::new("/a/dispatch"),
            Path::new("/a/h.md"),
            Path::new("/a/w"),
        );
        let b = seed(
            "orchard",
            &table(),
            Path::new("/b/dispatch"),
            Path::new("/b/h.md"),
            Path::new("/b/w"),
        );
        assert_ne!(a, b);
        assert_eq!(seed_hash("orchard", &table()), base);
    }

    #[test]
    fn the_hash_covers_the_guide_essentials() {
        let table = toml::to_string(&table()).unwrap();
        let with = Pipeline::fingerprint(&format!("{table}\norchard\n{GUIDE_ESSENTIALS}"));
        let without = Pipeline::fingerprint(&format!("{table}\norchard\n"));
        assert_eq!(with, seed_hash("orchard", &super::tests::table()));
        assert_ne!(with, without);
    }

    #[test]
    fn a_rotated_handoff_opens_with_the_sessions_date() {
        let text = rotated_handoff("watching #12\n", 1_759_000_000_000);
        assert!(
            text.starts_with("## From the session of 2025-09-27"),
            "{text}"
        );
        assert!(text.ends_with("watching #12\n"));
    }

    #[test]
    fn rotating_a_rotated_handoff_keeps_one_heading() {
        let mut text = "# Hand-off\n\nwatching #12\n".to_owned();
        for ms in [1_759_000_000_000, 1_759_100_000_000, 1_759_200_000_000] {
            text = rotated_handoff(&text, ms);
        }
        assert_eq!(text.matches(ROTATED_HEADING).count(), 1, "{text}");
        assert!(
            text.starts_with("## From the session of 2025-09-30"),
            "{text}"
        );
        assert!(text.ends_with("\n\n# Hand-off\n\nwatching #12\n"), "{text}");
    }

    #[test]
    fn a_session_heading_inside_the_body_is_kept() {
        let old = "# Hand-off\n\n## From the session of 2025-09-01 10:00 UTC\nnotes\n";
        let text = rotated_handoff(old, 1_759_000_000_000);
        assert_eq!(text.matches(ROTATED_HEADING).count(), 2, "{text}");
        assert!(text.ends_with(old), "{text}");
    }

    #[test]
    fn an_empty_handoff_rotates_without_a_heading() {
        assert!(!rotated_handoff("\n\n", 1_759_000_000_000).contains(ROTATED_HEADING));
        assert!(!rotated_handoff("", 1_759_000_000_000).contains(ROTATED_HEADING));
        let only = "## From the session of 2025-09-01 10:00 UTC\n\n";
        assert!(!rotated_handoff(only, 1_759_000_000_000).contains(ROTATED_HEADING));
    }
}
