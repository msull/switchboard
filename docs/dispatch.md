# Dispatch

A system that sits on top of Switchboard the way Switchboard sits on top
of tmux and Prompt Box. This is its design, with three real pipelines
written by hand to test the vocabulary. The choices behind it are
collected at the end, under "Design choices".

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
rename) under a single-writer lock file. Slow external work inside a
runner step, such as removing a closed ticket's tree, runs with that
lock let go, and the record is read again before the next write. It
never writes into a repository's own dot-directories; only the agents
commit code. Same hard rule as Switchboard.

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
a shell-special character is refused at `take`.

With `[policy] confine = true`, the pipeline commands Dispatch runs as
its own children (a lane's setup, command gates, a review round's
checks and command reviewers) run in a sandbox that allows writes only
to:

- every tree of the ticket, and the primary tree;
- the attempt or round directory the command reports into;
- the lane's `writable` paths, where its tools keep their caches
  (`~/.cargo`, `~/.cache/uv`, `~/.npm`); a root-context command gets
  those of every lane whose tree it has (the ticket's lanes, and a lane
  at `.` even before lanes are chosen);
- a base set that is always allowed: the system temp directory, the
  devices a shell and a pty need (`/dev/null`, `/dev/tty`, `/dev/fd`,
  `/dev/ptmx`, `/dev/ttys*`), and a linked worktree's own git directory
  (`<clone>/.git/worktrees/<name>`, which holds its index), never the
  clone's shared `.git`.

Reads are not limited. `network = "deny"` (the policy's, or a command
gate's own) closes connections off the machine and keeps loopback, so
a test that talks to a local database still runs. A denied write fails
the command with `EPERM`, and the check's log (`checks.log` for a gate,
the `stderr` file for a reviewer) starts with a header naming what the
command could write and ends with the kernel's `deny(1) file-write-…
<path>` lines for it. A denied setup parks the ticket with the same
header and lines in the reason. A project that works in place (`root`)
is protected less: its tree is the user's own checkout, and that is
writable. On macOS the mechanism is `sandbox-exec`; on any other
platform the command runs unconfined and its header says
`dispatch: unconfined (no sandbox on this platform)`.
`spikes/11-gate-sandbox/README.md` has the profile and what real
toolchains need: `setuid` programs such as `/bin/ps` cannot start
inside it, and creating a keychain fails, so a suite that does either
cannot be confined. Agents' panes and `serve` services are not
confined; they run in Switchboard.

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
  identity. The text as it was when taken is
  kept on the record, so what an operator was told is always known.
- **Lane.** One repository the ticket touches: a worktree, a branch,
  and the Switchboard project made for it. A lane with no worktree
  directory works in place on a branch and holds a repository lock
  (below) for as long as the ticket owns files there.
- **Root context.** Every ticket also has a Switchboard project at the
  pipeline's root, used before lanes exist and for joined stages. For
  a single-repository pipeline it is the repository; for Orchard it is
  the workspace directory. Agents run there before any branch is cut
  and are told to write nothing but their notes.
- **Pipeline.** Per project, a list of stages. A stage names an
  operator, a prompt, which context it runs in (the root, each lane,
  or all lanes joined), and a gate that says when it is done. When a
  ticket is taken, the pipeline file is copied whole into the ticket's
  directory, and the ticket runs from that copy until it finishes or is
  restarted (`dispatch restart`, below); editing the project's file
  changes only tickets taken afterwards and tickets restarted after the
  edit.
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
  decision, except a nudge, which is recorded before it is sent and is
  neither repeated nor asked about; see the control port.
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

A live session is observed, never resumed, never sent to. A session that
is gone is judged by the attempt's completion evidence (below), never by
the mere presence of a file. A check a previous runner left running is
stopped by its recorded process group before the gate runs again or its
attempt is cancelled (see "Runner children"). Resource holds are read
back from the records; nothing is released on restart. Switchboard's own
restart is covered by the same reads: every record the port made is an
ordinary Switchboard record that its reconcile treats like any other,
and launches nothing.

Closing a ticket is a sequence from a saved intent, as parking is.
`dispatch close` (or the port's `close`, or the page's Close) refuses
while an attempt is open (park first, so the cancellation sequence
runs) or a tree has changes, before anything is written; the preflight
reads each lane with a repository of its own on its own terms and the
ticket's tree with those lanes left out, since a clean nested lane is
untracked content in the outer tree. Then the ticket is written
`closing` and moved from the project's `queue` to its `closing` list.
`dispatch close` goes on to run the rest itself; the port's `close`
answers with the ticket `closing` right there, because the rest asks
Switchboard's control socket, and Switchboard's page is waiting on the
reply on the thread that answers it. Every pass runs the rest from the
flags on the record until it is done: unanswered requests resolved (a launch Switchboard is still
working on holds the close until it answers, and recovery parking the
ticket does not undo the intent to close), every process read back as gone,
pending decisions cancelled, the session unmarked once Switchboard
answers, each lane with a repository of its own removed from its clone
with `git worktree remove` and then the ticket's tree, never forced,
the card taken off the set by a `set.sync` on the closing ticket's own
ledger, and only then `closed`. The branch, the ticket's directory, the
record and its Switchboard projects stay. A later take of the same issue
deletes a kept branch that has nothing beyond its base, and asks
`reuse | fresh | park` about one that has commits (see Decisions);
`dispatch close` lists the branches it keeps. A ticket past its last stage
closes the same way; there a refusal by git does not hold the close,
it is recorded as the trees kept, and `close` on the closed ticket
tries the removal again. A closing ticket starts nothing, cuts no
tree, cannot be resumed, and counts against neither `slots` nor
`waiting_on_me`, on the page as in the scheduler: the runner reports
its pending decisions as `cancelling`, and recovery on a closing
ticket fails a lost launch without parking it or asking. A card left on the
set with nothing else to show (a close stopped before the project was
saved) is cleared on the next pass under that ticket's ledger.

Restarting a ticket is the way to give it a fixed pipeline. `dispatch
restart <ticket> [<stage>]` puts the ticket at its current stage, or at
an earlier one named, under a fresh copy of the project's live file
(`<project>.pr.toml` for a ticket from pull requests). It is checked
before anything is written, and refused for a closing or closed ticket
(that is a retake), for one still parking, for a live file that is
missing, does not parse, names another project, lacks the stage, adds a
stage before it that the ticket never ran, drops one of the ticket's
lanes, or changes a lane's `path`, `repo`, `base` or `remote` or the
project's `repo`, `base`, `remote` or `worktrees`, and for a stage after
the current one. Then the intent is saved on the ticket together with
`parking`, and the park sequence runs: open attempts cancelled, every
process read back as gone. Only then, on that call or a later pass, does
the restart apply; until it does, `dispatch restart` says the runner
finishes it. Stages are mapped by name into the new copy, which is
written beside the old ones as `pipeline.<n>.toml`; the first copy is
never rewritten. A stage named earlier than the current one discards
the later work: every completed attempt from that stage on is
cancelled as discarded, with its decisions and the stages' decisions
(an answered `lanes` question too, which chooses every lane again), and
each branch (the ticket's tree, and each lane with a repository of its
own) is reset with `git reset --keep` to the head it had as the ticket
last advanced into that stage, with the lane's base, bring-up record
and conflict put back as they were then. A ticket records those heads
on every advance (`entered`); one taken before that has none, and a
ranged restart of it is refused, as it is for a project that works in
place and for a ticket from pull requests. Each branch is checked (on
its branch, not mid-rebase, clean) before any moves, and each reset is
saved as it lands; a refused one parks the ticket with git's reason and
the intent kept, `resume` refuses while a restart is held, and
`dispatch restart` again carries it on. A restart at the current stage
discards that stage's completed attempts too, gate-only ones included,
and keeps the stage as brought up, so no bring-up runs. A lane whose
`setup` differs between the copies runs it again: before its next
agent, and before the checks a `check` answer starts, where a failing
setup fails the checks (with `check` offered again) rather than parking.
No agent of the stage launches until its `rerun` question is answered:
an attempt that failed at its checks, an agent attempt cancelled or
discarded with its checks started, and a code review cancelled mid-check
before its rewrite are offered `rerun | check | park`, so `check` runs
the new gate on the same head; any other attempt, a finished code
review included, is offered `rerun | park`. A gate-only stage opens its
watch or question again at once, and after a ranged restart the reset
branches are brought up on re-entry, as on any stage entry. Failures
under an earlier copy do not count against `max_reruns`.

Ticket and project records carry a `version`. Every read goes through
`store::read_ticket` and `store::read_project`, which refuse a record
from a newer `dispatch` and bring an older one up through
`store::migrate`; every write stamps the current version and refuses
to write over a newer one. Version 2 adds a ledger operation's
`settled` flag, which older records had only as recovery's verdict in
`error`; the migration reads it from those words. Version 3 adds a
lane's `pushed`, the head a refresh last pushed, which older records
never had and read as absent. Version 4 adds a code review attempt's
`carried_from` and `rework`, and a lane's `refreshed` gains `commits`,
`notes` and `at_ms`, all read as absent in older records; a send-back
note now lives on the attempt. Version 5 adds a code review attempt's
`rewrite`, absent in older records. Version 6 adds a review round's
`dirty_since_ms`, the time a pass first found the tree dirty after the
response settled, absent in older records; a round caught mid-wait
starts its clock on the first pass after the upgrade. Version 7 adds an
attempt's and a round's `nudges`, empty in older records. Version 8
adds a gate run's `group`, the check's process group as started, and
an attempt's `orphans_killed`, both absent in older records. Version 9 adds
a lane's `conflict`, a rebase that stopped and has not been brought up,
and the `conflict` and `after` of a lane's `refreshed`, the conflict a
bring-up resolved and the head it reached; all are absent in older
records, so a bring-up recorded before the upgrade gets no resolution
review.
Version 10 adds a rewrite's `stale` and `message`, empty and absent in
older records.
Version 11 adds a conflict's `at_ms`, 0 in older records. Version 12
adds a ticket's `restarts`, `restart` and `entered`, empty and absent in
older records. Version 13 adds a project's `supervisor`, a decision's
`refusals`, a ticket's `state_by` and its source's `taken_by`, all
empty or absent in older records.
Version 14 adds a ticket's `holds` and `services`, empty in older
records.

A project's record (`projects/<project>.json`) holds, besides its
space, set and queue, its supervisor (see "Supervisor"): `supervisor`
is `{project, workspace, current, past, intent, op, error}`. `current`
is `{session, seed_hash, created_ms, model}`, where `session` is
Switchboard's record id, never a resume handle; `past` keeps every
session it replaced, each with `replaced_ms` and why (`fresh` or
`kill: <reason>`); `intent` is a fresh asked for through the port and
not yet carried out; `op` is the one request to Switchboard in flight
for it, written before it is sent and its reply after, as a ticket's
ledger is, and recovered at start the same way. A ticket's `state_by`
says who made its last park, resume or close and its source's
`taken_by` who took it: `supervisor`, or absent for the owner and for
Dispatch itself.

### The event log and the runner's status

Two files in the data directory are logs and status, not records, and
neither goes through `migrate`.

`events.jsonl` is the event log the supervising commands read
(`dispatch events`, `dispatch wait`). Each line is one JSON object,
`{"v":1,"seq":N,"at_ms":…,"ticket":…,"project":…,"stage":…,"kind":…,"text":…}`,
with `attempt`, `decision`, `head`, `url`, `voids`, `by`, `conflicts`
and `actor` when they apply. `actor` is `supervisor` on a `taken`,
`parking`, `resumed`, `closing`, `answered` or `refused` a supervisor
session caused, and absent for the owner; the text of the first four
ends `(by supervisor)`. `refused` is a supervisor's answer to a
decision outside its `decides`, saved on the decision, which still
waits. `by` and `conflicts` are only on
`refreshed`: `conflicts` is 0 for `git`, and is absent for a `rebaser`,
`hand` or `stopped` bring-up with no conflict recorded.
There is one writer, `store::write_ticket_logged`, at the one place a
ticket is written (`Runner::save_ticket`, `decide` and `resume`): it
reads the record being replaced, diffs it against the new one
(`events::between`), appends what changed and syncs it, and only then
writes the record. Ledger entries, settle counts, polls and
`updated_ms` make no event, so the runner's several saves per pass
stay quiet. The `seq` is global, taken from the file's last complete
line under the writer lock; a torn last line is closed with a newline
and skipped by readers.

The order with the record's rename makes a crash between the two a
duplicate, never a miss: the event is on disk, the record is not, and
the next pass does the transition again and logs it again. A write
that fails without a crash (a full disk, a permission change) appends
a `void` naming the seqs it had appended, and readers drop those. If
the `void` cannot be appended either, the record's error is still the
one returned and the withdrawn events stand; `wait` confirms every
match against the record, so it never returns one. A match whose
write may still be in flight, appended but not yet renamed in, is
held until the record bears it out, passes its write, or its `void`
is read. An append that
fails refuses the record's write, since the transition would
otherwise never be logged. Text is built from record fields only (ids,
heads, stage names, a decision's question and options, a failure's
reason), capped at 512 bytes. The log is never rotated, and tickets
written before it existed have events from their next transition on.

`runner.json` is `dispatch run`'s status, replaced after every pass:
its pid, start, last pass and how long it took, the last success and
latency of a call to Switchboard, the failures of calls to Switchboard
and the PR providers in the last hour (time, ticket, the error's
text), and per ticket the last time the runner got on with it (a call
for it that succeeded, or a step that finished without an error).
`dispatch health` reads it, never taking `runner.lock`, and folds the
failures that repeat for one ticket with one error into one line with a
count (`--verbose` lists each).

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
  written on the attempt when seen. A Stop while the card reads
  `working` is a turn ended, not the work (background agents or a
  tool still at it), so the attempt is held, neither failed nor
  completed, whatever the artifact says. A stop with no file fails
  once the pane has sat idle at its prompt for about thirty seconds
  in a row (`STOP_IDLE_POLLS`; any other card starts the count again)
  or the pane exits. A card that reads idle is not a stop on its own:
  for a Codex session Switchboard shows a quiet pane as idle after
  twenty seconds, which can be an agent thinking. For Claude Code,
  `idle` follows a Stop, and only after one does it count toward the
  grace. A settled file with a running session waits. A stop with no
  file fails only after that grace; a nonzero exit, or a session gone
  with no recorded stop, fails the attempt at once. Recovery applies
  the same rule: a missing session is finished only if the stop and
  the settle were recorded before the crash.
- **Stalls.** Switchboard's stall notice is for workflow runs only.
  For agent attempts the port's `session` query reports how long the
  pane has been quiet, and Dispatch raises a decision at its own
  stall threshold. That read model is new work on the port.
- A *gate-only* stage (`lanes`, `deploy`, `ready`, `merge`) has no
  session; its attempt is the gate's own result, and its prompt
  fields (`{inputs.deploy.commit}`) are what the gate recorded. A
  skipped stage records nothing, and a template naming its field
  renders as `unknown (deploy skipped)`; the Orchard `try` prompt says
  so in words.
- **Command gates run once, on a clean tree.** After the agent stops,
  Dispatch requires the context's tree clean, records its head commit,
  and asks Switchboard to run the gate as a command record named for
  the attempt. The exit code is read back; then head and cleanliness
  are checked again, and the result is bound to that head only if
  both are unchanged. A dirty tree before or after is a decision, not
  a result. A dirty tree after the agent is first a nudge: one line
  typed into the same session, telling the agent to commit or revert
  and stop, up to the stage's `on_dirty`; only a later stop that is
  still dirty, or a nudge never answered within the idle grace, is the
  decision, which says how many nudges came before it. A gate is never
  run on a polling tick, and never before the agent has stopped, so it
  cannot pass on the untouched base.
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
  and a replacement attempt never run together. For Orchard a push after
  `tried` lists `implement`'s checks, `deploy`, `try` and `tried`;
  there is no path from a moved head to `merge` on old evidence.
- Failure is a decision, never a retry. A gate that fails, an agent
  that stalls (Switchboard already marks it), a review at its cap, a
  branch that no longer merges, a lost launch: each becomes a decision
  with the suggested next step. Nothing loops silently. A nudge is not
  a retry: it is bounded by `on_dirty`, recorded on the attempt,
  logged as a `nudged` event, and spends no attempt and no
  `max_reruns`.
- Prompts are text and take template values verbatim. Commands never
  do: a command is a fixed argv from the pipeline file, and template
  values reach it as environment variables (`DISPATCH_TICKET`,
  `DISPATCH_LANE`, `DISPATCH_BRANCH`, and so on),
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

Rejecting a decision cancels the ticket's current attempt, and so does
`dispatch park <ticket> [--reason <text>]`, which writes the same intent
by command rather than by an answer (the reason defaults to "parked by
hand"; a closing or closed ticket is a usage error, exit 64) and leaves
the rest to the runner's next pass, since the checks are its children.
Cancelling is a sequence, not a flag: write the intent on the record,
with every open decision on the ticket cancelled in the same write:
a pending one, an answer not yet acted on (a `rerun` still waiting for
a slot), an acted `rerun` whose replacement has not launched, and a
gate's send-back note no attempt has carried yet (the resume asks
`rerun` about that context, quoting the note, and answered `rerun`
the note goes back into the next attempt's prompt);
clear the session's waiting mark and read it back; pause any review
run (`workflow.pause`) so its tick cannot start a round; kill
everything on the ticket's process list that is still alive, including
a service started for an earlier stage; read each back until
Switchboard reports it gone; for an in-place lane, confirm the tree is
clean; and only then release the holds and move the ticket to
`parked`, where you can requeue or close it. Releasing a hold at the
end of a stage runs the same check over the ticket's whole process
list, not the current attempt's. If any writer cannot be confirmed
gone or the tree cannot be made clean, the holds stay and a decision
says why.

A cancelled decision cannot be answered, and a parked ticket counts
nothing against `waiting_on_me`. A `parking` record whose decisions
are still pending has them cancelled on the next pass; a `parked` one
keeps them until it is resumed or closed. The resume is itself the
answer for each context's latest attempt that the park it ends
cancelled mid-run: its reason is the park's (or the park's with what
its checks did past the limit), its end is logged after the park's
`parking` event (an earlier park with the same reason is asked about,
as is everything when the log has no such event), its
stage is an agent, code review or workflow stage, or a resolution
review the resolution loop would still launch, and no question about
it was withdrawn or answered `park` by that park. Each is recorded, in
the resume's own write, as a `rerun` decision answered by `resume`,
which the runner acts on as it does a hand answer, waiting for a slot.
A cancelled rebaser is not one: the next pass reads its lane again.
About every other failed or cancelled latest attempt, including one
that was waiting on a question when the park came, the resumed ticket
asks again under a new decision id, quoting the attempt's own reason
and offering what the failure first offered (`check` too after failed
checks). Asking launches nothing, so it does not wait for a slot.
`dispatch resume <ticket> --no-rerun` answers nothing and asks about
all of them. The `resumed` event names the reruns and comes before
their decision events.

A ticket that takes an issue again after a close finds the closed
ticket's branches in Dispatch's clones under its own name, since a close
keeps them. Before anything is cut, every clone is fetched and looked
at: a kept branch with nothing beyond its base (`<remote>/<base>`) is
only a name, and is deleted; one with commits is asked about once for
the whole cut, as the `branch` decision under the `cut` pseudo-stage,
naming each repository whose branch has commits and how many. `reuse`
checks the branch out at its head and goes on from that work (a lane's
recorded base is where the branch forked, so a code review diffs only
its own commits); `fresh` renames it to
`<branch>.closed-<yyyymmdd>` (`-2`, `-3`, … until the name is free in
every clone with commits, so each is renamed to the one name the
question gives) and cuts a new one; `park` parks the ticket and leaves
the branch with commits as it is. A branch the cut cannot read or
delete parks the ticket with git's error. The `decisions` dial never
answers this question, since both `reuse` and `fresh` change git
state. While it is pending the ticket stays active and held at the
cut, counted as waiting on you, rather than parked, so that the answer
is acted on; nothing is fetched or launched until then. An answer is
spent by the cut that uses it: a `reuse` or `fresh` whose cut fails
(git refuses a branch still checked out in a closed ticket's kept
tree) is withdrawn with the parking, and a resume asks again under a
new id, as it does after `park`.
A directory already at a tree's place is adopted as before and not
looked at, and a pull request's branch is reset by `-B` as before.

A pending decision holds its stage in the context it is about: one
about a lane's attempt holds that lane only, so another lane's first
launch or authorised rerun goes ahead, and one about no attempt holds
every context.

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
what the user said; other contexts keep their results. A `rerun`
answer to an agent stage's `rerun` question carries its own note the
same way, and without one, the note that sent the attempt back. A
gate with `confirm = true` is "you did this": its answers are `done`
and `park`, and it is never answered automatically.
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
remotes = { github = "git@github.com:..." }   # mirrors a pull request may be taken from, by remote name

[source]
kind = "github" | "task-file" | "manual" | "pull-request"   # pull-request: see "Tickets from pull requests"

[[lanes]]
name = "..."
path = "relative/to/tree"     # "." for a single-repo project
repo = "git@..."              # a repository of its own (a workspace of several): cloned by
                              # Dispatch too, cut as a worktree at path inside the ticket's tree
base = "main"                 # omitted: the project's
remotes = { github = "git@github.com:..." }   # this lane's mirrors, as the project's
setup = ["cmd", "args"]       # run once, before the lane's first agent
writable = ["~/.cargo"]       # under [policy] confine: extra paths the lane's setup, gates, checks and command reviewers may write

[[resources]]
name = "..."
count = 1

[operators.<name>]
kind = "claude" | "codex" | "command"
guidance = "..."
budget_usd = 0.0              # per ticket across the operator's attempts; 0 means the project default
argv = ["cmd", "args"]        # kind = "command" only: local tooling a code review stage runs as a reviewer
in = "lane"                   # or "root"; where the command runs

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
prompt = "..."                # templates: {issue.number} {issue.title} {task.text} {lane} {lanes} {lanes.all} {branch} {worktree} {project.root} {inputs.<artifact>}; the full list is the table below
gate = { kind = "command", argv = ["..."], in = "root" | "lane" | "lane:<name>" }
     | { kind = "command", per_lane = { <lane> = ["..."] }, in = "lane" }
     | { kind = "command", like = "implement" }   # an earlier stage's command gate by reference; its result at the same clean head is reused
     | { kind = "command", argv = ["..."], network = "allow" | "deny" }   # under confine: this gate's network over the policy's; a like gate takes the named gate's
     | { kind = "external", check = "review-finalized" | "pr-checks" | "pr-merged" }
     | { kind = "external", check = "pr-checks", checks = "none" }   # a repository with no CI: a PR at the head is enough
     | { kind = "human", decision = "...", confirm = true }
on_dirty = { nudge = 1 } | "ask"   # an agent stage with a command gate, or a code review stage's implementer: overrides the policy's
needs = ["resource name"]     # held from the first stage that names it to the last, contiguous
reviewers = ["style", "lint"] # present: a code review stage (see "The code review stage"); operators, run at once each round
implementer = "implementer"   # the claude operator that addresses a round's findings, fresh each round
cap = 3                       # review passes before the findings left are a question
review_prompt = "..."         # optional templates; see the section for the defaults and their variables
fix_prompt = "..."
no_feedback = "No findings."
style_rounds = 2              # from this round, a round with only style points converges (live)
commits = "keep"              # "fold": fix rounds fold into the commits they amend; "one": one commit; at completion, tree unchanged

[policy]
slots = 1                     # tickets with a running attempt or a held resource; read live from <project>.toml on every pass, not from a ticket's copy
waiting_on_me = 2             # pending decisions across the project before nothing new starts; live as well
rates = { "claude-sonnet-5" = [3.0, 15.0], ... }   # $ per million input, output tokens
decisions = { lanes = "ask", finalize = "ask", merge = "ask", budget = "ask", review-code = "ask", resolution = "ask" }
trust_folders = false         # true: Claude Code's folder trust question, which every fresh worktree asks, is answered for the project's agents
max_reruns = 3                # failed attempts a stage may collect in one context before the ticket parks instead of asking again
on_dirty = { nudge = 1 } | "ask"   # an agent that stops with a dirty tree is nudged in its session up to N times, each after a stop, before the question; "ask" asks at once. Read from the ticket's copy; a copy without the key, including one taken before the key existed, nudges once
min_free_gb = 10              # free space on the worktrees' volume below which nothing new starts; live, like slots
refresh = true                # each lane's branch is brought up to its base when a stage begins; a conflict goes to the rebaser
rebaser = "rebaser"           # the operator that rebases a PR that conflicts with its base, cloned from the lane's implementer; absent, a conflict is a question
max_rebases = 2               # rebases one PR may get before the conflict is a question
resolution_reviewer = "correctness"  # the one reviewer of a conflict's resolution brought up after the last code review stage; absent, that stage's first reviewer that is not `style`
fixer = "fixer"               # the operator that fixes a PR whose checks are red at the tree's head, cloned the same way; absent, red checks are a question
max_fixes = 2                 # fixes one PR may get before red checks are a question
confine = false               # true: setup, command gates, review checks and command reviewers run sandboxed (see the start of this document); absent, off
network = "allow"             # or "deny": under confine, whether those commands reach off this machine; loopback stays open
```

An agent stage needs no `gate` line: "the agent stopped and every
artifact it writes settled" is the default, described under "Stage
semantics". A workflow stage names the artifact it reviews with
`subject`.

A prompt's templates are these keys, plus a stage's `writes` names
(`{notes}`, `{plan}`); the review and code review variables are
described where they are used. A key that is not set reaches the agent
as written.

| Template | Renders |
|---|---|
| `{ticket}` | the ticket id |
| `{issue.number}`, `{issue.title}`, `{issue.body}`, `{issue.url}` | the source issue |
| `{task.text}`, `{task.context}` | the source as a task (title, body) |
| `{project.root}` | the ticket's tree, or the project root for a pipeline that works in place |
| `{worktree}`, `{branch}`, `{lane}` | the context's tree, branch and lane (`{branch}` only where a branch is cut: a lane, or the root of a pipeline with a `repo`; `{lane}` only in a lane context) |
| `{lanes}` | the lanes the ticket chose, comma-separated in pipeline order; every lane before the `lanes` decision, so `investigate` sees them all |
| `{lanes.all}` | every lane of the pipeline, comma-separated in pipeline order |
| `{inputs.<artifact>}` | the path of an earlier attempt's artifact |

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
writable = ["~/.cargo", "/tmp"]   # cargo's registry; the tests' tmux sockets and short paths under /tmp

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
confine = false              # its tests start /bin/ps (setuid) and make a keychain, which the sandbox refuses
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

### Pipeline: Orchard

The hard case, and the one the vocabulary was shaped for. Facts from
`~/code_repos/orchard` (its `CLAUDE.md`, `guides/ENVIRONMENTS_GUIDE.md`,
`guides/COLLABORATION_GUIDE.md`, the backend `tasks/`):

- The root is a planning workspace (`example-org/orchard-workspace`), and
  the issues live there with a label scheme (`type:`, `area:`,
  `priority:`, `status:`). The application repositories are separate
  git repositories nested under it: `orchard-backend` (integration branch
  `main`), `orchard-frontend` (`dev`; `master` is production),
  `orchard-admin` (`master`). Bitbucket is the origin; GitHub mirrors
  exist. PRs go through `tools/bb.py`, which squashes on merge.
- "Ready for the test environment" pre-merge means `my-dev`: one
  personal backend stack, deployed from any branch by
  `aws-vault exec -n orchard-dev -- uv run inv deploy -f` from
  `orchard-backend` with that env linked. Last deploy wins. The link is
  a per-checkout cache file, so a fresh worktree is unlinked until
  `inv link-env --env-name my-dev` runs in it. The frontends have no
  per-branch environment; a branch is tried against `my-dev` with
  `npm run link-env` and a local `npm start`. Shared `dev` deploys
  itself on merge to `main` and is post-merge, so it is not a gate.
- Machine checks: backend `uv run inv lint` and `uv run inv pytest`;
  frontend `npm ci --legacy-peer-deps` then
  `CI=true npm test -- --watchAll=false`; admin `npm test`. Bitbucket
  Pipelines runs tests on PRs but has no lint gate, and the GitHub
  mirrors have no CI, so `pr-checks` here reads Bitbucket through
  `bb.py` rather than `gh`.
- A PR is ready only with a self-contained body, an updated backend
  `CHANGELOG.md` "Unreleased" entry, a What's New entry for every
  user-visible frontend change, tests listed in the body, and no
  session-link attribution of any kind.
- Nothing in the repos locks an environment. Two tickets deploying to
  `my-dev` would overwrite each other, which is exactly what a
  resource with `count = 1` prevents.

```toml
version = 1

[project]
name = "Orchard"
repo = "git@github.com:example-org/orchard-workspace.git"
space = "Dispatch · Orchard"

[source]
kind = "github"
repo = "example-org/orchard-workspace"
label = "dispatch"
# area: labels are the suggested answer to the lanes decision.
lane_hints = { "area:backend" = "backend", "area:frontend" = "frontend", "area:admin" = "admin" }

[[lanes]]
name = "backend"
path = "orchard-backend"
repo = "git@bitbucket.org:example-co/orchard-backend.git"
base = "main"
setup = ["sh", "-c", "uv sync && uv run inv link-env --env-name my-dev"]
writable = ["~/.cache/uv"]    # measured: uv's cache; link-env writes only inside the tree

[[lanes]]
name = "frontend"
path = "orchard-frontend"
repo = "git@bitbucket.org:example-co/orchard-frontend.git"
base = "dev"
setup = ["npm", "ci", "--legacy-peer-deps"]
writable = ["~/.npm"]         # npm's cache; not yet measured under the sandbox
# A service Dispatch starts for a stage that asks. PORT is a port
# Dispatch allocates per ticket; the URL is what the tester is told.
serve = { argv = ["npm", "start"], env = { BROWSER = "none", PORT = "{port}" }, url = "http://localhost:{port}", ready = { http = "/", within_secs = 120 } }

[[lanes]]
name = "admin"
path = "orchard-admin"
repo = "git@bitbucket.org:example-co/orchard-admin.git"
base = "master"
setup = ["npm", "ci", "--legacy-peer-deps"]
writable = ["~/.npm"]
serve = { argv = ["npm", "start"], env = { BROWSER = "none", PORT = "{port}" }, url = "http://localhost:{port}", ready = { http = "/", within_secs = 120 } }

[[resources]]
name = "my-dev"
count = 1

[operators.investigator]
kind = "claude"
guidance = "Read the root CLAUDE.md, guides/ENVIRONMENTS_GUIDE.md and each repository's CLAUDE.md. Name the files, the endpoints and the screens the issue touches. Say which lanes it needs and why; that is a decision the human sees. Write nothing but your notes."

[operators.planner]
kind = "claude"
guidance = "Write the plan as plans/YYYYMMDD-name.md is written: per lane, the change, the tests, the CHANGELOG or What's New entry, and how it is tried on my-dev."

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
guidance = "The backend branch is already deployed to my-dev; the commit is in your prompt. Do not deploy. The frontend worktree is linked to my-dev and already being served at the address in your prompt; do not start another. Exercise the change with the tools/e2e probes or curl. Write what worked and what did not, with the commands and their output, to {notes}."

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
gate = { kind = "command", in = "lane", per_lane = { backend = ["sh", "-c", "uv run inv lint && uv run inv pytest"], frontend = ["sh", "-c", "CI=true npm test -- --watchAll=false"], admin = ["sh", "-c", "CI=true npm test"] } }

[[stages]]
name = "deploy"
context = "lane:backend"      # skipped when the ticket has no backend lane: the frontend is then tried against what my-dev already has
needs = ["my-dev"]
# Dispatch deploys, once, after linking again so the target cannot be
# whatever a previous checkout left; the deployed commit is recorded
# on the attempt. It needs AWS, so under confine with the network
# denied it keeps its own.
gate = { kind = "command", in = "lane:backend", network = "allow", argv = ["sh", "-c", "uv run inv link-env --env-name my-dev && aws-vault exec -n orchard-dev -- uv run inv deploy -f"] }

[[stages]]
name = "try"
operator = "tester"
context = "joined"
needs = ["my-dev"]
services = ["frontend", "admin"]   # each started only if its lane was cut; owned by the ticket until `tried` ends
before = { frontend = ["npm", "run", "link-env"], admin = ["npm", "run", "link-env"] }
writes = ["notes"]
prompt = "my-dev is running backend commit {inputs.deploy.commit} (when that reads as skipped, this ticket has no backend lane and my-dev runs whatever was deployed last). The admin frontend: {services.frontend}. The student portal: {services.admin}. A lane this ticket did not cut is not served; test it, if at all, against the existing deployment. Try ticket #{issue.number} end to end and report to {notes}."

[[stages]]
name = "tried"
needs = ["my-dev"]        # still held: what you are looking at must stay deployed
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
slots = 2                    # two tickets in flight; only one can hold my-dev
waiting_on_me = 2
ports = [3100, 3199]         # for services; one per service per ticket, tested free before use
rates = { "claude-sonnet-5" = [3.0, 15.0], "claude-opus-5-5" = [15.0, 75.0], "gpt-5-codex" = [1.25, 10.0] }
decisions = { lanes = "ask", finalize = "ask", budget = "ask" }
confine = false              # on once its gates have passed confined; the setup has (spikes/11-gate-sandbox)
```

What this pipeline showed, and what it added to the vocabulary:

- **Lanes with a `base` and a `setup`.** Each repository has its own
  integration branch and install step. The backend's setup also links
  the worktree to `my-dev`, and the deploy gate links again right
  before deploying, so the target is established twice and never
  inherited from another checkout.
- **`joined` stages.** Plan and review happen once per ticket across
  lanes; implementation happens per lane. That distinction did not
  exist until a multi-repo project needed it.
- **`lane:backend` as a context**, and a stage that is skipped when
  its lane was not cut. A frontend-only ticket is tried against
  whatever backend `my-dev` already runs, and the `tried` decision
  says so.
- **Services are Dispatch's.** A stage's `services` name lanes; each
  is started only if the ticket cut that lane, and a template field
  for one that was not renders as `not served (no <lane> lane)`, so a
  backend-only, frontend-only or portal-only ticket names nothing
  that does not exist. For each lane that was cut, in order: the
  `before` command runs as a child of the runner, with its log in the
  ticket directory, and must exit zero; Dispatch allocates a port from
  the policy's `ports` range, testing that it binds before choosing
  it, so a server you started by hand on the default port is simply
  not chosen; the service is a Switchboard service session made with
  `session.new`, launched through the login shell as `env
  <serve.env> <serve.argv>` with `{port}` filled in, so `serve.env`
  must hold no secrets (the argv is kept on the record and the pane's
  command line); and readiness is the `ready` probe answering on that
  URL within its limit. `service.new` and `command.run` stay unbuilt. A `before` failure, no free port, or a probe that never
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
- **A count is not an environment.** Raising `my-dev` to 2 would
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

## The code review stage

A stage with `reviewers` reviews the branch a lane's `implement`
produced, with several reviewers at once, and has a fresh implementer
address what they say, in rounds, until no point is open or the cap
is reached. It sits after `implement` and before `inspect`. It is a
fourth stage kind (`StageKind::Review`, `AttemptKind::Review`), not
the plan review's workflow: the subject is a branch, there are many
reviewers, and the implementer is a fresh agent per round.

**Reviewers** are operators. A Claude Code reviewer runs in the lane's
tree with its reviewer directory as its one allowed write target; a
Codex reviewer runs in that directory and is told the tree; a
`kind = "command"` operator is local tooling run as a child of the
runner in the lane (or `in = "root"`), with its stdout as its
feedback. Reviewers are read-only by detection: the round records the
head and cleanliness at its start and end, and any change voids it.

**The base** is the commit the lane was cut from, resolved once at the
cut into `LaneRecord.base_sha` (a lane cut before that existed gets
it from the merge base on the stage's first pass, and keeps it). It is
never resolved again: the shared clone's remote refs move whenever
another ticket fetches. Each round records `base` and `head` before
any reviewer starts, and the reviewers are pointed at exactly that
range.

**One round.** The tree must be clean at a head that is where the
previous round left it (its `head_after`, or its `head` when nothing
was fixed); any other movement parks the ticket with both heads named.
Every reviewer starts. A Claude reviewer is complete on its Stop with
its feedback file settled, and is held like any agent while its card
still reads `working` after the Stop: it fails for a missing file only
after the same idle grace, so a sibling is never killed early; Codex
on the file present and settled (it reports no Stop; gone or exited
without the file is a failure); a command on exit: 0 is nothing to
report (its output is diagnostic), 1 with output is findings, 1 with
nothing on stdout or any other exit is a failure. A finished agent's
session is killed. A failed reviewer fails the round into the ordinary
`rerun` question, its siblings retired first. When every reviewer is
done and the tree is still clean at the recorded head, the findings
are gathered into `feedback.md` under the round: one
`- <id> (<reviewer>): <text>` line per point,
ids `r<round>/<reviewer>-<n>`, then the points earlier rounds left
open that no reviewer withdrew. A reviewer's file that is exactly the
sentinel (`No findings.`, or the stage's `no_feedback`) contributes
nothing; otherwise each `- `, `* ` or `1. ` line is a point, and a
file with no list is one point. A reviewer's line saying every point it
has left is wording ("Every point left is wording; the round can close
on it.") is a note, not a point: it takes no id and is listed under
`## Reviewer notes`. The declaration must end at "wording" (or
"wording is left") with no "not", "except" or "but" in it, and only a
closing remark such as "the round can close on it", "the round can
close" or "it can close" may follow it; anything else, or a line
quoting code, makes the line a point again. Carried points: the
previous round's points the implementer marked `disputed` (or did not
answer) stay open under their original id unless a reviewer in the new
pass wrote `withdraw <id>` (bare or as a list item); `keep <id>: why`
keeps one with the reason shown.

Points are tagged by their text. A point that starts with `style:`
(case-insensitive) is about wording, naming, comments or layout; so is
any untagged point from the reviewer named `style`, so convergence does
not depend on that reviewer remembering the prefix. A point that starts
with `decided:` contests the plan's decisions: it never counts as open
and is listed in the round file under `## Found but not done`, out of
scope for the review and for the fix pass. Every other point holds the
round open. From round `style_rounds` (default 2, read from the
project's live pipeline file each round, the `.pr.toml` for a ticket
taken from a pull request), a round whose open points are all style
converges at the head the reviewers read: no fix pass runs that no
reviewer would see, and the points go under `## Left to the merge`.
Before that round, style points reach the fixer like any other. A
round file's sections are `## Points`, `## Still open from earlier
rounds`, `## Left to the merge`, `## Found but not done` and
`## Reviewer notes`, each written only when it lists something; a round
with none open but some left or found says "No open point." first, and
one with nothing but notes is the sentinel followed by its notes. Only
the first two carry into the next round.

**A new attempt** of the stage in the same context (a `rerun`, or a
later gate's send-back) carries the latest earlier attempt whose rounds
gathered findings (`Attempt.carried_from`; an attempt that failed
before gathering any passes on what it carried). Its round 1 reviewers
are told the old attempt's last gathered round: the head it read, the
points settled there (not to be raised again) and the points still
open, and they read the change from that head to the new one (`git
diff`, or `git range-diff` when the base moved). A point the old
attempt's last fix pass answered `fixed` is still open, marked for its
fix to be checked, since no round read that fix and the attempt may
have failed on it. The open points are open coming into round 1
under ids qualified with the attempt they were raised in,
`a<n>/r<round>/<reviewer>-<k>`, since ids are unique only within an
attempt; an id carried twice keeps its first qualifier.
Rounds are numbered from 1 again, but for `style_rounds` a carried
attempt's round k counts as round N + k, N being the last round the old
attempt read; the cap is per attempt. A note that contains `start over`
(case-insensitive) gives a plain attempt that reviews the whole branch,
and the rerun question says so. A send-back note leaves `t.rework` when
the attempt starts and is kept on it (`Attempt.rework`) for its first
fix pass; an attempt that failed before any fix pass passes it on to
the very next attempt, and only to that one.

No open point converges the round: the stage's checks run at that
head (see below) and the stage completes bound to it. Open points at
a pass under the cap go by the `review-code` dial: `auto` starts the
fix pass; `ask` is a `review-code` question with `fix`, `accept`
(the head as it is, checks next) and `park`. At the cap (`cap`,
default 3, counting review passes) the question is `review-cap`:
`accept`, `more` (exactly one fix pass, the checks, then one more
review pass, after which the cap question is asked again if points
remain) or `park`; it has no dial. A head no reviewer has seen is
never offered. An answer whose head no longer matches the round's is
stale: the ticket parks with both heads named.

**The fix pass** is a fresh implementer session in the lane with the
plan, the branch and `feedback.md`, asked to address each point on the
branch and write `response.md` answering each by id (`fixed` or
`disputed`). It is complete on its Stop with the response settled and
the tree clean and committed; it is held by a busy card after its Stop
the same way, and a response still missing after the idle grace fails
the round. A
tree still dirty when the response settles is waited on for five
minutes from the first dirty pass while the session lives
(`DIRTY_WAIT_MS`, measured from the round's `dirty_since_ms`, so a
restart keeps the clock), because a commit
whose pre-commit hook runs the test suite lands that late; dirty past
that, the implementer is nudged in its session (up to the stage's
`on_dirty`), and its next stop gets the wait afresh; dirty past the
wait with the nudges spent or the last one unanswered, or with the
session gone, the round fails. Its head is
`head_after`, an authorised transition. Then
the stage's checks run at that head, and the next round opens there.

**The checks** are the stage's own command gate, required. `like =
"implement"` names an earlier stage's command gate by reference: an
accepted head whose checks that stage already ran (same command, same
head, exit 0, the tree clean) is not checked twice; any other head,
and any other gate, runs. A failing check is the ordinary checks
question (`rerun`, `check`, `park`); `check` runs them again on the
same head. The rewrite after the checks pass has its own question,
`rerun | keep | park` (see "Clean commits"). A check lost to a runner
restart starts again on the same clean head, the one child that does,
once the orphan's group is gone or killed.

**Clean commits.** `commits` says what the stage does to the branch's
history once the checks pass at the head it completes at: `keep` (the
default) leaves the commits as the implementer and the fixers made
them; `fold` folds each fix round's commits into the commits they
amend; `one` makes the whole branch one commit, with the message and
author of the first commit no fix round made. A `fixup!` or `squash!`
commit folds into its target the way git's autosquash finds it (the
subject, then a sha prefix, then a subject prefix; a `squash!` keeps
its body). Any other commit a fix round made, from that round's
reviewed head to its `head_after`, folds into the tip of the folded
history at the head that round reviewed. Every review attempt of the
stage in the context counts, so a carried rerun folds the earlier
attempt's fixes too; a round whose heads a refresh has since rebased
away drops out and its commits stay as they are. A merge among the
commits fails the attempt. The fixer convention that makes this
useful: commit with `git commit --fixup=<sha>` naming the
implementation commit the fix amends when that is clear, and a plain
commit otherwise.

The rewrite happens in the attempt's completion, never as a new launch
and never with a push. The tree must be clean; the new commits are
built with `git merge-tree` and `git commit-tree` (no hook runs), and
their tree must equal the tree the checks passed at before the branch
moves; the move is a compare-and-swap (`git update-ref HEAD <new>
<old>`); after it the head, a clean tree and the same tree are checked
again, and the branch is moved back if any fails. The checks are not
run again: the tree is identical. The rewrite's own failures (no plan,
such as a merge among the commits; a replay conflict; a different
tree; a refused move; a move undone because the tree read wrong after
it) fail the attempt with `rerun | keep | park`, the branch at the
reviewed head. `rerun` reviews the branch again and folds it the same
way, which usually fails the same way. `keep` completes the stage at
the reviewed head with the history as it is, recorded as `skipped =
"the user kept them after the rewrite failed"`, and runs nothing: it
parks if the branch has moved from the reviewed head or if the checks
did not pass there (the attempt's own, or the ones reused from the
`like` stage), and fails the attempt again with `rerun | keep | park`
if the tree is not clean, so `keep` works once it is cleaned. A move
undone because the move left the tree dirty usually leaves it dirty,
so clean the tree before answering `keep`. The attempt keeps
`failed_at_rewrite`, so a park and a resume ask `rerun | keep | park`
again. Any other failure of the completion fails the attempt with
`rerun | park`. The intent (`rewrite`
with no `after`) is saved before git writes anything: a restart that
finds the branch still at `before` rewrites again, one at another head
with the same clean tree completes there, and anything else fails. The
old commits stay reachable through the branch's reflog
(`gc.reflogExpireUnreachable`, 30 days by default). A tree dirty when
the checks start or finish keeps the checks question (`rerun | check |
park`): the checks did not run on what is in the tree, so running them
again is a fair answer, and the rewrite never starts. Only a tree the
rewrite itself finds dirty (after a restart) fails with `rerun |
park`.

A branch the remote already holds is not rewritten: the rewrite is
recorded with `skipped = "the branch is published"` and the history
left alone, since folding it would make the next ordinary push a
non-fast-forward. Published means the lane's `pushed` is set, an
attempt in the context has a pull request, or the remote-tracking ref
of the checked-out branch (`refs/remotes/<remote>/<branch>`, read
locally, no fetch) holds a commit of `base..head`. The ref is the
signal that matters: an agent's `git push` records nothing on the
ticket. A ref holding only commits of the base or older does not
count. A pull-request pipeline takes only `keep`.

**Folded messages.** A fold keeps the message of the commit a fix
folds into, so a review that renamed `old_name` to `new_name` can leave
a folded commit saying it adds `old_name`. Once a `fold` or `one`
rewrite has moved the branch, each folded commit that absorbed a fix
(a group of more than one commit) has its message checked: the names
it wraps in single backticks (outside fenced blocks; a trailing `()`
and a `:<line>` location dropped; only spans of `[A-Za-z0-9_-./:]` at
least three long that are not all digits or a sha) must each occur,
as a whole word, in the folded commit's diff (added or removed lines,
file headers), anywhere in the tree at the folded head, or, for a
path-like name, as a tree path or a path's suffix. A name with `::`,
or a `.` that is not path-like, also counts when its last segment
occurs. Prose is never read, and only names the check is sure are
missing are reported. The check backs up the fixers' `squash!`
convention and does not replace it: a name that a `squash!` commit's
body in the same group also names has been addressed and is not
asked about, and the squash bodies' own names are not checked. A
published branch, an identity fold and `keep` check nothing. A read
error is logged and the attempt completes as before: the check is a
backstop.

A stale message holds the attempt open with the branch at the folded
head, before the `pr` stage opens anything, and asks `message`:
`rewrite | accept | park`, naming each stale commit by subject and
`<head>~k` with the missing names. `accept` completes the stage at the
folded head with the messages as written. `rewrite` starts the stage's
`implementer` operator, with `message/input.md` in the attempt
directory listing each stale commit, its message, the missing names,
the rounds' `r<n>/response` files and the plan, and asking for each
new message in `message/<sha8>.txt` (the folded commit's first eight
characters); a Claude rewriter may write that directory. The agent
writes files only: it does not commit or amend. Once it stops with
every file settled, its session is killed and Dispatch replays the
folded commits one for one with the new messages, through the same
replay, tree-equality check and compare-and-swap move the fold uses,
so the tree is unchanged by construction and checked before and
after; no hook runs and nothing is pushed. The reworded messages are
checked again, and the attempt completes at the reworded head. A
rewriter that stops without a file, leaves an empty file or subject
line, or whose swap is refused asks again with `accept | park` only,
the branch where it was; a second agent run is never spent unless the
user chooses it. A tree that is dirty, or a head that moved, while the
rewriter ran parks with both heads named, and so does an answer given
after the branch moved. `launched` is saved before the rewriter's
`session.new`, so a lost reply is found again by recovery and never
sent twice; `moving` is saved before git writes the reworded commits,
and a restart reads it as a rewrite's intent is read, against
`message.from`: the branch still at the folded head swaps again,
another head with the same clean tree is where it landed, and anything
else fails the attempt. A `message` dial of `auto` accepts without
asking, since `rewrite` spends an agent run.

**Records.** One attempt per context per stage run, with `rounds` on
it: each round's base, head, reviewers (name, kind, session or launch
intent, completion, result), the aggregated feedback, the open point
count, the implementer's session, response and `head_after`, and its
state (`reviewing`, `findings`, `fixing`, `fixed`, `converged`,
`accepted`, `failed`), plus `carried_from` and `rework` (see "A new
attempt") and `rewrite` (see "Clean commits": the mode, `before`,
`after`, the commit counts `from` and `to`, `skipped`, and, from
"Folded messages", `stale` (each stale commit's index, subject and
missing names) and `message` (the answer, the folded head `from`,
`launched`, the rewriter's session and its settling, `moving`, the
reworded head `to`, and `failed`); once the branch moves to a reworded
head, `after` is that head). Artifacts
per round: each reviewer's file (`r<n>/<reviewer>`), `r<n>/feedback`,
`r<n>/response`, `r<n>/checks`, and for a rewording `message/input`
and `message/<sha8>`;
and once the attempt completes, `summary` (`summary.md` in the attempt
directory): how it ended (converged, or accepted with how many points
open), how its commits were folded or why they were kept, the names a
folded message carried and what became of it, what it
carried, the last round's points left to the merge, its
open points when accepted, and every round's points found but not
done.
The wire view carries `AttemptView.rounds`; the ticket page shows one
line per round. Nothing in Switchboard's `model.rs`, `AppAction` or
`Effect` changed: reviewers and implementers are ordinary sessions
over the existing port commands.

**Runner children.** A command reviewer's launch intent is written on
its record before it starts; a restart that finds the intent with no
result fails the reviewer, never starts a second copy and never kills
an unrelated process. Children run in their own process group and are
killed with it when the ticket parks or a rerun retires the attempt.
A check's group is recorded on its `GateRun` as the leader's pid and
its start time (`ps -o lstart=` under `LC_ALL=C`, `TZ=UTC`). A runner
that finds a running gate whose check it never started, on the next
pass that polls it or on a park or a close, looks at the recorded
group: no member left clears the record; a live leader with another
start time took a reused pid and is left alone. A gone leader, or one
whose start time `ps` could not read, proves nothing (the id may have
gone to a new group whose leader exited), so its group is left alone
too and a leaderless orphan is missed. A live leader with the recorded
start time is ours, and the group gets TERM, then SIGKILL past the
stop limit, counted from that group's first TERM across restarts for
as long as the leader lives. A leader that dies of the TERM (the usual
`sh -c`) reads as gone on the next restart, so a member that ignored
the TERM is left running and never gets SIGKILL. The kill is recorded
on the attempt's `orphans_killed` and logged as `check-orphan-killed`;
only then do the checks start again or the attempt read cancelled. Two
gaps are left: a leaderless orphan, whether its leader exited before
the restart or died of our TERM, is left running beside the new
checks; and a crash between the spawn and the save that records the
group, one `fsync` long, leaves a group no record names. Command reviewers record no group and are not
adopted.

**Validation** refuses: an unknown or repeated reviewer, a command
reviewer with no `argv`, a missing implementer or one that is not
Claude Code, `cap = 0`, a subject other than `branch`, no command
gate, `operator`, `review` or `writes` beside `reviewers`, a project
without branches, a command operator anywhere but as a reviewer, and
a `like` that names no earlier stage with a command gate of its own,
and `commits` on a stage without reviewers, or other than `keep` in a
pull-request pipeline (an unknown value is a parse error).

**Templates.** `review_prompt` takes `{base}`, `{head}`, `{worktree}`,
`{feedback}`, `{no_feedback}`, `{plan}`, `{branch}` and the `{issue.*}`
values; when earlier points are open the reviewer is also told
`{previous_feedback}` and `{previous_response}` and how to withdraw
or keep each. `fix_prompt` takes `{feedback}`, `{response}`,
`{worktree}`, `{branch}`, `{plan}`. Operator guidance is rendered with
the same values and comes first. Four texts follow every agent
reviewer's template, whatever the stage's `review_prompt`: on round 1
of a carried attempt, the old attempt's settled and open points and the
range to read; on the first review of a rebase (below), the check of
it; when the ticket has a plan, its decisions section (a heading whose
title, after an optional number, starts with "Decisions") as settled,
with how to write a `decided:` point, or that the plan lists none (an
unreadable plan file adds nothing); and how to tag a `style:` point.
A command reviewer gets no prompt: its points are untagged and hold the
round open.

Out of scope for this cut: reviewers seeing each other's points, a
workflow or a human as a reviewer, review of anything but a branch,
cross-lane review of a joined result, and releasing the slot while
every lane waits on a question (an open review attempt holds the
ticket's slot as any open attempt does).

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
  "Decisions"; a deploy still running keeps `my-dev`. As built, a hold
  is released on the pass after the ticket leaves its range (an
  advance, a send-back, the pipeline's end), once the services it
  started there read back as gone: each `before` exited, each session
  killed and read back gone, each port binding again, and each session
  removed with `session.remove`. A park or a close releases the same
  way. A stop not confirmed within two minutes (`STOP_LIMIT_MS`) is a
  `stuck` decision with `wait` and `released`, asked even while the
  ticket parks or closes; only a `released` answer lets a hold go
  without the runner's confirmation.
- The in-place lane hold above is not built: a stage whose `needs`
  names a lane parks, saying so.
- Holds cover Dispatch tickets only. Dispatch cannot see a manual
  deploy to `my-dev`; `dispatch status` shows the holds so you can
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
decision; a `none` reading within two minutes of the head's last move,
`PR_YOUNG_HEAD_MS`, is pending instead, since GitHub creates a pushed
head's check runs a little after the push), and any lookup error
(retry with backoff, a decision after an hour). Green at an older head
is not green: a moved head voids it along with every other result made
against the old head set, as described under "Stage semantics".
`pr-merged` reads the same PR and resolves the pending merge decision;
it never merges.

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
spent `max_rebases` are each a `pr` question with `recheck`. A
remedy's completion logs as `rebaser complete at <head>` or `fixer
complete at <head>`, never the bare `complete` of the stage's own
agent, so the log does not read as the stage finishing.

A branch does not wait for a PR to be brought up to date. With
`refresh` on (the default), each stage that launches something begins
by fetching the lane's base and, when the base moved since the lane's
`base_sha`, bringing the branch up to it: a branch with no commits of
its own simply moves, one with commits is rebased, and either way
`base_sha` becomes the new base and the next agent's prompt says the
base moved and names the range, so a plan written against the old
code is read with that in mind. When the provider reports an open
pull request for the lane's branch, a branch the refresh rewrote is
pushed with `--force-with-lease` on the head the lane's records last
saw (the newest attempt in its context that recorded one, or the
lane's `pushed`, the head a refresh last pushed, when that came
later), after the rebase or the rebaser, at whatever stage the refresh
runs; a lane without one pushes nothing, and a refused lease is left
to the `pr` question about the head. A rebase that stops on a conflict
is aborted, and the policy's `rebaser` is continued from the lane's
last finished agent, told the base and the stage's checks, and asked
to resolve without pushing or, when a conflict's intent is unclear, to
leave the branch as it was and say why; the stage waits for it, reads
the branch again when it stops, and after `max_rebases` such attempts,
or without a rebaser, asks a `refresh` question with `recheck`. The
rebaser's attempts, and the `rerun` questions about them, carry the
pseudo-stage `refresh`, so no stage mistakes them for its own; the
`refresh` question carries the real stage and is named `refresh`.
Either holds the stage. A worktree with a rebase stopped part way
(`Repo::rebase_in_progress`, either backend), or whose `HEAD` is not
the lane's branch (`Repo::branch_head`), is not read at all, since
nothing read from `HEAD` there is the branch's: the stage waits on a
`refresh` question saying so, answered `recheck` once the owner has
finished, aborted or checked out by hand. A rebaser that fails holds
the stage on its `rerun` question, which says when it left the tree
mid-rebase; `rerun` reads the lane again rather than launching
another, so a hand-finished rebase is brought up, an aborted one
conflicts again, and one still stopped asks the `refresh` question. A
stage held on a lane is not marked refreshed, so a park and resume,
which withdraws the question, reads the lane again.

A lane with no `base_sha` gets its fork point from the base as
`base_sha` while it is still behind, before anything moves, so the bring-up after a rebaser reads the old base; a lane not
behind whose fork point is the base itself never moved, and records
that as `base_sha` with no bring-up; one whose fork point cannot be
read has an unknown old base and records `from` empty. Each bring-up
is recorded on the lane (`LaneRecord.refreshed`) with `from`, `to`,
whether the branch had commits of its own (its head before the
bring-up was not the old `base_sha`, or, when that is unknown, not the
new base), when it was recorded, and, when it followed a rebaser
(nothing left behind), that rebaser's notes: the latest finished
`refresh` attempt in the lane started after the lane's previous
bring-up, none after a bring-up recorded before its time was. The
first code review round that reads the new base after a rebase with
commits is told to check it: both sides of every conflicted hunk
present, and the base's additions in `from..to` unchanged by the
branch (with an empty `from`, the additions the rebase brought in,
naming no range), with the rebaser's notes when there are some. A
later round or a rerun that already read that base is not told again.

A rebase that stops is recorded on the lane (`LaneRecord.conflict`)
before the rebaser starts or the question is asked: the head the branch
was at, which is the last head a review read, its base, the base it
would not rebase onto, the stage, and the branch's commits whose
replay conflicts (`Repo::conflicting_commits`: each commit merged with
`git merge-tree` onto the result of the one before, writing no ref;
empty when it cannot be read). A conflict seen again keeps its head and
base, since the stage is held and nothing has read the branch since, and
updates the rest. A stage that goes ahead without a bring-up (a tree
left alone) drops the record: the stage reads
and moves the branch, and a later rebase that stops records its own.
The bring-up moves it to `refreshed.conflict` when the branch was
rewritten since the rebase stopped (by the rebaser or by hand), with the
bring-up's stage, and records the head it reached as `refreshed.after`;
a branch git rebased untouched onto a newer base resolved nothing, and
the record is dropped. A bring-up with a conflict at a stage after the
pipeline's last code review stage, which pushed nothing (no open PR),
gets one resolution review before the stage runs: an ordinary `review`
attempt under the pseudo-stage `resolution`, shaped as the last code
review stage with one reviewer (the policy's `resolution_reviewer`,
else that stage's first that is not `style`), one pass, style points
left to the merge and the default `No findings.` sentinel. Its prompt
names the reviewed head and base, the new base and head, the
conflicting commits, and the one range to read, `git range-diff
<from>..<before> <to>..<after>` (with the base unknown, the fork point
of the reviewed head from the new base, or else a commit-by-commit
comparison), with the rebaser's notes. A clean review completes it at
the head with nothing run. Points are a `resolution` question, `fix |
accept | park`, auto only when the `resolution` dial says so (not the
`review-code` dial): `fix` is one fixer, the stage's checks and its
`commits` mode (a published branch keeps its commits), and the attempt
completes with no second reviewer; `accept` completes at the reviewed
head. A failed pass asks `rerun`, and a rerun reviews the same
resolution afresh, carrying nothing. A conflict brought up at or
before the last code review stage is read by that stage's rebase
check, and a conflict at `ready` with an open PR was pushed by the
bring-up already; neither gets a pass. A pass belongs to the bring-up
it started at or after, so a later one gets its own.

Pull-request tickets are someone else's branch and are never
refreshed; `lanes`, a human look and the merge watch launch nothing
and are not refreshed either.

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
shared with its issue tickets: the counts in `<project>.toml` govern
both files' tickets, and the `.pr.toml`'s own `[policy]` counts are
not read.

```
dispatch take Orchard pr backend/123 frontend/45     # the lanes' own repositories
dispatch take Orchard pr github:backend/17           # a named mirror
dispatch take Switchboard pr 12                    # the lane may be left off when there is one
```

Each spec names a lane and a pull request number, and may name a
remote in front: without one the PR is read from the lane's own
repository, with one from that entry of the lane's (or the project's)
`remotes`, a mirror where collaborators without access to the main
host work. The provider is the remote's host, GitHub or Bitbucket,
and the PR must be open. One ticket carries one PR per lane; two lanes
that share the project's repository cannot each carry one. The
identity is the set of PRs with their providers, so a PR on a live
ticket is refused until that ticket closes. The snapshot records each
PR's lane, provider, repository, number, URL, branch, base, remote,
the branch the lane checks out, head and title; the ticket's title is
the first PR's.

The lanes are the PRs' branches as their remote has them. The clone
gains the named remote when it is a mirror. On GitHub the checkout is
the pull ref, `refs/pull/<n>/head`, on a local branch `pr/<n>`, so a
PR from a fork works the same as one from the repository; on
Bitbucket it is the PR's branch itself. Either way the worktree
tracks the remote's ref (`git worktree add --track -B <local>
<remote>/<local>`), and later readings of the PR (its checks, its
merge) go by number to the provider it came from. A PR in a lane
without a repository of its own makes the tree itself that branch.
Lanes no PR is in are not cut. Nothing in this mode pushes: a pull-request
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
- *Non-replayable* (`session.send`, `session.resume`,
  `workflow.continue`): a repeat
  can spend money twice. A lost reply to one is not repeated; it is a
  decision that shows what was sent and lets you look at the pane,
  asked once: the ledger entry is marked, recovery leaves it to your
  answer, and a `park` answer is acted on even with every slot taken.
  A lost reply to a nudge is the exception: it is neither repeated nor
  asked about, because it is recorded on the attempt before it is
  sent and bounded by `on_dirty`, and the idle grace ends the wait if
  it never arrived.

A failed reply to a query is judged by its words. Only `no such
session` or `no such run` means the record is gone, and only that fails
an attempt or reads a paused run as stopped; any other failed reply
(the app too busy to answer in time) or a socket error says nothing
about the record and is asked again on the next pass.

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
  no transcript is refused; `session.resume {session}`: an agent's
  conversation resumed from its handle with no terminal opened, a
  running pane left alone, and a session with no handle or one marked
  not resumable refused rather than launched fresh
- `service.new {project, name, argv, env}` → `session`: a service
  record Dispatch owns, killed and removed by it; whether it is
  listening is Dispatch's probe, not Switchboard's reply. Not built:
  Dispatch makes its services with `session.new` (kind `service`, an
  argv launch carrying the port through `env`) and removes them with
  `session.remove` meanwhile
- `command.run {project, name, argv, env}` → `session`; the exit code
  is read back with `session.get`. Not built: a command gate and a
  service's `before` run as children of the runner meanwhile
- `space.new {name}` → `space`; `set.new {space, name}` → `set`
  (`space` may be the global workspace's fixed id,
  `00000000-0000-0000-0000-000000000002`, for a set that holds cards
  from every workspace; `project.add` refuses that id, since the global
  workspace holds no projects);
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
- `session.screen {session, lines}`: the last `lines` (40 by default,
  at most 200) of a running session's pane, with the values of its
  project's secrets replaced by their names (`<NAME>`), since the
  reply becomes another program's output; `not running` for a session
  that is not. An app from before it answers `bad request`, which
  `dispatch tail` reads as "update the app"
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

## Supervisor

A project may have one supervisor: a long-lived Claude Code session
that watches its tickets and answers the decisions the owner lets it.
It comes from a table in the live `pipelines/<project>.toml` (never a
ticket's frozen copy, never `<project>.pr.toml`):

```toml
[supervisor]
guidance = "Keep the queue moving; the owner reviews code."
read = ["CLAUDE.md", "docs/design.md"]    # relative to the workspace
setup = ["git", "clone", "git@example.com:o/r.git", "."]
# or several: [[supervisor.setup]] argv = [...]
model = "sonnet"
decides = ["finalize", "rerun", "pr"]
merges = true      # it merges a green, clean pull request itself; false (default): it reports and the owner merges
```

`decides` takes decision names: those Dispatch asks of its own accord
(`finalize`, `paused`, `rerun`, `pr`, `branch`, `lanes`, `refresh`,
`review-cap`, `review-code`, `message`, `resolution`, `lost-send`) and
those a gate of the same file asks (a human gate's `decision`, an
external gate's, and `merge` for a `pr-merged` gate that names none).
An answer such as `recheck` is refused with the decisions that take it.

**Where it lives.** `projects/<project>/supervisor/` in the data
directory holds `seed.md`, `handoff.md` and every earlier hand-off as
`handoff.<yyyymmdd-hhmmss>.md`. The workspace, the session's cwd, is
`supervisor-<project>` under the root a ticket's trees use (the
pipeline's `worktrees`, else the data directory's setting), refused if
a shell would split its path, as a ticket's tree is; it is recorded the
first time it is made, so moving the worktrees root does not orphan it.
Setup runs there when it is missing or `--setup` is given: the table's
argvs, else `git clone <repo> .` for a project with a `repo`, else
nothing. A setup that fails removes a workspace it made and leaves the
current supervisor as it was.

**The seed** is the guidance, how the supervisor works (the owner hands
it work in the session and it takes no initiative of its own; at start it
runs `brief`, compares it with the hand-off, arms a background watch per
ticket in flight and then waits; watches are background calls, never a
foreground loop; the hand-off is written with the Edit tool, never a shell
script), the `read` paths made absolute, what
each listed decision's answers do and that every other is the owner's,
that `merge` is answered `park` only, the full path of the `dispatch`
executable (its allow rule matches that path), "run `dispatch brief
<project>` first", the hand-off to keep current, and the commands it
works with. The session's first prompt is "Read `<seed.md>` and do
what it says". The seed is stale when a hash of the table, the
project's name and the commands' text no longer matches the one the
session was seeded with; the rendered seed's paths are left out, since
they differ between builds.

**The session** is made through the control port as a ticket's are,
each creation on the project's record before it is sent: the space,
a Switchboard project `Supervisor · <project>` rooted at the workspace
(made once), and a Claude session of the same name with the model,
`--allowedTools Bash(<dispatch>:*)`, read and write rules for the
supervisor directory, and with `merges = true` rules for `gh pr view`,
`checks`, `diff` and `merge` and `git pull` and `log`, so a merging
supervisor is not stopped at the merge. `merges` also chooses the seed's
pull request paragraph: merge on green with a clean body, or report and
stop for the owner. No settings file is written: `--settings`
already carries Switchboard's hooks. `dispatch supervisor <project>
--fresh [--setup]` (or Fresh on the Dispatch page, through the port's
intent) sets up, writes the seed, kills the current session and keeps
it in `past`, rotates the hand-off under a single `## From the
session of <date>` heading, which replaces the ones earlier rotations
left on top (a hand-off that is empty, or holds only those headings,
gets no heading), and makes the new one. `--resume` sends `session.resume`;
`--kill [--reason]` kills it and keeps it in `past`. Nothing resumes or
starts a supervisor by itself.

**The actor.** `dispatch` reads `SWITCHBOARD_RECORD_ID`, which the
supervisor's pane sets and its Bash tool inherits. When it names a
project's current or past supervisor, the command is that
supervisor's: it may read anything, `take` and `queue` on its own
project, and `decide`, `park`, `resume --no-rerun` and `close` on its
own project's tickets; a plain `resume`, which reruns what the park
cancelled, needs `rerun` in `decides`. `restart`, `run`, `worktrees`
with a path or `--migrate`, `supervisor --fresh`, `--resume` and
`--kill`, any verb not on the list, and any other project are refused
with exit 1. A `decide` on a name outside `decides` saves a refusal on
the decision and logs a `refused` event; the decision still waits on
the owner. What a supervisor does is stamped `supervisor`: the
answer's `by`, `state_by`, `taken_by`, and the events' `actor`.

The rule guards against a supervisor's mistakes, not against a hostile
agent, which can unset the variable. Gaps: nothing wakes an idle
supervisor (a `session.send` costs money and cannot be replayed), so
it follows the log with `events --follow --timeout`; setup runs
unconfined and under the writer lock, like a lane's; with a
non-default `DISPATCH_DATA_DIR` the supervisor's commands need the
variable set, which its allow rule does not cover; and a refused
`resume`, `restart`, `worktrees` or `supervisor` is an exit and a
message, not an event, since no decision carries it.

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

The Dispatch page lists every ticket in one table: the numbered title
as the link, project, stage with its place in the pipeline, standing
(what the ticket waits on, or why it is not moving), and when it was
last written. A header sorts by its column and sorts the other way on
a second click; `Updated` starts newest first and the others the way
they read. Chips narrow it to tickets waiting on you, active, parked
or closed, and a word filter keeps only rows that contain every word
somewhere (number, title, project, stage, standing or labels), beside
the project chips. The last column acts: `Answer` on a ticket with a
decision pending, `Resume` on a parked one, `Open` on any. The order
and the narrowing are the core's (`AppCore::tickets_listed`), total
and stable across polls; the page only draws. Above the table the
decisions and agents waiting on you are cards; up to three stand open,
and a larger pile starts folded behind its count so the table stays on
the first screen. The fold is chosen when the first status arrives and
the user's clicks on it stand after that.

Dispatch also has a port of its own, so a reader can see tickets as
they are rather than through the sessions they made. While `dispatch
run` is up it serves `<data>/dispatch.sock` (wire crate
`dispatch-control`): `status` answers every project's queue and every
ticket as a view (stage names resolved, attempts, decisions, lanes,
artifact paths), `ticket` one in full (with its plan review's round
files and each lane's clone), `artifact` the text of a file under a
ticket's directory and nothing outside it, `events` a ticket's lines
of the event log after a cursor (with the next cursor and the seqs a
`void` withdrew), and `decide`,
`queue`, `take`, `resume`, `close` and `worktrees` do exactly what the command
line does, through the same runner methods under the same writer lock.
`supervisor-fresh {project}` writes the intent for a new supervisor and
answers with the status at once; the runner's next pass does the work,
which asks the caller's own control socket. A project's view carries
`supervisor` when its live pipeline has the table: the session's id,
its age, whether its seed is stale, how many it replaced, a fresh
pending, and the last error. Switchboard's Dispatch
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

The third slice builds Orchard's deploy, `try` and `tried`. A gate-only
stage with a command gate (`deploy`) runs the command in its context's
clean tree as a child of the runner, as an agent stage's checks run,
and the head it ran at is the deployed commit, `{inputs.deploy.commit}`
to later prompts; a stage in a lane the ticket did not choose is
skipped and reads `unknown (deploy skipped)`. A failure asks `rerun` or
`park`, and a deploy lost to a runner restart is that question, since
it may have run. `needs` are holds on the ticket record, taken on
entering the range under the writer lock against the other tickets'
records and released the pass after leaving it; a ticket waiting for
one asks nothing and costs no slot, and one holding it costs a slot.
`services` are Switchboard service sessions made with `session.new`
(an argv `env` launch with the port), each after its `before` ran as a
child of the runner; the tester starts once each answers, told each
URL, and they are stopped (killed, port free, removed) when the ticket
leaves the range, parks or closes.

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
| The tree is dirty when the agent stops | No check runs and nothing is killed; one nudge is typed into the session, recorded on the attempt; a clean stop after it runs the checks at the tree's head (`a_dirty_implementer_is_nudged_and_its_clean_stop_runs_the_checks`) |
| The tree is still dirty at the stop after the nudge | A failed attempt and the rerun decision with `rerun`, `check` and `park`, saying `after 1 nudge` (`a_dirty_implementer_after_its_nudge_is_asked_after_1_nudge`) |
| The tree is dirty when the agent stops, under `on_dirty = "ask"` | No check runs and nothing is typed; a failed attempt and the same decision (`a_dirty_tree_after_the_agent_never_runs_the_checks`) |
| Dispatch restarts after a nudge | No second nudge; a clean stop after it runs the checks (`a_restart_after_a_nudge_sends_no_second_one`) |
| The reply to a nudge is lost | Not repeated and not asked about; with no stop after it, the idle grace ends in the rerun decision saying `no stop came after the last nudge` (`a_lost_nudge_reply_asks_nothing_and_the_idle_grace_ends_it`) |
| A nudge goes unanswered with nudges left | The question, not a second nudge (`an_unanswered_nudge_is_asked_about_even_with_nudges_left`); with `{ nudge = 2 }` and two dirty stops after nudges, the question says `after 2 nudges` (`two_nudges_answered_by_dirty_stops_then_the_question`) |
| The session ends after a nudge with no stop after it | Judged as the stop before the nudge left it: the rerun decision with `rerun`, `check` and `park`, saying `after 1 nudge; the session ended with no stop after the last nudge` (`a_nudged_session_that_ends_is_asked_about_at_its_checks`); a fixer's round fails the same way (`a_nudged_fixer_whose_pane_goes_fails_on_its_dirty_tree`) |
| A review's fold whose replay conflicts | A failed attempt and a rerun decision with `rerun`, `keep` and `park`, the branch at the reviewed head; `keep` completes the stage there with `skipped = "the user kept them after the rewrite failed"` and no check run (`a_fold_whose_replay_conflicts_is_kept_by_hand`) |
| Checks reused from `implement`, then the fold fails | `keep` completes the same way on the reused checks, with none run (`keep_after_reused_checks_completes_without_running_them`) |
| A move undone over a tree the move left dirty | `keep` fails the attempt again with `rerun`, `keep` and `park` while the tree is dirty; once the tree is cleaned, before or after that, `keep` completes at the reviewed head (`keep_after_a_move_back_on_a_tree_left_dirty_is_asked_again_and_kept_once_cleaned`, `keep_after_a_move_back_completes_once_the_tree_is_cleaned`) |
| `keep` with the branch moved since the question | The ticket parks naming both heads; the attempt stays failed (`keep_with_the_branch_moved_since_parks`) |
| `keep` on a tree dirtied since the question | A fresh rerun decision with `rerun`, `keep` and `park` (`keep_on_a_dirty_tree_is_asked_again`) |
| A ticket parked over a rewrite failure is resumed | The question offers `rerun`, `keep` and `park` (`a_parked_rewrite_failure_offers_keep_on_resume`) |
| A fold whose message names a name neither the folded commit nor the tree has | The attempt stays open at the folded head and asks `message` with `rewrite`, `accept` and `park`, naming the commit and the name (`a_folded_message_naming_a_removed_name_asks`) |
| A fold whose message's names all exist | The attempt completes at the folded head; nothing is asked (`a_folded_message_whose_names_all_exist_completes`) |
| `rewrite` answered | The implementer operator writes the new message to a file; the folded commits are replayed one for one with it, the tree unchanged, the branch moved, the rewriter killed, nothing pushed, and the stage completes at the reworded head (`rewrite_rewords_the_folded_commit_and_keeps_the_tree`) |
| `accept` answered | The stage completes at the folded head; the summary says the message was kept as written (`accept_keeps_the_folded_message`) |
| The rewriter stops without its file | Its session is killed and `message` is asked again with `accept` and `park` only; `accept` completes at the folded head (`a_message_rewrite_that_fails_asks_accept_or_park`) |
| The reworded message still names a name neither has | The branch stays at the reworded head and `message` is asked again with `accept` and `park`; `accept` completes there, the summary saying the rewording was kept as rewritten (`accept_after_a_rewording_that_still_names_it_keeps_the_rewording`) |
| A fixer's `squash!` body names the renamed name | Nothing is asked (`a_squash_fix_that_renames_the_name_asks_nothing`) |
| A stage fails past the policy's `max_reruns` in one context | The ticket parks with the count and the last reason; nothing is asked |
| Free space on the worktrees' volume is under the policy's `min_free_gb` | Nothing new starts and `status` says why; running attempts are still watched; the hold lifts on its own (`a_full_disk_holds_new_starts_until_space_is_back`) |
| `take` while a close removes a large tree | returns at once; the next pass drives it (`a_take_during_a_long_tree_removal_returns_at_once_and_runs_next_pass`) |
| `take` while the last stage's close removes the tree | The take returns at once and stays in the queue; the ticket closes with its tree removed (`a_take_while_the_last_stage_closes_the_ticket_is_kept`) |
| A pass reaches a ticket a hand close is removing trees for | The pass leaves it to the hand close, which removes each tree once (`a_pass_during_a_hand_close_leaves_the_removal_to_it`) |
| A second `dispatch close` while a retry removes kept trees | Refused with "its trees are being removed by another dispatch"; the first retry finishes (`a_second_retry_during_a_retry_is_turned_away`) |
| A retry of kept trees is cut off mid-removal | The ticket still reads `trees_kept` and the next `dispatch close` retries it (`a_retry_in_progress_keeps_its_trees_retryable`) |
| `decide` on a ticket whose trees are being removed | Refused without writing the record; the ticket closes (`a_decision_on_a_ticket_whose_trees_are_being_removed_is_refused`) |
| The writer lock cannot be taken back after a tree removal | Every later write in that transaction is refused, so nothing read before the gap is saved over what landed in it (`a_lock_not_taken_back_refuses_every_later_write_in_its_transaction`) |
| A ticket with two pending decisions is parked from one | The other is cancelled in the same write as the parking intent; the park answer reads acted; the session is unmarked before the ticket reads parked; none is pending, `waiting_on_me` no longer counts it, the cancelled one cannot be answered; a resume asks a fresh rerun per lane and launches nothing (`parking_cancels_every_pending_decision_and_a_resume_asks_afresh`) |
| The pass dies at the unmark after the parking intent | No decision is pending on disk; the next pass, without a restart, sends the same operation again and only then reads parked (`a_park_cut_off_after_its_intent_asks_nothing_and_unmarks_on_the_next_pass`) |
| An earlier waiting mark's reply was lost when the ticket parks | It is resolved under its own id before the unmark is sent; parked only with every waiting request answered, and a restart's recovery does not turn the mark back on (`an_unanswered_mark_is_resolved_before_the_unmark_and_stays_off_after_a_restart`) |
| Two lanes' questions after a resume, the pass cut off between them | The next pass asks the missing one and not the other again; an answered lane starts while the other lane's question is open (`a_pass_cut_off_between_two_lanes_finishes_asking_on_the_next_pass`) |
| A waiting request on the ledger has no body when the ticket parks | It cannot be sent again, so it is left unanswered and parking does not wait on it (`a_waiting_request_without_its_body_does_not_hold_parking`) |
| Every slot is taken when a ticket is resumed | Its rerun questions are asked anyway; nothing launches (`a_resumed_ticket_is_asked_again_with_every_slot_taken`) |
| Every slot is taken when a ticket with a body-less waiting request is resumed | That entry does not stall the re-ask: each lane gets a fresh rerun question; nothing launches (`a_resumed_ticket_with_a_dead_request_is_asked_again_with_every_slot_taken`) |
| Every slot is taken when a ticket with a lost reply to an unrepeatable request is resumed | One `lost-send` question, which holds its lane while the other lane is asked; asked once, not again on later passes or after its answer; `park` parks without a slot (`a_resumed_ticket_with_a_lost_unrepeatable_request_is_asked_once_and_parks_unslotted`) |
| A lane's `rerun` is answered with every slot taken, then the other lane's question is answered `park` | The waiting answer is cancelled with the parking intent; the resume asks both lanes afresh, and with slots free again the old answer launches nothing (`an_answer_waiting_for_a_slot_is_withdrawn_by_an_unslotted_park`) |
| Two lanes' `rerun` questions are answered `rerun` and `park` before one pass, with slots free | The `rerun` is acted on first, then the `park` cancels it with the parking intent before its replacement launches; the resume asks both lanes afresh and launches nothing (`a_rerun_acted_before_a_park_in_the_same_pass_launches_nothing`) |
| Two lanes' `inspect` questions are answered `rerun` with a note and `park` before one pass, with slots free | The send-back is acted on first, then the `park` drops its note with the parking intent before an implementer carries it; the resume asks `rerun` about that lane's sent-back attempt, quoting the note, and launches nothing; answered `rerun`, the new attempt's prompt ends with the note (`a_send_back_acted_before_a_park_in_the_same_pass_launches_nothing`) |
| A ticket parked over failed checks is resumed | The question offers `rerun`, `check` and `park` again; `check` passes on the same attempt with no agent (`a_resume_after_failed_checks_offers_check_again`) |
| A ticket parked over failed checks past `max_reruns` is resumed | The park asked nothing, but the resume's question still offers `rerun`, `check` and `park`: the attempt keeps that it failed at the checks (`a_resume_after_failed_checks_past_max_reruns_offers_check_again`) |
| A ticket parked at a question about its open attempt is resumed | The question quotes the attempt's own cancellation reason (`a_resume_after_a_cancelled_attempt_quotes_its_reason`) |
| A ticket is parked while its checks run | The checks are killed with their process group; the attempt stays open and the ticket `Parking` until they read back as gone, then cancelled; the resume reruns the attempt with no question, and its checks start only after the old ones were killed (`parking_during_checks_kills_them_before_the_attempt_reads_cancelled`) |
| `dispatch park` with an implementer running and a question about something else | The intent is written with the question cancelled; the next pass cancels the attempt with the reason, kills its session and reads `parked`; the log has `parking`, `parked` and `decision-cancelled` (`dispatch_park_stops_a_running_attempt_and_withdraws_an_unrelated_question`) |
| That ticket is resumed | A `rerun` answered by `resume` is recorded, nothing is pending, the second attempt launches, and `resumed` names `implement (repo)` (`a_resume_reruns_what_the_park_cancelled_without_asking`) |
| Two lanes, one failed before `dispatch park` and one running when it came, resumed | The running lane relaunches; the failed lane is asked `rerun` (`a_resume_reruns_the_cancelled_lane_and_asks_about_the_failed_one`) |
| The same ticket resumed with `--no-rerun` | A pending `rerun` quoting the park's reason; nothing launches (`a_resume_with_no_rerun_asks_as_before`) |
| A park whose checks outlived the limit is resumed | The reason carries the checks' suffix and the attempt still reruns (`a_resume_reruns_an_attempt_whose_checks_outlived_the_park`) |
| `dispatch park` on a closing or closed ticket | Exit 64 with the reason; one already parking or parked is refused with exit 1 (`park_on_a_closed_ticket_is_a_usage_error`) |
| A ticket is closed while its checks run | The same: the ticket stays `Closing` and the attempt open until the checks are gone, then `the ticket closed: …` and `Closed` (`closing_during_checks_kills_them_before_the_attempt_reads_cancelled`) |
| A parked ticket's checks ignore the kill | Two minutes after the kill the group gets SIGKILL and the ticket parks; the cancellation reason says the checks were still running and names them (`a_check_that_ignores_the_kill_does_not_hold_the_park_past_the_limit`) |
| The runner restarts while the checks run, then the ticket is parked | The lost check reads as gone; the attempt is cancelled on the first pass and nothing starts again (`parking_after_a_restart_reads_a_lost_check_as_gone`) |
| A ticket is closed during a code review round | Its command reviewers are killed before the attempt reads `the ticket closed: …` (`closing_during_a_review_round_kills_its_command_reviewers`) |
| The runner restarts while the checks run | The lost check starts again on the same head; no second agent (an agent stage's checks; a gate-only command is a question) |
| `ready` with the PR's checks pending, then green | A gate-only attempt per context, no agent; no PR is a `pr` decision (`recheck`, `park`); pending waits and reads the provider once a minute; green at the tree's head completes the attempt bound to that head |
| The PR is at another head, its checks are red, or it has no checks | A `pr` decision naming which; `recheck` reads again at once; the same attempt throughout; `checks = "none"` on the stage passes on the PR at the head alone |
| The provider cannot be read | The error is recorded on the attempt and retried quietly for an hour, then a `pr` decision; a merged PR passes |
| `ready` with every slot taken by another ticket's agent | The PR is still read and the ticket still closes; a gate-only attempt holds no slot and the other agent keeps its own |
| A human gate (`inspect`) after `implement` | One gate-only attempt and one decision per lane, naming the branch and head, what it adds over its base, the tree and the notes; `proceed` completes it bound to the head; `park` stops |
| `rerun` with a note at `inspect` | That lane's `implement` result and the gate's attempt are cancelled, the ticket stands at `implement` again, a fresh implementer gets the note at the end of its prompt, other lanes are untouched, and `inspect` asks again on a new attempt when it is done |
| `merge` with the PR open | A confirmation decision with only `park`, the session marked waiting; `merged` by hand is refused; the PR is read once a minute |
| The user's own feedback round after the review converged | The pending `finalize` decision is cancelled and the session unmarked while the planner answers; when the run converges again a new decision names the new round count |
| The PR conflicts with its base at `merge` | The policy's rebaser starts in the lane, cloned from the implementer's session, with the PR, the base and the notes path in its prompt; the merge decision stays; when it stops the gate reads the PR again and, merged, the ticket closes; the rebaser's attempt still records `conflicting`, which its completion line names as the rebaser's (`a_conflicting_pr_is_rebased_by_a_clone_of_the_implementer`; the log text in `a_remedys_completion_names_its_role_and_head`) |
| The rebaser leaves the PR at the same head, or `max_rebases` is spent | A `pr` decision saying which; no further rebaser runs |
| The base moved while a plan sat; implementation begins | The branch is brought up to the base, `base_sha` is the new base, the implementer is told the range; nothing but git ran (`a_plan_that_sat_is_implemented_on_a_branch_brought_up_to_its_base`) |
| The base moved after the PR was opened; `ready` begins | The branch is rebased and pushed once with a lease on the head last seen; `ready` reads the PR at the tree's head (`a_refresh_at_ready_pushes_the_rebased_branch_once_with_the_lease`, `a_refresh_at_ready_pushes_after_the_rebaser_resolves_it`) |
| The same, the remote moved meanwhile | The lease refuses the push; `ready` asks its `pr` question about the head (`a_refused_lease_at_ready_asks_the_pr_question`) |
| The same, with no pull request for the branch | Nothing is pushed; `ready` asks for a PR to be opened (`a_refresh_at_ready_without_a_pull_request_pushes_nothing`) |
| A stage begins and the rebase onto the moved base conflicts | The rebaser, a clone of the lane's last finished agent, is told the base and the checks; the stage waits, then reads the branch again (`a_conflicting_refresh_is_rebased_by_a_clone_of_the_lanes_last_agent`) |
| The same, with no rebaser in the policy | A `refresh` question with `recheck`, answered after a rebase by hand (`a_conflicting_refresh_without_a_rebaser_is_a_question`) |
| The base moved after `review-code`; the rebase at `pr` is clean | The branch is brought up; no reviewer starts and `pr` runs (`a_clean_refresh_at_pr_launches_no_reviewer`) |
| The rebase at `pr` conflicts and the rebaser resolves it | The reviewed head and the conflicting commits are on the lane before the rebaser starts; then one `resolution` reviewer reads the range-diff of the reviewed and the resolved branch, and `pr` waits; a clean answer completes it with nothing run (`a_conflicting_refresh_at_pr_is_reviewed_before_the_pr_opens`) |
| The resolution review finds points, with `review-code = "auto"` | A `resolution` question with `fix`, `accept` and `park`; `fix` is one fixer and the checks, no second reviewer; `accept` completes at the reviewed head (`a_resolution_pass_with_points_asks_fix_accept_park`) |
| The conflict was brought up at `implement` and the base has not moved since | `review-code`'s first round checks the rebase; no resolution review at `pr` (`a_conflict_before_review_code_gets_no_resolution_pass`) |
| A resolution fix on a branch the remote already holds | The commits are kept, the rewrite recorded as skipped (`a_resolution_fix_on_a_published_branch_keeps_its_commits`) |
| The resolution reviewer fails | A `rerun` question that says a rerun reviews the same resolution with a fresh reviewer; nothing launches while it waits (`a_failed_resolution_pass_asks_rerun_without_carry_wording`) |
| A hand rebase, then `recheck` | The same review, with no rebaser's notes (`a_hand_rebase_after_the_refresh_question_is_reviewed`) |
| The rebaser leaves the branch alone and a newer base rebases cleanly | The conflict is dropped; no review (`a_clean_rebase_after_an_aborted_rebaser_drops_the_conflict`) |
| A restart while the resolution reviewer runs | It is polled from the record; nothing launches again (`a_restart_mid_resolution_reattaches_the_reviewer`) |
| A stage begins while the tree has work in it | The lane is left alone this stage; nothing is rebased over someone's work (`a_refresh_leaves_a_tree_with_work_in_it_alone`) |
| The rebaser stops with the rebase stopped part way | Its `rerun` question says the tree is mid-rebase and holds `pr`; the lane's base, bring-up and conflict are untouched; `rerun` while still mid-rebase asks the `refresh` question and launches nothing (`a_rebaser_that_stops_mid_rebase_holds_the_stage`) |
| The owner finishes that rebase by hand and answers `rerun` | The lane is brought up from the branch, the conflict moves to the bring-up and is reviewed; no second rebaser (`a_hand_finished_rebase_after_a_stopped_rebaser_is_brought_up`) |
| A stage begins with the worktree's `HEAD` detached off its branch | A `refresh` question says it is not on the branch; nothing is rebased or recorded and the stage waits (`a_detached_worktree_is_held_not_brought_up`) |
| A park on the mid-rebase `refresh` question, then resume | The lane is read again and asked about again; `pr` still waits (`a_park_on_the_mid_rebase_question_asks_again_on_resume`) |
| A park on a stopped rebaser's `rerun` question, then resume | The mid-rebase `refresh` question is asked; `pr` still waits (`a_park_on_a_stopped_rebasers_rerun_asks_again_on_resume`) |
| The PR's checks are red at the tree's head and the policy names a `fixer` | The fixer starts in the lane, cloned from the implementer, with the PR and the failed check names in its prompt; no question; when it stops the gate reads again and green checks pass it |
| Red checks with no fixer, or `max_fixes` spent | A `pr` decision with `recheck` and `park` |
| The provider reports the merge | The attempt completes at the merged head, the decision reads as answered `merged` by `dispatch`, and the ticket goes on (closes) |
| The queue view after `plan` replaces `investigate`, and after two tickets swap places | One card per ticket, in order, no stale card, no overlap failure |
| Plan session has no transcript yet | `workflow.start` fails; the attempt is failed and a decision, not retried |
| Plan file from an earlier attempt exists | The new attempt's own path is empty, so nothing advances |
| `project.add` fails to save | `failed` reply; attempt failed; nothing else made |
| `slots` raised in the project's live pipeline file while a ticket waits on a copy that says one | The waiting ticket starts on the next pass; the limits are the live file's, a ticket's frozen copy standing in only when the live file is unreadable (`slots_come_from_the_live_pipeline_file_not_a_tickets_copy`) |
| A gate exits 127 | The `rerun` question says a command was not found, names the lane's `setup` and the runner's `PATH` as the cause, and says to fix the pipeline, run `dispatch restart <ticket>` and answer `check`, since checking again without a restart runs the old copy |
| The pipeline is fixed, the ticket restarted at its stage, `check` answered | The new setup and the new gate run on the same attempt, with no agent and no bring-up (`a_restart_at_the_current_stage_runs_the_fixed_setup_and_gate_on_check`) |
| A restart while the stage's checks ignore TERM | The ticket stays `parking` with the intent until they are gone, then the restart applies (`a_restart_waits_for_running_checks_then_applies`) |
| `dispatch restart` on a ticket still parking without a restart | Refused with "still parking"; nothing written (`a_restart_is_refused_while_still_parking`) |
| Git refuses a ranged restart's reset | The ticket parks with git's reason and the intent kept; `resume` is refused while it is held; `dispatch restart` again finishes it (`a_reset_refused_by_git_keeps_the_intent_and_resume_refuses`) |
| A ranged restart of a ticket taken before version 12 | Refused: no head is recorded for the stage; the ticket stays active (`a_ranged_restart_is_refused_without_an_entry`) |
| A ranged restart of a ticket from pull requests | Refused: someone else's branches are never reset; only its current stage restarts (`a_restart_is_refused_ranged_on_a_pull_request_ticket`) |
| A restart, `check` fails under the new copy, then a rerun fails, with `max_reruns = 1` | Two failures under the new copy: the ticket parks; failures under the old copy are not counted (`a_checked_attempt_failing_under_the_new_copy_counts_toward_max_reruns`, `failures_under_the_old_copy_do_not_count_toward_max_reruns`) |
| `status` and `queue` list a pull-request ticket | Its source reads `pr <lane>/<n>` (lanes joined by `+`), never `#<n>`, so it cannot be mistaken for the issue of that number; piping either command into `head` ends quietly |
| `take <project> pr <lane>/<n>...` | A ticket on `<project>.pr.toml` with one PR per named lane, refused for a closed PR, an unknown or repeated lane, an unknown remote, two lanes in one repository, or a PR already on a live ticket |
| A pull-request ticket's first pass | Each PR's lane is a worktree on the PR's branch tracking the remote (a GitHub PR from its pull ref on `pr/<n>`), chosen; no branch of Dispatch's own; lanes without a PR are not cut |
| `take .. pr <remote>:<lane>/<n>` on a mirror | The clone gains that remote, the lane is checked out from it, the question shows the PR's base there, and the merge is read from that provider by number |
| The sign-off gate opens on a pull-request ticket | Each branch is fetched and fast-forwarded first; a force push parks the ticket; `proceed` leads to `pr-merged`, and the merges close the ticket |
| Two reviewers (one a command), no findings, `like = "implement"` at the same head | One pass; `implement`'s checks reused; the stage completes bound to the head; no implementer, no check run (`a_review_with_no_findings_completes_at_its_head_reusing_implements_checks`) |
| Reviewers object | `feedback.md` with each point's reviewer and id; `review-code` asks; `fix` starts a fresh implementer with the file; its commit is checked at the new head; round two opens there with the earlier file in the prompt (`findings_are_fixed_by_a_fresh_implementer_and_checked_at_the_new_head`) |
| A disputed point, no code change, the reviewer keeps it; the cap is reached | The point carries its original id, marked kept with the reason; fixed points close; `review-cap` offers accept, more, park; `accept` runs the checks at the reviewed head (not reused: a different head) and completes (`the_cap_offers_the_reviewed_head_and_accept_completes_at_it`) |
| A disputed point the reviewer withdraws; `review-code = "auto"` | No question; the fix pass runs; the point closes in pass two, which converges (`a_withdrawn_point_closes_and_the_auto_dial_fixes_without_asking`) |
| A rerun after three rounds and a failure | `carried_from` names the old attempt; round 1's reviewers are told the head it read through round 2, the settled points and the open ones under `a1/...` ids, and read only `git diff` since; the open point is still open in round 1's file and the settled one is not there (`a_rerun_review_carries_the_old_attempts_settled_and_open_points`) |
| A rerun after a fix pass answers `fixed` and its checks fail | The point is open in round 1 marked "check the fix", not settled (`a_rerun_after_a_failed_fix_carries_the_fixed_point_open`) |
| The same rerun noted "start over please" | No carry; the note is on the attempt, off the ticket, and ends the first fix pass's prompt; the rerun question names the phrase (`a_rerun_noted_start_over_reviews_the_whole_branch`) |
| Round 3 raises only a `style:` point | The round converges at the head the reviewers read with the point under "Left to the merge"; no fix pass; the checks run there and `summary.md` lists the point (`a_style_only_round_three_converges_and_leaves_the_point_to_the_merge`) |
| Round 3's style reviewer declares every point left is wording, then lists one `style:` point | The declaration is under `## Reviewer notes`, not a point; the one point is `r3/style-1` under "Left to the merge" and the only one in `summary.md` (`a_wording_declaration_is_a_note_and_the_round_leaves_one_point`) |
| Round 3's style reviewer writes only the declaration | The round converges with nothing left: the sentinel, then the note; the summary's "Left to the merge" is `None.` (`a_round_whose_only_style_feedback_is_the_declaration_converges_with_nothing_left`) |
| The plan has a "Decisions" section | The reviewers are given it as settled and not the rest of the plan; a `decided:` point holds nothing open and is listed as found but not done, in the round file and the summary (`the_plans_decisions_reach_the_reviewer_as_settled`) |
| The base moved under a branch with commits before `review-code` | The branch is rebased, `refreshed.commits` is set, round 1 is told to check the rebase, and a rerun at that base is not (`a_review_after_a_rebase_with_commits_checks_the_rebase`) |
| The same, on a lane with no recorded `base_sha` | The fork point is `from`, `commits` is set and the rebase is checked (`a_rebase_of_a_lane_without_a_recorded_base_is_checked`) |
| The same, on a lane with no recorded `base_sha` whose rebase conflicts and a rebaser resolves | The fork point read before the rebaser is `from`, not the new base (`a_rebaser_on_a_lane_without_a_recorded_base_keeps_its_fork_point`) |
| A lane with no recorded `base_sha` already caught up with commits | `from` is empty, `commits` is set and the rebase check names no range (`a_rebase_from_an_unknown_base_is_checked_without_a_range`) |
| The base moved under a branch with no commits | The branch moves; no rebase check (`a_review_after_a_clean_move_has_no_rebase_check`) |
| A command reviewer exits 2; an agent reviewer stops with no file | The round fails into `rerun`; the sibling session is killed first; neither is an approval (`a_failed_reviewer_fails_the_round_after_its_siblings_are_killed`) |
| An agent stops while its card still reads `working`, and writes its notes in a later turn | The attempt is held with no failure, no question and no kill; notes written mid-turn complete nothing until a Stop leaves the card idle (`a_stop_while_still_working_holds_the_attempt_until_the_notes_land`; idle without notes fails only after `STOP_IDLE_POLLS`: `a_stop_idle_without_notes_fails_after_the_grace`) |
| A Claude reviewer stops busy and writes later | No result, no rerun, no sibling killed while it works; its later write and Stop finish the round (`a_reviewer_that_stops_busy_and_writes_later_completes_the_round`; the implementer the same: `an_implementer_that_stops_busy_keeps_its_round`) |
| A `workflow` query the app could not answer, or a socket timeout | The review attempt keeps running and the next pass reads the run; only `no such run` fails it; a park waits until the run is confirmed paused (`a_workflow_query_the_app_could_not_answer_leaves_the_attempt_running`, `a_run_switchboard_no_longer_has_fails_the_attempt`, `parking_waits_while_the_app_cannot_say_the_run_paused`) |
| A PR reads no checks within two minutes of a push | The reading is recorded and the gate waits; past `PR_YOUNG_HEAD_MS` it is the `pr` question (`a_none_reading_soon_after_a_push_waits_then_asks`) |
| A decision marked a session that is no longer the ticket's newest (the `lanes` question's investigator, another lane's planner) | Answering clears every session the ledger still marks, so an agent held by the mark is given up on after the grace; a lost `waiting off` is sent again; a waiting request replaced by a later one is never sent after it (`an_answer_clears_every_mark_so_a_marked_agent_is_still_given_up_on`, `a_lost_unmark_is_sent_again_until_it_lands`, `a_replaced_waiting_request_is_never_sent_after_the_one_that_replaced_it`) |
| A command reviewer exits 1 with nothing on stdout | A failed reviewer (`a_command_reviewers_exit_codes_are_read_as_the_protocol_says`) |
| The tree is dirty when reviewers finish; the implementer leaves it dirty | The round's evidence is void, a `rerun` question naming the change; the fixer is nudged once past the commit wait, and a second dirty stop fails the round with `after 1 nudge` (`a_changed_tree_voids_the_round_and_a_dirty_implementer_fails_it`) |
| The implementer's response settles while its commit's hook still runs | The round waits for the tree while the session lives, then goes on from the committed head (`a_commit_that_lands_after_the_response_settles_is_not_a_dirty_tree`) |
| The head moved while `review-code` was pending | The answer is stale: the ticket parks with both heads named and nothing launches (`an_answer_for_a_moved_head_is_stale_and_parks_the_ticket`) |
| The runner lost a running command reviewer | Failed on the next pass, not started again (`a_lost_command_reviewer_is_failed_not_started_again`) |
| The socket fails mid-pass (Switchboard quit or restarted under the runner) | Nothing is parked; the pass ends with a log line and the next one goes on; the port remakes its connection and sends the request again, which the operations log makes safe |
| `close` on a parked ticket; on an active one with an open attempt; on a dirty tree; a ticket past its last stage | Each lane with its own repository, then the ticket's tree, is removed with `git worktree remove`; the branch, the ticket directory, the record and the Switchboard projects stay; the ticket leaves the queue and its card the set; pending decisions are cancelled and the session unmarked. Refused before anything is written while an attempt is open (park first) or a tree has changes. At the pipeline's end a refusal is recorded as the tree kept and the ticket still closes (`closing_by_hand_removes_the_lanes_then_the_tree_and_keeps_the_rest`, `a_close_is_refused_while_anything_runs_or_a_tree_has_changes`, `a_dirty_tree_at_the_pipelines_end_is_kept_and_removed_by_hand_later`) |
| The same issue taken again after a close, its kept branches in the clones | A kept branch with nothing beyond its base is deleted and the trees are cut; one with commits holds the ticket at the `cut`/`branch` question naming only that repository; `reuse` checks it out at its head, `fresh` renames it to `.closed-<yyyymmdd>` and cuts a new one, `park` parks (`a_retake_deletes_an_unmoved_kept_branch_and_cuts`, `a_retake_asks_about_a_kept_branch_with_commits`, `a_retake_with_one_moved_lane_of_three_deletes_the_others_and_names_only_it`, `fresh_renames_every_moved_branch_to_the_one_name_the_question_gave`); a cut that fails after `reuse` parks and a resume asks again (`a_reuse_whose_cut_fails_is_spent_and_a_resume_asks_again`); a kept branch git cannot read parks (`a_kept_branch_that_cannot_be_read_parks_the_retake`) |
| Dispatch killed at any point of a close | The ticket reads `closing` and starts nothing; the next pass finishes from the saved flags; nothing counts as done without its read-back (`a_close_cut_off_before_the_project_save_finishes_and_clears_the_set_once_answered` and its siblings) |
| User is viewing another workspace during the whole path | The window stays on it through every launch and the review start; no terminal window opens |
| A ticket reaches `deploy` in its backend lane | One gate-only attempt in that lane, no session, the command in the lane's clean tree with `DISPATCH_STAGE=deploy` and its log in `checks.log`; exit 0 binds the head and the tester is told that commit; the command ran once (`a_deploy_runs_once_at_the_lanes_clean_head_and_the_tester_is_told_the_commit`) |
| A frontend-only ticket reaches `deploy` | No attempt and no command; the tester reads `unknown (deploy skipped)`, and `tried` says the deploy was skipped and what is served (`a_deploy_for_a_lane_not_chosen_is_skipped_and_reads_as_skipped`) |
| The deploy exits 1 | A `rerun` question with `rerun` and `park`; parked and resumed, the question again and no second run; `rerun` runs attempt 2 with its own log (`a_failed_deploy_asks_rerun_or_park_and_a_resume_does_not_deploy_again_unasked`) |
| The runner restarts while the deploy runs | The attempt fails "it may have run" and asks; nothing runs again; the hold stays (`a_deploy_lost_to_a_runner_restart_is_a_question_not_a_rerun`) |
| A ticket parks while its deploy runs | `parking` until the command exits, never killed; then `parked` with no hold (`parking_during_a_deploy_waits_for_it_to_exit_then_releases_the_hold`) |
| A stage in `lane:<x>` for a ticket without that lane chosen | No context and skipped; an `each` stage with nothing chosen still parks (`a_lane_context_skips_unchosen_lanes_but_each_still_parks_with_none`) |
| A second ticket reaches `deploy` while the first holds `my-dev` | It waits with no attempt, no question and no slot, and `status` says `waiting for my-dev, held by <id> (#n)`; when the first answers `tried`, its service is stopped and removed, the hold released, and the second deploys (`a_second_ticket_waits_for_the_resource_without_a_question_or_a_slot_then_takes_it_when_tried_ends`) |
| A park or a close while a service runs that survives its first kill and then leaves its port taken | The hold is kept until the session is gone and the port binds again; the session is removed before `parked` or `closed` (`parking_and_closing_release_the_hold_after_the_service_is_gone`) |
| A `refresh` question at the stage after `tried` | The service still stops and the hold goes, so another ticket takes it (`a_refresh_question_after_tried_does_not_keep_the_hold`) |
| A stage names a lane in `needs` | The ticket parks: the in-place hold is not built (`a_stage_needing_a_lane_parks_as_not_built`) |
| The runner restarts while a ticket holds `my-dev` | The hold is read back from the record and the other ticket still waits (`a_hold_survives_a_runner_restart`) |
| `slots = 1` with a ticket holding `my-dev` at `tried` | Another ticket's tester does not start; `status` counts one running (`a_ticket_holding_a_resource_costs_a_slot_and_status_agrees`) |
| `try` begins with the frontend chosen | Its `before` runs in the frontend tree; after exit 0 a service session on the first free port, launched as `$SHELL -lc 'exec "$@"' dispatch-service env BROWSER=none PORT=3100 npm start` in that tree; no tester until it answers; the tester is told the URL (`services_start_after_before_on_a_free_port_and_the_tester_is_told_the_url`) |
| A backend-only ticket at `try` | Nothing is served and the tester reads `not served (no frontend lane)` (`a_lane_not_cut_is_not_served_and_reads_so`) |
| The first port is taken; every port is taken | The next port is used; with none free, a `service` question (`retry`, `park`) and nothing launched (`a_busy_port_is_skipped_and_no_free_port_is_a_question`) |
| The `before` exits 1 | A `service` question before any tester; `retry` makes record 2, whose `before` runs under its own key (`a_before_that_fails_is_a_question_before_the_tester_and_retry_runs_it_again`) |
| A park or a close while a `before` runs | It is waited for, never killed; `parking` or `closing` until it exits (`parking_and_closing_wait_for_a_running_before`) |
| A park whose service's port stays taken | No question before two minutes; then one `stuck` question naming the port and `lsof -i :3100`, pending in the view and kept by parking; `wait` is acted on and asks again after another limit; `released` removes the session, writes what was released, and parks with no hold (`a_park_held_by_a_busy_port_asks_stuck_and_released_parks_it`) |
| A `stuck` question asked while active, then the ticket parks | The intent to park withdraws it; a new one is asked while parking (`a_park_withdraws_a_stuck_question_asked_while_active_and_asks_afresh`) |
| A close whose `before` never exits | A `stuck` question answerable while the ticket reads `closing`; `released` kills the `before` and the ticket closes (`a_close_held_by_a_hung_before_asks_stuck_and_can_be_answered_while_closing`) |
| After `tried` the service's port stays taken | The hold stays and nothing of the next stage starts until `released`; then the other ticket takes it (`a_leaving_stop_past_the_limit_keeps_the_hold_until_answered`) |
| The runner stops between saving a service `starting` and writing its request | The launch is sent once on the next pass (`a_service_saved_but_never_sent_is_sent_once`) |
| The service never answers within `ready.within_secs` | Its session is killed and removed, then a `service` question; `retry` starts afresh (`a_probe_that_never_answers_is_a_question_and_the_service_is_stopped`) |
| `tried` answered, or parked | The service is killed, its port read free, its session removed, the hold released (`services_stop_when_tried_ends_and_on_park`) |
| The reply to a service's `session.new` is lost | Recovery finds the session; it is on the record and the process list; nothing launches twice (`a_lost_service_reply_is_found_again_not_started_twice`) |

Then, in order: `implement` with its command gate bound to a commit,
tested with one lane committing after another lane's checks finished;
`ready` and `merge` from GitHub and the human gate-only stage (built,
as above; a moved head is not yet voided); the code review stage
(`docs/review-stage-plan.md`); the queue and slots; the Orchard
pipeline with the deploy gate and the persisted `my-dev` hold (built,
as above; a head that moves after `tried` is not voided); the PTA
pipeline with its in-place hold; budget reporting; the `recommend`
dial.

## Design choices

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
