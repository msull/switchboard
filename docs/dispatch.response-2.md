# Response to review 2 of the Dispatch plan

Each code claim was checked before answering. `WorkflowDefinition::render`
substitutes a `{no_feedback}` placeholder and appends nothing
(`src/core/model.rs`); `round_paths` derives the feedback and response
files from the plan path and round number (`src/core/workflow.rs`); a
quiet pane reads idle after `QUIET_AFTER`, twenty seconds
(`src/core/reconcile.rs`); `start_workflow` ends with
`show(View::Workflow(id))`; the stall notice is kept per workflow run;
`PauseWorkflow` and `ContinueWorkflow` exist as actions but were not on
the port. All correct. Every item is accepted; two are accepted with a
narrower reading than asked, marked below.

## 1. Templates and frozen definitions — accepted

All six review templates now end with "write exactly this line alone:
`{no_feedback}`", and every `review_round` names `{plan}` and asks for
a re-read of the updated subject. The pipeline file is copied whole
into the ticket's directory when the ticket is taken, and the ticket
runs from the copy. Reviewer definitions are installed as
`Dispatch: <operator>@<content hash>`, so an edit is a new name and a
run in flight keeps the definition it started with.

## 2. Recovery tags and reply semantics — accepted

Notes are no longer the correlation field. Every port command carries
an operation id that Switchboard stores on every record the command
makes, including the review run and the reviewer and planner sessions
it creates, and answers with `find {op}`. The ledger on the ticket
record distinguishes the attempt from its operations. Persistence
before launch is stated as a requirement on the port with tests, which
is what makes "not found means nothing ran" true. A found record is
read for its own state (pane liveness, last exit, run state) rather
than labelled launched, and a run found in `Starting` with no planner
clone after either restart is a failed attempt. Replies are terminal:
`persisted` or `launched` or `failed`, one per request, plus an
`op.status` query. Switchboard restarting mid-creation is covered by
the same reads, since every record the port made is an ordinary record
to its reconcile.

## 3. Stage outputs, review mutation, stages without agents — accepted

Attempts now have three kinds with their own completion rules: agent
(session plus named artifacts), workflow (finalized run), gate-only
(the gate's own result). Artifacts are named by a `writes` list per
stage, expand to `<stage>/<attempt>/<context>/<name>.md`, and
`{inputs.<name>}` is the most recent completed stage's artifact of that
name, so PTA's `render` reads the `plan` that `draft` wrote and
Switchboard's `implement` reads the one the review finalized. A
workflow stage copies its `subject` into its own attempt directory and
runs on the copy, which keeps the original auditable and gives each
attempt its own round files. A skipped stage's fields render as
`unknown (deploy skipped)` and the Orchard `try` prompt says what that
means.

## 4. Idle is not completion — accepted

Completion evidence is the session's Stop event or a zero exit,
together with a settled artifact, both recorded on the attempt when
seen; idle is named as not a stop. Recovery applies the same rule: a
missing session counts as finished only if both facts were recorded
before the crash, so a partial file after a kill fails the attempt.
Ordinary stage operators are constrained to Claude Code because its
Stop hook is the only signal that means finished; Codex stays as a
workflow reviewer only, where the workflow reads its file. Generic
stall detection is named as new port work: the `session` query reports
how long the pane has been quiet, and Dispatch applies its own
threshold.

## 5. Clean, unchanged inputs — accepted

Command gates require a clean tree, and check head and cleanliness
before and after the run; a dirty tree either side is a decision. Every
code-dependent result records the set of lane heads it was made
against, and a moved head, whether by an agent or a push from your
machine, voids every result made against the old set and returns the
ticket to the earliest voided stage. For Orchard that is `implement`'s
checks, then `deploy`, `try` and `tried` again before `ready`, so there
is no path to `merge` on old test evidence.

## 6. Quiesce before release — accepted

Cancellation is written as a sequence: persist the intent, pause any
review run (`workflow.pause` and `workflow.continue` are now on the
port), kill every session and command the attempt owns including a
tester's frontend server, read each back until reported gone, confirm
an in-place tree clean, and only then release holds. If a writer cannot
be confirmed gone or the tree cannot be cleaned, the holds stay and a
decision says why. `retake` closes the old ticket by the same sequence.

## 7. PTA's handoff — accepted

A `release` field names the stage where a lane's hold ends, so PTA
releases after `render`, not after `ready`. Before releasing, Dispatch
copies every file in the writer's manifest into the attempt directory
as the `deliverable` artifact and checks out the lane's base, so the
next ticket starts on `main` and the PDF awaiting posting survives the
next checkout. The render gate now verifies each manifest path exists
in addition to the clean tree and the commit. A Drafts item cannot be
verified from here; the manifest records its subject line and the
`published` decision shows it. That last part is accepted narrowly:
verification stops at what the filesystem can prove.

## 8. Merge as pending human work — accepted

`merge` is both an external fact and a pending confirmation decision:
reaching it creates the decision, marks the current session waiting,
and counts against `waiting_on_me`. `pr-merged` resolves it without
your answer, and a hand answer is refused until the provider agrees.
Rebundling and mirror backup are stated as outside Dispatch's
definition of done, named in the decision text as a reminder and not
verified. That is accepted narrowly too: a second confirmation for
those acts would be a box you tick with nothing behind it.

## 9. First-slice counts and background behaviour — accepted

The happy path now expects four sessions (investigator, planner, the
run's reviewer and its planner clone), each carrying its operation id.
Every port command is quiet: the select-the-new-thing step is skipped,
including the review view that `start_workflow` opens, and the
open-terminal-on-launch setting is not honoured for port launches. The
acceptance table gained rows for a crash after `workflow.start`, for
Switchboard dying between persisting and launching the reviewer, for
a partial notes file, and the "another workspace active" case now spans
the whole path including the review start.
