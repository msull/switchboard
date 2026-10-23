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
  cancelled ones: the tree sets `Cancelled` for a ticket close
  (`scheduler.rs:2914`), a stop with a reason (`scheduler.rs:2872`) and a
  send-back (`scheduler.rs:4962`, `scheduler.rs:5585`, reason prefixed
  `SENT_BACK_FROM`, l.8979), where the owner discarded the attempt on
  purpose, and the facts file settles only failed attempts.
- **Step 2 of the repro (a runner restart gives `review run gone`).**
  Not borne out or contradicted: it is Unresolved U2 and owner question
  O1. `Paused{failed:true}` (`scheduler.rs:7840`) fails the attempt the
  same way, so the plan does not depend on the cause.
- **"Say which copy it will review" in `show`.** `show` lists only
  `Complete` or open attempts (`Ticket::shown_where`, `ticket.rs:1476`),
  so a failed copy is listed nowhere. The plan records where a rerun
  started from (below) rather than rediscovering it from files.

## The record

One additive field, `Attempt.started_from: Option<(String, u32)>`
(`ticket.rs:266`, `#[serde(default)]`): the `(stage, n)` of the attempt
whose copy this workflow attempt was started from. Set in
`start_workflow` for every workflow attempt; a plan review's first
attempt names the plan stage's attempt, a rerun names the failed
review attempt. Why a field and not a recomputation at display time:
the choice depends on whether a file existed when the attempt started,
and `show` should report what happened, not what the rule would pick
now. Attempts already running keep `None` and `show` says nothing about
them.

Version: `RECORD_VERSION` (`store.rs:26`) at implementation time, plus
one. Add the matching step comment in `migrate` (`store.rs:450`,
after the "27 to 28" step at l.608) in the same style: an attempt
gains `started_from`, absent from its serde default, nothing is
transformed, a build that would drop it must refuse the record. The
version today is 28; take whatever the file says when you start, since a
parallel ticket may have taken 29.

## The signals

None new. No hook event, no wire message: `Body::WorkflowStart.plan`
(`scheduler.rs:7519`) already carries the copy's path, and the app
reviews whatever file it is given. The app is not changed.

## The actions and effects

No new `AppAction`, `Effect`, port or adapter; the change is inside the
runner's `start_workflow` path.

1. **Choose the source** (`scheduler.rs:10534`). Change
   `review_subject(t, subject)` to
   `review_subject(t, stage, ctx, subject, exists)` returning a small
   struct `Subject { path, session, from: (String, u32) }` instead of a
   tuple (it now has three parts, and a tuple of `PathBuf, String, (String, u32)` is hard to read). `exists` is
   `impl Fn(&Path) -> bool` so the function stays testable without a
   disk (a closure parameter: the caller passes `Path::exists`, a test
   passes `|_| true`).
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
     stop, close or send-back, so a cancelled copy is not one the owner
     wants reviewed, and a review cancelled by a later stage's send-back
     with no new `plan` attempt would otherwise win. Widening to cancel
     reasons that are not `SENT_BACK_FROM…` is a follow-up that needs a
     line in the tree showing such a copy is wanted.
   - **Missing file**: falls back to the plan stage's copy
     (`facts.md` Settled: nothing removes the `artifacts` entry, but
     nothing checks the file either).
   - `start_workflow` (`scheduler.rs:7465`) passes
     `&stage.name, ctx, &subject, Path::exists`, copies from
     `subject.path` as now (l.7474-7477) and sets
     `started_from = Some(subject.from)` on the attempt it pushes
     (l.7499). `new_attempt` (`scheduler.rs:10277`) initialises `None`.
   - Chained failures work: `review/3` starts from `review/2`'s copy,
     which was itself started from `review/1`'s.
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
   park go through these two callers, so both carry the text.
3. **The default does not depend on the unresolved facts.** U1 (a
   half-written revised copy when `no such run` arrives) and O1 (what
   caused the loss) are answered by the same behaviour: the rerun
   starts from the newest copy and the question says which one and
   how many revisions it holds, so the owner can open it and answer
   `park` instead. No branch tests the cause. Nothing is built behind
   the default.

## The UI

`show` only; the Plan tab in the app is not changed.

- `dispatch-control/src/lib.rs:586` `LaneFile` gains
  `started_from: Option<PathBuf>`, the source copy's path. The struct
  is `#[serde(default)]` and derives `Default`, so an older runner's
  JSON reads as `None` (`facts.md` Settled; that the Plan tab ignores
  the field rests on serde ignoring unknown fields, which this plan
  assumes and the UI test below confirms for the struct's
  deserialisation, not for the tab's drawing).
- `serve.rs:538` `ticket_paths`: when the shown attempt has
  `started_from == Some((stage, n))` with `stage == a.stage` (a rerun),
  set the field to that attempt's `artifacts` path, found with
  `find_attempt` (`scheduler.rs:9452`). A first attempt leaves it `None`
  so a normal review prints nothing new.
- `main.rs:896` `print_lane_files`: after the existing
  "(reviewed copy, round N, review open)" line, print
  "(started from the previous attempt's copy: {path}; its owner rounds
  stay beside it)" when the field is set. That names the reviewed copy
  (already marked) and the prior revision, which the issue asked for.
  If the file has no capture test, move the line text into a function
  returning `String` and test that.

## The tests

Scheduler (`scheduler.rs` tests, beside the review-plan-input tests at
~l.12838-12869; read the fake that answers `Body::Workflow` there
first — `facts.md` could not say which harness answers it):

- `review_subject` unit tests with `|_| true`: a failed `review/1`
  after `plan/1` gives `review/1`'s path, `plan/1`'s session and
  `from == ("review", 1)`; two failed attempts give the newer; a
  `Cancelled` one does not (including one with a `SENT_BACK_FROM`
  reason, newest after `plan/1`: the result is `plan/1`); an open one
  does not; a `plan/2` `Complete`
  after the failed `review/1` gives `plan/2`; the exists closure
  returning false gives `plan/1`; no `Complete` writer is still the
  "nothing wrote" error.
- The gate (`facts.md` U3): seed a ticket with `plan/1` complete, a
  failed `review/1` whose copy differs and whose `revisions` is
  non-empty, a `rerun` answer acted (`may_rerun`, l.10665); run the
  runner pass; assert the `WorkflowStart.plan` sent is under
  `review/2`, its content equals `review/1`'s copy, `source` is
  `plan/1`'s session, and the new attempt's `started_from` is
  `Some(("review", 1))`. A second case with `plan/1` only asserts the
  old behaviour and `started_from == Some(("plan", 1))`.
- If the fake allows it, a test that answers `Reply::Failed { reason:
  NO_SUCH_RUN.into() }` to `Body::Workflow` (no existing test does,
  `facts.md` Q on fakes), asserts the attempt becomes `Failed` with
  "review run gone", the `rerun` question names `review/1`'s copy, and
  the next pass starts from it. If the fake cannot, say so in the
  commit and keep the seeded gate.
- `rerun_ask` text: workflow attempt names the path and revision count;
  a code review attempt's text and the `resolution` text are unchanged
  (`events.rs` rerun tests ~l.3313 pin neighbours; update only if they
  fail on the new text).

Store (`store.rs` tests near l.971): a record at the previous version
loads with `started_from == None` and is written at the new version.

Serve and control: `ticket_paths` on a ticket with `review/1` failed and
`review/2` open sets `LaneFile.started_from` to `review/1`'s copy;
with `review/1` open it is `None`. `dispatch-control`: JSON for a
`LaneFile` without the field deserialises.

Then `cargo fmt --all`, `cargo clippy --locked --workspace --all-targets
-- -D warnings`, `cargo test --locked --workspace`.

## Docs and changelog

- `CHANGELOG.md` Unreleased, under Fixed: "A Dispatch plan review rerun
  after a failed attempt starts from that attempt's copy of the plan,
  with the owner's revisions, instead of the plan stage's original; the
  rerun question and `show` say which copy it reviews."
- `docs/design.md`: the runner-patience paragraph (l.1782-1786, "a
  rerun kills the old run's agents and starts a fresh run") gains "from
  the newest copy a failed attempt held"; add the status line to the
  "Plan review workflow" section (l.1000) with the `started_from` field
  and the record version.
- `README.md`: no new file, env var or data-directory entry, so no
  change to the Layout table or dev aids.
- The app is unchanged, the app/script `SWITCHBOARD_SCRIPT` has nothing
  to reach, and `Ticket::input`/`lane_input` still hand later stages
  only `Complete` plans on purpose (pinned at `scheduler.rs:12838`).

## Follow-ups for the owner (not in this plan)

- O1: whether the real ticket's revised copy was complete and whether a
  runner or app restart caused `no such run`. If the app drops runs on
  restart, that is its own bug; this plan makes the loss harmless, not
  rare. The answer changes nothing here.
- The app's Plan tab could show the `started_from` marker; the tab was
  not read.
- The planner clone in a rerun is cloned from the plan stage's
  conversation, so it has not seen the owner's `revise` note as a
  message, only its effect in the copy. Handing it the note is a
  separate decision.

## Commit

`Dispatch: a plan review rerun starts from the failed attempt's copy and says so`

Contract additions (copy into the commit message):

- `Attempt.started_from: Option<(String, u32)>` (`dispatch/src/ticket.rs`), serde default.
- `LaneFile.started_from: Option<PathBuf>` (`dispatch-control/src/lib.rs`), serde default.
- `RECORD_VERSION` in `dispatch/src/store.rs`: the value there at implementation time, plus one, with a `migrate` comment step.
- No new `AppAction`, `Effect`, port method or wire message.
