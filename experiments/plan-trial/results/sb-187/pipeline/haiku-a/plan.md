# Issue #187: a plan review's rerun starts from the owner's revision

## Where this departs from the issue

- Bears out: the rerun copies `plan/1` because `review_subject` keeps only `Complete` attempts (`dispatch/src/scheduler.rs:10534`). `show` and the Plan tab name only the attempt-2 copy (`dispatch/src/ticket.rs:1476` `shown_where`, `dispatch/src/serve.rs:539`). The rerun question never names a copy: `rerun_carries` (`scheduler.rs:9382`) speaks only for code review.
- Differs: the owner's note is kept on the answered `finalize` decision, and `show` prints it (`dispatch/src/main.rs:1176`), so it already survives the failure. The new planner never receives it, and this plan does not re-send it. Sending needs a `workflow.object` on a `Converged` or `AtCap` run with round = round count + 1 (`docs/design.md:1190-1200`), and a new run has no rounds yet. The plan carries the owner's change in the copy instead. Re-sending the note is deferred to its own change.
- Differs from the investigation notes: the revised copy is gated on the attempt's `revisions` entry, not on the planner's `plan.response-N.md` (`dispatch/src/report.rs:202`). A run killed before the planner answers, which is the repro's shape, leaves no response file, so a response gate would miss the bug. The `revisions` entry is written before the send (`scheduler.rs:7620`) and popped on a refused send (`scheduler.rs:11313`, the refused-`REVISE` arm); the same refusal is described in the table row at `docs/dispatch.md:2975`.
- Not covered: a plan review that fails after reviewer rounds, with no owner `revise`, restarts from the newest revised copy if an earlier attempt has one, else from `plan/1`. Its own in-place round edits are lost, as they are today. The rerun question names the copy, so this is visible. Widening the gate to round files is a small follow-up if the owner wants it.
- Name check: the notes call the constant `SCHEMA_VERSION`. This tree has `RECORD_VERSION` (`dispatch/src/store.rs:26`), value 28.

## The record

No field added. The gate is the existing `Attempt.revisions` (`dispatch/src/ticket.rs:350`, `Revision` at `ticket.rs:510`) on the failed attempt that holds the revised copy. The rerun does not copy them. A round number means something only on the attempt whose round files it names: copied onto a new run, a `Revision { round: 1 }` would make the reviewer's own round 1 read as the owner's objection in `plan_review` (`dispatch/src/report.rs:342`, the match at `:361`), `plan_rounds` (`dispatch/src/serve.rs:515`) and `review_round` (`serve.rs:531`). Instead the source search (below) walks back past a failed attempt that has no revisions of its own, so a later failure still finds the revised copy: if `review/2` fails with no revise, `review/3` starts from `review/1`'s copy. A later `revise` on a new attempt sets its own round 1 through the existing `retain` at `scheduler.rs:7620`. Today a failed attempt is skipped altogether, so the walk-back is never worse than today.

No schema bump. `RECORD_VERSION` is 28 today (`store.rs:26`) and stays 28. `USAGE_VERSION` (`store.rs:642`) is untouched.

## The signals

None new. The failed or cancelled review attempt that has a `revisions` entry is the signal.

## The actions and effects

No new action or effect. The changes are inside existing paths:

- `review_subject` (`scheduler.rs:10534`) takes the copy from the source search: the newest attempt holding the subject artifact that is not `forgotten` for it and is either `Complete`, or `Failed` or `Cancelled` with a non-empty `revisions`. A failed attempt with none is passed over. The `forgotten` check is the one `input_where` (`ticket.rs:1452`) makes. Its session is not taken from that attempt: a failed review's session is dead. It stays the newest `Complete` attempt's that holds the subject, as today, so the reviewer's planner clone is the plan's author's.
- `start_workflow` (`scheduler.rs:7436`) copies from that path (`:7470-7471`, shape unchanged). The new attempt starts with no `revisions` (`:7487`): its round files are its own. `WorkflowStart` (`:7510`) is unchanged.
- `rerun_ask` (`scheduler.rs:9335`): for a plan review, add a sentence naming the copy a rerun would start from, found by the same source search on the failed attempt's one artifact (its key is the subject). The wording is conditional, since `park` is also offered and the rerun may never be asked again (`asks_again` at `scheduler.rs:10608`, `may_rerun` at `:10665`): "If you rerun, the review starts from `review/1`'s copy, which holds the owner's revision of round N." Otherwise: "If you rerun, the review starts from `plan/1`'s copy." New helper beside `rerun_carries` (`:9382`), returning a `String`.
- `shown_where` (`dispatch/src/ticket.rs:1476`): keep a `Workflow` attempt with a non-empty `revisions` as well as an open one. `input_where` (`ticket.rs:1452`) is unchanged, so an agent never reads a failed copy. The shown copy is then the one a rerun would start from, so `show`, the Plan tab and the rerun question agree.
- `ticket_paths` (`dispatch/src/serve.rs:538`): `plan_files` keeps its meaning, the copy as it stands, and gains the shown failed copy with `failed` set. Earlier failed revised copies go in a new `PathsView.prior_plans`: the newest failed or cancelled plan review with a `revisions` entry in each context, other than the shown one. Each takes its `lane` the way `lane_files_by` (`scheduler.rs:10908`) does: the attempt's `context` when its stage runs per lane, else none. A per-lane run's prior copy keeps its own lane, and `plan_files` never holds another lane's copy. `plan_failed` is set when the shown copy's attempt is failed or cancelled.

## The UI

- `show` `files:` (`dispatch/src/main.rs:896` `print_lane_files`): the parenthetical comes from one helper that returns it as a `String`, so it can be unit-tested:
  - open copy: `(reviewed copy, round N, review open)`, unchanged;
  - shown failed copy: `(revised copy, round N, review failed; not reviewed again yet)`;
  - each `prior_plans` entry, printed after the shown copy by a loop in `show` (`main.rs:981`) that uses the same labels: `(prior revision, round N, review failed)`.
- Plan tab (`src/ui/ticket.rs:427`): a `plan_failed` arm before the existing ones: "Revised copy, round N: review failed; not reviewed again yet".
- Spec text, exact edits: `docs/design.md:1785-1786`, "a rerun kills the old run's agents and starts a fresh run, and a" becomes "a rerun kills the old run's agents and starts a fresh run from the copy the rerun question names, and a". `docs/design.md:2160-2163`, the Plan tab's fallback "else the newest complete plan labelled with the stage that wrote it" gains, before its "else", a clause for a failed revised copy labelled "Revised copy, round N: review failed; not reviewed again yet". `docs/dispatch.md:1046` needs no edit: `input_where` is unchanged, and a failed copy is never a completed attempt.

## The tests

- `dispatch/tests/first_slice.rs`, after `a_failed_review_run_fails_the_attempt_and_a_rerun_starts_a_new_run` (`:3257`). New test `a_rerun_after_a_failed_review_starts_from_the_owners_revision`: set up the converged run as the revise test at `:3442` does; call `revise` (`:3417`); write the revised text to `artifact_of(&t, "review", "plan")` as the planner's in-place edit; clear the runs and step, so the attempt fails `review run gone: no such run` (as `:3809` does); assert the pending rerun question names `review/1`'s copy and its revision of round 1; answer `rerun`; assert `review/2/joined/plan.md` equals the revised text and that `review/2` has no `revisions` of its own. Then fail `review/2` without a revise and rerun: `review/3`'s copy still equals the revised text. A second test, `a_rerun_reviewing_its_own_round_one_reads_it_as_the_reviewers`: on `review/2`, write `plan.feedback-1.md` beside its copy as a reviewer's round, and assert `plan_rounds` and `review_round` (`serve.rs`) give it no owner's `by` and no owner line, and `plan_review` (`report.rs:342`, a unit test beside it) reports it as a reviewer's round, not as an owner's objection.
- Same file, extend `a_failed_review_run_fails_the_attempt_and_a_rerun_starts_a_new_run` (`:3257`): with no revise, the question says the copy is from `plan/1`, and the new copy equals `plan/1`'s. This pins today's behaviour for a revision-less failure.
- `dispatch/src/scheduler.rs` tests, beside `lane_files_leave_out_a_forgotten_plan_and_keep_pipeline_order` (`:12615`): a failed revised copy whose `forgotten` names `plan` is not chosen, so `review_subject` returns `plan/1`'s copy.
- `dispatch/src/ticket.rs` tests, beside `the_plan_shown_is_an_open_review_copy_but_an_input_never_is` (`:1827`): a failed plan review with a revision is shown but is not an input.
- `dispatch/src/serve.rs` tests (module at `:996`): `ticket_paths` lists the shown failed copy in `plan_files` with `failed` set; an earlier failed revised copy is in `prior_plans` with its round and, for a per-lane stage, its own lane; `plan_files` holds no other lane's copy.
- `dispatch/src/main.rs` tests (module at `:1899`): the label helper's output for each case, and the prior line after the shown copy.
- `tests/ui.rs:5855-5862`: a case in `the_plan_tab_labels_a_reviewed_copy` for a `plan_failed` copy, "Revised copy, round 1: review failed; not reviewed again yet".
- Checks, from the repo root: `cargo test --locked --workspace`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo fmt --all`, and `cargo clippy --locked -p dispatch --all-targets --target x86_64-unknown-linux-gnu -- -D warnings`, since CI runs on Linux.
- Docs: a `CHANGELOG.md` line under `## [Unreleased]`, and the three spec edits under "Spec text, exact edits" above. `README.md` needs no change: no new file, no dev aid, no data-dir entry. In `docs/design.md`, a new dated section, `## Review rerun copy status (2026-10-21)`, with Built and Known gaps. Known gaps: the note is not re-sent; a failed review's own round edits are lost on its rerun, which starts from the newest revised copy or `plan/1`; a copy whose run died mid-edit may be partly revised, and the `show` label is the only warning.

Commit message:

Dispatch: a plan review's rerun starts from the owner's revised copy, and show names the copy reviewed

Contract additions (all in `dispatch-control/src/lib.rs`, serde-default, not persisted):

- `PathsView.plan_failed: bool` (struct at `:546`)
- `PathsView.prior_plans: Vec<LaneFile>` (struct at `:546`)
- `LaneFile.failed: bool` (struct at `:586`)

Update the struct literals in `tests/ui.rs` that build `PathsView`, or use `..Default::default()`. No change to `model.rs`, `action.rs`, or the ports.
