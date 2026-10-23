# Plan: issue #187, a plan review rerun starts from the owner's revision

Line numbers are from the worktree. "Facts" is `facts.md`; S is its Settled
section, U its Unresolved, O its For the owner.

## Where this departs from the issue

- The issue says the `revise` note "should survive a runner failure the
  same way an answered decision does". It already does, and nothing in
  the failure path touches it. The note is in the decision's
  `DecisionState::Answered { note }` and in `<subject>.feedback-<round>.md`
  beside the copy. `fail_attempt` (`dispatch/src/scheduler.rs:7148`) touches no
  file, and the app deletes none (S, "The app does not touch the copy").
  The plan adds no storage for the note. It names the feedback file in the
  rerun question so the owner can find it.
- The issue's "newest plan the stage holds" has no counterpart in the
  code: `review_subject` (`scheduler.rs:10534`) takes the newest
  `Complete` attempt that wrote `plan`, and a `revise` edits the copy of
  an attempt that is never `Complete` until `finalize`. The plan adds the
  one case the issue is about (a failed attempt holding a finished owner
  revision), not a general rule.
- The investigation notes say a non-empty `Attempt.revisions` shows that
  the copy holds an owner revision. It does not (S, "Partial or finished
  revision"): `revise` pushes the `Revision` before sending the round and
  keeps it when the app refuses the round (`scheduler.rs:7607-7647`;
  only the feedback file is removed). The field's own doc comment at
  `dispatch/src/ticket.rs:348-349` says "popped when Switchboard refuses
  it", which the code does not do. The plan corrects the comment and does
  not change the behaviour. The evidence of a finished revision is the
  response file the planner writes.
- Not fixed, because the issue does not ask and it needs the app: the
  rerun's planner is still a clone of the plan stage's original session
  (`source`, S "`source` and the new planner"), so it has not seen the
  note either. It reviews the revised file, which contains the revision.
  Named under follow-ups.

## The record

One additive field. `Attempt.revised_from: Option<u32>` in
`dispatch/src/ticket.rs`, `#[serde(default)]`: for a plan review attempt, the
number of the earlier attempt of the same stage and context whose
owner-revised copy this attempt's copy was made from; none when it was made
from the plan stage's output. Written when the attempt is made in
`start_workflow` and never changed, as `carried_from` is. It is a decision
made once, so `show` and a restarted runner report what was done rather than
re-deriving it from files that may have changed. `new_attempt`
(`scheduler.rs:10277`) sets it `None`.

`dispatch/src/store.rs`: `RECORD_VERSION` (`store.rs:26`) goes from the value
there at implementation time, plus one. Add the matching comment-only step in
`migrate` (`store.rs:452`), as the other steps that only add a defaulted
field do. Nothing is transformed. The store already refuses to write a newer
version. `crate::model` (`SCHEMA_VERSION`, `src/core/model.rs:12`) is not
touched. `docs/dispatch.md` has its per-version list near line 393: add the
paragraph there.

Rehydration: `workflow_stage` re-reads the ticket every pass, and the chosen
copy is `attempts[n].artifacts[subject]` plus `revised_from`. A runner
restart between the `rerun` answer and the launch re-derives the choice with
the same function, from files that still exist (S: the app removes nothing).

## The signals

No new signal and nothing new from outside. The one new read is whether a
response file exists beside a failed attempt's copy
(`report::response_file`, `dispatch/src/report.rs:202`).

Choosing the plan: replace `review_subject` (one caller, `start_workflow`
at `scheduler.rs:7465`; S "Plan readers") with
`plan_to_review(t, stage, ctx, subject)`, returning an owned struct
`{ path, session, revised_from: Option<u32>, skipped: Option<(u32, u32)> }`
(owned `PathBuf`s because `start_workflow` mutates `t.attempts` right after,
and a struct holding references into `t` would still be borrowed then).

Walk `t.attempts` newest first by `Vec` position:

1. A `Complete` attempt whose `artifacts` has `subject` ends the walk. Its
   `session` is the `source`, as today. Its path is the plan, unless step 2
   found a revised copy earlier in the walk.
2. A `Workflow` attempt of this `stage` and `ctx`, `Failed` or `Cancelled`,
   with `artifacts[subject]` a file on disk and a non-empty `revisions`, is a
   candidate. Take the highest `Revision.round` whose feedback file
   (`report::round_file`) still exists, since a refused round's file was
   removed. No such round: not a candidate. Its response file exists: the
   revision is finished, and this is the revised copy (first one wins).
   Otherwise the planner clone may still be editing, so record it in
   `skipped` as `(attempt n, round)` and keep walking.
3. Anything else is passed over.

Why these choices:
- U2 (is a failed copy ever mid-edit) is unresolved and nothing here can
  answer it, so the default does not assume either way. An unfinished
  revision is not used; the plan stage's output is, and the question says
  why. The enhancement, the revised copy, is only taken when the response
  file proves it finished. Both branches give a working rerun.
- U1 (can the plan stage rerun after review/1 failed): ranking by `Vec`
  position makes a later `Complete` plan win either way, because the walk
  stops at it before reaching review/1. Right in both branches (a fresh plan
  supersedes an old revision). A unit test pins it.
- O1 (was the file intact on the owner's disk): a missing or non-file copy
  fails the `is_file` check in step 2 and the walk falls back to the plan
  stage's output. Same code whichever the owner reports.
- Only owner revisions are carried, not the reviewer-round edits a failed
  attempt's planner made. Those are not owner input, and today's behaviour
  of starting again from the plan stage's output stays for them. A later
  failure without a new `revise` therefore still reaches review/1's
  revision (review/2 has no `revisions` and is passed over).

## The actions and effects

No new `AppAction` or `Effect`. The wire to Switchboard is unchanged:
`start_workflow` copies `plan_to_review().path` to the new attempt's
directory (`scheduler.rs:7466-7471`) and sends the same `Body::WorkflowStart
{ source, plan, .. }`. Add the call's `revised_from` to the attempt it pushes
(`:7480`) and log one `info` line naming the file chosen and, when
`skipped` is set, the unfinished revision passed over.

`rerun_carries` (`scheduler.rs:9382`) returns `String` instead of
`&'static str` and gains an arm for a `Workflow` attempt (today it returns
"" there). Both callers format it already (`:7266`, `:9373`). It calls
`plan_to_review` with the attempt's stage and context, and the subject name
from `a.artifacts.keys().next()`. It adds one sentence:
- revised: "The rerun reviews the owner-revised copy from `<stage>/<n>` at
  `<path>`; your note for round `<r>` is at `<feedback file>`."
- skipped: "`<stage>/<n>` holds a revision (round `<r>`) the planner had not
  finished, so the rerun reviews the plan stage's output at `<path>`; your
  note is at `<feedback file>`."
- otherwise: "The rerun reviews the plan stage's output at `<path>`."
- an `Err` from `plan_to_review` gives "", as now.

The question is a snapshot (`ensure_decision`, `scheduler.rs:3225`, dedups on
stage, name and attempt, not on text), so it states what the choice is when
asked. `revised_from` on the launched attempt is what `show` reports.

Also fix the `revisions` doc comment (`ticket.rs:348`): a refused round
leaves the `Revision` and removes only the feedback file.

## The UI

CLI `show` only. The wire type is additive.

- `dispatch-control/src/lib.rs:586`: `LaneFile` gains `prior:
  Option<PriorCopy>`, with a new `PriorCopy { n: u32, path: PathBuf }`
  (`Default`, `#[serde(default)]` like its neighbours). `LaneFile` is
  `#[serde(default)]`, so older readers and the app, which reads only
  `plan_reviewing`, `plan_reviewed`, `plan_round` and `plan_stage`
  (`src/ui/ticket.rs:427-431`), are unaffected (S, "Wire types").
- `ticket_paths` (`dispatch/src/serve.rs`, the `plan_files` build near the
  `LaneFile { .. }` literal) fills `prior` from the attempt's
  `revised_from`: that attempt's `artifacts[subject]`.
- `print_lane_files` (`dispatch/src/main.rs:896`) prints, under the plan line
  and whether or not the review is still open: `(started from the owner's
  revision in <stage>/<n>: <path>)`. This answers "which is the reviewed copy
  and which is a prior revision" for the case in the issue: the listed path is
  the reviewed copy, and the line gives the revision it began from.
- The ticket page's Plan tab in the app and the Swift client are not changed.
  Added to the design.md gap list below. (`grep planFiles|plan_files client/`
  finds nothing, S.)

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
   pointing at review/1's copy.
2. Unfinished: same without the response file. Attempt 2's copy equals
   `plan/1`'s output, `revised_from` is none, the question says the revision
   was unfinished.
3. Refused round: the fake refuses `workflow.object`, so the feedback file
   is gone and the `Revision` stays; the rerun starts from the plan stage's
   output without the "unfinished" sentence.
4. The existing test and its siblings: no revise, so the copy is the plan
   stage's output. Update any assertion on the exact rerun question text;
   the question now ends with the "reviews the plan stage's output" sentence.

Unit, in `scheduler.rs` beside the plan-reader tests (about `:12449`):
`plan_to_review` on built tickets: (a) a `Complete` plan only; (b) revised
and finished failed review before it; (c) a `Complete` plan attempt appended
after the failed review wins (U1); (d) review/2 failed with no revisions
after a finished review/1: review/1's copy is chosen again; (e) a
`Cancelled` attempt counts as a `Failed` one; (f) the copy file missing
falls back (O1). `store.rs`: a version-28-era record (use the previous
`RECORD_VERSION` fixture style at `:1104`) loads with `revised_from` none and
is written at the new version. `serve.rs` tests (about `:1590-1710`):
`ticket_paths` sets `prior`. A `main.rs` test for `print_lane_files` if one
exists for the `reviewing` line; otherwise cover it through the integration
test's `show` output.

## Docs

- `docs/dispatch.md` near `:393`: the version paragraph.
- `docs/design.md`: the plan review rerun paragraph (`:1783-1790`, "a rerun
  kills the old run's agents and starts a fresh run"): add that the fresh
  run starts from the failed attempt's owner-revised copy when its
  response file shows the revision finished. Under the Dispatch ticket page
  section (`:2135`) list the gap: the Plan tab does not show `prior`.
- `README.md`: no new file, so no Layout change; no new environment
  variable.
- `CHANGELOG.md` under Unreleased: "A plan review rerun after a failed
  attempt starts from the owner's `revise` copy when the planner finished it,
  and the rerun question and `show` say which copy it reviews."

## One commit

Message: `Dispatch: a plan review rerun starts from the failed attempt's owner-revised copy (#187)`

Contract additions (copy into the commit message body):
- `dispatch::ticket::Attempt.revised_from: Option<u32>`, `#[serde(default)]`;
  dispatch `RECORD_VERSION` bumped (value in `store.rs` at implementation
  time, plus one) with a defaulted-field step in `store::migrate`.
- `dispatch_control::PriorCopy { n: u32, path: PathBuf }` and
  `dispatch_control::LaneFile.prior: Option<PriorCopy>`, additive on the wire.
- Internal: `plan_to_review` replaces `review_subject`; `rerun_carries`
  returns `String`. No `AppAction`, `Effect`, port or `model.rs` change.

## For the owner, after this lands

- The rerun's planner is a clone of the original planner and has not seen the
  `revise` note (S). Giving it the note means changing what
  `workflow.start` sends or clones, in the app. Needs a spike and a contract
  change; not in this commit.
- The app's Plan tab does not show `prior` yet.
- O1 and U2 are unanswered. The plan works either way; the owner's data
  directory would show whether any real failure left a half-written copy, and
  if so whether the planner should be stopped when an attempt fails.
