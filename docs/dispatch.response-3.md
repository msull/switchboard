# Response to review 3 of the Dispatch plan

Each code claim was checked. `prompt_planner` sends the rendered
`respond` text to the clone and nothing else (`src/core/workflow.rs`);
`add_to_working_set` returns early when the target is already pinned
and placement rejects overlaps (`src/core/action.rs`); the port listed
no removal, move or replacement command. All correct. All six items are
accepted; one is accepted with a narrower reading, marked.

## 1. The cloned planner and the copy — accepted

All three `respond` templates and all three `respond_to_user`
templates now name the absolute `{plan}` path, say it is the copy under
review, and say to edit that file and only that file, leaving the
earlier plan untouched. The "A review is a copy" rule explains why:
the clone's history names the original. The first slice gained a case
with one objection, asserting the review copy changes and the plan
attempt's file does not.

## 2. Recovery for commands that create nothing — accepted

Commands are now in three classes with their own recovery. Creations
are found by `find {op}`, and "not found means nothing ran" is scoped
to them, with the reason stated: only Dispatch removes records, and
only through its ledger. Idempotent state commands (notes, waiting,
kill, remove, pause, finalize, set sync) are simply sent again after a
lost reply. Non-replayable commands (`session.send`,
`workflow.continue`) are never repeated; a lost reply is a decision
showing what was sent. `op.status` gained `interrupted`, which
Switchboard can answer because it marks the record with the pending
effect before running it. An operation `in progress` is waited for:
the plan no longer fails a run found in `Starting` on sight, and the
first slice has a case for a Dispatch restart during the clone.

## 3. Multi-lane progress versus invalidation — accepted

Per-lane results bind to their own lane's head; joined results bind
to all. A sibling lane's commit during `implement` therefore voids
nothing, an `each` stage completes when every lane's result is bound
to its own final head, and a joined stage starts only at that
barrier with every agent stopped. Movement after a result exists
parks the ticket with a single decision listing the voided stages;
the answer authorises the reruns, superseded attempts are retired by
the cancellation sequence first, and old and replacement attempts
never overlap. The `implement` slice is to be tested with one lane
committing after another's checks finished.

## 4. PTA's manifest — accepted

The shell validation is gone. The manifest has a stated format
(`file: <path>` or `draft: <text>`), and a new "Manifests" section
makes Dispatch parse it as part of completion, failing closed on an
unreadable or empty file, a line in neither form, a missing or
non-regular file, a path escaping the root, or no valid entry.
Files are copied under their relative paths, fsynced, and recorded on
the attempt before the gate runs, the base is restored, or the hold is
released. The seven parser tests asked for are listed, draft-only
included as valid.

## 5. Quiescence across the whole hold — accepted

Every session, service and command made for a ticket is on the ticket
record until Dispatch removes it, and both cancellation and end-of-
stage release check that whole list, not the current attempt's.
Agent panes are killed as soon as completion evidence is recorded, so
nothing of a finished stage lingers. The tester's server is no longer
its own: the frontend lane declares a `serve` entry, the `try` stage
lists `services = ["frontend"]`, Dispatch starts it through a new
`service.new` command before the agent and tells the tester its
address and not to start another. The service lives until `tried`
ends and is killed through its record.

## 6. Queue view pin operations — accepted

`set.add` is replaced by `set.sync {set, items}`, one action that
makes the set's pins exactly the given list: unlisted pins removed,
listed ones placed or moved, and any overlap or foreign target failing
the whole request with no change. Dispatch computes the full layout
from the queue and each ticket's current session and sends it whole,
which is what makes replacing the investigator with the planner and
swapping two positions safe. The first slice has a case for both.

Narrow reading: `set.sync` is specified as an all-or-nothing
reconcile rather than separate remove and place commands, because
one atomic layout is simpler to reason about and cannot half-apply a
swap.
