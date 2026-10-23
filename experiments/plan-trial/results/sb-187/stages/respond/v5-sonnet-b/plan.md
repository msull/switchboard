# Plan: a review rerun starts from the owner's revised plan (issue #187)

All of it is in the `dispatch/` and `dispatch-control/` crates. The egui app only hosts the run and takes `source` and `plan` as given (`src/core/control.rs:441-455`, facts: Settled, "The planner clone and the source session"). Line numbers are from this worktree.

## Where this departs from the issue

- **"The `revise` note … should survive a runner failure the same way an answered decision does."** It already does, in three places: `DecisionState::Answered{note}`, the round file `review/1/joined/plan.feedback-<n>.md`, and `Attempt.revisions` (round, by, time; `ticket.rs:346-350`, `Revision` at `ticket.rs:510`; facts: Settled, "What survives"). What is lost is the revised copy as the next attempt's input. The plan changes nothing about how the note is stored.
- **"Start from the newest plan the stage holds."** The plan narrows this to a failed or cancelled review attempt whose record shows the owner answered `revise` (`revisions` non-empty). `ticket.rs:1856` asserts on purpose that "a parked review's copy is not the plan going forward", and a failed review that was never revised has no owner input at risk. So `Ticket::shown_where`, `input_where` and that test are left alone. Only `review_subject` (`scheduler.rs:10534`, its only caller is `start_workflow` at `7465`) changes. A failed review without a `revise` that holds accepted-point edits is a follow-up (see the end).
- **"Or at least say which copy it will review."** Both are done: the seed is chosen, and the `rerun` question and `show` say which copy it is.
- **Not fixable by seeding a file.** The planner clone is always `CloneAllTranscript` of the plan stage's session (`src/core/workflow.rs:289-372`), which never saw the `revise` exchange or attempt 1's rounds. This is the "For the owner" line in the facts file. The plan's default is the design that works either way: seed the file, and say in the question that the planner's earlier rounds are not carried. Nothing in the plan depends on the owner's answer.
- A `--note` on a Workflow `rerun` has no consumer (`rerun_note`, `scheduler.rs:8988`). The issue does not ask for one, so it is left alone.

## The record

`Attempt` gains `seeded_from: Option<(String, u32)>`, `#[serde(default)]`, next to `carried_from` (`ticket.rs:317`) and written the same way: set when the attempt is made, never changed. It names the stage and attempt number whose revised copy this workflow attempt was copied from. It is `None` when the copy came from the plan stage as before.

- Why record it rather than infer it later: the question could be derived from the failed attempt, but `show` after the rerun has nothing but the new attempt's own record, and "what is the on-disk record" comes first.
- `RECORD_VERSION` (`dispatch/src/store.rs:26`, 28 today): the value at implementation time plus one, because another ticket may take the next number first. Add a step to `migrate` (`store.rs:452`), with the usual comment after "27 to 28": an attempt gains `seeded_from`, absent from its serde default, nothing transformed, and a build that would drop it on its next write must refuse the record, or `show` would stop saying which copy a review used. The Switchboard app's `SCHEMA_VERSION` in `src/core/model.rs` is not touched.
- `new_attempt` (`scheduler.rs:10277`) keeps its signature. `start_workflow` sets the field on the pushed attempt (`scheduler.rs:7487`).

## The picker (the fix)

Replace `review_subject` with a pure `review_source(t, subject)` returning the path, the source session, and `Option<(String, u32)>` for the attempt it was seeded from. No I/O in it, so the question can use it too.

- Walk `t.attempts` newest first, as today. Stop at the first of:
  - a `Complete` attempt holding `subject` (as today; the plan stage's output, or a plan stage rerun that is newer than any failed review wins here, which keeps a plan rerun after a failed review correct);
  - a `Workflow` attempt that is `Failed` or `Cancelled`, holds `subject`, and has non-empty `revisions` (new; its copy is the path, and it is the `seeded_from`).
- The session is the `session` of the newest `Complete` attempt holding `subject`, exactly as today. A review attempt has none, so it can never be the source session. Error texts (`nothing wrote …`, `… was not written by a session`) stay as they are.
- **The unresolved line (is the failed copy whole?).** The decision is made from the attempt's own record: `revisions` says a `revise` was answered and by whom. File contents are never inspected. `start_workflow` then does one filesystem check: if the picked revised copy is not a file, it parks with `stage review: the revised copy review/1/joined/plan.md is gone`, rather than silently falling back to the original, which is the bug in the issue. If the file is whole, that is the favourable case and nothing more happens. If it is half-written, the owner sees the path in the question and can answer `park`; no extra option or content check is built.
- `forgotten` is not consulted: a plan is not a secret artifact.

## The signals

Nothing new: no event, no hook, no poll. The failure still reaches `fail_attempt_with` (`scheduler.rs:7195`) from `poll_workflow` (`scheduler.rs:7741`) and asks `rerun` as before.

## The actions and effects

No new action or effect. `rerun` is answered at `scheduler.rs:3524`; `may_rerun` (`scheduler.rs:10665`) relaunches through `start_workflow`, which now calls `review_source`.

`rerun_carries` (`scheduler.rs:9382`) becomes `-> String` (a private function with two callers: `rerun_ask` at `9373` and the question built at `7263`). For a Workflow attempt it appends, from `review_source` on the failed attempt's single artifact name:

- seeded case: ` The rerun reviews the revised copy {path} (the owner's revise, round {n}), not the plan stage's {original}. The planner starts from the plan stage's transcript and does not carry this attempt's rounds; the owner's note is in {feedback file}.`
- unseeded case: ` The rerun reviews {original path}, the plan stage's output. The planner does not carry this attempt's rounds.`

The `Agent`/`Review` text and the resolution text are unchanged. The round-file path is the revised copy's stem plus `.feedback-{round}.md`, the same name `report.rs` reads.

## The UI

- `dispatch-control` `LaneFile` (`dispatch-control/src/lib.rs:586`) gains `seeded_from: Option<(String, u32)>`, `#[serde(default)]` like its siblings. `ticket_paths` (`serve.rs:~560`) fills it from the attempt that wrote the shown file.
- `print_lane_files` (`dispatch/src/main.rs:896`) prints, under the file when set: `(seeded from the revised copy of review/1, which it supersedes)`. It sits beside the existing `(reviewed copy, round N, review open)` line and stays after the review completes.
- Between the failure and the answer, `show` still lists the plan stage's file as `plan:` (a failed copy is not shown, by `ticket.rs:1856`). That window is covered by the pending `rerun` question, which `show` already prints and which names the copy. No `show` change is made for it.
- No egui change, no `SWITCHBOARD_SCRIPT` line (Dispatch is not driven that way).

## The tests

First slice, `dispatch/tests/first_slice.rs`, using `at_finalize`, `revise` (3417), and the pattern of `a_run_switchboard_no_longer_has_fails_the_attempt` (3809). The fake Switchboard does not write files, so each test writes the "revised" text into `review/1/joined/plan.md` itself before the failure.

1. Repro: finalize, `revise` with a note, write revised text to the copy, `env.sb().runs.clear()`, step, assert the pending `rerun` question names that path and says the planner does not carry earlier rounds, answer `rerun`, step. Assert `review/2`'s copy holds the revised text, its `seeded_from == Some(("review", 1))`, and `source` is still the plan stage's session.
2. A failed review with no `revise`: the rerun copies `plan/1/joined/plan.md`, `seeded_from` is `None`, and the question names the original.
3. The revised copy is deleted before the rerun: the ticket parks with the reason above and no `WorkflowStart` is sent.
4. A `park` then resume (Cancelled with `revisions`) seeds from the revised copy too.
5. The plan stage reruns after the failed review and completes: the newer plan wins over the revised copy.

Unit tests beside `review_subject`'s old tests (`scheduler.rs`, near the "finalized: next stage follows the reviewed copy" tests at ~12838): `review_source` for the cases above, as pure functions on a built `Ticket`. `ticket.rs` tests at 1836-1905 are unchanged and must still pass, which is the check that `shown` and `input` were not touched.

`store.rs`: a migrate test next to the existing ones: a record one version below, without `seeded_from`, loads with `None`; one a version above is refused.

`serve.rs`: `ticket_paths` sets `LaneFile.seeded_from` for the shown review copy and leaves it `None` otherwise.

Docs in the same commit: `CHANGELOG.md` line under Unreleased; `docs/dispatch.md` decision 16 ("A review runs on a copy of the reviewed artifact", line 3212) gains the revised-copy rule and the test table gains the rows; the Dispatch status section in `docs/design.md` (around 1775-1800) notes the new field and `RECORD_VERSION`. The README Layout table gains nothing (no new file). The app runs at every step: the field is additive and defaulted, so a record written before loads and an older build refuses a newer one.

## Follow-ups for the owner (not in this plan)

- Whether a failed review that was never revised but holds accepted-point edits should also seed a rerun. That reverses the `ticket.rs:1856` rule and is a decision, not a fix.
- Carrying the planner's earlier rounds, or the owner's note, into the rerun's clone. The facts file's owner question decides whether it matters; it needs a new field on `WorkflowStart` and an app-side change, which is an extension of the contract and is not built here.
- A `--note` on a Workflow `rerun` has no consumer.

## Commit message

Dispatch: a review rerun after a failed attempt starts from the owner's revised copy and says which copy it reviews

## Contract additions

- `dispatch::ticket::Attempt.seeded_from: Option<(String, u32)>` (`#[serde(default)]`)
- `dispatch_control::LaneFile.seeded_from: Option<(String, u32)>` (`#[serde(default)]`)
- `dispatch::store::RECORD_VERSION`: from the value in `store.rs` at implementation time (28 today), plus one, with a `migrate` step
- `scheduler::review_source` replaces `review_subject` (private); `rerun_carries` returns `String` (private). No change to `AppAction`, `Effect`, `ports/`, `model.rs`, or the `WorkflowStart` body.
