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
  into the new attempt (see the last bullet for the gap).
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
- Known gap, not fixed: `revise` pushes the `Revision` before it sends
  the round, and keeps it on a lost send. So a failed attempt with a
  revision may be one whose planner never applied the owner's round
  (the runner died first, or the send was lost). The new attempt then
  starts from a copy that lacks the note. `WorkflowStart` has no field
  to pass the round file along, and the contract is not widened for
  this. Instead the plan says so: the rerun question names the round
  file, the changelog and docs say the rerun starts from the copy the
  owner's revise was sent to, not that the copy has applied it.

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

- Add one pure helper beside `review_subject`, which the scheduler, the
  question and `show` all call, so they cannot disagree:
  `revised_source(t, stage, ctx, subject, exists) -> Option<RevisedSource>`
  with `RevisedSource { attempt: &Attempt, path: &PathBuf, missing:
  Option<&PathBuf> }`. It finds the index `i` of the newest Complete
  attempt holding `subject` (today's rule), then the newest attempt
  after `i` with `stage == stage`, `context == ctx`, `kind == Workflow`,
  state Failed or Cancelled, and a non-empty `revisions`, that holds
  `subject`. If its copy satisfies `exists`, that is the result. If the
  file is gone, the result is `None` and `missing` is reported by a
  second function, `revised_source_gone`, built on the same walk, so
  `start_workflow` can log it.
- The rule is limited to attempts with `revisions` because only those
  hold owner input. A copy that planner rounds edited and the owner
  never finalized is not a better source than the finished original.
  If `review/2` also fails with no revision of its own, `review/3`
  still takes `review/1`'s revised copy.
- `exists` is a predicate argument (`Path::exists` in production, a
  closure in tests), so the helper does no I/O and stays testable.
- The match is on `stage` and `ctx`, as `latest_attempt(t, &stage.name,
  &ctx)` (9444) does, so one lane's failed review never feeds another
  lane's.
- `review_subject(t, subject, stage, ctx, exists)` returns the helper's
  path, else the Complete attempt's, and the Complete attempt's session.
  Its only caller is `start_workflow` at 7465, which passes
  `&stage.name`, `ctx` and `Path::exists`. The error strings (`nothing
  wrote ...`, `... was not written by a session`) are unchanged. When
  the revised copy is gone, `start_workflow` logs a `log::warn!` naming
  the missing path and the original it fell back to; today the fallback
  would be silent.
- Where the subject name comes from. `rerun_ask(t, a)` (9335) has no
  pipeline, and the resume call at 8294 holds only `&t` and `&a`. A
  workflow attempt's `artifacts` has exactly one key, the subject:
  `start_workflow` builds it as `BTreeMap::from([(subject, copy)])`
  (7496). `rerun_carries` takes the subject from that key, so no
  pipeline is needed. An attempt whose `artifacts` does not have
  exactly one key gets the empty sentence, as today.
- `rerun_carries` (9382) is called from 7266 and from `rerun_ask`. Give
  it the attempt in place of `(stage, n)` and add an arm for
  `AttemptKind::Workflow`. It needs the Complete source's path, which
  the helper's walk finds; if there is no Complete source (so
  `start_workflow` would park with `nothing wrote ...`) or the source
  has no session, the sentence is empty, since the question cannot
  return an error and the park says why. Otherwise:
  - the helper has a result: "If `<path>` is still there, the next
    attempt reviews it: the copy of `<stage>/<n>` the owner's revise
    was sent to, whose note is at `<round file>`; it is not the stage's
    first plan.";
  - none: "The next attempt reviews `<path>`, the plan as `<stage>/<n>`
    finished it."
  The sentence is fixed when the question is asked (`ensure_decision`
  dedupes on stage, name and attempt, not text), so it states what the
  rerun will do if the file is still there, which is why the first form
  says so; the warn above covers a file deleted in between. The round
  file is `round_file(<copy>, <last revision's round>)`. The resolution
  and code-review arms stay as they are.
- Because the sentence depends on files, `rerun_ask` and
  `rerun_carries` take the `exists` predicate too. The call at 7266 and
  the resume call at 8294 pass `Path::exists`.

## The UI

`show` is the only surface; there is no egui. It is text, so it needs
the data and a print arm.

- `dispatch-control/src/lib.rs:586` `LaneFile` gets
  `prior: Option<u32>`: the attempt number of the failed or cancelled
  review attempt whose copy a rerun would start from; `None` for every
  file listed today. The struct is `#[serde(default)]`, so older readers
  and older output stay valid. `dispatch-control` has no protocol or
  version constant (checked: none in `lib.rs`), so `show --json` needs
  no bump. The struct literals in serve.rs that do not end in
  `..LaneFile::default()` (the ones near 1469 and 1540) get
  `prior: None`; the others compile unchanged.
- `shown_lane_files` (10900) is left alone, since `shown_where` is
  display-only and `lane_files_by` has other callers. Add
  `prior_lane_files(t, p, exists)` in scheduler.rs, anchored on the
  workflow attempts themselves, not on the shown row, because the shown
  row is `plan/1`, a Complete agent attempt, from the moment `review/1`
  fails until `review/2` opens: exactly when the owner is asked "Run it
  again?". For each distinct (stage, context) among the ticket's
  Workflow attempts, it calls `revised_source` with that attempt's
  subject key. A result becomes a row with `prior: Some(n)`,
  `reviewing: false`, placed straight after the shown row of the same
  lane (or last when that lane has none). Its lane is the context when
  `p` says the stage runs per lane, else none. This is the scheduler's
  rule, not a copy of it: a revised copy older than a later Complete
  attempt, or whose file is gone, gives no row, and once `review/2`
  completes the row goes away because `review/1` is then older than the
  Complete source.
- `ticket_paths` (serve.rs:538) appends those rows to `plan_files`.
- `print_lane_files` (main.rs:896) prints, under a row with `prior`:
  `(prior attempt review/1, owner revised, not reviewed)`. Every row
  that has `prior` has a revision, because the helper requires one, so
  the text needs no further field. It says "revised", not "applied",
  for the gap above. The shown row is unchanged, so "the reviewed copy"
  and "a prior revision" are told apart by the line under each.
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
- `a_rerun_after_a_lost_send_still_starts_from_the_revised_copy`: the
  failed attempt's revision is there because the send was lost (the
  `lost-send` question is on the ticket), so the planner never applied
  it. It pins the documented gap: the rerun starts from that copy and
  the question names the round file.
- `a_second_failed_review_still_starts_from_the_first_revision`:
  `review/1` revised and failed, `review/2` failed with none; `review/3`
  starts from `review/1`'s.
- `revised_source` and `review_subject` unit tests for the same cases,
  plus: a failed review in another context is ignored; a revised
  attempt older than a later Complete one is ignored; a missing file
  gives `None` and `start_workflow` logs the fallback.
- `rerun_ask` tests for the two new sentences (the round file named,
  the subject read from the attempt's single artifact key, no pipeline
  given), the empty sentence with no Complete source, and that the
  code-review and resolution sentences are unchanged.
- serve.rs `ticket_paths` tests (near 1395-1570): with `plan/1`
  Complete and `review/1` Failed with a revision and a rerun question
  pending, the shown row is `plan/1` and a prior row `Some(1)` follows
  it (the window the owner is asked in); the same with the rerun
  answered and unacted; with `review/2` running, the row stays; with
  `review/2` Complete, it is gone; a failed attempt with no revision, or
  a missing file, adds no row; a per-lane ticket pairs each prior row
  with its own lane.
- main.rs: `print_lane_files` formatting if the file has a test for it;
  otherwise the `ticket_paths` test stands for it.
- docs/dispatch.md: add rows to the behaviour table beside the
  `revise --note` row (~2975), each naming its test as the others do.

The docs also change. docs/dispatch.md "A review is a copy" (~550) gets
a sentence: a rerun after a failed attempt starts from that attempt's
copy when the owner's `revise` was sent to it, else from the stage's
source; if the runner failed before the planner applied the round, the
copy lacks the note, which is in the round file the question names.
`CHANGELOG.md` gets one line under `[Unreleased]`:
"A review rerun after a failed attempt starts from the copy the owner's
`revise` was sent to, not the stage's first plan, and the rerun question
and `show` name which copy it reviews; if the runner failed before the
planner applied the revision, the note is in the round file the question
names."
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
  removed; no wire version constant exists to bump.
- `scheduler::review_subject` takes `stage`, `ctx` and `exists`
  (crate-private, one caller); new crate-private `revised_source`,
  `revised_source_gone` and `prior_lane_files`.
- `rerun_carries` and `rerun_ask` take the attempt and the `exists`
  predicate (crate-private).
- No change to `model.rs`, `AppAction`, `Effect`, a port, the ticket
  record or `RECORD_VERSION` (store.rs:26, 28 today).
