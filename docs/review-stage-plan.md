# Dispatch: the code review stage (plan skeleton)

Status: built. The design as built, with every `PLANNER:` decision
taken, is the "The code review stage" section of `docs/dispatch.md`;
this file is the plan it was built from and is kept for the record.
The decisions, in short: command reviewers are `[operators.<name>]`
with `kind = "command"`; the dial is `review-code` (`fix`, `accept`,
`park`) and the cap question `review-cap` (`accept`, `more`, `park`);
one attempt per context with `rounds` on it; a command reviewer's
launch intent is on its record before it runs and a lost one is
failed, never relaunched; Codex reviewers complete on a settled file
and fail when their session is gone without one; a check lost to a
restart is the one child started again. Not built from the acceptance
table: the partial fan-out restart cases beyond a lost command (agent
reviewers go through the ledger like any session), a reused pid (a
lost child is failed by key, never adopted), parking mid-round as its
own test (the park sequence is the existing one over the process
list), and the slot released while every lane waits.

## What it is

A stage that reviews the branch a lane's `implement` produced, with
several reviewers at once, and has the implementer address what they
say, in rounds, until no reviewer objects or a cap is reached. It sits
after `implement` and before `inspect` (the human gate) and the stage
that pushes and opens the PR. The pipeline defines the reviewers;
Dispatch runs them.

The plan review already built (`review = "reviewer"`, `subject =
"plan"`) reviews one file with one reviewer through Switchboard's
planner-and-reviewer workflow. This stage is a different mechanism:
the subject is a branch, there are many reviewers, and the implementer
is a fresh agent per round. It must not be forced into the existing
workflow stage. `Stage::kind()` recognises agent, workflow and
gate-only stages by which fields are present; this stage has neither
`operator` nor `review`, so it needs a discriminator of its own
(`reviewers` present) and a fourth `StageKind`.

## Pipeline shape (proposal, to be confirmed)

```toml
[operators.style]
kind = "claude"
guidance = "Hold the diff to the conventions in CLAUDE.md: ..."

[operators.comments]
kind = "codex"
guidance = "Comments say why, not what, and never narrate history ..."

[reviewers.lint]                      # a command reviewer: local tooling, no agent
argv = ["sh", "-c", "uv run --frozen ruff check --output-format concise ."]
in = "lane"

[[stages]]
name = "review-code"
context = "each"
subject = "branch"                    # base..head of the lane's branch
reviewers = ["style", "comments", "lint"]
implementer = "implementer"           # who addresses the feedback
cap = 3
gate = { kind = "command", in = "lane", argv = [...] }   # the stage's checks: required at every accepted head
```

- PLANNER: settle the table names. Is a command reviewer an
  `[operators.<name>]` with `kind = "command"`, or its own
  `[reviewers.<name>]` table? One answer, applied everywhere.
- `subject = "branch"` is the only subject in the first cut. The base
  is the commit the lane was cut from: `<remote>/<base>` is resolved
  to a SHA at the cut and persisted on the lane record (an additive
  `LaneRecord.base_sha`; a lane cut before the field existed gets it
  from `git merge-base` on the stage's first pass, persisted then).
  It is never resolved again: the shared clone's remote refs move
  whenever another ticket fetches, and a review diff against a moved
  base would show unrelated upstream changes as apparent reversions.
  The head is the lane's head when the round starts. Both are
  persisted on the round before any reviewer is launched, and the
  diff a reviewer is pointed at is `base..head` in the lane's
  worktree, never "whatever the branch is now". Rebasing the branch
  is not part of this stage; a head whose history no longer contains
  the recorded base is an unexpected movement (below).
- PLANNER: decide how a stage-level `cap` and its end-of-cap decision
  relate to the plan review's `cap` and `finalize`, and name the
  decision (`review-code`? `finalize-code`?). The dial for it lives in
  `[policy] decisions`.
- The stage has its own `gate`, required. The plan may allow it to
  name `implement`'s gate by reference, but a review stage with no
  checks at all is refused at parse time.
- PLANNER: validation: unknown or duplicate reviewer names, an empty
  `reviewers` list, a missing or unknown `implementer`, an
  implementer whose kind is not Claude Code (the Stop hook is the one
  completion signal), a `cap` of zero, `subject` other than
  `branch`, a context with no branch (a `root` project), and stage
  fields that make no sense together (`operator`, `review`, `writes`
  alongside `reviewers`). Each refused with the stage named, as
  `validate_stage` does today.

## One round

1. The round is opened with its base and head SHAs recorded, the tree
   confirmed clean at that head. Every reviewer then runs at once
   against the lane's worktree. Agent reviewers get a prompt naming
   the base, the head, the worktree, the plan and the issue, and a
   feedback file under the attempt directory to write to. Command
   reviewers run as a child of the runner, with their stdout as their
   feedback.
2. When every reviewer has finished (rule below), the tree must still
   be clean at the recorded head; otherwise the round's evidence is
   discarded and the round fails into a decision (something wrote to
   the tree during review). Dispatch then aggregates the feedback
   files into one numbered `feedback.md` with the reviewer's name on
   each point and a stable point id (`<reviewer>-<n>`).
3. If nothing was contributed, the round has converged: the stage's
   checks run at that head, and the stage completes bound to it only
   on a zero exit with the tree still clean; a failing gate is the
   ordinary checks decision (`rerun`, `check`, `park`). Checks
   recorded earlier are reused only when they are the same gate: the
   same resolved argv, cwd and environment, run at this same head on a
   clean tree, which in practice means the stage names `implement`'s
   gate by reference and nothing has moved. A different gate at the
   same head runs, however strong the earlier one looked. This holds
   on every completion path, the cap acceptance included.
   Otherwise a fresh implementer session gets the plan, the branch and
   `feedback.md`, addresses each point on the branch, and writes
   `response.md` answering each point by id (fixed, or disputed with
   why). It must leave the tree clean and committed; a dirty tree or a
   missing `response.md` fails the round into a decision. Then the
   stage's checks run at the new head, as after `implement`; a
   failure is the same checks decision, and nothing paid starts on
   its own after any failure.
4. The next review pass starts at the new head, with the previous
   round's `feedback.md` and `response.md` handed to the fresh
   reviewers. A point the implementer disputed without a code change
   stays open: the next reviewer must either withdraw it or carry it
   forward as unresolved, and the pass converges only with no open
   point. Point ids are scoped by round (`r2/<reviewer>-<n>`), and a
   carried point keeps its original id, so a fresh pass's first point
   is never confused with an earlier round's. `cap` counts review
   passes. The last pass either converges (accepts its head, checks
   as above) or ends the stage in a decision: accept the last
   reviewed head as is with findings left, one more round, or park.
   "One more round" authorises exactly one fix pass by a fresh
   implementer against the last pass's findings, then the checks,
   then one review pass, after which the cap decision is asked again
   if findings remain; it stands in for the per-pass `ask` decision
   of that round. A head that no reviewer has seen is never offered
   for acceptance: fixes after the final pass are not applied, since
   the final pass is a review pass, not a fix pass.

- PLANNER: specify the prompt templates and their variables
  (`{base}`, `{head}`, `{worktree}`, `{feedback}`, `{response}`,
  `{previous_response}`, `{no_feedback}`, `{plan}`, `{issue.*}`),
  with defaults so a pipeline can give only guidance.
- PLANNER: specify the aggregated file's format so the implementer
  can answer point by point and the page can show it.
- Reviewer completion. A Claude reviewer is complete on its Stop with
  its feedback file present and settled (`SETTLE_POLLS`), the same
  rule as an ordinary agent stage; Stop means a finished turn, not an
  exited session, so the session is killed and read back as gone
  (the existing process-list sequence) before anything else touches
  the tree. PLANNER: choose the Codex rule; the existing workflow
  stage settles Codex output by file mtime with no Stop, and this
  stage may do the same, but say so and state the stall rule (an
  unchanged missing file after N polls is a failed reviewer). A
  command reviewer is complete on exit. A missing or empty agent
  feedback file after completion is a failed reviewer, never an
  approval. The no-feedback sentinel matches only when the file,
  trimmed of surrounding whitespace, is exactly the sentinel; a
  sentinel followed by anything else is findings.
- Command reviewer protocol. Exit 0 with output: the output is
  diagnostic, kept on the record, and contributes no findings. Exit
  1: findings, the stdout is the feedback. Any other exit, or exit 1
  with empty stdout: an execution error, the reviewer failed. stderr
  is kept beside stdout in the reviewer's directory, never merged
  into feedback. A tool with other conventions is wrapped by a
  pipeline-owned script; Dispatch does not normalise per tool.
  PLANNER: confirm the codes or make them configurable per reviewer.
- A failed reviewer (its session died, an execution error, a stall)
  fails the round into the ordinary rerun decision; its siblings are
  retired first by the process-list sequence. Nothing is retried on
  its own.
- The implementer of a round is always fresh: agents are never
  resumed automatically, and a fresh agent reads the branch as it is.
- Per-round evidence: the base and head every reviewer read, each
  reviewer's completion (stop or exit, the file's settle), the head
  after the implementer, and the checks' exit at that head. A round's
  results bind to the head they were made at, like every other result
  in Dispatch.

## Heads and the tree during review

- Two kinds of head movement are distinguished by the record. The
  implementer's commits in step 3 are an authorised transition: the
  round records the head before and after, and the review evidence
  from before stays as history of that round, not as evidence for the
  new head. Any other movement (the head differs from the recorded one
  when reviewers finish, when a decision is answered, or when a
  round would start) is unexpected: the design's moved-head rule
  applies, the round's evidence is void, and the ticket parks with
  the two heads named.
- The stage's own accepted head is what downstream gates (`inspect`,
  `pr`, `ready`) bind to; `implement`'s checks at an earlier head are
  history once this stage accepts a later one, and the stage's own
  checks at the accepted head replace them.
- Another lane's commits never touch this lane's result: per-lane
  results bind to their own lane's head (the existing rule).
- A decision is bound to the lane, attempt, round and head it was
  made for; an answer whose head no longer matches launches nothing
  and is reported as stale.
- Reviewers are read-only by detection, not prevention: Dispatch
  cannot keep an agent from writing to a tree it can see, so the
  round records the head and cleanliness at start and end, and any
  change fails the round. Claude reviewers run in the tree with the
  attempt directory as their one allowed write target
  (`OperatorKind::write_flags`), Codex reviewers in the attempt
  directory (`OperatorKind::reviews_in_tree`), the same as the plan
  review today. PLANNER: say how a command reviewer that writes a
  cache into the tree is handled (an ignored path is clean; anything
  else fails the round), and make the dirty-tree test cover an
  uncommitted change.

## Runner children and recovery

- A command reviewer is not a command gate. `poll_gate` starts a lost
  check again on its own, which is right for a check (a check is worth
  nothing until its result is bound, and it is the one child of its
  attempt) and wrong here: a review round has several children and a
  no-second-launch rule. What the plan must provide is a launch and
  result protocol with these properties: the intent to launch is
  durable before anything executes; the running process can be
  identified as the one launched, not by pid alone (pids are reused,
  and a pid is not known until after the spawn, which leaves a
  window); an exit result can be obtained after a runner restart,
  which the `std::process::Child` handle `poll_check` relies on
  cannot give; and an ambiguous state (intent recorded, no
  acknowledgement) resolves to a failed reviewer, never to a
  duplicate launch or a kill of an unrelated process. A small
  supervisor or wrapper that writes a durable acknowledgement with
  the process's identity at start and its exit record at the end is
  one shape. PLANNER: choose the mechanism, say how the child's
  descendants are found and killed (a process group per reviewer is
  the likely answer), and add them to what parking and rerun retire.
  Tests cover the window between spawn and the acknowledgement, and a
  reused pid, not only a restart with a fully recorded live child.
- The ticket's process list covers command reviewers and checks as
  well as agent sessions, so parking and rerun retire them by the one
  sequence.
- PLANNER: walk every crash boundary: before any launch, after some
  reviewers launched (partial fan-out), after all finished but before
  aggregation, after the implementer launched, after the checks
  started. State what a restart finds and does at each, with the
  ledger for Switchboard operations and the launch records for
  children. No boundary may launch a second copy of a reviewer or an
  implementer. The one exception is the stage's checks, which keep
  the command gate's rule of starting again on the same clean head
  after a restart, and only once the previous check and its
  descendants are confirmed gone.

## Records and views

- PLANNER: attempt per round, or one attempt for the stage with rounds
  inside it? Choose and say why. Either way `Attempt` holds one
  session and one gate today, and a round has several reviewers and
  an implementer; the plan names the additive fields (a `rounds`
  list on the attempt, or a round record with its own reviewer
  records) in `ticket.rs`, their `#[serde(default)]`s, and the
  matching `AttemptView` additions in `dispatch-control`. The page
  must show each round's feedback and response, like the plan
  review's rounds today.
- Artifacts, per attempt, lane and round: each reviewer's raw output
  (`<round>/<reviewer>/feedback.md`, plus `stdout`/`stderr` for a
  command), the aggregated `feedback.md`, `response.md`, and the
  checks' log.
- PLANNER: list what `switchboard`'s side needs. The expectation is
  nothing in `model.rs`, `AppAction` or `Effect`: reviewers are
  ordinary sessions over the existing port commands and the page reads
  views. If that holds, say so; if not, name the additions and the
  `SCHEMA_VERSION` step.
- Dials: `review-code = "auto"` runs rounds without asking;
  `"ask"` makes a pending decision after each review pass with
  findings, authorising the fix pass (`fix`, `accept as is`, `park`);
  a converged pass never asks. The cap decision has only `ask`.
  PLANNER: confirm the names.

## Concurrency and cost

- One ticket, one slot: a round's reviewers run at once but count as
  the one attempt's slot, since the slot limit is about tickets in
  flight, not sessions. PLANNER: say whether budgets (when built)
  charge each reviewer session separately; the expectation is yes.
- The slot is the ticket's, and its lanes run independently, so it
  is held while any lane of the stage has active work (a reviewer, an
  implementer or a check running) and released only when the whole
  ticket is waiting on a decision with nothing running. An answered
  decision reacquires the slot before it launches anything, under the
  same limit as a new ticket.
- Two lanes of an `each` stage review side by side, as `implement`
  does; attempt numbers come from `next_n`, and each lane's decisions
  and evidence are its own.

## Out of scope for the first cut

Reviewers seeing each other's points, a reviewer that is a Switchboard
workflow, a human as a reviewer entry (the human's place is the dial
above and the `inspect` stage after), review of anything but a branch,
and cross-lane review of a joined result.

## Acceptance (to be completed with a test name per row)

| Case | Expected |
|---|---|
| Two agent reviewers and one command reviewer, no findings | One pass; the stage's checks run at that head; on exit 0 the stage completes bound to it; no implementer ran |
| No findings and the checks fail | The stage does not complete; the checks decision (`rerun`, `check`, `park`); nothing launched on its own |
| One reviewer objects | `feedback.md` carries its point with its name and id; a fresh implementer addresses it and writes `response.md`; the checks run; pass two converges |
| A point is disputed, no code change, and the reviewer withdraws it | The point carries its original id into pass two; withdrawn there; the pass converges |
| A point is disputed, no code change, and the reviewer rejects the response | The point stays open under its original id; the stage is not converged |
| `implement`'s gate is tests and the review gate is lint and tests, same head | The review gate runs; the earlier checks are not reused |
| The review gate names `implement`'s gate by reference, same clean head | The earlier checks are reused; nothing runs twice |
| Another ticket fetches a newer base between passes | Pass two diffs against the recorded base SHA; the diff is unchanged |
| `cap = 1` with findings | No fix pass; the cap decision offers the reviewed head, one more round, or park |
| `cap = 1`, answered "one more round" | Exactly one fix pass, then the checks, then one review pass; a failed check stops the sequence in the checks decision |
| The cap is reached with findings left | The offered head is the last reviewed one; accepting runs the checks at it and completes |
| Two lanes under `slots = 1`, lane A at an `ask` decision while lane B reviews, a second ticket queued | The second ticket does not start until lane B is also waiting; lane A's answer reacquires the slot before launching |
| Launch intent recorded, runner crashes before the acknowledgement | The reviewer is failed on restart; nothing launched twice, nothing unrelated killed |
| The runner restarts and the recorded pid now belongs to another process | Not adopted; the reviewer is failed |
| A reviewer's session dies, or a command exits 2 | The round fails into the rerun decision; siblings are killed and read back as gone first |
| A command reviewer prints findings and exits 1 | Findings, not a failure |
| A command reviewer exits 0 with output | Diagnostic only; contributes nothing |
| An agent reviewer stops with no feedback file | A failed reviewer, never an approval |
| The tree is dirty when reviewers finish | The round's evidence is void; a decision naming the change |
| The head moved between passes, not by the implementer | The ticket parks with both heads named; earlier evidence is history |
| The other lane commits mid-round | This lane's round is untouched |
| The implementer leaves the tree dirty or writes no `response.md` | The round fails into a decision |
| The runner restarts after some reviewers launched | Each launched one is found by ledger or launch record; the rest launch once; no second copy |
| The runner restarts while a command reviewer runs | Alive: adopted; gone: failed, not restarted |
| Parked mid-round | Every reviewer, the implementer and any command child are killed and read back as gone before the ticket reads as parked |
| A stale decision answer (head changed) | Nothing launches; the answer is reported stale |
| Two lanes | Each converges on its own head and evidence |

- REVIEWER: hold the plan to the design's invariants: every request
  in the ledger before it is sent; results bound to heads; agents
  never resumed automatically; no command from the repository, only
  from the pipeline file; nothing written into the tree by a
  reviewer, enforced by detection; a failure is a decision, never a
  retry on its own.
- REVIEWER: check that the plan adds to `model.rs`, `AppAction`,
  `Effect` and the wire crates additively and names the additions, or
  shows none are needed.
- REVIEWER: check the plan names one test in
  `dispatch/tests/first_slice.rs` per acceptance row, against the fake
  Switchboard and `FakeRepo`, and names the `#[ignore]`d live test in
  `dispatch/tests/live.rs` that verifies the Codex completion rule and
  the reviewers' write permissions against real agents, since the fake
  cannot.
