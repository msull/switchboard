//! The pipeline file: one TOML document per project, the whole schema of
//! `docs/dispatch.md` parsed and checked here, so a later slice adds an
//! executor rather than a field. A ticket runs from the copy taken when
//! it was made; the project's file only shapes tickets taken afterwards.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

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
}

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
    /// Run once in a new worktree.
    #[serde(default)]
    pub setup: Vec<String>,
    #[serde(default)]
    pub serve: Option<Serve>,
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
            Self::Claude => true,
            Self::Codex => false,
        }
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
            Self::Codex => Vec::new(),
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
        #[serde(rename = "in")]
        run_in: String,
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
    /// Artifact names; each expands as `{name}` in the prompt.
    #[serde(default)]
    pub writes: Vec<String>,
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
}

/// What a stage is, from which fields it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageKind {
    Agent,
    Workflow,
    GateOnly,
}

impl Stage {
    #[must_use]
    pub fn kind(&self) -> StageKind {
        if self.operator.is_some() {
            StageKind::Agent
        } else if self.review.is_some() {
            StageKind::Workflow
        } else {
            StageKind::GateOnly
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
    /// The operator that rebases a branch whose PR conflicts with its
    /// base, cloned from the lane's implementer; absent, a conflict is
    /// a question.
    #[serde(default)]
    pub rebaser: Option<String>,
    /// How many rebases one PR may get before the conflict is a
    /// question instead.
    #[serde(default = "default_max_rebases")]
    pub max_rebases: u32,
}

fn default_max_reruns() -> u32 {
    3
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
            rebaser: None,
            max_rebases: default_max_rebases(),
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

    /// Whether tickets work in worktrees of Dispatch's clone (`repo`),
    /// as opposed to in place at `root`.
    #[must_use]
    pub fn cuts_worktrees(&self) -> bool {
        self.project.repo.is_some()
    }

    /// Parse and validate one file's text.
    pub fn parse(text: &str) -> Result<Self> {
        let mut p = Self::parse_raw(text)?;
        if let Some(dir) = &p.project.worktrees {
            p.project.worktrees = Some(crate::store::expand_home(dir));
        }
        Ok(p)
    }

    fn parse_raw(text: &str) -> Result<Self> {
        let pipeline: Self = toml::from_str(text).context("parse the pipeline file")?;
        pipeline.validate()?;
        Ok(pipeline)
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

    /// One stage's references, given what earlier stages wrote.
    fn validate_stage(&self, stage: &Stage, written: &[&str]) -> Result<()> {
        if stage.operator.is_some() && stage.review.is_some() {
            bail!("stage {:?} names both an operator and a review", stage.name);
        }
        Self::validate_gate(stage)?;
        if let Some(op) = &stage.operator
            && !self.operators.contains_key(op)
        {
            bail!("stage {:?} names an unknown operator {op:?}", stage.name);
        }
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
        let mut seen = std::collections::BTreeSet::new();
        for name in &stage.writes {
            if !seen.insert(name) {
                bail!("stage {:?} writes {name:?} twice", stage.name);
            }
        }
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
            ..
        }) = &stage.gate
        {
            if argv.is_none() && per_lane.is_none() {
                bail!(
                    "stage {:?}: a command gate needs argv or per_lane",
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
        Ok(())
    }

    /// Every reference resolves and every stage is one of the three kinds.
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
        if let Some(rebaser) = &self.policy.rebaser
            && !self.operators.contains_key(rebaser)
        {
            bail!("[policy] rebaser names an unknown operator {rebaser:?}");
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
        }
        let mut stage_names = std::collections::BTreeSet::new();
        let mut written: Vec<&str> = Vec::new();
        for stage in &self.stages {
            if !stage_names.insert(&stage.name) {
                bail!("stage {:?} is listed twice", stage.name);
            }
            self.validate_stage(stage, &written)?;
            written.extend(stage.writes.iter().map(String::as_str));
            if stage.review.is_some()
                && let Some(subject) = &stage.subject
            {
                // A review's finalized copy is the subject under its
                // own name, for the stages after it.
                written.push(subject.as_str());
            }
        }
        for (name, dial) in &self.policy.decisions {
            if !matches!(dial.as_str(), "ask" | "recommend" | "auto") {
                bail!("decision {name:?}: the dial is ask, recommend or auto, not {dial:?}");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const SWITCHBOARD: &str = r#"
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
}
