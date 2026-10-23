# Plan: issue #187, a plan review rerun starts from the owner's revision

Line numbers are from the worktree. "Facts" is `facts.md`; S is its Settled
section, U its Unresolved, O its For the owner.

## Where this departs from the issue

- The issue says the `revise` note "should survive a runner failure the
  same way an answered decision does". It already does (S, "The app does not
  touch the copy"). The plan adds no storage for the note. It names the
  feedback file (`<subject>.feedback-<round>.md`) in the rerun question so the
  owner can find it.
- The issue's "newest plan the stage holds" has no counterpart in the
  code: `review_subject` (`scheduler.rs:10534`) takes the newest
  `Complete` attempt that wrote `plan`, and a `revise` edits the copy of
  an attempt that is never `Complete` until `finalize`. The plan adds the
  one case the issue is about (a failed attempt holding a finished owner
  revision), not a general rule.
- The investigation notes say a non-empty `Attempt.revisions` shows that
  the copy holds an owner revision. It does not (S, "Partial or finished
  revision"): `revise` pushes the `Revision` before sending the round and
  keeps it when the app refuses the round (`scheduler.rs:7607-7647`). The
  plan corrects the `revisions` doc comment (`dispatch/src/ticket.rs:348`) and
  does not change the behaviour. The evidence of a finished revision is the
  response file the planner writes.

## The record

One additive field. `Attempt.revised_from: Option<u32>` in
`dispatch/src/ticket.rs`, `#[serde(default)]`: for a plan review attempt, the
number of the earlier attempt of the same stage and context whose
owner-revised copy this attempt's copy was made from; none when it was made
from the plan stage's output. Written when the attempt is made in
`start_workflow` and never changed, as `carried_from` is, so `show` and a
restarted runner report what was done. `new_attempt` (`scheduler.rs:10277`)
sets it `None`.

`dispatch/src/store.rs`: `RECORD_VERSION` (`store.rs:26`) goes from the value
there at implementation time, plus one. Add the matching comment-only step in
`migrate` (`store.rs:452`), as the other steps that only add a defaulted
field do. `crate::model` (`SCHEMA_VERSION`, `src/core/model.rs:12`) is not
touched. `docs/dispatch.md` has its per-version list near line 393: add the
paragraph there.

## The signals

No new signal and nothing new from outside. The one new read is whether a
response file exists beside a failed attempt's copy
(`report::response_file`, `dispatch/src/report.rs:202`).

Choosing the plan: replace `review_subject` (one caller, `start_workflow`
at `scheduler.rs:7465`; S "Plan readers") with
`plan_to_review(t, stage, ctx, subject)`, returning an owned struct
`{ path, session, revised_from: Option<u32>, skipped: Option<(u32, u32)> }`.

Walk `t.attempts` newest first by `Vec` position:

1. A `Complete` attempt whose `artifacts` has `subject` ends the walk. Its
   `session` is the `source`, as today. Its path is the plan, unless step 2
   found a revised copy earlier in the walk.
2. A `Workflow` attempt of this `stage` and `ctx`, `Failed` or `Cancelled`,
   with `artifacts[subject]` a file on disk and a non-empty `revisions`, is a
   candidate. Take the highest `Revision.round` whose feedback file
   (`report::round_file`) still exists, since a refused round's file was
   removed. No such round: not a candidate. Its response file exists: the
   planner has answered that round, and this is the revised copy (first one
   wins). Otherwise no response file was found, so record it in `skipped` as
   `(attempt n, round)` and keep walking.
3. Anything else is passed over.

Why these choices:
- U2 (is a failed copy ever mid-edit) is unresolved and nothing here can
  answer it, so the default does not assume either way. A round with no
  response file is not used; the plan stage's output is, and the question
  says what was found. The revised copy is taken when the planner has
  answered the round. That is weaker than "finished": the respond prompt
  (`src/core/model.rs:279-284`) says to accept items by updating the plan but
  does not order the edit before the response, and the app counts a round
  responded when the response file appears (`src/core/workflow.rs:663-668`),
  so the copy taken may lack the planner's last edit. The implementer's first
  check is to read that template and code, confirm the ordering gap, and say
  so in the `dispatch.md` paragraph and the `design.md` text. Both branches
  give a working rerun.
- U1: pinned by test (c).
- O1: pinned by test (f).
- The copy carried is the failed attempt's whole file, not the owner's edit
  alone: the owner round is round N+1 on the same copy (`revise` uses
  `view.round + 1`, `scheduler.rs:7627`) and nothing separates it from the
  reviewer rounds. A later failure without a new `revise` still reaches
  review/1's copy (review/2 has no `revisions` and is passed over).

## The actions and effects

The same `WorkflowStart`, with `plan_to_review().path` copied
(`scheduler.rs:7466-7471`). Add the call's `revised_from` to the attempt it
pushes (`:7489-7497`) and log one `info` line naming the file chosen and, when
`skipped` is set, the revision with no response file that was passed over.

`rerun_carries` (`scheduler.rs:9382`) returns `String` instead of
`&'static str` and gains an arm for a `Workflow` attempt (today it returns
"" there). Both callers format it already (`:7266`, `:9373`). It calls
`plan_to_review` with the attempt's stage and context, taking the subject from
the attempt's `artifacts` only when that map has exactly one key (otherwise
""). It adds one sentence:
- revised: "The rerun reviews the copy from `<stage>/<n>` at `<path>`, which
  holds that attempt's edits up to round `<r>`, your `revise` round; your
  note for it is at `<feedback file>`."
- skipped: "`<stage>/<n>` has a revision (round `<r>`) but no response file
  was found for it, so the rerun reviews the plan stage's output at `<path>`;
  your note is at `<feedback file>`."
- otherwise: "The rerun reviews the plan stage's output at `<path>`."
- an `Err` from `plan_to_review` gives "", as now.

## The UI

CLI `show` only.

- `dispatch-control/src/lib.rs:586`: `LaneFile` gains `prior:
  Option<PriorCopy>`, with a new `PriorCopy { n: u32, path: PathBuf }`
  (`Default`, `#[serde(default)]` like its neighbours).
- `ticket_paths` (`dispatch/src/serve.rs`, the `plan_files` build near the
  `LaneFile { .. }` literal) fills `prior` from the attempt's
  `revised_from`: that attempt's `artifacts[subject]`.
- `print_lane_files` (`dispatch/src/main.rs:896`) prints, under the plan line
  and whether or not the review is still open: `(started from the copy in
  <stage>/<n>: <path>)`.

## The tests

Integration, `dispatch/tests/first_slice.rs`, next to
`a_failed_review_run_fails_the_attempt_and_a_rerun_starts_a_new_run` (`:3257`)
using `at_finalize`, `revise` (`:3417`) and `review_run_fails` (`:3249`):

1. Revised then failed then rerun: after `revise`, the test plays the
   planner (edit the copy, write `response_file(copy, 2)`), fails the run,
   answers `rerun`. Assert the attempt-2 copy has the revised text,
   `revised_from == Some(1)`, the `WorkflowStart` plan is the attempt-2 copy,
   its `source` is the plan stage's session, and the rerun question names
   review/1's copy and the feedback file. `show --json` has `prior`
   pointing at review/1's copy, and the `show` text has the `started from`
   line (this covers `print_lane_files`).
2. No response file: same without it. Attempt 2's copy equals `plan/1`'s
   output, `revised_from` is none, the question says no response file was
   found.
3. Refused round: the fake refuses `workflow.object`, so the feedback file
   is gone and the `Revision` stays; the rerun starts from the plan stage's
   output without the "no response file was found" sentence.

Unit, in `scheduler.rs` beside the plan-reader tests (about `:12449`):
`plan_to_review` on built tickets: (b) a failed review with a response file
before it, where a `Cancelled` attempt counts as a `Failed` one; (c) a
`Complete` plan attempt appended after the failed review wins (U1); (d)
review/2 failed with no revisions after a finished review/1: review/1's copy
is chosen again; (f) the copy file missing falls back (O1). `store.rs`: add a
previous-version fixture test like the neighbours: the record loads with
`revised_from` none and is written at the new version. `serve.rs` tests
(about `:1590-1710`): `ticket_paths` sets `prior`.

## Docs

- `docs/dispatch.md` near `:393`: the version paragraph.
- `docs/design.md`: the plan review rerun paragraph (`:1783-1790`, "a rerun
  kills the old run's agents and starts a fresh run"): add that the fresh
  run starts from the failed attempt's copy, with all its edits, when the
  planner has written a response to the owner's round, and that the copy is
  not proven final (U2). Under the Dispatch ticket page section (`:2135`) list
  the gap: the Plan tab does not show `prior`.
- `CHANGELOG.md` under Unreleased: "A plan review rerun after a failure
  starts from the failed attempt's copy when the planner answered the owner's
  `revise` round."

## One commit

Message: `Dispatch: a plan review rerun starts from the failed attempt's copy with the owner's revision (#187)`

Contract additions (copy into the commit message body):
- `Attempt.revised_from` (dispatch `RECORD_VERSION` bumped, value in
  `store.rs` at implementation time plus one)
- `PriorCopy`
- `LaneFile.prior`

## Follow-ups for the owner

The rerun's planner is still a clone of the plan stage's original session
(S, "`source` and the new planner"), so it has not seen the `revise` note;
fixing that is an app change and a spike. The owner's data directory settles
O1 and U2.
