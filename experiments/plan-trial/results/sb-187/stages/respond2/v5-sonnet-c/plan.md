# Plan: issue #187, a plan review rerun starts from the newest copy

All line numbers are this worktree (HEAD 7fe8949, Release 0.4.1). The
code is the `dispatch` crate; the review stage is a *workflow* stage
(`AttemptKind::Workflow`), not the code review in `dispatch/src/review.rs`.

## Where this departs from the issue

- **"The `revise` note should survive a runner failure the same way an
  answered decision does."** It already does. The note is stored in the
  answered `Decision.state` and in `plan.feedback-N.md` beside the
  failed attempt's copy (`scheduler.rs:7611-7612`, `OWNER_ROUND_HEADING`).
  What is lost is the *revised plan text*: `review_subject`
  (`scheduler.rs:10534`) keeps only `Complete` attempts, so failed
  `review/1` is skipped and `plan/1`'s copy is reviewed
  (`facts.md` Settled, root cause). The plan fixes the choice of source
  and leaves the note where it is.
- **The issue frames the loss as a post-`revise` problem.** The rule is
  wider: a review edits its copy in place under one path from the first
  round (`docs/design.md:2215`), so any failed attempt's copy is newer
  than the plan stage's, with or without a `revise`. The plan applies to
  every failed review attempt, not only ones with
  `Attempt.revisions` (`ticket.rs:350`) non-empty. It does not apply to
  cancelled ones (reason under Path below).
- **"Say which copy it will review" in `show`.** `show` lists only
  `Complete` or open attempts (`Ticket::shown_where`, `ticket.rs:1476`),
  so a failed copy is listed nowhere. The plan says which copy in the
  rerun question, the place the owner decides, and leaves `show` alone.

## The record

No record change: no new field, no `RECORD_VERSION` bump.

## The actions and effects

1. **Choose the source** (`scheduler.rs:10534`). Change
   `review_subject(t, subject)` to
   `review_subject(t, stage, ctx, subject, exists)`, still returning
   `Result<(PathBuf, String), String>`. `exists` is `impl Fn(&Path) ->
   bool` so the function stays testable without a disk (the caller
   passes `Path::exists`, a test passes `|_| true`).
   - **Session**: unchanged. The newest `Complete` attempt that wrote
     `subject`, its `session`, with the same two errors ("nothing wrote
     …", "… was not written by a session"). A review attempt's own
     `session` is the planner/reviewer face (`scheduler.rs:7777-7781`),
     so the clone source must stay the plan stage's session; path and
     session no longer come from one attempt.
   - **Path**: the newest attempt of `(stage, ctx)` with kind `Workflow`
     and state `Failed`, *positioned after* that `Complete` writer in
     `t.attempts`, whose `artifacts[subject]` exists. Else the
     `Complete` writer's path. The position test is what stops a failed
     `review/1` from beating a `plan/2` that was written after a
     send-back. `Cancelled` does not qualify: every `Cancelled` write in
     the tree (`scheduler.rs:2872`, `2914`, `4962`, `5585`) is an owner
     stop, close or send-back (the last two reasons prefixed
     `SENT_BACK_FROM`, l.8979), so a cancelled copy is not one the owner
     wants reviewed, and a review cancelled by a later stage's send-back
     with no new `plan` attempt would otherwise win.
   - **Missing file**: falls back to the plan stage's copy
     (`facts.md` Settled: nothing removes the `artifacts` entry, but
     nothing checks the file either).
   - `start_workflow` (`scheduler.rs:7465`) passes
     `&stage.name, ctx, &subject, Path::exists` and copies from the
     returned path as now (l.7474-7477).
2. **Say it in the rerun question.** `rerun_carries`
   (`scheduler.rs:9382`) returns `&'static str` and is empty for a
   workflow attempt. Make it return `String`; its two callers are
   `fail_attempt_with` (l.7266) and `rerun_ask` (l.9373). For a
   `Workflow` attempt whose `artifacts` holds a copy, it computes the
   source with the same `review_subject` call (subject name = the
   attempt's one artifact key, as `plan_rounds` does in `serve.rs`) and
   appends: " A rerun reviews {path}" plus, when that is the failed
   attempt's own copy, " (this attempt's copy, {n} revision(s) by the
   owner; the planner may have been mid-edit)", else " (the plan stage's
   output; this attempt's copy is missing)". The finalize question
   already names its copy the same way (`review_done`,
   `scheduler.rs:7692-7695`). Both the first ask and the re-ask after a
   park go through these two callers, so both carry the text. This is
   also how the owner, who may doubt the copy is complete (`facts.md`
   U1), can open it and answer `park` instead.

## The tests

Scheduler (`scheduler.rs` tests, beside the review-plan-input tests at
~l.12838-12869):

- `review_subject` unit tests with `|_| true`: a failed `review/1`
  after `plan/1` gives `review/1`'s path and `plan/1`'s session; two
  failed attempts give the newer; a `Cancelled` one does not (including
  one with a `SENT_BACK_FROM` reason, newest after `plan/1`: the result
  is `plan/1`); an open one does not; a `plan/2` `Complete` after the
  failed `review/1` gives `plan/2`; the exists closure returning false
  gives `plan/1`; no `Complete` writer is still the "nothing wrote"
  error.
- The gate (`facts.md` U3): seed a ticket with `plan/1` complete, a
  failed `review/1` whose copy differs and whose `revisions` is
  non-empty, a `rerun` answer acted (`may_rerun`, l.10665); run the
  runner pass; assert the `WorkflowStart.plan` sent is under
  `review/2`, its content equals `review/1`'s copy, and `source` is
  `plan/1`'s session. A second case with `plan/1` only asserts the old
  behaviour.
- `rerun_ask` text: workflow attempt names the path and revision count;
  a code review attempt's text and the `resolution` text are unchanged
  (`events.rs` rerun tests ~l.3313 pin neighbours; update only if they
  fail on the new text).

## Docs and changelog

- `CHANGELOG.md` Unreleased, under Fixed: "A Dispatch plan review rerun
  after a failed attempt starts from that attempt's copy of the plan,
  with the owner's revisions, instead of the plan stage's original; the
  rerun question says which copy it reviews."
- `docs/design.md`: the runner-patience paragraph (l.1782-1786, "a
  rerun kills the old run's agents and starts a fresh run") gains "from
  the newest copy a failed attempt held"; add the status line to the
  "Plan review workflow" section (l.1000).

## Follow-ups for the owner (not in this plan)

- A `started_from` field on `Attempt`, surfaced in `show` and the Plan
  tab, if the owner wants the source copy on the record rather than only
  in the rerun question. It needs a `RECORD_VERSION` bump and a
  `migrate` step.

## Commit

`Dispatch: a plan review rerun starts from the failed attempt's copy and says so`

Contract additions (copy into the commit message):

- None: no `AppAction`, `Effect`, port method, wire message, record field or `RECORD_VERSION` change.
