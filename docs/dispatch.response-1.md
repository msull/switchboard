# Response to review 1 of the Dispatch plan

Every factual claim in the review was checked against the code before
answering: the built-in definition is `Plan review`
(`src/core/model.rs`, `BUILTIN_WORKFLOW`); `start_workflow` rejects an
unknown definition name, a planner that is not a Claude Code session
with a resume handle, and a relative plan path (`src/core/workflow.rs`);
definitions are installed with `SetWorkflowDefinitions`;
`SetSessionNotes` exists; working sets accept only targets in their own
workspace (`target_in` in `src/core/action.rs`); `PinTarget` is
sessions and files only; `Converged` and `Finalized` are distinct run
states. The review was right on all of them.

Most items are accepted and the plan is rewritten around them. The
rejections and partial acceptances are marked.

## 1. Durable execution and recovery — accepted

Added "Records and recovery": one record per ticket with source
snapshot, pipeline version, lanes, attempts (each with a request id,
state `intended` / `launched` / `lost` / `failed` / `done`, record ids,
outputs, and for command gates the commit the result is bound to),
decisions, holds and queue rank; atomic writes and a single-writer lock
file. The restart reconcile is spelled out per attempt state, including
the two cases raised: a launch sent but not recorded is found by the
attempt tag Dispatch puts in every session's notes and command name; a
launch never sent is `lost` and needs a decision. A command gate with no
result is never re-run because it may have run. The distinction the
review asked for is now explicit: a restart observes, continues only
what is already running, and never launches to finish an interrupted
attempt; new launches come only from the scheduler's normal pass.

## 2. Control port operation identity — accepted

The port is now its own wire contract of named commands and queries,
validated before dispatch, not the `AppAction` enum. Every request has
a client `req` id; replies are `accepted`, `persisted`, `launched` (with
the ids made) or `failed` (with the notice or effect error). Repeated
`req` ids replay the earlier reply within a Switchboard run; across a
restart the notes tag is the deduplication. Added the missing
operations: `session.notes`, `session.waiting`, `command.run`,
`space.new`, `set.new`, `set.add`, `workflow.definitions.install`,
`workflow.finalize`, `workflow.remove`, and queries for spaces, sets,
tagged sessions and a file probe. Creation commands take an explicit
space or project and skip the core's select-the-new-thing step, so
background work never moves the user's window. The claim that deriving
serde would implement the contract is withdrawn.

## 3. Queue versus workspace privacy — accepted

The per-ticket workspace is dropped. All of a project's tickets share
one workspace so one working set can show them, and the plan now says
plainly that workspaces were never a filesystem boundary between agents.
The queue is a Dispatch record edited by `dispatch queue`; the working
set is a view Dispatch re-pins in queue order, because a grid has no
order of its own. A stable ticket card in Switchboard is named as later
work rather than a precondition.

## 4. Human decisions — accepted

Decisions are records with ids, options, recommendation, and a
pending / answered / cancelled state, answered by `dispatch decide` and
listed by `dispatch decisions`. The current session (or a placeholder
shell per ticket when no agent is running, which covers `lanes`, `tried`
and `merge`) is marked waiting through the new `session.waiting`
command. Permission and confirmation gates are now different things:
a confirmation gate has no dial, is verified from the provider where
possible (`pr-merged`), and otherwise completes only on your word.
Rejection cancels the attempt, releases holds and parks the ticket.

Partially accepted: `recommend` and its delay. The plan names the value
and says outright that it is not designed, because it needs a persisted
deadline and a restart rule and nothing in the first slices uses it.
Designing it now would be speculation.

## 5. Review integration — accepted, with one rejection

Accepted: a reviewer operator is now a complete workflow definition in
the pipeline file (every prompt template and the sentinel), installed
under a `Dispatch: ` prefix through `workflow.definitions.install`.
The plan attempt's session is the review's source, so the planner is
constrained to Claude Code, and the `{plan}` path is absolute under
Dispatch's directory. PTA's draft is now written to `{plan}` as
markdown so it is the reviewed subject; rendering happens after
finalization. Who finalizes is explicit: the `finalize` decision after
`Converged`, `ask` by default.

Rejected: intercepting reviewer objections as structured decisions.
The workflow compares only the reviewer's sentinel and the rounds are
prose, as the review says. Making objections machine-readable would be
a new workflow mechanism in Switchboard, and the human already reads
the rounds at the `finalize` decision. The `review_objection` dial is
removed rather than promised.

## 6. Stage completion versus polling — accepted

Outputs are per attempt and per context under
`<ticket>/<stage>/<attempt>/<context>.md`, so a stale or shared file
cannot pass. Completion needs two facts: the session stopped and the
output settled. Command gates run once per attempt, after the agent
stops, as a command record named for the attempt; the result is bound
to the head commit and discarded if the head moves. Joined stages
require every lane's agent stopped and every worktree clean, and a
dirty tree is a decision.

## 7. Execution context — accepted

Added the root context: a Switchboard project at the pipeline's root
per ticket, used by `investigate` and by joined stages, which are given
every lane's path and head commit. Delta's investigation now says
`context = "root"`. The `try` stage was split: `deploy` runs in
`lane:backend` and is skipped when no backend lane was cut, in which
case the frontend is tried against what `sully-dev` already runs and
the `tried` decision says so. The tester links and starts the
frontend worktree itself; the original checkout's `webserver` service
is no longer assumed to serve the ticket's code.

## 8. The Delta deploy gate — accepted

One owner: Dispatch's command gate deploys, after running
`inv link-env --env-name sully-dev` in the worktree in the same
command, so the target is established rather than inherited. The lane's
`setup` links too. The deployed commit is recorded on the attempt, the
tester is told it and told not to deploy, and the `tried` decision
shows the tester's evidence file with that commit. The plan now says
that guidance is advisory and that the real fix, if needed, is
credentials that cannot reach other environments. The claim that
raising a resource count gives parallel environments is withdrawn:
that needs instances with a bound `link-env` name each, named as later
work.

## 9. Resource holds and capacity — accepted, with one rejection

Holds are on the ticket record under the writer lock, released on
completion, rejection or cancellation, and never on restart. `slots`
counts tickets with a running attempt or a held resource; human gates
cost no slot. PTA's in-place lane is an implicit repository resource
held from `draft` through `render` and released once the branch is
committed and the tree clean, which is what lets three finished drafts
wait while a fourth is written. A dirty tree before taking the hold is
a decision. Backpressure: at `waiting_on_me`, nothing new starts in
that project.

Rejected: covering manual deploys with the lock. Dispatch cannot
observe a deploy you run by hand, so promising to lock against it
would be false. The plan says holds cover Dispatch tickets only and
that `dispatch status` shows them so you can look first.

## 10. PTA's commit gate — accepted

The writer commits its own outputs on the ticket branch. The gate
verifies rather than performs: a clean tree and a branch ahead of its
base. Mail drafts are identified by their Drafts location written to
the notes. Commands are fixed argv from the file; template values reach
them only as environment variables, never spliced into shell text.

## 11. Source identity and PR evidence — accepted

Task-file identity is a hash of the normalised line text without the
marker; moving a line keeps the ticket, editing it makes a new
identity; `dispatch retake` is the explicit reopen and the taken text
is snapshotted on the record. GitHub identity is repository plus issue
number. `pr-checks` is bound to PR id, provider, and head revision,
with pending, green, red, missing, and lookup-error readings spelled
out; pending waits, and green at an old head does not count. A lane
whose branch moves after passing returns to `ready`, and a later joined
stage becomes a decision.

## 12. Budget — accepted

Budgets are reporting, not enforcement: token counts times a `rates`
table per model in the pipeline file, counted from the attempt's start
so cloned history is not double-charged, unknown shown as unknown.
Crossing a budget blocks the next launch for the ticket through a
decision and never stops a running agent. Operator budgets are per
ticket across the operator's attempts.

## 13. First-slice acceptance — accepted

The first slice is the Switchboard pipeline only, ending at the
`finalize` decision answered by `dispatch decide`. An acceptance table
covers the seven cases the review listed, each to be run against a fake
Switchboard on the socket and the real one. The order of later slices
now names gate revision binding, persisted holds, and decision
submission.
