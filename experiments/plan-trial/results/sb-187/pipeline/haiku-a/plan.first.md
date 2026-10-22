# Issue #187: a plan review's rerun starts from the owner's revision

## Where this departs from the issue

- Bears out: the rerun copies `plan/1` because `review_subject` keeps only `Complete` attempts (`dispatch/src/scheduler.rs:10534`). `show` and the Plan tab name only the attempt-2 copy (`dispatch/src/ticket.rs:1476` `shown_where`, `dispatch/src/serve.rs:539`). The rerun question never names a copy: `rerun_carries` (`scheduler.rs:9382`) speaks only for code review.
- Differs: the owner's note is kept on the answered `finalize` decision, and `show` prints it (`dispatch/src/main.rs:1176`), so it already survives the failure. The new planner never receives it, and this plan does not re-send it. Sending needs a `workflow.object` on a `Converged` or `AtCap` run with round = round count + 1 (`docs/design.md:1190-1200`), and a new run has no rounds yet. The plan carries the owner's change in the copy instead. Re-sending the note is deferred to its own change.
- Differs from the investigation notes: the revised copy is gated on the attempt's `revisions` entry, not on the planner's `plan.response-N.md` (`dispatch/src/report.rs:202`). A run killed before the planner answers, which is the repro's shape, leaves no response file, so a response gate would miss the bug. The `revisions` entry is written before the send (`scheduler.rs:7620`) and removed on a refusal (`docs/design.md:2975`).
- Not covered: a plan review that fails after reviewer rounds, with no owner `revise`, still restarts from `plan/1`. Its in-place round responses are lost. The rerun question will name the copy, so this is visible. Widening the gate to round files is a small follow-up if the owner wants it.
- Name check: the notes call the constant `SCHEMA_VERSION`. This tree has `RECORD_VERSION` (`dispatch/src/store.rs:26`), value 28.

## The record

No field added. The gate is the existing `Attempt.revisions` (`dispatch/src/ticket.rs:350`, `Revision` at `ticket.rs:510`). A rerun of a plan review also copies its source's `revisions` onto the new attempt, so a second failure still finds the revised copy. The copied entries are the copy's history, not the new run's rounds. A later `revise` on the new attempt replaces round 1 by the existing `retain` at `scheduler.rs:7620`.

No schema bump. `RECORD_VERSION` is 28 today (`store.rs:26`) and stays 28. `USAGE_VERSION` (`store.rs:642`) is untouched.

## The signals

None new. The failed or cancelled review attempt that has a `revisions` entry is the signal.

## The actions and effects

No new action or effect. The changes are inside existing paths:

- `review_source(t, subject)` (new, `scheduler.rs`, beside `review_subject` at `:10534`): returns the newest attempt that holds the subject artifact and is `Complete`, or is `Failed` or `Cancelled` with a non-empty `revisions`.
- `review_subject` (`scheduler.rs:10534`) takes the copy path and its `revisions` from `review_source`. The session stays as it is today, from the newest `Complete` attempt, so the reviewer's planner clone still comes from the plan's author.
- `start_workflow` (`scheduler.rs:7436`) copies from that path (`:7470-7471`, shape unchanged). After its `t.attempts.push(new_attempt(..))` (`:7487`) it sets the new attempt's `revisions` to the source's. `WorkflowStart` (`:7510`) is unchanged.
- `rerun_ask` (`scheduler.rs:9335`): for a plan review, append a sentence naming the source: "The rerun reviews the copy from `review/1`, which holds the owner's revision of round N." When the source has no revisions, the sentence is "The rerun reviews the copy from `plan/1`." New helper beside `rerun_carries` (`:9382`), returning a `String`.
- `shown_where` (`dispatch/src/ticket.rs:1476`): keep a `Workflow` attempt with a non-empty `revisions` as well as an open one. `input_where` (`ticket.rs:1452`) is unchanged, so an agent never reads a failed copy.
- `ticket_paths` (`dispatch/src/serve.rs:538`): `plan_files` lists the shown copy, then each other failed or cancelled plan review that has a `revisions` entry, marked `prior`. `plan_failed` is set when the shown copy's attempt is failed or cancelled.

## The UI

- `show` `files:` (`dispatch/src/main.rs:896` `print_lane_files`): the labels, from one helper that returns the note as an `Option<String>` so it can be unit-tested:
  - open copy: `(reviewed copy, round N, review open)`, unchanged;
  - shown failed copy: `(revised copy, round N, review failed: the rerun reviews it)`;
  - prior copy: `(prior revision, round N, review failed)`.
- Plan tab (`src/ui/ticket.rs:427`): a `plan_failed` arm before the existing ones: "Revised copy, round N: review failed; the rerun reviews it".
- Spec text to match: `docs/dispatch.md:1046` (the `{inputs.<artifact>}` rule: complete attempts stay the input for agents, and a plan review's copy follows the rule above), `docs/design.md:1785-1787` ("starts a fresh run" gains "from the copy the question names"), `docs/design.md:2160-2163` (the Plan tab's fallback).

## The tests

- `dispatch/tests/first_slice.rs`, after `a_failed_review_run_fails_the_attempt_and_a_rerun_starts_a_new_run` (`:3257`). New test `a_rerun_after_a_failed_review_starts_from_the_owners_revision`: set up the converged run as the revise test at `:3442` does; call `revise` (`:3417`); write the revised text to `artifact_of(&t, "review", "plan")` as the planner's in-place edit; clear the runs and step, so the attempt fails `review run gone: no such run` (as `:3809` does); assert the pending rerun question contains "owner's revision"; answer `rerun`; assert `review/2/joined/plan.md` equals the revised text and that attempt carries round 1 in `revisions`. Then fail `review/2` without a revise and rerun: `review/3` still has the revised text.
- Same file, extend `a_failed_review_run_fails_the_attempt_and_a_rerun_starts_a_new_run` (`:3257`): with no revise, the question says the copy is from `plan/1`, and the new copy equals `plan/1`'s. This pins today's behaviour for a revision-less failure.
- `dispatch/src/ticket.rs` tests, beside `the_plan_shown_is_an_open_review_copy_but_an_input_never_is` (`:1827`): a failed plan review with a revision is shown but is not an input.
- `dispatch/src/serve.rs` tests (module at `:996`): `ticket_paths` lists the shown failed copy with `failed` set, and an earlier failed copy as `prior`, each with its round.
- `dispatch/src/main.rs` tests (module at `:1899`): the label helper's three outputs.
- `tests/ui.rs:5855-5862`: a row for the failed copy, "Revised copy, round 1: review failed".
- Checks, from the repo root: `cargo test --locked --workspace`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo fmt --all`, and `cargo clippy --locked -p dispatch --all-targets --target x86_64-unknown-linux-gnu -- -D warnings`, since CI runs on Linux.
- Docs: a `CHANGELOG.md` line under `## [Unreleased]`. `README.md` needs no change: no new file, no dev aid, no data-dir entry. In `docs/design.md`, a new dated section, `## Review rerun copy status (2026-10-21)`, with Built and Known gaps. Known gaps: the note is not re-sent; a round-only failure restarts from `plan/1`; a copy whose run died mid-edit may be partly revised, and the `show` label is the only warning.

Commit message:

Dispatch: a plan review's rerun starts from the owner's revised copy, and show names the copy reviewed

Contract additions (all in `dispatch-control/src/lib.rs`, serde-default, not persisted):

- `PathsView.plan_failed: bool` (struct at `:546`)
- `LaneFile.failed: bool` (struct at `:586`)
- `LaneFile.prior: bool` (struct at `:586`)

Update the struct literals in `tests/ui.rs` that build `PathsView`, or use `..Default::default()`. No change to `model.rs`, `action.rs`, or the ports.
