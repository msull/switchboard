//! The pipeline file: one TOML document per project, the whole schema of
//! `docs/dispatch.md` parsed and checked here, including the parts the
//! runner parks on rather than runs. A ticket runs from the copy taken
//! when it was made; the project's file only shapes tickets taken
//! afterwards.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

use crate::history::Commits;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pipeline {
    pub version: u32,
    pub project: ProjectSection,
    pub source: Source,
    #[serde(default)]
    pub lanes: Vec<Lane>,
    #[serde(default)]
    pub resources: Vec<Resource>,
    #[serde(default)]
    pub operators: BTreeMap<String, Operator>,
    #[serde(default)]
    pub stages: Vec<Stage>,
    #[serde(default)]
    pub policy: Policy,
    /// The project's supervisor session, when it has one. Read only from
    /// the live `pipelines/<project>.toml`, never from a ticket's copy.
    #[serde(default)]
    pub supervisor: Option<Supervisor>,
}

/// `[supervisor]`: one long-lived agent per project that watches its
/// tickets and answers the decisions `decides` lists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Supervisor {
    /// What the supervisor is for, in the owner's words; the seed opens
    /// with it.
    pub guidance: String,
    /// Files to read first, relative to the workspace.
    #[serde(default)]
    pub read: Vec<PathBuf>,
    /// What makes the workspace: one argv, or `[[supervisor.setup]]`
    /// entries for several. Absent, `git clone <repo> .` for a project
    /// with a `repo`, an empty directory for one with a `root`.
    #[serde(default)]
    pub setup: SupervisorSetup,
    /// `--model` for the session.
    #[serde(default)]
    pub model: Option<String>,
    /// The decisions the supervisor may answer; every other is the
    /// owner's.
    #[serde(default)]
    pub decides: Vec<String>,
    /// Whether the supervisor merges a ticket's pull request itself once
    /// its checks pass and its body is clean. True gives the session
    /// allow rules for `gh pr` and `git pull`; false (the default) has
    /// it report a green pull request and stop, so the owner merges.
    #[serde(default)]
    pub merges: bool,
    /// Capabilities beyond decisions that the supervisor may use; every
    /// other is the owner's. Today only `runner`: stop, start and
    /// restart the runner the app runs. Not written when empty, so a
    /// table without it hashes into the seed as it did before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub may: Vec<String>,
}

/// `setup = [...]` (one argv, the shape of a lane's `setup`) or
/// `[[supervisor.setup]] argv = [...]` (several).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SupervisorSetup {
    One(Vec<String>),
    Many(Vec<SetupEntry>),
}

impl Default for SupervisorSetup {
    fn default() -> Self {
        Self::One(Vec::new())
    }
}

/// One command of a supervisor's setup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetupEntry {
    pub argv: Vec<String>,
}

impl SupervisorSetup {
    /// Every argv in order; empty when the table names none.
    #[must_use]
    pub fn argvs(&self) -> Vec<Vec<String>> {
        match self {
            Self::One(argv) if argv.is_empty() => Vec::new(),
            Self::One(argv) => vec![argv.clone()],
            Self::Many(entries) => entries.iter().map(|e| e.argv.clone()).collect(),
        }
    }
}

/// Answers that are easily mistaken for decisions in `decides`, with
/// the decisions that take them.
const ANSWERS: &[(&str, &str)] = &[
    ("recheck", "`pr` and `refresh`"),
    ("continue", "`paused`"),
    ("keep", "`rerun`"),
    ("check", "`rerun`"),
    ("proceed", "a human gate's decision"),
    ("reuse", "`branch`"),
    ("fresh", "`branch`"),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSection {
    pub name: String,
    /// The repository's URL. Dispatch keeps its own clone of it under
    /// its data directory, fetched before every cut, and every ticket
    /// works in a worktree of that clone; the user's checkout is never
    /// touched. One of `repo` and `root`.
    #[serde(default)]
    pub repo: Option<String>,
    /// A directory to work in as it is, with no branch and no clone:
    /// for a project that is not a repository. One of `repo` and `root`.
    #[serde(default)]
    pub root: Option<PathBuf>,
    /// The branch tickets branch from, in the clone's `remote`.
    #[serde(default = "default_base")]
    pub base: String,
    #[serde(default = "default_remote")]
    pub remote: String,
    /// Where tickets' trees go; absent, Dispatch's own `worktrees`
    /// directory.
    #[serde(default)]
    pub worktrees: Option<PathBuf>,
    /// The Switchboard workspace every ticket's project goes in.
    pub space: String,
    /// Other places the repository lives (a mirror), by remote name,
    /// where a pull request may be taken from: `dispatch take .. pr
    /// <name>:<lane>/<n>`.
    #[serde(default)]
    pub remotes: BTreeMap<String, String>,
}

fn default_base() -> String {
    "main".into()
}

fn default_remote() -> String {
    "origin".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Source {
    Github {
        repo: String,
        label: String,
        #[serde(default)]
        lane_hints: BTreeMap<String, String>,
    },
    TaskFile {
        path: PathBuf,
        marker: String,
    },
    Manual,
    /// Tickets are taken from pull requests named on the command line,
    /// one per lane; the pipeline reviews them and pushes nothing.
    PullRequest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lane {
    pub name: String,
    /// Where the lane lives inside the ticket's tree; `.` for a
    /// single-repository project.
    pub path: PathBuf,
    /// A repository of the lane's own (a workspace of several): Dispatch
    /// clones it too and cuts the lane as a worktree of that clone at
    /// `path` inside the ticket's tree, so the layout matches a checkout.
    #[serde(default)]
    pub repo: Option<String>,
    /// The branch this lane branches from; absent, the project's.
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub remote: Option<String>,
    /// Other places this lane's repository lives, by remote name; see
    /// the project's.
    #[serde(default)]
    pub remotes: BTreeMap<String, String>,
    /// Run once in a new worktree.
    #[serde(default)]
    pub setup: Vec<String>,
    #[serde(default)]
    pub serve: Option<Serve>,
    /// Paths this lane's setup, gates, checks and command reviewers may
    /// write besides the ticket's trees, when `[policy] confine` is on:
    /// the caches its tools fill (`~/.cargo`, `~/.cache/uv`, `~/.npm`).
    /// `~` is expanded.
    #[serde(default)]
    pub writable: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Serve {
    pub argv: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub url: String,
    pub ready: Ready,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ready {
    pub http: String,
    pub within_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resource {
    pub name: String,
    #[serde(default = "one")]
    pub count: u32,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OperatorKind {
    Claude,
    Codex,
    /// Not an agent: a fixed command from the pipeline file, run as a
    /// child of the runner. Only a code review stage's reviewer may be
    /// one (local tooling: a linter, a type checker).
    Command,
}

/// What each kind of agent needs from its launch. A new kind is one
/// variant here plus these answers; the scheduler asks, never matches.
impl OperatorKind {
    /// Whether a reviewer of this kind works in the ticket's tree, where
    /// the code is and where the tree's trust was already granted, or
    /// must sit in the attempt directory because it can only write
    /// there. Codex's sandbox writes inside its cwd alone.
    #[must_use]
    pub fn reviews_in_tree(self) -> bool {
        match self {
            Self::Claude | Self::Command => true,
            Self::Codex => false,
        }
    }

    /// Whether this kind is an agent in a Switchboard session at all.
    #[must_use]
    fn is_agent(self) -> bool {
        !matches!(self, Self::Command)
    }

    /// The flags that let this kind write into `dir` unasked when `dir`
    /// is outside its cwd. Claude Code writes elsewhere only under an
    /// allow rule for the path (an added directory still asks before
    /// creating a file); Codex has no such flag, so it is given `dir`
    /// as its cwd instead.
    #[must_use]
    pub fn write_flags(self, dir: &std::path::Path) -> Vec<String> {
        match self {
            Self::Claude => vec![
                "--allowedTools".into(),
                format!("Edit(//{}/**)", dir.display()),
            ],
            Self::Codex | Self::Command => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Operator {
    pub kind: OperatorKind,
    /// Extra flags for the agent's command line, such as a model.
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub guidance: String,
    #[serde(default)]
    pub budget_usd: f64,
    /// Present on a reviewer: a complete Switchboard workflow definition.
    #[serde(default)]
    pub review: Option<Review>,
    /// For `kind = "command"`: the fixed command, run in the lane (or
    /// `in = "root"`, the tree). Exit 0 is nothing to report, exit 1
    /// with output is findings, anything else is a failure.
    #[serde(default)]
    pub argv: Vec<String>,
    #[serde(rename = "in", default = "default_run_in")]
    pub run_in: String,
}

fn default_run_in() -> String {
    "lane".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Review {
    pub reviewer: OperatorKind,
    pub review_first: String,
    pub review_round: String,
    pub respond: String,
    pub respond_to_user: String,
    pub handoff: String,
    pub no_feedback: String,
    #[serde(default)]
    pub cap: Option<u32>,
}

/// Where a stage runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Context {
    /// The pipeline's root, before lanes exist or across them.
    #[default]
    Root,
    /// Once per lane the ticket cut.
    Each,
    /// Once in the root with every lane named.
    Joined,
    /// One named lane; skipped when the ticket did not cut it.
    Lane(String),
    /// These lanes only.
    Lanes(Vec<String>),
}

impl<'de> Deserialize<'de> for Context {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            One(String),
            Many(Vec<String>),
        }
        Ok(match Raw::deserialize(d)? {
            Raw::One(s) if s == "root" => Self::Root,
            Raw::One(s) if s == "each" => Self::Each,
            Raw::One(s) if s == "joined" => Self::Joined,
            Raw::One(s) => match s.strip_prefix("lane:") {
                Some(lane) => Self::Lane(lane.to_owned()),
                None => {
                    return Err(serde::de::Error::custom(format!(
                        "context {s:?}: expected root, each, joined, lane:<name> or a list of lanes"
                    )));
                }
            },
            Raw::Many(lanes) => Self::Lanes(lanes),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Gate {
    Command {
        #[serde(default)]
        argv: Option<Vec<String>>,
        #[serde(default)]
        per_lane: Option<BTreeMap<String, Vec<String>>>,
        #[serde(rename = "in", default = "default_run_in")]
        run_in: String,
        /// The name of an earlier stage whose command gate this one
        /// is, by reference: the same command, and a result of it at
        /// the same clean head is reused rather than run again.
        #[serde(default)]
        like: Option<String>,
        /// `allow` or `deny`: this gate's own network policy when it runs
        /// confined, over `[policy] network`. A gate given by `like`
        /// takes the referenced gate's.
        #[serde(default)]
        network: Option<crate::git::Network>,
    },
    External {
        check: String,
        #[serde(default)]
        provider: Option<String>,
        /// A pending decision while the fact is awaited.
        #[serde(default)]
        decision: Option<String>,
        /// For `pr-checks`: `"none"` when the repository runs no CI, so
        /// the gate passes on a PR at the head alone instead of asking
        /// about the missing checks every time.
        #[serde(default)]
        checks: Option<String>,
    },
    Human {
        decision: String,
        #[serde(default)]
        confirm: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stage {
    pub name: String,
    /// Present: an agent stage.
    #[serde(default)]
    pub operator: Option<String>,
    /// Present: a workflow stage run by this reviewer operator.
    #[serde(default)]
    pub review: Option<String>,
    #[serde(default)]
    pub context: Context,
    /// The artifacts the stage writes; each name expands as `{name}` in
    /// the prompt. The file spells one as a name or as `{ name, secret }`.
    #[serde(default)]
    pub writes: Vec<Write>,
    /// The artifact a workflow stage reviews.
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub manifest: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub gate: Option<Gate>,
    #[serde(default)]
    pub needs: Vec<String>,
    #[serde(default)]
    pub release: Option<String>,
    #[serde(default)]
    pub services: Vec<String>,
    #[serde(default)]
    pub before: BTreeMap<String, Vec<String>>,
    /// Present: a code review stage; the operators that review the
    /// lane's branch, at once, each round.
    #[serde(default)]
    pub reviewers: Vec<String>,
    /// The operator that addresses a round's findings on the branch;
    /// a fresh session each round.
    #[serde(default)]
    pub implementer: Option<String>,
    /// Review passes a code review stage may make before the findings
    /// left are a question; default 3.
    #[serde(default)]
    pub cap: Option<u32>,
    /// Templates for a code review stage; each has a default.
    #[serde(default)]
    pub review_prompt: Option<String>,
    #[serde(default)]
    pub fix_prompt: Option<String>,
    #[serde(default)]
    pub no_feedback: Option<String>,
    /// The round from which a round with only style points converges;
    /// default 2. Read from the live pipeline file each round.
    #[serde(default)]
    pub style_rounds: Option<u32>,
    /// What a code review stage does to the branch's commits as it
    /// completes; default `keep`. Read from the ticket's copy, so a
    /// ticket's mode never changes halfway.
    #[serde(default)]
    pub commits: Option<Commits>,
    /// What this stage's agent (a code review stage's implementer) gets
    /// when it stops with a dirty tree; absent, the policy's.
    #[serde(default)]
    pub on_dirty: Option<OnDirty>,
}

/// One artifact a stage writes. A secret one is written by a gate-only
/// command, never read by Dispatch, and deleted when the stage's hold is
/// released.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "WriteFile", into = "WriteFile")]
pub struct Write {
    /// The artifact's name: `{name}` in the prompt, `<name>.md` on disk.
    pub name: String,
    /// Written by a gate-only command only, never read by Dispatch, and
    /// deleted when the hold its stage needs is released.
    pub secret: bool,
}

impl Write {
    /// A plain, readable artifact.
    #[must_use]
    pub fn named(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            secret: false,
        }
    }
}

/// `Write` as the file spells it: a name, or a table. A plain artifact
/// writes back as a name, so older copies of a pipeline read the same.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum WriteFile {
    Name(String),
    Table {
        name: String,
        #[serde(default)]
        secret: bool,
    },
}

impl From<WriteFile> for Write {
    fn from(file: WriteFile) -> Self {
        match file {
            WriteFile::Name(name) => Self {
                name,
                secret: false,
            },
            WriteFile::Table { name, secret } => Self { name, secret },
        }
    }
}

impl From<Write> for WriteFile {
    fn from(w: Write) -> Self {
        if w.secret {
            Self::Table {
                name: w.name,
                secret: true,
            }
        } else {
            Self::Name(w.name)
        }
    }
}

/// The environment variable name for an artifact name: upper-cased,
/// with anything not a letter or digit as `_`.
#[must_use]
pub fn env_key(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Artifact names Dispatch reads itself, which therefore cannot be
/// secret.
const READ_BY_NAME: [&str; 4] = ["plan", "notes", "summary", "checks"];

/// What a stage is, from which fields it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageKind {
    Agent,
    Workflow,
    GateOnly,
    /// Several reviewers of a branch and an implementer, in rounds.
    Review,
}

/// Every artifact a stage leaves for the stages after it: what it
/// writes, the `checks` log of a command gate, a code review's
/// `summary`, and a workflow's finalized copy of its subject.
fn produced(stage: &Stage) -> impl Iterator<Item = &str> {
    let checks = matches!(stage.gate, Some(Gate::Command { .. })).then_some("checks");
    let summary = (stage.kind() == StageKind::Review).then_some("summary");
    let subject = stage.subject.as_deref().filter(|_| stage.review.is_some());
    stage
        .write_names()
        .chain(checks)
        .chain(summary)
        .chain(subject)
}

/// Review passes a code review stage makes when the file says nothing.
pub const DEFAULT_REVIEW_CAP: u32 = 3;

/// The round from which a round with only style points converges when
/// the file says nothing.
pub const DEFAULT_STYLE_ROUNDS: u32 = 2;

impl Stage {
    #[must_use]
    pub fn kind(&self) -> StageKind {
        if !self.reviewers.is_empty() {
            StageKind::Review
        } else if self.operator.is_some() {
            StageKind::Agent
        } else if self.review.is_some() {
            StageKind::Workflow
        } else {
            StageKind::GateOnly
        }
    }

    /// The stage runs in more than one lane context, so it writes one file
    /// per lane under each name.
    #[must_use]
    pub fn runs_per_lane(&self) -> bool {
        match &self.context {
            Context::Each => true,
            Context::Lanes(l) => l.len() > 1,
            Context::Root | Context::Joined | Context::Lane(_) => false,
        }
    }

    /// A gate-only stage whose gate is a command (a deploy).
    #[must_use]
    pub fn is_command_stage(&self) -> bool {
        self.kind() == StageKind::GateOnly && matches!(self.gate, Some(Gate::Command { .. }))
    }

    /// The names of the artifacts the stage writes.
    pub fn write_names(&self) -> impl Iterator<Item = &str> {
        self.writes.iter().map(|w| w.name.as_str())
    }

    /// The names of the stage's secret artifacts.
    pub fn secret_writes(&self) -> impl Iterator<Item = &str> {
        self.writes
            .iter()
            .filter(|w| w.secret)
            .map(|w| w.name.as_str())
    }

    /// The review passes a code review stage may make.
    #[must_use]
    pub fn review_cap(&self) -> u32 {
        self.cap.unwrap_or(DEFAULT_REVIEW_CAP)
    }

    /// The round from which a round with only style points converges.
    #[must_use]
    pub fn style_rounds(&self) -> u32 {
        self.style_rounds.unwrap_or(DEFAULT_STYLE_ROUNDS)
    }

    /// What a code review stage does to the branch's commits.
    #[must_use]
    pub fn commits(&self) -> Commits {
        self.commits.unwrap_or_default()
    }
}

/// What an agent that stops with its tree not clean gets before the
/// question: `"ask"` puts the question at once; `{ nudge = N }` first
/// types a line into its session, up to N times, each after a stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "OnDirtyFile", into = "OnDirtyFile")]
pub enum OnDirty {
    Ask,
    Nudge(u32),
}

impl Default for OnDirty {
    fn default() -> Self {
        Self::Nudge(1)
    }
}

/// `OnDirty` as the file spells it: a word or a table.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum OnDirtyFile {
    Word(String),
    Table { nudge: u32 },
}

impl TryFrom<OnDirtyFile> for OnDirty {
    type Error = String;

    fn try_from(file: OnDirtyFile) -> Result<Self, String> {
        match file {
            OnDirtyFile::Word(w) if w == "ask" => Ok(Self::Ask),
            OnDirtyFile::Word(w) => {
                Err(format!("on_dirty: {w:?} is not \"ask\" or {{ nudge = N }}"))
            }
            OnDirtyFile::Table { nudge: 0 } => {
                Err("on_dirty: nudge = 0 sends nothing; use \"ask\"".into())
            }
            OnDirtyFile::Table { nudge } => Ok(Self::Nudge(nudge)),
        }
    }
}

impl From<OnDirty> for OnDirtyFile {
    fn from(on: OnDirty) -> Self {
        match on {
            OnDirty::Ask => Self::Word("ask".into()),
            OnDirty::Nudge(nudge) => Self::Table { nudge },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Policy {
    pub slots: u32,
    pub waiting_on_me: u32,
    #[serde(default)]
    pub ports: Option<[u16; 2]>,
    #[serde(default)]
    pub rates: BTreeMap<String, [f64; 2]>,
    /// Per decision kind: `ask`, `recommend` or `auto`.
    #[serde(default)]
    pub decisions: BTreeMap<String, String>,
    /// Answer Claude Code's folder trust question for the project's
    /// agents, which every fresh worktree asks once.
    #[serde(default)]
    pub trust_folders: bool,
    /// How many failed attempts a stage may collect in one context
    /// before the ticket parks instead of asking for another run.
    #[serde(default = "default_max_reruns")]
    pub max_reruns: u32,
    /// Free space on the worktrees' volume, in GB, below which nothing
    /// new starts: a build or an agent on a full disk fails for nothing
    /// and costs the run. Read live, like `slots`.
    #[serde(default = "default_min_free_gb")]
    pub min_free_gb: u32,
    /// Bring a lane's branch up to its base when a stage begins, so a
    /// plan that sat is not implemented on stale code. A clean rebase
    /// is mechanical; a conflict goes to the `rebaser`.
    #[serde(default = "yes")]
    pub refresh: bool,
    /// The operator that rebases a branch whose PR conflicts with its
    /// base, cloned from the lane's implementer; absent, a conflict is
    /// a question.
    #[serde(default)]
    pub rebaser: Option<String>,
    /// How many rebases one PR may get before the conflict is a
    /// question instead.
    #[serde(default = "default_max_rebases")]
    pub max_rebases: u32,
    /// The operator that fixes a branch whose PR checks are red, cloned
    /// from the lane's last agent; absent, red checks are a question.
    #[serde(default)]
    pub fixer: Option<String>,
    /// How many fixes one PR may get before red checks are a question.
    #[serde(default = "default_max_fixes")]
    pub max_fixes: u32,
    /// What an agent that stops with a dirty tree gets before the
    /// question; a stage's own `on_dirty` overrides it. Read from the
    /// ticket's copy, like `max_reruns`.
    #[serde(default)]
    pub on_dirty: OnDirty,
    /// Run setup, command gates, review checks and command reviewers
    /// confined to the ticket's trees, the attempt directory and the
    /// lane's `writable` paths. Off when absent, so a ticket's copy
    /// taken before the key existed keeps running as it did.
    #[serde(default)]
    pub confine: bool,
    /// `allow` or `deny`: whether a confined command may reach off this
    /// machine. Loopback stays open either way. A command gate's own
    /// `network` overrides it.
    #[serde(default)]
    pub network: crate::git::Network,
    /// The reviewer of a conflict's resolution brought up after the last
    /// code review stage; absent, that stage's first reviewer that is
    /// not `style`.
    #[serde(default)]
    pub resolution_reviewer: Option<String>,
}

fn default_max_fixes() -> u32 {
    2
}

fn default_max_reruns() -> u32 {
    3
}

fn default_min_free_gb() -> u32 {
    10
}

fn yes() -> bool {
    true
}

fn default_max_rebases() -> u32 {
    2
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            slots: 1,
            waiting_on_me: 2,
            ports: None,
            rates: BTreeMap::new(),
            decisions: BTreeMap::new(),
            trust_folders: false,
            max_reruns: default_max_reruns(),
            min_free_gb: default_min_free_gb(),
            refresh: true,
            rebaser: None,
            max_rebases: default_max_rebases(),
            fixer: None,
            max_fixes: default_max_fixes(),
            on_dirty: OnDirty::default(),
            confine: false,
            network: crate::git::Network::default(),
            resolution_reviewer: None,
        }
    }
}

impl Pipeline {
    /// The branch a lane branches from: its own, else the project's.
    #[must_use]
    pub fn lane_base<'a>(&'a self, lane: &'a Lane) -> &'a str {
        lane.base.as_deref().unwrap_or(&self.project.base)
    }

    #[must_use]
    pub fn lane_remote<'a>(&'a self, lane: &'a Lane) -> &'a str {
        lane.remote.as_deref().unwrap_or(&self.project.remote)
    }

    /// A named remote of a lane's repository (or the project's, for a
    /// lane without one): the default one, or one of the `remotes`.
    #[must_use]
    pub fn remote_url(&self, lane: Option<&Lane>, name: &str) -> Option<String> {
        let (default_name, default_url, extra) = match lane {
            Some(l) if l.repo.is_some() => (self.lane_remote(l), l.repo.clone(), &l.remotes),
            _ => (
                self.project.remote.as_str(),
                self.project.repo.clone(),
                &self.project.remotes,
            ),
        };
        if name == default_name {
            default_url
        } else {
            extra.get(name).cloned()
        }
    }

    /// Whether tickets work in worktrees of Dispatch's clone (`repo`),
    /// as opposed to in place at `root`.
    #[must_use]
    pub fn cuts_worktrees(&self) -> bool {
        self.project.repo.is_some()
    }

    /// Parse and validate one file's text.
    pub fn parse(text: &str) -> Result<Self> {
        let mut p: Self = toml::from_str(text).context("parse the pipeline file")?;
        p.validate()?;
        if let Some(dir) = &p.project.worktrees {
            p.project.worktrees = Some(crate::store::expand_home(dir));
        }
        for lane in &mut p.lanes {
            for path in &mut lane.writable {
                *path = crate::store::expand_home(path);
            }
        }
        Ok(p)
    }

    /// A short hash of the text a ticket was taken under.
    #[must_use]
    pub fn fingerprint(text: &str) -> String {
        use std::fmt::Write as _;
        let digest = Sha256::digest(text.as_bytes());
        digest.iter().take(6).fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
    }

    /// The dial for a decision kind; `ask` when the file says nothing.
    #[must_use]
    pub fn dial(&self, decision: &str) -> &str {
        self.policy
            .decisions
            .get(decision)
            .map_or("ask", String::as_str)
    }

    /// What `stage`'s agent gets when it stops with a dirty tree: the
    /// stage's own rule, else the policy's.
    #[must_use]
    pub fn on_dirty(&self, stage: &Stage) -> OnDirty {
        stage.on_dirty.unwrap_or(self.policy.on_dirty)
    }

    #[must_use]
    pub fn lane(&self, name: &str) -> Option<&Lane> {
        self.lanes.iter().find(|l| l.name == name)
    }

    /// A gate names a check Dispatch knows and one the project can answer.
    fn validate_gate(stage: &Stage) -> Result<()> {
        if let Some(Gate::External { check, checks, .. }) = &stage.gate {
            if !matches!(
                check.as_str(),
                "review-finalized" | "pr-checks" | "pr-merged"
            ) {
                bail!(
                    "stage {:?}: {check:?} is not a check Dispatch knows",
                    stage.name
                );
            }
            match checks.as_deref() {
                None => {}
                Some("none") if check == "pr-checks" => {}
                Some(other) => bail!(
                    "stage {:?}: checks = {other:?}; only pr-checks takes checks = \"none\"",
                    stage.name
                ),
            }
        }
        Ok(())
    }

    /// A stage's own `on_dirty` goes only where an agent stops and its
    /// tree is judged: an agent stage with a command gate, or a code
    /// review stage's implementer.
    fn validate_on_dirty(stage: &Stage) -> Result<()> {
        let judged = match stage.kind() {
            StageKind::Review => true,
            StageKind::Agent => matches!(stage.gate, Some(Gate::Command { .. })),
            StageKind::Workflow | StageKind::GateOnly => false,
        };
        if stage.on_dirty.is_some() && !judged {
            bail!(
                "stage {:?}: on_dirty needs an agent stage with a command gate, or a code review stage",
                stage.name
            );
        }
        Ok(())
    }

    /// One stage's references, given what earlier stages wrote.
    fn validate_stage(&self, stage: &Stage, written: &[&str]) -> Result<()> {
        if stage.operator.is_some() && stage.review.is_some() {
            bail!("stage {:?} names both an operator and a review", stage.name);
        }
        Self::validate_gate(stage)?;
        Self::validate_on_dirty(stage)?;
        if stage.kind() == StageKind::Review {
            self.validate_review_stage(stage)?;
        } else if stage.implementer.is_some()
            || stage.cap.is_some()
            || stage.style_rounds.is_some()
            || stage.commits.is_some()
        {
            bail!(
                "stage {:?} has implementer, cap, style_rounds or commits but no reviewers",
                stage.name
            );
        }
        self.validate_operator_ref(stage)?;
        self.validate_like(stage)?;
        if let Some(rv) = &stage.review {
            let Some(operator) = self.operators.get(rv) else {
                bail!("stage {:?} names an unknown reviewer {rv:?}", stage.name);
            };
            if operator.review.is_none() {
                bail!(
                    "stage {:?}: operator {rv:?} has no [operators.{rv}.review] table",
                    stage.name
                );
            }
            let Some(subject) = &stage.subject else {
                bail!("review stage {:?} names no subject", stage.name);
            };
            if !written.contains(&subject.as_str()) {
                bail!(
                    "review stage {:?} reviews {subject:?}, which no earlier stage writes",
                    stage.name
                );
            }
        }
        match &stage.context {
            Context::Lane(lane) => {
                if self.lane(lane).is_none() {
                    bail!("stage {:?} names an unknown lane {lane:?}", stage.name);
                }
            }
            Context::Lanes(lanes) => {
                for lane in lanes {
                    if self.lane(lane).is_none() {
                        bail!("stage {:?} names an unknown lane {lane:?}", stage.name);
                    }
                }
            }
            Context::Root | Context::Each | Context::Joined => {}
        }
        self.validate_writes(stage)?;
        for need in &stage.needs {
            if !self.resources.iter().any(|r| &r.name == need)
                && !self.lanes.iter().any(|l| &l.name == need)
            {
                bail!("stage {:?} needs an unknown resource {need:?}", stage.name);
            }
        }
        if let Some(Gate::Command {
            argv,
            per_lane,
            run_in,
            like,
            ..
        }) = &stage.gate
        {
            if argv.is_none() && per_lane.is_none() && like.is_none() {
                bail!(
                    "stage {:?}: a command gate needs argv, per_lane or like",
                    stage.name
                );
            }
            let ok = matches!(run_in.as_str(), "root" | "lane")
                || run_in
                    .strip_prefix("lane:")
                    .is_some_and(|l| self.lane(l).is_some());
            if !ok {
                bail!(
                    "stage {:?}: a command gate runs in root, lane or lane:<name>",
                    stage.name
                );
            }
        }
        if stage.kind() == StageKind::GateOnly && stage.gate.is_none() {
            bail!(
                "stage {:?} has no operator, no review and no gate",
                stage.name
            );
        }
        Self::validate_gate_context(stage)?;
        self.validate_services(stage)
    }

    /// What a stage may write: a gate-only stage writes only through a
    /// command it runs itself, in one context, never its own `checks`;
    /// no stage writes `commit`, which `inputs.<stage>.commit` already
    /// means; and a secret is written by a gate-only command, under a
    /// name Dispatch never reads, in a stage that holds a resource whose
    /// release deletes it.
    fn validate_writes(&self, stage: &Stage) -> Result<()> {
        let name = &stage.name;
        let mut seen = std::collections::BTreeSet::new();
        for w in stage.write_names() {
            if !seen.insert(w) {
                bail!("stage {name:?} writes {w:?} twice");
            }
        }
        if stage.write_names().any(|w| w == "commit") {
            bail!("stage {name:?} writes \"commit\", which inputs.{name}.commit already names");
        }
        if stage.kind() == StageKind::GateOnly && !stage.writes.is_empty() {
            let runs = matches!(
                &stage.gate,
                Some(Gate::Command {
                    argv,
                    per_lane,
                    like: None,
                    ..
                }) if argv.is_some() || per_lane.is_some()
            );
            if !runs {
                bail!(
                    "stage {name:?}: a gate-only stage writes only with a command gate of its own (argv or per_lane, not like)"
                );
            }
            if stage.write_names().any(|w| w == "checks") {
                bail!("stage {name:?} writes \"checks\", which is its command's own log");
            }
            // A gate-only writer's file (a deploy's outputs) is read by
            // root stages and by Dispatch's own reads, which see no lane,
            // so one written per lane would hand them whichever lane
            // finished last.
            if stage.runs_per_lane() {
                bail!("stage {name:?}: a gate-only stage writes only in one context, not per lane");
            }
        }
        for secret in stage.secret_writes() {
            if stage.kind() != StageKind::GateOnly {
                bail!(
                    "stage {name:?}: secret {secret:?}: secret artifacts are written by a gate-only command"
                );
            }
            if READ_BY_NAME.contains(&secret) {
                bail!("stage {name:?}: {secret:?} is read by Dispatch, so it cannot be secret");
            }
            if !stage.needs.iter().any(|n| self.resource(n).is_some()) {
                bail!(
                    "stage {name:?}: secret {secret:?} needs a [[resources]] entry in needs, whose release deletes it"
                );
            }
        }
        Ok(())
    }

    /// A gate-only command runs in its context's tree, so a gate that
    /// names a lane must be in that lane's context.
    fn validate_gate_context(stage: &Stage) -> Result<()> {
        if stage.kind() == StageKind::GateOnly
            && let Some(Gate::Command { run_in, .. }) = &stage.gate
            && let Some(lane) = run_in.strip_prefix("lane:")
            && stage.context != Context::Lane(lane.to_owned())
        {
            bail!(
                "stage {:?}: a gate-only command in lane:{lane} needs context = \"lane:{lane}\"",
                stage.name
            );
        }
        Ok(())
    }

    /// A stage's `services` name lanes that serve, once each, with a
    /// resource held while they run and ports to put them on; its
    /// `before` keys are among them.
    fn validate_services(&self, stage: &Stage) -> Result<()> {
        let name = &stage.name;
        let mut seen = std::collections::BTreeSet::new();
        for lane in &stage.services {
            if !seen.insert(lane) {
                bail!("stage {name:?} names service {lane:?} twice");
            }
            match self.lane(lane) {
                None => bail!("stage {name:?} serves an unknown lane {lane:?}"),
                Some(l) if l.serve.is_none() => {
                    bail!("stage {name:?} serves lane {lane:?}, which has no serve")
                }
                Some(_) => {}
            }
        }
        for lane in stage.before.keys() {
            if !stage.services.contains(lane) {
                bail!("stage {name:?}: before names {lane:?}, which is not in its services");
            }
        }
        if !stage.services.is_empty() {
            // The services live until the resource's range ends, so the
            // stage must hold one.
            if !stage.needs.iter().any(|n| self.resource(n).is_some()) {
                bail!("stage {name:?} names services but needs no [[resources]] entry");
            }
            match self.policy.ports {
                None => bail!("stage {name:?} names services but [policy] has no ports"),
                Some([lo, hi]) if lo > hi => bail!("[policy] ports = [{lo}, {hi}] is empty"),
                Some(_) => {}
            }
        }
        Ok(())
    }

    /// A lane's `serve`: a command, a URL with the port in it, a probe
    /// path, and an environment with nothing in it but literals and the
    /// port. The environment travels on the service's command line,
    /// which a record and the pane keep, so no secret may be there.
    fn validate_serve(lane: &Lane) -> Result<()> {
        let Some(serve) = &lane.serve else {
            return Ok(());
        };
        let l = &lane.name;
        if serve.argv.is_empty() {
            bail!("lane {l}: serve.argv is empty");
        }
        if !serve.url.contains("{port}") {
            bail!("lane {l}: serve.url must contain {{port}}");
        }
        if !serve.ready.http.starts_with('/') {
            bail!("lane {l}: serve.ready.http must start with /");
        }
        for (key, value) in &serve.env {
            let upper = key.to_ascii_uppercase();
            if ["TOKEN", "SECRET", "PASSWORD", "KEY", "CREDENTIAL"]
                .iter()
                .any(|word| upper.contains(word))
            {
                bail!(
                    "lane {l}: serve.env {key} looks like a secret; service env travels on the command line, so secrets are not allowed there"
                );
            }
            if value.replace("{port}", "").contains('{') {
                bail!("lane {l}: serve.env {key}: the only field a value may name is {{port}}");
            }
        }
        Ok(())
    }

    /// The `[[resources]]` entry named `name`; `None` for a lane name,
    /// which a stage's `needs` may also hold.
    #[must_use]
    pub fn resource(&self, name: &str) -> Option<&Resource> {
        self.resources.iter().find(|r| r.name == name)
    }

    /// The first and last index of the stages whose `needs` name the
    /// `[[resources]]` entry `resource`: the range a ticket holds it
    /// for. `None` for a lane name or a resource no stage needs.
    #[must_use]
    pub fn hold_range(&self, resource: &str) -> Option<(usize, usize)> {
        self.resource(resource)?;
        let mut indexes = self
            .stages
            .iter()
            .enumerate()
            .filter(|(_, s)| s.needs.iter().any(|n| n == resource))
            .map(|(i, _)| i);
        let first = indexes.next()?;
        Some((first, indexes.next_back().unwrap_or(first)))
    }

    /// The union of the hold ranges of the resources `stage` needs;
    /// `None` when it needs none.
    #[must_use]
    pub fn needs_range(&self, stage: usize) -> Option<(usize, usize)> {
        let s = self.stages.get(stage)?;
        s.needs
            .iter()
            .filter_map(|n| self.hold_range(n))
            .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
    }

    /// The last stage a service started at `stage` lives through: the
    /// end of the `needs` ranges that contain it, `stage` itself when it
    /// holds none.
    #[must_use]
    pub fn services_until(&self, stage: usize) -> usize {
        self.stages.get(stage).map_or(stage, |s| {
            s.needs
                .iter()
                .filter_map(|n| self.hold_range(n))
                .map(|(_, last)| last)
                .max()
                .unwrap_or(stage)
        })
    }

    /// Each `[[resources]]` name in `needs` is named by one run of
    /// stages with no gap, so "held until the last stage needing it
    /// ends" is a fact the file states. A hold retaken partway through
    /// the run cancels what the earlier stages completed and goes back
    /// to them, which a review's rounds or a workflow's run cannot be
    /// sent back through, so neither may stand before the run's last
    /// stage.
    fn validate_contiguous_needs(&self) -> Result<()> {
        for r in &self.resources {
            let Some((first, last)) = self.hold_range(&r.name) else {
                continue;
            };
            if let Some(gap) = self.stages[first..=last]
                .iter()
                .find(|s| !s.needs.contains(&r.name))
            {
                bail!(
                    "stage {:?} breaks the run of stages needing {:?}; needs must name a resource on stages next to each other",
                    gap.name,
                    r.name
                );
            }
            if let Some(s) = self.stages[first..last]
                .iter()
                .find(|s| matches!(s.kind(), StageKind::Review | StageKind::Workflow))
            {
                bail!(
                    "stage {:?} is a review or workflow stage in the run of stages needing {:?}; only the run's last stage may be one",
                    s.name,
                    r.name
                );
            }
        }
        Ok(())
    }

    /// A code review stage: reviewers that exist, once each; an
    /// implementer that is Claude Code (its Stop is the one completion
    /// signal); a cap of at least one; the branch as its only subject;
    /// its own command gate; none of an agent or workflow stage's
    /// fields; and a project with branches to review.
    fn validate_review_stage(&self, stage: &Stage) -> Result<()> {
        let name = &stage.name;
        if stage.operator.is_some() || stage.review.is_some() || !stage.writes.is_empty() {
            bail!("stage {name:?}: reviewers go with no operator, review or writes");
        }
        let mut seen = std::collections::BTreeSet::new();
        for r in &stage.reviewers {
            if !seen.insert(r) {
                bail!("stage {name:?} names reviewer {r:?} twice");
            }
            let Some(op) = self.operators.get(r) else {
                bail!("stage {name:?} names an unknown reviewer {r:?}");
            };
            if op.kind == OperatorKind::Command && op.argv.is_empty() {
                bail!("stage {name:?}: command reviewer {r:?} has no argv");
            }
        }
        let Some(implementer) = &stage.implementer else {
            bail!("stage {name:?} names no implementer");
        };
        match self.operators.get(implementer) {
            None => bail!("stage {name:?} names an unknown implementer {implementer:?}"),
            Some(op) if op.kind != OperatorKind::Claude => bail!(
                "stage {name:?}: implementer {implementer:?} must be claude (its Stop is the completion signal)"
            ),
            Some(_) => {}
        }
        if stage.cap == Some(0) {
            bail!("stage {name:?}: cap must be at least 1");
        }
        if stage.style_rounds == Some(0) {
            bail!("stage {name:?}: style_rounds must be at least 1");
        }
        // Someone else's branch is theirs to change.
        if stage.commits() != Commits::Keep && self.source == Source::PullRequest {
            bail!("stage {name:?}: a pull-request pipeline rewrites no history");
        }
        if stage.subject.as_deref().is_some_and(|s| s != "branch") {
            bail!("stage {name:?}: a code review stage reviews the branch only");
        }
        if !matches!(stage.gate, Some(Gate::Command { .. })) {
            bail!("stage {name:?}: a code review stage needs a command gate (its checks)");
        }
        if !self.cuts_worktrees() {
            bail!("stage {name:?}: a code review stage needs branches (a project with repo)");
        }
        Ok(())
    }

    /// A stage's operator exists and is an agent.
    fn validate_operator_ref(&self, stage: &Stage) -> Result<()> {
        if let Some(op) = &stage.operator {
            let Some(operator) = self.operators.get(op) else {
                bail!("stage {:?} names an unknown operator {op:?}", stage.name);
            };
            if !operator.kind.is_agent() {
                bail!(
                    "stage {:?}: operator {op:?} is a command, which only a code review stage runs",
                    stage.name
                );
            }
        }
        Ok(())
    }

    /// A gate given by reference names an earlier stage with a command
    /// gate of its own.
    fn validate_like(&self, stage: &Stage) -> Result<()> {
        if let Some(Gate::Command {
            like: Some(like),
            network: Some(_),
            ..
        }) = &stage.gate
        {
            bail!(
                "stage {:?}: a gate given by `like` takes its network from {like}",
                stage.name
            );
        }
        if let Some(Gate::Command {
            like: Some(like), ..
        }) = &stage.gate
        {
            let earlier = self
                .stages
                .iter()
                .take_while(|s| s.name != stage.name)
                .find(|s| &s.name == like);
            match earlier {
                Some(s) if matches!(s.gate, Some(Gate::Command { like: None, .. })) => {}
                _ => bail!(
                    "stage {:?}: gate like {like:?} names no earlier stage with a command gate",
                    stage.name
                ),
            }
        }
        Ok(())
    }

    /// The command a stage's gate runs: its own, or the earlier stage's
    /// it names by `like`. `None` when the gate is not a command gate.
    #[must_use]
    pub fn command_gate<'a>(&'a self, stage: &'a Stage) -> Option<&'a Gate> {
        match &stage.gate {
            Some(Gate::Command {
                like: Some(like), ..
            }) => self
                .stages
                .iter()
                .find(|s| &s.name == like)
                .and_then(|s| s.gate.as_ref()),
            Some(gate @ Gate::Command { .. }) => Some(gate),
            _ => None,
        }
    }

    /// Every reference resolves and every stage is one of the four kinds.
    fn validate(&self) -> Result<()> {
        if self.version != 1 {
            bail!(
                "pipeline version {} is not understood (only 1)",
                self.version
            );
        }
        if self.lanes.is_empty() {
            bail!("a pipeline needs at least one lane");
        }
        for (key, name) in [
            ("rebaser", &self.policy.rebaser),
            ("fixer", &self.policy.fixer),
        ] {
            if let Some(name) = name {
                match self.operators.get(name) {
                    None => bail!("[policy] {key} names an unknown operator {name:?}"),
                    Some(op) if !op.kind.is_agent() => {
                        bail!("[policy] {key} names {name:?}, which is a command, not an agent")
                    }
                    Some(_) => {}
                }
            }
            // Someone else's branch is theirs to change.
            if name.is_some() && self.source == Source::PullRequest {
                bail!("[policy] {key}: a pull-request pipeline pushes nothing");
            }
        }
        if let Some(name) = &self.policy.resolution_reviewer {
            match self.operators.get(name) {
                None => bail!("[policy] resolution_reviewer names an unknown operator {name:?}"),
                Some(op) if op.kind == OperatorKind::Command && op.argv.is_empty() => {
                    bail!("[policy] resolution_reviewer {name:?} is a command with no argv")
                }
                Some(_) => {}
            }
        }
        match (&self.project.repo, &self.project.root) {
            (Some(_), None) | (None, Some(_)) => {}
            (Some(_), Some(_)) => bail!("[project] takes repo or root, not both"),
            (None, None) => bail!("[project] needs repo (a URL to clone) or root (a directory)"),
        }
        let mut names = std::collections::BTreeSet::new();
        for lane in &self.lanes {
            if !names.insert(&lane.name) {
                bail!("lane {:?} is listed twice", lane.name);
            }
            Self::validate_serve(lane)?;
        }
        let mut stage_names = std::collections::BTreeSet::new();
        let mut written: Vec<&str> = Vec::new();
        let mut secrets: Vec<&str> = Vec::new();
        let mut keys = BTreeMap::new();
        for (i, stage) in self.stages.iter().enumerate() {
            if !stage_names.insert(&stage.name) {
                bail!("stage {:?} is listed twice", stage.name);
            }
            self.validate_stage(stage, &written)?;
            self.validate_inputs(stage, &self.stages[..i])?;
            if let Some(subject) = &stage.subject
                && secrets.contains(&subject.as_str())
            {
                bail!(
                    "stage {:?} reviews {subject:?}, which is secret and never read",
                    stage.name
                );
            }
            Self::validate_keys(stage, &mut keys)?;
            written.extend(stage.write_names());
            secrets.extend(stage.secret_writes());
            if stage.review.is_some()
                && let Some(subject) = &stage.subject
            {
                // A review's finalized copy is the subject under its
                // own name, for the stages after it.
                written.push(subject.as_str());
            }
        }
        self.validate_contiguous_needs()?;
        for (name, dial) in &self.policy.decisions {
            if !matches!(dial.as_str(), "ask" | "recommend" | "auto") {
                bail!("decision {name:?}: the dial is ask, recommend or auto, not {dial:?}");
            }
        }
        if let Some(sup) = &self.supervisor {
            self.validate_supervisor(sup)?;
        }
        Ok(())
    }

    /// One variable name per artifact name, so no two artifacts share a
    /// `DISPATCH_WRITES_*` or `DISPATCH_INPUT_*` key; and one per
    /// stage-qualified input, whose `DISPATCH_INPUT_<STAGE>_<NAME>` must
    /// not land on another. `keys` holds what the stages before claimed.
    fn validate_keys<'a>(
        stage: &'a Stage,
        keys: &mut BTreeMap<String, (Option<&'a str>, &'a str)>,
    ) -> Result<()> {
        let shown = |(s, n): (Option<&str>, &str)| match s {
            Some(s) => format!("{s}'s {n:?}"),
            None => format!("{n:?}"),
        };
        let bare = stage.write_names().map(|n| (env_key(n), (None, n)));
        let qualified = produced(stage).map(|n| {
            (
                format!("{}_{}", env_key(&stage.name), env_key(n)),
                (Some(stage.name.as_str()), n),
            )
        });
        for (key, this) in bare.chain(qualified) {
            if let Some(other) = keys.insert(key.clone(), this)
                && other != this
            {
                bail!(
                    "stage {:?} leaves {:?}, whose DISPATCH_INPUT_{key} is the same variable name as {}",
                    stage.name,
                    this.1,
                    shown(other)
                );
            }
        }
        Ok(())
    }

    /// A root or joined stage never names a file a stage before it
    /// writes once per lane: whichever lane it took would be a guess.
    /// Every template rendered with the stage's inputs is read.
    fn validate_inputs(&self, stage: &Stage, before: &[Stage]) -> Result<()> {
        if !matches!(stage.context, Context::Root | Context::Joined) {
            return Ok(());
        }
        let text = self.templates_of(stage);
        let name = &stage.name;
        let ctx = if stage.context == Context::Root {
            "root"
        } else {
            "joined"
        };
        let advice = "name it from a stage in that lane's context, which reads its own lane's file";
        let per_lane = |s: &str| before.iter().any(|x| x.name == s && x.runs_per_lane());
        for s in crate::template::commit_stages(&text) {
            if per_lane(&s) {
                bail!(
                    "stage {name:?} ({ctx}) names {{inputs.{s}.commit}}, but {s} records a commit once per lane; {advice}"
                );
            }
        }
        for (s, x) in crate::template::input_names(&text) {
            match s {
                Some(s) if per_lane(&s) => bail!(
                    "stage {name:?} ({ctx}) names {{inputs.{s}.{x}}}, but {s} writes {x} once per lane; {advice}"
                ),
                Some(_) => {}
                None => {
                    let producers: Vec<&Stage> = before
                        .iter()
                        .filter(|b| produced(b).any(|p| p == x))
                        .collect();
                    if !producers.is_empty() && producers.iter().all(|b| b.runs_per_lane()) {
                        let names: Vec<&str> = producers.iter().map(|b| b.name.as_str()).collect();
                        bail!(
                            "stage {name:?} ({ctx}) names {{inputs.{x}}}, but {} writes {x} once per lane; {advice}",
                            names.join(", ")
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Every template rendered with a stage's inputs, in one string: its
    /// prompt and its operator's guidance, a code review's prompts and
    /// its reviewers' and implementer's guidance, a workflow reviewer's
    /// templates, and the guidance of the policy's `rebaser` (a PR gate
    /// can rebase) and `fixer` (a `pr-checks` gate can fix red checks).
    fn templates_of(&self, stage: &Stage) -> String {
        let (rebaser, fixer) = match &stage.gate {
            Some(Gate::External { check, .. }) if check == "pr-checks" => {
                (self.policy.rebaser.as_ref(), self.policy.fixer.as_ref())
            }
            Some(Gate::External { check, .. }) if check == "pr-merged" => {
                (self.policy.rebaser.as_ref(), None)
            }
            _ => (None, None),
        };
        let guidance = stage
            .operator
            .iter()
            .chain(&stage.reviewers)
            .chain(&stage.implementer)
            .chain(rebaser)
            .chain(fixer)
            .filter_map(|o| self.operators.get(o))
            .map(|o| o.guidance.as_str());
        let workflow = stage
            .review
            .as_ref()
            .and_then(|o| self.operators.get(o))
            .and_then(|o| o.review.as_ref())
            .into_iter()
            .flat_map(|r| {
                [
                    r.review_first.as_str(),
                    &r.review_round,
                    &r.respond,
                    &r.respond_to_user,
                    &r.handoff,
                ]
            });
        guidance
            .chain(stage.prompt.as_deref())
            .chain(stage.review_prompt.as_deref())
            .chain(stage.fix_prompt.as_deref())
            .chain(workflow)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The decisions a gate of this file asks: a human gate's, an
    /// external gate's, and `merge` for a `pr-merged` gate that names
    /// none.
    #[must_use]
    pub fn gate_decisions(&self) -> Vec<&str> {
        self.stages
            .iter()
            .filter_map(|s| match &s.gate {
                Some(Gate::Human { decision, .. }) => Some(decision.as_str()),
                Some(Gate::External {
                    decision: Some(d), ..
                }) => Some(d.as_str()),
                Some(Gate::External {
                    check,
                    decision: None,
                    ..
                }) if check == "pr-merged" => Some(crate::scheduler::DEFAULT_MERGE),
                _ => None,
            })
            .collect()
    }

    fn validate_supervisor(&self, sup: &Supervisor) -> Result<()> {
        if sup.guidance.trim().is_empty() {
            bail!("[supervisor] guidance is empty; say what the supervisor is for");
        }
        if let Some(path) = sup.read.iter().find(|p| p.is_absolute()) {
            bail!(
                "[supervisor] read: {} is absolute; paths are relative to the workspace",
                path.display()
            );
        }
        if let SupervisorSetup::Many(entries) = &sup.setup
            && entries.iter().any(|e| e.argv.is_empty())
        {
            bail!("[supervisor] setup: an entry has an empty argv");
        }
        let asked = self.gate_decisions();
        for name in &sup.decides {
            if crate::scheduler::DECISIONS.contains(&name.as_str())
                || asked.contains(&name.as_str())
            {
                continue;
            }
            if let Some((_, takes)) = ANSWERS.iter().find(|(a, _)| a == name) {
                bail!("[supervisor] decides: `{name}` is an answer to {takes}; name the decision");
            }
            bail!(
                "[supervisor] decides: `{name}` is not a decision Dispatch asks or a gate of this file asks"
            );
        }
        let known = crate::supervisor::CAPABILITIES;
        if let Some(x) = sup.may.iter().find(|x| !known.contains(&x.as_str())) {
            bail!(
                "[supervisor] may: `{x}` is not a capability Dispatch knows ({})",
                known.join(", ")
            );
        }
        Ok(())
    }
}

/// A pipeline with lanes `A` and `B` and a claude operator `agent`,
/// then `rest` (its stages and any other operators), for tests.
#[cfg(test)]
pub(crate) fn two_lanes(rest: &str) -> Pipeline {
    Pipeline::parse(&format!(
        r#"
version = 1

[project]
name = "P"
repo = "git@example.com:o/p.git"
space = "Dispatch · P"

[source]
kind = "github"
repo = "o/p"
label = "dispatch"

[[lanes]]
name = "A"
path = "a"

[[lanes]]
name = "B"
path = "b"

[operators.agent]
kind = "claude"
{rest}"#
    ))
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::Network;

    const SWITCHBOARD: &str = r#"
version = 1

[project]
name = "Switchboard"
repo = "git@example.com:msull/switchboard.git"
space = "Dispatch · Switchboard"

[source]
kind = "github"
repo = "msull/switchboard"
label = "dispatch"

[[lanes]]
name = "repo"
path = "."
setup = ["cargo", "fetch", "--locked"]

[operators.investigator]
kind = "claude"
guidance = "Read CLAUDE.md first."

[operators.planner]
kind = "claude"

[operators.reviewer]
kind = "codex"
[operators.reviewer.review]
reviewer = "codex"
review_first = "Review {plan}; write to {feedback}; else {no_feedback}"
review_round = "Again {response} {plan} {feedback} {no_feedback}"
respond = "Feedback at {feedback}; edit {plan}; answer at {response}."
respond_to_user = "{text} {plan} {response}"
handoff = "The plan at {plan} is final."
no_feedback = "No further feedback."
cap = 4

[operators.implementer]
kind = "claude"

[[stages]]
name = "investigate"
operator = "investigator"
context = "root"
writes = ["notes"]
prompt = "Issue #{issue.number}: {issue.title}\n\n{issue.body}\n\nWrite to {notes}."

[[stages]]
name = "lanes"
gate = { kind = "human", decision = "lanes" }

[[stages]]
name = "plan"
operator = "planner"
context = "each"
writes = ["plan"]
prompt = "Using {inputs.notes}, plan #{issue.number} to {plan} on {branch}."

[[stages]]
name = "review"
review = "reviewer"
context = "each"
subject = "plan"
gate = { kind = "external", check = "review-finalized" }

[[stages]]
name = "implement"
operator = "implementer"
context = "each"
writes = ["notes"]
prompt = "Implement {inputs.plan} on {branch}; PR number to {notes}."
gate = { kind = "command", argv = ["sh", "-c", "cargo test"], in = "lane" }

[[stages]]
name = "ready"
context = "each"
gate = { kind = "external", check = "pr-checks" }

[[stages]]
name = "merge"
context = "each"
gate = { kind = "external", check = "pr-merged", decision = "merge" }

[policy]
slots = 1
waiting_on_me = 2
rates = { "claude-sonnet-5" = [3.0, 15.0] }
decisions = { lanes = "auto", finalize = "ask", budget = "ask" }
"#;

    /// The Switchboard pipeline with `table` appended as its
    /// `[supervisor]`.
    fn supervised(table: &str) -> Result<Pipeline> {
        Pipeline::parse(&format!("{SWITCHBOARD}\n[supervisor]\n{table}"))
    }

    #[test]
    fn a_supervisor_table_reads_in_both_setup_shapes() {
        let p = supervised(
            r#"
guidance = "Keep the queue moving."
read = ["CLAUDE.md"]
setup = ["git", "clone", "git@example.com:o/r.git", "."]
model = "haiku"
decides = ["finalize", "rerun", "merge", "lanes"]
"#,
        )
        .unwrap();
        let sup = p.supervisor.unwrap();
        assert_eq!(sup.model.as_deref(), Some("haiku"));
        assert_eq!(
            sup.setup.argvs(),
            [vec!["git", "clone", "git@example.com:o/r.git", "."]]
        );
        let p = supervised(
            r#"
guidance = "Keep the queue moving."
decides = []
[[supervisor.setup]]
argv = ["git", "clone", "git@example.com:o/r.git", "."]
[[supervisor.setup]]
argv = ["make", "deps"]
"#,
        )
        .unwrap();
        assert_eq!(
            p.supervisor.unwrap().setup.argvs(),
            [
                vec!["git", "clone", "git@example.com:o/r.git", "."],
                vec!["make", "deps"]
            ]
        );
        let none = supervised("guidance = \"g\"").unwrap().supervisor.unwrap();
        assert!(none.setup.argvs().is_empty());
        assert!(none.decides.is_empty());
    }

    #[test]
    fn a_file_without_a_supervisor_table_parses_as_before() {
        assert_eq!(Pipeline::parse(SWITCHBOARD).unwrap().supervisor, None);
    }

    #[test]
    fn decides_names_decisions_and_only_decisions() {
        let err = |table: &str| supervised(table).unwrap_err().to_string();
        let e = err("guidance = \"g\"\ndecides = [\"approve\"]");
        assert!(e.contains("`approve` is not a decision"), "{e}");
        let e = err("guidance = \"g\"\ndecides = [\"recheck\"]");
        assert_eq!(
            e,
            "[supervisor] decides: `recheck` is an answer to `pr` and `refresh`; name the decision"
        );
        // A gate of this file asks it: a human gate's own name.
        supervised("guidance = \"g\"\ndecides = [\"lanes\", \"merge\"]").unwrap();
    }

    #[test]
    fn may_names_capabilities_and_is_not_written_when_empty() {
        let sup = supervised("guidance = \"g\"\nmay = [\"runner\"]")
            .unwrap()
            .supervisor
            .unwrap();
        assert_eq!(sup.may, ["runner"]);
        assert!(
            toml::to_string(&sup)
                .unwrap()
                .contains("may = [\"runner\"]")
        );
        let e = supervised("guidance = \"g\"\nmay = [\"deploy\"]")
            .unwrap_err()
            .to_string();
        assert_eq!(
            e,
            "[supervisor] may: `deploy` is not a capability Dispatch knows (runner)"
        );
        let none = supervised("guidance = \"g\"").unwrap().supervisor.unwrap();
        assert!(!toml::to_string(&none).unwrap().contains("may"));
    }

    #[test]
    fn merge_is_a_decision_only_where_a_gate_asks_it() {
        let table = "\n[supervisor]\nguidance = \"g\"\ndecides = [\"merge\"]";
        // Named on the gate.
        Pipeline::parse(&format!("{SWITCHBOARD}{table}")).unwrap();
        // A `pr-merged` gate that names none asks `merge`.
        let unnamed = SWITCHBOARD.replace(
            r#"check = "pr-merged", decision = "merge" }"#,
            r#"check = "pr-merged" }"#,
        );
        assert_ne!(unnamed, SWITCHBOARD);
        Pipeline::parse(&format!("{unnamed}{table}")).unwrap();
        // No gate asks it.
        let none = SWITCHBOARD.replace(
            r#"gate = { kind = "external", check = "pr-merged", decision = "merge" }"#,
            r#"gate = { kind = "human", decision = "ship" }"#,
        );
        assert_ne!(none, SWITCHBOARD);
        let e = Pipeline::parse(&format!("{none}{table}"))
            .unwrap_err()
            .to_string();
        assert!(e.contains("`merge` is not a decision"), "{e}");
    }

    #[test]
    fn a_supervisor_needs_guidance_and_relative_reads() {
        let e = supervised("guidance = \"  \"").unwrap_err().to_string();
        assert!(e.contains("guidance is empty"), "{e}");
        let e = supervised("guidance = \"g\"\nread = [\"/etc/passwd\"]")
            .unwrap_err()
            .to_string();
        assert!(e.contains("/etc/passwd is absolute"), "{e}");
        let e = supervised("guidance = \"g\"\n[[supervisor.setup]]\nargv = []")
            .unwrap_err()
            .to_string();
        assert!(e.contains("empty argv"), "{e}");
    }

    #[test]
    fn the_switchboard_pipeline_parses_whole() {
        let p = Pipeline::parse(SWITCHBOARD).unwrap();
        assert_eq!(p.stages.len(), 7);
        assert_eq!(p.stages[0].kind(), StageKind::Agent);
        assert_eq!(p.stages[1].kind(), StageKind::GateOnly);
        assert_eq!(p.stages[3].kind(), StageKind::Workflow);
        assert_eq!(p.stages[2].context, Context::Each);
        assert_eq!(p.dial("lanes"), "auto");
        assert_eq!(p.dial("merge"), "ask");
        assert!(matches!(&p.source, Source::Github { repo, .. } if repo == "msull/switchboard"));
        assert_eq!(
            p.operators["reviewer"].review.as_ref().unwrap().cap,
            Some(4)
        );
        assert!(matches!(
            &p.stages[4].gate,
            Some(Gate::Command { run_in, .. }) if run_in == "lane"
        ));
        assert_eq!(Pipeline::fingerprint(SWITCHBOARD).len(), 12);
    }

    #[test]
    fn contexts_read_every_spelling() {
        let p = Pipeline::parse(
            &SWITCHBOARD.replace(r#"context = "root""#, r#"context = "lane:repo""#),
        )
        .unwrap();
        assert_eq!(p.stages[0].context, Context::Lane("repo".into()));
        let p =
            Pipeline::parse(&SWITCHBOARD.replace(r#"context = "root""#, r#"context = ["repo"]"#))
                .unwrap();
        assert_eq!(p.stages[0].context, Context::Lanes(vec!["repo".into()]));
        assert!(
            Pipeline::parse(&SWITCHBOARD.replace(r#"context = "root""#, r#"context = "sideways""#))
                .is_err()
        );
    }

    #[test]
    fn broken_references_are_refused_with_the_stage_named() {
        let cases = [
            (
                r#"operator = "investigator""#,
                r#"operator = "nobody""#,
                "unknown operator",
            ),
            (
                r#"review = "reviewer""#,
                r#"review = "planner""#,
                "no [operators.planner.review]",
            ),
            (
                r#"subject = "plan""#,
                r#"subject = "draft""#,
                "no earlier stage writes",
            ),
            (
                r#"context = "each"
writes = ["plan"]"#,
                r#"context = "lane:nope"
writes = ["plan"]"#,
                "unknown lane",
            ),
            (r#"lanes = "auto""#, r#"lanes = "maybe""#, "the dial"),
            ("version = 1", "version = 2", "version 2"),
            (
                r#"check = "review-finalized" }"#,
                r#"check = "pr-green" }"#,
                "not a check Dispatch knows",
            ),
            (
                r#"check = "review-finalized" }"#,
                r#"check = "review-finalized", checks = "none" }"#,
                r#"only pr-checks takes checks = "none""#,
            ),
            (
                r#"check = "review-finalized" }"#,
                r#"check = "pr-checks", checks = "some" }"#,
                r#"only pr-checks takes checks = "none""#,
            ),
        ];
        for (from, to, expected) in cases {
            let text = SWITCHBOARD.replace(from, to);
            assert_ne!(text, SWITCHBOARD, "{from}");
            let err = Pipeline::parse(&text).unwrap_err().to_string();
            assert!(err.contains(expected), "{from}: {err}");
        }
    }

    #[test]
    fn a_resolution_reviewer_must_be_an_operator() {
        let text =
            SWITCHBOARD.replace("[policy]\n", "[policy]\nresolution_reviewer = \"nobody\"\n");
        assert_ne!(text, SWITCHBOARD);
        let err = Pipeline::parse(&text).unwrap_err().to_string();
        assert!(
            err.contains("resolution_reviewer names an unknown operator \"nobody\""),
            "{err}"
        );
        let text = SWITCHBOARD.replace(
            "[policy]\n",
            "[policy]\nresolution_reviewer = \"planner\"\n",
        );
        let p = Pipeline::parse(&text).unwrap();
        assert_eq!(p.policy.resolution_reviewer.as_deref(), Some("planner"));
    }

    /// A pipeline for someone else's pull requests reads their
    /// branches and pushes nothing: no rebaser, no fixer.
    #[test]
    fn a_pull_request_pipeline_names_no_operator_that_pushes() {
        let text = SWITCHBOARD
            .replace(
                "[source]\nkind = \"github\"\nrepo = \"msull/switchboard\"\nlabel = \"dispatch\"\n",
                "[source]\nkind = \"pull-request\"\n",
            )
            .replace("[policy]\n", "[policy]\nfixer = \"planner\"\n");
        let err = Pipeline::parse(&text).unwrap_err().to_string();
        assert!(err.contains("pushes nothing"), "{err}");
        let ok = text.replace("fixer = \"planner\"\n", "");
        assert_eq!(Pipeline::parse(&ok).unwrap().source, Source::PullRequest);
    }

    /// `SWITCHBOARD` with a code review stage after `implement`, its
    /// review keys given as `keys`.
    fn with_review_stage(keys: &str) -> String {
        SWITCHBOARD.replace(
            "[[stages]]\nname = \"ready\"\n",
            &format!(
                "[[stages]]\nname = \"review-code\"\ncontext = \"each\"\nreviewers = [\"planner\"]\nimplementer = \"implementer\"\n{keys}gate = {{ kind = \"command\", like = \"implement\" }}\n\n[[stages]]\nname = \"ready\"\n"
            ),
        )
    }

    #[test]
    fn style_rounds_has_a_default_and_is_refused_at_zero() {
        let p = Pipeline::parse(&with_review_stage("")).unwrap();
        let stage = p.stages.iter().find(|s| s.name == "review-code").unwrap();
        assert_eq!(stage.style_rounds(), DEFAULT_STYLE_ROUNDS);
        let p = Pipeline::parse(&with_review_stage("style_rounds = 3\n")).unwrap();
        let stage = p.stages.iter().find(|s| s.name == "review-code").unwrap();
        assert_eq!(stage.style_rounds(), 3);
        let err = Pipeline::parse(&with_review_stage("style_rounds = 0\n"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("style_rounds must be at least 1"), "{err}");
    }

    #[test]
    fn style_rounds_without_reviewers_is_refused() {
        let text = SWITCHBOARD.replace(
            "prompt = \"Implement {inputs.plan}",
            "style_rounds = 2\nprompt = \"Implement {inputs.plan}",
        );
        assert_ne!(text, SWITCHBOARD);
        let err = Pipeline::parse(&text).unwrap_err().to_string();
        assert!(
            err.contains("has implementer, cap, style_rounds or commits but no reviewers"),
            "{err}"
        );
    }

    #[test]
    fn commits_defaults_to_keep_and_reads_each_mode() {
        let stage_of = |p: &Pipeline| {
            p.stages
                .iter()
                .find(|s| s.name == "review-code")
                .unwrap()
                .commits()
        };
        let p = Pipeline::parse(&with_review_stage("")).unwrap();
        assert_eq!(stage_of(&p), Commits::Keep);
        for (text, mode) in [
            ("keep", Commits::Keep),
            ("fold", Commits::Fold),
            ("one", Commits::One),
        ] {
            let keys = format!("commits = \"{text}\"\n");
            let p = Pipeline::parse(&with_review_stage(&keys)).unwrap();
            assert_eq!(stage_of(&p), mode);
        }
    }

    #[test]
    fn an_unknown_commits_value_is_refused() {
        let err = Pipeline::parse(&with_review_stage("commits = \"squash\"\n")).unwrap_err();
        assert!(format!("{err:#}").contains("commits"), "{err:#}");
    }

    #[test]
    fn commits_without_reviewers_is_refused() {
        let text = SWITCHBOARD.replace(
            "prompt = \"Implement {inputs.plan}",
            "commits = \"fold\"\nprompt = \"Implement {inputs.plan}",
        );
        assert_ne!(text, SWITCHBOARD);
        let err = Pipeline::parse(&text).unwrap_err().to_string();
        assert!(
            err.contains("has implementer, cap, style_rounds or commits but no reviewers"),
            "{err}"
        );
    }

    #[test]
    fn a_pull_request_pipeline_refuses_commits_other_than_keep() {
        let pull_request = |keys: &str| {
            with_review_stage(keys).replace(
                "[source]\nkind = \"github\"\nrepo = \"msull/switchboard\"\nlabel = \"dispatch\"\n",
                "[source]\nkind = \"pull-request\"\n",
            )
        };
        let err = Pipeline::parse(&pull_request("commits = \"fold\"\n"))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("a pull-request pipeline rewrites no history"),
            "{err}"
        );
        Pipeline::parse(&pull_request("commits = \"keep\"\n")).unwrap();
    }

    #[test]
    fn on_dirty_defaults_to_one_nudge_and_reads_both_spellings() {
        let p = Pipeline::parse(SWITCHBOARD).unwrap();
        assert_eq!(p.policy.on_dirty, OnDirty::Nudge(1));
        let implement = |p: &Pipeline| p.stages.iter().find(|s| s.name == "implement").cloned();
        assert_eq!(p.on_dirty(&implement(&p).unwrap()), OnDirty::Nudge(1));
        let text = SWITCHBOARD.replace("slots = 1\n", "slots = 1\non_dirty = \"ask\"\n");
        let p = Pipeline::parse(&text).unwrap();
        assert_eq!(p.policy.on_dirty, OnDirty::Ask);
        let text = text.replace(
            "prompt = \"Implement {inputs.plan}",
            "on_dirty = { nudge = 2 }\nprompt = \"Implement {inputs.plan}",
        );
        let p = Pipeline::parse(&text).unwrap();
        let stage = implement(&p).unwrap();
        assert_eq!(stage.on_dirty, Some(OnDirty::Nudge(2)));
        assert_eq!(p.on_dirty(&stage), OnDirty::Nudge(2));
        let p = Pipeline::parse(&with_review_stage("on_dirty = \"ask\"\n")).unwrap();
        let stage = p.stages.iter().find(|s| s.name == "review-code").unwrap();
        assert_eq!(p.on_dirty(stage), OnDirty::Ask);
    }

    #[test]
    fn on_dirty_refuses_zero_nudges_and_unknown_words() {
        for value in ["{ nudge = 0 }", "\"retry\""] {
            let text =
                SWITCHBOARD.replace("slots = 1\n", &format!("slots = 1\non_dirty = {value}\n"));
            let err = format!("{:#}", Pipeline::parse(&text).unwrap_err());
            assert!(err.contains("on_dirty") && err.contains("\"ask\""), "{err}");
            let text = with_review_stage(&format!("on_dirty = {value}\n"));
            let err = format!("{:#}", Pipeline::parse(&text).unwrap_err());
            assert!(err.contains("on_dirty") && err.contains("\"ask\""), "{err}");
        }
    }

    #[test]
    fn on_dirty_is_refused_on_a_stage_without_an_agent_behind_a_command_gate() {
        let expect = "on_dirty needs an agent stage with a command gate, or a code review stage";
        for (anchor, stage) in [
            ("subject = \"plan\"\n", "review"),
            (
                "gate = { kind = \"human\", decision = \"lanes\" }\n",
                "lanes",
            ),
            ("writes = [\"plan\"]\n", "plan"),
        ] {
            let text = SWITCHBOARD.replacen(anchor, &format!("{anchor}on_dirty = \"ask\"\n"), 1);
            assert_ne!(text, SWITCHBOARD);
            let err = Pipeline::parse(&text).unwrap_err().to_string();
            assert!(err.contains(expect) && err.contains(stage), "{err}");
        }
        Pipeline::parse(&with_review_stage("on_dirty = { nudge = 3 }\n")).unwrap();
    }

    #[test]
    fn writable_defaults_to_empty_and_expands_home() {
        let p = Pipeline::parse(SWITCHBOARD).unwrap();
        assert!(p.lanes[0].writable.is_empty());
        let text = SWITCHBOARD.replace(
            "setup = [\"cargo\", \"fetch\", \"--locked\"]\n",
            "setup = [\"cargo\", \"fetch\", \"--locked\"]\nwritable = [\"~/.cargo\", \"/opt/cache\"]\n",
        );
        let p = Pipeline::parse(&text).unwrap();
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        assert_eq!(
            p.lanes[0].writable,
            vec![home.join(".cargo"), PathBuf::from("/opt/cache")]
        );
    }

    #[test]
    fn confine_is_off_and_the_network_allowed_when_absent() {
        let p = Pipeline::parse(SWITCHBOARD).unwrap();
        assert!(!p.policy.confine);
        assert_eq!(p.policy.network, Network::Allow);
        // No `[policy]` table at all: `Policy::default()` must agree
        // with the serde default.
        let start = SWITCHBOARD.find("[policy]").unwrap();
        let p = Pipeline::parse(&SWITCHBOARD[..start]).unwrap();
        assert!(!p.policy.confine);
        assert_eq!(p.policy.network, Network::Allow);
        let text = SWITCHBOARD.replace(
            "slots = 1\n",
            "slots = 1\nconfine = true\nnetwork = \"deny\"\n",
        );
        let p = Pipeline::parse(&text).unwrap();
        assert!(p.policy.confine);
        assert_eq!(p.policy.network, Network::Deny);
    }

    #[test]
    fn network_is_allow_or_deny_on_the_policy_and_on_a_gate() {
        let text = SWITCHBOARD.replace("slots = 1\n", "slots = 1\nnetwork = \"local\"\n");
        let err = format!("{:#}", Pipeline::parse(&text).unwrap_err());
        assert!(err.contains("unknown variant `local`"), "{err}");
        let gate =
            "gate = { kind = \"command\", argv = [\"sh\", \"-c\", \"cargo test\"], in = \"lane\" }";
        let with = |value: &str| {
            SWITCHBOARD.replace(
                gate,
                &gate.replace(" }", &format!(", network = \"{value}\" }}")),
            )
        };
        assert_ne!(with("deny"), SWITCHBOARD);
        let p = Pipeline::parse(&with("deny")).unwrap();
        let stage = p.stages.iter().find(|s| s.name == "implement").unwrap();
        assert!(matches!(
            &stage.gate,
            Some(Gate::Command {
                network: Some(Network::Deny),
                ..
            })
        ));
        let err = format!("{:#}", Pipeline::parse(&with("off")).unwrap_err());
        assert!(err.contains("unknown variant `off`"), "{err}");
    }

    #[test]
    fn a_gate_given_by_like_takes_its_network_from_the_gate_it_names() {
        let text = with_review_stage("").replace(
            "like = \"implement\" }",
            "like = \"implement\", network = \"allow\" }",
        );
        let err = format!("{:#}", Pipeline::parse(&text).unwrap_err());
        assert!(
            err.contains("review-code") && err.contains("takes its network from implement"),
            "{err}"
        );
    }

    /// The back half of a multi-repository pipeline that deploys: a
    /// deploy in the backend lane, a tester with two served lanes, and a
    /// confirmation, all holding `my-dev`.
    const DEPLOYING_BACK_HALF: &str = r#"
version = 1

[project]
name = "Orchard"
repo = "git@github.com:example-org/orchard-workspace.git"
space = "Dispatch · Orchard"

[source]
kind = "github"
repo = "example-org/orchard-workspace"
label = "dispatch"

[[lanes]]
name = "backend"
path = "orchard-backend"
repo = "git@example.invalid:example/backend.git"
base = "main"

[[lanes]]
name = "frontend"
path = "orchard-frontend"
repo = "git@example.invalid:example/frontend.git"
base = "dev"
serve = { argv = ["npm", "start"], env = { BROWSER = "none", PORT = "{port}" }, url = "http://localhost:{port}", ready = { http = "/", within_secs = 120 } }

[[lanes]]
name = "admin"
path = "orchard-admin"
repo = "git@example.invalid:example/admin.git"
base = "master"
serve = { argv = ["npm", "start"], env = { BROWSER = "none", PORT = "{port}" }, url = "http://localhost:{port}", ready = { http = "/", within_secs = 120 } }

[[resources]]
name = "my-dev"
count = 1

[operators.implementer]
kind = "claude"

[operators.tester]
kind = "claude"

[[stages]]
name = "implement"
operator = "implementer"
context = "each"
prompt = "Implement the {lane} part on {branch}."

[[stages]]
name = "inspect"
context = "each"
gate = { kind = "human", decision = "inspect" }

[[stages]]
name = "deploy"
context = "lane:backend"
needs = ["my-dev"]
gate = { kind = "command", in = "lane:backend", argv = ["sh", "-c", "uv run --frozen inv link-env --env-name my-dev && aws-vault exec -n orchard-dev -- uv run inv deploy -f"] }

[[stages]]
name = "try"
operator = "tester"
context = "joined"
needs = ["my-dev"]
services = ["frontend", "admin"]
before = { frontend = ["npm", "run", "link-env"], admin = ["npm", "run", "link-env"] }
writes = ["notes"]
prompt = "my-dev is running backend commit {inputs.deploy.commit}. The frontend: {services.frontend}. The admin frontend: {services.admin}. Report to {notes}."

[[stages]]
name = "tried"
needs = ["my-dev"]
gate = { kind = "human", decision = "tried", confirm = true }

[[stages]]
name = "pr"
operator = "implementer"
context = "each"
prompt = "Push {branch}."

[policy]
slots = 2
ports = [3100, 3199]
"#;

    #[test]
    fn a_deploying_back_half_parses_deploy_try_and_tried() {
        let p = Pipeline::parse(DEPLOYING_BACK_HALF).unwrap();
        assert_eq!(p.hold_range("my-dev"), Some((2, 4)));
        assert_eq!(p.services_until(3), 4);
        assert_eq!(p.services_until(5), 5, "a stage holding nothing");
        assert_eq!(p.stages[2].kind(), StageKind::GateOnly);
        assert_eq!(p.stages[2].context, Context::Lane("backend".into()));
        assert_eq!(p.stages[3].services, ["frontend", "admin"]);
        assert_eq!(p.hold_range("backend"), None, "a lane is not a resource");
    }

    fn back_half_refused(from: &str, to: &str, expected: &str) {
        let text = DEPLOYING_BACK_HALF.replace(from, to);
        assert_ne!(text, DEPLOYING_BACK_HALF, "{from}");
        let err = Pipeline::parse(&text).unwrap_err().to_string();
        assert!(err.contains(expected), "{from}: {err}");
    }

    /// The back half with a gate-only `try-setup` before `try` that
    /// writes a secret and a plain artifact.
    fn secret_half() -> String {
        DEPLOYING_BACK_HALF.replace(
            "[[stages]]\nname = \"try\"\n",
            "[[stages]]\nname = \"try-setup\"\ncontext = \"lane:backend\"\nneeds = [\"my-dev\"]\nwrites = [{ name = \"personas\", secret = true }, \"seed\"]\ngate = { kind = \"command\", in = \"lane:backend\", argv = [\"make\", \"personas\"] }\n\n[[stages]]\nname = \"try\"\n",
        )
    }

    fn secret_half_refused(from: &str, to: &str, expected: &str) {
        let base = secret_half();
        let text = base.replace(from, to);
        assert_ne!(text, base, "{from}");
        let err = Pipeline::parse(&text).unwrap_err().to_string();
        assert!(err.contains(expected), "{from}: {err}");
    }

    #[test]
    fn writes_take_a_name_or_a_table_and_a_plain_one_writes_back_as_a_name() {
        #[derive(Serialize, Deserialize)]
        struct Writes {
            writes: Vec<Write>,
        }
        let p = Pipeline::parse(&secret_half()).unwrap();
        let setup = &p.stages[3];
        assert_eq!(setup.name, "try-setup");
        assert_eq!(
            setup.writes,
            [
                Write {
                    name: "personas".into(),
                    secret: true
                },
                Write::named("seed")
            ]
        );
        assert_eq!(setup.secret_writes().collect::<Vec<_>>(), ["personas"]);
        assert_eq!(p.needs_range(3), Some((2, 5)));
        assert_eq!(p.needs_range(6), None);
        assert_eq!(
            serde_json::to_value(&setup.writes).unwrap(),
            serde_json::json!([{ "name": "personas", "secret": true }, "seed"])
        );
        let back = toml::to_string(&Writes {
            writes: setup.writes.clone(),
        })
        .unwrap();
        assert!(back.contains("\"seed\""), "{back}");
        assert_eq!(
            toml::from_str::<Writes>(&back).unwrap().writes,
            setup.writes
        );
        assert_eq!(env_key("try-setup.personas"), "TRY_SETUP_PERSONAS");
    }

    #[test]
    fn writes_and_secrets_are_refused_where_nothing_could_write_or_forget_them() {
        let gate =
            "gate = { kind = \"command\", in = \"lane:backend\", argv = [\"make\", \"personas\"] }";
        secret_half_refused(
            gate,
            "gate = { kind = \"human\", decision = \"setup\" }",
            "writes only with a command gate",
        );
        secret_half_refused(
            gate,
            "gate = { kind = \"command\", in = \"lane:backend\", like = \"deploy\" }",
            "writes only with a command gate",
        );
        secret_half_refused("\"seed\"]", "\"checks\"]", "its command's own log");
        secret_half_refused(
            "context = \"lane:backend\"\nneeds = [\"my-dev\"]\nwrites = [{",
            "context = \"each\"\nneeds = [\"my-dev\"]\nwrites = [{",
            "writes only in one context",
        );
        secret_half_refused("\"seed\"]", "\"commit\"]", "inputs.try-setup.commit");
        secret_half_refused("\"seed\"]", "\"Personas\"]", "the same variable name");
        secret_half_refused(
            "writes = [\"notes\"]",
            "writes = [{ name = \"token\", secret = true }]",
            "written by a gate-only command",
        );
        secret_half_refused(
            "{ name = \"personas\", secret = true }",
            "{ name = \"summary\", secret = true }",
            "read by Dispatch",
        );
        secret_half_refused(
            "needs = [\"my-dev\"]\nwrites = [{",
            "needs = [\"backend\"]\nwrites = [{",
            "needs a [[resources]] entry",
        );
    }

    #[test]
    fn a_secret_is_never_a_reviews_subject() {
        let mut p = Pipeline::parse(&secret_half()).unwrap();
        p.operators.insert(
            "reviewer".into(),
            toml::from_str(
                "kind = \"codex\"\n[review]\nreviewer = \"codex\"\nreview_first = \"r\"\nreview_round = \"r\"\nrespond = \"r\"\nrespond_to_user = \"r\"\nhandoff = \"h\"\nno_feedback = \"n\"\n",
            )
            .unwrap(),
        );
        let mut review: Stage = toml::from_str(
            "name = \"check-personas\"\nreview = \"reviewer\"\nsubject = \"personas\"\n",
        )
        .unwrap();
        review.context = Context::Root;
        p.stages.push(review);
        let err = p.validate().unwrap_err().to_string();
        assert!(err.contains("which is secret and never read"), "{err}");
    }

    #[test]
    fn needs_must_be_contiguous() {
        // `tried` dropped from the run leaves `deploy`..`try`, which is
        // contiguous; a hole in the middle is not.
        Pipeline::parse(&DEPLOYING_BACK_HALF.replace(
            "name = \"tried\"\nneeds = [\"my-dev\"]\n",
            "name = \"tried\"\n",
        ))
        .unwrap();
        back_half_refused(
            "name = \"try\"\noperator = \"tester\"\ncontext = \"joined\"\nneeds = [\"my-dev\"]\nservices = [\"frontend\", \"admin\"]\nbefore = { frontend = [\"npm\", \"run\", \"link-env\"], admin = [\"npm\", \"run\", \"link-env\"] }\n",
            "name = \"try\"\noperator = \"tester\"\ncontext = \"joined\"\n",
            "breaks the run of stages needing \"my-dev\"",
        );
    }

    #[test]
    fn a_review_or_workflow_stands_only_last_in_a_needs_run() {
        let mut p = Pipeline::parse(DEPLOYING_BACK_HALF).unwrap();
        p.stages[4].reviewers = vec!["correctness".into()];
        p.validate_contiguous_needs().unwrap();
        p.stages[4].reviewers.clear();
        p.stages[3].operator = None;
        p.stages[3].review = Some("tester".into());
        let err = p.validate_contiguous_needs().unwrap_err().to_string();
        assert!(
            err.contains("stage \"try\" is a review or workflow stage"),
            "{err}"
        );
    }

    #[test]
    fn services_name_served_lanes_and_need_a_resource_and_ports() {
        back_half_refused(
            "services = [\"frontend\", \"admin\"]",
            "services = [\"frontend\", \"backend\"]",
            "which has no serve",
        );
        back_half_refused(
            "services = [\"frontend\", \"admin\"]",
            "services = [\"frontend\", \"frontend\"]",
            "twice",
        );
        back_half_refused(
            "before = { frontend = [\"npm\", \"run\", \"link-env\"], admin",
            "before = { backend = [\"npm\", \"run\", \"link-env\"], admin",
            "not in its services",
        );
        back_half_refused("ports = [3100, 3199]\n", "", "has no ports");
        back_half_refused("ports = [3100, 3199]", "ports = [3199, 3100]", "is empty");
        back_half_refused(
            "url = \"http://localhost:{port}\", ready = { http = \"/\", within_secs = 120 } }\n\n[[lanes]]\nname = \"admin\"",
            "url = \"http://localhost:3000\", ready = { http = \"/\", within_secs = 120 } }\n\n[[lanes]]\nname = \"admin\"",
            "serve.url must contain {port}",
        );
        back_half_refused(
            "ready = { http = \"/\", within_secs = 120 } }\n\n[[lanes]]\nname = \"admin\"",
            "ready = { http = \"health\", within_secs = 120 } }\n\n[[lanes]]\nname = \"admin\"",
            "must start with /",
        );
        back_half_refused(
            "PORT = \"{port}\" }, url = \"http://localhost:{port}\", ready = { http = \"/\", within_secs = 120 } }\n\n[[lanes]]\nname = \"admin\"",
            "PORT = \"{port}\", WHO = \"{ticket}\" }, url = \"http://localhost:{port}\", ready = { http = \"/\", within_secs = 120 } }\n\n[[lanes]]\nname = \"admin\"",
            "the only field a value may name is {port}",
        );
        // Services live until the resource's range ends, so a stage
        // that serves must hold a resource.
        let text = DEPLOYING_BACK_HALF
            .replace(
                "context = \"lane:backend\"\nneeds = [\"my-dev\"]\n",
                "context = \"lane:backend\"\n",
            )
            .replace(
                "context = \"joined\"\nneeds = [\"my-dev\"]\n",
                "context = \"joined\"\n",
            )
            .replace(
                "name = \"tried\"\nneeds = [\"my-dev\"]\n",
                "name = \"tried\"\n",
            );
        let err = Pipeline::parse(&text).unwrap_err().to_string();
        assert!(err.contains("needs no [[resources]] entry"), "{err}");
    }

    #[test]
    fn serve_env_refuses_a_secret_looking_key() {
        for key in [
            "API_TOKEN",
            "client_secret",
            "DB_PASSWORD",
            "AWS_KEY",
            "Credentials",
        ] {
            back_half_refused(
                "BROWSER = \"none\", PORT = \"{port}\" }, url = \"http://localhost:{port}\", ready = { http = \"/\", within_secs = 120 } }\n\n[[lanes]]\nname = \"admin\"",
                &format!(
                    "BROWSER = \"none\", {key} = \"x\", PORT = \"{{port}}\" }}, url = \"http://localhost:{{port}}\", ready = {{ http = \"/\", within_secs = 120 }} }}\n\n[[lanes]]\nname = \"admin\""
                ),
                &format!("serve.env {key} looks like a secret"),
            );
        }
    }

    #[test]
    fn a_gate_only_command_runs_in_its_context() {
        back_half_refused(
            "name = \"deploy\"\ncontext = \"lane:backend\"",
            "name = \"deploy\"\ncontext = \"joined\"",
            "needs context = \"lane:backend\"",
        );
    }

    #[test]
    fn contiguity_is_checked_for_resources_not_lanes() {
        // A lane named in `needs` (the in-place hold, not built) parses
        // wherever it sits, gaps and all; it parks at run time.
        let text = DEPLOYING_BACK_HALF
            .replace(
                "name = \"implement\"\noperator = \"implementer\"\n",
                "name = \"implement\"\noperator = \"implementer\"\nneeds = [\"backend\"]\n",
            )
            .replace(
                "name = \"tried\"\nneeds = [\"my-dev\"]\n",
                "name = \"tried\"\nneeds = [\"my-dev\", \"backend\"]\n",
            );
        let p = Pipeline::parse(&text).unwrap();
        assert_eq!(p.hold_range("backend"), None);
    }

    /// The Switchboard pipeline with `extra` appended, parsed.
    fn with(extra: &str) -> Result<Pipeline> {
        Pipeline::parse(&format!("{SWITCHBOARD}\n{extra}"))
    }

    fn refused(extra: &str, expected: &[&str]) {
        let err = with(extra).unwrap_err().to_string();
        for e in expected {
            assert!(err.contains(e), "{extra}: {err}");
        }
    }

    /// A joined agent stage `test` whose prompt is `prompt`.
    fn joined(prompt: &str) -> String {
        format!(
            "[[stages]]\nname = \"test\"\noperator = \"implementer\"\ncontext = \"joined\"\nprompt = \"{prompt}\"\n"
        )
    }

    #[test]
    fn a_joined_stage_naming_a_per_lane_file_is_refused() {
        refused(
            &joined("Read {inputs.implement.notes}."),
            &[
                "stage \"test\" (joined) names {inputs.implement.notes}",
                "implement writes notes once per lane",
            ],
        );
        // Lane `repo`'s agents read their own lane's file.
        let lane = joined("Read {inputs.implement.notes}.").replace("joined", "lane:repo");
        with(&lane).unwrap();
        // `investigate` writes notes once, in the root, so the bare form
        // has a file to take.
        with(&joined("Read {inputs.notes}.")).unwrap();
    }

    #[test]
    fn a_joined_stage_naming_a_bare_name_only_lanes_write_is_refused() {
        refused(
            &joined("Read {inputs.plan}."),
            &[
                "names {inputs.plan}",
                "plan, review writes plan once per lane",
            ],
        );
        let root_writer = "[[stages]]\nname = \"summarise\"\noperator = \"planner\"\ncontext = \"root\"\nwrites = [\"plan\"]\nprompt = \"Write {plan}.\"\n";
        with(&format!("{root_writer}\n{}", joined("Read {inputs.plan}."))).unwrap();
        // A command gate's log and a code review's summary count as
        // written.
        refused(
            &joined("Read {inputs.checks}."),
            &["names {inputs.checks}", "implement writes checks"],
        );
        refused(
            &format!("{CODE_REVIEW}\n{}", joined("Read {inputs.summary}.")),
            &["names {inputs.summary}", "review-code writes summary"],
        );
        // A workflow writes no summary, so plan review leaves none to take.
        with(&joined("Read {inputs.summary}.")).unwrap();
    }

    /// A code review stage in each lane.
    const CODE_REVIEW: &str = "[[stages]]\nname = \"review-code\"\ncontext = \"each\"\nreviewers = [\"planner\"]\nimplementer = \"implementer\"\ngate = { kind = \"command\", like = \"implement\" }\n";

    #[test]
    fn a_joined_review_s_templates_are_read_like_a_prompt() {
        let joined_review = CODE_REVIEW
            .replace("review-code", "review-all")
            .replace("\"each\"", "\"joined\"");
        refused(
            &format!("{joined_review}review_prompt = \"Against {{inputs.implement.notes}}.\"\n"),
            &["stage \"review-all\" (joined) names {inputs.implement.notes}"],
        );
        refused(
            &format!("{joined_review}fix_prompt = \"Per {{inputs.plan}}.\"\n"),
            &["names {inputs.plan}"],
        );
        let guided = "[operators.critic]\nkind = \"claude\"\nguidance = \"Start from {inputs.implement.notes}.\"\n";
        refused(
            &format!("{guided}\n{}", joined_review.replace("planner", "critic")),
            &["names {inputs.implement.notes}"],
        );
        let workflow = "[[stages]]\nname = \"review-notes\"\nreview = \"reviewer\"\ncontext = \"joined\"\nsubject = \"notes\"\ngate = { kind = \"external\", check = \"review-finalized\" }\n";
        with(workflow).unwrap();
        let handoff = format!("{SWITCHBOARD}\n{workflow}").replace(
            "handoff = \"The plan at {plan} is final.\"",
            "handoff = \"Final; see {inputs.implement.notes}.\"",
        );
        let err = Pipeline::parse(&handoff).unwrap_err().to_string();
        assert!(err.contains("names {inputs.implement.notes}"), "{err}");
    }

    #[test]
    fn guidance_is_read_like_the_prompt() {
        let guided = "[operators.tester]\nkind = \"claude\"\nguidance = \"Start from {inputs.implement.notes}.\"\n";
        let stage = joined("Test it.").replace("implementer", "tester");
        refused(
            &format!("{guided}\n{stage}"),
            &["names {inputs.implement.notes}"],
        );
        // An unknown operator is its own error, not a panic here.
        refused(
            &joined("Test it.").replace("implementer", "nobody"),
            &["unknown operator \"nobody\""],
        );
    }

    #[test]
    fn a_pr_gate_reads_the_rebaser_s_and_fixer_s_guidance() {
        let guided = "[operators.mender]\nkind = \"claude\"\nguidance = \"Start from {inputs.implement.notes}.\"\n";
        let pr = |check: &str| {
            format!(
                "{guided}\n{}gate = {{ kind = \"external\", check = \"{check}\" }}\n",
                joined("Open a PR.")
            )
        };
        let policy = |role: &str, check: &str| {
            format!("{SWITCHBOARD}\n{}", pr(check))
                .replace("[policy]\n", &format!("[policy]\n{role} = \"mender\"\n"))
        };
        for (role, check) in [
            ("rebaser", "pr-checks"),
            ("rebaser", "pr-merged"),
            ("fixer", "pr-checks"),
        ] {
            let err = Pipeline::parse(&policy(role, check))
                .unwrap_err()
                .to_string();
            assert!(
                err.contains("names {inputs.implement.notes}"),
                "{role} {check}: {err}"
            );
        }
        // A `pr-merged` gate never fixes checks.
        Pipeline::parse(&policy("fixer", "pr-merged")).unwrap();
    }

    #[test]
    fn a_joined_stage_naming_a_per_lane_commit_is_refused() {
        let deploy = "[[stages]]\nname = \"deploy\"\ncontext = \"each\"\ngate = { kind = \"command\", argv = [\"make\", \"deploy\"], in = \"lane\" }\n";
        let reader = joined("Check {inputs.deploy.commit}.");
        refused(
            &format!("{deploy}\n{reader}"),
            &[
                "names {inputs.deploy.commit}",
                "deploy records a commit once per lane",
            ],
        );
        let one = deploy
            .replace("\"each\"", "\"lane:repo\"")
            .replace("in = \"lane\"", "in = \"lane:repo\"");
        with(&format!("{one}\n{reader}")).unwrap();
    }

    #[test]
    fn a_stage_qualified_variable_may_not_land_on_another() {
        // Stage `implement` has a command gate, so
        // `{inputs.implement.checks}` is DISPATCH_INPUT_IMPLEMENT_CHECKS.
        let writer = "[[stages]]\nname = \"after\"\noperator = \"planner\"\ncontext = \"root\"\nwrites = [\"implement-checks\"]\nprompt = \"Write {implement-checks}.\"\n";
        refused(
            writer,
            &[
                "DISPATCH_INPUT_IMPLEMENT_CHECKS",
                "the same variable name as implement's \"checks\"",
            ],
        );
        refused(
            &writer.replace("implement-checks", "plan-plan"),
            &["DISPATCH_INPUT_PLAN_PLAN", "the same variable name"],
        );
    }

    #[test]
    fn a_gate_only_stage_writes_in_one_context() {
        let deploy = "[[stages]]\nname = \"deploy\"\ncontext = \"each\"\nwrites = [\"url\"]\ngate = { kind = \"command\", argv = [\"make\", \"deploy\"], in = \"lane\" }\n";
        refused(deploy, &["a gate-only stage writes only in one context"]);
        let one = deploy
            .replace("\"each\"", "\"lane:repo\"")
            .replace("in = \"lane\"", "in = \"lane:repo\"");
        with(&one).unwrap();
    }
}
