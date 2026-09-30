# Dispatch

A system that sits on top of Switchboard the way Switchboard sits on top
of tmux and Prompt Box. Nothing here is built; this is the design to
argue with, with three real pipelines written by hand to test the
vocabulary before any code. Decisions made without you are collected
at the end.

The idea in one line, moved up a level: **tickets are data, sessions are
a cache.** Switchboard keeps sessions and files durable on top of tmux
and shows which agents are waiting. Dispatch keeps tickets, attempts,
decisions and resource holds durable on top of Switchboard, and shows
which tickets are waiting on you.

## The boundary

Switchboard stays the console and the process host. Dispatch is the
scheduler and the policy. Dispatch knows nothing about tmux,
transcripts, hooks, or panes; it only ever says "make a project at this
root in this space", "start a plan review with this definition", "send
this prompt", and asks "which sessions are waiting, what state is this
run in, what did this command exit with". Switchboard knows nothing
about tickets, GitHub, worktrees, or what "ready" means.

That mirrors Switchboard and Prompt Box: Prompt Box knows nothing about
sessions, Switchboard nothing about the voice runtime, and the join is
one small port (`PromptSink`). The join here is a **control port**: a
Unix socket beside the wake socket with a narrow wire contract (see
"Control port"). Dispatch is a separate process talking over it. One
process owns each data directory, and you can always drop into the real
window and see every card, because Dispatch draws nothing of its own.

Dispatch's own state lives in its own data directory
(`~/Library/Application Support/Dispatch`), one record per ticket,
written the way Switchboard writes records (temp file, fsync, `.bak`,
rename) under a single-writer lock file. It never writes into a
repository's own dot-directories; only the agents commit code. Same
hard rule as Switchboard.

Workspaces in Switchboard are a presentation boundary, not a filesystem
boundary: two tickets' agents can read each other's worktrees. Dispatch
does not pretend otherwise. Isolation between tickets is that each has
its own worktree and branch, and that an operator's guidance names the
worktree it may touch.

Invariant: Dispatch runs only commands written in a pipeline file in
its own data directory, never a command a repository declares, the way
Switchboard never runs a project's `project.json` entries without the
user's approval of each. What a pipeline's command then calls inside
the repository is the repository's code, running with the user's
authority; Dispatch reduces that exposure rather than removing it. The
trees it hands that code live under `~/.dispatch/worktrees` (the
`dispatch worktrees` setting; a pipeline may name its own), never under
the data directory, whose path on macOS holds a space that a
repository's tooling may not survive, and a root holding whitespace or
a shell-special character is refused at `take`. Confining those
commands to the tree (a sandbox) is filed as an issue and not built.

## Objects

- **Ticket.** The unit of work. Its identity is its source identity
  (below) plus a Dispatch id. The record holds the source snapshot, the
  pipeline name and version it was taken under, the lanes, the current
  stage, every attempt, every decision, the resource holds, the queue
  rank, and the usage counted so far.
- **Source identity.** For a GitHub source, the repository and issue
  number. For a task file, a hash of the task line's text with the
  marker removed and whitespace normalised; moving the line does not
  make a new ticket, editing its text does. A ticket is taken once per
  identity. `dispatch retake` makes a new ticket from the same identity
  on purpose, and closes the old one. The text as it was when taken is
  kept on the record, so what an operator was told is always known.
- **Lane.** One repository the ticket touches: a worktree, a branch,
  and the Switchboard project made for it. A lane with no worktree
  directory works in place on a branch and holds a repository lock
  (below) for as long as the ticket owns files there.
- **Root context.** Every ticket also has a Switchboard project at the
  pipeline's root, used before lanes exist and for joined stages. For
  a single-repository pipeline it is the repository; for Delta it is
  the workspace directory. Agents run there before any branch is cut
  and are told to write nothing but their notes.
- **Pipeline.** Per project, a list of stages. A stage names an
  operator, a prompt, which context it runs in (the root, each lane,
  or all lanes joined), and a gate that says when it is done. When a
  ticket is taken, the pipeline file is copied whole into the ticket's
  directory, and the ticket runs from that copy until it finishes;
  editing the project's file changes only tickets taken afterwards.
  Reviewer definitions are installed into Switchboard under
  `Dispatch: <operator>@<content hash>`, so an edited definition is a
  new name and a run in flight keeps looking up the one it started
  with.
- **Attempt.** One run of one stage in one context. Every launch,
  gate command, file, and result belongs to an attempt, which is why
  a restart can tell what was already done. A failed or abandoned
  attempt stays on the record; a new one is only made by a decision.
  Attempts come in three kinds, with different completion rules:
  *agent* (one fresh session and a named output), *workflow* (a
  Switchboard review run, complete when finalized), and *gate-only*
  (no session: a command, an external fact, or a decision).
- **Operation.** One request to Switchboard, with its own id, kept in
  a ledger on the ticket record: what was asked, when, and the reply.
  An attempt owns several operations (a project, a session, a
  command). The operation id is the correlation field: Switchboard
  stores it on whatever record the request made, and answers
  `find {op}` with it, so recovery never depends on notes text.
- **Gate.** How a stage is known to be done. Three kinds:
  - *command*: a command run once for the attempt, in a named
    context, exiting zero; the result is bound to the commit it ran
    against;
  - *external*: a fact read back from Switchboard or a provider (a
    review run's state, a PR's checks at a head revision);
  - *human*: a decision with an answer.
- **Operator.** A named role: an agent kind and model class, guidance,
  a budget, and a definition of done. A reviewer operator is a
  complete Switchboard `WorkflowDefinition` (the prompt templates and
  the no-feedback sentinel), installed into Switchboard under a
  prefixed name when the pipeline is loaded.
- **Decision.** A point where a human would normally answer. Each has
  an id, the ticket and stage, the options, an operator's
  recommendation when there is one, and a state: pending, answered
  (with the answer, who gave it, and when), or cancelled. Answers
  arrive through `dispatch decide`. Every decision is logged whether
  you took it or the policy did.
- **Resource.** Something only one ticket can hold at a time: a shared
  test stack, an in-place repository. Holds are on the ticket record,
  taken under the writer lock, and survive both processes restarting.
- **Queue.** An ordered list of tickets per project, on a Dispatch
  record, reordered by `dispatch queue`. Dispatch takes from the top
  whenever a slot is free.

## Records and recovery

Dispatch never asks Switchboard to do anything it has not first written
down. The sequence for every operation is: append it to the ledger with
a fresh operation id and state `sent`; send the request with that id;
append the reply. Every port command that creates something takes the
operation id and stores it on the record it makes (a field on the
session record, the project, the space, the working set, the workflow
run, and on the run's reviewer and planner sessions), and Switchboard
persists that record before it runs the effect that launches anything.
That ordering is a requirement on the port, not a hope: the port's
tests assert it.

On start, Dispatch reads every ticket, then reconciles each operation
in the ledger with no reply against Switchboard's `find {op}`:

- Not in the operations log, for a creation: the log entry is
  written before any effect, so an absent entry means nothing ran.
  The operation is `lost`, the attempt fails, and the stage is retried
  only through a decision. For the other command classes, see the
  port: idempotent ones are sent again, non-replayable ones become a
  decision.
- In the log but the record removed: someone removed it in the
  window, which is allowed; Dispatch-owned records carry a visible
  "Dispatch" mark and their notes say which ticket they serve, but
  nothing stops a removal. The attempt is failed as `removed by hand`
  and a decision shows it. Any pane the removal killed is treated as
  stopped without completion evidence.
- Found: the record exists, which proves persistence, not launch.
  Dispatch reads the record's own state (host liveness, last exit,
  the run's state) and records the operation as whatever that says:
  `launched` if a pane exists or existed, `failed` if the record is
  there but the launch never happened (Switchboard's own reconcile
  marks a record whose pane never appeared). A review run found in
  `Starting` with no planner clone is waited on while `op.status`
  says `in progress`, and failed once it says `interrupted`.

A live session is observed, never resumed, never sent to. A session
that is gone is judged by the attempt's completion evidence (below),
never by the mere presence of a file. A command gate that was `sent`
with no reply is not run again; it is a decision, because it may have
run. Resource holds are read back from the records; nothing is
released on restart. Switchboard's own restart is covered by the same
reads: every record the port made is an ordinary Switchboard record
that its reconcile treats like any other, and launches nothing.

The "never resume automatically" rule holds on both sides. New
launches happen because the scheduler finds a runnable ticket at the
top of a queue with a free slot, which is the same thing it would have
done had it not restarted, and never because it wants to finish an
interrupted attempt.

## Stage semantics

- A stage runs in one of three contexts. `root` is the root context.
  `each` runs once per lane, in parallel, and the ticket advances when
  every lane's gate passes. `joined` runs once in the root context
  with every lane's worktree path and head commit named in the prompt,
  after every lane's agent has stopped and every worktree is clean; a
  dirty worktree is a decision.
- An *agent* stage names an operator and the artifacts it `writes`.
  Every attempt is a fresh Switchboard session in the stage's
  context, with the ticket's earlier artifacts named in the prompt.
  Fresh per attempt costs more and keeps each stage honest about what
  is on disk. Ordinary stage operators are Claude Code: its Stop hook
  is the only completion signal Switchboard has that means "the agent
  finished its turn". Codex takes part only as a workflow reviewer,
  where the workflow reads the file it writes.
- **Artifacts are named and per attempt.** A stage's `writes` lists
  artifact names; each expands in the prompt to a path
  `<ticket>/<stage>/<attempt>/<context>/<name>.md` under Dispatch's
  data directory. `{inputs.<name>}` is the artifact of that name from
  the most recent completed stage that wrote it, so PTA's `render`
  reads `{inputs.plan}` written by `draft`, and Switchboard's
  `implement` reads the `plan` that the review finalized. A new
  attempt cannot find a stale file already settled, and parallel
  `each` attempts never share a file. Earlier artifacts are read-only
  inputs; a stage that changes one writes its own copy.
- **A review is a copy.** A *workflow* stage copies the reviewed
  artifact into its own attempt directory and starts the run on the
  copy, so the workflow's round files (derived from the plan path) are
  distinct per attempt and the original attempt's file is the
  auditable snapshot. The planner clone's history names the original
  file, so every prompt sent to it names the copy and says to edit
  only that. The copy after finalization is the review stage's `plan`
  artifact.
- **Completion is evidence, recorded.** An agent attempt is complete
  when Switchboard has reported the session's Stop event (or a zero
  exit) *and* the artifact exists and has settled; both facts are
  written on the attempt when seen. A card that merely reads idle is
  not a stop: Switchboard shows a quiet pane as idle after twenty
  seconds, which can be an agent thinking. A settled file with a
  running session waits; a stop with no file, a nonzero exit, or a
  session gone with no recorded stop is a failed attempt. Recovery
  applies the same rule: a missing session is finished only if the
  stop and the settle were recorded before the crash.
- **Stalls.** Switchboard's stall notice is for workflow runs only.
  For agent attempts the port's `session` query reports how long the
  pane has been quiet, and Dispatch raises a decision at its own
  stall threshold. That read model is new work on the port.
- A *gate-only* stage (`lanes`, `deploy`, `ready`, `merge`) has no
  session; its attempt is the gate's own result, and its prompt
  fields (`{inputs.deploy.commit}`) are what the gate recorded. A
  skipped stage records nothing, and a template naming its field
  renders as `unknown (deploy skipped)`; the Delta `try` prompt says
  so in words.
- **Command gates run once, on a clean tree.** After the agent stops,
  Dispatch requires the context's tree clean, records its head commit,
  and asks Switchboard to run the gate as a command record named for
  the attempt. The exit code is read back; then head and cleanliness
  are checked again, and the result is bound to that head only if
  both are unchanged. A dirty tree before or after is a decision, not
  a result. A gate is never run on a polling tick, and never before
  the agent has stopped, so it cannot pass on the untouched base.
- **Evidence carries the heads it depends on.** A per-lane result (a
  lane's checks, its `pr-checks`) records that lane's head only; a
  joined result (a deploy, the tester's report, a `tried` answer)
  records every lane's head. So while `implement` runs, the backend's
  checks finishing before the frontend commits is ordinary progress:
  the frontend's later commit touches no backend result. An `each`
  stage completes when every lane's own result is bound to its own
  final head, and a joined stage starts only at that barrier, with
  every lane's agent stopped.
- **A moved head parks the ticket.** After a result exists, movement
  of a head it depends on, by an agent or by a push from your own
  machine, voids that result and everything after it. The ticket is
  parked with one decision that lists the voided stages and asks to
  rerun from the earliest. The answer authorises those attempts; no
  paid rerun happens without it. Before any rerun starts, the
  cancellation sequence retires the superseded attempts, so an old
  and a replacement attempt never run together. For Delta a push after
  `tried` lists `implement`'s checks, `deploy`, `try` and `tried`;
  there is no path from a moved head to `merge` on old evidence.
- Failure is a decision, never a retry. A gate that fails, an agent
  that stalls (Switchboard already marks it), a review at its cap, a
  branch that no longer merges, a lost launch: each becomes a decision
  with the suggested next step. Nothing loops silently.
- Prompts are text and take template values verbatim. Commands never
  do: a command is a fixed argv from the pipeline file, and template
  values reach it as environment variables (`DISPATCH_TICKET`,
  `DISPATCH_LANE`, `DISPATCH_BRANCH`, `DISPATCH_NOTES`, and so on),
  never spliced into shell source.

## Decisions

A decision is a record, not a message. The ticket's current session
(or, when there is none, a placeholder session Dispatch keeps per
ticket: a shell in the root context whose notes carry the pending
decision) is marked waiting through the control port, so the Dock
count, the rail, and the working set all show it. The text and the
options go into that session's notes with `SetSessionNotes`. The
answer is given with:

```
dispatch decide <ticket> <decision> <answer> [--note ...]
```

and read back by the scheduler. `dispatch decisions` lists what is
pending. A choice card in Switchboard can replace the command later
without changing the record.

Two kinds of human gate are kept apart:

- *permission*: "may Dispatch do this" (cut these lanes, spend past
  budget, run this deploy). The answer authorises the next attempt.
- *confirmation*: "you did this" (merged, published, looked at the
  stack). The gate completes when Dispatch can verify it where it can
  (the PR is merged, read from the provider) and otherwise when you
  say so. An automatic answer to a confirmation gate is never allowed;
  the dial for those has only `ask`.

The dial per decision kind is `ask` or `auto`. `recommend` (surface it
with the operator's answer preselected, taken after a delay) is named
so the file format has room for it, but is not designed here: it needs
a persisted deadline and a restart rule, and nothing in the first
slices wants it. Everything starts at `ask`.

Every session, service and command Dispatch makes for a ticket is
listed on the ticket record from creation until Dispatch removes it,
whichever attempt made it. An agent's pane is killed as soon as its
attempt's completion evidence is recorded, so nothing of a finished
stage keeps running, and the record stays for viewing.

Rejecting a decision cancels the ticket's current attempt, and
cancelling is a sequence, not a flag: write the intent on the record;
pause any review run (`workflow.pause`) so its tick cannot start a
round; kill everything on the ticket's process list that is still
alive, including a service started for an earlier stage; read each
back until Switchboard reports it gone; for an in-place lane, confirm
the tree is clean; and only then release the holds and move the ticket
to `parked`, where you can requeue or close it. Releasing a hold at
the end of a stage runs the same check over the ticket's whole
process list, not the current attempt's. If any writer cannot be confirmed gone or the
tree cannot be made clean, the holds stay and a decision says why.
`dispatch retake` closes the old ticket by the same sequence before
making the new one.

Merge is a confirmation decision as well as an external fact: when a
ticket reaches `merge`, a pending decision is made, the current
session is marked waiting, and it counts against `waiting_on_me`.
`pr-merged` resolves it without your answer when the provider reports
the merge; answering it by hand is refused until the provider agrees
(the decision's only option is `park`).

A human gate on a gate-only stage other than `lanes` (an `inspect`
stage after `implement`, before anything is pushed or a PR opened) is
one permission decision per context: the question names the branch
and its head, what it adds over its base (the commits and the files
changed), the tree to open and the latest notes. `proceed` completes
the attempt bound to the head. `rerun` with a note sends that context
back: the gate's attempt and the nearest earlier agent stage's result
for that context are cancelled, the ticket stands at that stage
again, and the note goes at the end of the next attempt's prompt as
what the user said; other contexts keep their results. A gate with
`confirm = true` is "you did this": its answers are `done` and
`park`, and it is never answered automatically.
What follows a merge on your side (a rebundle, a mirror backup) is
outside Dispatch's definition of done and is named in the decision's
text as a reminder, not verified.

## The pipeline file

One file per project, TOML, in Dispatch's data directory (never in the
repository).

```toml
version = 1

[project]
name = "..."                  # Switchboard project name
repo = "git@..."              # the repository: Dispatch keeps its own clone under its data
                              # directory, fetches it before every cut, and cuts each ticket's
                              # worktree from <remote>/<base>; the user's checkout is never used
root = "/path"                # instead of repo: work in place in this directory, no branch
base = "main"                 # the branch tickets branch from (default main)
remote = "origin"             # (default origin)
worktrees = "/path"           # where this project's trees go (~ expands); omitted: the `dispatch worktrees` setting, default ~/.dispatch/worktrees
space = "..."                 # the Switchboard workspace every ticket's project goes in

[source]
kind = "github" | "task-file" | "manual" | "pull-request"   # pull-request: see "Tickets from pull requests"

[[lanes]]
name = "..."
path = "relative/to/tree"     # "." for a single-repo project
repo = "git@..."              # a repository of its own (a workspace of several): cloned by
                              # Dispatch too, cut as a worktree at path inside the ticket's tree
base = "main"                 # omitted: the project's
setup = ["cmd", "args"]       # run once, before the lane's first agent

[[resources]]
name = "..."
count = 1

[operators.<name>]
kind = "claude" | "codex"
guidance = "..."
budget_usd = 0.0              # per ticket across the operator's attempts; 0 means the project default

[operators.<name>.review]     # present on a reviewer: a complete Switchboard definition;
                              # Dispatch renders {worktree}, {branch}, {project.root} and
                              # {inputs.*} into these templates before installing, since the
                              # reviewer works in the attempt directory, not the repository
reviewer = "codex"            # or "claude": the operator's args go on the reviewer's command
                              # line. A Claude Code reviewer runs in the ticket's tree with an
                              # allow rule for the attempt directory; Codex, which writes only
                              # in its cwd, runs in the attempt directory and is told the tree
review_first = "..."
review_round = "..."
respond = "..."
respond_to_user = "..."
handoff = "..."
no_feedback = "..."
cap = 4

[[stages]]
name = "..."
operator = "..."              # present: an agent stage; absent with a review operator: a workflow stage; absent: gate-only
context = "root" | "each" | "joined" | ["front"]
writes = ["notes"]            # artifact names; each expands as {notes}, {plan}, ...
prompt = "..."                # templates: {issue} {task} {lane} {branch} {inputs.<artifact>} {inputs.<stage>.<field>}
gate = { kind = "command", argv = ["..."], in = "root" | "lane" | "lane:<name>" }
     | { kind = "command", per_lane = { <lane> = ["..."] }, in = "lane" }
     | { kind = "external", check = "review-finalized" | "pr-checks" | "pr-merged" }
     | { kind = "external", check = "pr-checks", checks = "none" }   # a repository with no CI: a PR at the head is enough
     | { kind = "human", decision = "...", confirm = true }
needs = ["resource name"]     # held from the first stage that names it to the last, contiguous

[policy]
slots = 1                     # tickets with a running attempt or a held resource
waiting_on_me = 2             # pending decisions across the project before nothing new starts
rates = { "claude-sonnet-5" = [3.0, 15.0], ... }   # $ per million input, output tokens
decisions = { lanes = "ask", finalize = "ask", merge = "ask", budget = "ask" }
trust_folders = false         # true: Claude Code's folder trust question, which every fresh worktree asks, is answered for the project's agents
max_reruns = 3                # failed attempts a stage may collect in one context before the ticket parks instead of asking again
rebaser = "rebaser"           # the operator that rebases a PR that conflicts with its base, cloned from the lane's implementer; absent, a conflict is a question
max_rebases = 2               # rebases one PR may get before the conflict is a question
fixer = "fixer"               # the operator that fixes a PR whose checks are red at the tree's head, cloned the same way; absent, red checks are a question
max_fixes = 2                 # fixes one PR may get before red checks are a question
```

An agent stage needs no `gate` line: "the agent stopped and every
artifact it writes settled" is the default, described under "Stage
semantics". A workflow stage names the artifact it reviews with
`subject`.

### Pipeline: Switchboard

Single repository, GitHub issues, deployable only in the sense of
"merge and rebundle". Ready means a PR with checks green at its head.

```toml
version = 1

[project]
name = "Switchboard"
repo = "git@github.com:msull/switchboard.git"
base = "main"
space = "Dispatch · Switchboard"

[source]
kind = "github"
repo = "msull/switchboard"
label = "dispatch"            # marking an issue with it is the handover

[[lanes]]
name = "repo"
path = "."
setup = ["cargo", "fetch", "--locked"]

[operators.investigator]
kind = "claude"
guidance = "Read CLAUDE.md, docs/design.md and docs/lessons.md first. Find the code the issue touches, name the files, and say what the on-disk record and rehydration would be before what the pane looks like. Write nothing but your notes."

[operators.planner]
kind = "claude"
guidance = "Write the plan the way docs/design.md sections are written: the record, the signals, the actions and effects, the UI, the tests. One commit, README and design status updated, the app runs at every step."

[operators.reviewer]
kind = "codex"
guidance = "Hold the plan to the layering in src/lib.rs and the hard rules in CLAUDE.md."
[operators.reviewer.review]
reviewer = "codex"
review_first = "The plan at {plan} is for the repository at {worktree}, on branch {branch}. Review it against that repository's CLAUDE.md and docs/design.md: no keystroke injection, no writes into a project's dot-directories, additive contract, schema bump for any serialized change. Write numbered objections to {feedback}. If there are none, write exactly this line alone: {no_feedback}"
review_round = "The planner answered at {response} and updated the plan at {plan} (for the repository at {worktree}). Re-read the plan, reply to each point at {feedback}, and if you are satisfied write exactly this line alone: {no_feedback}"
respond = "Feedback on your plan is at {feedback}. The plan under review is the copy at {plan}: edit that file and only that file, leaving the earlier plan untouched, then answer each point at {response}."
respond_to_user = "The user says: {text}. Edit the copy at {plan} and only that file, then answer at {response}."
handoff = "The plan at {plan} was reviewed and is final."
no_feedback = "No further feedback."
cap = 4

[operators.implementer]
kind = "claude"
guidance = "Implement the finalized plan on the branch. cargo test --locked, cargo clippy --locked --all-targets -- -D warnings, cargo fmt --all. Commit messages carry no tool attribution. Open a PR with gh and write its number to your notes."

[[stages]]
name = "investigate"
operator = "investigator"
context = "root"
writes = ["notes"]
prompt = "Issue #{issue.number}: {issue.title}\n\n{issue.body}\n\nInvestigate and write your findings to {notes}."

[[stages]]
name = "lanes"
gate = { kind = "human", decision = "lanes" }   # one lane: auto below

[[stages]]
name = "plan"
operator = "planner"
context = "each"
writes = ["plan"]
prompt = "Using {inputs.notes}, write a plan for issue #{issue.number} to {plan}."

[[stages]]
name = "review"
review = "reviewer"
context = "each"
subject = "plan"
# A Switchboard plan review on a copy of the plan: the plan attempt's
# session is the source. Done when the run is finalized, which is the
# `finalize` decision below unless the dial says auto. Writes `plan`.
gate = { kind = "external", check = "review-finalized" }

[[stages]]
name = "implement"
operator = "implementer"
context = "each"
writes = ["notes"]
prompt = "The plan at {inputs.plan} was reviewed and finalized. Implement it on branch {branch}, then open a PR against main and write its number to {notes}."
gate = { kind = "command", argv = ["sh", "-c", "cargo test --locked && cargo clippy --locked --all-targets -- -D warnings && cargo fmt --all -- --check"], in = "lane" }

[[stages]]
name = "ready"
context = "each"
gate = { kind = "external", check = "pr-checks" }   # at the branch head; pending is waiting, not failure

[[stages]]
name = "merge"
context = "each"
gate = { kind = "external", check = "pr-merged", decision = "merge" }   # pending on you until GitHub says merged; rebundle is yours

[policy]
slots = 1
waiting_on_me = 2
rates = { "claude-sonnet-5" = [3.0, 15.0], "claude-opus-5-5" = [15.0, 75.0], "gpt-5-codex" = [1.25, 10.0] }
decisions = { lanes = "auto", finalize = "ask", budget = "ask" }
```

`lanes = "auto"` is safe here because there is only one lane; the
decision is trivial and asking would be noise. The merge gate is a
confirmation the provider resolves, so it has no dial, but it is a
pending decision while it waits so the ticket shows as waiting on you.

### Pipeline: PTA

No repository remote, no deploy, no CI. A local git repository of
guides, templates and small scripts run through `inv`. Tickets are
lines of `task-list.md`, and most of them need you (a phone call, a
vote). Dispatch's use is the ones that are writing: a draft, a flyer, a
guide skeleton. Ready means a committed draft or a rendered PDF for you
to look at. Nothing is ever sent.

```toml
version = 1

[project]
name = "PTA"
root = "/Users/sully/Documents/SBPTA"
space = "Dispatch · PTA"

[source]
kind = "task-file"
path = "task-list.md"
marker = "@dispatch"          # a task line carrying this is handed over; Dispatch never edits the file

[[lanes]]
name = "kb"
path = "."
base = "main"                 # in place, on a branch per ticket; the repository is a resource

[operators.writer]
kind = "claude"
guidance = "Follow CLAUDE.md: extract only facts present in the sources, mark the rest _TBD_ with an Open Questions bullet, cite sources at the bottom, keep file names lowercase-hyphenated, never rename or move files. Commit what you produce on the ticket's branch; nothing else."

[operators.reviewer]
kind = "codex"
guidance = "Check every fact against the cited source."
[operators.reviewer.review]
reviewer = "codex"
review_first = "Review the draft at {plan}; the sources and notable-people.md are in the repository at {project.root}. Anything not traceable to a cited source is an objection. Names and contacts must match notable-people.md. Write objections to {feedback}. If there are none, write exactly this line alone: {no_feedback}"
review_round = "The writer answered at {response} and updated the draft at {plan} (sources in {project.root}). Re-read it, reply at {feedback}, and if you are satisfied write exactly this line alone: {no_feedback}"
respond = "Feedback on the draft is at {feedback}. The draft under review is the copy at {plan}: edit that file and only that file, leaving the earlier draft untouched, then answer each point at {response}."
respond_to_user = "The user says: {text}. Edit the copy at {plan} and only that file, then answer at {response}."
handoff = "The draft at {plan} was reviewed and is final."
no_feedback = "No further feedback."
cap = 3

[[stages]]
name = "draft"
operator = "writer"
context = "each"
writes = ["plan"]
# The draft is the reviewable subject, so it is written to {plan}: a
# markdown file in the ticket's directory, whatever it will become.
prompt = "Task: {task.text}\n\nContext: {task.context}\n\nWrite the draft the task asks for (a guide, a template, flyer copy, or a mail) as markdown to {plan}. List the sources you used at its foot."

[[stages]]
name = "review"
review = "reviewer"
context = "each"
subject = "plan"
gate = { kind = "external", check = "review-finalized" }

[[stages]]
name = "render"
operator = "writer"
context = "each"
writes = ["manifest"]
manifest = "manifest"         # Dispatch parses and validates this artifact (see "Manifests")
prompt = "The draft at {inputs.plan} is final. Put it in the repository where CLAUDE.md says it belongs. A flyer or handout is rendered with the matching inv task; a mail is saved with inv mail-draft. Commit on {branch} with a message naming the task. Never send. Write to {manifest} one line per deliverable: `file: <path relative to the repository>` for each file you produced, `draft: <subject line>` for a mail in Drafts."
# Verified, not performed: the tree is clean and the branch is ahead
# of its base. The manifest's own checks are Dispatch's, not shell's.
gate = { kind = "command", argv = ["sh", "-c", "test -z \"$(git status --porcelain)\" && test \"$(git rev-list --count \"$DISPATCH_BASE..$DISPATCH_BRANCH\")\" -gt 0"], in = "lane" }
release = "kb"                # the repository hold ends here, after the manifest's files are copied out and the base checked out

[[stages]]
name = "ready"
context = "each"
gate = { kind = "human", decision = "published", confirm = true }   # shows the copied deliverables and the Drafts subject; you send or post, then say so

[policy]
slots = 1
waiting_on_me = 3
rates = { "claude-sonnet-5" = [3.0, 15.0], "gpt-5-codex" = [1.25, 10.0] }
decisions = { lanes = "auto", finalize = "ask", budget = "ask" }
```

What this pipeline showed: a source that is a file rather than an
issue tracker, a lane that works in place, a reviewable subject that is
the deliverable itself, and a "ready" that is a human act Dispatch
cannot verify. The in-place lane means the repository is a resource:
a ticket holds it from `draft` through `render`, and `release` names
where the hold ends. Before releasing, Dispatch copies the manifest's
files into the attempt directory, so the PDF you are asked to post
still exists after the next ticket checks out its own branch, and
checks out the lane's base so the next ticket starts where it expects.
Three finished drafts can therefore wait for you while the next
ticket writes. A mail in Drafts cannot be verified from here; the
decision shows the subject line the writer recorded. The `marker` on a task line is the only thing
Dispatch reads from the project as configuration, which matches
Switchboard's rule for `.switchboard/project.json`.

### Pipeline: Delta

The hard case, and the one the vocabulary was shaped for. Facts from
`~/code_repos/delta` (its `CLAUDE.md`, `guides/ENVIRONMENTS_GUIDE.md`,
`guides/COLLABORATION_GUIDE.md`, the backend `tasks/`):

- The root is a planning workspace (`k3systems/delta-workspace`), and
  the issues live there with a label scheme (`type:`, `area:`,
  `priority:`, `status:`). The application repositories are separate
  git repositories nested under it: `delta-backend` (integration branch
  `main`), `delta-frontend` (`dev`; `master` is production),
  `delta-snp` (`master`). Bitbucket is the origin; GitHub mirrors
  exist. PRs go through `tools/bb.py`, which squashes on merge.
- "Ready for the test environment" pre-merge means `sully-dev`: one
  personal backend stack, deployed from any branch by
  `aws-vault exec -n deltadev -- uv run inv deploy -f` from
  `delta-backend` with that env linked. Last deploy wins. The link is
  a per-checkout cache file, so a fresh worktree is unlinked until
  `inv link-env --env-name sully-dev` runs in it. The frontends have no
  per-branch environment; a branch is tried against `sully-dev` with
  `npm run link-env` and a local `npm start`. Shared `dev` deploys
  itself on merge to `main` and is post-merge, so it is not a gate.
- Machine checks: backend `uv run inv lint` and `uv run inv pytest`;
  frontend `npm ci --legacy-peer-deps` then
  `CI=true npm test -- --watchAll=false`; snp `npm test`. Bitbucket
  Pipelines runs tests on PRs but has no lint gate, and the GitHub
  mirrors have no CI, so `pr-checks` here reads Bitbucket through
  `bb.py` rather than `gh`.
- A PR is ready only with a self-contained body, an updated backend
  `CHANGELOG.md` "Unreleased" entry, a What's New entry for every
  user-visible frontend change, tests listed in the body, and no
  session-link attribution of any kind.
- Nothing in the repos locks an environment. Two tickets deploying to
  `sully-dev` would overwrite each other, which is exactly what a
  resource with `count = 1` prevents.

```toml
version = 1

[project]
name = "Delta"
repo = "git@github.com:k3systems/delta-workspace.git"
space = "Dispatch · Delta"

[source]
kind = "github"
repo = "k3systems/delta-workspace"
label = "dispatch"
# area: labels are the suggested answer to the lanes decision.
lane_hints = { "area:backend" = "backend", "area:frontend" = "frontend", "area:snp" = "snp" }

[[lanes]]
name = "backend"
path = "delta-backend"
repo = "git@bitbucket.org:cainfosec/delta-backend.git"
base = "main"
setup = ["sh", "-c", "uv sync && uv run inv link-env --env-name sully-dev"]

[[lanes]]
name = "frontend"
path = "delta-frontend"
repo = "git@bitbucket.org:cainfosec/delta.git"
base = "dev"
setup = ["npm", "ci", "--legacy-peer-deps"]
# A service Dispatch starts for a stage that asks. PORT is a port
# Dispatch allocates per ticket; the URL is what the tester is told.
serve = { argv = ["npm", "start"], env = { BROWSER = "none", PORT = "{port}" }, url = "http://localhost:{port}", ready = { http = "/", within_secs = 120 } }

[[lanes]]
name = "snp"
path = "delta-snp"
repo = "git@bitbucket.org:cainfosec/delta-snp.git"
base = "master"
setup = ["npm", "ci", "--legacy-peer-deps"]
serve = { argv = ["npm", "start"], env = { BROWSER = "none", PORT = "{port}" }, url = "http://localhost:{port}", ready = { http = "/", within_secs = 120 } }

[[resources]]
name = "sully-dev"
count = 1

[operators.investigator]
kind = "claude"
guidance = "Read the root CLAUDE.md, guides/ENVIRONMENTS_GUIDE.md and each repository's CLAUDE.md. Name the files, the endpoints and the screens the issue touches. Say which lanes it needs and why; that is a decision the human sees. Write nothing but your notes."

[operators.planner]
kind = "claude"
guidance = "Write the plan as plans/YYYYMMDD-name.md is written: per lane, the change, the tests, the CHANGELOG or What's New entry, and how it is tried on sully-dev."

[operators.reviewer]
kind = "codex"
guidance = "Hold the plan to COLLABORATION_GUIDE."
[operators.reviewer.review]
reviewer = "codex"
review_first = "The plan at {plan} is for the repository at {worktree}, on branch {branch}. Review it against that repository's guides/COLLABORATION_GUIDE.md: branch naming, self-contained PR body, changelog entries, 100% coverage on new frontend components, no attribution lines. Check the backend and frontend halves agree on the API. Write objections to {feedback}. If there are none, write exactly this line alone: {no_feedback}"
review_round = "The planner answered at {response} and updated the plan at {plan} (for the repository at {worktree}). Re-read the plan, reply at {feedback}, and if you are satisfied write exactly this line alone: {no_feedback}"
respond = "Feedback on your plan is at {feedback}. The plan under review is the copy at {plan}: edit that file and only that file, leaving the earlier plan untouched, then answer each point at {response}."
respond_to_user = "The user says: {text}. Edit the copy at {plan} and only that file, then answer at {response}."
handoff = "The plan at {plan} was reviewed and is final."
no_feedback = "No further feedback."
cap = 4

[operators.implementer]
kind = "claude"
guidance = "Implement the lane's part of the finalized plan on branch {branch} in this worktree only. Run the lane's checks before every commit. Never run bb.py promote or any deploy. Guidance is advisory: the deploy this pipeline performs is Dispatch's, not yours."

[operators.tester]
kind = "claude"
guidance = "The backend branch is already deployed to sully-dev; the commit is in your prompt. Do not deploy. The frontend worktree is linked to sully-dev and already being served at the address in your prompt; do not start another. Exercise the change with the tools/e2e probes or curl. Write what worked and what did not, with the commands and their output, to {notes}."

[[stages]]
name = "investigate"
operator = "investigator"
context = "root"
writes = ["notes"]
prompt = "Issue #{issue.number}: {issue.title}\n\n{issue.body}\n\nInvestigate and write findings, including the lanes needed, to {notes}."

[[stages]]
name = "lanes"
gate = { kind = "human", decision = "lanes" }   # which worktrees to cut; the label hints and the notes are shown

[[stages]]
name = "plan"
operator = "planner"
context = "joined"
writes = ["plan"]
prompt = "Using {inputs.notes}, write one plan covering the lanes {lanes} of ticket #{issue.number} to {plan}."

[[stages]]
name = "review"
review = "reviewer"
context = "joined"
subject = "plan"
gate = { kind = "external", check = "review-finalized" }

[[stages]]
name = "implement"
operator = "implementer"
context = "each"
prompt = "The plan at {inputs.plan} is final. Implement the {lane} part on {branch}."
gate = { kind = "command", in = "lane", per_lane = { backend = ["sh", "-c", "uv run inv lint && uv run inv pytest"], frontend = ["sh", "-c", "CI=true npm test -- --watchAll=false"], snp = ["sh", "-c", "CI=true npm test"] } }

[[stages]]
name = "deploy"
context = "lane:backend"      # skipped when the ticket has no backend lane: the frontend is then tried against what sully-dev already has
needs = ["sully-dev"]
# Dispatch deploys, once, after linking again so the target cannot be
# whatever a previous checkout left; the deployed commit is recorded
# on the attempt.
gate = { kind = "command", in = "lane:backend", argv = ["sh", "-c", "uv run inv link-env --env-name sully-dev && aws-vault exec -n deltadev -- uv run inv deploy -f"] }

[[stages]]
name = "try"
operator = "tester"
context = "joined"
needs = ["sully-dev"]
services = ["frontend", "snp"]   # each started only if its lane was cut; owned by the ticket until `tried` ends
before = { frontend = ["npm", "run", "link-env"], snp = ["npm", "run", "link-env"] }
writes = ["notes"]
prompt = "sully-dev is running backend commit {inputs.deploy.commit} (when that reads as skipped, this ticket has no backend lane and sully-dev runs whatever was deployed last). The admin frontend: {services.frontend}. The student portal: {services.snp}. A lane this ticket did not cut is not served; test it, if at all, against the existing deployment. Try ticket #{issue.number} end to end and report to {notes}."

[[stages]]
name = "tried"
needs = ["sully-dev"]        # still held: what you are looking at must stay deployed
gate = { kind = "human", decision = "tried", confirm = true }   # the tester's evidence and the deployed commit are shown

[[stages]]
name = "ready"
operator = "implementer"
context = "each"
writes = ["notes"]
prompt = "Open the PR with bb.py create: self-contained body, tests listed, changelog and What's New entries present. Write the PR id to {notes}."
gate = { kind = "external", check = "pr-checks", provider = "bitbucket" }

[[stages]]
name = "merge"
context = "each"
gate = { kind = "external", check = "pr-merged", provider = "bitbucket", decision = "merge" }   # pending on you until Bitbucket says merged; backup.py is yours

[policy]
slots = 2                    # two tickets in flight; only one can hold sully-dev
waiting_on_me = 2
ports = [3100, 3199]         # for services; one per service per ticket, tested free before use
rates = { "claude-sonnet-5" = [3.0, 15.0], "claude-opus-5-5" = [15.0, 75.0], "gpt-5-codex" = [1.25, 10.0] }
decisions = { lanes = "ask", finalize = "ask", budget = "ask" }
```

What this pipeline showed, and what it added to the vocabulary:

- **Lanes with a `base` and a `setup`.** Each repository has its own
  integration branch and install step. The backend's setup also links
  the worktree to `sully-dev`, and the deploy gate links again right
  before deploying, so the target is established twice and never
  inherited from another checkout.
- **`joined` stages.** Plan and review happen once per ticket across
  lanes; implementation happens per lane. That distinction did not
  exist until a multi-repo project needed it.
- **`lane:backend` as a context**, and a stage that is skipped when
  its lane was not cut. A frontend-only ticket is tried against
  whatever backend `sully-dev` already runs, and the `tried` decision
  says so.
- **Services are Dispatch's.** A stage's `services` name lanes; each
  is started only if the ticket cut that lane, and a template field
  for one that was not renders as `not served (no <lane> lane)`, so a
  backend-only, frontend-only or portal-only ticket names nothing
  that does not exist. For each lane that was cut, in order: the
  `before` command runs as a tracked `command.run` operation and must
  exit zero; Dispatch allocates a port from the policy's `ports`
  range, testing that it binds before choosing it, so a server you
  started by hand on the default port is simply not chosen; the
  service is started through `service.new` with `{port}` filled in;
  and readiness is the `ready` probe answering on that URL within its
  limit. A `before` failure, no free port, or a probe that never
  answers is a decision before the tester is launched. The tester is
  told each URL and not to start a server of its own. Services live on
  the ticket record until the last stage holding the resource ends.
- **One owner of the deploy.** Dispatch runs it as a command gate,
  records the commit, and the tester is told the commit and told not
  to deploy. The `tried` decision shows the tester's evidence file and
  that commit, so what you look at is what was tested.
- **A resource held across three stages.** `needs` on consecutive
  stages is one hold from the first to the last. Losing it in the
  middle would let a second ticket overwrite the stack while you are
  looking, which is why holds are on the record and not in memory.
- **`lane_hints`.** The issue's `area:` labels and the investigator's
  notes are shown with the lanes decision. It still starts at `ask`.
- **Guidance is advisory.** Nothing in Dispatch stops an agent from
  running a deploy; the operator is told not to, and the only deploy
  in the file is Dispatch's own. If that is not enough the next step
  is an environment the agent's credentials cannot reach, not more
  guidance.
- **A count is not an environment.** Raising `sully-dev` to 2 would
  need a second stack to exist and each ticket bound to one (a
  `link-env` name per hold, and the frontend linked to match). The
  mechanism is a resource with instances rather than a count, and it
  waits for the day a second stack exists.

## Manifests

A stage that names a `manifest` artifact has it parsed by Dispatch
when it settles, before the stage's gate runs, and the parse is part
of completion. The format is one entry per line: `file: <path>` for a
regular file, relative to the lane's root and staying inside it after
resolving, or `draft: <text>` for something outside the repository
that only a human can find. Anything else is a parse error. The rules
fail closed: an unreadable or empty manifest, a line that matches
neither form, a `file:` path that is missing, not a regular file, or
outside the root, or a manifest with no valid entry at all, each fail
the attempt with the line named. Dispatch then copies every `file:`
entry into the attempt directory under its relative path, so two
deliverables with the same basename in different directories stay
apart, fsyncs the copies and records them on the attempt as the
`deliverable` artifact, and only then runs the gate, restores the
base, and releases the hold. The tests for the parser are: unreadable,
empty, missing file, draft-only (valid), a line in neither form, a
path escaping the root, and two entries sharing a basename.

## Resources and slots

- A hold is taken by writing it onto the ticket record under the
  writer lock, checked against every other ticket's holds. It is
  released when the last stage naming it completes, when the ticket is
  cancelled or a decision rejects it, and never on a restart.
- An in-place lane (no `worktrees`) is an implicit resource named for
  the repository path, held from the first stage in that lane to the
  stage whose `release` names it (the last stage in the lane when none
  does). Before a ticket takes it, the tree must be clean and on the
  lane's base; a dirty tree is a decision, not a stash. Releasing it
  is Dispatch checking out the base again, after copying whatever the
  ticket still needs out of the tree.
- A hold is released only after every writer that could touch the
  resource is confirmed stopped, by the cancellation sequence under
  "Decisions"; a deploy still running keeps `sully-dev`.
- Holds cover Dispatch tickets only. Dispatch cannot see a manual
  deploy to `sully-dev`; `dispatch status` shows the holds so you can
  look before doing one by hand.
- `slots` counts tickets that have a running attempt or hold a
  resource. A ticket sitting at a human gate with no holds costs no
  slot, and neither does a gate-only stage (`lanes`, `ready`) or a
  ticket closing past its last stage: they launch nothing, so they
  run beside a full project. When `waiting_on_me` pending decisions exist in a project,
  nothing new is started there until one is answered; running attempts
  finish.

## PR checks and merges

`pr-checks` is bound to a PR id, its repository and provider, and the
branch's head revision at the time of the check. Its readings are:
pending (wait), green at the head (pass), red at the head (a decision
with the failed check named), no PR or no checks configured (a
decision), and any lookup error (retry with backoff, a decision after
an hour). Green at an older head is not green: a moved head voids it
along with every other result made against the old head set, as
described under "Stage semantics". `pr-merged` reads the same PR and
resolves the pending merge decision; it never merges.

The provider is read from the lane's remote (the lane's `repo`, else
the project's, else the tree's own `origin` for a project named by
`root`): a `github.com` remote is read through `gh`, a
`bitbucket.org` remote through the Bitbucket Cloud API with `curl`
(the account token from `BITBUCKET_EMAIL` and `BITBUCKET_API_TOKEN`
in the runner's environment, else `<data dir>/env`, handed to curl on
stdin and never logged; Bitbucket reports a short head, which matches
the tree's by prefix); `provider` on the gate overrides the guess.
Another host parks the ticket saying so, as does a tree with no
remote at all, whose "ready" is a human gate (the PTA pipeline). A repository that runs no CI says `checks = "none"` on the
stage, and the gate then passes on an open PR whose head is the
tree's head, without reading checks; without it, a PR with no checks
is the decision above, every time. Every reading but pending and
green is one decision, `pr`, with `recheck` and `park`: the attempt is
never failed, since the work is done and the world around it is what
needs a look (open the PR, push the branch, fix the workflow), and
`recheck` reads again on the next pass instead of after the poll
interval of a minute. A merged PR passes whatever its checks say. The
PR is recorded on the attempt (provider, repository, number, url, the
head it was at, what its checks said, when) and shown on the ticket
page.

A PR the provider reports as conflicting with its base (GitHub's
`mergeable`; Bitbucket does not say) is rebased rather than asked
about when the policy names a `rebaser`: an agent attempt of the same
stage in the lane, made by `session.clone` from the lane's last
finished agent so it knows the change, told the PR, the base, the plan
and where its notes go, and asked to rebase, resolve, run the checks
and push with `--force-with-lease`. The gate's own attempt stays open
and reads the PR again once the rebaser has stopped and its notes
settled. The conflicting head is on the rebaser's attempt: a rebase
that leaves the PR at that head, a policy without a rebaser, and a
spent `max_rebases` are each a `pr` question with `recheck`.

Red checks on the PR at the tree's head are handled the same way by
the policy's `fixer`: cloned from the lane's last finished agent, told
the PR and the failed check names, how to read the failed run (`gh pr
checks`, `gh run view --log-failed`), and asked to fix the cause on
the branch, run the checks locally, commit and push. What failed is on
the fixer's attempt (`failed: <names>`), so a fix that leaves the PR
at the same head, no fixer, and a spent `max_fixes` are each a `pr`
question. Rebases and fixes are counted separately; each kind counts
only agents that ran.

## Tickets from pull requests

The other kind of work: someone else's change, which the user tests,
reviews and signs off rather than makes. It is a ticket like any
other, on a second pipeline file per project, `<project>.pr.toml`,
whose `[source]` is `kind = "pull-request"` and whose lanes are the
project's. The queue, the slots and `waiting_on_me` are the project's,
shared with its issue tickets.

```
dispatch take Delta pr backend/123 frontend/45
dispatch take Switchboard pr 12          # the lane may be left off when there is one
```

Each spec names a lane and a pull request number; the PR is read from
the lane's repository on its provider (GitHub or Bitbucket, by the
remote's host) and must be open. One ticket carries one PR per lane;
two lanes that share the project's repository cannot each carry one.
The identity is the set of PRs, so a PR on a live ticket is refused
until that ticket closes. The snapshot records each PR's lane,
provider, repository, number, URL, branch, head and title; the
ticket's title is the first PR's.

The lanes are the PRs' own branches: a lane with a repository of its
own is a worktree of Dispatch's clone on that branch, tracking the
remote (`git worktree add --track -B <branch> <remote>/<branch>`); a
PR in a lane without one makes the tree itself that branch. Lanes no
PR is in are not cut. Nothing in this mode pushes: a pull-request
pipeline refuses a `rebaser` or `fixer` in its policy, and the
conflicting or red readings of `pr-checks` stay questions for the
author to act on. Each time a human gate opens, the branches are
brought up to what the remote has (fetch, fast-forward); a branch that
no longer fast-forwards, a force push, parks the ticket, since what
was looked at is gone.

The pipeline is the tail of an issue's: what runs on the branch (the
project's `deploy` and `try` stages, once built, bind to the PR head
like any command gate), an `inspect` human gate for the sign-off, and
`pr-merged` to watch the merge. The sign-off is recorded on the ticket
only: approval and feedback go to the provider by the user's hand for
now, and reviewers' notes, when the review stage gains a notes-only
mode, stay private to the user. The ticket closes when the provider
reports the merge, as an issue ticket does.

Later: a notes-only review stage before the sign-off; posting the
approval or a request for changes to the provider from the decision;
a PR that moves after sign-off asking again.

## Budget

Budgets are reporting, not enforcement. Switchboard reads token counts
from transcripts; Dispatch multiplies them by the `rates` table for
the model each session ran with, counted from the attempt's start so a
cloned transcript's history is not charged twice. A session whose
usage cannot be read is shown as unknown, not zero. Crossing an
operator's or the project's budget is a decision that blocks the
*next* launch for the ticket; it never stops a running agent.
Operator budgets are per ticket across the operator's attempts.

## Control port

The one thing Switchboard has to grow. A Unix socket
`<data dir>/control.sock`, mode 0600, newline-delimited JSON, one
request per line, one reply per line. The wire contract is its own
small set of commands and queries, validated before anything reaches
the core; it is not the `AppAction` enum, which carries adapter results
and startup actions that nothing outside may send.

Every request carries an operation id `op`. Each command has one
terminal reply naming it: for a command that only changes records,
`persisted {ids}` or `failed {reason}`; for one that starts a process,
`launched {ids}` or `failed {reason}`, sent after the launch effect
has run, with the record persisted first. There are no intermediate
replies. Switchboard stores `op` on every record the command makes and
answers `find {op}` with those records and their state, which is both
the deduplication (a repeated `op` is answered from the records, not
acted on again) and the recovery query. `op.status {op}` answers
`unknown`, `in progress` (an effect of the operation is still
running), `interrupted` (Switchboard shut down with one running: it
marks the record with the pending effect before running it and clears
the mark after), or the terminal reply again.

Commands fall into three classes, and recovery treats each
differently:

- *Creations* (`project.add`, `session.new`, `service.new`,
  `command.run`, `space.new`, `set.new`, `workflow.start`) are found
  by `find {op}`. "Not found means nothing ran" holds for them because
  of the operations log, not a convention: before running any effect,
  Switchboard appends `{op, kind, ids, time}` to `<data
  dir>/operations.log` and persists the record; the log is
  append-only, and removing the record through the window never
  touches it. `find {op}` answers from the log first, so a record you
  removed by hand is reported as `made, removed` rather than absent.
  Only an `op` missing from the log means nothing ran.
- *Idempotent state* (`session.notes`, `session.waiting`,
  `session.kill`, `session.remove`, `workflow.pause`,
  `workflow.finalize`, `workflow.remove`, `set.sync`) is recovered
  from the state itself: repeating one is harmless, so a lost reply
  is answered by sending it again.
- *Non-replayable* (`session.send`, `workflow.continue`): a repeat
  can spend money twice. A lost reply to one is not repeated; it is a
  decision that shows what was sent and lets you look at the pane.

An operation whose status is `in progress` is waited for, not judged:
Dispatch restarting while Switchboard is still cloning a planner finds
a run in `Starting`, asks `op.status`, and waits. Only `interrupted`,
or `unknown` with the record present and no pane, makes the attempt
fail.

Commands:

- `project.add {space, name, root}` → `project`; `project.remove`;
  `project.rename {project, name}`; `project.root {project, root}` (the
  tree moved)
- `session.new {project, kind, cwd, launch, prompt?, notes}` →
  `session`; `session.send {session, text}`; `session.kill`;
  `session.remove`; `session.notes {session, text}`;
  `session.waiting {session, on: bool, reason}`;
  `session.trust {session}` (Claude Code's folder trust question,
  reported on the session view as `trust_question`, answered yes);
  `session.clone {source, name, prompt, notes}` → `session`: a Claude
  Code session whose conversation is a copy of `source`'s whole
  transcript, in its project and cwd, launched with `prompt`; the
  record exists (with the op) before the copy does, and a source with
  no transcript is refused
- `service.new {project, name, argv, env}` → `session`: a service
  record Dispatch owns, killed and removed by it; whether it is
  listening is Dispatch's probe, not Switchboard's reply
- `command.run {project, name, argv, env}` → `session`; the exit code
  is read back with `session.get`
- `space.new {name}` → `space`; `set.new {space, name}` → `set`;
  `set.sync {set, items: [{target, rect}]}`: the set's pins become
  exactly this list, in one action: pins not listed are removed,
  listed ones placed or moved, and an overlap or a target outside the
  set's workspace fails the whole request without changing anything.
  Idempotent, so it is also how the queue view is redrawn.
- `workflow.definitions.install [definition]`;
  `workflow.start {source, plan, definition}` → `run` with its
  reviewer session, and the planner clone once made;
  `workflow.pause {run}`; `workflow.continue {run}`;
  `workflow.finalize {run}`; `workflow.remove {run}`

Queries:

- `projects {space?}`, `spaces`, `sets {space}`
- `sessions {project}`, `session {id}` (record, card state, activity,
  host liveness, last exit, the last Stop event's time, how long the
  pane has been quiet, notes, usage), `find {op}` (every record made
  by that operation, with the same state)
- `waiting`, `workflow {run}` (state, round, cap), `workflows
  {project}`
- `file.stat {path}` (the round-file probe, for a path under
  Dispatch's own directory)

Every command from the port is quiet: it takes an explicit `space` or
`project`, never changes what the window shows (the core's "select the
new thing" step, including the review view that starting a workflow
opens, is skipped for it), and never opens a terminal window for a
launch whatever the open-terminal-on-launch setting says. That is one
`quiet` flag on the internal actions, additive.

Trust: anything that can write the socket can already write the data
directory. The port launches agents on request, which is the same
trust the script runner has. Switchboard's own restart still launches
nothing; every session the port made is an ordinary record afterwards.

## Surfacing

Everything you need to see goes through Switchboard, since the badge,
the rail count, working sets, the controller and the keys already
exist:

- All of a project's tickets live in one workspace, `Dispatch ·
  <project>`. Each lane is a project named `#<n> <lane>`, the root
  context a project named `#<n>`. Working sets only pin what is in
  their own workspace, so this is what makes the next point possible.
- One working set per project holds the current session of every
  ticket in flight or waiting, laid out in queue order, top to bottom.
  It is a view: Dispatch computes the whole layout from the queue and
  each ticket's current session and sends it as one `set.sync`, when
  a ticket's current session changes and when the queue is reordered,
  so a replaced card never leaves the old one behind and a swap of
  two positions never collides with itself. The order of record is the
  queue on Dispatch's side, edited with `dispatch queue`, because a
  grid has no order of its own. A ticket card of Switchboard's own
  (one that stays put across a ticket's fresh sessions and shows its
  stage) would be better and is a later Switchboard feature, not a
  precondition.
- A pending decision marks the ticket's current session waiting
  through `session.waiting`, so the Dock count and the rail include
  it, and the decision text is in its notes.

Dispatch also has a port of its own, so a reader can see tickets as
they are rather than through the sessions they made. While `dispatch
run` is up it serves `<data>/dispatch.sock` (wire crate
`dispatch-control`): `status` answers every project's queue and every
ticket as a view (stage names resolved, attempts, decisions, lanes,
artifact paths), `ticket` one in full, `artifact` the text of a file
under a ticket's directory and nothing outside it, and `decide`,
`queue`, `take`, `resume` and `worktrees` do exactly what the command
line does, through the same runner methods under the same writer lock. Switchboard's Dispatch
page is a client of this port and knows nothing of the records; a
runner on another machine looks the same through a forwarded socket.
The port is served only by the runner that holds `runner.lock`, so a
socket file left by a dead runner is never a live one.

## The first slice

The Switchboard pipeline only, and no autonomy. A command:

```
dispatch take switchboard <issue>
```

that makes the ticket record, the worktree, the workspace and the two
projects, runs `investigate` in the root context, takes the trivial
lanes decision, runs `plan` in the lane, and starts the review run
with the plan attempt's session as the source, then stops at the
`finalize` decision. You answer it with `dispatch decide`, which calls
`workflow.finalize`. That exercises the control port, the pipeline
file, the attempt records, one root and one lane context, the
default output gate, an external gate, and a human gate.

The second slice adds `implement`: an agent stage in every chosen
lane with a command gate. The gate is built as a child process of the
runner rather than a Switchboard command record for now (the port has
no per-record environment yet); its output goes to the attempt's
`checks.log`, shown on the ticket page as the `checks` artifact.

Acceptance, each as a test against a fake Switchboard on the socket
and one against the real one:

| Case | Expected |
|---|---|
| Issue to finalize decision | Two projects; four sessions (investigator, planner, the run's reviewer and planner clone), each carrying its operation id; one run; the decision names the review copy of the plan |
| Dispatch killed after `session.new` sent, before the reply written | On restart `find` returns the session; the attempt continues; no second launch |
| Same, and the session was removed in the window before Dispatch restarted | `find` answers `made, removed` from the log; the attempt fails as removed by hand; no second launch |
| Dispatch killed after `workflow.start` sent, before the reply written | On restart `find` returns the run and its sessions; the attempt continues |
| Dispatch killed before `session.new` sent | Attempt `lost`, a decision, no launch |
| Switchboard killed between persisting the reviewer and launching it | `find` returns the record with no pane; the attempt fails; a decision |
| Investigator killed after a partial notes file | No Stop recorded; the attempt fails; nothing advances |
| A review with one objection | The copy in the review attempt changes; the plan attempt's file does not |
| Dispatch restarts while the planner clone is still being made | `op.status` is `in progress`; Dispatch waits; the attempt continues |
| The implementer stops on a clean tree | The checks start at the tree's head in the lane with `DISPATCH_*` in their environment; the agent is killed; the attempt stays open until they exit; exit 0 binds the head and completes it |
| The checks fail | A failed attempt and a rerun decision with `rerun`, `check` and `park`; nothing retried on its own; a rerun is a fresh agent, `check` runs the checks again on the same attempt with no agent |
| The tree is dirty when the agent stops | No check runs; a failed attempt and the same decision |
| A stage fails past the policy's `max_reruns` in one context | The ticket parks with the count and the last reason; nothing is asked |
| The runner restarts while the checks run | The lost check starts again on the same head; no second agent |
| `ready` with the PR's checks pending, then green | A gate-only attempt per context, no agent; no PR is a `pr` decision (`recheck`, `park`); pending waits and reads the provider once a minute; green at the tree's head completes the attempt bound to that head |
| The PR is at another head, its checks are red, or it has no checks | A `pr` decision naming which; `recheck` reads again at once; the same attempt throughout; `checks = "none"` on the stage passes on the PR at the head alone |
| The provider cannot be read | The error is recorded on the attempt and retried quietly for an hour, then a `pr` decision; a merged PR passes |
| `ready` with every slot taken by another ticket's agent | The PR is still read and the ticket still closes; a gate-only attempt holds no slot and the other agent keeps its own |
| A human gate (`inspect`) after `implement` | One gate-only attempt and one decision per lane, naming the branch and head, what it adds over its base, the tree and the notes; `proceed` completes it bound to the head; `park` stops |
| `rerun` with a note at `inspect` | That lane's `implement` result and the gate's attempt are cancelled, the ticket stands at `implement` again, a fresh implementer gets the note at the end of its prompt, other lanes are untouched, and `inspect` asks again on a new attempt when it is done |
| `merge` with the PR open | A confirmation decision with only `park`, the session marked waiting; `merged` by hand is refused; the PR is read once a minute |
| The user's own feedback round after the review converged | The pending `finalize` decision is cancelled and the session unmarked while the planner answers; when the run converges again a new decision names the new round count |
| The PR conflicts with its base at `merge` | The policy's rebaser starts in the lane, cloned from the implementer's session, with the PR, the base and the notes path in its prompt; the merge decision stays; when it stops the gate reads the PR again and, merged, the ticket closes |
| The rebaser leaves the PR at the same head, or `max_rebases` is spent | A `pr` decision saying which; no further rebaser runs |
| The PR's checks are red at the tree's head and the policy names a `fixer` | The fixer starts in the lane, cloned from the implementer, with the PR and the failed check names in its prompt; no question; when it stops the gate reads again and green checks pass it |
| Red checks with no fixer, or `max_fixes` spent | A `pr` decision with `recheck` and `park` |
| The provider reports the merge | The attempt completes at the merged head, the decision reads as answered `merged` by `dispatch`, and the ticket goes on (closes) |
| The queue view after `plan` replaces `investigate`, and after two tickets swap places | One card per ticket, in order, no stale card, no overlap failure |
| Plan session has no transcript yet | `workflow.start` fails; the attempt is failed and a decision, not retried |
| Plan file from an earlier attempt exists | The new attempt's own path is empty, so nothing advances |
| `project.add` fails to save | `failed` reply; attempt failed; nothing else made |
| `take <project> pr <lane>/<n>...` | A ticket on `<project>.pr.toml` with one PR per named lane, refused for a closed PR, an unknown or repeated lane, two lanes in one repository, or a PR already on a live ticket |
| A pull-request ticket's first pass | Each PR's lane is a worktree on the PR's branch tracking the remote, chosen; no branch of Dispatch's own; lanes without a PR are not cut |
| The sign-off gate opens on a pull-request ticket | Each branch is fetched and fast-forwarded first; a force push parks the ticket; `proceed` leads to `pr-merged`, and the merges close the ticket |
| The socket fails mid-pass (Switchboard quit or restarted under the runner) | Nothing is parked; the pass ends with a log line and the next one goes on; the port remakes its connection and sends the request again, which the operations log makes safe |
| User is viewing another workspace during the whole path | The window stays on it through every launch and the review start; no terminal window opens |

Then, in order: `implement` with its command gate bound to a commit,
tested with one lane committing after another lane's checks finished;
`ready` and `merge` from GitHub and the human gate-only stage (built,
as above; a moved head is not yet voided); the code review stage
(`docs/review-stage-plan.md`); the queue and slots; the PTA pipeline
with its in-place hold; the Delta pipeline with the deploy gate and
the persisted `sully-dev` hold; budget reporting; the `recommend`
dial.

## Decisions I made (review these)

1. **Dispatch is a separate process over a control socket**, not a
   library user of the switchboard crate and not a thread inside the
   app. Reason: one owner of each data directory and the window; the
   deterministic core makes the socket cheap.
2. **Pipelines are TOML in Dispatch's data directory**, one per
   project, never in the repository. Reason: the hard rule about
   dot-directories, and the config-editor precedent for the one
   exception.
3. **Three gate kinds** (command, external, human) plus the default
   output gate, and no step language.
4. **Fresh session per attempt, attempt-scoped files as the memory.**
   Costs more than continuing one session. Reason: each stage reads
   what is on disk, and a stale file can never pass a new gate.
5. **A reviewer operator is a full workflow definition**, installed
   into Switchboard under a `Dispatch: ` prefix on load. The planner
   is always Claude Code, because the workflow clones its transcript.
6. **Reviewer objections are not intercepted.** The workflow already
   loops planner and reviewer; the human's interception point is the
   `finalize` decision, which shows the rounds. A structured objection
   channel would be a new workflow mechanism and is not designed here.
7. **Confirmation gates have no dial.** Merge, publish and tried are
   verified where a provider can say so and otherwise answered by you.
8. **One workspace per project, not per ticket.** Reason: working sets
   only pin their own workspace, and one view of the queue is worth
   more than tickets hidden from each other, which was never a real
   boundary anyway.
9. **The queue is a Dispatch record edited by command**; the working
   set is a view of it. A Switchboard ticket card is later work.
10. **Dispatch owns the deploy**, as a command gate that links the
    environment right before deploying; agents are told not to, and
    that telling is advisory.
11. **Holds are per-record and never released on restart.** Manual
    deploys are outside them.
12. **Budgets report and block the next launch**; they never stop a
    running agent.
13. **`recommend` is named but not designed.**
14. **Correlation is an operation id stored on Switchboard's records
    and in an append-only operations log**, not a tag in notes.
    Reason: notes are editable, the review run makes sessions of its
    own, and a record can be removed in the window; the log is what
    makes "absent means never ran" true rather than assumed.
15. **Ordinary stage operators are Claude Code only.** Reason: its
    Stop hook is the one completion signal that means finished; idle
    is not.
16. **A review runs on a copy of the reviewed artifact.** Reason: the
    original stays auditable and the round files land in the review's
    own directory.
17. **Evidence carries the heads it depends on**, and a moved head
    voids everything after the earliest affected stage rather than
    only repeating `pr-checks`.
18. **Release is a sequence** that stops writers and confirms them
    gone; a hold that cannot be safely released stays held.
19. **Per-lane results bind to their own lane's head**, joined
    results to all; a moved head parks the ticket and the rerun is
    authorised by one decision.
20. **Servers an agent needs are started by Dispatch** through the
    port, never by the agent in its shell.
21. **Manifests are parsed by Dispatch**, fail closed, and their files
    are copied out before the checkout moves.
22. **A service is started only for a lane that was cut**, on a port
    Dispatch allocated and tested, and counts as up only when its
    readiness probe answers.
