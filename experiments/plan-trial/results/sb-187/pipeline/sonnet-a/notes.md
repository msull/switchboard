# Issue #187: review rerun starts from the plan stage's original output

Notes only; no code changed. Paths are in this worktree. I read CLAUDE.md and
docs/lessons.md in full; for docs/design.md I read by section (see "Docs") rather than all 3678 lines.

## On-disk record and rehydration (first)

A review attempt is a `Ticket` record in the data directory, with one
`Attempt` per try (`dispatch/src/ticket.rs`). Relevant fields:
- `Attempt.artifacts`: map name -> path. A review attempt records its
  reviewed copy as `plan` -> `<attempt dir>/plan.md` (written at
  `scheduler.rs:7470`, recorded at `:7493`).
- `Attempt.revisions` (`ticket.rs:350`, `Revision` at `:510`): which rounds the
  owner opened with `revise` (round, by, at_ms). It does not hold the text.
- The owner's `revise` note: stored on the `finalize` `Decision`
  (`DecisionState::Answered { note, .. }`), and written by `revise()`
  (`scheduler.rs:7565`, file at `:7616`) as the round file
  `plan.feedback-<round>.md` beside the attempt-1 copy (`report::round_file`,
  `report.rs:156`).
- The revised plan: the planner edits the reviewed copy in place, so the
  revision is the content of `review/1/joined/plan.md`.

Rehydration is the problem. A new attempt is built from the record by
`start_workflow` (`scheduler.rs:7436`). It asks `review_subject(t, "plan")`
(`scheduler.rs:10534`), which walks `t.attempts` newest first and keeps only
`state == AttemptState::Complete` attempts that have a `plan` artifact. Attempt
review/1 is `Failed { "review run gone: no such run" }` (set at
`scheduler.rs:7761`, via `fail_attempt` `:7148`). So it is skipped, and the
newest Complete attempt with `plan` is plan/1. `start_workflow` then copies
plan/1's file over the new attempt's `plan.md` (`:7471`, `std::fs::copy`).

So the revision is not lost on disk. `review/1/joined/plan.md` and
`plan.feedback-N.md` are still there. They just aren't reachable from the
record as "the plan", because everything keys on `Complete`.

## Root cause

"Newest plan the stage holds" is computed as "newest Complete attempt's
artifact", the same rule `Ticket::input_where` uses for agents
(`ticket.rs:1452`). That rule suits agent stages, where a failed attempt's
output is untrustworthy. A review attempt is different: its copy is edited in
place, and the owner's `revise` is applied to it, before the attempt is
Complete (Complete happens only on `finalize`). A review that fails after a
`revise` therefore holds the newest plan but is never Complete.

Second part: the source *session* is paired with the path in the same
`review_subject` tuple (`a.session`, `:10538`). A review attempt's `session`
is the planner clone (`poll_workflow`, around `scheduler.rs:7790`, "The card for the
ticket is the planner's"), not the plan author. So the fix cannot just take the
review/1 tuple wholesale: it needs the path from the review copy and the
session from the plan attempt (the one the planner is cloned from).

## What `show` prints

`ticket_paths` (`dispatch/src/serve.rs:539`) -> `shown_lane_files`
(`scheduler.rs:10903`) -> `lane_files_by` (`:10908`) -> `Ticket::shown_where`
(`ticket.rs:1476`), which keeps Complete attempts plus open Workflow attempts.
A Failed review/1 is neither, so only review/2's copy (open) is listed.
`main.rs:960-975` (`show`) and `print_lane_files` (`main.rs:896`) print it as
`plan (...)` with `(reviewed copy, round N, review open)`; plan rounds print
from `p.plan_rounds`, which comes only from the latest review attempt. Nothing
tells the reader a prior revision exists or that attempt 1's `plan.md` /
`plan.feedback-N.md` are now orphaned.

## The rerun question

`rerun_ask` (`scheduler.rs:9335`) builds "review (joined) attempt 1 failed:
review run gone: no such run. Run it again?" plus `rerun_carries` (`:9382`),
which has text only for code review ("carries this one's settled and open
points") and resolution review; for the plan review it adds nothing, so it
never says which plan it will review. `rerun_note` (`:8988`) carries only the
owner's note on the *rerun* decision, not the earlier `revise` note.

## Candidate fix (not done)

1. In `review_subject`, or a sibling used by `start_workflow`, prefer the
   newest Failed or Cancelled *workflow* attempt of the same stage whose
   copy holds an owner revision (`!revisions.is_empty()`), or simply the newest
   copy among review attempts of that stage that has a `plan` artifact and
   `revisions` non-empty, over the plan stage's output. Keep `session` from the
   plan attempt. Check that the file still exists; otherwise fall back and
   say so in the question.
2. `rerun_carries`: for a plan review, name the copy the next attempt will
   review ("reviews review/1's revised copy at <path>" or "the plan stage's
   output at <path>"). Needs `t` and `p`, so it can call the same selector.
3. `show`: add to `LaneFile` (`dispatch-control/src/lib.rs:586`, additive)
   a note, or list prior attempts' copies as "prior revision: <path>" when more
   than one attempt has a `plan.md`. Changing `LaneFile` is a contract
   extension, so additive with `#[serde(default)]`.
4. Optionally carry the owner's revise round text: copy
   `plan.feedback-N.md` forward or, simpler, leave it where it is and list it.
   The owner's note must survive a runner failure "like an answered decision
   does"; today the Decision keeps it, the round file stays on disk, only the
   new attempt does not see it.
No schema change is needed for 1-2 (reads existing fields). `SCHEMA_VERSION`
bump only if a new serialized field is added.

## Tests to add or touch

- Core/scheduler test next to `lane_plans` tests (`scheduler.rs` ~12439-12870),
  and in `dispatch/tests/first_slice.rs`, near
  `a_run_switchboard_no_longer_has_fails_the_attempt` (`:3809`): plan, review,
  `revise`, fail with `NO_SUCH_RUN`, `rerun`; assert the new attempt's
  `WorkflowStart.plan` copy equals the revised bytes and the `source` session
  is the plan author's. Fake in `dispatch/tests/support/mod.rs:690`.
- `rerun_ask` text test; a `show`/`ticket_paths` test for the prior-revision line.
- Existing tests that assert the review reads plan/1 after failure need
  checking: run `cargo test --locked -p dispatch` before and after.

## Docs to update when fixed

`docs/design.md` "Plan review workflow" (line 1000) and the line-1787 note
about `no such run`; `docs/dispatch.md` around 2497 and the table at 3078
(add the new test name); `CHANGELOG.md` Unreleased line.

## Not read in full

docs/design.md and docs/dispatch.md beyond the sections above;
`review_round` and `plan_rounds` in serve.rs; the control-port side that
replies `no such run` (not needed to fix this).
