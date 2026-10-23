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
   planner has answered that round, and this is the revised copy (first one
   wins). Otherwise no response file was found, so record it in `skipped` as
   `(attempt n, round)` and keep walking.
3. Anything else is passed over.

Why these choices:
- U2 (is a failed copy ever mid-edit) is unresolved and nothing here can
  answer it, so the default does not assume either way. A round with no
  response file is not used; the plan stage's output is, and the question
  says what was found. The enhancement, the revised copy, is taken when the
  planner has answered the round. That is weaker than "finished": the respond
  prompt (`src/core/model.rs:279-284`) says to accept items by updating the
  plan but does not order the edit before the response, and the app counts a
  round responded when the response file appears (`src/core/workflow.rs:663-668`).
  A planner that writes the response first is still editing with the file
  present, so the copy taken may lack its last edit. That is U2 again, so the
  plan claims no more than the file shows. The implementer's first check is
  to read the respond template and that app code, confirm the ordering gap,
  and say so in the `dispatch.md` paragraph. Both branches give a working
  rerun.
- U1 (can the plan stage rerun after review/1 failed): ranking by `Vec`
  position makes a later `Complete` plan win either way, because the walk
  stops at it before reaching review/1. Right in both branches (a fresh plan
  supersedes an old revision). A unit test pins it.
- O1 (was the file intact on the owner's disk): a missing or non-file copy
  fails the `is_file` check in step 2 and the walk falls back to the plan
  stage's output. Same code whichever the owner reports.
- The copy carried is the failed attempt's whole file, not the owner's edit
  alone. The planner clone edits that one file in every round, reviewer
  rounds and the owner round alike (the owner round is round N+1 on the same
  copy: `revise` uses `view.round + 1`, `scheduler.rs:7627`), and nothing in
  facts.md or the tree separates the two kinds of edit. So the rerun reviews
  everything that attempt's planner had done up to the answered owner round.
  The attempt is chosen by having an owner revision; what is carried is all
  of its work. A later failure without a new `revise` still reaches
  review/1's copy (review/2 has no `revisions` and is passed over).

## The actions and effects

No new `AppAction` or `Effect`. The wire to Switchboard is unchanged:
`start_workflow` copies `plan_to_review().path` to the new attempt's
directory (`scheduler.rs:7466-7471`) and sends the same `Body::WorkflowStart
{ source, plan, .. }`. Add the call's `revised_from` to the attempt it pushes
(`:7489-7497`) and log one `info` line naming the file chosen and, when
`skipped` is set, the revision with no response file that was passed over.

`rerun_carries` (`scheduler.rs:9382`) returns `String` instead of
`&'static str` and gains an arm for a `Workflow` attempt (today it returns
"" there). Both callers format it already (`:7266`, `:9373`). It calls
`plan_to_review` with the attempt's stage and context. It has no stage
definition to read the subject from, so it takes it from the attempt's
`artifacts`, and only when that map has exactly one key (otherwise ""). The
tree supports that: the one non-test `new_attempt(.., AttemptKind::Workflow,
..)` call is in `start_workflow` (`scheduler.rs:7489-7497`) and passes
`BTreeMap::from([(subject.clone(), copy.clone())])`, one key, the subject.
(`AttemptKind::Workflow` at `:8407` is a match arm, not a second
constructor.) It adds one sentence:
- revised: "The rerun reviews the copy from `<stage>/<n>` at `<path>`, which
  holds that attempt's edits up to round `<r>`, your `revise` round; your
  note for it is at `<feedback file>`."
- skipped: "`<stage>/<n>` has a revision (round `<r>`) but no response file
  was found for it, so the rerun reviews the plan stage's output at `<path>`;
  your note is at `<feedback file>`."
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
  and whether or not the review is still open: `(started from the copy in
  <stage>/<n>: <path>)`. This answers "which is the reviewed copy
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
2. No response file: same without it. Attempt 2's copy equals `plan/1`'s
   output, `revised_from` is none, the question says no response file was
   found.
3. Refused round: the fake refuses `workflow.object`, so the feedback file
   is gone and the `Revision` stays; the rerun starts from the plan stage's
   output without the "no response file was found" sentence (the feedback
   file is absent, so the round is not a candidate).
4. The existing test and its siblings: no revise, so the copy is the plan
   stage's output. Update any assertion on the exact rerun question text;
   the question now ends with the "reviews the plan stage's output" sentence.

Unit, in `scheduler.rs` beside the plan-reader tests (about `:12449`):
`plan_to_review` on built tickets: (a) a `Complete` plan only; (b) a failed
review with a response file before it; (c) a `Complete` plan attempt appended
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
  run starts from the failed attempt's copy, with all its edits, when the
  planner has written a response to the owner's round; the response can land
  before the planner's last edit, so the copy is not proven final. Under the Dispatch ticket page
  section (`:2135`) list the gap: the Plan tab does not show `prior`.
- `README.md`: no new file, so no Layout change; no new environment
  variable.
- `CHANGELOG.md` under Unreleased: "A plan review rerun after a failed
  attempt starts from the failed attempt's copy, including the owner's `revise` round, when the planner answered it,
  and the rerun question and `show` say which copy it reviews."

## One commit

Message: `Dispatch: a plan review rerun starts from the failed attempt's copy with the owner's revision (#187)`

Contract additions (copy into the commit message body):
- `dispatch::ticket::Attempt.revised_from: Option<u32>`, `#[serde(default)]`;
  dispatch `RECORD_VERSION` bumped (value in `store.rs` at implementation
  time, plus one) with a defaulted-field step in `store::migrate`.
- `dispatch_control::PriorCopy { n: u32, path: PathBuf }` and
  `dispatch_control::LaneFile.prior: Option<PriorCopy>`, additive on the wire.
- Internal: `plan_to_review` replaces `review_subject`; `rerun_carries`
  returns `String`. No `AppAction`, `Effect`, port or `model.rs` change.

## For the owner, after this lands

- Whether the planner always edits before it responds (U2) is the same open
  question; the plan reads a response file as "answered", not "finished".
- The rerun's planner is a clone of the original planner and has not seen the
  `revise` note (S). Giving it the note means changing what
  `workflow.start` sends or clones, in the app. Needs a spike and a contract
  change; not in this commit.
- The app's Plan tab does not show `prior` yet.
- O1 and U2 are unanswered. The plan works either way; the owner's data
  directory would show whether any real failure left a half-written copy, and
  if so whether the planner should be stopped when an attempt fails.
