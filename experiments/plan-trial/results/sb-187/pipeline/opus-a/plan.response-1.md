# Response to round 1

1. **Restarts.** Agreed. Changed: "The rule". The walk now stops at an
   attempt Cancelled with a reason starting with `DISCARDED_BY`
   (`restart.rs:34`), and at any attempt that ended before
   `t.restarts.last().at_ms`, compared on `ended_ms.unwrap_or(started_ms)`
   as `ask_rerun` does (`scheduler.rs:7230-7241`). The cutoff is the
   check that catches a Failed attempt the restart left Failed. The
   case list under the rule says a restart at review or earlier does not
   carry. Test added in "The tests":
   `a_restart_at_review_after_revise_starts_from_the_plan_stage`.

2. **Send-back to the review stage.** Chosen explicitly. Changed: "The
   rule" (the walk stops at a `SENT_BACK_FROM` cancellation,
   `scheduler.rs:8979`, set at `4960-4999`, plus a case-list entry saying
   why), "Docs" (the design.md bullet states it), and "The tests"
   (`a_send_back_to_review_starts_from_the_plan_stage`). A send-back
   keeps today's behaviour and starts from the plan stage's copy. It
   asks for the review to be done again, and the finalized copy it
   cancels is the thing being objected to.

3. **Another lane's attempt.** Agreed. Changed: "The rule". The walk now
   covers only attempts of this stage *and* context, and its stop
   conditions don't depend on `review_subject`'s walk, so another lane's
   Complete review can't end it. `review_subject` still chooses the
   session and still ignores context (`scheduler.rs:10534-10547`). The
   plan names that as an existing bug outside this issue. Test added in
   "The tests": a per-lane variant of the `serve.rs:1521-1568` fixture.

4. **The note can still be lost.** Agreed that the note could be dropped
   silently, and the first departure in "Where this departs from the
   issue" no longer says the note was never lost. I took the minimum
   option, not delivering the note to the new run. `WorkflowStart` has no
   field for an opening objection, and an objection is only sent to a
   `Converged`/`AtCap` run (`scheduler.rs:7608-7614`). Delivering it
   would mean a wire change on both sides. Changed:
   - "The actions and effects": `ReviewSource::unanswered` is the newest
     owner round file that has no `plan.response-<n>.md`. It is named in
     the log line (step 2), in the rerun question (step 3), and in the
     rerun's own `finalize` question (step 4, `scheduler.rs:7693-7696`)
     with "revise with --file <file>".
   - "The UI": `LaneFile::unanswered` and a `show` line.
   - "Commit": the second contract addition.
   - "The tests": `a_revise_the_planner_never_answered_is_named_in_the_rerun`
     fails the run before the copy changes or any response file exists.

5. **Cancelled.** Agreed. Changed: "The tests":
   `a_parked_review_after_revise_resumes_on_the_revised_copy` uses
   `park_and_resume` (`first_slice.rs:10584`) and checks the
   `rerun_ask` Cancelled-arm question names the revised copy. "The
   actions and effects" step 3 now notes that `rerun_carries` serves
   both of `rerun_ask`'s arms (`9336-9366`) as well as `ask_rerun`.
   "Docs" now says "failed or parked".

Also changed: "The rule" now settles the second-failure case. `review/3`
starts from `review/2`'s copy, the newest one, not from `review/1`'s,
and that test now asserts this.
