# Issue #187: a plan review rerun starts from the newest plan, owner revision included

## Where this departs from the issue

- **"`show` lists `review/1 joined failed` and `review/2 joined running`, and the files line names only the attempt-2 copy as the plan."** Borne out. `shown_where` (`dispatch/src/ticket.rs:1476`) keeps only complete attempts and open workflow attempts, so the failed attempt 1's copy cannot appear, and `plan_rounds` (`dispatch/src/serve.rs:504`) is read from the newest workflow attempt with artifacts only, so attempt 1's owner round file drops out of `show` too.
- **"Attempt 2 started from `plan/1/joined/plan.md`."** Borne out, and the cause is one function: `review_subject` (`dispatch/src/scheduler.rs:10534`) takes the newest *complete* attempt that wrote the subject. A review attempt that failed after a `revise` is not complete, so `start_workflow` (`dispatch/src/scheduler.rs:7436`) copies the plan stage's output again. Nothing in the rerun path passes a plan input other than that.
- **"The `revise` note is owner input and should survive a runner failure the same way an answered decision does."** Partly. The note already survives: it is on the answered decision and in `plan.feedback-<n>.md` beside attempt 1's copy (`revise`, `dispatch/src/scheduler.rs:7565`), and the planner's answer to it is in the revised copy. What is lost is the copy and, from `show`, the file. The plan carries the copy forward and lists the earlier attempt's owner rounds; it does not re-send the note to the new run (`Body::WorkflowStart` has no field for one, and the revised copy already answers it).
- **"Two of attempt 2's three round-1 points were things the revised copy had already fixed."** Follows from the above; nothing separate to do.
- The issue calls the rerun question's content "which copy it will review". Today `rerun_ask` (`dispatch/src/scheduler.rs:9335`) names no copy for a plan review; the plan adds a clause.
- The planner clone for the new run is still made from the plan stage's session (`source` in `Body::WorkflowStart`), because the failed attempt's session is a clone, not the writer. The clone's conversation does not hold the revise round; the file does. Left as is.

## The record

`Attempt` (`dispatch/src/ticket.rs`, beside `carried_from`, line 317) gains `subject_from: Option<(String, u32)>`, `#[serde(default)]`: the stage and number of the earlier attempt whose copy a plan review attempt started from, set only when that was not the plan stage's output. Written when the attempt is made, never changed. `carried_from` is not reused: it means a code review's points carry over and `report.rs` reads it that way.

`RECORD_VERSION` (`dispatch/src/store.rs:26`): the value there at implementation time, plus one (28 today). Add the matching "N to N+1" comment in `migrate` (the comment block ends at "27 to 28"): nothing is transformed, the field comes from its serde default, and a build that would drop it must refuse the record, or a rerun would forget which copy it reviews. Add a "Version N+1" paragraph in `docs/dispatch.md` after the existing version paragraphs (around line 393).

`new_attempt` (`dispatch/src/scheduler.rs:10277`) sets `subject_from: None`.

## The signals

- The `attempt-started` event text (`dispatch/src/events.rs:696`) gains ` from review/1's copy` when `subject_from` is set. No new event kind.
- The `rerun` question for a plan review attempt (`rerun_ask`, `dispatch/src/scheduler.rs:9335`) gains a clause, beside `rerun_carries`, when the copy the next attempt would review is not the plan stage's output: ` The next attempt reviews review attempt 1's copy, which holds your revision (round 2), not the plan stage's output.` Only that case gets a clause; the wording for every other rerun is unchanged.
- Both read the same pure function, so the question and the start cannot disagree.

## The actions and effects

No new action, effect or port method. The change is in what `start_workflow` copies.

Replace `review_subject(t, subject)` with `review_source(t, stage, ctx, subject) -> Result<Source, String>`, where `Source { path, session, from: Option<(String, u32)> }`, pure, in `dispatch/src/scheduler.rs`:

1. `session`: as today, the session of the newest complete attempt that wrote `subject` (the plan stage's writer, whose conversation the planner clone is made from). No session is still the existing "was not written by a session" refusal.
2. `path`: walk attempts newest first. A candidate is either (a) a complete attempt that wrote `subject`, as today, or (b) an attempt of this `stage` and `ctx`, kind `Workflow`, neither open nor complete, that wrote `subject`, whose file still exists and is not in `forgotten`, and that either holds an owner revision (`revisions` not empty) or itself started from an earlier copy (`subject_from` set). The first candidate wins; for (b), `from` is that attempt's `(stage, n)`.
3. The second clause of (b) is what keeps the revision through a second failure: attempt 2 fails with no revision of its own, attempt 3 starts from attempt 2's copy, which started from attempt 1's.
4. A failed attempt with no revision stays out of (b): its copy is whatever the planner had edited when the run died, and the issue asks only for the owner's revision to survive. A rerun of it reviews the plan stage's output again, as now.

`start_workflow` passes `stage.name` and `ctx` (it already has both), copies `source.path`, and stores `subject_from` on the new attempt. The `Sessions`/`WorkflowStart` request is unchanged. The existing park-on-error arm stays for "nothing wrote ..."; a missing file under (b) falls through to the next candidate rather than parking.

The lookup is scoped to the stage and context, which the current code is not (it takes any attempt of any stage). That is a deliberate narrowing for (b) only; (a) keeps today's reach.

## The UI

`show` (`dispatch/src/main.rs:981`, `print_lane_files` at line 896) and the dispatch-control wire (`dispatch-control/src/lib.rs`, additive, `#[serde(default)]`):

- `LaneFile` gains `from: Option<String>`, e.g. `review/1`, set when the shown copy's attempt has `subject_from`. `print_lane_files` prints `(reviewed copy, round N, review open, from review/1)`.
- `PathsView` gains `plan_earlier: Vec<EarlierPlan>`, each `{ stage, n, state, path, rounds: Vec<PlanRoundView> }`: the other attempts of the shown stage and context that hold a `plan` copy, oldest first, with the owner's rounds (`PlanRoundView.by` set) of that attempt. `ticket_paths` (`dispatch/src/serve.rs:538`) fills it with `plan_rounds` for each such attempt.
- `show` prints under the plan line: `plan (earlier): review/1 failed, your revision at round 2: <path>` and, beneath it, `owner round 2: <feedback path>`.
- The app's ticket page is not changed in this commit; it ignores the new fields.

Update `docs/dispatch-agent-guide.md` (the "files to read next" paragraph near line 205) with the `from` suffix and the `plan (earlier)` lines.

## The tests

In `dispatch/tests/first_slice.rs`, using `at_finalize` (line 3365) and the `revise` helper beside `revise_writes_the_owners_round_and_sends_it_as_an_objection` (line 3442):

- `a_rerun_after_a_revise_reviews_the_revised_copy`: at finalize, revise, edit the review copy on disk to stand for the planner's answer, clear `sb().runs`, step so the attempt fails `review run gone`, answer `rerun`, step. Assert the new attempt's copy has the edited text, `subject_from == Some(("review", 1))`, and that `WorkflowStart.plan` is the new copy.
- `a_rerun_without_a_revision_reviews_the_plan_output_again`: the existing `a_run_switchboard_no_longer_has_fails_the_attempt` setup, rerun; the copy equals the plan stage's output and `subject_from` is `None`.
- `a_second_failure_keeps_the_first_revision`: fail attempt 2 too; attempt 3's copy still has the edit and `subject_from == Some(("review", 2))`.
- `a_swept_revised_copy_falls_back_to_the_plan_output`: delete attempt 1's copy before the rerun; attempt 2 reviews the plan output, nothing parks.
- The rerun question names the revised copy only in the revised case (assert on the pending decision's text in the first two tests).

Unit tests beside the code: `review_source` over hand-built tickets in `dispatch/src/scheduler.rs`'s test module (complete only; revised failed; failed without revision; forgotten; other context), a `ticket_paths` test in `dispatch/src/serve.rs` for `plan_earlier` and `LaneFile.from`, a `print_lane_files` assertion in `dispatch/src/main.rs`, and an events test for the `attempt-started` text. `dispatch/src/store.rs` gets a migration test that a record at the previous version reads with `subject_from == None`.

## Docs and release

- README Layout table: no new file, so no row; update the `src/ticket.rs`-style description of `Attempt` if the Dispatch row lists fields (it lists `revisions` for the plan review), adding `subject_from`.
- `docs/dispatch.md`: the version paragraph above, and the `revise` row near line 2975 gains "a rerun of a review that failed after a revise starts from that attempt's copy".
- `docs/design.md`: add one line under the status section of the milestone that holds Dispatch's plan review stating the gap is closed. If it has no such section, nothing there.
- `CHANGELOG.md`, under Unreleased: "A Dispatch plan review rerun after a failed attempt starts from the owner's revised copy instead of the plan stage's original output, and `show` and the rerun question say which copy it reviews."

## Commit message

Dispatch plan review reruns start from the previous attempt's revised copy and say which copy they review

### Contract additions

- `dispatch::ticket::Attempt.subject_from: Option<(String, u32)>` (`#[serde(default)]`).
- `dispatch::store::RECORD_VERSION`: the value in `store.rs` at implementation time, plus one, with a `migrate` comment step.
- `dispatch_control::LaneFile.from: Option<String>`.
- `dispatch_control::PathsView.plan_earlier: Vec<EarlierPlan>` and the new `dispatch_control::EarlierPlan { stage, n, state, path, rounds }`.
- `scheduler::review_subject` replaced by `review_source` (crate-private; not part of the shared contract).
