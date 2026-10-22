# Plan: a review rerun starts from the owner's revised copy (#187)

Paths are in `dispatch/src/` unless they name another crate or `docs/`.
Line numbers were checked against this worktree.

## Where this departs from the issue

- The issue says the rerun path decides the input. It does not:
  `workflow_stage` (scheduler.rs:7379) calls `start_workflow` (7436)
  for a rerun and for a first start alike. The choice is made in
  `review_subject` (10534), which keeps only `AttemptState::Complete`
  attempts. A failed `review/1` is skipped and the walk lands on
  `plan/1`. The fix is there, not in the rerun path.
- The issue says the `revise` note should survive a runner failure
  like an answered decision. It already does. The note is the
  `finalize` decision's `Answered { note }`, the owner's round file
  `round_file(<copy>, round)` written by `revise` (7565), and a
  `Revision` on the failed attempt (`ticket.rs:350`). Nothing deletes
  them. What is lost is their use, and this plan fixes that by
  starting from the failed attempt's copy. It does not copy the note
  into the new attempt (see the last bullet).
- The issue offers "or at least say in the rerun question and in
  `show`". The plan does both as well as fixing the input, because
  both are wrong today. `rerun_carries` (9382) returns "" for a
  workflow attempt, and `show` lists only `review/2`'s copy:
  `Ticket::shown_where` (`ticket.rs:1476`) keeps only Complete or open
  workflow attempts, so a Failed one never appears.
- `review_subject` also returns the Complete attempt's session, which
  the planner clone forks from. The plan keeps that session even when
  it takes the failed attempt's file. The file is what carries the
  revision, and the planner is told to read and edit the copy only.
  The failed attempt's `run_sessions["planner"]` would be a closer
  fork, but nothing in the repo shows that a clone of a clone works,
  so it is left alone.
- Known gap, not fixed: if the runner died before the planner applied
  the owner's round, the failed copy lacks the revision and the new
  attempt does not see the note. The round file and the decision still
  hold it. The question's text cannot tell the two cases apart without
  reading the response file, so the plan says nothing either way.

## The record

No record change. Everything needed is already on the failed attempt:
`artifacts["plan"]` (the copy's path), `revisions`, `stage`, `context`,
`state`. `RECORD_VERSION` (store.rs:26, 28 today) is not bumped, and no
migration step is added. Rehydration is the existing reconcile that
finds a failed attempt and a `may_rerun` answer. The source is derived
from the ticket on every call, never from process state.

## The signals

No new signal. The poll, the `rerun` decision and the `StoreLoaded` /
`HostListed` reconcile are unchanged.

## The actions and effects

No new action or effect. The scheduler's choice of file changes.

- Add a pure helper beside `review_subject`:
  `workflow_source(t, stage, ctx, subject, exists)`. It finds the newest
  Complete attempt holding `subject` (today's rule), at index `i` in
  `t.attempts`. Then it looks for the newest attempt after `i` with
  `stage == stage`, `context == ctx`, `kind == Workflow`, state Failed
  or Cancelled, a non-empty `revisions`, and `artifacts[subject]`
  satisfying `exists`. If one is found, its copy is the source. If not,
  the Complete attempt's file is, as now.
- The rule is limited to attempts with `revisions` because only those
  hold owner input. A copy that planner rounds edited and the owner
  never finalized is not a better source than the finished original.
  If `review/2` also fails with no revision of its own, `review/3`
  still takes `review/1`'s revised copy.
- `exists` is a predicate argument (`Path::exists` in production, a
  closure in tests), so the helper does no I/O and stays testable.
  A missing revised file falls back to the Complete source.
- The match is on `stage` and `ctx`, as `latest_attempt(t, &stage.name,
  &ctx)` (9444) does, so one lane's failed review never feeds another
  lane's.
- `review_subject(t, subject, stage, ctx, exists)` returns the helper's
  path and the Complete attempt's session. Its only caller is
  `start_workflow` at 7465, which passes `&stage.name`, `ctx` and
  `Path::exists`. The error strings (`nothing wrote ...`, `... was not
  written by a session`) are unchanged.
- `rerun_carries` (9382) is called from 7266 and from `rerun_ask`
  (9335). Give it the attempt in place of `(stage, n)` and add an arm
  for `AttemptKind::Workflow`. When the helper picks a failed attempt's
  copy, the arm says: "The next attempt reviews the revised copy
  `<path>` from `<stage>/<n>`, not the stage's first plan." Otherwise
  it says: "The next attempt reviews `<path>`, the plan as `<stage>/<n>`
  finished it." That is the same pair of facts the `finalize` question
  states (7694: "The reviewed copy is ..."). The resolution and
  code-review arms stay as they are.
- Because the sentence depends on files, `rerun_ask` and
  `rerun_carries` take the `exists` predicate too. The call at 7266 and
  the resume call at 8294 pass `Path::exists`.

## The UI

`show` is the only surface; there is no egui. It is text, so it needs
the data and a print arm.

- `dispatch-control/src/lib.rs:586` `LaneFile` gets
  `prior: Option<u32>`: the attempt number of a failed or cancelled
  review attempt's copy that a later attempt superseded; `None` for
  every file listed today. The struct is `#[serde(default)]`, so older
  readers and older output stay valid.
- `shown_lane_files` (10900) is left alone, since `shown_where` is
  display-only and `lane_files_by` has other callers. Add `prior_lane_files` in
  scheduler.rs. For each shown row whose attempt is a Workflow, it
  lists attempts of the same stage and context with `n` below the shown
  attempt's, state Failed or Cancelled, `kind == Workflow`, and a
  `plan` artifact, newest first. `ticket_paths` (serve.rs:538) pushes
  those after the shown row into `plan_files`, with `prior: Some(n)`
  and `reviewing: false`.
- `print_lane_files` (main.rs:896) prints, under a row with `prior`:
  `(prior attempt review/1, not reviewed)`. It adds
  `, owner revised` when the attempt has revisions. `LaneFile` has no
  field for that, so `prior_lane_files` only returns attempts that have
  one, which is the same rule the scheduler uses to pick its source.
  The shown row is unchanged, so "the reviewed copy" and "a prior
  revision" are told apart by the line under each.
- `docs/dispatch-agent-guide.md` (~205-218) gets one sentence for the
  new line, beside the `(reviewed copy ...)` one.

## The tests

Core, with fakes and `Clock::at`, no sleeping, modelled on
`lanes_ticket` (scheduler.rs:12388):

- `a_rerun_after_a_failed_review_starts_from_the_revised_copy`: `plan/1`
  Complete, `review/1` Failed with a `Revision` and a real temp file
  holding different text, the `rerun` answered and acted. Assert the
  `WorkflowStart.plan` copy's content equals the revised file's, and
  the session is the plan writer's.
- `a_rerun_without_a_revision_starts_from_the_plan`: the same with empty
  `revisions`; the copy equals `plan/1`'s file.
- `a_rerun_whose_revised_copy_is_gone_starts_from_the_plan`: the file
  is missing; the copy equals `plan/1`'s file.
- `a_second_failed_review_still_starts_from_the_first_revision`:
  `review/1` revised and failed, `review/2` failed with none; `review/3`
  starts from `review/1`'s.
- `review_subject` unit tests for the same four cases, plus the lane
  case: a failed review in another context is ignored.
- `rerun_ask` tests for the two new sentences, and that the code-review
  and resolution sentences are unchanged.
- serve.rs `ticket_paths` tests (near 1395-1570): a prior row follows
  the shown one with `prior: Some(1)`; a failed attempt with no
  revision adds no row; a per-lane ticket pairs each prior row with its
  own lane.
- main.rs: `print_lane_files` formatting if the file has a test for it;
  otherwise the `ticket_paths` test stands for it.
- docs/dispatch.md: add rows to the behaviour table beside the
  `revise --note` row (~2975), each naming its test as the others do.

The docs also change. docs/dispatch.md "A review is a copy" (~550) gets
a sentence: a rerun after a failed attempt starts from that attempt's
copy when the owner revised it, else from the stage's source.
`CHANGELOG.md` gets one line under `[Unreleased]`:
"A review rerun after a failed attempt starts from the owner's revised
plan, and the rerun question and `show` name which copy it reviews."
README.md's Layout table needs no new file; if its scheduler.rs line
(459) lists this behaviour, add the phrase there.

The app runs at every step. The order is the helper with its tests,
then the question text, then the `LaneFile` field and `show`, all in
the one commit.

## Commit message

Review reruns start from the owner's revised copy of a failed attempt, and the rerun question and `show` name the reviewed copy

## Contract additions

- `dispatch_control::LaneFile.prior: Option<u32>`, `#[serde(default)]`
  through the struct's existing attribute; additive, nothing renamed or
  removed.
- `scheduler::review_subject` takes `stage`, `ctx` and `exists`
  (crate-private, one caller); new crate-private `workflow_source` and
  `prior_lane_files`.
- `rerun_carries` and `rerun_ask` take the attempt and the `exists`
  predicate (crate-private).
- No change to `model.rs`, `AppAction`, `Effect`, a port, the ticket
  record or `RECORD_VERSION` (store.rs:26, 28 today).
